import test from "node:test";
import assert from "node:assert/strict";

import {
  runTerminalSurfaceResize,
  terminalResizeStartPlan,
} from "./terminal_resize.js";

function buildRuntime(overrides = {}) {
  const calls = [];
  const terminal = Object.prototype.hasOwnProperty.call(overrides, "terminal")
    ? overrides.terminal
    : {
      fitToContainer(width, height, dpr) {
        calls.push(["terminal.fitToContainer", width, height, dpr]);
        return { cols: 100, rows: 30 };
      },
      resize(cols, rows) {
        calls.push(["terminal.resize", cols, rows]);
      },
    };
  const hud = Object.prototype.hasOwnProperty.call(overrides, "hud")
    ? overrides.hud
    : null;
  const state = {
    terminal,
    hud,
    currentCols: 80,
    currentRows: 24,
    resizeQueued: true,
    resizePushResize: true,
    resizeForce: true,
    ...overrides.state,
  };
  const deferredOperations = [...(overrides.deferredOperations || [])];
  const runtime = {
    state,
    el: {
      terminalStage: {
        getBoundingClientRect() {
          calls.push(["getBoundingClientRect"]);
          return overrides.rect || { width: 1280, height: 720 };
        },
      },
    },
    devicePixelRatio: () => overrides.dpr ?? 2,
    surfaceBusy: () => Boolean(overrides.surfaceBusy),
    queueMeasureAndResizeSurface: (pushResize, force) =>
      calls.push(["queueMeasureAndResizeSurface", pushResize, force]),
    withSurfaceOperation: (name, operation) => {
      calls.push(["withSurfaceOperation", name]);
      const deferredIndex = deferredOperations.indexOf(name);
      if (deferredIndex >= 0) {
        deferredOperations.splice(deferredIndex, 1);
        return { deferred: true };
      }
      return { deferred: false, value: operation() };
    },
    renderHudSurface: () => calls.push(["renderHudSurface"]),
    scheduleRender: () => calls.push(["scheduleRender"]),
    sendResize: () => calls.push(["sendResize", state.currentCols, state.currentRows]),
    captureTerminalRendererDiagnostic: (reason) =>
      calls.push(["captureTerminalRendererDiagnostic", reason]),
  };
  return { calls, runtime };
}

test("terminalResizeStartPlan preserves no-surface and busy gates", () => {
  assert.deepEqual(terminalResizeStartPlan({ hasReferenceSurface: false }), { type: "ignore" });
  assert.deepEqual(terminalResizeStartPlan({
    hasReferenceSurface: true,
    surfaceBusy: true,
  }), { type: "queue" });
  assert.deepEqual(terminalResizeStartPlan({
    hasReferenceSurface: true,
    surfaceBusy: false,
  }), { type: "measure" });
});

test("runTerminalSurfaceResize ignores missing reference surfaces", () => {
  const { calls, runtime } = buildRuntime({ terminal: null, hud: null });

  runTerminalSurfaceResize({ pushResize: true, force: true }, runtime);

  assert.deepEqual(calls, []);
  assert.equal(runtime.state.resizeQueued, true);
});

test("runTerminalSurfaceResize queues while surfaces are busy", () => {
  const { calls, runtime } = buildRuntime({ surfaceBusy: true });

  runTerminalSurfaceResize({ pushResize: true, force: false }, runtime);

  assert.deepEqual(calls, [["queueMeasureAndResizeSurface", true, false]]);
  assert.equal(runtime.state.resizeQueued, true);
});

test("runTerminalSurfaceResize preserves fit, resize, send, and diagnostic ordering", () => {
  const { calls, runtime } = buildRuntime();
  runtime.state.hud = {
    resize(cols, rows) {
      calls.push(["hud.resize", cols, rows]);
    },
  };
  runtime.state.terminal = {
    fitToContainer(width, height, dpr) {
      calls.push(["terminal.fitToContainer", width, height, dpr]);
      return { cols: 100.9, rows: 30.2 };
    },
    resize(cols, rows) {
      calls.push(["terminal.resize", cols, rows]);
    },
  };

  runTerminalSurfaceResize({ pushResize: true, force: false }, runtime);

  assert.equal(runtime.state.resizeQueued, false);
  assert.equal(runtime.state.resizePushResize, false);
  assert.equal(runtime.state.resizeForce, false);
  assert.equal(runtime.state.currentCols, 100);
  assert.equal(runtime.state.currentRows, 30);
  assert.deepEqual(calls, [
    ["getBoundingClientRect"],
    ["withSurfaceOperation", "fitToContainer"],
    ["terminal.fitToContainer", 1280, 720, 2],
    ["withSurfaceOperation", "resize"],
    ["hud.resize", 100, 30],
    ["terminal.resize", 100, 30],
    ["renderHudSurface"],
    ["scheduleRender"],
    ["sendResize", 100, 30],
    ["captureTerminalRendererDiagnostic", "resize"],
  ]);
});

test("runTerminalSurfaceResize preserves no-resize behavior after measurement", () => {
  const { calls, runtime } = buildRuntime({
    terminal: {
      fitToContainer() {
        calls.push(["terminal.fitToContainer"]);
        return { cols: 80, rows: 24 };
      },
      resize() {
        calls.push(["terminal.resize"]);
      },
    },
  });

  runTerminalSurfaceResize({ pushResize: true, force: false }, runtime);

  assert.equal(runtime.state.resizeQueued, false);
  assert.deepEqual(calls, [
    ["getBoundingClientRect"],
    ["withSurfaceOperation", "fitToContainer"],
    ["terminal.fitToContainer"],
  ]);
});

test("runTerminalSurfaceResize requeues deferred fit and resize operations", () => {
  const fit = buildRuntime({ deferredOperations: ["fitToContainer"] });
  runTerminalSurfaceResize({ pushResize: true, force: true }, fit.runtime);
  assert.deepEqual(fit.calls, [
    ["getBoundingClientRect"],
    ["withSurfaceOperation", "fitToContainer"],
    ["queueMeasureAndResizeSurface", true, true],
  ]);

  const resize = buildRuntime({ deferredOperations: ["resize"] });
  runTerminalSurfaceResize({ pushResize: false, force: true }, resize.runtime);
  assert.deepEqual(resize.calls, [
    ["getBoundingClientRect"],
    ["withSurfaceOperation", "fitToContainer"],
    ["terminal.fitToContainer", 1280, 720, 2],
    ["withSurfaceOperation", "resize"],
    ["queueMeasureAndResizeSurface", false, true],
  ]);
});
