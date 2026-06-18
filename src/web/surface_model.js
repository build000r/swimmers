import {
  trogdorSurfaceSessionTrogdorState,
} from "./trogdor_logic.js";

export function relativeCwd(cwd) {
  if (!cwd) return "unknown cwd";
  const parts = cwd.split("/").filter(Boolean);
  if (!parts.length) return cwd;
  return parts.slice(-2).join("/");
}

export function canonicalCwd(session) {
  return session?.environment?.canonical_cwd || session?.cwd || "";
}

export function sessionHostLabel(session) {
  const environment = session?.environment || {};
  if (String(environment.scope || "local").toLowerCase() !== "remote") {
    return "";
  }
  return environment.display_host || environment.target_label || environment.target_id || "";
}

export function sessionCwdLabel(session) {
  const label = relativeCwd(canonicalCwd(session));
  const host = sessionHostLabel(session);
  return host ? `${label} @ ${host}` : label;
}

export function formatTime(raw) {
  if (!raw) return "unknown";
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) {
    return raw;
  }
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function summarizeThought(session) {
  const thought = (session?.thought || "").trim();
  if (!thought) {
    return "No thought snapshot yet.";
  }
  return thought.length > 110 ? `${thought.slice(0, 107)}...` : thought;
}

export function sessionStateConfidence(session) {
  return String(session?.state_evidence?.confidence || "low").toLowerCase();
}

export function sessionStateObserved(session) {
  return Boolean(session?.state_evidence?.observed_at);
}

export function sessionStateDisplay(session) {
  const label = String(session?.state || "unknown");
  if (sessionStateConfidence(session) !== "high" || !sessionStateObserved(session)) {
    return `${label}?`;
  }
  return label;
}

export function sessionStateTrustLabel(session) {
  const evidence = session?.state_evidence || {};
  const confidence = sessionStateConfidence(session);
  const freshness = sessionStateObserved(session) ? "observed" : "unobserved";
  const cause = String(evidence.cause || "unknown");
  return `${confidence} ${freshness} ${cause}`;
}

export function surfaceSession(session, {
  detail = false,
  operatorPressure = null,
  sessionBurnt = () => false,
  dismissedClawgs = {},
  readProgress = {},
} = {}) {
  const surface = {
    sessionId: session.session_id,
    name: session.tmux_name || session.session_id,
    state: String(session.state || "unknown"),
    displayState: sessionStateDisplay(session),
    stateTrustLabel: sessionStateTrustLabel(session),
    stateConfidence: sessionStateConfidence(session),
    stateObserved: sessionStateObserved(session),
    restLabel: String(session.rest_state || "unknown"),
    transportLabel: String(session.transport_health || "unknown"),
    toolLabel: session.tool || "shell",
    cwdLabel: sessionCwdLabel(session),
    fullCwd: session.cwd || "",
    canonicalCwd: canonicalCwd(session),
    thoughtLabel: detail ? session.thought || "No thought snapshot yet." : summarizeThought(session),
    clawgText: session.thought || "",
    thoughtUpdatedAt: session.thought_updated_at || "",
    objectiveChangedAt: session.objective_changed_at || "",
    contextLabel: `${session.token_count ?? 0} / ${session.context_limit ?? 0}`,
    skillLabel: session.last_skill || "none",
    activityLabel: formatTime(session.last_activity_at),
    commandLabel: session.current_command || "idle",
    attachedLabel: String(session.attached_clients ?? 0),
    commitCandidate: Boolean(session.commit_candidate),
    actionCues: Array.isArray(session.action_cues) ? session.action_cues : [],
    operatorPressure: operatorPressure?.pressure || null,
    batchSendSessionIds: Array.isArray(operatorPressure?.batch_send_session_ids)
      ? operatorPressure.batch_send_session_ids
      : [],
    repoKey: operatorPressure?.repo_key || canonicalCwd(session),
    repoLabel: operatorPressure?.repo_label || relativeCwd(canonicalCwd(session)),
    isStale: Boolean(session.is_stale),
  };
  Object.assign(surface, trogdorSurfaceSessionTrogdorState(surface, {
    burnt: sessionBurnt(surface),
    dismissedClawgs,
    readProgress,
  }));
  return surface;
}

export function buildSurfaceModel({
  state,
  boot,
  currentSession = () => null,
  operatorPressureSnapshot = () => null,
  sessionBurnt = () => false,
  normalizeSessionId = (sessionId) => sessionId || null,
  now = () => 0,
  websocketOpen = 1,
} = {}) {
  const selectedSession = currentSession();
  const surfaceOptions = (session, extra = {}) => ({
    operatorPressure: operatorPressureSnapshot(session.session_id),
    sessionBurnt,
    dismissedClawgs: state.trogdorDismissedClawgs,
    readProgress: state.trogdorReadProgress,
    ...extra,
  });
  const surfaceSessions = state.sessions.map((session) => surfaceSession(session, surfaceOptions(session)));
  const terminalReady = Boolean(state.terminal && state.ws && state.ws.readyState === websocketOpen);
  return {
    cols: state.currentCols,
    rows: state.currentRows,
    focusLayout: Boolean(boot.focus_layout && state.followPublishedSelection),
    followPublishedSelection: state.followPublishedSelection,
    connectionLabel: state.connectionLabel,
    connectionMuted: state.connectionMuted,
    modeLabel: state.modeLabel,
    modeMuted: state.modeMuted,
    searchLabel: state.searchLabel,
    searchMuted: state.searchMuted,
    utilityLabel: state.utilityLabel,
    utilityMuted: state.utilityMuted,
    searchQuery: state.searchQuery,
    selectMode: state.selectMode,
    readOnly: state.readOnly,
    frankenTermAvailable: boot.franken_term_available,
    terminalReady,
    snapshotFallback: !boot.franken_term_available,
    activeSheet: state.activeSheet,
    hoveredLinkUrl: state.hoveredLinkUrl,
    hoveredTrogdorSessionId: state.hoveredTrogdorSessionId,
    trogdorAtlasOpen: state.trogdorAtlasOpen,
    trogdorWpm: state.trogdorWpm,
    trogdorReading: state.trogdorReading,
    trogdorReaderStartIndex: state.trogdorReaderStartIndex,
    trogdorReaderElapsedMs: state.hoveredTrogdorSessionId
      ? Math.max(0, now() - state.trogdorReaderStartedAt)
      : 0,
    sessions: surfaceSessions,
    selectedSessionId: state.selectedSessionId,
    publishedSessionId: normalizeSessionId(state.publishedSelection?.session_id),
    publishedAtLabel: formatTime(state.publishedSelection?.published_at),
    currentSession: selectedSession
      ? surfaceSession(selectedSession, surfaceOptions(selectedSession, { detail: true }))
      : null,
  };
}
