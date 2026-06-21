import {
  applyWorkbenchWidgetResults,
  buildWorkbenchWidgetRequestPlan,
  shouldThrottleWorkbenchWidgets,
} from "./workbench_render.js";
import { normalizeWorkbenchWidgetResults } from "./contracts.js";

export function workbenchRefreshStartPlan(context = {}) {
  if (!context.hasSession || context.trogdorAtlasOpen) {
    return { type: "reset_and_render" };
  }
  // The panel is collapsed (it defaults closed on mobile): skip background
  // refreshes so we don't poll six sidecar endpoints into a hidden surface. A
  // forced refresh (the panel just opened) always loads.
  if (context.workbenchClosed && !context.force) {
    return { type: "ignore" };
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
  const requestSeqMatches = context.requestSeq === context.currentRequestSeq;
  return {
    stale: !requestSeqMatches ||
      context.selectedSessionId !== context.sessionId,
    clearLoading: requestSeqMatches,
  };
}

export async function runWorkbenchWidgetRefresh(options = {}, runtime) {
  const session = runtime.currentSession();
  const sessionId = session?.session_id;
  const startPlan = workbenchRefreshStartPlan({
    hasSession: Boolean(session),
    trogdorAtlasOpen: runtime.state.trogdorAtlasOpen,
    workbenchClosed: runtime.state.terminalWorkbenchOpen === false,
    force: Boolean(options.force),
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
  const results = normalizeWorkbenchWidgetResults(await fetchWorkbenchWidgetResults(requestPlan.paths, runtime));
  const stalePlan = workbenchRefreshStalePlan({
    requestSeq: startPlan.requestSeq,
    currentRequestSeq: runtime.state.workbenchWidgets.requestSeq,
    selectedSessionId: runtime.state.selectedSessionId,
    sessionId: startPlan.sessionId,
  });
  if (stalePlan.stale) {
    if (stalePlan.clearLoading) {
      runtime.state.workbenchWidgets.loading = false;
      runtime.renderWorkbenchWidgets();
    }
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
