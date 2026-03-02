use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::thought::runtime_config::ThoughtConfig;

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
pub enum BubblePrecedence {
    ThoughtFirst,
}

impl Default for BubblePrecedence {
    fn default() -> Self {
        Self::ThoughtFirst
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtCadenceProfile {
    pub hot: u64,
    pub warm: u64,
    pub cold: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtPolicy {
    pub lifecycle_mode: String,
    pub cadence_ms: ThoughtCadenceProfile,
    pub sleeping_after_ms: u64,
    pub bubble_precedence: BubblePrecedence,
}

impl ThoughtPolicy {
    pub fn phase_gated_v1() -> Self {
        Self {
            lifecycle_mode: "phase_gated_v1".to_string(),
            cadence_ms: ThoughtCadenceProfile {
                hot: 15_000,
                warm: 45_000,
                cold: 120_000,
            },
            sleeping_after_ms: 60_000,
            bubble_precedence: BubblePrecedence::ThoughtFirst,
        }
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
}

/// Per-repository Thronglet sprite override pack.
/// All four variants must be present; inline SVG markup is stored as strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpritePack {
    pub active: String,
    pub drowsy: String,
    pub sleeping: String,
    pub deep_sleep: String,
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
    #[serde(default)]
    pub last_skill: Option<String>,
    pub is_stale: bool,
    pub attached_clients: u32,
    pub transport_health: TransportHealth,
    pub last_activity_at: DateTime<Utc>,
    /// Key into `BootstrapResponse.sprite_packs`; absent when no per-repo
    /// sprite override was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_pack_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResponse {
    pub server_time: DateTime<Utc>,
    pub auth_mode: String,
    pub realtime_url: String,
    pub workspace_history_mode: String,
    pub poll_fallback_ms: u64,
    pub thought_tick_ms: u64,
    pub thoughts_enabled_default: bool,
    pub terminal_cache_ttl_ms: u64,
    pub session_delete_mode: String,
    pub legacy_parity_locked: bool,
    pub thought_policy: ThoughtPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_config: Option<ThoughtConfig>,
    pub sessions: Vec<SessionSummary>,
    /// Per-repository sprite packs; keyed by project root path (the
    /// `sprite_pack_id` on each `SessionSummary`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sprite_packs: HashMap<String, SpritePack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub spawn_tool: Option<SpawnTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub has_children: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_running: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirListResponse {
    pub path: String,
    pub entries: Vec<DirEntry>,
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
    pub sprite_pack: Option<SpritePack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummary>,
    pub version: u64,
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

fn default_bubble_precedence() -> BubblePrecedence {
    BubblePrecedence::ThoughtFirst
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreatedPayload {
    pub reason: String, // "startup_discovery" | "runtime_discovery" | "api_create"
    pub session: SessionSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite_pack: Option<SpritePack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDeletedPayload {
    pub reason: String,
    pub delete_mode: String,
    pub tmux_session_alive: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayTruncatedPayload {
    pub code: String,
    pub requested_resume_from_seq: u64,
    pub replay_window_start_seq: u64,
    pub latest_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOverloadedPayload {
    pub code: String,
    pub queue_depth: usize,
    pub queue_bytes: usize,
    pub retry_after_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSubscriptionPayload {
    pub state: String, // "subscribed" | "unsubscribed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_from_seq: Option<u64>,
    pub latest_seq: u64,
    pub replay_window_start_seq: u64,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

// --- Client -> Server Control ---

#[derive(Debug, Clone, Deserialize)]
pub struct ClientControlMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub request_id: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeSessionPayload {
    pub session_id: String,
    pub resume_from_seq: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnsubscribeSessionPayload {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResizePayload {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DismissAttentionPayload {
    pub session_id: String,
}

// --- Binary Frame Constants ---

pub mod opcodes {
    pub const TERMINAL_INPUT: u8 = 0x10;
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
    fn create_session_response_serializes_sprite_pack() {
        let pack = SpritePack {
            active: "<svg id='a'/>".into(),
            drowsy: "<svg id='d'/>".into(),
            sleeping: "<svg id='s'/>".into(),
            deep_sleep: "<svg id='ds'/>".into(),
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
                last_skill: None,
                is_stale: false,
                attached_clients: 0,
                transport_health: TransportHealth::Healthy,
                last_activity_at: chrono::Utc::now(),
                sprite_pack_id: Some("/tmp/proj".into()),
            },
            sprite_pack: Some(pack),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(
            json.contains("sprite_pack"),
            "sprite_pack must be present in JSON"
        );
        assert!(
            json.contains("<svg id='a'/>"),
            "pack SVG content must serialize"
        );

        // When sprite_pack is None, the field should be omitted entirely.
        let resp_none = CreateSessionResponse {
            session: resp.session.clone(),
            sprite_pack: None,
        };
        let json_none = serde_json::to_string(&resp_none).unwrap();
        assert!(
            !json_none.contains("\"sprite_pack\""),
            "null sprite_pack must be omitted"
        );
    }

    #[test]
    fn session_created_payload_serializes_sprite_pack() {
        let pack = SpritePack {
            active: "<svg/>".into(),
            drowsy: "<svg/>".into(),
            sleeping: "<svg/>".into(),
            deep_sleep: "<svg/>".into(),
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
                last_skill: None,
                is_stale: false,
                attached_clients: 0,
                transport_health: TransportHealth::Healthy,
                last_activity_at: chrono::Utc::now(),
                sprite_pack_id: Some("/tmp".into()),
            },
            sprite_pack: Some(pack),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"sprite_pack\""));

        // Deserialize roundtrip
        let parsed: SessionCreatedPayload = serde_json::from_str(&json).unwrap();
        assert!(parsed.sprite_pack.is_some());
        assert_eq!(parsed.sprite_pack.unwrap().active, "<svg/>");
    }
}
