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

export function normalizeFleetFilter(filter) {
  const kind = String(filter?.kind || "").trim().toLowerCase();
  const key = String(filter?.key || "").trim();
  if (!kind || !key) {
    return { kind: "", key: "" };
  }
  return { kind, key };
}

export function sessionReadiness(session) {
  const state = String(session?.state || "").toLowerCase();
  const rest = String(session?.rest_state || "").toLowerCase();
  const cues = Array.isArray(session?.action_cues) ? session.action_cues : [];
  if (state === "attention" || session?.commit_candidate || cues.length > 0) {
    return { key: "needs_attention", label: "needs attention" };
  }
  if (state === "busy") {
    return { key: "working", label: "working" };
  }
  if (rest === "sleeping" || rest === "deep_sleep") {
    return { key: "sleeping", label: "sleeping" };
  }
  return { key: "quiet", label: "quiet" };
}

function sessionTarget(session) {
  const environment = session?.environment || {};
  if (String(environment.scope || "local").toLowerCase() !== "remote") {
    return { key: "local", label: "local" };
  }
  const key = environment.target_id || environment.display_host || environment.target_label || "remote";
  const label = environment.display_host || environment.target_label || environment.target_id || "remote";
  return { key: String(key), label: String(label) };
}

function addFleetBucket(buckets, kind, key, label, session) {
  const bucketKey = `${kind}\0${key}`;
  const bucket = buckets.get(bucketKey) || {
    kind,
    key,
    label,
    count: 0,
    degraded_count: 0,
    stale_count: 0,
    attention_count: 0,
    commit_ready_count: 0,
  };
  bucket.count += 1;
  if (session.isStale || session.transportLabel !== "healthy") bucket.degraded_count += 1;
  if (session.isStale) bucket.stale_count += 1;
  if (session.readinessKey === "needs_attention") bucket.attention_count += 1;
  if (session.commitCandidate) bucket.commit_ready_count += 1;
  buckets.set(bucketKey, bucket);
}

export function buildFleetLensSummary(sessions) {
  const buckets = new Map();
  for (const session of Array.isArray(sessions) ? sessions : []) {
    addFleetBucket(buckets, "target", session.targetKey, session.targetLabel, session);
    addFleetBucket(buckets, "repo", session.repoKey, session.repoLabel, session);
    addFleetBucket(buckets, "state", session.stateKey, session.state, session);
    addFleetBucket(buckets, "readiness", session.readinessKey, session.readinessLabel, session);
    addFleetBucket(buckets, "transport", session.transportKey, session.transportLabel, session);
  }
  const order = new Map([
    ["target", 0],
    ["repo", 1],
    ["state", 2],
    ["readiness", 3],
    ["transport", 4],
  ]);
  return {
    total_sessions: Array.isArray(sessions) ? sessions.length : 0,
    buckets: Array.from(buckets.values()).sort((left, right) => (
      (order.get(left.kind) ?? 99) - (order.get(right.kind) ?? 99)
      || right.count - left.count
      || right.degraded_count - left.degraded_count
      || left.label.localeCompare(right.label)
      || left.key.localeCompare(right.key)
    )),
  };
}

export function sessionMatchesFleetFilter(session, filter) {
  const active = normalizeFleetFilter(filter);
  if (!active.kind) return true;
  switch (active.kind) {
    case "target":
      return session.targetKey === active.key;
    case "repo":
      return session.repoKey === active.key;
    case "state":
      return session.stateKey === active.key;
    case "readiness":
      return session.readinessKey === active.key;
    case "transport":
      return session.transportKey === active.key;
    default:
      return true;
  }
}

function bucketFor(lens, kind, key = "") {
  const buckets = Array.isArray(lens?.buckets) ? lens.buckets : [];
  return buckets.find((bucket) => bucket.kind === kind && (!key || bucket.key === key)) || null;
}

function firstNonHealthyTransport(lens) {
  const buckets = Array.isArray(lens?.buckets) ? lens.buckets : [];
  return buckets.find((bucket) => bucket.kind === "transport" && bucket.key !== "healthy") || null;
}

export function fleetLensChips(lens, filter) {
  const active = normalizeFleetFilter(filter);
  const total = Number(lens?.total_sessions || 0);
  const chips = [{
    label: `all ${total}`,
    kind: "",
    key: "",
    active: !active.kind,
  }];
  if (active.kind) {
    const bucket = bucketFor(lens, active.kind, active.key);
    chips.push({
      label: `${active.kind} ${bucket?.label || active.key} ${bucket?.count ?? 0}`,
      kind: active.kind,
      key: active.key,
      active: true,
    });
    return chips;
  }
  const target = bucketFor(lens, "target");
  const repo = bucketFor(lens, "repo");
  const needs = bucketFor(lens, "readiness", "needs_attention");
  const degraded = firstNonHealthyTransport(lens);
  for (const bucket of [target, repo, needs, degraded].filter(Boolean)) {
    chips.push({
      label: `${bucket.kind} ${bucket.label} ${bucket.count}`,
      kind: bucket.kind,
      key: bucket.key,
      active: false,
    });
  }
  return chips.slice(0, 5);
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
  const target = sessionTarget(session);
  const readiness = sessionReadiness(session);
  const stateKey = String(session.state || "unknown").toLowerCase();
  const transportKey = String(session.transport_health || "unknown").toLowerCase();
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
    transportKey,
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
    targetKey: target.key,
    targetLabel: target.label,
    stateKey,
    readinessKey: readiness.key,
    readinessLabel: readiness.label,
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
  const allSurfaceSessions = state.sessions.map((session) => surfaceSession(session, surfaceOptions(session)));
  const fleetFilter = normalizeFleetFilter(state.fleetFilter);
  const fleetLens = buildFleetLensSummary(allSurfaceSessions);
  const surfaceSessions = allSurfaceSessions.filter((session) => sessionMatchesFleetFilter(session, fleetFilter));
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
    allSessionCount: allSurfaceSessions.length,
    fleetFilter,
    fleetLens,
    fleetChips: fleetLensChips(fleetLens, fleetFilter),
    selectedSessionId: state.selectedSessionId,
    publishedSessionId: normalizeSessionId(state.publishedSelection?.session_id),
    publishedAtLabel: formatTime(state.publishedSelection?.published_at),
    currentSession: selectedSession
      ? surfaceSession(selectedSession, surfaceOptions(selectedSession, { detail: true }))
      : null,
  };
}
