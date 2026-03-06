// ---- Enums & Literals ----

export type SessionState = "idle" | "busy" | "error" | "attention" | "exited";
export type SessionDeleteMode = "detach_bridge" | "kill_tmux";

export type TransportHealth =
  | "healthy"
  | "degraded"
  | "overloaded"
  | "disconnected";

export type ThoughtState = "active" | "holding" | "sleeping";
export type ThoughtSource = "carry_forward" | "llm" | "static_sleeping";
export type BubblePrecedence = "thought_first";

export type SpawnTool = "claude" | "codex";
export type SkillRegistryTool = "claude" | "codex";

// ---- REST Payloads ----

export interface SpritePack {
  active: string;
  drowsy: string;
  sleeping: string;
  deep_sleep: string;
}

export interface SessionSummary {
  session_id: string;
  tmux_name: string;
  state: SessionState;
  current_command: string | null;
  cwd: string;
  tool: string | null;
  token_count: number;
  context_limit: number;
  thought: string | null;
  thought_state: ThoughtState;
  thought_source: ThoughtSource;
  thought_updated_at: string | null;
  last_skill: string | null;
  is_stale: boolean;
  attached_clients: number;
  transport_health: TransportHealth;
  last_activity_at: string; // ISO 8601
  sprite_pack_id: string | null;
}

export interface ThoughtPolicy {
  lifecycle_mode: string;
  cadence_ms: {
    hot: number;
    warm: number;
    cold: number;
  };
  sleeping_after_ms: number;
  bubble_precedence: BubblePrecedence;
}

export interface ThoughtConfig {
  enabled: boolean;
  model: string;
  cadence_hot_ms: number;
  cadence_warm_ms: number;
  cadence_cold_ms: number;
  agent_prompt?: string | null;
  terminal_prompt?: string | null;
}

export interface TerminalSnapshot {
  session_id: string;
  latest_seq: number;
  truncated: boolean;
  screen_text: string;
}

export interface SessionPaneTailResponse {
  session_id: string;
  text: string;
}

export interface BootstrapResponse {
  server_time: string; // ISO 8601
  auth_mode: string;
  realtime_url: string;
  workspace_history_mode: string;
  poll_fallback_ms: number;
  thought_tick_ms: number;
  thoughts_enabled_default: boolean;
  terminal_cache_ttl_ms: number;
  session_delete_mode: SessionDeleteMode;
  legacy_parity_locked: boolean;
  thought_policy: ThoughtPolicy;
  thought_config?: ThoughtConfig;
  sessions: SessionSummary[];
  sprite_packs: Record<string, SpritePack>;
}

export interface NativeDesktopStatus {
  supported: boolean;
  platform?: string | null;
  app?: string | null;
  reason?: string | null;
}

export interface NativeDesktopOpenResponse {
  session_id: string;
  status: "created" | "focused" | string;
  pane_id?: string | null;
}

export interface SessionListResponse {
  sessions: SessionSummary[];
  version: number;
}

export interface CreateSessionResponse {
  session: SessionSummary;
  sprite_pack?: SpritePack;
}

export interface ErrorResponse {
  code: string;
  message?: string;
}

export interface DirEntry {
  name: string;
  has_children: boolean;
  is_running?: boolean;
}

export interface DirListResponse {
  path: string;
  entries: DirEntry[];
}

export interface DirRestartResponse {
  ok: boolean;
  path: string;
  services: string[];
}

export interface SkillSummary {
  name: string;
  description?: string;
}

export interface SkillListResponse {
  tool: SkillRegistryTool;
  skills: SkillSummary[];
}

// ---- Control Events (Server -> Client JSON) ----

export interface ControlEvent {
  event: string;
  session_id: string;
  payload: unknown;
}

export interface SessionStatePayload {
  state: SessionState;
  previous_state: SessionState;
  current_command: string | null;
  transport_health: TransportHealth;
  at: string;
}

export interface SessionTitlePayload {
  title: string;
  at: string;
}

export interface ThoughtUpdatePayload {
  thought: string | null;
  token_count: number;
  context_limit: number;
  thought_state: ThoughtState;
  thought_source: ThoughtSource;
  objective_changed: boolean;
  bubble_precedence: BubblePrecedence;
  at: string;
}

export interface SessionSkillPayload {
  last_skill: string | null;
  at: string;
}

export interface SessionCreatedPayload {
  reason: string; // "startup_discovery" | "api_create"
  session: SessionSummary;
  sprite_pack?: SpritePack;
}

export interface SessionDeletedPayload {
  reason: string;
  delete_mode: SessionDeleteMode;
  tmux_session_alive: boolean;
  at: string;
}

export interface ReplayTruncatedPayload {
  code: string;
  requested_resume_from_seq: number;
  replay_window_start_seq: number;
  latest_seq: number;
}

export interface SessionOverloadedPayload {
  code: string;
  queue_depth: number;
  queue_bytes: number;
  retry_after_ms: number;
}

export interface SessionSubscriptionPayload {
  state: "subscribed" | "unsubscribed";
  resume_from_seq?: number;
  latest_seq: number;
  replay_window_start_seq: number;
  at: string;
}

export interface ControlErrorPayload {
  code: string;
  message: string;
  request_id?: string;
}

// ---- Client -> Server Control (JSON) ----

export interface ClientControlMessage {
  type: string;
  request_id?: string;
  payload: unknown;
}

export interface SubscribeSessionPayload {
  session_id: string;
  resume_from_seq?: number;
}

export interface UnsubscribeSessionPayload {
  session_id: string;
}

export interface ResizePayload {
  session_id: string;
  cols: number;
  rows: number;
}

export interface DismissAttentionPayload {
  session_id: string;
}

// ---- Binary Frame Opcodes ----

export const Opcodes = {
  TERMINAL_INPUT: 0x10,
  TERMINAL_OUTPUT: 0x11,
} as const;
