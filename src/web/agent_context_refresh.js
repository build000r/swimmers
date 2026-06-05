import { normalizeSessionAgentContextResponse } from "./contracts.js";

export function agentContextRequestPath(sessionId) {
  return `/v1/sessions/${encodeURIComponent(sessionId)}/agent-context`;
}

export function agentContextRefreshStartPlan(context = {}) {
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
    loading: !context.silent || !context.hasCurrentPayload,
  };
}

export function agentContextRefreshStalePlan(context = {}) {
  return {
    stale: context.requestSeq !== context.currentRequestSeq ||
      context.selectedSessionId !== context.sessionId,
  };
}

export async function runAgentContextRefresh(options = {}, runtime) {
  const session = runtime.currentSession();
  if (!session || runtime.state.trogdorAtlasOpen) {
    runtime.state.agentContextLoading = false;
    runtime.renderTerminalWorkbench();
    return;
  }

  const sessionId = session.session_id;
  const hasCurrentPayload =
    runtime.state.agentContextSessionId === sessionId && Boolean(runtime.state.agentContextPayload);
  const now = runtime.now();
  const startPlan = agentContextRefreshStartPlan({
    hasSession: Boolean(session),
    trogdorAtlasOpen: runtime.state.trogdorAtlasOpen,
    throttled: Boolean(options.throttle) &&
      hasCurrentPayload &&
      now - runtime.state.agentContextLastLoadedAt < runtime.throttleMs,
    loadingBlocked: runtime.state.agentContextLoading && !options.force,
    sessionId,
    requestSeq: runtime.state.agentContextRequestSeq,
    silent: options.silent,
    hasCurrentPayload,
  });
  if (startPlan.type === "ignore") {
    return;
  }

  runtime.state.agentContextRequestSeq = startPlan.requestSeq;
  runtime.state.agentContextSessionId = startPlan.sessionId;
  runtime.state.agentContextError = "";
  runtime.state.agentContextLoading = startPlan.loading;
  runtime.renderTerminalWorkbench();

  try {
    const response = await runtime.apiFetch(agentContextRequestPath(startPlan.sessionId));
    const payload = await runtime.responseJson(response, normalizeSessionAgentContextResponse);
    if (agentContextRefreshStalePlan({
      requestSeq: startPlan.requestSeq,
      currentRequestSeq: runtime.state.agentContextRequestSeq,
      selectedSessionId: runtime.state.selectedSessionId,
      sessionId: startPlan.sessionId,
    }).stale) {
      return;
    }
    runtime.state.agentContextPayload = payload;
    runtime.state.agentContextError = "";
    runtime.state.agentContextLastLoadedAt = runtime.now();
  } catch (error) {
    if (agentContextRefreshStalePlan({
      requestSeq: startPlan.requestSeq,
      currentRequestSeq: runtime.state.agentContextRequestSeq,
      selectedSessionId: runtime.state.selectedSessionId,
      sessionId: startPlan.sessionId,
    }).stale) {
      return;
    }
    runtime.state.agentContextPayload = null;
    runtime.state.agentContextError = error?.message || "context unavailable";
  } finally {
    if (startPlan.requestSeq === runtime.state.agentContextRequestSeq) {
      runtime.state.agentContextLoading = false;
      runtime.renderTerminalWorkbench();
    }
  }
}
