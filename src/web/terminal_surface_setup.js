import {
  terminalSurfaceInitErrorPlan,
  terminalSurfacePostInitPlan,
} from "./input_support.js";

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
