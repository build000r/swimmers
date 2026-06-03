import { terminalResizeGeometryPlan } from "./input_support.js";

export function terminalResizeStartPlan(context = {}) {
  if (!context.hasReferenceSurface) {
    return { type: "ignore" };
  }
  if (context.surfaceBusy) {
    return { type: "queue" };
  }
  return { type: "measure" };
}

export function runTerminalSurfaceResize(options = {}, runtime) {
  const referenceSurface = runtime.state.terminal || runtime.state.hud;
  const startPlan = terminalResizeStartPlan({
    hasReferenceSurface: Boolean(referenceSurface),
    surfaceBusy: runtime.surfaceBusy(),
  });
  if (startPlan.type === "ignore") {
    return;
  }
  if (startPlan.type === "queue") {
    runtime.queueMeasureAndResizeSurface(options.pushResize, options.force);
    return;
  }

  runtime.state.resizeQueued = false;
  runtime.state.resizePushResize = false;
  runtime.state.resizeForce = false;

  const rect = runtime.el.terminalStage.getBoundingClientRect();
  const dpr = runtime.devicePixelRatio();
  const fit = runtime.withSurfaceOperation("fitToContainer", () =>
    referenceSurface.fitToContainer(rect.width, rect.height, dpr),
  );
  if (fit.deferred) {
    runtime.queueMeasureAndResizeSurface(options.pushResize, options.force);
    return;
  }

  const resizePlan = terminalResizeGeometryPlan({
    cols: fit.value?.cols,
    rows: fit.value?.rows,
    currentCols: runtime.state.currentCols,
    currentRows: runtime.state.currentRows,
    force: options.force,
    pushResize: options.pushResize,
    hasTerminal: Boolean(runtime.state.terminal),
  });
  if (!resizePlan.shouldResize) {
    return;
  }

  const resized = runtime.withSurfaceOperation("resize", () => {
    if (runtime.state.hud) {
      runtime.state.hud.resize(resizePlan.cols, resizePlan.rows);
    }
    if (runtime.state.terminal) {
      runtime.state.terminal.resize(resizePlan.cols, resizePlan.rows);
    }
  });
  if (resized.deferred) {
    runtime.queueMeasureAndResizeSurface(options.pushResize, options.force);
    return;
  }

  runtime.state.currentCols = resizePlan.cols;
  runtime.state.currentRows = resizePlan.rows;
  runtime.renderHudSurface();
  runtime.scheduleRender();

  if (resizePlan.sendResize) {
    runtime.sendResize();
  }
  if (resizePlan.captureDiagnostic) {
    runtime.captureTerminalRendererDiagnostic(resizePlan.diagnosticReason);
  }
}
