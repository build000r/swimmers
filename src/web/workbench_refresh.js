import {
  applyWorkbenchWidgetResults,
  buildWorkbenchWidgetRequestPlan,
  shouldThrottleWorkbenchWidgets,
} from "./workbench_render.js";

export function workbenchRefreshStartPlan(context = {}) {
  if (!context.hasSession || context.trogdorAtlasOpen) {
    return { type: "reset_and_render" };
  }
  if (context.throttled || context.loadingBlocked) {
    return { type: "ignore" };
  }
  return {
    type: "start_refresh",
    sessionId: context.sessionId,
    requestSeq: Number(context.requestSeq || 0) + 1,
    loading: !context.silent,
  };
}

export function workbenchRefreshStalePlan(context = {}) {
  return {
    stale: context.requestSeq !== context.currentRequestSeq ||
      context.selectedSessionId !== context.sessionId,
  };
}

export async function runWorkbenchWidgetRefresh(options = {}, runtime) {
  const session = runtime.currentSession();
  const sessionId = session?.session_id;
  const startPlan = workbenchRefreshStartPlan({
    hasSession: Boolean(session),
    trogdorAtlasOpen: runtime.state.trogdorAtlasOpen,
    throttled: session ? shouldThrottleWorkbenchWidgets({
      options,
      widgets: runtime.state.workbenchWidgets,
      sessionId,
      throttleMs: runtime.throttleMs,
    }) : false,
    loadingBlocked: runtime.state.workbenchWidgets.loading && !options.force,
    sessionId,
    requestSeq: runtime.state.workbenchWidgets.requestSeq,
    silent: options.silent,
  });
  if (startPlan.type === "reset_and_render") {
    runtime.state.workbenchWidgets.loading = false;
    runtime.renderWorkbenchWidgets();
    return;
  }
  if (startPlan.type === "ignore") {
    return;
  }

  Object.assign(runtime.state.workbenchWidgets, {
    requestSeq: startPlan.requestSeq,
    sessionId: startPlan.sessionId,
    error: "",
    loading: startPlan.loading,
  });
  runtime.renderWorkbenchWidgets();

  const requestPlan = buildWorkbenchWidgetRequestPlan({
    sessionId: startPlan.sessionId,
    selectedTurnId: runtime.state.workbenchSelectedTurnId,
    widgets: runtime.state.workbenchWidgets,
    force: Boolean(options.force),
  });
  const results = await fetchWorkbenchWidgetResults(requestPlan.paths, runtime);
  const stalePlan = workbenchRefreshStalePlan({
    requestSeq: startPlan.requestSeq,
    currentRequestSeq: runtime.state.workbenchWidgets.requestSeq,
    selectedSessionId: runtime.state.selectedSessionId,
    sessionId: startPlan.sessionId,
  });
  if (stalePlan.stale) {
    return;
  }

  const applied = applyWorkbenchWidgetResults(runtime.state.workbenchWidgets, results, {
    canDeltaTranscript: requestPlan.canDeltaTranscript,
    requestedTurnId: requestPlan.requestedTurnId,
    selectedTurnId: runtime.state.workbenchSelectedTurnId,
  });
  runtime.state.workbenchSelectedTurnId = applied.selectedTurnId;
  runtime.state.workbenchWidgets.lastLoadedAt = runtime.now ? runtime.now() : Date.now();
  runtime.renderWorkbenchWidgets();
}

function fetchWorkbenchWidgetResults(paths, runtime) {
  return Promise.allSettled([
    runtime.apiMaybeFetch(paths.timeline).then(runtime.responseJsonOrNull),
    runtime.apiMaybeFetch(paths.skills).then(runtime.responseJsonOrNull),
    runtime.apiMaybeFetch(paths.paneTail).then(runtime.responseJsonOrNull),
    runtime.apiMaybeFetch(paths.transcript).then(runtime.responseJsonOrNull),
    runtime.apiMaybeFetch(paths.artifact).then(runtime.responseJsonOrNull),
    runtime.apiMaybeFetch(paths.gitDiff).then(runtime.responseJsonOrNull),
  ]).then(([timelineResult, skillsResult, tailResult, transcriptResult, artifactResult, diffResult]) => ({
    timelineResult,
    skillsResult,
    tailResult,
    transcriptResult,
    artifactResult,
    diffResult,
  }));
}
