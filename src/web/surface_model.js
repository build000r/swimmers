import {
  trogdorSurfaceSessionTrogdorState,
} from "./trogdor_logic.js";

const REPO_NAMESPACE_PARTS = new Set(["opensource", "clients", "personal", "work", "projects"]);
const ACTIONABLE_CUE_KINDS = new Set([
  "awaiting_user",
  "commit_ready",
  "validation_missing_after_edit",
  "dirty_check_missing",
]);

export function relativeCwd(cwd) {
  if (!cwd) return "unknown cwd";
  const parts = cwd.split("/").filter(Boolean);
  if (!parts.length) return cwd;
  return parts.slice(-2).join("/");
}

function normalizeCwd(cwd) {
  const trimmed = String(cwd || "").trim();
  if (trimmed === "/") return "/";
  return trimmed.replace(/\/+$/g, "");
}

export function canonicalCwd(session) {
  return session?.environment?.canonical_cwd || session?.cwd || "";
}

function cwdPathParts(cwd) {
  return normalizeCwd(cwd)
    .split("/")
    .map((part) => part.trim())
    .filter(Boolean)
    .filter((part) => part !== "." && part !== "..");
}

function repoNamespacePart(part) {
  return REPO_NAMESPACE_PARTS.has(String(part || "").toLowerCase());
}

function repoComponentIndex(parts) {
  const reposIndex = parts.findIndex((part) => part.toLowerCase() === "repos");
  if (reposIndex < 0) return -1;
  const first = reposIndex + 1;
  const firstPart = parts[first];
  if (!firstPart) return -1;
  return repoNamespacePart(firstPart) && parts[first + 1] ? first + 1 : first;
}

function rootedPath(parts, absolute) {
  const joined = parts.join("/");
  return absolute && joined ? `/${joined}` : joined;
}

export function repoKeyForCwd(cwd) {
  const normalized = normalizeCwd(cwd);
  const parts = cwdPathParts(normalized);
  const repoIndex = repoComponentIndex(parts);
  if (repoIndex < 0) return normalized;
  return rootedPath(parts.slice(0, repoIndex + 1), normalized.startsWith("/"));
}

function cwdTailLabel(cwd) {
  return normalizeCwd(cwd)
    .split("/")
    .filter(Boolean)
    .at(-1) || "";
}

export function repoLabelForKey(repoKey) {
  const parts = cwdPathParts(repoKey);
  const repoIndex = repoComponentIndex(parts);
  if (repoIndex >= 0) {
    const repo = parts[repoIndex];
    const namespace = parts[repoIndex - 1];
    return namespace && repoNamespacePart(namespace) ? `${namespace}/${repo}` : repo;
  }
  return cwdTailLabel(repoKey) || String(repoKey || "").trim();
}

export function sessionHostLabel(session) {
  const environment = session?.environment || {};
  if (String(environment.scope || "local").trim().toLowerCase() !== "remote") {
    return "";
  }
  return firstNonEmpty(
    environment.display_host,
    environment.target_label,
    environment.target_id,
  ) || "";
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

export function normalizeSessionGroupMode(mode) {
  return String(mode || "").trim().toLowerCase() === "project" ? "project" : "flat";
}

export function sessionReadiness(session) {
  const state = String(session?.state || "").toLowerCase();
  const rest = String(session?.rest_state || "").toLowerCase();
  const cues = Array.isArray(session?.action_cues) ? session.action_cues : [];
  const hasActionableCue = cues.some((cue) => (
    ACTIONABLE_CUE_KINDS.has(String(cue?.kind || "").trim().toLowerCase())
  ));
  if (state === "attention" || session?.commit_candidate || hasActionableCue) {
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
  if (String(environment.scope || "local").trim().toLowerCase() !== "remote") {
    return { key: "local", label: "local" };
  }
  const key = firstNonEmpty(
    environment.target_id,
    environment.display_host,
    environment.target_label,
  ) || "remote";
  const label = firstNonEmpty(
    environment.display_host,
    environment.target_label,
    environment.target_id,
  ) || "remote";
  return { key, label };
}

function firstNonEmpty(...values) {
  for (const value of values) {
    const trimmed = String(value ?? "").trim();
    if (trimmed) return trimmed;
  }
  return "";
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
    advisory_count: 0,
  };
  bucket.count += 1;
  if (session.isStale || session.transportLabel !== "healthy") bucket.degraded_count += 1;
  if (session.isStale) bucket.stale_count += 1;
  if (session.readinessKey === "needs_attention") bucket.attention_count += 1;
  if (session.commitCandidate) bucket.commit_ready_count += 1;
  bucket.advisory_count += session.advisoryBadges?.length || 0;
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

function inboxChipLabel(bucket) {
  if (bucket?.kind === "readiness" && bucket?.key === "needs_attention") {
    return appendAdvisoryCount(`inbox ${bucket.count}`, bucket);
  }
  return appendAdvisoryCount(
    `${bucket?.kind || ""} ${bucket?.label || bucket?.key || ""} ${bucket?.count ?? 0}`.trim(),
    bucket,
  );
}

function appendAdvisoryCount(label, bucket) {
  const count = Number(bucket?.advisory_count || 0);
  return count > 0 ? `${label} · ext ${count}` : label;
}

export function availableFleetFilter(lens, filter) {
  const active = normalizeFleetFilter(filter);
  if (!active.kind) {
    return active;
  }
  return bucketFor(lens, active.kind, active.key) ? active : { kind: "", key: "" };
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
      label: inboxChipLabel(bucket || { kind: active.kind, key: active.key, label: active.key, count: 0 }),
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
      label: inboxChipLabel(bucket),
      kind: bucket.kind,
      key: bucket.key,
      active: false,
    });
  }
  return chips.slice(0, 5);
}

function attentionInboxReasonRank(session) {
  switch (String(session?.operatorPressure?.reason_kind || "").toLowerCase()) {
    case "awaiting_user":
      return 5;
    case "commit_ready":
      return 4;
    case "validation_missing_after_edit":
    case "dirty_check_missing":
    case "needs_input":
      return 3;
    case "error":
      return 2;
    case "sleeping":
    case "untrusted_state":
    case "stale":
    case "transport":
      return 1;
    default:
      return 0;
  }
}

function attentionInboxPressure(session) {
  const score = Number(session?.operatorPressure?.score);
  return Number.isFinite(score) ? Math.max(1, Math.min(99, score)) : 1;
}

function attentionInboxDegraded(session) {
  return Boolean(session?.isStale || session?.transportKey !== "healthy");
}

function attentionInboxTime(session) {
  const time = Date.parse(session?.lastActivityAt || "");
  return Number.isFinite(time) ? time : 0;
}

export function buildAttentionInbox(sessions) {
  return (Array.isArray(sessions) ? sessions : [])
    .filter((session) => session.readinessKey === "needs_attention")
    .slice()
    .sort((left, right) => (
      Number(attentionInboxDegraded(left)) - Number(attentionInboxDegraded(right))
      || attentionInboxPressure(right) - attentionInboxPressure(left)
      || attentionInboxReasonRank(right) - attentionInboxReasonRank(left)
      || attentionInboxTime(right) - attentionInboxTime(left)
      || String(left.sessionId || "").localeCompare(String(right.sessionId || ""))
    ));
}

function readinessBucketCount(lens, key) {
  return bucketFor(lens, "readiness", key)?.count || 0;
}

function groupHostLabel(session) {
  return session.targetLabel || "local";
}

function groupHostSummary(sessions) {
  const counts = new Map();
  for (const session of sessions) {
    const label = groupHostLabel(session);
    counts.set(label, (counts.get(label) || 0) + 1);
  }
  return Array.from(counts.entries())
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .map(([label, count]) => (count > 1 ? `${label} x${count}` : label))
    .join(" + ");
}

export function buildSessionRailRows(sessions, mode = "flat") {
  const items = Array.isArray(sessions) ? sessions : [];
  if (normalizeSessionGroupMode(mode) !== "project") {
    return items.map((session) => ({ type: "session", session }));
  }

  const groups = new Map();
  for (const session of items) {
    const key = session.repoKey || session.canonicalCwd || session.fullCwd || session.cwdLabel || session.name;
    const existing = groups.get(key) || {
      key,
      label: session.repoLabel || relativeCwd(key),
      sessions: [],
    };
    existing.sessions.push(session);
    groups.set(key, existing);
  }

  return Array.from(groups.values())
    .map((group) => ({
      ...group,
      hostSummary: groupHostSummary(group.sessions),
    }))
    .sort((left, right) => (
      right.sessions.length - left.sessions.length
      || left.label.localeCompare(right.label)
      || left.key.localeCompare(right.key)
    ))
    .flatMap((group) => group.sessions
      .slice()
      .sort((left, right) => (
        left.targetLabel.localeCompare(right.targetLabel)
        || left.name.localeCompare(right.name)
        || left.sessionId.localeCompare(right.sessionId)
      ))
      .map((session, index) => ({
        type: "session",
        session,
        group: {
          key: group.key,
          label: group.label,
          count: group.sessions.length,
          hostSummary: group.hostSummary,
          first: index === 0,
        },
      })));
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

export function advisoryBadgeText(badge) {
  const label = String(badge?.label || badge?.source || "advisory").trim() || "advisory";
  const value = String(badge?.value || "").trim();
  const status = String(badge?.status || "external").trim() || "external";
  const stale = badge?.stale ? " stale" : "";
  return value ? `${label}: ${value} (${status}${stale})` : `${label} (${status}${stale})`;
}

export function advisorySummaryLabel(badges) {
  const items = Array.isArray(badges) ? badges : [];
  if (!items.length) {
    return "";
  }
  return items.map(advisoryBadgeText).join(" · ");
}

export function sessionAdvisoryBadges(session) {
  const advisories = Array.isArray(session?.environment?.advisory)
    ? session.environment.advisory
    : [];
  return advisories
    .map((advisory) => ({
      source: String(advisory?.source || "").trim(),
      label: String(advisory?.label || "").trim(),
      value: String(advisory?.value || "").trim(),
      status: String(advisory?.status || "external").trim() || "external",
      stale: advisory?.stale !== false,
    }))
    .filter((advisory) => advisory.source && advisory.label && advisory.value);
}

export function surfaceSession(session, {
  detail = false,
  operatorPressure = null,
  sessionBurnt = () => false,
  dismissedClawgs = {},
  readProgress = {},
} = {}) {
  const target = sessionTarget(session);
  const environment = session?.environment || {};
  const sessionCanonicalCwd = canonicalCwd(session);
  const fallbackRepoKey = repoKeyForCwd(sessionCanonicalCwd)
    || session.tmux_name
    || session.session_id
    || "";
  const remoteEnvironment = String(environment.scope || "local").trim().toLowerCase() === "remote";
  const launchCwd = remoteEnvironment ? String(environment.local_cwd || "") : String(session?.cwd || "");
  const launchTarget = remoteEnvironment && launchCwd ? firstNonEmpty(environment.target_id) : "";
  const readiness = sessionReadiness(session);
  const stateKey = String(session.state || "unknown").toLowerCase();
  const transportKey = String(session.transport_health || "unknown").toLowerCase();
  const advisoryBadges = sessionAdvisoryBadges(session);
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
    canonicalCwd: sessionCanonicalCwd,
    launchCwd,
    launchTarget,
    thoughtLabel: detail ? session.thought || "No thought snapshot yet." : summarizeThought(session),
    clawgText: session.thought || "",
    thoughtUpdatedAt: session.thought_updated_at || "",
    objectiveChangedAt: session.objective_changed_at || "",
    contextLabel: `${session.token_count ?? 0} / ${session.context_limit ?? 0}`,
    skillLabel: session.last_skill || "none",
    lastActivityAt: session.last_activity_at || "",
    activityLabel: formatTime(session.last_activity_at),
    commandLabel: session.current_command || "idle",
    attachedLabel: String(session.attached_clients ?? 0),
    commitCandidate: Boolean(session.commit_candidate),
    actionCues: Array.isArray(session.action_cues) ? session.action_cues : [],
    operatorPressure: operatorPressure?.pressure || null,
    batchSendSessionIds: Array.isArray(operatorPressure?.batch_send_session_ids)
      ? operatorPressure.batch_send_session_ids
      : [],
    repoKey: operatorPressure?.repo_key || fallbackRepoKey,
    repoLabel: operatorPressure?.repo_label || repoLabelForKey(fallbackRepoKey) || relativeCwd(sessionCanonicalCwd),
    targetKey: target.key,
    targetLabel: target.label,
    advisoryBadges,
    advisoryLabel: advisorySummaryLabel(advisoryBadges),
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
  const sessionGroupMode = normalizeSessionGroupMode(state.sessionGroupMode);
  const fleetLens = buildFleetLensSummary(allSurfaceSessions);
  const fleetFilter = availableFleetFilter(fleetLens, state.fleetFilter);
  const surfaceSessions = allSurfaceSessions.filter((session) => sessionMatchesFleetFilter(session, fleetFilter));
  const filteredFleetLens = buildFleetLensSummary(surfaceSessions);
  const attentionInbox = buildAttentionInbox(surfaceSessions);
  const selectedSessionVisible = selectedSession
    ? surfaceSessions.some((session) => session.sessionId === selectedSession.session_id)
    : false;
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
    sessionGroupMode,
    sessionRailRows: buildSessionRailRows(surfaceSessions, sessionGroupMode),
    allSessionCount: allSurfaceSessions.length,
    attentionInbox,
    attentionInboxCount: readinessBucketCount(filteredFleetLens, "needs_attention"),
    fleetFilter,
    fleetLens,
    filteredFleetLens,
    fleetChips: fleetLensChips(fleetLens, fleetFilter),
    selectedSessionId: state.selectedSessionId,
    publishedSessionId: normalizeSessionId(state.publishedSelection?.session_id),
    publishedAtLabel: formatTime(state.publishedSelection?.published_at),
    currentSession: selectedSessionVisible
      ? surfaceSession(selectedSession, surfaceOptions(selectedSession, { detail: true }))
      : null,
  };
}
