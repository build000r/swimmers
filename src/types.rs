use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};

/// Browser-compatible minimum terminal columns accepted by resize messages.
pub const TERMINAL_RESIZE_MIN_COLS: u16 = 24;
/// Browser-compatible maximum terminal columns accepted by resize messages.
pub const TERMINAL_RESIZE_MAX_COLS: u16 = 240;
/// Browser-compatible minimum terminal rows accepted by resize messages.
pub const TERMINAL_RESIZE_MIN_ROWS: u16 = 12;
/// Browser-compatible maximum terminal rows accepted by resize messages.
pub const TERMINAL_RESIZE_MAX_ROWS: u16 = 120;

pub fn clamp_terminal_resize(cols: u16, rows: u16) -> (u16, u16) {
    (
        cols.clamp(TERMINAL_RESIZE_MIN_COLS, TERMINAL_RESIZE_MAX_COLS),
        rows.clamp(TERMINAL_RESIZE_MIN_ROWS, TERMINAL_RESIZE_MAX_ROWS),
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    #[default]
    Idle,
    Busy,
    Error,
    Attention,
    Exited,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtState {
    Active,
    #[default]
    Holding,
    Sleeping,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtSource {
    #[default]
    CarryForward,
    Llm,
    StaticSleeping,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestState {
    #[default]
    Active,
    Drowsy,
    Sleeping,
    DeepSleep,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BubblePrecedence {
    #[default]
    ThoughtFirst,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCue {
    pub kind: ActionCueKind,
    pub status: ActionCueStatus,
    pub source: ActionCueSource,
    pub confidence: ActionCueConfidence,
    pub evidence: Vec<String>,
}

impl ActionCue {
    pub fn expected_evidence(kind: ActionCueKind) -> &'static [&'static str] {
        match kind {
            ActionCueKind::AwaitingUser => &["awaiting_user_input"],
            ActionCueKind::CommitReady => &[
                "edit_seen",
                "validation_succeeded",
                "dirty_tree_checked_after_latest_edit",
                "commit_not_seen_after_latest_edit",
            ],
            ActionCueKind::ValidationMissingAfterEdit => &[
                "edit_seen",
                "fresh_validation_not_seen",
                "commit_not_seen_after_latest_edit",
            ],
            ActionCueKind::DirtyCheckMissing => &[
                "edit_seen",
                "validation_succeeded",
                "dirty_tree_check_not_seen_after_latest_edit",
                "commit_not_seen_after_latest_edit",
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueKind {
    AwaitingUser,
    CommitReady,
    ValidationMissingAfterEdit,
    DirtyCheckMissing,
}

impl ActionCueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingUser => "awaiting_user",
            Self::CommitReady => "commit_ready",
            Self::ValidationMissingAfterEdit => "validation_missing_after_edit",
            Self::DirtyCheckMissing => "dirty_check_missing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueStatus {
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueSource {
    Transcript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionCueConfidence {
    Deterministic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportHealth {
    #[default]
    Healthy,
    Degraded,
    Overloaded,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyHealthStatus {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyHealthSnapshot {
    pub status: DependencyHealthStatus,
    pub last_checked_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub freshness_ms: Option<u64>,
    pub details: BTreeMap<String, String>,
}

impl DependencyHealthSnapshot {
    pub fn unknown(now: DateTime<Utc>) -> Self {
        Self::new(DependencyHealthStatus::Unknown, now)
    }

    pub fn healthy(now: DateTime<Utc>) -> Self {
        Self::new(DependencyHealthStatus::Healthy, now).with_last_seen(now)
    }

    pub fn degraded(now: DateTime<Utc>, error: impl Into<String>) -> Self {
        Self::new(DependencyHealthStatus::Degraded, now).with_error(now, error)
    }

    pub fn unavailable(now: DateTime<Utc>, error: impl Into<String>) -> Self {
        Self::new(DependencyHealthStatus::Unavailable, now).with_error(now, error)
    }

    pub fn not_configured(now: DateTime<Utc>) -> Self {
        Self::new(DependencyHealthStatus::NotConfigured, now)
    }

    pub fn with_last_seen(mut self, at: DateTime<Utc>) -> Self {
        self.last_seen_at = Some(at);
        self.freshness_ms = freshness_ms(at, self.last_checked_at);
        self
    }

    pub fn with_error(mut self, at: DateTime<Utc>, error: impl Into<String>) -> Self {
        self.last_error_at = Some(at);
        self.last_error = Some(error.into());
        self
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    fn new(status: DependencyHealthStatus, now: DateTime<Utc>) -> Self {
        Self {
            status,
            last_checked_at: now,
            last_seen_at: None,
            last_error_at: None,
            last_error: None,
            freshness_ms: None,
            details: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyHealthLedger {
    pub tmux_discovery: DependencyHealthSnapshot,
    pub tmux_capture: DependencyHealthSnapshot,
    pub persistence: DependencyHealthSnapshot,
    pub thought_bridge: DependencyHealthSnapshot,
    pub native_scripts: DependencyHealthSnapshot,
    pub overlay: DependencyHealthSnapshot,
    pub remote_targets: DependencyHealthSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThoughtPersistenceBackpressureSnapshot {
    pub queue_capacity: usize,
    pub queue_depth: usize,
    pub pending_count: usize,
    pub overflow_slots: usize,
    pub queue_full_count: u64,
    pub coalesced_count: u64,
    pub dropped_count: u64,
}

fn freshness_ms(seen_at: DateTime<Utc>, checked_at: DateTime<Utc>) -> Option<u64> {
    checked_at
        .signed_duration_since(seen_at)
        .to_std()
        .ok()
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateConfidence {
    #[default]
    Low,
    Medium,
    High,
}

pub const SUMMARY_CAUSE_PERSISTENCE_STALE: &str = "persistence_stale";
pub const SUMMARY_CAUSE_STARTUP_MISSING_TMUX: &str = "startup_missing_tmux";
pub const SUMMARY_CAUSE_TMUX_RECONCILE_MISSING: &str = "tmux_reconcile_missing";
pub const SUMMARY_CAUSE_SUPERVISOR_PLACEHOLDER: &str = "supervisor_placeholder";
pub const SUMMARY_CAUSE_REMOTE_POLL_DEGRADED: &str = "remote_poll_degraded";
pub const SUMMARY_CAUSE_CACHE_DISCONNECTED: &str = "summary_cache_disconnected";
pub const SUMMARY_CAUSE_CACHE_DEGRADED: &str = "summary_cache_degraded";
pub const SUMMARY_CAUSE_CACHE_OVERLOADED: &str = "summary_cache_overloaded";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryFallbackReason {
    ChannelClosed,
    Dropped,
    Missing,
    Timeout,
}

impl SummaryFallbackReason {
    pub const fn metric_label(self) -> &'static str {
        match self {
            Self::ChannelClosed => "channel_closed",
            Self::Dropped => "dropped",
            Self::Missing => "missing",
            Self::Timeout => "timeout",
        }
    }

    /// Cached-summary fallback cause/transport-health pair for this reason, or
    /// `None` for `Missing` (which has no cached fallback). Both projections are
    /// derived from the same match so they can never disagree.
    pub const fn cached_fallback(self) -> Option<(&'static str, TransportHealth)> {
        match self {
            Self::ChannelClosed => Some((
                SUMMARY_CAUSE_CACHE_DISCONNECTED,
                TransportHealth::Disconnected,
            )),
            Self::Dropped => Some((SUMMARY_CAUSE_CACHE_DEGRADED, TransportHealth::Degraded)),
            Self::Timeout => Some((SUMMARY_CAUSE_CACHE_OVERLOADED, TransportHealth::Overloaded)),
            Self::Missing => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateEvidence {
    #[serde(default = "default_state_cause")]
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub confidence: StateConfidence,
}

impl StateEvidence {
    pub fn new(cause: impl Into<String>) -> Self {
        Self::with_observed_at(cause, Some(Utc::now()))
    }

    pub fn unobserved(cause: impl Into<String>) -> Self {
        Self::with_observed_at(cause, None)
    }

    pub fn with_observed_at(cause: impl Into<String>, observed_at: Option<DateTime<Utc>>) -> Self {
        let cause = cause.into();
        let confidence = Self::confidence_for_cause(&cause);
        Self {
            cause,
            observed_at,
            confidence,
        }
    }

    pub fn unknown() -> Self {
        Self {
            cause: default_state_cause(),
            observed_at: None,
            confidence: StateConfidence::Low,
        }
    }

    fn confidence_for_cause(cause: &str) -> StateConfidence {
        match cause {
            "osc133_command"
            | "osc133_prompt"
            | "error_pattern"
            | "process_exit"
            | SUMMARY_CAUSE_STARTUP_MISSING_TMUX
            | "liveness_no_children"
            | "liveness_has_children" => StateConfidence::High,
            "fallback_non_prompt_output"
            | "fallback_prompt_detected"
            | "output_silence_expired"
            | "local_input"
            | "dismiss_attention"
            | "error_timer_expired"
            | "attention_timer_expired" => StateConfidence::Medium,
            _ => StateConfidence::Low,
        }
    }
}

impl<'de> Deserialize<'de> for StateEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawStateEvidence {
            #[serde(default = "default_state_cause")]
            cause: String,
            #[serde(default)]
            observed_at: Option<DateTime<Utc>>,
            #[serde(default)]
            confidence: Option<StateConfidence>,
        }

        let raw = RawStateEvidence::deserialize(deserializer)?;
        Ok(Self {
            confidence: raw
                .confidence
                .unwrap_or_else(|| Self::confidence_for_cause(&raw.cause)),
            cause: raw.cause,
            observed_at: raw.observed_at,
        })
    }
}

impl Default for StateEvidence {
    fn default() -> Self {
        Self::unknown()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnTool {
    Claude,
    Codex,
    Grok,
}

impl SpawnTool {
    pub fn command(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
        }
    }

    // Used by swimmers-tui picker; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Grok => "grok",
        }
    }

    // Used by swimmers-tui app; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Grok,
            Self::Grok => Self::Claude,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchPathMapping {
    pub local_prefix: String,
    pub remote_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchTargetSummary {
    pub id: String,
    pub label: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_mappings: Vec<LaunchPathMapping>,
}

impl LaunchTargetSummary {
    pub fn local() -> Self {
        Self {
            id: "local".to_string(),
            label: "Local machine".to_string(),
            kind: "local".to_string(),
            base_url: None,
            auth_token_env: None,
            path_mappings: Vec::new(),
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
    Window,
}

impl GhosttyOpenMode {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "add" | "split" | "new" => Self::Add,
            "window" | "new-window" => Self::Window,
            _ => Self::Swap,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Swap => "swap",
            Self::Add => "add",
            Self::Window => "window",
        }
    }

    /// Human-facing label used in TUI status messages. `label()` returns the
    /// short env-value forms; this returns the phrase users recognize in the
    /// menu ("swap" / "new split").
    pub fn display_label(self) -> &'static str {
        match self {
            Self::Swap => "swap",
            Self::Add => "new split",
            Self::Window => "new window",
        }
    }

    // Used by swimmers-tui app; not called from the daemon binary directly.
    #[allow(dead_code)]
    pub fn toggle(self) -> Self {
        match self {
            Self::Swap => Self::Add,
            Self::Add => Self::Window,
            Self::Window => Self::Swap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AttentionGroupLayout {
    #[default]
    Tiled,
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
}

impl AttentionGroupLayout {
    pub fn from_env_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "even-horizontal" | "horizontal" | "columns" | "side-by-side" => Self::EvenHorizontal,
            "even-vertical" | "vertical" | "rows" | "stacked" => Self::EvenVertical,
            "main-horizontal" | "main-top" | "main-bottom" => Self::MainHorizontal,
            "main-vertical" | "main-left" | "main-right" => Self::MainVertical,
            _ => Self::Tiled,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tiled => "tiled",
            Self::EvenHorizontal => "even-horizontal",
            Self::EvenVertical => "even-vertical",
            Self::MainHorizontal => "main-horizontal",
            Self::MainVertical => "main-vertical",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBatchMembership {
    pub id: String,
    pub label: String,
    pub index: usize,
    pub total: usize,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub tmux_name: String,
    pub state: SessionState,
    pub current_command: Option<String>,
    #[serde(default)]
    pub state_evidence: StateEvidence,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective_changed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_skill: Option<String>,
    pub is_stale: bool,
    pub attached_clients: u32,
    #[serde(default)]
    pub stale_attached_clients: u32,
    pub transport_health: TransportHealth,
    pub last_activity_at: DateTime<Utc>,
    /// Key into `SessionListResponse.repo_themes`; absent when no supported
    /// repo theme cache directory exists for this session cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<SessionBatchMembership>,
}

impl SessionSummary {
    pub fn live(
        session_id: impl Into<String>,
        tmux_name: impl Into<String>,
        state: SessionState,
        current_command: Option<String>,
        state_evidence: StateEvidence,
        cwd: impl Into<String>,
        tool: Option<String>,
        attached_clients: u32,
        stale_attached_clients: u32,
        last_activity_at: DateTime<Utc>,
    ) -> Self {
        let context_limit = context_limit_for_tool(tool.as_deref());
        Self {
            session_id: session_id.into(),
            tmux_name: tmux_name.into(),
            state,
            current_command,
            state_evidence,
            cwd: cwd.into(),
            tool,
            token_count: 0,
            context_limit,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: rest_state_from_idle(state, last_activity_at, Utc::now()),
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients,
            stale_attached_clients,
            transport_health: TransportHealth::Healthy,
            last_activity_at,
            repo_theme_id: None,
            batch: None,
        }
    }

    pub fn placeholder(
        session_id: impl Into<String>,
        tmux_name: impl Into<String>,
        last_activity_at: DateTime<Utc>,
    ) -> Self {
        let mut summary = Self::live(
            session_id,
            tmux_name,
            SessionState::Idle,
            None,
            StateEvidence::unobserved(SUMMARY_CAUSE_SUPERVISOR_PLACEHOLDER),
            String::new(),
            None,
            0,
            0,
            last_activity_at,
        );
        summary.rest_state = fallback_rest_state(SessionState::Idle, ThoughtState::Holding);
        summary
    }

    pub fn into_stale_exited(
        self,
        cause: &'static str,
        observed_at: Option<DateTime<Utc>>,
        transport_health: TransportHealth,
    ) -> Self {
        let rest_state = fallback_rest_state(SessionState::Exited, self.thought_state);
        self.into_stale_exited_with_rest_state(cause, observed_at, transport_health, rest_state)
    }

    pub fn into_stale_exited_with_rest_state(
        mut self,
        cause: &'static str,
        observed_at: Option<DateTime<Utc>>,
        transport_health: TransportHealth,
        rest_state: RestState,
    ) -> Self {
        self.state = SessionState::Exited;
        self.current_command = None;
        self.state_evidence = StateEvidence::with_observed_at(cause, observed_at);
        self.rest_state = rest_state;
        self.is_stale = true;
        self.attached_clients = 0;
        self.stale_attached_clients = 0;
        self.transport_health = transport_health;
        self
    }

    pub fn into_missing_tmux_stale(self, cause: &'static str) -> Self {
        self.into_stale_exited(cause, Some(Utc::now()), TransportHealth::Disconnected)
    }

    pub fn into_cached_collection_fallback(mut self, reason: SummaryFallbackReason) -> Self {
        if let Some((cause, transport_health)) = reason.cached_fallback() {
            self.transport_health = transport_health;
            self.state_evidence = StateEvidence::unobserved(cause);
        }
        self
    }

    pub fn into_remote_poll_degraded(mut self, last_seen_at: Option<DateTime<Utc>>) -> Self {
        self.is_stale = true;
        self.transport_health = TransportHealth::Degraded;
        self.state_evidence =
            StateEvidence::with_observed_at(SUMMARY_CAUSE_REMOTE_POLL_DEGRADED, last_seen_at);
        self
    }

    pub fn revive_from_stale(
        mut self,
        session_id: impl Into<String>,
        tmux_name: impl Into<String>,
        cause: &'static str,
    ) -> Self {
        self.session_id = session_id.into();
        self.tmux_name = tmux_name.into();
        self.state = SessionState::Idle;
        self.current_command = None;
        self.state_evidence = StateEvidence::unobserved(cause);
        self.rest_state = fallback_rest_state(SessionState::Idle, self.thought_state);
        self.is_stale = false;
        self.attached_clients = 0;
        self.stale_attached_clients = 0;
        self.transport_health = TransportHealth::Healthy;
        self
    }
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
pub struct AgentContextActionSummary {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionAgentTurn {
    pub id: String,
    pub source: String,
    pub text: String,
    pub byte_start: u64,
    pub byte_end: u64,
    pub order: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTranscriptRecord {
    pub id: String,
    pub source: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub summary: String,
    pub raw: String,
    pub byte_start: u64,
    pub byte_end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionAgentContextResponse {
    pub session_id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_task: Option<String>,
    #[serde(default)]
    pub turns: Vec<SessionAgentTurn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<AgentContextActionSummary>,
    #[serde(default)]
    pub recent_actions: Vec<AgentContextActionSummary>,
    pub token_count: u64,
    pub context_limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTranscriptResponse {
    pub session_id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_turn: Option<SessionAgentTurn>,
    pub next_cursor: u64,
    #[serde(default)]
    pub records: Vec<SessionTranscriptRecord>,
    #[serde(default)]
    pub turns: Vec<SessionAgentTurn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTimelinePinnedItem {
    pub title: String,
    pub summary: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionTimelinePinned {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<SessionTimelinePinnedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_action: Option<SessionTimelinePinnedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<SessionTimelinePinnedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_tail: Option<SessionTimelinePinnedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<SessionTimelinePinnedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTimelineEvent {
    pub id: String,
    pub kind: String,
    pub source: String,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTimelineResponse {
    pub session_id: String,
    pub available: bool,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default)]
    pub events: Vec<SessionTimelineEvent>,
    #[serde(default)]
    pub pinned: SessionTimelinePinned,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionGitDiffResponse {
    pub session_id: String,
    pub available: bool,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub status_short: String,
    #[serde(default)]
    pub unstaged_diff: String,
    #[serde(default)]
    pub staged_diff: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub files: Vec<SessionGitDiffFileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionGitDiffFileSummary {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub source: String,
    pub change: String,
    pub added_lines: u64,
    pub removed_lines: u64,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hunks: Vec<SessionGitDiffHunkSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionGitDiffHunkSummary {
    pub header: String,
    pub added_lines: u64,
    pub removed_lines: u64,
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

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeAttentionGroupOpenRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sessions: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub current_session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_unnumbered_sessions: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<AttentionGroupLayout>,
    #[serde(default = "default_true")]
    pub focus: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeDesktopOpenResponse {
    pub session_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeAttentionGroupOpenResponse {
    pub session_id: String,
    pub tmux_name: String,
    pub session_count: usize,
    pub session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backlog_session_ids: Vec<String>,
    pub status: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_command: Option<String>,
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
    pub launch_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_request: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdoptSessionRequest {
    pub tmux_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdoptSessionResponse {
    pub session: SessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme: Option<RepoTheme>,
    pub reused_session_id: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionsBatchRequest {
    pub dirs: Vec<String>,
    pub spawn_tool: Option<SpawnTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_request: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionsBatchResult {
    pub index: usize,
    pub cwd: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme: Option<RepoTheme>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionsBatchResponse {
    pub results: Vec<CreateSessionsBatchResult>,
}

impl CreateSessionsBatchResponse {
    pub fn success_count(&self) -> usize {
        self.results.iter().filter(|result| result.ok).count()
    }
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
    /// Effective directory groups this entry belongs to after overlay defaults
    /// and operator-managed membership deltas have been merged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch_targets: Vec<LaunchTargetSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_launch_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirRepoSearchResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    pub entries: Vec<DirEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirGroupMemberships {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, DirGroupMembershipDelta>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirGroupMembershipDelta {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub include_paths: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub exclude_paths: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirGroupMembershipUpdateRequest {
    pub path: String,
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirGroupMembershipUpdateResponse {
    pub path: String,
    pub groups: Vec<String>,
    pub available_groups: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSkillSummary {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSkillIssue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSkillListResponse {
    pub session_id: String,
    pub source: String,
    pub cwd: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default)]
    pub skills: Vec<SessionSkillSummary>,
    #[serde(default)]
    pub issues: Vec<SessionSkillIssue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
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
    #[serde(default)]
    pub submit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInputResponse {
    pub ok: bool,
    pub session_id: String,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroupInputRequest {
    pub session_ids: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroupInputResult {
    pub session_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroupInputResponse {
    pub results: Vec<SessionGroupInputResult>,
    pub delivered: usize,
    pub skipped: usize,
}

impl SessionGroupInputResponse {
    pub fn from_results(results: Vec<SessionGroupInputResult>) -> Self {
        let delivered = results.iter().filter(|result| result.ok).count();
        let skipped = results.len().saturating_sub(delivered);
        Self {
            results,
            delivered,
            skipped,
        }
    }
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

impl ErrorResponse {
    pub fn new(code: impl Into<String>, message: Option<String>) -> Self {
        Self {
            code: code.into(),
            message,
        }
    }

    pub fn with_message(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Some(message.into()))
    }

    pub fn display_message(&self, fallback: impl std::fmt::Display) -> String {
        match (self.code.trim(), self.message.as_deref()) {
            ("", Some(message)) => message.to_string(),
            ("", None) => format!("request failed: {fallback}"),
            (code, Some(message)) if message.trim().is_empty() => code.to_string(),
            (code, Some(message)) => format!("{code}: {message}"),
            (code, None) => format!("{code} ({fallback})"),
        }
    }
}

// --- Control Events (Server -> Client JSON) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEvent {
    pub event: String,
    pub session_id: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "payload", rename_all = "snake_case")]
pub enum KnownControlEventPayload {
    SessionState(SessionStatePayload),
    SessionTitle(SessionTitlePayload),
    SessionSkill(SessionSkillPayload),
    ThoughtUpdate(ThoughtUpdatePayload),
}

#[derive(Debug, Clone)]
pub enum ControlEventPayloadContract {
    Known(KnownControlEventPayload),
    Unknown {
        event: String,
        payload: serde_json::Value,
    },
}

impl ControlEvent {
    pub fn payload_contract(&self) -> ControlEventPayloadContract {
        let known = match self.event.as_str() {
            "session_state" => serde_json::from_value(self.payload.clone())
                .ok()
                .map(KnownControlEventPayload::SessionState),
            "session_title" => serde_json::from_value(self.payload.clone())
                .ok()
                .map(KnownControlEventPayload::SessionTitle),
            "session_skill" => serde_json::from_value(self.payload.clone())
                .ok()
                .map(KnownControlEventPayload::SessionSkill),
            "thought_update" => serde_json::from_value(self.payload.clone())
                .ok()
                .map(KnownControlEventPayload::ThoughtUpdate),
            _ => None,
        };

        known
            .map(ControlEventPayloadContract::Known)
            .unwrap_or_else(|| ControlEventPayloadContract::Unknown {
                event: self.event.clone(),
                payload: self.payload.clone(),
            })
    }
}

impl ControlEventPayloadContract {
    pub fn event_name(&self) -> &str {
        match self {
            Self::Known(KnownControlEventPayload::SessionState(_)) => "session_state",
            Self::Known(KnownControlEventPayload::SessionTitle(_)) => "session_title",
            Self::Known(KnownControlEventPayload::SessionSkill(_)) => "session_skill",
            Self::Known(KnownControlEventPayload::ThoughtUpdate(_)) => "thought_update",
            Self::Unknown { event, .. } => event.as_str(),
        }
    }

    pub fn payload_value(&self) -> serde_json::Value {
        // Every Known arm serializes its wrapped payload identically; only the
        // concrete inner type differs, so bind it once via a macro to avoid four
        // copy-pasted `to_value(..).unwrap_or(Null)` arms.
        macro_rules! known_payload_value {
            ($payload:expr) => {
                serde_json::to_value($payload).unwrap_or(serde_json::Value::Null)
            };
        }
        match self {
            Self::Known(KnownControlEventPayload::SessionState(payload)) => {
                known_payload_value!(payload)
            }
            Self::Known(KnownControlEventPayload::SessionTitle(payload)) => {
                known_payload_value!(payload)
            }
            Self::Known(KnownControlEventPayload::SessionSkill(payload)) => {
                known_payload_value!(payload)
            }
            Self::Known(KnownControlEventPayload::ThoughtUpdate(payload)) => {
                known_payload_value!(payload)
            }
            Self::Unknown { payload, .. } => payload.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatePayload {
    pub state: SessionState,
    pub previous_state: SessionState,
    pub current_command: Option<String>,
    #[serde(default)]
    pub state_evidence: StateEvidence,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
    #[serde(default)]
    pub objective_changed: bool,
    #[serde(default = "default_bubble_precedence")]
    pub bubble_precedence: BubblePrecedence,
    #[serde(default, skip_serializing_if = "is_false")]
    pub persistence_degraded: bool,
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

fn default_state_cause() -> String {
    "unknown".to_string()
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
// FIXME(2026-04-21): Typed WS control envelopes are retained for protocol hardening.
// Current handlers in `src/web/mod.rs` still emit and parse ad-hoc JSON payloads.

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
    // FIXME(2026-04-21): WS transport currently uses text JSON frames; binary opcodes stay reserved.
    #[allow(dead_code)]
    pub const TERMINAL_INPUT: u8 = 0x10;
    #[allow(dead_code)]
    pub const TERMINAL_OUTPUT: u8 = 0x11;
}

// --- Context Limits ---

pub fn context_limit_for_tool(tool: Option<&str>) -> u64 {
    match tool {
        Some("Codex") => 192_000,
        Some("Claude Code" | "Amp" | "Goose" | "Cline" | "Cursor") => 200_000,
        _ => 128_000,
    }
}

pub fn detect_tool_name(comm: &str) -> Option<&'static str> {
    let token = comm.split_whitespace().next().unwrap_or(comm);
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
        "grok" | "grok-cli" | "grok_cli" => Some("Grok"),
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
    fn state_evidence_deserializes_partial_payload_with_mapped_confidence() {
        let evidence: StateEvidence = serde_json::from_str(r#"{"cause":"osc133_prompt"}"#).unwrap();

        assert_eq!(evidence.cause, "osc133_prompt");
        assert_eq!(evidence.confidence, StateConfidence::High);
        assert!(evidence.observed_at.is_none());
    }

    #[test]
    fn unobserved_state_evidence_omits_freshness() {
        let evidence = StateEvidence::unobserved(SUMMARY_CAUSE_PERSISTENCE_STALE);

        assert_eq!(evidence.cause, SUMMARY_CAUSE_PERSISTENCE_STALE);
        assert_eq!(evidence.confidence, StateConfidence::Low);
        assert!(evidence.observed_at.is_none());
    }

    #[test]
    fn session_summary_placeholder_uses_shared_lifecycle_contract() {
        let at = DateTime::parse_from_rfc3339("2026-05-30T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let summary = SessionSummary::placeholder("sess_1", "work", at);

        assert_eq!(summary.session_id, "sess_1");
        assert_eq!(summary.tmux_name, "work");
        assert_eq!(summary.state, SessionState::Idle);
        assert_eq!(
            summary.state_evidence.cause,
            SUMMARY_CAUSE_SUPERVISOR_PLACEHOLDER
        );
        assert!(summary.state_evidence.observed_at.is_none());
        assert_eq!(summary.transport_health, TransportHealth::Healthy);
        assert!(!summary.is_stale);
        assert_eq!(summary.rest_state, RestState::Drowsy);
        assert_eq!(summary.context_limit, 128_000);
    }

    #[test]
    fn session_summary_fallback_reason_parity_is_shared() {
        let at = DateTime::parse_from_rfc3339("2026-05-30T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let summary = SessionSummary::live(
            "sess_1",
            "work",
            SessionState::Busy,
            Some("cargo test".to_string()),
            StateEvidence::new("osc133_command"),
            "/tmp/swimmers",
            Some("Codex".to_string()),
            1,
            2,
            at,
        );

        let dropped = summary
            .clone()
            .into_cached_collection_fallback(SummaryFallbackReason::Dropped);
        assert_eq!(SummaryFallbackReason::Dropped.metric_label(), "dropped");
        assert_eq!(dropped.state_evidence.cause, SUMMARY_CAUSE_CACHE_DEGRADED);
        assert_eq!(dropped.transport_health, TransportHealth::Degraded);
        assert!(!dropped.is_stale);

        let timeout = summary.into_cached_collection_fallback(SummaryFallbackReason::Timeout);
        assert_eq!(SummaryFallbackReason::Timeout.metric_label(), "timeout");
        assert_eq!(timeout.state_evidence.cause, SUMMARY_CAUSE_CACHE_OVERLOADED);
        assert_eq!(timeout.transport_health, TransportHealth::Overloaded);
    }

    #[test]
    fn session_summary_stale_and_remote_helpers_preserve_wire_shape() {
        let at = DateTime::parse_from_rfc3339("2026-05-30T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let live = SessionSummary::placeholder("sess_1", "work", at);

        let missing = live
            .clone()
            .into_missing_tmux_stale(SUMMARY_CAUSE_TMUX_RECONCILE_MISSING);
        assert_eq!(missing.state, SessionState::Exited);
        assert_eq!(
            missing.state_evidence.cause,
            SUMMARY_CAUSE_TMUX_RECONCILE_MISSING
        );
        assert!(missing.is_stale);
        assert_eq!(missing.transport_health, TransportHealth::Disconnected);
        assert_eq!(missing.rest_state, RestState::DeepSleep);

        let remote = live.into_remote_poll_degraded(Some(at));
        assert_eq!(
            remote.state_evidence.cause,
            SUMMARY_CAUSE_REMOTE_POLL_DEGRADED
        );
        assert_eq!(remote.state, SessionState::Idle);
        assert!(remote.is_stale);
        assert_eq!(remote.transport_health, TransportHealth::Degraded);

        let json = serde_json::to_value(&remote).expect("serialize session summary");
        assert_eq!(json["session_id"], "sess_1");
        assert_eq!(json["tmux_name"], "work");
        assert_eq!(json["is_stale"], true);
        assert_eq!(json["transport_health"], "degraded");
        assert!(
            json.get("fallback_reason").is_none(),
            "helper must not introduce a new wire field"
        );
    }

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
        assert_eq!(detect_tool_name("grok-cli"), Some("Grok"));
    }

    #[test]
    fn detect_tool_name_ignores_unknown_tokens() {
        assert_eq!(detect_tool_name("zsh"), None);
        assert_eq!(detect_tool_name("node"), None);
        assert_eq!(detect_tool_name(""), None);
    }

    #[test]
    fn dependency_health_snapshot_serializes_stable_shape() {
        let checked_at = DateTime::parse_from_rfc3339("2026-05-30T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let seen_at = DateTime::parse_from_rfc3339("2026-05-29T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let health = DependencyHealthSnapshot::healthy(checked_at)
            .with_last_seen(seen_at)
            .with_detail("dependency", "tmux_discovery");

        let json = serde_json::to_value(&health).expect("serialize health");

        assert_eq!(json["status"], "healthy");
        assert_eq!(json["last_checked_at"], "2026-05-30T00:00:00Z");
        assert_eq!(json["last_seen_at"], "2026-05-29T23:59:59Z");
        assert_eq!(json["last_error_at"], serde_json::Value::Null);
        assert_eq!(json["last_error"], serde_json::Value::Null);
        assert_eq!(json["freshness_ms"], 1000);
        assert_eq!(json["details"]["dependency"], "tmux_discovery");
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
        assert_eq!(SpawnTool::Grok.command(), "grok");
    }

    #[test]
    fn attention_group_layout_accepts_tmux_names_and_operator_aliases() {
        assert_eq!(
            AttentionGroupLayout::from_env_value("even-horizontal"),
            AttentionGroupLayout::EvenHorizontal
        );
        assert_eq!(
            AttentionGroupLayout::from_env_value("columns"),
            AttentionGroupLayout::EvenHorizontal
        );
        assert_eq!(
            AttentionGroupLayout::from_env_value("stacked"),
            AttentionGroupLayout::EvenVertical
        );
        assert_eq!(
            AttentionGroupLayout::from_env_value("main_left"),
            AttentionGroupLayout::MainVertical
        );
        assert_eq!(
            AttentionGroupLayout::from_env_value("unknown"),
            AttentionGroupLayout::Tiled
        );
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
                state_evidence: Default::default(),
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
                action_cues: Vec::new(),
                objective_changed_at: None,
                last_skill: None,
                is_stale: false,
                attached_clients: 0,
                stale_attached_clients: 0,
                transport_health: TransportHealth::Healthy,
                last_activity_at: chrono::Utc::now(),
                repo_theme_id: Some("/tmp/proj".into()),
                batch: None,
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
            session: resp.session,
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
                state_evidence: Default::default(),
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
                action_cues: Vec::new(),
                objective_changed_at: None,
                last_skill: None,
                is_stale: false,
                attached_clients: 0,
                stale_attached_clients: 0,
                transport_health: TransportHealth::Healthy,
                last_activity_at: chrono::Utc::now(),
                repo_theme_id: Some("/tmp".into()),
                batch: None,
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
