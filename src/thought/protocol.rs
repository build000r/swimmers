use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::thought::loop_runner::SessionInfo;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{BubblePrecedence, RestState, SessionState, ThoughtSource, ThoughtState};

pub const HELLO_MESSAGE_TYPE: &str = "hello";
pub const SYNC_MESSAGE_TYPE: &str = "sync";
pub const SYNC_RESULT_MESSAGE_TYPE: &str = "sync_result";
pub const SYNC_RESPONSE_MESSAGE_TYPE: &str = SYNC_RESULT_MESSAGE_TYPE;
pub const EMIT_PROTOCOL_V1: &str = "clawgs.emit.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSnapshotPayload {
    pub session_id: String,
    pub state: SessionState,
    pub exited: bool,
    pub tool: Option<String>,
    pub cwd: String,
    pub replay_text: String,
    pub thought: Option<String>,
    pub thought_state: ThoughtState,
    pub thought_source: ThoughtSource,
    pub rest_state: RestState,
    pub objective_fingerprint: Option<String>,
    pub thought_updated_at: Option<DateTime<Utc>>,
    pub token_count: u64,
    pub context_limit: u64,
    pub last_activity_at: DateTime<Utc>,
}

impl From<&SessionInfo> for SessionSnapshotPayload {
    fn from(value: &SessionInfo) -> Self {
        Self {
            session_id: value.session_id.clone(),
            state: value.state,
            exited: value.exited,
            tool: value.tool.clone(),
            cwd: value.cwd.clone(),
            replay_text: value.replay_text.clone(),
            thought: value.thought.clone(),
            thought_state: value.thought_state,
            thought_source: value.thought_source,
            rest_state: value.rest_state,
            objective_fingerprint: value.objective_fingerprint.clone(),
            thought_updated_at: value.thought_updated_at,
            token_count: value.token_count,
            context_limit: value.context_limit,
            last_activity_at: value.last_activity_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncRequestConfig {
    pub enabled: bool,
    pub model: String,
    pub cadence_hot_ms: u64,
    pub cadence_warm_ms: u64,
    pub cadence_cold_ms: u64,
    pub agent_prompt: String,
    pub terminal_prompt: String,
}

impl Default for SyncRequestConfig {
    fn default() -> Self {
        Self::from(ThoughtConfig::default())
    }
}

impl From<ThoughtConfig> for SyncRequestConfig {
    fn from(value: ThoughtConfig) -> Self {
        Self::from(&value)
    }
}

impl From<&ThoughtConfig> for SyncRequestConfig {
    fn from(value: &ThoughtConfig) -> Self {
        Self {
            enabled: value.enabled,
            model: value.model.clone(),
            cadence_hot_ms: value.cadence_hot_ms,
            cadence_warm_ms: value.cadence_warm_ms,
            cadence_cold_ms: value.cadence_cold_ms,
            agent_prompt: value.agent_prompt.clone().unwrap_or_default(),
            terminal_prompt: value.terminal_prompt.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncRequest {
    #[serde(rename = "type")]
    pub message_type: String,
    pub id: String,
    pub now: DateTime<Utc>,
    pub config: SyncRequestConfig,
    #[serde(skip)]
    pub request_id: u64,
    pub sessions: Vec<SessionSnapshotPayload>,
}

impl SyncRequest {
    pub fn from_session_snapshots(request_id: u64, sessions: &[SessionInfo]) -> Self {
        Self::from_session_snapshots_with_config(request_id, sessions, SyncRequestConfig::default())
    }

    pub fn from_session_snapshots_with_config(
        request_id: u64,
        sessions: &[SessionInfo],
        config: SyncRequestConfig,
    ) -> Self {
        Self {
            message_type: SYNC_MESSAGE_TYPE.to_string(),
            id: request_id.to_string(),
            now: Utc::now(),
            config,
            request_id,
            sessions: sessions.iter().map(SessionSnapshotPayload::from).collect(),
        }
    }
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
    pub objective_changed: bool,
    pub bubble_precedence: BubblePrecedence,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub objective_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThoughtDeliveryState {
    #[serde(default)]
    pub stream_instance_id: Option<String>,
    #[serde(default)]
    pub emission_seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncResponse {
    pub request_id: String,
    pub stream_instance_id: Option<String>,
    pub updates: Vec<SyncUpdate>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
enum WireRequestId {
    Numeric(u64),
    Stringy(String),
}

fn parse_wire_request_id(value: WireRequestId) -> String {
    match value {
        WireRequestId::Numeric(v) => v.to_string(),
        WireRequestId::Stringy(v) => v,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RestState, SessionState, ThoughtSource, ThoughtState};

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
            objective_fingerprint: Some("obj-1".to_string()),
            thought_updated_at: Some(now),
            token_count: 12,
            context_limit: 100,
            last_activity_at: now,
        }
    }

    #[test]
    fn sync_request_serializes_expected_shape() {
        let request = SyncRequest::from_session_snapshots(7, &[sample_session()]);
        let json = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(json["type"], SYNC_MESSAGE_TYPE);
        assert_eq!(json["id"], "7");
        assert!(json.get("request_id").is_none());
        assert!(chrono::DateTime::parse_from_rfc3339(
            json["now"]
                .as_str()
                .expect("now should be an RFC3339 string")
        )
        .is_ok());
        assert_eq!(json["config"]["enabled"], true);
        assert_eq!(json["config"]["model"], "");
        assert_eq!(json["config"]["cadence_hot_ms"], 15_000);
        assert_eq!(json["config"]["cadence_warm_ms"], 45_000);
        assert_eq!(json["config"]["cadence_cold_ms"], 120_000);
        assert_eq!(json["config"]["agent_prompt"], "");
        assert_eq!(json["config"]["terminal_prompt"], "");
        assert_eq!(json["sessions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["sessions"][0]["session_id"], "sess-1");
        assert_eq!(json["sessions"][0]["state"], "busy");
    }

    #[test]
    fn sync_request_config_maps_optional_prompts_from_runtime_config() {
        let runtime = ThoughtConfig {
            enabled: false,
            model: "openrouter/custom".to_string(),
            cadence_hot_ms: 9_000,
            cadence_warm_ms: 60_000,
            cadence_cold_ms: 120_000,
            agent_prompt: Some("Agent prompt".to_string()),
            terminal_prompt: None,
        };

        let mapped = SyncRequestConfig::from(&runtime);
        assert!(!mapped.enabled);
        assert_eq!(mapped.model, "openrouter/custom");
        assert_eq!(mapped.cadence_hot_ms, 9_000);
        assert_eq!(mapped.cadence_warm_ms, 60_000);
        assert_eq!(mapped.cadence_cold_ms, 120_000);
        assert_eq!(mapped.agent_prompt, "Agent prompt");
        assert_eq!(mapped.terminal_prompt, "");
    }

    #[test]
    fn inbound_message_deserializes_sync_result_with_string_id() {
        let raw = r#"{
            "type": "sync_result",
            "id": "42",
            "updates": []
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("sync_result should deserialize");

        match message {
            DaemonInboundMessage::SyncResponse {
                request_id,
                stream_instance_id,
                updates,
            } => {
                assert_eq!(request_id, "42");
                assert_eq!(stream_instance_id.as_deref(), None);
                assert!(updates.is_empty());
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
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
                assert_eq!(SYNC_RESPONSE_MESSAGE_TYPE, "sync_result");
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
    }

    #[test]
    fn inbound_message_deserializes_non_numeric_string_id() {
        let raw = r#"{
            "type": "sync_result",
            "id": "req-sync-17",
            "updates": []
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("non-numeric string id should deserialize");

        match message {
            DaemonInboundMessage::SyncResponse {
                request_id,
                stream_instance_id,
                updates,
            } => {
                assert_eq!(request_id, "req-sync-17");
                assert_eq!(stream_instance_id.as_deref(), None);
                assert!(updates.is_empty());
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
    }

    #[test]
    fn inbound_message_deserializes_stream_identity_fields() {
        let raw = r#"{
            "type": "sync_result",
            "id": "tmux-1",
            "stream_instance_id": "stream-a",
            "updates": [
                {
                    "session_id": "tmux:work:1.0:%1",
                    "stream_instance_id": "stream-a",
                    "emission_seq": 1,
                    "thought": "Indexing repo",
                    "token_count": 10,
                    "context_limit": 100,
                    "thought_state": "active",
                    "thought_source": "llm",
                    "objective_changed": true,
                    "bubble_precedence": "thought_first",
                    "at": "2026-03-08T14:00:05Z"
                }
            ]
        }"#;

        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("stream identity should deserialize");

        match message {
            DaemonInboundMessage::SyncResponse {
                request_id,
                stream_instance_id,
                updates,
            } => {
                assert_eq!(request_id, "tmux-1");
                assert_eq!(stream_instance_id.as_deref(), Some("stream-a"));
                assert_eq!(updates.len(), 1);
                assert_eq!(updates[0].stream_instance_id.as_deref(), Some("stream-a"));
                assert_eq!(updates[0].emission_seq, Some(1));
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
    }

    #[test]
    fn inbound_error_deserializes_string_id() {
        let raw = r#"{
            "type": "error",
            "id": "req-err-8",
            "code": "bad_request",
            "message": "invalid payload"
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("error with string id should deserialize");

        match message {
            DaemonInboundMessage::Error {
                code,
                message,
                request_id,
            } => {
                assert_eq!(code, "bad_request");
                assert_eq!(message, "invalid payload");
                assert_eq!(request_id.as_deref(), Some("req-err-8"));
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
    }

    #[test]
    fn hello_roundtrip_is_stable() {
        let hello = HelloMessage {
            message_type: HELLO_MESSAGE_TYPE.to_string(),
            protocol: EMIT_PROTOCOL_V1.to_string(),
        };
        let encoded = serde_json::to_string(&hello).expect("hello should serialize");
        let decoded: HelloMessage = serde_json::from_str(&encoded).expect("hello should parse");

        assert_eq!(decoded, hello);
    }

    #[test]
    fn hello_inbound_parsing_tolerates_extra_fields() {
        let raw = r#"{
            "type": "hello",
            "protocol": "clawgs.emit.v1",
            "engine_version": "0.12.4",
            "capabilities": ["sync_result"]
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("hello with extra fields should deserialize");

        match message {
            DaemonInboundMessage::Hello { protocol } => {
                assert_eq!(protocol, EMIT_PROTOCOL_V1);
            }
            other => panic!("unexpected message variant: {other:?}"),
        }
    }

    #[test]
    fn thought_delivery_state_defaults_empty() {
        let states: std::collections::HashMap<String, ThoughtDeliveryState> =
            serde_json::from_str("{}").expect("empty watermark map");
        assert!(states.is_empty());
    }
}
