const DEFAULT_BOOT_PAYLOAD = Object.freeze({
  franken_term_available: false,
  franken_term_js_url: "",
  franken_term_wasm_url: "",
  franken_term_font_url: "",
  franken_term_asset_info: null,
  follow_published_selection: false,
  focus_layout: false,
});

function objectRecord(value) {
  return value && typeof value === "object" && !Array.isArray(value) ? value : null;
}

function stringValue(value, fallback = "") {
  return value === null || value === undefined ? fallback : String(value);
}

function optionalString(value) {
  return value === null || value === undefined ? null : String(value);
}

function finiteNumber(value, fallback = 0) {
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

function booleanValue(value, fallback = false) {
  return typeof value === "boolean" ? value : fallback;
}

function stringArray(value) {
  return Array.isArray(value)
    ? value.map((entry) => String(entry || "").trim()).filter(Boolean)
    : [];
}

function stringArrayKeepingEmpty(value) {
  return Array.isArray(value)
    ? value.map((entry) => String(entry ?? "").trim())
    : [];
}

function objectArray(value) {
  return Array.isArray(value) ? value.map(objectRecord).filter(Boolean) : [];
}

function objectMap(value) {
  return objectRecord(value) ? { ...value } : {};
}

function normalizeAssetFileInfo(value) {
  const file = objectRecord(value);
  if (!file) {
    return null;
  }
  return {
    ...file,
    route: stringValue(file.route),
    size_bytes: finiteNumber(file.size_bytes),
    checksum: stringValue(file.checksum),
  };
}

function normalizeFrankenTermAssetInfo(value) {
  const info = objectRecord(value);
  if (!info) {
    return null;
  }
  const js = normalizeAssetFileInfo(info.js);
  const wasm = normalizeAssetFileInfo(info.wasm);
  if (!js || !wasm) {
    return null;
  }
  return {
    ...info,
    js,
    wasm,
    font: normalizeAssetFileInfo(info.font),
  };
}

export function normalizeBootPayload(value) {
  const payload = objectRecord(value) || {};
  return {
    ...DEFAULT_BOOT_PAYLOAD,
    ...payload,
    franken_term_available: booleanValue(payload.franken_term_available),
    franken_term_js_url: stringValue(payload.franken_term_js_url),
    franken_term_wasm_url: stringValue(payload.franken_term_wasm_url),
    franken_term_font_url: stringValue(payload.franken_term_font_url),
    franken_term_asset_info: normalizeFrankenTermAssetInfo(payload.franken_term_asset_info),
    follow_published_selection: booleanValue(payload.follow_published_selection),
    focus_layout: booleanValue(payload.focus_layout),
  };
}

export function normalizeStateEvidence(value) {
  const evidence = objectRecord(value) || {};
  return {
    ...evidence,
    cause: stringValue(evidence.cause, "unknown"),
    observed_at: optionalString(evidence.observed_at),
    confidence: stringValue(evidence.confidence, "low"),
  };
}

export function normalizeActionCue(value) {
  const cue = objectRecord(value) || {};
  return {
    ...cue,
    kind: stringValue(cue.kind),
    status: stringValue(cue.status),
    source: stringValue(cue.source),
    confidence: stringValue(cue.confidence),
    evidence: stringArray(cue.evidence),
  };
}

export function normalizeSessionSummary(value) {
  const session = objectRecord(value);
  if (!session) {
    return null;
  }
  return {
    ...session,
    session_id: stringValue(session.session_id),
    tmux_name: stringValue(session.tmux_name || session.session_id),
    state: stringValue(session.state, "unknown"),
    current_command: optionalString(session.current_command),
    state_evidence: normalizeStateEvidence(session.state_evidence),
    cwd: stringValue(session.cwd),
    tool: optionalString(session.tool),
    token_count: finiteNumber(session.token_count),
    context_limit: finiteNumber(session.context_limit),
    thought: optionalString(session.thought),
    thought_state: stringValue(session.thought_state, "holding"),
    thought_source: stringValue(session.thought_source, "carry_forward"),
    thought_updated_at: optionalString(session.thought_updated_at),
    rest_state: stringValue(session.rest_state, "active"),
    commit_candidate: booleanValue(session.commit_candidate),
    action_cues: objectArray(session.action_cues).map(normalizeActionCue),
    objective_changed_at: optionalString(session.objective_changed_at),
    last_skill: optionalString(session.last_skill),
    is_stale: booleanValue(session.is_stale),
    attached_clients: finiteNumber(session.attached_clients),
    stale_attached_clients: finiteNumber(session.stale_attached_clients),
    transport_health: stringValue(session.transport_health, "healthy"),
    last_activity_at: stringValue(session.last_activity_at),
    repo_theme_id: optionalString(session.repo_theme_id),
    batch: objectRecord(session.batch),
    environment: normalizeSessionEnvironmentSummary(session.environment),
  };
}

export function normalizeSessionEnvironmentSummary(value) {
  const environment = objectRecord(value) || {};
  return {
    scope: stringValue(environment.scope, "local"),
    target_id: stringValue(environment.target_id, "local"),
    target_label: stringValue(environment.target_label, "Local machine"),
    target_kind: stringValue(environment.target_kind, "local"),
    display_host: stringValue(environment.display_host, "local"),
    remote_session_id: optionalString(environment.remote_session_id),
    launch_source: optionalString(environment.launch_source),
    local_cwd: optionalString(environment.local_cwd),
    remote_cwd: optionalString(environment.remote_cwd),
    canonical_cwd: optionalString(environment.canonical_cwd),
  };
}

function normalizeEnvironmentAuthSummary(value) {
  const auth = objectRecord(value) || {};
  const tokenEnvPresent =
    typeof auth.token_env_present === "boolean" ? auth.token_env_present : null;
  return {
    mode: stringValue(auth.mode, "none"),
    token_env_present: tokenEnvPresent,
  };
}

export function normalizeEnvironmentSummary(value) {
  const environment = objectRecord(value) || {};
  return {
    id: stringValue(environment.id, "local"),
    label: stringValue(environment.label, "Local machine"),
    kind: stringValue(environment.kind, "local"),
    backend_mode: stringValue(environment.backend_mode, "local"),
    base_url: optionalString(environment.base_url),
    auth: normalizeEnvironmentAuthSummary(environment.auth),
    path_mapping_count: finiteNumber(environment.path_mapping_count),
    status: stringValue(environment.status, "Unknown"),
    last_seen_at: optionalString(environment.last_seen_at),
    last_error_at: optionalString(environment.last_error_at),
    last_error: optionalString(environment.last_error),
    freshness_ms: environment.freshness_ms === null || environment.freshness_ms === undefined
      ? null
      : finiteNumber(environment.freshness_ms),
  };
}

export function normalizeSessionListResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    sessions: objectArray(payload.sessions).map(normalizeSessionSummary).filter(Boolean),
    version: finiteNumber(payload.version),
    repo_themes: objectMap(payload.repo_themes),
    environments: objectArray(payload.environments).map(normalizeEnvironmentSummary),
  };
}

export function normalizePublishedSelectionResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: optionalString(payload.session_id),
    session: normalizeSessionSummary(payload.session),
    published_at: optionalString(payload.published_at),
    error: objectRecord(payload.error),
  };
}

export function normalizeOperatorPressure(value) {
  const pressure = objectRecord(value) || {};
  return {
    score: finiteNumber(pressure.score),
    reason: stringValue(pressure.reason),
    reason_kind: stringValue(pressure.reason_kind, "idle"),
    glyph: stringValue(pressure.glyph, "a"),
    tone: stringValue(pressure.tone, "quiet"),
    needs_input: booleanValue(pressure.needs_input),
    launch_ready: booleanValue(pressure.launch_ready),
    commit_ready: booleanValue(pressure.commit_ready),
    action_cue_count: finiteNumber(pressure.action_cue_count),
  };
}

export function normalizeOperatorPressureSession(value) {
  const session = objectRecord(value) || {};
  return {
    session_id: stringValue(session.session_id),
    repo_key: stringValue(session.repo_key),
    repo_label: stringValue(session.repo_label),
    pressure: normalizeOperatorPressure(session.pressure),
    batch_send_session_ids: stringArray(session.batch_send_session_ids),
  };
}

function normalizeOperatorPressureRepo(value) {
  const repo = objectRecord(value) || {};
  return {
    repo_key: stringValue(repo.repo_key),
    repo_label: stringValue(repo.repo_label),
    score: finiteNumber(repo.score),
    reason: stringValue(repo.reason),
    session_ids: stringArray(repo.session_ids),
  };
}

export function normalizeOperatorPressureResponse(value) {
  const payload = objectRecord(value) || {};
  const summary = objectRecord(payload.summary) || {};
  return {
    sessions: objectArray(payload.sessions).map(normalizeOperatorPressureSession),
    repos: objectArray(payload.repos).map(normalizeOperatorPressureRepo),
    summary: {
      max_score: finiteNumber(summary.max_score),
      action_cues: finiteNumber(summary.action_cues),
      batch_send_groups: finiteNumber(summary.batch_send_groups),
    },
  };
}

export function normalizeTerminalServerFrame(value) {
  const frame = objectRecord(value);
  if (!frame) {
    return { type: "unknown", raw: value };
  }
  switch (frame.type) {
    case "ready":
      return {
        ...frame,
        type: "ready",
        sessionId: stringValue(frame.sessionId || frame.session_id),
        readOnly: booleanValue(frame.readOnly),
        replay: normalizeReplayCursor(frame.replay),
        protocol: normalizeTerminalProtocol(frame.protocol),
        summary: normalizeSessionSummary(frame.summary),
      };
    case "replay_truncated":
      return { ...frame, type: "replay_truncated" };
    case "error":
      return {
        ...frame,
        type: "error",
        code: stringValue(frame.code, "error"),
        message: optionalString(frame.message),
      };
    case "overloaded":
      return {
        ...frame,
        type: "overloaded",
        retryAfterMs: finiteNumber(frame.retryAfterMs || frame.retry_after_ms, 4000),
      };
    case "input_ack":
      return {
        ...frame,
        type: "input_ack",
        clientMessageId: optionalString(frame.clientMessageId || frame.client_message_id),
        delivered: booleanValue(frame.delivered),
        method: stringValue(frame.method),
        message: optionalString(frame.message),
      };
    case "control_event":
      return normalizeControlEventFrame(frame);
    case "lifecycle_event":
      return normalizeLifecycleEventFrame(frame);
    case "event_stream_lagged":
      return {
        ...frame,
        type: "event_stream_lagged",
        stream: stringValue(frame.stream),
        skipped: finiteNumber(frame.skipped),
      };
    case "pong":
      return { ...frame, type: "pong" };
    default:
      return { ...frame, type: "unknown", rawType: optionalString(frame.type) };
  }
}

function normalizeReplayCursor(value) {
  const replay = objectRecord(value) || {};
  return {
    latestSeq: finiteNumber(replay.latestSeq || replay.latest_seq),
    windowStartSeq: finiteNumber(replay.windowStartSeq || replay.window_start_seq),
    resumeFromSeq: finiteNumber(replay.resumeFromSeq || replay.resume_from_seq),
  };
}

function normalizeTerminalProtocol(value) {
  const protocol = objectRecord(value) || {};
  return {
    ...protocol,
    output: stringValue(protocol.output, "raw"),
  };
}

export function normalizeControlEventFrame(value) {
  const frame = objectRecord(value) || {};
  return {
    ...frame,
    type: "control_event",
    event: stringValue(frame.event),
    sessionId: stringValue(frame.sessionId || frame.session_id),
    payload: frame.payload ?? null,
  };
}

export function normalizeLifecycleEventFrame(value) {
  const frame = objectRecord(value) || {};
  return {
    ...frame,
    type: "lifecycle_event",
    event: stringValue(frame.event),
    sessionId: stringValue(frame.sessionId || frame.session_id),
    reason: optionalString(frame.reason),
    summary: normalizeSessionSummary(frame.summary),
    repoTheme: objectRecord(frame.repoTheme || frame.repo_theme),
    deleteMode: optionalString(frame.deleteMode || frame.delete_mode),
    tmuxSessionAlive: typeof frame.tmuxSessionAlive === "boolean"
      ? frame.tmuxSessionAlive
      : typeof frame.tmux_session_alive === "boolean"
        ? frame.tmux_session_alive
        : null,
  };
}

export function normalizeTerminalSnapshotResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    latest_seq: finiteNumber(payload.latest_seq),
    truncated: booleanValue(payload.truncated),
    screen_text: stringValue(payload.screen_text),
  };
}

export function normalizeDirEntry(value) {
  const entry = objectRecord(value) || {};
  return {
    ...entry,
    name: stringValue(entry.name),
    has_children: booleanValue(entry.has_children),
    is_running: typeof entry.is_running === "boolean" ? entry.is_running : null,
    repo_dirty: typeof entry.repo_dirty === "boolean" ? entry.repo_dirty : null,
    repo_action: objectRecord(entry.repo_action),
    group: optionalString(entry.group),
    groups: stringArray(entry.groups),
    full_path: optionalString(entry.full_path),
    has_restart: typeof entry.has_restart === "boolean" ? entry.has_restart : null,
    open_url: optionalString(entry.open_url),
  };
}

export function normalizeLaunchTargetSummary(value) {
  const target = objectRecord(value) || {};
  return {
    ...target,
    id: stringValue(target.id, "local"),
    label: stringValue(target.label || target.id || "Local machine"),
    kind: stringValue(target.kind, "local"),
    base_url: optionalString(target.base_url),
    auth_token_env: optionalString(target.auth_token_env),
    path_mappings: objectArray(target.path_mappings).map((mapping) => ({
      ...mapping,
      local_prefix: stringValue(mapping.local_prefix),
      remote_prefix: stringValue(mapping.remote_prefix),
    })),
  };
}

export function normalizeDirListResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    path: stringValue(payload.path),
    entries: objectArray(payload.entries).map(normalizeDirEntry),
    overlay_label: optionalString(payload.overlay_label),
    groups: stringArray(payload.groups),
    launch_targets: objectArray(payload.launch_targets).map(normalizeLaunchTargetSummary),
    default_launch_target: optionalString(payload.default_launch_target),
  };
}

export function normalizeDirRepoSearchResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    roots: stringArray(payload.roots),
    entries: objectArray(payload.entries).map(normalizeDirEntry),
  };
}

export function normalizeMermaidArtifactResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    available: booleanValue(payload.available),
    path: optionalString(payload.path),
    updated_at: optionalString(payload.updated_at),
    source: optionalString(payload.source),
    error: optionalString(payload.error),
    slice_name: optionalString(payload.slice_name),
    plan_files: stringArray(payload.plan_files),
  };
}

export function normalizePlanFileResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    name: stringValue(payload.name),
    content: optionalString(payload.content),
    error: optionalString(payload.error),
  };
}

export function normalizeNativeDesktopStatusResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    supported: booleanValue(payload.supported),
    platform: optionalString(payload.platform),
    app_id: optionalString(payload.app_id),
    ghostty_mode: optionalString(payload.ghostty_mode),
    app: optionalString(payload.app),
    reason: optionalString(payload.reason),
  };
}

export function normalizeNativeDesktopOpenResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    status: stringValue(payload.status),
    pane_id: optionalString(payload.pane_id),
  };
}

export function normalizeNativeAttentionGroupOpenResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    tmux_name: stringValue(payload.tmux_name),
    session_count: finiteNumber(payload.session_count),
    session_ids: stringArray(payload.session_ids),
    backlog_session_ids: stringArray(payload.backlog_session_ids),
    status: stringValue(payload.status),
    focused: booleanValue(payload.focused),
    pane_id: optionalString(payload.pane_id),
    attach_command: optionalString(payload.attach_command),
  };
}

export function normalizeThoughtConfig(value) {
  const config = objectRecord(value) || {};
  return {
    ...config,
    enabled: typeof config.enabled === "boolean" ? config.enabled : true,
    model: stringValue(config.model),
    backend: stringValue(config.backend),
    cadence_hot_ms: finiteNumber(config.cadence_hot_ms),
    cadence_warm_ms: finiteNumber(config.cadence_warm_ms),
    cadence_cold_ms: finiteNumber(config.cadence_cold_ms),
    agent_prompt: optionalString(config.agent_prompt),
    terminal_prompt: optionalString(config.terminal_prompt),
  };
}

export function normalizeThoughtConfigResponse(value) {
  const payload = objectRecord(value) || {};
  const nestedConfig = objectRecord(payload.config);
  if (nestedConfig) {
    return {
      ...payload,
      config: normalizeThoughtConfig(nestedConfig),
      daemon_defaults: normalizeDaemonDefaults(payload.daemon_defaults),
      ui: normalizeThoughtConfigUiMetadata(payload.ui),
    };
  }
  return {
    ...normalizeThoughtConfig(payload),
    daemon_defaults: normalizeDaemonDefaults(payload.daemon_defaults),
    ui: normalizeThoughtConfigUiMetadata(payload.ui),
    version: finiteNumber(payload.version),
  };
}

function normalizeDaemonDefaults(value) {
  const defaults = objectRecord(value);
  return defaults
    ? {
        ...defaults,
        model: stringValue(defaults.model),
        backend: stringValue(defaults.backend),
        agent_prompt: stringValue(defaults.agent_prompt),
        terminal_prompt: stringValue(defaults.terminal_prompt),
      }
    : null;
}

function normalizeThoughtConfigUiMetadata(value) {
  const ui = objectRecord(value) || {};
  return {
    ...ui,
    backends: objectArray(ui.backends).map((backend) => ({
      ...backend,
      key: stringValue(backend.key),
      label: stringValue(backend.label),
      model_presets_hint: stringValue(backend.model_presets_hint),
      model_presets: stringArrayKeepingEmpty(backend.model_presets),
    })),
  };
}

export function normalizeThoughtConfigProbeResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    ok: booleanValue(payload.ok),
    llm_calls: finiteNumber(payload.llm_calls),
    message: optionalString(payload.message),
    last_backend_error: optionalString(payload.last_backend_error),
  };
}

export function normalizeSessionPaneTailResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    text: stringValue(payload.text),
  };
}

export function normalizeAgentContextActionSummary(value) {
  const action = objectRecord(value) || {};
  return {
    ...action,
    tool: stringValue(action.tool),
    detail: optionalString(action.detail),
  };
}

export function normalizeSessionAgentTurn(value) {
  const turn = objectRecord(value) || {};
  return {
    ...turn,
    id: stringValue(turn.id),
    source: stringValue(turn.source),
    text: stringValue(turn.text),
    byte_start: finiteNumber(turn.byte_start),
    byte_end: finiteNumber(turn.byte_end),
    order: finiteNumber(turn.order),
    timestamp: optionalString(turn.timestamp),
  };
}

export function normalizeSessionTranscriptRecord(value) {
  const record = objectRecord(value) || {};
  return {
    ...record,
    id: stringValue(record.id),
    source: stringValue(record.source),
    kind: stringValue(record.kind),
    role: optionalString(record.role),
    summary: stringValue(record.summary),
    raw: stringValue(record.raw),
    byte_start: finiteNumber(record.byte_start),
    byte_end: finiteNumber(record.byte_end),
    timestamp: optionalString(record.timestamp),
    truncated: booleanValue(record.truncated),
  };
}

export function normalizeSessionAgentContextResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    available: booleanValue(payload.available),
    tool: optionalString(payload.tool),
    cwd: stringValue(payload.cwd),
    user_task: optionalString(payload.user_task),
    turns: objectArray(payload.turns).map(normalizeSessionAgentTurn),
    current_tool: objectRecord(payload.current_tool)
      ? normalizeAgentContextActionSummary(payload.current_tool)
      : null,
    recent_actions: objectArray(payload.recent_actions).map(normalizeAgentContextActionSummary),
    token_count: finiteNumber(payload.token_count),
    context_limit: finiteNumber(payload.context_limit),
    message: optionalString(payload.message),
  };
}

export function normalizeSessionTranscriptResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    available: booleanValue(payload.available),
    tool: optionalString(payload.tool),
    cwd: stringValue(payload.cwd),
    selected_turn_id: optionalString(payload.selected_turn_id),
    selected_turn: objectRecord(payload.selected_turn)
      ? normalizeSessionAgentTurn(payload.selected_turn)
      : null,
    next_cursor: finiteNumber(payload.next_cursor),
    records: objectArray(payload.records).map(normalizeSessionTranscriptRecord),
    turns: objectArray(payload.turns).map(normalizeSessionAgentTurn),
    message: optionalString(payload.message),
  };
}

export function normalizeSessionTimelineResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    available: booleanValue(payload.available),
    cwd: stringValue(payload.cwd),
    tool: optionalString(payload.tool),
    events: objectArray(payload.events).map((event) => ({
      ...event,
      id: stringValue(event.id),
      kind: stringValue(event.kind),
      source: stringValue(event.source),
      title: stringValue(event.title),
      summary: stringValue(event.summary),
      timestamp: optionalString(event.timestamp),
      order: event.order === null || event.order === undefined ? null : finiteNumber(event.order),
      detail: optionalString(event.detail),
    })),
    pinned: objectMap(payload.pinned),
    message: optionalString(payload.message),
  };
}

export function normalizeSessionSkillListResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    source: stringValue(payload.source),
    cwd: stringValue(payload.cwd),
    available: booleanValue(payload.available),
    query: optionalString(payload.query),
    skills: objectArray(payload.skills).map((skill) => ({
      ...skill,
      name: stringValue(skill.name),
      description: optionalString(skill.description),
      state: optionalString(skill.state),
      availability: optionalString(skill.availability),
      layer: optionalString(skill.layer),
      source_bucket: optionalString(skill.source_bucket),
      source: optionalString(skill.source),
      path: optionalString(skill.path),
    })),
    issues: objectArray(payload.issues).map((issue) => ({
      ...issue,
      skill: optionalString(issue.skill),
      action: optionalString(issue.action),
      hint: optionalString(issue.hint),
      source_path: optionalString(issue.source_path),
      message: stringValue(issue.message),
    })),
    message: optionalString(payload.message),
  };
}

export function normalizeSessionGitDiffResponse(value) {
  const payload = objectRecord(value) || {};
  return {
    ...payload,
    session_id: stringValue(payload.session_id),
    available: booleanValue(payload.available),
    cwd: stringValue(payload.cwd),
    repo_root: optionalString(payload.repo_root),
    status_short: stringValue(payload.status_short),
    unstaged_diff: stringValue(payload.unstaged_diff),
    staged_diff: stringValue(payload.staged_diff),
    truncated: booleanValue(payload.truncated),
    message: optionalString(payload.message),
    files: objectArray(payload.files).map((file) => ({
      ...file,
      path: stringValue(file.path),
      old_path: optionalString(file.old_path),
      source: stringValue(file.source),
      change: stringValue(file.change),
      added_lines: finiteNumber(file.added_lines),
      removed_lines: finiteNumber(file.removed_lines),
      truncated: booleanValue(file.truncated),
      hunks: objectArray(file.hunks).map((hunk) => ({
        ...hunk,
        header: stringValue(hunk.header),
        added_lines: finiteNumber(hunk.added_lines),
        removed_lines: finiteNumber(hunk.removed_lines),
      })),
    })),
  };
}

function normalizeSettled(result, normalizer) {
  if (result?.status !== "fulfilled") {
    return result;
  }
  return {
    ...result,
    value: result.value === null ? null : normalizer(result.value),
  };
}

export function normalizeWorkbenchWidgetResults(results = {}) {
  return {
    timelineResult: normalizeSettled(results.timelineResult, normalizeSessionTimelineResponse),
    skillsResult: normalizeSettled(results.skillsResult, normalizeSessionSkillListResponse),
    tailResult: normalizeSettled(results.tailResult, normalizeSessionPaneTailResponse),
    transcriptResult: normalizeSettled(results.transcriptResult, normalizeSessionTranscriptResponse),
    artifactResult: normalizeSettled(results.artifactResult, normalizeMermaidArtifactResponse),
    diffResult: normalizeSettled(results.diffResult, normalizeSessionGitDiffResponse),
  };
}

export function normalizeTrogdorSurfaceSession(value) {
  const session = objectRecord(value) || {};
  return {
    ...session,
    sessionId: stringValue(session.sessionId),
    name: stringValue(session.name || session.sessionId),
    state: stringValue(session.state, "unknown"),
    displayState: stringValue(session.displayState || session.state || "unknown"),
    stateTrustLabel: stringValue(session.stateTrustLabel),
    stateConfidence: stringValue(session.stateConfidence, "low"),
    stateObserved: booleanValue(session.stateObserved),
    restLabel: stringValue(session.restLabel, "unknown"),
    transportLabel: stringValue(session.transportLabel, "unknown"),
    toolLabel: stringValue(session.toolLabel, "shell"),
    cwdLabel: stringValue(session.cwdLabel),
    fullCwd: stringValue(session.fullCwd),
    canonicalCwd: stringValue(session.canonicalCwd),
    thoughtLabel: stringValue(session.thoughtLabel),
    clawgText: stringValue(session.clawgText),
    thoughtUpdatedAt: stringValue(session.thoughtUpdatedAt),
    objectiveChangedAt: stringValue(session.objectiveChangedAt),
    contextLabel: stringValue(session.contextLabel),
    skillLabel: stringValue(session.skillLabel),
    activityLabel: stringValue(session.activityLabel),
    commandLabel: stringValue(session.commandLabel),
    attachedLabel: stringValue(session.attachedLabel),
    commitCandidate: booleanValue(session.commitCandidate),
    actionCues: objectArray(session.actionCues).map(normalizeActionCue),
    operatorPressure: objectRecord(session.operatorPressure)
      ? normalizeOperatorPressure(session.operatorPressure)
      : null,
    batchSendSessionIds: stringArray(session.batchSendSessionIds),
    repoKey: stringValue(session.repoKey),
    repoLabel: stringValue(session.repoLabel),
    isStale: booleanValue(session.isStale),
    clawgReadIndex: finiteNumber(session.clawgReadIndex),
    clawgWordCount: finiteNumber(session.clawgWordCount),
    trogdorAwaitingUser: booleanValue(session.trogdorAwaitingUser),
    trogdorBurnt: booleanValue(session.trogdorBurnt),
    trogdorDismissed: booleanValue(session.trogdorDismissed),
    trogdorSwordsmanVisible: booleanValue(session.trogdorSwordsmanVisible),
  };
}

export function normalizeSurfaceModel(value) {
  const model = objectRecord(value) || {};
  const currentSession = objectRecord(model.currentSession);
  return {
    ...model,
    cols: finiteNumber(model.cols),
    rows: finiteNumber(model.rows),
    focusLayout: booleanValue(model.focusLayout),
    followPublishedSelection: booleanValue(model.followPublishedSelection),
    connectionLabel: stringValue(model.connectionLabel),
    connectionMuted: booleanValue(model.connectionMuted),
    modeLabel: stringValue(model.modeLabel),
    modeMuted: booleanValue(model.modeMuted),
    searchLabel: stringValue(model.searchLabel),
    searchMuted: booleanValue(model.searchMuted),
    utilityLabel: stringValue(model.utilityLabel),
    utilityMuted: booleanValue(model.utilityMuted),
    searchQuery: stringValue(model.searchQuery),
    selectMode: booleanValue(model.selectMode),
    readOnly: booleanValue(model.readOnly),
    frankenTermAvailable: booleanValue(model.frankenTermAvailable),
    terminalReady: booleanValue(model.terminalReady),
    snapshotFallback: booleanValue(model.snapshotFallback),
    activeSheet: stringValue(model.activeSheet),
    hoveredLinkUrl: stringValue(model.hoveredLinkUrl),
    hoveredTrogdorSessionId: stringValue(model.hoveredTrogdorSessionId),
    trogdorAtlasOpen: booleanValue(model.trogdorAtlasOpen),
    trogdorWpm: finiteNumber(model.trogdorWpm),
    trogdorReading: booleanValue(model.trogdorReading),
    trogdorReaderStartIndex: finiteNumber(model.trogdorReaderStartIndex),
    trogdorReaderElapsedMs: finiteNumber(model.trogdorReaderElapsedMs),
    sessions: objectArray(model.sessions).map(normalizeTrogdorSurfaceSession),
    selectedSessionId: optionalString(model.selectedSessionId),
    publishedSessionId: optionalString(model.publishedSessionId),
    publishedAtLabel: stringValue(model.publishedAtLabel),
    currentSession: currentSession ? normalizeTrogdorSurfaceSession(currentSession) : null,
  };
}
