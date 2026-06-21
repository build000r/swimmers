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
const BUILT_IN_FLEET_PRESETS = [
  { id: "all", label: "All environments", source: "builtin", matchers: [{ type: "all" }] },
  { id: "local", label: "Local", source: "builtin", matchers: [{ type: "target_id", id: "local" }] },
  { id: "remote-api", label: "Remote API", source: "builtin", matchers: [{ type: "target_kind", kind: "swimmers_api" }] },
  { id: "ssh-handoff", label: "SSH handoff", source: "builtin", matchers: [{ type: "target_kind", kind: "ssh_only" }] },
  { id: "current-repo", label: "Current repo", source: "builtin", matchers: [{ type: "current_repo" }] },
  { id: "needs-attention", label: "Needs attention", source: "builtin", matchers: [{ type: "needs_attention" }] },
  { id: "degraded", label: "Degraded", source: "builtin", matchers: [{ type: "degraded" }] },
];

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

function normalizePresetId(id) {
  return String(id || "").trim().toLowerCase();
}

function normalizePresetMatcher(matcher = {}) {
  const type = String(matcher?.type || "").trim().toLowerCase();
  return {
    ...matcher,
    type,
    kind: String(matcher?.kind || "").trim().toLowerCase(),
    key: String(matcher?.key || "").trim(),
    id: String(matcher?.id || "").trim(),
  };
}

export function buildFleetLensPresets(presets = []) {
  const seen = new Set();
  const merged = [];
  for (const preset of [...BUILT_IN_FLEET_PRESETS, ...(Array.isArray(presets) ? presets : [])]) {
    const id = normalizePresetId(preset?.id);
    const label = String(preset?.label || id).trim();
    const matchers = Array.isArray(preset?.matchers)
      ? preset.matchers.map(normalizePresetMatcher).filter((matcher) => matcher.type)
      : [];
    if (!id || !label || !matchers.length || seen.has(id)) continue;
    seen.add(id);
    merged.push({
      id,
      label,
      source: String(preset?.source || "").trim() || "builtin",
      matchers,
    });
  }
  return merged;
}

function currentRepoKeyForPreset(selectedSession, allSurfaceSessions) {
  const selectedId = selectedSession?.session_id || selectedSession?.sessionId || "";
  const selected = selectedId
    ? allSurfaceSessions.find((session) => session.sessionId === selectedId)
    : null;
  return selected?.repoKey || allSurfaceSessions[0]?.repoKey || "";
}

function resolveFleetPreset(preset, selectedSession, allSurfaceSessions) {
  if (!preset) return null;
  const currentRepoKey = currentRepoKeyForPreset(selectedSession, allSurfaceSessions);
  const matchers = preset.matchers.map((matcher) => (
    matcher.type === "current_repo"
      ? { type: "repo", key: currentRepoKey }
      : matcher
  ));
  if (matchers.some((matcher) => matcher.type === "repo" && !matcher.key)) {
    return null;
  }
  return { ...preset, matchers };
}

function sessionMatchesPresetMatcher(session, matcher) {
  switch (matcher.type) {
    case "all":
      return true;
    case "fleet_bucket":
      return sessionMatchesFleetFilter(session, { kind: matcher.kind, key: matcher.key });
    case "target_id":
      return session.targetKey === matcher.id;
    case "target_kind":
      return String(session.targetKind || "").trim().toLowerCase() === matcher.kind;
    case "repo":
      return session.repoKey === matcher.key;
    case "readiness":
      return session.readinessKey === matcher.key;
    case "transport":
      return session.transportKey === matcher.key;
    case "capability":
      return sessionCapabilityEnabled(session, matcher.key);
    case "degraded":
      return Boolean(session.isStale || session.transportKey !== "healthy");
    case "needs_attention":
      return session.readinessKey === "needs_attention";
    default:
      return false;
  }
}

function sessionMatchesPreset(session, preset) {
  if (!preset) return true;
  return preset.matchers.every((matcher) => sessionMatchesPresetMatcher(session, matcher));
}

function sessionHasMappedCwd(session) {
  if (session?.targetKind === "swimmers_api") {
    return Boolean(String(session?.launchCwd || "").trim());
  }
  return Boolean(String(session?.launchCwd || session?.canonicalCwd || session?.fullCwd || "").trim());
}

function sessionCapabilityEnabled(session, key) {
  const capability = String(key || "").trim().toLowerCase();
  const local = session.targetKey === "local";
  const remoteApi = session.targetKind === "swimmers_api";
  const remoteObservable = remoteApi;
  const remoteReady = remoteApi && session.transportKey === "healthy";
  switch (capability) {
    case "observe":
    case "observe_sessions":
      return local || remoteObservable;
    case "launch":
    case "launch_session":
      return local || remoteReady;
    case "send":
    case "send_input":
      return local || remoteReady;
    case "group":
    case "group_input":
      return local || remoteReady;
    case "dirs":
    case "remote_dir_inventory":
      return local || (remoteReady && sessionHasMappedCwd(session));
    case "native_attach":
      return local;
    case "ssh":
    case "ssh_attach_hint":
    case "bootstrap":
    case "bootstrap_hint":
      return false;
    case "health":
    case "health_probe":
      return local || remoteApi;
    case "external":
    case "advisory":
    case "advisory_metadata":
      return true;
    default:
      return false;
  }
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
  if (session.isStale || session.transportKey !== "healthy") bucket.degraded_count += 1;
  if (session.isStale) bucket.stale_count += 1;
  if (session.readinessKey === "needs_attention") bucket.attention_count += 1;
  if (session.commitCandidate) bucket.commit_ready_count += 1;
  bucket.advisory_count += session.advisoryBadges?.length || 0;
  buckets.set(bucketKey, bucket);
}

function advisoryFleetKey(badge) {
  const explicit = String(badge?.group_key || "").trim();
  if (explicit) return explicit;
  const source = String(badge?.source || "").trim().toLowerCase();
  const label = String(badge?.label || "").trim().toLowerCase();
  const value = String(badge?.value || "").trim().toLowerCase();
  return source && label && value ? [source, label, value].join(":") : "";
}

function advisoryFleetLabel(badge) {
  const label = String(badge?.label || "").trim();
  const value = String(badge?.value || "").trim();
  return label && value ? `${label}: ${value}` : "";
}

function addAdvisoryFleetBucket(buckets, badge) {
  const key = advisoryFleetKey(badge);
  const label = advisoryFleetLabel(badge);
  if (!key || !label) return;
  const bucketKey = `advisory\0${key}`;
  const bucket = buckets.get(bucketKey) || {
    kind: "advisory",
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
  bucket.advisory_count += 1;
  if (badge.stale) {
    bucket.stale_count += 1;
    bucket.degraded_count += 1;
  }
  buckets.set(bucketKey, bucket);
}

export function buildFleetLensSummary(sessions) {
  const buckets = new Map();
  for (const session of Array.isArray(sessions) ? sessions : []) {
    addFleetBucket(buckets, "target", session.targetKey, session.targetLabel, session);
    addFleetBucket(buckets, "repo", session.repoKey, session.repoLabel, session);
    for (const badge of session.advisoryBadges || []) {
      addAdvisoryFleetBucket(buckets, badge);
    }
    addFleetBucket(buckets, "state", session.stateKey, session.state, session);
    addFleetBucket(buckets, "readiness", session.readinessKey, session.readinessLabel, session);
    addFleetBucket(buckets, "transport", session.transportKey, session.transportLabel, session);
  }
  const order = new Map([
    ["target", 0],
    ["repo", 1],
    ["advisory", 2],
    ["state", 3],
    ["readiness", 4],
    ["transport", 5],
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
    case "advisory":
      return (session.advisoryBadges || []).some((badge) => advisoryFleetKey(badge) === active.key);
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

function environmentCapabilityLabels(environment) {
  const capabilities = environment?.capabilities || {};
  const labels = [];
  if (capabilities.observe_sessions) labels.push("observe");
  if (capabilities.launch_session) labels.push("launch");
  if (capabilities.send_input) labels.push("send");
  if (capabilities.remote_dir_inventory) labels.push("dirs");
  if (capabilities.ssh_attach_hint) labels.push("ssh");
  if (capabilities.bootstrap_hint) labels.push("bootstrap");
  if (capabilities.advisory_metadata) labels.push("external");
  return labels;
}

function environmentReadiness(environment, sessionStats) {
  const status = String(environment?.status || "").trim().toLowerCase();
  const capabilities = environment?.capabilities || {};
  if (sessionStats.attentionCount > 0) return { key: "needs_attention", label: "needs attention" };
  if (status === "degraded" || status === "unavailable" || status === "unknown") {
    return { key: "degraded", label: "degraded" };
  }
  if (String(environment?.kind || "").trim().toLowerCase() === "ssh_only") {
    return capabilities.ssh_attach_hint || capabilities.bootstrap_hint
      ? { key: "handoff", label: "handoff" }
      : { key: "blocked", label: "missing handoff" };
  }
  if (capabilities.observe_sessions || capabilities.launch_session) {
    return { key: "ready", label: "ready" };
  }
  return { key: "advisory", label: "advisory" };
}

function sessionStatsByTarget(sessions) {
  const stats = new Map();
  for (const session of Array.isArray(sessions) ? sessions : []) {
    const key = session.targetKey || "local";
    const current = stats.get(key) || { sessionCount: 0, attentionCount: 0, degradedCount: 0 };
    current.sessionCount += 1;
    if (session.readinessKey === "needs_attention") current.attentionCount += 1;
    if (session.isStale || session.transportKey !== "healthy") current.degradedCount += 1;
    stats.set(key, current);
  }
  return stats;
}

export function buildEnvironmentMatrix(environments, sessions) {
  const stats = sessionStatsByTarget(sessions);
  return (Array.isArray(environments) ? environments : [])
    .map((environment) => {
      const id = environmentRowId(environment);
      const kind = String(environment?.kind || "local").trim().toLowerCase() || "local";
      const rowStats = stats.get(id) || { sessionCount: 0, attentionCount: 0, degradedCount: 0 };
      const readiness = environmentReadiness(environment, rowStats);
      return {
        id,
        label: firstNonEmpty(environment?.label, id) || id,
        kind,
        displayHost: firstNonEmpty(environment?.display_host, environment?.label, id) || id,
        backendMode: String(environment?.backend_mode || "").trim(),
        status: String(environment?.status || "Unknown").trim() || "Unknown",
        readinessKey: readiness.key,
        readinessLabel: readiness.label,
        sessionCount: rowStats.sessionCount,
        attentionCount: rowStats.attentionCount,
        degradedCount: rowStats.degradedCount,
        pathMappingCount: Number(environment?.path_mapping_count || 0),
        capabilityLabels: environmentCapabilityLabels(environment),
        handoffOnly: kind === "ssh_only",
        observeCapable: environment?.capabilities?.observe_sessions === true,
        launchCapable: environment?.capabilities?.launch_session === true,
        attachHint: String(environment?.attach_hint || "").trim(),
        bootstrapHint: String(environment?.bootstrap_hint || "").trim(),
        lastError: String(environment?.last_error || "").trim(),
      };
    })
    .sort((left, right) => (
      Number(right.readinessKey === "needs_attention") - Number(left.readinessKey === "needs_attention")
      || Number(right.status.toLowerCase() === "degraded") - Number(left.status.toLowerCase() === "degraded")
      || Number(left.handoffOnly) - Number(right.handoffOnly)
      || right.sessionCount - left.sessionCount
      || left.displayHost.localeCompare(right.displayHost)
      || left.id.localeCompare(right.id)
    ));
}

function environmentRowId(environment) {
  return firstNonEmpty(environment?.id, environment?.label, "local") || "local";
}

function environmentMatchesPresetMatcher(environment, matcher) {
  switch (matcher.type) {
    case "all":
      return true;
    case "fleet_bucket":
      return matcher.kind === "target" && environment.id === matcher.key;
    case "target_id":
      return environment.id === matcher.id;
    case "target_kind":
      return environment.kind === matcher.kind;
    case "capability":
      return environmentCapabilityEnabled(environment, matcher.key);
    case "degraded":
      return ["degraded", "unavailable", "unknown"].includes(String(environment.status || "").toLowerCase());
    default:
      return false;
  }
}

function environmentMatchesPreset(environment, preset) {
  if (!preset) return true;
  return preset.matchers.every((matcher) => environmentMatchesPresetMatcher(environment, matcher));
}

function environmentCapabilityEnabled(environment, key) {
  const capabilities = environment?.capabilities || {};
  switch (String(key || "").trim().toLowerCase()) {
    case "observe":
    case "observe_sessions":
      return capabilities.observe_sessions === true;
    case "launch":
    case "launch_session":
      return capabilities.launch_session === true;
    case "send":
    case "send_input":
      return capabilities.send_input === true;
    case "group":
    case "group_input":
      return capabilities.group_input === true;
    case "dirs":
    case "remote_dir_inventory":
      return capabilities.remote_dir_inventory === true;
    case "native_attach":
      return capabilities.native_attach === true;
    case "ssh":
    case "ssh_attach_hint":
      return capabilities.ssh_attach_hint === true;
    case "bootstrap":
    case "bootstrap_hint":
      return capabilities.bootstrap_hint === true;
    case "external":
    case "advisory":
    case "advisory_metadata":
      return capabilities.advisory_metadata === true;
    case "health":
    case "health_probe":
      return capabilities.health_probe === true;
    default:
      return false;
  }
}

export function fleetPresetChips(presets, activePresetId) {
  const active = normalizePresetId(activePresetId);
  return buildFleetLensPresets(presets).map((preset) => ({
    label: preset.label,
    presetId: preset.id,
    active: preset.id === active,
    source: preset.source,
  }));
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
  const advisory = bucketFor(lens, "advisory");
  const degraded = firstNonHealthyTransport(lens);
  for (const bucket of [target, repo, advisory, needs, degraded].filter(Boolean)) {
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
    .map((advisory) => {
      const freshness = advisory?.freshness_ms;
      return {
        source: String(advisory?.source || "").trim(),
        label: String(advisory?.label || "").trim(),
        value: String(advisory?.value || "").trim(),
        status: String(advisory?.status || "external").trim() || "external",
        stale: advisory?.stale !== false,
        group_key: String(advisory?.group_key || "").trim(),
        observed_at: String(advisory?.observed_at || "").trim(),
        freshness_ms: freshness === null || freshness === undefined || freshness === ""
          ? null
          : (Number.isFinite(Number(freshness)) ? Number(freshness) : null),
      };
    })
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
    targetKind: String(environment.target_kind || (target.key === "local" ? "local" : "")).trim().toLowerCase(),
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

// Single source of truth for which sessions a surface shows: an active fleet
// preset (resolved from state.fleetPresetId) filters via its matchers and
// suppresses the manual fleetFilter; otherwise the manual fleetFilter applies.
// Both the HUD model and the Trogdor atlas use this so they never diverge.
export function resolveSurfaceSessions(state, selectedSession, allSurfaceSessions, fleetLens) {
  const fleetPresets = buildFleetLensPresets(state.fleetPresets);
  const requestedPresetId = normalizePresetId(state.fleetPresetId);
  const activePreset = resolveFleetPreset(
    fleetPresets.find((preset) => preset.id === requestedPresetId),
    selectedSession,
    allSurfaceSessions,
  );
  const fleetFilter = activePreset
    ? { kind: "", key: "" }
    : availableFleetFilter(fleetLens, state.fleetFilter);
  const surfaceSessions = activePreset
    ? allSurfaceSessions.filter((session) => sessionMatchesPreset(session, activePreset))
    : allSurfaceSessions.filter((session) => sessionMatchesFleetFilter(session, fleetFilter));
  return { activePreset, fleetFilter, surfaceSessions, fleetPresets };
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
  const { activePreset, fleetFilter, surfaceSessions, fleetPresets } = resolveSurfaceSessions(
    state,
    selectedSession,
    allSurfaceSessions,
    fleetLens,
  );
  const filteredFleetLens = buildFleetLensSummary(surfaceSessions);
  const environmentSessions = activePreset ? surfaceSessions : allSurfaceSessions;
  const allEnvironmentMatrix = buildEnvironmentMatrix(state.environments, environmentSessions);
  const environmentById = new Map((Array.isArray(state.environments) ? state.environments : [])
    .map((environment) => [environmentRowId(environment), environment]));
  const presetTargetIds = new Set(surfaceSessions.map((session) => session.targetKey).filter(Boolean));
  const environmentMatrix = activePreset
    ? allEnvironmentMatrix.filter((row) => (
      presetTargetIds.has(row.id) || environmentMatchesPreset(environmentById.get(row.id), activePreset)
    ))
    : allEnvironmentMatrix;
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
    environmentMatrix,
    fleetFilter,
    fleetPresetId: activePreset?.id || "",
    fleetPreset: activePreset || null,
    fleetPresets,
    fleetLens,
    filteredFleetLens,
    fleetChips: fleetLensChips(fleetLens, fleetFilter),
    fleetPresetChips: fleetPresetChips(fleetPresets, activePreset?.id || ""),
    fleetEmptyMessage: activePreset && surfaceSessions.length === 0
      ? `No sessions match ${activePreset.label}.`
      : "",
    selectedSessionId: state.selectedSessionId,
    publishedSessionId: normalizeSessionId(state.publishedSelection?.session_id),
    publishedAtLabel: formatTime(state.publishedSelection?.published_at),
    currentSession: selectedSessionVisible
      ? surfaceSession(selectedSession, surfaceOptions(selectedSession, { detail: true }))
      : null,
  };
}
