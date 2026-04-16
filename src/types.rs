use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Busy,
    Error,
    Attention,
    Exited,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtState {
    Active,
    Holding,
    Sleeping,
}

impl Default for ThoughtState {
    fn default() -> Self {
        Self::Holding
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtSource {
    CarryForward,
    Llm,
    StaticSleeping,
}

impl Default for ThoughtSource {
    fn default() -> Self {
        Self::CarryForward
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestState {
    Active,
    Drowsy,
    Sleeping,
    DeepSleep,
}

impl Default for RestState {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubblePrecedence {
    ThoughtFirst,
}

impl Default for BubblePrecedence {
    fn default() -> Self {
        Self::ThoughtFirst
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportHealth {
    Healthy,
    Degraded,
    Overloaded,
    Disconnected,
}

impl Default for TransportHealth {
    fn default() -> Self {
        Self::Healthy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnTool {
    Claude,
    Codex,
}

impl SpawnTool {
    pub fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    // Used by swimmers-tui picker; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    // Used by swimmers-tui app; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Claude,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoActionKind {
    Commit,
    Restart,
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoActionState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoActionStatus {
    pub kind: RepoActionKind,
    pub state: RepoActionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDesktopApp {
    Iterm,
    Ghostty,
}

impl NativeDesktopApp {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ghostty" => Self::Ghostty,
            "iterm" | "iterm2" | "i_term" | "i-term" => Self::Iterm,
            _ => Self::Iterm,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Iterm => "iTerm",
            Self::Ghostty => "Ghostty",
        }
    }

    // Used by swimmers-tui app; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            Self::Iterm => Self::Ghostty,
            Self::Ghostty => Self::Iterm,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GhosttyOpenMode {
    Swap,
    Add,
}

impl GhosttyOpenMode {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "add" | "split" | "new" => Self::Add,
            _ => Self::Swap,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Swap => "swap",
            Self::Add => "add",
        }
    }

    // Used by swimmers-tui app; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            Self::Swap => Self::Add,
            Self::Add => Self::Swap,
        }
    }
}

/// Per-repository Swimmer palette used by the native TUI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoTheme {
    pub body: String,
    pub outline: String,
    pub accent: String,
    pub shirt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub tmux_name: String,
    pub state: SessionState,
    pub current_command: Option<String>,
    pub cwd: String,
    pub tool: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    pub thought: Option<String>,
    #[serde(default = "default_thought_state")]
    pub thought_state: ThoughtState,
    #[serde(default = "default_thought_source")]
    pub thought_source: ThoughtSource,
    #[serde(default)]
    pub thought_updated_at: Option<DateTime<Utc>>,
    #[serde(default = "default_rest_state")]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective_changed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_skill: Option<String>,
    pub is_stale: bool,
    pub attached_clients: u32,
    pub transport_health: TransportHealth,
    pub last_activity_at: DateTime<Utc>,
    /// Key into `SessionListResponse.repo_themes`; absent when no supported
    /// repo theme cache directory exists for this session cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub session_id: String,
    pub latest_seq: u64,
    pub truncated: bool,
    pub screen_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPaneTailResponse {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MermaidArtifactResponse {
    pub session_id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanFileResponse {
    pub session_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDesktopStatusResponse {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<NativeDesktopApp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ghostty_mode: Option<GhosttyOpenMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDesktopConfigRequest {
    pub app: NativeDesktopApp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDesktopModeRequest {
    pub mode: GhosttyOpenMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDesktopOpenRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDesktopOpenResponse {
    pub session_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishSelectionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishedSelectionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub spawn_tool: Option<SpawnTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_request: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub has_children: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_running: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_dirty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_action: Option<RepoActionStatus>,
    /// When set, this entry represents a virtual directory group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Explicit absolute path for entries whose parent differs from the
    /// response `path` (e.g. entries inside a group listing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_path: Option<String>,
    /// Whether this entry has a restart command available from the overlay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_restart: Option<bool>,
    /// URL to open in a browser (local dev URL from the overlay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirListResponse {
    pub path: String,
    pub entries: Vec<DirEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirRestartRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirRestartResponse {
    pub ok: bool,
    pub path: String,
    pub services: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirRepoActionRequest {
    pub path: String,
    pub kind: RepoActionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirRepoActionResponse {
    pub ok: bool,
    pub path: String,
    pub status: RepoActionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListResponse {
    pub tool: String,
    pub skills: Vec<SkillSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session: SessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme: Option<RepoTheme>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtConfigUiMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backends: Vec<ThoughtConfigBackendMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtConfigBackendMetadata {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model_presets_hint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_presets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtConfigResponse {
    #[serde(flatten)]
    pub config: ThoughtConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_defaults: Option<DaemonDefaults>,
    #[serde(default, skip_serializing_if = "is_default_thought_config_ui")]
    pub ui: ThoughtConfigUiMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInputRequest {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInputResponse {
    pub ok: bool,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummary>,
    pub version: u64,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub repo_themes: HashMap<String, RepoTheme>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// --- Control Events (Server -> Client JSON) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEvent {
    pub event: String,
    pub session_id: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatePayload {
    pub state: SessionState,
    pub previous_state: SessionState,
    pub current_command: Option<String>,
    pub transport_health: TransportHealth,
    /// Reason for session exit: "process_exit", "startup_missing_tmux", or null
    /// for normal state transitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_reason: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTitlePayload {
    pub title: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtUpdatePayload {
    pub thought: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    #[serde(default = "default_thought_state")]
    pub thought_state: ThoughtState,
    #[serde(default = "default_thought_source")]
    pub thought_source: ThoughtSource,
    #[serde(default = "default_rest_state")]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    #[serde(default)]
    pub objective_changed: bool,
    #[serde(default = "default_bubble_precedence")]
    pub bubble_precedence: BubblePrecedence,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSkillPayload {
    #[serde(default)]
    pub last_skill: Option<String>,
    pub at: DateTime<Utc>,
}

fn default_thought_state() -> ThoughtState {
    ThoughtState::Holding
}

fn default_thought_source() -> ThoughtSource {
    ThoughtSource::CarryForward
}

fn default_rest_state() -> RestState {
    RestState::Active
}

fn default_bubble_precedence() -> BubblePrecedence {
    BubblePrecedence::ThoughtFirst
}

fn is_default_thought_config_ui(value: &ThoughtConfigUiMetadata) -> bool {
    value.backends.is_empty()
}

pub fn fallback_rest_state(state: SessionState, thought_state: ThoughtState) -> RestState {
    match state {
        SessionState::Exited => RestState::DeepSleep,
        SessionState::Idle => match thought_state {
            ThoughtState::Active => RestState::Active,
            ThoughtState::Holding => RestState::Drowsy,
            ThoughtState::Sleeping => RestState::Sleeping,
        },
        SessionState::Busy | SessionState::Error | SessionState::Attention => RestState::Active,
    }
}

/// An idle session stays `Active` until it has been silent for this long.
pub const REST_STATE_DROWSY_AFTER: Duration = Duration::seconds(30);

/// Compute a session's `RestState` from how long it has been silent.
///
/// This is the no-thought-daemon fallback. When the thought daemon is running
/// it publishes its own `RestState` which supersedes this value in the
/// supervisor merge path. Non-idle states (`Busy`, `Error`, `Attention`) are
/// unaffected by elapsed time — they always stay `Active` so attention-flagged
/// sessions keep their attention animation and busy sessions never "fall
/// asleep" mid-task. In this fallback path, idle sessions only age into
/// `Drowsy`; `Sleeping` is reserved for transcript-aware "waiting on user"
/// updates from the thought daemon. `Exited` stays `DeepSleep`.
///
/// Negative durations (e.g. from clock skew or future-dated
/// `last_activity_at`) resolve to `Active` — a fresh session is never sleepy.
pub fn rest_state_from_idle(
    state: SessionState,
    last_activity_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> RestState {
    match state {
        SessionState::Exited => RestState::DeepSleep,
        SessionState::Busy | SessionState::Error | SessionState::Attention => RestState::Active,
        SessionState::Idle => {
            let elapsed = now.signed_duration_since(last_activity_at);
            if elapsed >= REST_STATE_DROWSY_AFTER {
                RestState::Drowsy
            } else {
                RestState::Active
            }
        }
    }
}

#[cfg(test)]
mod rest_state_tests {
    use super::*;

    fn base() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn tc1_freshly_active_idle_stays_active() {
        let now = base();
        let last = now - Duration::seconds(5);
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, last, now),
            RestState::Active
        );
    }

    #[test]
    fn tc2_two_minutes_silent_becomes_drowsy() {
        let now = base();
        let last = now - Duration::minutes(2);
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, last, now),
            RestState::Drowsy
        );
    }

    #[test]
    fn tc3_long_idle_stays_drowsy_without_thought_daemon() {
        let now = base();
        let last = now - Duration::minutes(10);
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, last, now),
            RestState::Drowsy
        );
    }

    #[test]
    fn tc4_hours_silent_stays_drowsy_without_thought_daemon() {
        let now = base();
        let last = now - Duration::hours(2);
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, last, now),
            RestState::Drowsy
        );
    }

    #[test]
    fn tc5_busy_session_ignores_idle_duration() {
        let now = base();
        let last = now - Duration::hours(1);
        assert_eq!(
            rest_state_from_idle(SessionState::Busy, last, now),
            RestState::Active
        );
    }

    #[test]
    fn tc5b_attention_session_ignores_idle_duration() {
        let now = base();
        let last = now - Duration::minutes(10);
        assert_eq!(
            rest_state_from_idle(SessionState::Attention, last, now),
            RestState::Active,
            "attention-flagged sessions must keep animating until dismissed"
        );
    }

    #[test]
    fn tc7_exited_stays_deep_sleep() {
        let now = base();
        let last = now - Duration::seconds(1);
        assert_eq!(
            rest_state_from_idle(SessionState::Exited, last, now),
            RestState::DeepSleep
        );
    }

    #[test]
    fn tc11_future_last_activity_resolves_to_active() {
        let now = base();
        let last = now + Duration::minutes(1);
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, last, now),
            RestState::Active,
            "clock skew must not panic or sleep the session"
        );
    }

    #[test]
    fn threshold_boundaries() {
        let now = base();
        // Exactly at drowsy threshold → Drowsy
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, now - REST_STATE_DROWSY_AFTER, now),
            RestState::Drowsy
        );
        // Long-idle fallback remains Drowsy; sleeping requires transcript state.
        assert_eq!(
            rest_state_from_idle(SessionState::Idle, now - Duration::hours(3), now),
            RestState::Drowsy
        );
    }

    #[test]
    fn fallback_rest_state_unchanged() {
        // TC-10: fallback_rest_state must keep its existing behavior for
        // preserved call sites (stale-session path, test fixtures).
        assert_eq!(
            fallback_rest_state(SessionState::Exited, ThoughtState::Holding),
            RestState::DeepSleep
        );
        assert_eq!(
            fallback_rest_state(SessionState::Idle, ThoughtState::Holding),
            RestState::Drowsy
        );
        assert_eq!(
            fallback_rest_state(SessionState::Busy, ThoughtState::Holding),
            RestState::Active
        );
    }
}

// --- WebSocket Push Protocol Types ---
// TODO: re-evaluate when the push-based WS API is wired up; these are schema
// types for the server→client and client→server control envelopes that will be
// constructed once the subscription path is implemented.

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreatedPayload {
    pub reason: String, // "startup_discovery" | "runtime_discovery" | "api_create"
    pub session: SessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme: Option<RepoTheme>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDeletedPayload {
    pub reason: String,
    pub delete_mode: String,
    pub tmux_session_alive: bool,
    pub at: DateTime<Utc>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayTruncatedPayload {
    pub code: String,
    pub requested_resume_from_seq: u64,
    pub replay_window_start_seq: u64,
    pub latest_seq: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOverloadedPayload {
    pub code: String,
    pub queue_depth: usize,
    pub queue_bytes: usize,
    pub retry_after_ms: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSubscriptionPayload {
    pub state: String, // "subscribed" | "unsubscribed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_from_seq: Option<u64>,
    pub latest_seq: u64,
    pub replay_window_start_seq: u64,
    pub at: DateTime<Utc>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

// --- Client -> Server Control ---

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ClientControlMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: Option<String>,
    pub payload: serde_json::Value,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeSessionPayload {
    pub session_id: String,
    pub resume_from_seq: Option<u64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct UnsubscribeSessionPayload {
    pub session_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ResizePayload {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct DismissAttentionPayload {
    pub session_id: String,
}

// --- Binary Frame Constants ---

pub mod opcodes {
    // TODO: re-evaluate when binary frame encoding is implemented in the WS handler
    #[allow(dead_code)]
    pub const TERMINAL_INPUT: u8 = 0x10;
    #[allow(dead_code)]
    pub const TERMINAL_OUTPUT: u8 = 0x11;
}

// --- Context Limits ---

pub fn context_limit_for_tool(tool: Option<&str>) -> u64 {
    match tool {
        Some("Claude Code") => 200_000,
        Some("Codex") => 192_000,
        Some("Amp") => 200_000,
        Some("OpenCode") => 128_000,
        Some("Aider") => 128_000,
        Some("Goose") => 200_000,
        Some("Cline") => 200_000,
        Some("Cursor") => 200_000,
        Some(_) => 128_000,
        None => 128_000,
    }
}

pub fn detect_tool_name(comm: &str) -> Option<&'static str> {
    let token = comm.trim().split_whitespace().next().unwrap_or(comm);
    let token = token.trim_matches(|c: char| {
        c == '"'
            || c == '\''
            || c == '`'
            || c == ','
            || c == ';'
            || c == ':'
            || c == '('
            || c == ')'
            || c == '['
            || c == ']'
            || c == '{'
            || c == '}'
    });
    let token = token.rsplit('/').next().unwrap_or(token);
    let token = token.trim_start_matches('-');

    match token.to_lowercase().as_str() {
        "claude" | "claude-code" | "claude_code" => Some("Claude Code"),
        "codex" | "codex-cli" | "codex_cli" => Some("Codex"),
        "amp" => Some("Amp"),
        "opencode" | "open-code" | "open_code" => Some("OpenCode"),
        "aider" => Some("Aider"),
        "goose" => Some("Goose"),
        "cline" => Some("Cline"),
        "cursor" => Some("Cursor"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tool_name_normalizes_aliases_and_paths() {
        assert_eq!(detect_tool_name("claude"), Some("Claude Code"));
        assert_eq!(detect_tool_name("CLAUDE"), Some("Claude Code"));
        assert_eq!(
            detect_tool_name("/usr/local/bin/claude-code"),
            Some("Claude Code")
        );
        assert_eq!(detect_tool_name("codex-cli"), Some("Codex"));
        assert_eq!(detect_tool_name("'codex'"), Some("Codex"));
    }

    #[test]
    fn detect_tool_name_ignores_unknown_tokens() {
        assert_eq!(detect_tool_name("zsh"), None);
        assert_eq!(detect_tool_name("node"), None);
        assert_eq!(detect_tool_name(""), None);
    }

    #[test]
    fn context_limit_falls_back_for_unknown_tool() {
        assert_eq!(context_limit_for_tool(None), 128_000);
        assert_eq!(context_limit_for_tool(Some("UnknownTool")), 128_000);
    }

    #[test]
    fn spawn_tool_commands_match_cli_entrypoints() {
        assert_eq!(SpawnTool::Claude.command(), "claude");
        assert_eq!(SpawnTool::Codex.command(), "codex");
    }

    #[test]
    fn create_session_response_serializes_repo_theme() {
        let theme = RepoTheme {
            body: "#B89875".into(),
            outline: "#3D2F24".into(),
            accent: "#1D1914".into(),
            shirt: "#AA9370".into(),
            sprite: Some("jelly".into()),
        };
        let resp = CreateSessionResponse {
            session: SessionSummary {
                session_id: "s1".into(),
                tmux_name: "1".into(),
                state: SessionState::Idle,
                current_command: None,
                cwd: "/tmp/proj".into(),
                tool: None,
                token_count: 0,
                context_limit: 200_000,
                thought: None,
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::CarryForward,
                thought_updated_at: None,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                objective_changed_at: None,
                last_skill: None,
                is_stale: false,
                attached_clients: 0,
                transport_health: TransportHealth::Healthy,
                last_activity_at: chrono::Utc::now(),
                repo_theme_id: Some("/tmp/proj".into()),
            },
            repo_theme: Some(theme),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(
            json.contains("\"repo_theme\""),
            "repo_theme must be present in JSON"
        );
        assert!(
            json.contains("\"sprite\":\"jelly\""),
            "repo_theme sprite must roundtrip when present"
        );

        // When repo_theme is None, the field should be omitted entirely.
        let resp_none = CreateSessionResponse {
            session: resp.session.clone(),
            repo_theme: None,
        };
        let json_none = serde_json::to_string(&resp_none).unwrap();
        assert!(
            !json_none.contains("\"repo_theme\""),
            "null repo_theme must be omitted"
        );
    }

    #[test]
    fn session_created_payload_serializes_repo_theme() {
        let theme = RepoTheme {
            body: "#B89875".into(),
            outline: "#3D2F24".into(),
            accent: "#1D1914".into(),
            shirt: "#AA9370".into(),
            sprite: Some("balls".into()),
        };
        let payload = SessionCreatedPayload {
            reason: "api_create".into(),
            session: SessionSummary {
                session_id: "s1".into(),
                tmux_name: "1".into(),
                state: SessionState::Idle,
                current_command: None,
                cwd: "/tmp".into(),
                tool: None,
                token_count: 0,
                context_limit: 200_000,
                thought: None,
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::CarryForward,
                thought_updated_at: None,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                objective_changed_at: None,
                last_skill: None,
                is_stale: false,
                attached_clients: 0,
                transport_health: TransportHealth::Healthy,
                last_activity_at: chrono::Utc::now(),
                repo_theme_id: Some("/tmp".into()),
            },
            repo_theme: Some(theme),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"repo_theme\""));

        // Deserialize roundtrip
        let parsed: SessionCreatedPayload = serde_json::from_str(&json).unwrap();
        let parsed_theme = parsed.repo_theme.expect("repo theme");
        assert_eq!(parsed_theme.body, "#B89875");
        assert_eq!(parsed_theme.sprite.as_deref(), Some("balls"));
    }
}
