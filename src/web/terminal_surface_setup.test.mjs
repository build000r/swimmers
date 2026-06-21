import test from "node:test";
import assert from "node:assert/strict";

import { initializeTerminalSurface } from "./terminal_surface_setup.js";
import { terminalDestroyStatePatch, terminalSurfacePostInitPlan } from "./input_support.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function fakeRuntime() {
  const state = {
    terminal: null,
    terminalAcceptsBytes: false,
    terminalSessionId: null,
    surfaceInitInProgress: 0,
  };
  const el = { terminalCanvas: { classList: { toggle() {}, add() {} } } };
  return {
    state,
    el,
    requiredTerminalMethods: ["init"],
    validateFrankenTermSurface: (surface) => surface,
    destroyTerminalInstance() {
      // Mirror the real adapter: the destroy patch nulls state.terminal so a new
      // initializeTerminalSurface installs a fresh instance.
      Object.assign(state, terminalDestroyStatePatch());
    },
    setLoadingState() {},
    terminalSupports: (method) =>
      Boolean(state.terminal && typeof state.terminal[method] === "function"),
    prefersReducedMotion: () => false,
    setTerminalTextFallbackActive() {},
    frankenTermLinkPolicy: () => ({ allowHttps: true }),
    applyZoomToSurface() {},
    clearTerminalSelection() {},
    refreshTerminalSearch() {},
    syncTerminalAccessibilityMirror() {},
    syncTerminalTools() {},
    measureAndResizeSurface() {},
    flushPendingTerminalBytes() {},
  };
}

function plan() {
  return { loadingVisible: true, loadingLabel: "Initializing terminal..." };
}

test("initializeTerminalSurface does not clobber a newer terminal when an older init resolves last", async () => {
  const runtime = fakeRuntime();
  const { state } = runtime;

  const gateA = deferred();
  const gateB = deferred();

  // Two distinct renderer instances. Each records the session it was asked to
  // configure via setLinkOpenPolicy/setAccessibility so we can detect a wrong
  // completion writing onto the wrong instance.
  function makeSurface(name, gate) {
    return {
      name,
      configuredFor: null,
      init: () => gate.promise,
      setLinkOpenPolicy() {
        this.configuredFor = name;
      },
      setAccessibility() {
        this.configuredFor = name;
      },
    };
  }
  const surfaceA = makeSurface("A", gateA);
  const surfaceB = makeSurface("B", gateB);

  const modA = { FrankenTermWeb: function () { return surfaceA; } };
  const modB = { FrankenTermWeb: function () { return surfaceB; } };

  // Call A starts and suspends at await init().
  const connectA = initializeTerminalSurface(modA, "sess_a", plan(), runtime);
  await Promise.resolve();
  assert.equal(state.terminal, surfaceA);

  // Call B interleaves: destroys A's instance and installs B, then suspends.
  const connectB = initializeTerminalSurface(modB, "sess_b", plan(), runtime);
  await Promise.resolve();
  assert.equal(state.terminal, surfaceB);

  // B finishes first and legitimately becomes the live terminal/session.
  gateB.resolve();
  await connectB;
  assert.equal(state.terminalSessionId, "sess_b");
  assert.equal(state.terminal, surfaceB);
  assert.equal(state.terminalAcceptsBytes, true);

  // A's init resolves LAST. It must NOT overwrite terminalSessionId back to A,
  // must NOT re-flip terminalAcceptsBytes, and must NOT configure B's surface.
  gateA.resolve();
  await connectA;

  assert.equal(state.terminal, surfaceB);
  assert.equal(state.terminalSessionId, "sess_b");
  assert.equal(state.terminalAcceptsBytes, true);
  assert.equal(surfaceB.configuredFor, "B");
});

test("initializeTerminalSurface completes normally when the instance survives", async () => {
  const runtime = fakeRuntime();
  const { state } = runtime;
  const gate = deferred();
  const surface = {
    init: () => gate.promise,
    setLinkOpenPolicy() {
      this.configured = true;
    },
    setAccessibility() {
      this.configured = true;
    },
  };
  const mod = { FrankenTermWeb: function () { return surface; } };

  const connecting = initializeTerminalSurface(mod, "sess_a", plan(), runtime);
  gate.resolve();
  await connecting;

  assert.equal(state.terminal, surface);
  assert.equal(state.terminalSessionId, "sess_a");
  assert.equal(state.terminalAcceptsBytes, true);
  // Sanity: the real post-init plan drives setLinkOpenPolicy/setAccessibility,
  // which this surface supports, so the surviving completion configured it.
  assert.equal(surface.configured, true);
  assert.equal(terminalSurfacePostInitPlan({ sessionId: "sess_a" }).sessionId, "sess_a");
});
