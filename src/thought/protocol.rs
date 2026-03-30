use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::thought::loop_runner::SessionInfo;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{BubblePrecedence, RestState, SessionState, ThoughtSource, ThoughtState};

pub const HELLO_MESSAGE_TYPE: &str = "hello";
pub const SYNC_MESSAGE_TYPE: &str = "sync";
pub const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";
pub const SYNC_RESPONSE_MESSAGE_TYPE: &str = SYNC_RESULT_MESSAGE_TYPE;
pub const EMIT_PROTOCOL_V1: &str = "clawgs.emit.v1";

#[derive(Debug, Default)]
pub struct SyncRequestSequence {
    next_request_id: AtomicU64,
}

impl SyncRequestSequence {
    pub fn new() -> Self {
        Self {
            next_request_id: AtomicU64::new(0),
        }
    }

    pub fn peek_next(&self) -> u64 {
        self.next_request_id
            .load(Ordering::Relaxed)
            .saturating_add(1)
    }

    pub fn next(&self) -> u64 {
        self.next_request_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtDeliveryState {
    #[serde(default)]
    pub stream_instance_id: Option<String>,
    #[serde(default)]
    pub emission_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncUpdate {
    pub session_id: String,
    #[serde(default)]
    pub stream_instance_id: Option<String>,
    #[serde(default)]
    pub emission_seq: Option<u64>,
    pub thought: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    pub thought_state: ThoughtState,
    pub thought_source: ThoughtSource,
    #[serde(default)]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    pub objective_changed: bool,
    pub bubble_precedence: BubblePrecedence,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub objective_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncResponse {
    pub request_id: String,
    pub stream_instance_id: Option<String>,
    pub updates: Vec<SyncUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub state: SessionState,
    pub exited: bool,
    pub tool: Option<String>,
    pub cwd: String,
    pub replay_text: String,
    pub thought: Option<String>,
    #[serde(default)]
    pub thought_state: ThoughtState,
    #[serde(default)]
    pub thought_source: ThoughtSource,
    pub objective_fingerprint: Option<String>,
    pub thought_updated_at: Option<DateTime<Utc>>,
    pub token_count: u64,
    pub context_limit: u64,
    pub last_activity_at: DateTime<Utc>,
    #[serde(default)]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncRequest {
    pub id: String,
    pub now: DateTime<Utc>,
    pub config: ThoughtConfig,
    pub sessions: Vec<SessionSnapshot>,
}

impl SyncRequest {
    pub fn new(
        id: impl Into<String>,
        now: DateTime<Utc>,
        config: ThoughtConfig,
        sessions: Vec<SessionSnapshot>,
    ) -> Self {
        Self {
            id: id.into(),
            now,
            config,
            sessions,
        }
    }
}

impl Serialize for SyncRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("SyncRequest", 5)?;
        state.serialize_field("type", SYNC_MESSAGE_TYPE)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("now", &self.now)?;
        state.serialize_field("config", &SyncRequestConfigWireRef::from(&self.config))?;
        state.serialize_field("sessions", &self.sessions)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for SyncRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SyncRequestWire::deserialize(deserializer)?;
        Ok(Self {
            id: wire.id,
            now: wire.now,
            config: wire.config.into_runtime_config(),
            sessions: wire.sessions,
        })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct SyncRequestWire {
    #[serde(default = "default_sync_message_type", rename = "type")]
    _message_type: String,
    id: String,
    now: DateTime<Utc>,
    config: SyncRequestConfigWire,
    #[serde(default)]
    sessions: Vec<SessionSnapshot>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct SyncRequestConfigWire {
    enabled: bool,
    model: String,
    cadence_hot_ms: u64,
    cadence_warm_ms: u64,
    cadence_cold_ms: u64,
    #[serde(default, deserialize_with = "deserialize_wire_prompt_string")]
    agent_prompt: String,
    #[serde(default, deserialize_with = "deserialize_wire_prompt_string")]
    terminal_prompt: String,
}

impl SyncRequestConfigWire {
    fn into_runtime_config(self) -> ThoughtConfig {
        let mut config = ThoughtConfig {
            enabled: self.enabled,
            model: self.model,
            cadence_hot_ms: self.cadence_hot_ms,
            cadence_warm_ms: self.cadence_warm_ms,
            cadence_cold_ms: self.cadence_cold_ms,
            agent_prompt: string_to_optional_prompt(self.agent_prompt),
            terminal_prompt: string_to_optional_prompt(self.terminal_prompt),
        };
        config.normalize();
        config
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct SyncRequestConfigWireRef<'a> {
    enabled: bool,
    model: &'a str,
    cadence_hot_ms: u64,
    cadence_warm_ms: u64,
    cadence_cold_ms: u64,
    agent_prompt: &'a str,
    terminal_prompt: &'a str,
}

impl<'a> From<&'a ThoughtConfig> for SyncRequestConfigWireRef<'a> {
    fn from(value: &'a ThoughtConfig) -> Self {
        Self {
            enabled: value.enabled,
            model: value.model.as_str(),
            cadence_hot_ms: value.cadence_hot_ms,
            cadence_warm_ms: value.cadence_warm_ms,
            cadence_cold_ms: value.cadence_cold_ms,
            agent_prompt: value.agent_prompt.as_deref().unwrap_or_default(),
            terminal_prompt: value.terminal_prompt.as_deref().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum WireRequestId {
    Numeric(u64),
    Stringy(String),
}

fn parse_wire_request_id(value: WireRequestId) -> String {
    match value {
        WireRequestId::Numeric(value) => value.to_string(),
        WireRequestId::Stringy(value) => value,
    }
}

fn deserialize_request_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = WireRequestId::deserialize(deserializer)?;
    Ok(parse_wire_request_id(raw))
}

fn deserialize_optional_request_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<WireRequestId>::deserialize(deserializer)?;
    Ok(raw.map(parse_wire_request_id))
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonInboundMessage {
    Hello {
        protocol: String,
    },
    #[serde(rename = "sync_result", alias = "sync_response", alias = "sync")]
    SyncResponse {
        #[serde(
            rename = "id",
            alias = "request_id",
            deserialize_with = "deserialize_request_id"
        )]
        request_id: String,
        #[serde(default)]
        stream_instance_id: Option<String>,
        #[serde(default)]
        updates: Vec<SyncUpdate>,
    },
    Error {
        code: String,
        message: String,
        #[serde(
            default,
            rename = "id",
            alias = "request_id",
            deserialize_with = "deserialize_optional_request_id"
        )]
        request_id: Option<String>,
    },
}

impl DaemonInboundMessage {
    pub fn message_type(&self) -> &'static str {
        match self {
            Self::Hello { .. } => HELLO_MESSAGE_TYPE,
            Self::SyncResponse { .. } => SYNC_RESPONSE_MESSAGE_TYPE,
            Self::Error { .. } => "error",
        }
    }
}

pub fn build_sync_request(
    request_id: u64,
    runtime_config: &ThoughtConfig,
    sessions: &[SessionInfo],
) -> SyncRequest {
    build_sync_request_with_now(request_id, Utc::now(), runtime_config, sessions)
}

pub fn build_sync_request_with_now(
    request_id: u64,
    now: DateTime<Utc>,
    runtime_config: &ThoughtConfig,
    sessions: &[SessionInfo],
) -> SyncRequest {
    SyncRequest::new(
        request_id.to_string(),
        now,
        runtime_config.clone(),
        sessions.iter().map(session_snapshot_from_info).collect(),
    )
}

fn session_snapshot_from_info(session: &SessionInfo) -> SessionSnapshot {
    SessionSnapshot {
        session_id: session.session_id.clone(),
        state: session.state,
        exited: session.exited,
        tool: session.tool.clone(),
        cwd: session.cwd.clone(),
        replay_text: session.replay_text.clone(),
        thought: session.thought.clone(),
        thought_state: session.thought_state,
        thought_source: session.thought_source,
        rest_state: session.rest_state,
        objective_fingerprint: session.objective_fingerprint.clone(),
        thought_updated_at: session.thought_updated_at,
        token_count: session.token_count,
        context_limit: session.context_limit,
        last_activity_at: session.last_activity_at,
        commit_candidate: session.commit_candidate,
    }
}

fn string_to_optional_prompt(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn deserialize_wire_prompt_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn default_sync_message_type() -> String {
    SYNC_MESSAGE_TYPE.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session() -> SessionInfo {
        let now = Utc::now();
        SessionInfo {
            session_id: "sess-1".to_string(),
            state: SessionState::Busy,
            exited: false,
            tool: Some("Codex".to_string()),
            cwd: "/tmp".to_string(),
            replay_text: "cargo test".to_string(),
            thought: Some("Running tests".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            objective_fingerprint: Some("obj-1".to_string()),
            thought_updated_at: Some(now),
            token_count: 12,
            context_limit: 100,
            last_activity_at: now,
        }
    }

    #[test]
    fn sync_request_serializes_expected_shape() {
        let request = build_sync_request_with_now(
            7,
            Utc::now(),
            &ThoughtConfig::default(),
            &[sample_session()],
        );
        let json = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(json["type"], SYNC_MESSAGE_TYPE);
        assert_eq!(json["id"], "7");
        assert_eq!(json["config"]["agent_prompt"], "");
        assert_eq!(json["config"]["terminal_prompt"], "");
        assert_eq!(json["sessions"][0]["session_id"], "sess-1");
        assert_eq!(json["sessions"][0]["state"], "busy");
    }

    #[test]
    fn sync_request_deserializes_null_prompt_fields_to_none() {
        let raw = serde_json::json!({
            "type": "sync",
            "id": "req-1",
            "now": "2026-02-26T21:00:00Z",
            "config": {
                "enabled": true,
                "model": "",
                "cadence_hot_ms": 15000,
                "cadence_warm_ms": 45000,
                "cadence_cold_ms": 120000,
                "agent_prompt": null,
                "terminal_prompt": null
            },
            "sessions": []
        })
        .to_string();

        let request: SyncRequest = serde_json::from_str(&raw).expect("sync request should parse");
        assert_eq!(request.id, "req-1");
        assert!(request.config.agent_prompt.is_none());
        assert!(request.config.terminal_prompt.is_none());
    }

    #[test]
    fn sync_request_sequence_peek_is_read_only() {
        let sequence = SyncRequestSequence::new();
        assert_eq!(sequence.peek_next(), 1);
        assert_eq!(sequence.peek_next(), 1);
        assert_eq!(sequence.next(), 1);
        assert_eq!(sequence.peek_next(), 2);
    }

    #[test]
    fn thought_delivery_state_defaults_empty() {
        let states: std::collections::HashMap<String, ThoughtDeliveryState> =
            serde_json::from_str("{}").expect("empty watermark map");
        assert!(states.is_empty());
    }

    #[test]
    fn inbound_message_deserializes_legacy_sync_response_alias() {
        let raw = r#"{
            "type": "sync_response",
            "request_id": 17,
            "updates": []
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("legacy sync_response alias should deserialize");

        match message {
            DaemonInboundMessage::SyncResponse {
                request_id,
                stream_instance_id,
                updates,
            } => {
                assert_eq!(request_id, "17");
                assert_eq!(stream_instance_id.as_deref(), None);
                assert!(updates.is_empty());
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
    }
}
