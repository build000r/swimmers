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
/// Maximum terminal input payload accepted by WebSocket and REST surfaces.
pub const MAX_SESSION_INPUT_BYTES: usize = 786_432;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_attach_command_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_hint: Option<String>,
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
            ssh_alias: None,
            remote_attach_command_template: None,
            bootstrap_hint: None,
            path_mappings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEnvironmentScope {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryMetadataSummary {
    pub source: String,
    pub label: String,
    pub value: String,
    pub status: String,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEnvironmentSummary {
    pub scope: SessionEnvironmentScope,
    pub target_id: String,
    pub target_label: String,
    pub target_kind: String,
    pub display_host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_attach_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advisory: Vec<AdvisoryMetadataSummary>,
}

impl SessionEnvironmentSummary {
    pub fn local(cwd: impl Into<String>) -> Self {
        let cwd = cwd.into();
        Self {
            canonical_cwd: (!cwd.is_empty()).then_some(cwd.clone()),
            local_cwd: (!cwd.is_empty()).then_some(cwd),
            ..Self::default()
        }
    }

    pub fn remote(
        target: &LaunchTargetSummary,
        remote_session_id: impl Into<String>,
        remote_cwd: impl Into<String>,
        local_cwd: Option<String>,
        launch_source: impl Into<String>,
    ) -> Self {
        let remote_cwd = remote_cwd.into();
        let canonical_cwd = local_cwd
            .clone()
            .or_else(|| (!remote_cwd.is_empty()).then_some(remote_cwd.clone()));
        Self {
            scope: SessionEnvironmentScope::Remote,
            target_id: target.id.clone(),
            target_label: target.label.clone(),
            target_kind: target.kind.clone(),
            display_host: target.label.clone(),
            remote_session_id: Some(remote_session_id.into()),
            launch_source: Some(launch_source.into()),
            remote_attach_command: None,
            local_cwd,
            remote_cwd: (!remote_cwd.is_empty()).then_some(remote_cwd),
            canonical_cwd,
            advisory: Vec::new(),
        }
    }
}

impl Default for SessionEnvironmentSummary {
    fn default() -> Self {
        Self {
            scope: SessionEnvironmentScope::Local,
            target_id: "local".to_string(),
            target_label: "Local machine".to_string(),
            target_kind: "local".to_string(),
            display_host: "local".to_string(),
            remote_session_id: None,
            launch_source: Some("local_supervisor".to_string()),
            remote_attach_command: None,
            local_cwd: None,
            remote_cwd: None,
            canonical_cwd: None,
            advisory: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentAuthSummary {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env_present: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentCapabilitySummary {
    pub observe_sessions: bool,
    pub launch_session: bool,
    pub send_input: bool,
    pub group_input: bool,
    pub remote_dir_inventory: bool,
    pub native_attach: bool,
    pub ssh_attach_hint: bool,
    pub bootstrap_hint: bool,
    pub advisory_metadata: bool,
    pub health_probe: bool,
}

impl EnvironmentCapabilitySummary {
    pub fn local() -> Self {
        Self {
            observe_sessions: true,
            launch_session: true,
            send_input: true,
            group_input: true,
            remote_dir_inventory: true,
            native_attach: true,
            ssh_attach_hint: false,
            bootstrap_hint: false,
            advisory_metadata: true,
            health_probe: true,
        }
    }

    pub fn remote_swimmers_api(
        ready: bool,
        has_path_mappings: bool,
        has_safe_ssh_alias: bool,
        has_bootstrap_hint: bool,
    ) -> Self {
        Self::remote_swimmers_api_with_state(
            ready,
            ready,
            has_path_mappings,
            has_safe_ssh_alias,
            has_bootstrap_hint,
        )
    }

    pub fn remote_swimmers_api_with_state(
        observe_sessions: bool,
        write_ready: bool,
        has_path_mappings: bool,
        has_safe_ssh_alias: bool,
        has_bootstrap_hint: bool,
    ) -> Self {
        Self {
            observe_sessions,
            launch_session: write_ready,
            send_input: write_ready,
            group_input: write_ready,
            remote_dir_inventory: write_ready && has_path_mappings,
            native_attach: has_safe_ssh_alias,
            ssh_attach_hint: has_safe_ssh_alias,
            bootstrap_hint: has_bootstrap_hint,
            advisory_metadata: true,
            health_probe: true,
        }
    }

    pub fn ssh_handoff(has_safe_alias: bool) -> Self {
        Self {
            observe_sessions: false,
            launch_session: false,
            send_input: false,
            group_input: false,
            remote_dir_inventory: false,
            native_attach: false,
            ssh_attach_hint: has_safe_alias,
            bootstrap_hint: has_safe_alias,
            advisory_metadata: true,
            health_probe: false,
        }
    }

    pub fn advisory_only() -> Self {
        Self {
            observe_sessions: false,
            launch_session: false,
            send_input: false,
            group_input: false,
            remote_dir_inventory: false,
            native_attach: false,
            ssh_attach_hint: false,
            bootstrap_hint: false,
            advisory_metadata: true,
            health_probe: false,
        }
    }
}

impl Default for EnvironmentCapabilitySummary {
    fn default() -> Self {
        Self::advisory_only()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentSummary {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub backend_mode: String,
    #[serde(default = "default_environment_display_host")]
    pub display_host: String,
    #[serde(default)]
    pub capabilities: EnvironmentCapabilitySummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub auth: EnvironmentAuthSummary,
    pub path_mapping_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_hint: Option<String>,
    pub status: DependencyHealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advisory: Vec<AdvisoryMetadataSummary>,
}

fn default_environment_display_host() -> String {
    "local".to_string()
}

impl EnvironmentSummary {
    pub fn local() -> Self {
        Self {
            id: "local".to_string(),
            label: "Local machine".to_string(),
            kind: "local".to_string(),
            backend_mode: "local".to_string(),
            display_host: "local".to_string(),
            capabilities: EnvironmentCapabilitySummary::local(),
            base_url: None,
            auth: EnvironmentAuthSummary {
                mode: "none".to_string(),
                token_env_present: None,
            },
            path_mapping_count: 0,
            ssh_alias: None,
            attach_hint: None,
            bootstrap_hint: None,
            status: DependencyHealthStatus::Healthy,
            last_seen_at: Some(Utc::now()),
            last_error_at: None,
            last_error: None,
            freshness_ms: Some(0),
            advisory: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FleetLensBucketKind {
    Target,
    Repo,
    Advisory,
    State,
    Readiness,
    Transport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetLensBucket {
    pub kind: FleetLensBucketKind,
    pub key: String,
    pub label: String,
    pub count: usize,
    pub degraded_count: usize,
    pub stale_count: usize,
    pub attention_count: usize,
    pub commit_ready_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FleetLensPresetMatcher {
    All,
    FleetBucket {
        kind: FleetLensBucketKind,
        key: String,
    },
    TargetId {
        id: String,
    },
    TargetKind {
        kind: String,
    },
    Repo {
        key: String,
    },
    CurrentRepo,
    Readiness {
        key: String,
    },
    Transport {
        key: String,
    },
    Capability {
        key: String,
    },
    Degraded,
    NeedsAttention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetLensPreset {
    pub id: String,
    pub label: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matchers: Vec<FleetLensPresetMatcher>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetLensSummary {
    pub total_sessions: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buckets: Vec<FleetLensBucket>,
}

impl FleetLensSummary {
    pub fn is_empty(&self) -> bool {
        self.total_sessions == 0 && self.buckets.is_empty()
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
    #[serde(default)]
    pub environment: SessionEnvironmentSummary,
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
        let cwd = cwd.into();
        Self {
            session_id: session_id.into(),
            tmux_name: tmux_name.into(),
            state,
            current_command,
            state_evidence,
            cwd: cwd.clone(),
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
            environment: SessionEnvironmentSummary::local(cwd),
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
        self.environment = SessionEnvironmentSummary::local(self.cwd.clone());
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
    pub launch_receipt: Option<LaunchReceipt>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_theme: Option<RepoTheme>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_receipt: Option<LaunchReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchReceipt {
    pub outcome: String,
    pub target_id: String,
    pub target_label: String,
    pub target_kind: String,
    pub target_capability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_override: bool,
}

impl LaunchReceipt {
    pub fn local(
        cwd: impl Into<String>,
        session_id: impl Into<String>,
        local_override: bool,
    ) -> Self {
        let local_cwd = cwd.into();
        let session_id = session_id.into();
        Self {
            outcome: "created".to_string(),
            target_id: "local".to_string(),
            target_label: "Local machine".to_string(),
            target_kind: "local".to_string(),
            target_capability: if local_override {
                "local_override"
            } else {
                "local"
            }
            .to_string(),
            local_cwd: (!local_cwd.is_empty()).then_some(local_cwd),
            remote_cwd: None,
            session_id: (!session_id.is_empty()).then_some(session_id),
            remote_session_id: None,
            attach_hint: None,
            bootstrap_hint: None,
            message: local_override.then(|| "explicit local override".to_string()),
            local_override,
        }
    }

    pub fn mark_local_override(&mut self) {
        if self.target_id != "local" {
            return;
        }
        self.target_capability = "local_override".to_string();
        self.local_override = true;
        self.message = Some("explicit local override".to_string());
    }
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
    #[serde(default)]
    pub version: u64,
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
    /// True when the input was only partially delivered (e.g. text without the
    /// trailing Enter). `ok`/`delivered` stay true to preserve the some-vs-none
    /// contract; a caller that needs an all-or-nothing submit branches on this
    /// to retry (swimmers-bjsu).
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
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
    /// Per-session partial-delivery flag, mirroring SessionInputResponse.partial
    /// for the group path (swimmers-bjsu).
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<EnvironmentSummary>,
    #[serde(default, skip_serializing_if = "FleetLensSummary::is_empty")]
    pub fleet_lens: FleetLensSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fleet_presets: Vec<FleetLensPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<EnvironmentSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fleet_presets: Vec<FleetLensPreset>,
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
mod rest_state_tests;

// --- WebSocket Push Protocol Types ---

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

// --- Binary Frame Constants ---

pub mod opcodes {
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

const TOOL_NAME_ALIASES: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("claude-code", "Claude Code"),
    ("claude_code", "Claude Code"),
    ("codex", "Codex"),
    ("codex-cli", "Codex"),
    ("codex_cli", "Codex"),
    ("grok", "Grok"),
    ("grok-cli", "Grok"),
    ("grok_cli", "Grok"),
    ("amp", "Amp"),
    ("opencode", "OpenCode"),
    ("open-code", "OpenCode"),
    ("open_code", "OpenCode"),
    ("aider", "Aider"),
    ("goose", "Goose"),
    ("cline", "Cline"),
    ("cursor", "Cursor"),
];

const TOOL_NAME_TRIM_CHARS: &[char] =
    &['"', '\'', '`', ',', ';', ':', '(', ')', '[', ']', '{', '}'];

pub fn detect_tool_name(comm: &str) -> Option<&'static str> {
    let token = normalized_command_token(comm);
    known_tool_name(&token)
}

fn normalized_command_token(comm: &str) -> String {
    let token = command_token(comm);
    let token = trim_command_punctuation(token);
    let token = command_basename(token);
    let token = token.trim_start_matches('-');
    token.to_lowercase()
}

fn command_token(comm: &str) -> &str {
    comm.split_whitespace().next().unwrap_or(comm)
}

fn trim_command_punctuation(token: &str) -> &str {
    token.trim_matches(TOOL_NAME_TRIM_CHARS)
}

fn command_basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn known_tool_name(normalized_token: &str) -> Option<&'static str> {
    TOOL_NAME_ALIASES
        .iter()
        .find_map(|(alias, name)| (*alias == normalized_token).then_some(*name))
}

#[cfg(test)]
mod tests;
