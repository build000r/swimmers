import test from "node:test";
import assert from "node:assert/strict";

import {
  assertFrankenTermModule,
  canvasHasVisiblePixels,
  frankenTermAssetSummary,
  isFrankenTermReentryError,
  surfaceBusy,
  surfaceSupports,
  validateFrankenTermSurface,
  withSurfaceOperation,
} from "./terminal_runtime.js";
import {
  createFrankenTermRuntimeAdapter,
  resolveStableFrankenTermCanvases,
} from "./terminal_surface_controller.js";

test("FrankenTerm module and surface validators preserve error messages", () => {
  const mod = { default() {}, FrankenTermWeb() {} };
  assert.equal(assertFrankenTermModule(mod), mod);
  assert.throws(
    () => assertFrankenTermModule({ FrankenTermWeb() {} }),
    /missing its wasm initializer/,
  );
  assert.throws(
    () => assertFrankenTermModule({ default() {} }),
    /missing FrankenTermWeb/,
  );

  const surface = { init() {}, render() {} };
  assert.equal(surfaceSupports(surface, "render"), true);
  assert.equal(validateFrankenTermSurface(surface, ["init", "render"], "HUD"), surface);
  assert.throws(
    () => validateFrankenTermSurface(surface, ["init", "resize", "feed"], "HUD"),
    /HUD missing methods: resize, feed/,
  );
});

test("FrankenTerm asset summary preserves js wasm font ordering and optional fields", () => {
  assert.equal(frankenTermAssetSummary(null), "");
  assert.equal(
    frankenTermAssetSummary({
      wasm: { checksum: "sha256:wasm", size_bytes: 12 },
      js: { checksum: "sha256:js" },
      font: { size_bytes: 34 },
      ignored: { checksum: "nope", size_bytes: 99 },
    }),
    "js sha256:js; wasm sha256:wasm 12b; font 34b",
  );
});

test("surface operation guard defers busy and records recursive renderer errors", () => {
  const busyState = {
    surfaceInitInProgress: 1,
    surfaceOperationDepth: 0,
    lastRendererDiagnosticError: "",
  };
  assert.deepEqual(withSurfaceOperation(busyState, "render", () => "unused"), { deferred: true });

  const readyState = {
    surfaceInitInProgress: 0,
    surfaceOperationDepth: 0,
    lastRendererDiagnosticError: "",
  };
  assert.deepEqual(withSurfaceOperation(readyState, "render", () => "ok"), {
    deferred: false,
    value: "ok",
  });
  assert.equal(readyState.surfaceOperationDepth, 0);
  assert.equal(surfaceBusy(readyState), false);

  const recursive = withSurfaceOperation(readyState, "render", () => {
    throw new Error("recursive use of an object");
  });
  assert.equal(recursive.deferred, true);
  assert.match(readyState.lastRendererDiagnosticError, /render: recursive use of an object/);
  assert.equal(isFrankenTermReentryError(recursive.error), true);

  assert.throws(
    () => withSurfaceOperation(readyState, "render", () => {
      throw new Error("ordinary failure");
    }),
    /ordinary failure/,
  );
  assert.equal(readyState.surfaceOperationDepth, 0);
});

test("canvas visible pixel probe preserves dimensions threshold and failure fallbacks", () => {
  assert.equal(canvasHasVisiblePixels(null, {}), false);
  assert.equal(canvasHasVisiblePixels({ width: 0, height: 10 }, {}), false);

  const calls = [];
  const documentRef = {
    createElement() {
      return {
        width: 0,
        height: 0,
        getContext() {
          return {
            drawImage(_canvas, _x, _y, width, height) {
              calls.push({ width, height });
            },
            getImageData() {
              return { data: new Uint8ClampedArray([0, 0, 0, 255, 33, 0, 0, 255]) };
            },
          };
        },
      };
    },
  };

  assert.equal(canvasHasVisiblePixels({ width: 500, height: 300 }, documentRef), true);
  assert.deepEqual(calls, [{ width: 180, height: 120 }]);

  const blankDocument = {
    createElement() {
      return {
        width: 0,
        height: 0,
        getContext() {
          return {
            drawImage() {},
            getImageData() {
              return { data: new Uint8ClampedArray([32, 32, 32, 255]) };
            },
          };
        },
      };
    },
  };
  assert.equal(canvasHasVisiblePixels({ width: 20, height: 10 }, blankDocument), false);

  const throwingDocument = {
    createElement() {
      return {
        width: 0,
        height: 0,
        getContext() {
          return {
            drawImage() {
              throw new Error("tainted");
            },
          };
        },
      };
    },
  };
  assert.equal(canvasHasVisiblePixels({ width: 20, height: 10 }, throwingDocument), false);
});

function classList() {
  const values = new Set();
  return {
    contains: (name) => values.has(name),
    add: (name) => values.add(name),
    remove: (name) => values.delete(name),
    toggle: (name, force) => {
      if (force) {
        values.add(name);
      } else {
        values.delete(name);
      }
      return Boolean(force);
    },
  };
}

function element(name) {
  return {
    name,
    textContent: "",
    value: "",
    scrollTop: 0,
    scrollHeight: 0,
    clientHeight: 0,
    style: {},
    classList: classList(),
    setAttribute(key, value) {
      this[key] = value;
    },
    getBoundingClientRect() {
      return { width: 1280, height: 720 };
    },
  };
}

function adapterState(overrides = {}) {
  return {
    terminal: null,
    hud: null,
    ws: null,
    selectedSessionId: "sess-1",
    terminalSessionId: null,
    terminalAcceptsBytes: true,
    terminalFallbackActive: false,
    terminalFallbackAutoFollow: true,
    terminalMirrorText: "",
    terminalPaintVerified: false,
    terminalFrameBytesSeen: 0,
    terminalPaintProbeTimer: null,
    pendingTerminalByteChunks: [],
    pendingTerminalByteLength: 0,
    surfaceInitInProgress: 0,
    surfaceOperationDepth: 0,
    resizeQueued: false,
    resizePushResize: false,
    resizeForce: false,
    renderQueued: false,
    renderRetryQueued: false,
    hudRenderQueued: false,
    currentCols: 80,
    currentRows: 24,
    searchQuery: "",
    connectionGeneration: 0,
    rendererDiagnosticSequence: 0,
    lastRendererDiagnosticError: "",
    trogdorAtlasOpen: false,
    selectionAnchor: null,
    selectionFocus: null,
    ...overrides,
  };
}

function baseAdapterRuntime(overrides = {}) {
  const calls = [];
  const state = overrides.state || adapterState();
  const el = {
    terminalCanvas: element("terminal-canvas"),
    hudCanvas: element("hud-canvas"),
    terminalFallback: element("terminal-fallback"),
    terminalA11yMirror: element("terminal-a11y"),
    terminalAnnouncer: element("terminal-announcer"),
    terminalStage: element("terminal-stage"),
    ...overrides.el,
  };
  return {
    calls,
    state,
    el,
    runtime: {
      state,
      el,
      boot: {
        franken_term_available: true,
        franken_term_js_url: "/assets/frankenterm/FrankenTerm.js",
        franken_term_wasm_url: "/assets/frankenterm/FrankenTerm_bg.wasm",
        franken_term_font_url: "",
        franken_term_asset_info: null,
        ...overrides.boot,
      },
      canvases: {
        terminalCanvas: el.terminalCanvas,
        hudCanvas: el.hudCanvas,
        ...overrides.canvases,
      },
      requiredTerminalMethods: ["init", "destroy", "feed", "render", "fitToContainer", "resize"],
      hudMethods: ["init", "destroy", "render", "fitToContainer", "resize", "applyPatchBatchFlat"],
      maxPendingTerminalBytes: 1024,
      assertFrankenTermModule,
      validateFrankenTermSurface,
      surfaceSupports: (surface, method) => typeof surface?.[method] === "function",
      runtimeSurfaceBusy: surfaceBusy,
      runSurfaceOperation: withSurfaceOperation,
      runTerminalSurfaceResize: () => calls.push(["runTerminalSurfaceResize"]),
      terminalDestroyStatePatch: () => ({
        terminal: null,
        terminalSessionId: null,
        terminalAcceptsBytes: true,
        selectionAnchor: null,
        selectionFocus: null,
      }),
      terminalPaintProbeSchedulePlan: () => ({ scheduleProbe: false }),
      terminalPaintVerificationPlan: () => ({ type: "ignore", done: true }),
      terminalPresentationPlan: () => ({
        terminalFocusMode: true,
        terminalStageActive: true,
        hudHidden: true,
        hudDisplay: "none",
        hudVisibility: "hidden",
        showTerminalCanvas: true,
        terminalCanvasHidden: false,
        terminalCanvasDisplay: "",
        terminalCanvasVisibility: "",
        terminalFallbackHidden: true,
      }),
      buildSurfaceFrame: () => ({ zones: [], masks: [], spans: [], cells: [] }),
      buildSurfaceModelFromState: () => ({}),
      currentSession: () => ({ session_id: "sess-1" }),
      operatorPressureSnapshot: () => null,
      sessionBurnt: () => false,
      normalizeSessionId: (value) => value || null,
      terminalSupports: (method) => Boolean(state.terminal && typeof state.terminal[method] === "function"),
      frankenTermLinkPolicy: () => ({ allowHttp: false, allowHttps: true }),
      clearReconnectTimer: () => calls.push(["clearReconnectTimer"]),
      clearHoveredLink: () => calls.push(["clearHoveredLink"]),
      flushEncodedInputBytes: () => calls.push(["flushEncodedInputBytes"]),
      startSnapshotPolling: () => calls.push(["startSnapshotPolling"]),
      stopSnapshotPolling: () => calls.push(["stopSnapshotPolling"]),
      focusTerminalInputSurface: () => calls.push(["focusTerminalInputSurface"]),
      syncTerminalTools: () => calls.push(["syncTerminalTools"]),
      syncTerminalStatusStrip: () => calls.push(["syncTerminalStatusStrip"]),
      applyZoomToSurface: () => calls.push(["applyZoomToSurface"]),
      clearTerminalSelection: () => calls.push(["clearTerminalSelection"]),
      setLoadingState: (...args) => calls.push(["setLoadingState", ...args]),
      setUtilityStatus: (...args) => calls.push(["setUtilityStatus", ...args]),
      setConnectionStatus: (...args) => calls.push(["setConnectionStatus", ...args]),
      renderTrogdorSurface: () => calls.push(["renderTrogdorSurface"]),
      advanceTrogdorReaderProgressForCurrentHover: () => calls.push(["advanceTrogdorReaderProgressForCurrentHover"]),
      syncTerminalInputDock: () => calls.push(["syncTerminalInputDock"]),
      syncTrogdorBackButton: () => calls.push(["syncTrogdorBackButton"]),
      syncTerminalWorkbench: () => calls.push(["syncTerminalWorkbench"]),
      refreshTerminalSearch: () => calls.push(["refreshTerminalSearch"]),
      drainTerminalLinkClicks: () => calls.push(["drainTerminalLinkClicks"]),
      refreshSnapshotFallback: async () => false,
      canvasHasVisiblePixels: () => false,
      windowRef: { location: { href: "http://swimmers.test/app" } },
      documentRef: { body: element("body") },
      URLImpl: URL,
      WebSocketClass: { OPEN: 1 },
      Uint8ArrayClass: Uint8Array,
      importModule: async () => {
        throw new Error("unexpected import");
      },
      requestAnimationFrameRef: (callback) => callback(),
      setTimeoutRef: (callback) => {
        callback();
        return 1;
      },
      clearTimeoutRef: () => {},
      prefersReducedMotion: () => true,
      devicePixelRatio: () => 2,
      isoTimestamp: () => "2026-06-05T00:00:00.000Z",
      now: () => 42,
      formatFrankenTermAssetSummary: frankenTermAssetSummary,
      ...overrides.runtime,
    },
  };
}

test("FrankenTerm adapter requires stable terminal and HUD canvases", () => {
  const terminalCanvas = element("terminal");
  const hudCanvas = element("hud");

  assert.deepEqual(
    resolveStableFrankenTermCanvases({
      terminalCanvas: { current: terminalCanvas },
      hudCanvas: { current: hudCanvas },
    }),
    { terminalCanvas, hudCanvas },
  );
  assert.throws(
    () => createFrankenTermRuntimeAdapter({
      state: {},
      boot: {},
      el: {},
      canvases: { terminalCanvas: null, hudCanvas },
    }),
    /stable terminalCanvas/,
  );
  assert.throws(
    () => createFrankenTermRuntimeAdapter({
      state: {},
      boot: {},
      el: {},
      canvases: { terminalCanvas, hudCanvas: null },
    }),
    /stable hudCanvas/,
  );
});

test("FrankenTerm adapter owns boot URL dynamic import and wasm initialization", async () => {
  const { calls, runtime, state } = baseAdapterRuntime({
    boot: {
      franken_term_font_url: "/assets/frankenterm/pragmasevka-nf-subset.woff2",
      franken_term_asset_info: {
        js: { checksum: "sha256:js" },
        wasm: { checksum: "sha256:wasm", size_bytes: 12 },
      },
    },
  });
  const mod = {
    default: async (wasmUrl) => calls.push(["wasm", String(wasmUrl)]),
    FrankenTermWeb: function FrankenTermWeb() {},
  };
  runtime.documentRef = {
    fonts: {
      load: async (font) => calls.push(["font", font]),
    },
  };
  runtime.importModule = async (url) => {
    calls.push(["import", url]);
    return mod;
  };

  const adapter = createFrankenTermRuntimeAdapter(runtime);

  assert.equal(await adapter.ensureFrankenTerm(), mod);
  assert.equal(await adapter.ensureFrankenTerm(), mod);
  assert.deepEqual(calls, [
    ["font", '12px "Pragmasevka NF"'],
    ["import", "/assets/frankenterm/FrankenTerm.js"],
    ["wasm", "http://swimmers.test/assets/frankenterm/FrankenTerm_bg.wasm"],
  ]);
  assert.equal(state.frankenModule, mod);
  assert.equal(state.frankenLoadError, "");
  assert.equal(state.frankenAssetSummary, "js sha256:js; wasm sha256:wasm 12b");
});

test("FrankenTerm adapter initializes terminal surface and flushes buffered bytes", async () => {
  const state = adapterState({ terminalAcceptsBytes: false });
  const { calls, runtime, el } = baseAdapterRuntime({ state });

  class FakeSurface {
    async init(canvas) {
      calls.push(["init", canvas.name]);
    }

    destroy() {
      calls.push(["destroy"]);
    }

    feed(bytes) {
      calls.push(["feed", Array.from(bytes)]);
    }

    render() {
      calls.push(["render"]);
    }

    fitToContainer() {
      return { cols: 80, rows: 24 };
    }

    resize(cols, rows) {
      calls.push(["resize", cols, rows]);
    }

    setLinkOpenPolicy(policy) {
      calls.push(["setLinkOpenPolicy", policy]);
    }

    setAccessibility(accessibility) {
      calls.push(["setAccessibility", accessibility]);
    }

    screenReaderMirrorText() {
      return "ready";
    }
  }

  runtime.importModule = async () => ({
    default: async () => calls.push(["wasm"]),
    FrankenTermWeb: FakeSurface,
  });

  const adapter = createFrankenTermRuntimeAdapter(runtime);
  await adapter.setupTerminalSurface();

  assert.equal(state.terminalAcceptsBytes, true);
  assert.equal(state.terminalSessionId, "sess-1");
  assert.equal(el.terminalA11yMirror.value, "ready");
  assert.deepEqual(calls, [
    ["stopSnapshotPolling"],
    ["wasm"],
    ["clearHoveredLink"],
    ["setLoadingState", true, "Initializing terminal..."],
    ["init", "terminal-canvas"],
    ["stopSnapshotPolling"],
    ["syncTerminalStatusStrip"],
    ["setLinkOpenPolicy", { allowHttp: false, allowHttps: true }],
    ["setAccessibility", { reducedMotion: true, screenReader: true }],
    ["applyZoomToSurface"],
    ["clearTerminalSelection"],
    ["refreshTerminalSearch"],
    ["syncTerminalTools"],
    ["runTerminalSurfaceResize"],
    ["setLoadingState", false],
  ]);

  calls.length = 0;
  assert.equal(adapter.bufferTerminalBytes(new Uint8Array([1, 2, 3])), true);
  assert.equal(state.pendingTerminalByteLength, 3);
  assert.equal(adapter.flushPendingTerminalBytes(), true);
  assert.equal(state.pendingTerminalByteLength, 0);
  assert.deepEqual(state.pendingTerminalByteChunks, []);
  assert.deepEqual(calls, [
    ["setConnectionStatus", "buffering terminal; renderer attaching"],
    ["feed", [1, 2, 3]],
    ["flushEncodedInputBytes"],
    ["drainTerminalLinkClicks"],
    ["render"],
  ]);
});

test("FrankenTerm adapter activates snapshot fallback when renderer assets are unavailable", async () => {
  const { calls, runtime, state, el } = baseAdapterRuntime({
    boot: { franken_term_available: false },
  });
  let adapter = null;
  runtime.importModule = async () => {
    throw new Error("asset import should not run");
  };
  runtime.refreshSnapshotFallback = async () => {
    calls.push([
      "refreshSnapshotFallback",
      state.terminalFallbackActive,
      el.terminalFallback["aria-hidden"],
    ]);
    adapter.updateTerminalFallbackText("snapshot prompt\n$ cargo test");
    return true;
  };
  adapter = createFrankenTermRuntimeAdapter(runtime);

  assert.equal(await adapter.ensureFrankenTerm(), null);
  await adapter.setupTerminalSurface();

  assert.equal(state.terminalFallbackActive, true);
  assert.equal(el.terminalFallback["aria-hidden"], "false");
  assert.equal(el.terminalFallback.textContent, "snapshot prompt\n$ cargo test");
  assert.equal(el.terminalA11yMirror.value, "snapshot prompt\n$ cargo test");
  assert.deepEqual(
    calls.find((call) => call[0] === "refreshSnapshotFallback"),
    ["refreshSnapshotFallback", true, "false"],
  );
  assert.equal(calls.some((call) => call[0] === "import"), false);
});

test("FrankenTerm adapter records module validation errors and allows load retry", async () => {
  const { calls, runtime, state } = baseAdapterRuntime();
  runtime.importModule = async () => ({ default: async () => calls.push(["wasm"]) });
  const adapter = createFrankenTermRuntimeAdapter(runtime);

  await assert.rejects(
    () => adapter.ensureFrankenTerm(),
    /FrankenTerm module is missing FrankenTermWeb/,
  );
  assert.equal(state.frankenInit, null);
  assert.equal(state.frankenModule, null);
  assert.match(state.frankenLoadError, /missing FrankenTermWeb/);

  const mod = {
    default: async () => calls.push(["wasm-retry"]),
    FrankenTermWeb: function FrankenTermWeb() {},
  };
  runtime.importModule = async () => mod;
  assert.equal(await adapter.ensureFrankenTerm(), mod);
  assert.equal(state.frankenModule, mod);
  assert.equal(state.frankenLoadError, "");
  assert.deepEqual(calls, [["wasm-retry"]]);
});

test("FrankenTerm adapter falls back when terminal surface init fails", async () => {
  const { calls, runtime, state, el } = baseAdapterRuntime();

  class FailingSurface {
    async init() {
      throw new Error("init exploded");
    }

    destroy() {
      calls.push(["destroy"]);
    }

    feed() {}

    render() {}

    fitToContainer() {
      return { cols: 80, rows: 24 };
    }

    resize() {}
  }

  runtime.importModule = async () => ({
    default: async () => calls.push(["wasm"]),
    FrankenTermWeb: FailingSurface,
  });
  runtime.refreshSnapshotFallback = async () => {
    calls.push(["refreshSnapshotFallback", state.terminalFallbackActive]);
    return false;
  };
  const adapter = createFrankenTermRuntimeAdapter(runtime);

  await adapter.setupTerminalSurface();

  assert.equal(state.terminal, null);
  assert.equal(state.terminalFallbackActive, true);
  assert.equal(el.terminalFallback["aria-hidden"], "false");
  assert.deepEqual(
    calls.find((call) => call[0] === "refreshSnapshotFallback"),
    ["refreshSnapshotFallback", true],
  );
  assert.deepEqual(
    calls.find((call) => call[0] === "setUtilityStatus"),
    [
      "setUtilityStatus",
      "Live terminal renderer unavailable: init exploded",
      true,
      3600,
    ],
  );
  assert.deepEqual(calls.at(-2), ["setLoadingState", false]);
});

test("FrankenTerm adapter reuses same-session terminal without reinitializing", async () => {
  const { calls, runtime, state, el } = baseAdapterRuntime({
    state: adapterState({
      frankenModule: {
        default: async () => calls.push(["wasm"]),
        FrankenTermWeb: function FrankenTermWeb() {},
      },
      terminalSessionId: "sess-1",
    }),
  });
  state.frankenInit = Promise.resolve(state.frankenModule);
  state.terminal = {
    destroy() {
      calls.push(["destroy"]);
    },
    init() {
      calls.push(["init"]);
    },
    render() {
      calls.push(["render"]);
    },
    screenReaderMirrorText() {
      return "same-session";
    },
  };
  el.terminalCanvas.classList.add("hidden");
  el.terminalFallback.classList.add("hidden");
  const adapter = createFrankenTermRuntimeAdapter(runtime);

  await adapter.setupTerminalSurface();

  assert.equal(state.terminalSessionId, "sess-1");
  assert.equal(state.terminalMirrorText, "same-session");
  assert.equal(el.terminalA11yMirror.value, "same-session");
  assert.equal(el.terminalCanvas.classList.contains("hidden"), false);
  assert.equal(el.terminalFallback.classList.contains("hidden"), true);
  assert.deepEqual(calls, [
    ["stopSnapshotPolling"],
    ["refreshTerminalSearch"],
    ["syncTerminalTools"],
    ["setLoadingState", false],
  ]);
});

test("FrankenTerm adapter defers resize and render work while a surface is busy", () => {
  const timers = [];
  const { calls, runtime, state } = baseAdapterRuntime({
    state: adapterState({ surfaceInitInProgress: 1 }),
    runtime: {
      setTimeoutRef: (callback) => {
        timers.push(callback);
        return timers.length;
      },
    },
  });
  state.terminal = {
    render() {
      calls.push(["render"]);
    },
  };
  const adapter = createFrankenTermRuntimeAdapter(runtime);

  adapter.queueMeasureAndResizeSurface(true, true);
  assert.equal(state.resizeQueued, true);
  assert.equal(state.resizePushResize, true);
  assert.equal(state.resizeForce, true);
  timers.shift()();
  assert.equal(state.resizeQueued, true);
  assert.equal(calls.some((call) => call[0] === "runTerminalSurfaceResize"), false);

  state.surfaceInitInProgress = 0;
  adapter.queueMeasureAndResizeSurface(false, false);
  timers.shift()();
  assert.equal(state.resizeQueued, false);
  assert.equal(state.resizePushResize, false);
  assert.equal(state.resizeForce, false);
  assert.deepEqual(calls, [["runTerminalSurfaceResize"]]);

  calls.length = 0;
  state.surfaceInitInProgress = 1;
  adapter.scheduleRender();
  assert.equal(state.renderQueued, false);
  assert.equal(state.renderRetryQueued, true);
  timers.shift()();
  assert.equal(state.renderRetryQueued, false);
  assert.deepEqual(calls, []);

  state.surfaceInitInProgress = 0;
  adapter.scheduleRender();
  assert.deepEqual(calls, [["render"]]);
});

test("FrankenTerm adapter syncs accessibility mirror and announcements", () => {
  const { runtime, state, el } = baseAdapterRuntime();
  state.terminal = {
    screenReaderMirrorText() {
      return "visible terminal text";
    },
    drainAccessibilityAnnouncements() {
      return ["build started", "build passed"];
    },
  };
  const adapter = createFrankenTermRuntimeAdapter(runtime);

  adapter.syncTerminalAccessibilityMirror();

  assert.equal(state.terminalMirrorText, "visible terminal text");
  assert.equal(el.terminalA11yMirror.value, "visible terminal text");
  assert.equal(el.terminalAnnouncer.textContent, "build started\nbuild passed");
});

test("FrankenTerm adapter preserves fallback snapshot ordering during paint verification", async () => {
  const { calls, runtime, state, el } = baseAdapterRuntime({
    state: adapterState({
      terminal: {
        render() {},
      },
    }),
  });
  let phase = 0;
  let adapter = null;
  runtime.terminalPaintVerificationPlan = (context) => {
    calls.push([
      "paintPlan",
      phase,
      context.terminalFallbackActive,
      context.afterSnapshotRefresh ?? false,
      context.hasSnapshotText ?? null,
    ]);
    phase += 1;
    if (phase === 1) {
      return { type: "check_canvas", done: false };
    }
    if (phase === 2) {
      return { type: "refresh_snapshot", done: false };
    }
    if (phase === 3) {
      return {
        type: "activate_fallback",
        done: true,
        fallbackActive: true,
        clearText: false,
      };
    }
    return { type: "ignore", done: true };
  };
  runtime.refreshSnapshotFallback = async () => {
    calls.push(["refreshSnapshotFallback", state.terminalFallbackActive]);
    adapter.updateTerminalFallbackText("snapshot after blank canvas");
    return true;
  };
  adapter = createFrankenTermRuntimeAdapter(runtime);

  await adapter.verifyTerminalPaintOrFallback();

  assert.equal(state.terminalFallbackActive, true);
  assert.equal(el.terminalFallback.textContent, "snapshot after blank canvas");
  assert.equal(el.terminalA11yMirror.value, "snapshot after blank canvas");
  assert.deepEqual(calls, [
    ["paintPlan", 0, false, false, null],
    ["paintPlan", 1, false, false, null],
    ["refreshSnapshotFallback", false],
    ["paintPlan", 2, false, true, null],
    ["startSnapshotPolling"],
    ["focusTerminalInputSurface"],
    ["syncTerminalStatusStrip"],
    ["syncTerminalInputDock"],
    ["syncTrogdorBackButton"],
    ["syncTerminalWorkbench"],
  ]);
});
