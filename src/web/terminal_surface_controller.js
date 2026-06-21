import { createTerminalSurfaceRuntimeHelpers } from "./terminal_surface_setup.js";

export function resolveStableFrankenTermCanvases(canvases = {}) {
  const terminalCanvas = canvases.terminalCanvas?.current ?? canvases.terminalCanvas;
  const hudCanvas = canvases.hudCanvas?.current ?? canvases.hudCanvas;
  if (!terminalCanvas) {
    throw new Error("FrankenTerm runtime adapter requires a stable terminalCanvas element or ref");
  }
  if (!hudCanvas) {
    throw new Error("FrankenTerm runtime adapter requires a stable hudCanvas element or ref");
  }
  return { terminalCanvas, hudCanvas };
}

export function createFrankenTermRuntimeAdapter(runtime) {
  const { state, boot } = runtime;
  const canvases = resolveStableFrankenTermCanvases(runtime.canvases);
  const el = { ...runtime.el, ...canvases };
  let controller = null;

  async function loadFrankenTermFont() {
    if (!boot.franken_term_font_url || !runtime.documentRef.fonts?.load) {
      return null;
    }
    if (!state.frankenFontInit) {
      state.frankenFontInit = runtime.documentRef.fonts
        .load('12px "Pragmasevka NF"')
        .catch((error) => {
          state.frankenLoadError = `font load failed: ${error?.message || String(error)}`;
          state.frankenFontInit = null;
          return null;
        });
    }
    return state.frankenFontInit;
  }

  async function ensureFrankenTerm() {
    if (!boot.franken_term_available) {
      return null;
    }

    if (!state.frankenInit) {
      state.frankenInit = (async () => {
        await loadFrankenTermFont();
        const mod = runtime.assertFrankenTermModule(await runtime.importModule(boot.franken_term_js_url));
        const wasmUrl = boot.franken_term_wasm_url
          ? new runtime.URLImpl(boot.franken_term_wasm_url, runtime.windowRef.location.href)
          : undefined;
        if (wasmUrl) {
          await mod.default(wasmUrl);
        } else {
          await mod.default();
        }
        state.frankenModule = mod;
        state.frankenLoadError = "";
        state.frankenAssetSummary = runtime.formatFrankenTermAssetSummary(boot.franken_term_asset_info);
        return mod;
      })().catch((error) => {
        state.frankenInit = null;
        state.frankenModule = null;
        state.frankenLoadError = error?.message || String(error || "FrankenTerm load failed");
        throw error;
      });
    }

    return state.frankenInit;
  }

  const adapterRuntime = {
    ...runtime,
    el,
    loadFrankenTermFont,
    ensureFrankenTerm,
    teardownTerminal: (...args) => controller.teardownTerminal(...args),
    destroyTerminalInstance: (...args) => controller.destroyTerminalInstance(...args),
    measureAndResizeSurface: (...args) => controller.measureAndResizeSurface(...args),
    feedTerminalBytes: (...args) => controller.feedTerminalBytes(...args),
  };
  const helpers = createTerminalSurfaceRuntimeHelpers(adapterRuntime);
  Object.assign(adapterRuntime, helpers);
  adapterRuntime.terminalResizeRuntime = {
    state,
    el,
    surfaceBusy: (...args) => controller.surfaceBusy(...args),
    queueMeasureAndResizeSurface: (...args) => controller.queueMeasureAndResizeSurface(...args),
    withSurfaceOperation: (...args) => controller.withSurfaceOperation(...args),
    renderHudSurface: (...args) => controller.renderHudSurface(...args),
    scheduleRender: (...args) => controller.scheduleRender(...args),
    sendResize: (...args) => controller.sendResize(...args),
    captureTerminalRendererDiagnostic: (...args) => controller.captureTerminalRendererDiagnostic(...args),
    devicePixelRatio: runtime.devicePixelRatio,
  };
  controller = createTerminalSurfaceController(adapterRuntime);
  return {
    ...helpers,
    ...controller,
    loadFrankenTermFont,
    ensureFrankenTerm,
    canvases,
    terminalResizeRuntime: adapterRuntime.terminalResizeRuntime,
  };
}

export function createTerminalSurfaceController(runtime) {
  const { state, el, boot } = runtime;
  const { loadFrankenTermFont, ensureFrankenTerm } = runtime;

  async function setupHudSurface() {
    const mod = await ensureFrankenTerm();
    if (!mod) {
      return null;
    }

    if (state.hud) {
      return state.hud;
    }

    runtime.setLoadingState(true, "Loading rendered control surface...");
    state.hud = runtime.validateFrankenTermSurface(
      new mod.FrankenTermWeb(),
      runtime.hudMethods,
      "HUD renderer",
    );
    state.surfaceInitInProgress += 1;
    try {
      await state.hud.init(el.hudCanvas, undefined);
    } finally {
      state.surfaceInitInProgress -= 1;
    }
    if (runtime.surfaceSupports(state.hud, "setAccessibility")) {
      state.hud.setAccessibility({
        reducedMotion: runtime.prefersReducedMotion(),
      });
    }
    runtime.applyZoomToSurface(state.hud);
    el.hudCanvas.classList.remove("hidden");
    measureAndResizeSurface(false, true);
    renderHudSurface();
    runtime.setLoadingState(false);
    return state.hud;
  }

  function destroyTerminalInstance() {
    const destroyPatch = runtime.terminalDestroyStatePatch();
    state.selectionAnchor = destroyPatch.selectionAnchor;
    state.selectionFocus = destroyPatch.selectionFocus;
    runtime.clearHoveredLink(false);
    clearTerminalPaintProbe();
    runtime.clearPendingTerminalBytes();
    if (state.terminal) {
      state.terminal.destroy();
    }
    // Forget this session's replay high-water mark: the rendered scrollback is
    // gone with the destroyed instance, so a future fresh terminal for the same
    // session must do a full replay/snapshot rather than resume_from_seq into a
    // blank surface (which would silently drop earlier output). Same-session
    // transient reconnects reuse the instance and never reach this path, so
    // resume-after-blip is preserved. Also bounds the per-session seq map.
    if (state.terminalSessionId) {
      state.lastTerminalSeqBySession.delete(state.terminalSessionId);
    }
    Object.assign(state, destroyPatch);
    if (el.terminalA11yMirror) {
      el.terminalA11yMirror.value = "";
    }
    el.terminalCanvas.classList.add("hidden");
  }

  function clearTerminalPaintProbe() {
    if (state.terminalPaintProbeTimer) {
      runtime.clearTimeoutRef(state.terminalPaintProbeTimer);
      state.terminalPaintProbeTimer = null;
    }
  }

  function teardownTerminal() {
    disconnectSocket();
    runtime.stopSnapshotPolling();
    destroyTerminalInstance();
    runtime.setTerminalTextFallbackActive(false);
    runtime.syncTerminalTools();
    renderHudSurface();
  }

  function disconnectSocket() {
    state.connectionGeneration += 1;
    runtime.clearReconnectTimer();
    if (state.ws) {
      state.ws.onopen = null;
      state.ws.onmessage = null;
      state.ws.onclose = null;
      state.ws.onerror = null;
      state.ws.close();
      state.ws = null;
    }
    // Input-ack waiters can never be resolved once the socket is gone; clear them
    // so the map can't grow unbounded across flaky reconnects / dropped acks.
    state.pendingInputMessages?.clear?.();
  }

  function surfaceBusy() {
    return runtime.runtimeSurfaceBusy(state);
  }

  function withSurfaceOperation(label, callback) {
    return runtime.runSurfaceOperation(state, label, callback);
  }

  function queueRenderRetry() {
    if (state.renderRetryQueued) {
      return;
    }
    state.renderRetryQueued = true;
    runtime.setTimeoutRef(() => {
      state.renderRetryQueued = false;
      if (!surfaceBusy()) {
        scheduleRender();
      }
    }, 0);
  }

  function queueHudRender() {
    if (state.hudRenderQueued) {
      return;
    }
    state.hudRenderQueued = true;
    runtime.setTimeoutRef(() => {
      state.hudRenderQueued = false;
      if (!surfaceBusy()) {
        renderHudSurface();
      }
    }, 0);
  }

  function queueMeasureAndResizeSurface(pushResize = false, force = false) {
    state.resizeQueued = true;
    state.resizePushResize = state.resizePushResize || Boolean(pushResize);
    state.resizeForce = state.resizeForce || Boolean(force);
    if (state.resizeRetryTimer) {
      return;
    }
    state.resizeRetryTimer = runtime.setTimeoutRef(() => {
      state.resizeRetryTimer = null;
      if (!state.resizeQueued || surfaceBusy()) {
        return;
      }
      const queuedPushResize = state.resizePushResize;
      const queuedForce = state.resizeForce;
      state.resizeQueued = false;
      state.resizePushResize = false;
      state.resizeForce = false;
      measureAndResizeSurface(queuedPushResize, queuedForce);
    }, 0);
  }

  function scheduleRender() {
    if (state.renderQueued) {
      return;
    }
    if (!state.terminal && !state.hud) {
      return;
    }
    state.renderQueued = true;
    runtime.requestAnimationFrameRef(() => {
      state.renderQueued = false;
      // A surface `init()` holds the wasm instance borrowed across its internal
      // `await`; calling `render()` during that window re-enters the same borrow
      // and trips the wasm-bindgen "recursive use of an object" panic. Re-queue
      // until init settles.
      if (surfaceBusy()) {
        queueRenderRetry();
        return;
      }
      const rendered = withSurfaceOperation("render", () => {
        if (state.terminal) {
          state.terminal.render();
        }
        if (state.hud) {
          state.hud.render();
        }
      });
      if (rendered.deferred) {
        queueRenderRetry();
      }
    });
  }

  function sendResize() {
    if (!state.ws || state.ws.readyState !== runtime.WebSocketClass.OPEN || !state.selectedSessionId) {
      return;
    }
    state.ws.send(JSON.stringify({ type: "resize", cols: state.currentCols, rows: state.currentRows }));
  }

  function measureAndResizeSurface(pushResize = false, force = false) {
    runtime.runTerminalSurfaceResize({ pushResize, force }, runtime.terminalResizeRuntime);
  }

  function captureTerminalRendererDiagnostic(reason = "frame") {
    if (!runtime.terminalSupports("snapshotResizeStormFrameJsonl")) {
      return null;
    }
    if (surfaceBusy()) {
      return null;
    }
    const frameIndex = state.rendererDiagnosticSequence;
    state.rendererDiagnosticSequence += 1;
    const timestamp = runtime.isoTimestamp();
    const diagnostic = withSurfaceOperation("snapshotResizeStormFrameJsonl", () => {
      const line = state.terminal.snapshotResizeStormFrameJsonl("swimmers-web", 0, timestamp, frameIndex);
      const parsed = JSON.parse(String(line || "{}"));
      return { line, parsed };
    });
    if (diagnostic.deferred) {
      return null;
    }
    try {
      const { line, parsed } = diagnostic.value;
      state.lastRendererDiagnostic = { reason, line, parsed };
      state.lastRendererDiagnosticError = "";
      return line;
    } catch (error) {
      state.lastRendererDiagnosticError = error?.message || String(error);
      return null;
    }
  }

  function buildSurfaceModel() {
    return runtime.buildSurfaceModelFromState({
      state,
      boot,
      currentSession: runtime.currentSession,
      operatorPressureSnapshot: runtime.operatorPressureSnapshot,
      sessionBurnt: runtime.sessionBurnt,
      normalizeSessionId: runtime.normalizeSessionId,
      now: runtime.now,
      websocketOpen: runtime.WebSocketClass.OPEN,
    });
  }

  function renderHudSurface() {
    runtime.advanceTrogdorReaderProgressForCurrentHover();
    runtime.renderTrogdorSurface();
    syncTerminalPresentation();
    if (!state.hud) {
      return;
    }
    // `applyPatchBatchFlat()` takes `&mut self`; while a surface `init()` is still
    // awaiting it holds that borrow, so re-entering here would panic. Defer the
    // HUD patch until init settles, then re-run.
    if (surfaceBusy()) {
      queueHudRender();
      return;
    }
    // Build into a reused buffer (no per-render realloc), then upload only the
    // cells that changed since the last uploaded frame. Two buffers alternate as
    // "build target" and "diff baseline"; a dimension change makes
    // buildSurfaceFrame allocate fresh and computeSurfaceDirtySpans fall back to
    // a full upload.
    const frame = runtime.buildSurfaceFrame(buildSurfaceModel(), { cells: state.hudBuildCells });
    state.surfaceZones = frame.zones ?? [];
    state.surfaceMasks = frame.masks ?? [];
    const dirtySpans = runtime.computeSurfaceDirtySpans(
      frame.cells,
      state.hudPrevCells,
      frame.cols,
      frame.rows,
    );
    if (dirtySpans.length === 0) {
      // Nothing changed; the surface already shows this frame. Keep the build
      // buffer and leave the baseline in place.
      state.hudBuildCells = frame.cells;
      scheduleRender();
      return;
    }
    const patched = withSurfaceOperation("applyPatchBatchFlat", () => {
      state.hud.applyPatchBatchFlat(dirtySpans, frame.cells);
    });
    if (patched.deferred) {
      // The patch did not land; keep the baseline and rebuild into the same
      // buffer on retry.
      state.hudBuildCells = frame.cells;
      queueHudRender();
      return;
    }
    // The uploaded frame becomes the next diff baseline; recycle the old
    // baseline as the next build buffer (undefined on the first frame -> fresh).
    state.hudBuildCells = state.hudPrevCells;
    state.hudPrevCells = frame.cells;
    scheduleRender();
  }

  function syncTerminalPresentation() {
    const plan = runtime.terminalPresentationPlan({ hasCurrentSession: Boolean(runtime.currentSession()), trogdorAtlasOpen: state.trogdorAtlasOpen, hasTerminal: Boolean(state.terminal), terminalFallbackActive: state.terminalFallbackActive });
    runtime.documentRef.body.classList.toggle("terminal-focus-mode", plan.terminalFocusMode);
    el.terminalStage.classList.toggle("terminal-view-active", plan.terminalStageActive);
    runtime.syncTerminalInputDock();
    runtime.syncTrogdorBackButton();
    runtime.syncTerminalWorkbench();
    if (state.hud) {
      el.hudCanvas.classList.toggle("hidden", plan.hudHidden);
      [el.hudCanvas.style.display, el.hudCanvas.style.visibility] = [plan.hudDisplay, plan.hudVisibility];
    }
    if (plan.showTerminalCanvas) {
      el.terminalCanvas.classList.toggle("hidden", plan.terminalCanvasHidden);
      [el.terminalCanvas.style.display, el.terminalCanvas.style.visibility] = [plan.terminalCanvasDisplay, plan.terminalCanvasVisibility];
    }
    el.terminalFallback.classList.toggle("hidden", plan.terminalFallbackHidden);
  }

  function feedTerminalBytes(bytes) {
    if (!(bytes instanceof runtime.Uint8ArrayClass)) {
      return false;
    }
    if (!state.terminal || !state.terminalAcceptsBytes) {
      return runtime.bufferTerminalBytes(bytes);
    }

    state.terminal.feed(bytes);
    state.terminalFrameBytesSeen += bytes.byteLength;
    runtime.flushEncodedInputBytes();
    if (state.searchQuery) {
      runtime.refreshTerminalSearch();
    }
    runtime.drainTerminalLinkClicks();
    runtime.syncTerminalAccessibilityMirror();
    runtime.syncTerminalFallbackFromLiveFrame();
    scheduleRender();
    scheduleTerminalPaintProbe();
    return true;
  }

  function scheduleTerminalPaintProbe() {
    const plan = runtime.terminalPaintProbeSchedulePlan({ terminalPaintVerified: state.terminalPaintVerified, terminalFallbackActive: state.terminalFallbackActive, hasProbeTimer: Boolean(state.terminalPaintProbeTimer), hasTerminal: Boolean(state.terminal), hasCurrentSession: Boolean(runtime.currentSession()), terminalFrameBytesSeen: state.terminalFrameBytesSeen });
    if (!plan.scheduleProbe) {
      return;
    }

    state.terminalPaintProbeTimer = runtime.setTimeoutRef(() => {
      state.terminalPaintProbeTimer = null;
      runtime.requestAnimationFrameRef(() => {
        runtime.requestAnimationFrameRef(() => {
          void verifyTerminalPaintOrFallback();
        });
      });
    }, plan.delayMs);
  }

  function terminalPaintVerificationContext(extra = {}) {
    return { hasTerminal: Boolean(state.terminal), terminalPaintVerified: state.terminalPaintVerified, terminalFallbackActive: state.terminalFallbackActive, hasCurrentSession: Boolean(runtime.currentSession()), ...extra };
  }

  function applyTerminalPaintVerificationPlan(plan) {
    if (plan.type === "painted") {
      state.terminalPaintVerified = true;
      captureTerminalRendererDiagnostic(plan.diagnosticReason);
      runtime.setTerminalTextFallbackActive(plan.fallbackActive);
      return true;
    }
    if (plan.type === "activate_fallback") {
      runtime.setTerminalTextFallbackActive(plan.fallbackActive, { clearText: plan.clearText });
      syncTerminalPresentation();
      return true;
    }
    return plan.done;
  }

  async function verifyTerminalPaintOrFallback() {
    let plan = runtime.terminalPaintVerificationPlan(terminalPaintVerificationContext());
    if (applyTerminalPaintVerificationPlan(plan)) return;
    plan = runtime.terminalPaintVerificationPlan(terminalPaintVerificationContext({ canvasHasVisiblePixels: terminalCanvasHasVisiblePixels() }));
    if (applyTerminalPaintVerificationPlan(plan)) return;
    const hasSnapshotText = await runtime.refreshSnapshotFallback();
    plan = runtime.terminalPaintVerificationPlan(terminalPaintVerificationContext({ afterSnapshotRefresh: true }));
    if (applyTerminalPaintVerificationPlan(plan)) return;
    applyTerminalPaintVerificationPlan(runtime.terminalPaintVerificationPlan(terminalPaintVerificationContext({ afterSnapshotRefresh: true, canvasHasVisiblePixels: terminalCanvasHasVisiblePixels(), hasSnapshotText })));
  }

  function terminalCanvasHasVisiblePixels() {
    return runtime.canvasHasVisiblePixels(el.terminalCanvas, runtime.documentRef);
  }

  return {
    loadFrankenTermFont,
    ensureFrankenTerm,
    setupHudSurface,
    destroyTerminalInstance,
    clearTerminalPaintProbe,
    teardownTerminal,
    disconnectSocket,
    surfaceBusy,
    withSurfaceOperation,
    queueRenderRetry,
    queueHudRender,
    queueMeasureAndResizeSurface,
    scheduleRender,
    sendResize,
    measureAndResizeSurface,
    captureTerminalRendererDiagnostic,
    buildSurfaceModel,
    renderHudSurface,
    syncTerminalPresentation,
    feedTerminalBytes,
    scheduleTerminalPaintProbe,
    terminalPaintVerificationContext,
    applyTerminalPaintVerificationPlan,
    verifyTerminalPaintOrFallback,
    terminalCanvasHasVisiblePixels,
  };
}
