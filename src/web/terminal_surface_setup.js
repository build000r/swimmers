import {
  terminalFallbackActivationPlan,
  terminalFallbackTextScrollPlan,
  terminalLiveFrameFallbackPlan,
  terminalPendingByteBufferPlan,
  terminalSurfaceInitErrorPlan,
  terminalSurfacePostInitPlan,
  terminalSurfaceRendererPlan,
  terminalSurfaceSessionPlan,
} from "./input_support.js";

export function createTerminalSurfaceRuntimeHelpers(runtime) {
  const { state, el } = runtime;

  function clearPendingTerminalBytes() {
    state.pendingTerminalByteChunks = [];
    state.pendingTerminalByteLength = 0;
  }

  function bufferTerminalBytes(bytes) {
    const isUint8Array = bytes instanceof Uint8Array;
    const plan = terminalPendingByteBufferPlan({ isUint8Array, byteLength: isUint8Array ? bytes.byteLength : 0, pendingByteLength: state.pendingTerminalByteLength, pendingChunkByteLengths: state.pendingTerminalByteChunks.map((chunk) => chunk?.byteLength || 0), maxPendingBytes: runtime.maxPendingTerminalBytes });
    if (!plan.accept) return false;
    const copy = new Uint8Array(bytes);
    state.pendingTerminalByteChunks.push(copy);
    state.pendingTerminalByteLength += copy.byteLength;
    for (let index = 0; index < plan.dropCount; index += 1) {
      const dropped = state.pendingTerminalByteChunks.shift();
      state.pendingTerminalByteLength -= dropped?.byteLength || 0;
    }
    runtime.setConnectionStatus(plan.status);
    return true;
  }

  function flushPendingTerminalBytes() {
    if (!state.terminal || !state.pendingTerminalByteChunks.length) {
      return false;
    }
    const chunks = state.pendingTerminalByteChunks;
    clearPendingTerminalBytes();
    for (const chunk of chunks) {
      runtime.feedTerminalBytes(chunk);
    }
    return true;
  }

  function setTerminalTextFallbackActive(active, options = {}) {
    const hasCurrentSession = Boolean(runtime.currentSession());
    const wasActive = state.terminalFallbackActive;
    const nextActive = Boolean(active && hasCurrentSession);
    const plan = terminalFallbackActivationPlan({ active, hasCurrentSession, wasActive, hasTerminal: Boolean(state.terminal), clearText: options.clearText !== false, nearBottom: nextActive && wasActive ? terminalFallbackIsNearBottom() : false });
    state.terminalFallbackActive = plan.terminalFallbackActive;
    el.terminalFallback.classList.toggle("hidden", plan.hidden);
    el.terminalFallback.setAttribute("aria-hidden", plan.ariaHidden);
    if (plan.updateAutoFollow) state.terminalFallbackAutoFollow = plan.autoFollow;
    if (plan.clearText) el.terminalFallback.textContent = "";
    if (plan.startSnapshotPolling) runtime.startSnapshotPolling();
    if (plan.focusTerminal) runtime.focusTerminalInputSurface({ onlyIfSurfaceFocused: true, preventScroll: true });
    if (plan.stopSnapshotPolling) runtime.stopSnapshotPolling();
    runtime.syncTerminalStatusStrip();
  }

  function terminalFallbackIsNearBottom() {
    const maxScrollTop = Math.max(0, el.terminalFallback.scrollHeight - el.terminalFallback.clientHeight);
    return maxScrollTop - el.terminalFallback.scrollTop < 48;
  }

  function updateTerminalFallbackText(text) {
    const previousScrollTop = el.terminalFallback.scrollTop;
    const nearBottom = state.terminalFallbackAutoFollow ? false : terminalFallbackIsNearBottom();
    const fallbackText = text || "";
    el.terminalFallback.textContent = fallbackText;
    const scrollPlan = terminalFallbackTextScrollPlan({ terminalFallbackAutoFollow: state.terminalFallbackAutoFollow, nearBottom, previousScrollTop, scrollHeight: el.terminalFallback.scrollHeight, clientHeight: el.terminalFallback.clientHeight });
    el.terminalFallback.scrollTop = scrollPlan.scrollTop;
    syncTerminalAccessibilityMirror(fallbackText);
  }

  function syncTerminalAccessibilityMirror(fallbackText = null) {
    const mirrorText = typeof fallbackText === "string" ? fallbackText : terminalMirrorTextFromRenderer();
    state.terminalMirrorText = mirrorText;
    if (el.terminalA11yMirror) {
      el.terminalA11yMirror.value = mirrorText;
    }
    if (runtime.terminalSupports("drainAccessibilityAnnouncements") && el.terminalAnnouncer) {
      const announcements = state.terminal.drainAccessibilityAnnouncements();
      if (Array.isArray(announcements) && announcements.length) {
        el.terminalAnnouncer.textContent = announcements.join("\n");
      }
    }
  }

  function terminalMirrorTextFromRenderer() {
    if (runtime.terminalSupports("screenReaderMirrorText")) {
      return state.terminal.screenReaderMirrorText() || "";
    }
    if (runtime.terminalSupports("accessibilityDomSnapshot")) {
      return state.terminal.accessibilityDomSnapshot()?.value || "";
    }
    return "";
  }

  function syncTerminalFallbackFromLiveFrame() {
    const canReadLiveText = state.terminalFallbackActive && state.terminal;
    const plan = terminalLiveFrameFallbackPlan({ terminalFallbackActive: state.terminalFallbackActive, hasTerminal: Boolean(state.terminal), liveText: canReadLiveText ? terminalMirrorTextFromRenderer() : "", existingFallbackText: el.terminalFallback.textContent });
    if (!plan.update) {
      return false;
    }
    updateTerminalFallbackText(plan.text);
    return true;
  }

  async function setupTerminalSurface() {
    runtime.stopSnapshotPolling();

    const sessionPlan = terminalSurfaceSessionPlan({ session: runtime.currentSession() });
    if (sessionPlan.type === "teardown_terminal") { runtime.teardownTerminal(); return; }

    const mod = await runtime.ensureFrankenTerm();
    const rendererPlan = terminalSurfaceRendererPlan({ hasRendererModule: Boolean(mod), hasTerminal: Boolean(state.terminal), terminalSessionId: state.terminalSessionId, sessionId: sessionPlan.sessionId, terminalFallbackActive: state.terminalFallbackActive });
    if (rendererPlan.type === "activate_snapshot_fallback") {
      return activateTerminalSurfaceFallback(rendererPlan, runtime);
    }
    if (rendererPlan.type === "reuse_terminal") {
      return reuseTerminalSurface(rendererPlan, runtime);
    }
    return initializeTerminalSurface(mod, sessionPlan.sessionId, rendererPlan, runtime);
  }

  return {
    clearPendingTerminalBytes,
    bufferTerminalBytes,
    flushPendingTerminalBytes,
    setTerminalTextFallbackActive,
    terminalFallbackIsNearBottom,
    updateTerminalFallbackText,
    syncTerminalAccessibilityMirror,
    syncTerminalFallbackFromLiveFrame,
    setupTerminalSurface,
  };
}

export async function activateTerminalSurfaceFallback(plan, runtime) {
  runtime.teardownTerminal();
  runtime.setTerminalTextFallbackActive(true, { clearText: plan.clearText });
  await runtime.refreshSnapshotFallback();
}

export function reuseTerminalSurface(plan, runtime) {
  runtime.el.terminalCanvas.classList.toggle("hidden", plan.terminalCanvasHidden);
  runtime.el.terminalFallback.classList.toggle("hidden", plan.terminalFallbackHidden);
  runtime.refreshTerminalSearch();
  runtime.syncTerminalAccessibilityMirror();
  runtime.syncTerminalTools();
  runtime.setLoadingState(plan.loadingVisible);
}

export async function initializeTerminalSurface(mod, sessionId, plan, runtime) {
  runtime.destroyTerminalInstance();
  runtime.setLoadingState(plan.loadingVisible, plan.loadingLabel);
  try {
    runtime.state.terminal = runtime.validateFrankenTermSurface(
      new mod.FrankenTermWeb(),
      runtime.requiredTerminalMethods,
      "terminal renderer",
    );
    runtime.state.terminalAcceptsBytes = false;
    runtime.state.surfaceInitInProgress += 1;
    try {
      await runtime.state.terminal.init(runtime.el.terminalCanvas, undefined);
    } finally {
      runtime.state.surfaceInitInProgress -= 1;
    }
    runtime.state.terminalAcceptsBytes = true;
  } catch (error) {
    await handleTerminalSurfaceInitError(error, runtime);
    return;
  }
  completeTerminalSurfaceInit(sessionId, runtime);
}

async function handleTerminalSurfaceInitError(error, runtime) {
  const plan = terminalSurfaceInitErrorPlan(error.message);
  runtime.destroyTerminalInstance();
  runtime.setTerminalTextFallbackActive(true, { clearText: plan.clearText });
  if (plan.refreshSnapshot) {
    await runtime.refreshSnapshotFallback();
  }
  runtime.setLoadingState(plan.loadingVisible);
  runtime.setUtilityStatus(plan.status, plan.statusError, plan.statusTimeoutMs);
}

function completeTerminalSurfaceInit(sessionId, runtime) {
  const plan = terminalSurfacePostInitPlan({
    sessionId,
    linkPolicySupported: runtime.terminalSupports("setLinkOpenPolicy"),
    accessibilitySupported: runtime.terminalSupports("setAccessibility"),
    reducedMotion: runtime.prefersReducedMotion(),
  });
  Object.assign(runtime.state, {
    terminalSessionId: plan.sessionId,
    terminalPaintVerified: plan.terminalPaintVerified,
    terminalFrameBytesSeen: plan.terminalFrameBytesSeen,
  });
  runtime.setTerminalTextFallbackActive(plan.terminalFallbackActive);
  if (plan.setLinkOpenPolicy) {
    runtime.state.terminal.setLinkOpenPolicy(runtime.frankenTermLinkPolicy());
  }
  if (plan.setAccessibility) {
    runtime.state.terminal.setAccessibility(plan.accessibility);
  }
  runtime.applyZoomToSurface(runtime.state.terminal);
  runtime.el.terminalCanvas.classList.toggle("hidden", plan.terminalCanvasHidden);
  if (plan.clearSelection) {
    runtime.clearTerminalSelection();
  }
  if (plan.refreshSearch) {
    runtime.refreshTerminalSearch();
  }
  if (plan.syncMirror) {
    runtime.syncTerminalAccessibilityMirror();
  }
  if (plan.syncTools) {
    runtime.syncTerminalTools();
  }
  runtime.measureAndResizeSurface(plan.resize.pushResize, plan.resize.force);
  if (plan.flushPendingBytes) {
    runtime.flushPendingTerminalBytes();
  }
  runtime.setLoadingState(plan.loadingVisible);
}
