export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  [key: string]: JsonValue;
}

export type IsoDateTime = string;
export type Nullable<T> = T | null;

export type SessionState = "idle" | "busy" | "error" | "attention" | "exited" | string;
export type ThoughtState = "active" | "holding" | "sleeping" | string;
export type ThoughtSource = "carry_forward" | "llm" | "static_sleeping" | string;
export type RestState = "active" | "drowsy" | "sleeping" | "deep_sleep" | string;
export type TransportHealth = "healthy" | "degraded" | "overloaded" | "disconnected" | string;
export type StateConfidence = "low" | "medium" | "high" | string;
export type RepoActionKind = "commit" | "restart" | "open";
export type RepoActionState = "running" | "succeeded" | "failed" | string;
export type NativeDesktopApp = "iterm" | "ghostty" | string;
export type GhosttyOpenMode = "swap" | "add" | "window" | string;
export type SpawnTool = "claude" | "codex" | "grok" | string;
export type OperatorPressureReasonKind =
  | "awaiting_user"
  | "commit_ready"
  | "validation_missing_after_edit"
  | "dirty_check_missing"
  | "needs_input"
  | "error"
  | "sleeping"
  | "untrusted_state"
  | "stale"
  | "transport"
  | "busy"
  | "idle"
  | string;
export type OperatorPressureTone = "quiet" | "working" | "warning" | "danger" | string;
export type TrogdorActionId =
  | "trogdor_read_toggle"
  | "trogdor_wpm_down"
  | "trogdor_wpm_up"
  | "trogdor_send"
  | "trogdor_group_send"
  | "trogdor_launch"
  | "trogdor_mermaid"
  | "trogdor_commit";

export interface OperatorPressure {
  score: number;
  reason: string;
  reason_kind: OperatorPressureReasonKind;
  glyph: string;
  tone: OperatorPressureTone;
  needs_input: boolean;
  launch_ready: boolean;
  commit_ready: boolean;
  action_cue_count: number;
}

export interface OperatorPressureSession {
  session_id: string;
  repo_key: string;
  repo_label: string;
  pressure: OperatorPressure;
  batch_send_session_ids: string[];
}

export interface OperatorPressureRepo {
  repo_key: string;
  repo_label: string;
  score: number;
  reason: string;
  session_ids: string[];
}

export interface OperatorPressureSummary {
  max_score: number;
  action_cues: number;
  batch_send_groups: number;
}

export interface OperatorPressureResponse {
  sessions: OperatorPressureSession[];
  repos: OperatorPressureRepo[];
  summary: OperatorPressureSummary;
}

export interface FrankenTermAssetFileInfo {
  route: string;
  size_bytes: number;
  checksum: string;
}

export interface FrankenTermAssetInfo {
  js: FrankenTermAssetFileInfo;
  wasm: FrankenTermAssetFileInfo;
  font: Nullable<FrankenTermAssetFileInfo>;
}

export interface BootPayload {
  franken_term_available: boolean;
  franken_term_js_url: string;
  franken_term_wasm_url: string;
  franken_term_font_url: string;
  franken_term_asset_info: Nullable<FrankenTermAssetInfo>;
  follow_published_selection: boolean;
  focus_layout: boolean;
}

export interface StateEvidence {
  cause: string;
  observed_at: Nullable<IsoDateTime>;
  confidence: StateConfidence;
}

export interface ActionCue {
  kind: string;
  status: string;
  source: string;
  confidence: string;
  evidence: string[];
}

export interface SessionBatchMembership {
  id: string;
  label: string;
  index: number;
  total: number;
  created_at: IsoDateTime;
  prompt_excerpt?: Nullable<string>;
}

export type SessionEnvironmentScope = "local" | "remote" | string;

export interface AdvisoryMetadataSummary {
  source: string;
  label: string;
  value: string;
  status: string;
  stale: boolean;
}

export interface SessionEnvironmentSummary {
  scope: SessionEnvironmentScope;
  target_id: string;
  target_label: string;
  target_kind: string;
  display_host: string;
  remote_session_id: Nullable<string>;
  launch_source: Nullable<string>;
  local_cwd: Nullable<string>;
  remote_cwd: Nullable<string>;
  canonical_cwd: Nullable<string>;
  advisory: AdvisoryMetadataSummary[];
}

export interface SessionSummary {
  session_id: string;
  tmux_name: string;
  state: SessionState;
  current_command: Nullable<string>;
  state_evidence: StateEvidence;
  cwd: string;
  tool: Nullable<string>;
  token_count: number;
  context_limit: number;
  thought: Nullable<string>;
  thought_state: ThoughtState;
  thought_source: ThoughtSource;
  thought_updated_at: Nullable<IsoDateTime>;
  rest_state: RestState;
  commit_candidate: boolean;
  action_cues: ActionCue[];
  objective_changed_at: Nullable<IsoDateTime>;
  last_skill: Nullable<string>;
  is_stale: boolean;
  attached_clients: number;
  stale_attached_clients: number;
  transport_health: TransportHealth;
  last_activity_at: IsoDateTime;
  repo_theme_id: Nullable<string>;
  batch: Nullable<SessionBatchMembership | Record<string, unknown>>;
  environment: SessionEnvironmentSummary;
}

export interface RepoTheme {
  body: string;
  outline: string;
  accent: string;
  shirt: string;
  sprite?: Nullable<string>;
}

export interface EnvironmentAuthSummary {
  mode: string;
  token_env_present: Nullable<boolean>;
}

export interface EnvironmentSummary {
  id: string;
  label: string;
  kind: string;
  backend_mode: string;
  base_url: Nullable<string>;
  auth: EnvironmentAuthSummary;
  path_mapping_count: number;
  status: string;
  last_seen_at: Nullable<IsoDateTime>;
  last_error_at: Nullable<IsoDateTime>;
  last_error: Nullable<string>;
  freshness_ms: Nullable<number>;
  advisory: AdvisoryMetadataSummary[];
}

export type FleetLensBucketKind = "target" | "repo" | "state" | "readiness" | "transport";

export interface FleetLensBucket {
  kind: FleetLensBucketKind | string;
  key: string;
  label: string;
  count: number;
  degraded_count: number;
  stale_count: number;
  attention_count: number;
  commit_ready_count: number;
}

export interface FleetLensSummary {
  total_sessions: number;
  buckets: FleetLensBucket[];
}

export interface SessionListResponse {
  sessions: SessionSummary[];
  version: number;
  repo_themes: Record<string, RepoTheme | Record<string, unknown>>;
  environments: EnvironmentSummary[];
  fleet_lens: FleetLensSummary;
}

export interface ErrorResponse {
  code: string;
  message?: Nullable<string>;
}

export interface PublishedSelectionResponse {
  session_id: Nullable<string>;
  session: Nullable<SessionSummary>;
  published_at: Nullable<IsoDateTime>;
  error: Nullable<ErrorResponse | Record<string, unknown>>;
}

export interface TerminalSnapshotResponse {
  session_id: string;
  latest_seq: number;
  truncated: boolean;
  screen_text: string;
}

export interface TerminalReplayCursor {
  latestSeq: number;
  windowStartSeq: number;
  resumeFromSeq: number;
}

export interface TerminalProtocolInfo {
  output: "raw" | "framed" | string;
}

export type TerminalClientFrame =
  | { type: "auth"; token: string }
  | { type: "input_text"; data: string; clientMessageId?: string }
  | { type: "submit_line"; data: string; clientMessageId?: string }
  | { type: "resize"; cols: number; rows: number }
  | { type: "ping" };

export type TerminalInputEvent =
  | {
      kind: "key";
      phase: "down" | "up" | string;
      key: string;
      code: string;
      mods: number;
      repeat: boolean;
    }
  | {
      kind: "mouse";
      phase: "down" | "up" | "move" | "wheel" | string;
      button: number;
      x: number;
      y: number;
      mods: number;
    }
  | { kind: "paste"; data: string };

export interface TerminalOutputFrame {
  seq: string;
  payload: Uint8Array;
}

export type TerminalServerFrame =
  | {
      type: "ready";
      sessionId: string;
      readOnly: boolean;
      replay: TerminalReplayCursor;
      protocol: TerminalProtocolInfo;
      summary: Nullable<SessionSummary>;
    }
  | { type: "replay_truncated" }
  | { type: "error"; code: string; message: Nullable<string> }
  | { type: "overloaded"; retryAfterMs: number }
  | {
      type: "input_ack";
      clientMessageId: Nullable<string>;
      delivered: boolean;
      method: string;
      message: Nullable<string>;
    }
  | ControlEventFrame
  | LifecycleEventFrame
  | { type: "event_stream_lagged"; stream: string; skipped: number }
  | { type: "pong" }
  | { type: "unknown"; raw?: unknown; rawType?: Nullable<string> };

export type ControlEventName =
  | "session_state"
  | "session_title"
  | "session_skill"
  | "thought_update"
  | string;

export interface SessionStatePayload {
  state: SessionState;
  previous_state: SessionState;
  current_command: Nullable<string>;
  state_evidence: StateEvidence;
  transport_health: TransportHealth;
  exit_reason?: Nullable<string>;
  at: IsoDateTime;
}

export interface SessionTitlePayload {
  title: string;
  at: IsoDateTime;
}

export interface ThoughtUpdatePayload {
  thought: Nullable<string>;
  token_count: number;
  context_limit: number;
  thought_state: ThoughtState;
  thought_source: ThoughtSource;
  rest_state: RestState;
  commit_candidate: boolean;
  action_cues: ActionCue[];
  objective_changed: boolean;
  bubble_precedence: string;
  persistence_degraded: boolean;
  at: IsoDateTime;
}

export interface SessionSkillPayload {
  last_skill: Nullable<string>;
  at: IsoDateTime;
}

export type KnownControlEventPayload =
  | SessionStatePayload
  | SessionTitlePayload
  | ThoughtUpdatePayload
  | SessionSkillPayload;

export interface ControlEventFrame {
  type: "control_event";
  event: ControlEventName;
  sessionId: string;
  payload: KnownControlEventPayload | JsonValue | unknown;
}

export type LifecycleEventFrame =
  | {
      type: "lifecycle_event";
      event: "session_created";
      sessionId: string;
      reason: Nullable<string>;
      summary: Nullable<SessionSummary>;
      repoTheme: Nullable<RepoTheme | Record<string, unknown>>;
    }
  | {
      type: "lifecycle_event";
      event: "session_deleted" | string;
      sessionId: string;
      reason: Nullable<string>;
      deleteMode: Nullable<string>;
      tmuxSessionAlive: Nullable<boolean>;
      summary?: Nullable<SessionSummary>;
      repoTheme?: Nullable<RepoTheme | Record<string, unknown>>;
    };

export interface RepoActionStatus {
  kind: RepoActionKind;
  state: RepoActionState;
  detail?: Nullable<string>;
}

export interface LaunchPathMapping {
  local_prefix: string;
  remote_prefix: string;
}

export interface LaunchTargetSummary {
  id: string;
  label: string;
  kind: string;
  base_url: Nullable<string>;
  auth_token_env: Nullable<string>;
  path_mappings: LaunchPathMapping[];
}

export interface DirEntry {
  name: string;
  has_children: boolean;
  is_running: Nullable<boolean>;
  repo_dirty: Nullable<boolean>;
  repo_action: Nullable<RepoActionStatus | Record<string, unknown>>;
  group: Nullable<string>;
  groups: string[];
  full_path: Nullable<string>;
  has_restart: Nullable<boolean>;
  open_url: Nullable<string>;
}

export interface DirListResponse {
  path: string;
  entries: DirEntry[];
  overlay_label: Nullable<string>;
  groups: string[];
  launch_targets: LaunchTargetSummary[];
  default_launch_target: Nullable<string>;
}

export interface DirRepoSearchResponse {
  roots: string[];
  entries: DirEntry[];
}

export interface MermaidArtifactResponse {
  session_id: string;
  available: boolean;
  path: Nullable<string>;
  updated_at: Nullable<IsoDateTime>;
  source: Nullable<string>;
  error: Nullable<string>;
  slice_name: Nullable<string>;
  plan_files: string[];
}

export interface PlanFileResponse {
  session_id: string;
  name: string;
  content: Nullable<string>;
  error: Nullable<string>;
}

export interface NativeDesktopStatusResponse {
  supported: boolean;
  platform: Nullable<string>;
  app_id: Nullable<NativeDesktopApp>;
  ghostty_mode: Nullable<GhosttyOpenMode>;
  app: Nullable<string>;
  reason: Nullable<string>;
}

export interface NativeDesktopConfigRequest {
  app: NativeDesktopApp;
}

export interface NativeDesktopModeRequest {
  mode: GhosttyOpenMode;
}

export interface NativeDesktopOpenRequest {
  session_id: string;
}

export interface NativeDesktopOpenResponse {
  session_id: string;
  status: string;
  pane_id: Nullable<string>;
}

export interface NativeAttentionGroupOpenResponse {
  session_id: string;
  tmux_name: string;
  session_count: number;
  session_ids: string[];
  backlog_session_ids: string[];
  status: string;
  focused: boolean;
  pane_id: Nullable<string>;
  attach_command: Nullable<string>;
}

export interface ThoughtConfig {
  enabled: boolean;
  model: string;
  backend: string;
  cadence_hot_ms: number;
  cadence_warm_ms: number;
  cadence_cold_ms: number;
  agent_prompt: Nullable<string>;
  terminal_prompt: Nullable<string>;
}

export interface DaemonDefaults {
  model: string;
  backend: string;
  agent_prompt: string;
  terminal_prompt: string;
}

export interface ThoughtConfigBackendMetadata {
  key: string;
  label: string;
  model_presets_hint: string;
  model_presets: string[];
}

export interface ThoughtConfigUiMetadata {
  backends: ThoughtConfigBackendMetadata[];
}

export type ThoughtConfigResponse =
  | (ThoughtConfig & {
      daemon_defaults: Nullable<DaemonDefaults>;
      ui: ThoughtConfigUiMetadata;
      version: number;
    })
  | {
      config: ThoughtConfig;
      daemon_defaults: Nullable<DaemonDefaults>;
      ui: ThoughtConfigUiMetadata;
      version?: number;
    };

export interface ThoughtConfigProbeResponse {
  ok: boolean;
  llm_calls: number;
  message: Nullable<string>;
  last_backend_error: Nullable<string>;
}

export interface AgentContextActionSummary {
  tool: string;
  detail: Nullable<string>;
}

export interface SessionAgentTurn {
  id: string;
  source: string;
  text: string;
  byte_start: number;
  byte_end: number;
  order: number;
  timestamp: Nullable<string>;
}

export interface SessionTranscriptRecord {
  id: string;
  source: string;
  kind: string;
  role: Nullable<string>;
  summary: string;
  raw: string;
  byte_start: number;
  byte_end: number;
  timestamp: Nullable<string>;
  truncated: boolean;
}

export interface SessionAgentContextResponse {
  session_id: string;
  available: boolean;
  tool: Nullable<string>;
  cwd: string;
  user_task: Nullable<string>;
  turns: SessionAgentTurn[];
  current_tool: Nullable<AgentContextActionSummary>;
  recent_actions: AgentContextActionSummary[];
  token_count: number;
  context_limit: number;
  message: Nullable<string>;
}

export interface SessionPaneTailResponse {
  session_id: string;
  text: string;
}

export interface SessionTranscriptResponse {
  session_id: string;
  available: boolean;
  tool: Nullable<string>;
  cwd: string;
  selected_turn_id: Nullable<string>;
  selected_turn: Nullable<SessionAgentTurn>;
  next_cursor: number;
  records: SessionTranscriptRecord[];
  turns: SessionAgentTurn[];
  message: Nullable<string>;
}

export interface SessionTimelinePinnedItem {
  title: string;
  summary: string;
  source: string;
  event_id?: Nullable<string>;
}

export interface SessionTimelinePinned {
  task?: Nullable<SessionTimelinePinnedItem>;
  current_action?: Nullable<SessionTimelinePinnedItem>;
  diff?: Nullable<SessionTimelinePinnedItem>;
  pane_tail?: Nullable<SessionTimelinePinnedItem>;
  artifact?: Nullable<SessionTimelinePinnedItem>;
}

export interface SessionTimelineEvent {
  id: string;
  kind: string;
  source: string;
  title: string;
  summary: string;
  timestamp: Nullable<IsoDateTime>;
  order: Nullable<number>;
  detail: Nullable<string>;
}

export interface SessionTimelineResponse {
  session_id: string;
  available: boolean;
  cwd: string;
  tool: Nullable<string>;
  events: SessionTimelineEvent[];
  pinned: SessionTimelinePinned | Record<string, unknown>;
  message: Nullable<string>;
}

export interface SessionSkillSummary {
  name: string;
  description: Nullable<string>;
  state: Nullable<string>;
  availability: Nullable<string>;
  layer: Nullable<string>;
  source_bucket: Nullable<string>;
  source: Nullable<string>;
  path: Nullable<string>;
}

export interface SessionSkillIssue {
  skill: Nullable<string>;
  action: Nullable<string>;
  hint: Nullable<string>;
  source_path: Nullable<string>;
  message: string;
}

export interface SessionSkillListResponse {
  session_id: string;
  source: string;
  cwd: string;
  available: boolean;
  query: Nullable<string>;
  skills: SessionSkillSummary[];
  issues: SessionSkillIssue[];
  message: Nullable<string>;
}

export interface SessionGitDiffHunkSummary {
  header: string;
  added_lines: number;
  removed_lines: number;
}

export interface SessionGitDiffFileSummary {
  path: string;
  old_path: Nullable<string>;
  source: string;
  change: string;
  added_lines: number;
  removed_lines: number;
  truncated: boolean;
  hunks: SessionGitDiffHunkSummary[];
}

export interface SessionGitDiffResponse {
  session_id: string;
  available: boolean;
  cwd: string;
  repo_root: Nullable<string>;
  status_short: string;
  unstaged_diff: string;
  staged_diff: string;
  truncated: boolean;
  message: Nullable<string>;
  files: SessionGitDiffFileSummary[];
}

export interface SettledFulfilled<T> {
  status: "fulfilled";
  value: T;
}

export interface SettledRejected {
  status: "rejected";
  reason: unknown;
}

export type SettledResult<T> = SettledFulfilled<T> | SettledRejected;

export interface WorkbenchWidgetResults {
  timelineResult?: SettledResult<Nullable<SessionTimelineResponse>>;
  skillsResult?: SettledResult<Nullable<SessionSkillListResponse>>;
  tailResult?: SettledResult<Nullable<SessionPaneTailResponse>>;
  transcriptResult?: SettledResult<Nullable<SessionTranscriptResponse>>;
  artifactResult?: SettledResult<Nullable<MermaidArtifactResponse>>;
  diffResult?: SettledResult<Nullable<SessionGitDiffResponse>>;
}

export interface WorkbenchWidgetsState {
  sessionId: Nullable<string>;
  loading: boolean;
  timeline: Nullable<SessionTimelineResponse>;
  skills: Nullable<SessionSkillListResponse>;
  paneTail: Nullable<SessionPaneTailResponse>;
  transcript: Nullable<SessionTranscriptResponse>;
  transcriptTurnId: string;
  transcriptNextCursor: number;
  artifact: Nullable<MermaidArtifactResponse>;
  gitDiff: Nullable<SessionGitDiffResponse>;
  error: string;
  requestSeq: number;
  lastLoadedAt: number;
  lastHtml: string;
}

export interface TrogdorReadProgress {
  [clawgKey: string]: number;
}

export interface TrogdorDismissedClawgs {
  [clawgKey: string]: boolean;
}

export interface TrogdorReaderContractState {
  hoveredSessionId: Nullable<string>;
  wpm: number;
  reading: boolean;
  readerClawgKey: string;
  readerStartIndex: number;
  readerStartedAt: number;
  readerElapsedMs: number;
  readProgress: TrogdorReadProgress;
  dismissedClawgs: TrogdorDismissedClawgs;
  burntSessionIds: string[];
  awaitingSessionIds: string[];
}

export interface TrogdorSurfaceSession {
  sessionId: string;
  name: string;
  state: string;
  displayState: string;
  stateTrustLabel: string;
  stateConfidence: string;
  stateObserved: boolean;
  restLabel: string;
  transportLabel: string;
  transportKey: string;
  toolLabel: string;
  cwdLabel: string;
  fullCwd: string;
  canonicalCwd: string;
  thoughtLabel: string;
  clawgText: string;
  thoughtUpdatedAt: string;
  objectiveChangedAt: string;
  contextLabel: string;
  skillLabel: string;
  activityLabel: string;
  commandLabel: string;
  attachedLabel: string;
  commitCandidate: boolean;
  actionCues: ActionCue[];
  operatorPressure: Nullable<OperatorPressure>;
  batchSendSessionIds: string[];
  repoKey: string;
  repoLabel: string;
  targetKey: string;
  targetLabel: string;
  stateKey: string;
  readinessKey: string;
  readinessLabel: string;
  isStale: boolean;
  clawgReadIndex: number;
  clawgWordCount: number;
  trogdorAwaitingUser: boolean;
  trogdorBurnt: boolean;
  trogdorDismissed: boolean;
  trogdorSwordsmanVisible: boolean;
}

export interface TrogdorRepoGroup {
  key: string;
  label: string;
  sessions: TrogdorSurfaceSession[];
  pressure: number;
  reason: string;
}

export interface TrogdorSummary {
  score: string;
  level: number;
  actionCues: number;
}

export interface TrogdorDragonPose {
  x: number;
  y: number;
  direction: "left" | "right" | string;
  bodyFrame: string;
  walkX: string;
  walkY: string;
  heated: boolean;
  firing: boolean;
}

export interface TrogdorReaderDisplayState {
  bannerText: string;
  readComplete: boolean;
}

export interface TrogdorDerivedPresentationState {
  sessions: TrogdorSurfaceSession[];
  groups: TrogdorRepoGroup[];
  summary: TrogdorSummary;
  dragonPose: TrogdorDragonPose;
  reader: TrogdorReaderDisplayState;
  readOnly: boolean;
}

export interface TrogdorIslandInput {
  sessions: SessionSummary[];
  operatorPressure: OperatorPressureResponse;
  reader: TrogdorReaderContractState;
  readOnly: boolean;
  selectedSessionId: Nullable<string>;
}

export interface TrogdorSessionActionPayload {
  type: "session";
  sessionId: string;
  label: string;
}

export interface TrogdorGroupActionPayload {
  type: "group";
  sessionIds: string[];
  label: string;
}

export interface TrogdorLaunchActionPayload {
  cwd: string;
}

export interface TrogdorSessionIdActionPayload {
  sessionId: string;
}

export type TrogdorActionPayload =
  | TrogdorSessionActionPayload
  | TrogdorGroupActionPayload
  | TrogdorLaunchActionPayload
  | TrogdorSessionIdActionPayload
  | null;

export type TrogdorActionZone =
  | { type: "trogdor_agent"; sessionId: string; disabled?: boolean; rect?: unknown }
  | { type: "trogdor_reader"; sessionId?: string; disabled?: boolean; rect?: unknown }
  | {
      type: "action";
      actionId: TrogdorActionId | string;
      sessionId?: string;
      sessionIds?: string[];
      label?: string;
      cwd?: string;
      disabled?: boolean;
      rect?: unknown;
      payload?: TrogdorActionPayload;
    };

export interface TrogdorIslandOutput {
  html: string;
  signature: string;
  actionZones: TrogdorActionZone[];
}

export interface SurfaceModel {
  cols: number;
  rows: number;
  focusLayout: boolean;
  followPublishedSelection: boolean;
  connectionLabel: string;
  connectionMuted: boolean;
  modeLabel: string;
  modeMuted: boolean;
  searchLabel: string;
  searchMuted: boolean;
  utilityLabel: string;
  utilityMuted: boolean;
  searchQuery: string;
  selectMode: boolean;
  readOnly: boolean;
  frankenTermAvailable: boolean;
  terminalReady: boolean;
  snapshotFallback: boolean;
  activeSheet: string;
  hoveredLinkUrl: string;
  hoveredTrogdorSessionId: string;
  trogdorAtlasOpen: boolean;
  trogdorWpm: number;
  trogdorReading: boolean;
  trogdorReaderStartIndex: number;
  trogdorReaderElapsedMs: number;
  sessions: TrogdorSurfaceSession[];
  allSessionCount: number;
  fleetFilter: { kind: string; key: string };
  fleetLens: FleetLensSummary;
  fleetChips: Array<{ label: string; kind: string; key: string; active: boolean }>;
  selectedSessionId: Nullable<string>;
  publishedSessionId: Nullable<string>;
  publishedAtLabel: string;
  currentSession: Nullable<TrogdorSurfaceSession>;
}

export type SurfaceActionZone =
  | { type: "session"; sessionId: string; disabled?: boolean; rect?: unknown }
  | { type: "trogdor_agent"; sessionId: string; disabled?: boolean; rect?: unknown }
  | { type: "trogdor_reader"; sessionId?: string; disabled?: boolean; rect?: unknown }
  | { type: "action"; actionId: string; disabled?: boolean; rect?: unknown; payload?: unknown };

export type SurfaceActionDispatchPlan =
  | { type: "ignore" }
  | { type: "select_session"; sessionId: string }
  | { type: "open_trogdor_agent_terminal"; sessionId: string }
  | { type: "trogdor_read_toggle" }
  | { type: "trogdor_wpm"; actionId: "trogdor_wpm_down" | "trogdor_wpm_up" | string }
  | { type: "toggle_trogdor_atlas" }
  | { type: "open_send_sheet_for_zone" }
  | { type: "open_create_sheet_for_zone_cwd" }
  | { type: "select_then_open_mermaid_for_zone" }
  | { type: "select_then_launch_commit_for_zone" }
  | { type: "open_sheet"; sheetId: string }
  | { type: "open_send_sheet_for_current_session"; payload: { type: "session"; sessionId: string; label: string } }
  | { type: "open_thought_config" }
  | { type: "open_native" }
  | { type: "open_mermaid" }
  | { type: "launch_commit" }
  | { type: "open_sheet"; sheetId: "create" }
  | { type: "toggle_follow" }
  | { type: "toggle_select" }
  | { type: "copy_selection" }
  | { type: "focus_terminal" }
  | { type: "refresh" };

export type SurfaceActionExecutionPlan =
  | { type: "ignore" }
  | { type: "open_send_sheet"; payload: unknown }
  | { type: "open_create_sheet_for_cwd"; cwd: unknown }
  | { type: "select_then_open_mermaid"; sessionId: unknown }
  | { type: "select_then_launch_commit"; sessionId: unknown }
  | { type: "open_sheet"; sheetId: string }
  | { type: "open_thought_config" }
  | { type: "open_native" }
  | { type: "open_mermaid" }
  | { type: "launch_commit" }
  | { type: "toggle_follow" }
  | { type: "toggle_select" }
  | { type: "copy_selection" }
  | { type: "refresh" }
  | {
      type: "focus_terminal";
      atlasTransitionAction: "close";
      focusOptions: { preventScroll: boolean };
      statusMessage: unknown;
      statusError: unknown;
      statusTimeoutMs: unknown;
    }
  | {
      type: "apply_trogdor_reader";
      session: unknown;
      readAgain: unknown;
      statePatch: Record<string, unknown>;
      restartClock: boolean;
      resetAfterWpmChange: boolean;
      syncReaderTimer: boolean;
    };

export function normalizeBootPayload(value: unknown): BootPayload;
export function normalizeStateEvidence(value: unknown): StateEvidence;
export function normalizeActionCue(value: unknown): ActionCue;
export function normalizeSessionSummary(value: unknown): Nullable<SessionSummary>;
export function normalizeSessionListResponse(value: unknown): SessionListResponse;
export function normalizePublishedSelectionResponse(value: unknown): PublishedSelectionResponse;
export function normalizeTerminalServerFrame(value: unknown): TerminalServerFrame;
export function normalizeControlEventFrame(value: unknown): ControlEventFrame;
export function normalizeLifecycleEventFrame(value: unknown): LifecycleEventFrame;
export function normalizeOperatorPressure(value: unknown): OperatorPressure;
export function normalizeOperatorPressureSession(value: unknown): OperatorPressureSession;
export function normalizeOperatorPressureResponse(value: unknown): OperatorPressureResponse;
export function normalizeTerminalSnapshotResponse(value: unknown): TerminalSnapshotResponse;
export function normalizeDirEntry(value: unknown): DirEntry;
export function normalizeLaunchTargetSummary(value: unknown): LaunchTargetSummary;
export function normalizeDirListResponse(value: unknown): DirListResponse;
export function normalizeDirRepoSearchResponse(value: unknown): DirRepoSearchResponse;
export function normalizeMermaidArtifactResponse(value: unknown): MermaidArtifactResponse;
export function normalizePlanFileResponse(value: unknown): PlanFileResponse;
export function normalizeNativeDesktopStatusResponse(value: unknown): NativeDesktopStatusResponse;
export function normalizeNativeDesktopOpenResponse(value: unknown): NativeDesktopOpenResponse;
export function normalizeNativeAttentionGroupOpenResponse(value: unknown): NativeAttentionGroupOpenResponse;
export function normalizeThoughtConfig(value: unknown): ThoughtConfig;
export function normalizeThoughtConfigResponse(value: unknown): ThoughtConfigResponse;
export function normalizeThoughtConfigProbeResponse(value: unknown): ThoughtConfigProbeResponse;
export function normalizeSessionPaneTailResponse(value: unknown): SessionPaneTailResponse;
export function normalizeAgentContextActionSummary(value: unknown): AgentContextActionSummary;
export function normalizeSessionAgentTurn(value: unknown): SessionAgentTurn;
export function normalizeSessionTranscriptRecord(value: unknown): SessionTranscriptRecord;
export function normalizeSessionAgentContextResponse(value: unknown): SessionAgentContextResponse;
export function normalizeSessionTranscriptResponse(value: unknown): SessionTranscriptResponse;
export function normalizeSessionTimelineResponse(value: unknown): SessionTimelineResponse;
export function normalizeSessionSkillListResponse(value: unknown): SessionSkillListResponse;
export function normalizeSessionGitDiffResponse(value: unknown): SessionGitDiffResponse;
export function normalizeWorkbenchWidgetResults(results?: WorkbenchWidgetResults): WorkbenchWidgetResults;
export function normalizeTrogdorSurfaceSession(value: unknown): TrogdorSurfaceSession;
export function normalizeSurfaceModel(value: unknown): SurfaceModel;
