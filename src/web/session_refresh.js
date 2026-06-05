import {
  normalizePublishedSelectionResponse,
  normalizeSessionListResponse,
} from "./contracts.js";

export function sessionRefreshRequestPlan(followPublishedSelection) {
  return {
    sessionsPath: "/v1/sessions",
    operatorPressurePath: "/v1/operator-pressure",
    healthPath: "/health",
    selectionPath: followPublishedSelection ? "/v1/selection" : null,
  };
}

export function sessionRefreshSelectionPlan(context = {}) {
  if (context.hasPublishedResponse) {
    return {
      publishedSelection: context.publishedSelection ?? null,
      persistSelectedSession: context.publishedSessionId && context.publishedSessionExists
        ? context.publishedSessionId
        : null,
    };
  }

  const selectedExists = Boolean(context.selectedSessionId) && context.selectedSessionExists;
  return {
    publishedSelection: null,
    ...(selectedExists ? {} : {
      persistSelectedSession: context.trogdorAtlasOpen ? null : context.fallbackSessionId ?? null,
    }),
  };
}

export function sessionRefreshSuccessStatusPlan(context = {}) {
  const waiting = Boolean(context.followPublishedSelection && !context.selectedSessionId);
  return {
    connection: {
      label: waiting ? "waiting" : context.selectedSessionId ? "live" : "idle",
      muted: waiting,
    },
    mode: {
      label: context.readOnly ? "observer" : "operator",
      muted: !context.token,
    },
  };
}

export function sessionRefreshErrorPlan(error) {
  const authRequired = error?.status === 401 || error?.status === 403;
  return {
    connection: {
      label: authRequired ? "auth required" : "backend unavailable",
      muted: true,
    },
    mode: {
      label: authRequired ? "token needed" : "offline",
      muted: !authRequired,
    },
  };
}

export function sessionDisplayName(session) {
  return String(session?.tmux_name || session?.name || session?.session_id || "session");
}

export function conciseHealthDetail(value) {
  const text = String(value || "").trim();
  if (!text) {
    return "";
  }
  return text.length > 64 ? `${text.slice(0, 61)}...` : text;
}

export function backendHealthWarningText(health) {
  if (!health || typeof health !== "object") {
    return "";
  }
  const persistence = health.persistence || {};
  if (!persistence.available) {
    return "persistence unavailable";
  }
  if (!persistence.ok) {
    const operation = persistence.last_failed_operation || "write";
    const detail = conciseHealthDetail(persistence.last_error);
    return `persistence degraded: ${operation}${detail ? `: ${detail}` : ""}`;
  }
  const thought = health.thought_bridge || {};
  const status = String(thought.status || "").toLowerCase();
  if (!status || status === "healthy") {
    return "";
  }
  if (status === "degraded") {
    const detail = conciseHealthDetail(thought.last_backend_error || thought.last_error);
    return `thought bridge degraded${detail ? `: ${detail}` : ""}`;
  }
  if (status === "unhealthy") {
    const detail = conciseHealthDetail(thought.shutdown_reason || thought.last_error);
    return `thought bridge unhealthy${detail ? `: ${detail}` : ""}`;
  }
  return `thought bridge ${status}`;
}

export async function runSessionRefresh(runtime) {
  try {
    const requestPlan = sessionRefreshRequestPlan(runtime.state.followPublishedSelection);
    const publishedRequest = requestPlan.selectionPath
      ? runtime.apiFetch(requestPlan.selectionPath)
      : Promise.resolve(null);
    const [response, pressureResponse, healthResponse, publishedResponse] = await Promise.all([
      runtime.apiFetch(requestPlan.sessionsPath),
      runtime.apiMaybeFetch(requestPlan.operatorPressurePath),
      runtime.apiMaybeFetch(requestPlan.healthPath),
      publishedRequest,
    ]);
    const payload = normalizeSessionListResponse(await response.json());
    const pressurePayload = await runtime.responseJsonOrNull(pressureResponse);
    const healthPayload = await runtime.responseJsonOrNull(healthResponse);
    runtime.state.sessions = Array.isArray(payload.sessions) ? payload.sessions : [];
    runtime.applyOperatorPressure(pressurePayload);
    runtime.applyBackendHealth(healthPayload);
    runtime.syncTrogdorCueTransitions();
    await applySessionRefreshSelection(publishedResponse, runtime);
    await runSessionRefreshSuccessSideEffects(runtime);
  } catch (error) {
    applySessionRefreshError(error, runtime);
  }
}

async function applySessionRefreshSelection(publishedResponse, runtime) {
  const publishedSelection = publishedResponse
    ? normalizePublishedSelectionResponse(await publishedResponse.json())
    : null;
  const publishedSessionId = publishedResponse
    ? runtime.normalizeSessionId(publishedSelection?.session_id)
    : null;
  const plan = sessionRefreshSelectionPlan({
    hasPublishedResponse: Boolean(publishedResponse),
    publishedSelection,
    publishedSessionId,
    publishedSessionExists: publishedSessionId ? runtime.sessionExists(publishedSessionId) : false,
    selectedSessionId: runtime.state.selectedSessionId,
    selectedSessionExists: runtime.sessionExists(runtime.state.selectedSessionId),
    trogdorAtlasOpen: runtime.state.trogdorAtlasOpen,
    fallbackSessionId: runtime.state.sessions[0]?.session_id ?? null,
  });
  runtime.state.publishedSelection = plan.publishedSelection;
  if (Object.prototype.hasOwnProperty.call(plan, "persistSelectedSession")) {
    runtime.persistSelectedSession(plan.persistSelectedSession);
  }
}

async function runSessionRefreshSuccessSideEffects(runtime) {
  await runtime.setupHudSurface();
  runtime.renderHudSurface();
  runtime.syncTerminalTools();
  await runtime.connectSelectedSession();
  void runtime.refreshAgentContextForSelectedSession({ throttle: true, silent: true });
  void runtime.refreshWorkbenchWidgetsForSelectedSession({ throttle: true, silent: true });
  const statusPlan = sessionRefreshSuccessStatusPlan({
    followPublishedSelection: runtime.state.followPublishedSelection,
    selectedSessionId: runtime.state.selectedSessionId,
    readOnly: runtime.state.readOnly,
    token: runtime.state.token,
  });
  runtime.setConnectionStatus(statusPlan.connection.label, statusPlan.connection.muted);
  runtime.setModeStatus(statusPlan.mode.label, statusPlan.mode.muted);
}

function applySessionRefreshError(error, runtime) {
  runtime.state.sessions = [];
  runtime.state.operatorPressureBySession = new Map();
  runtime.state.backendHealth = null;
  runtime.state.publishedSelection = null;
  runtime.persistSelectedSession(null);
  runtime.resetAgentContextForSession(null);
  runtime.resetWorkbenchWidgetsForSession(null);
  runtime.renderHudSurface();
  const statusPlan = sessionRefreshErrorPlan(error);
  runtime.setConnectionStatus(statusPlan.connection.label, statusPlan.connection.muted);
  runtime.setModeStatus(statusPlan.mode.label, statusPlan.mode.muted);
}
