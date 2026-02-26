use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::thought::loop_runner::SessionInfo;
use crate::types::{BubblePrecedence, SessionState, ThoughtSource, ThoughtState};

pub const HELLO_MESSAGE_TYPE: &str = "hello";
pub const SYNC_MESSAGE_TYPE: &str = "sync";
pub const SYNC_RESPONSE_MESSAGE_TYPE: &str = "sync_response";
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
            objective_fingerprint: value.objective_fingerprint.clone(),
            thought_updated_at: value.thought_updated_at,
            token_count: value.token_count,
            context_limit: value.context_limit,
            last_activity_at: value.last_activity_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncRequest {
    #[serde(rename = "type")]
    pub message_type: String,
    pub request_id: u64,
    pub sessions: Vec<SessionSnapshotPayload>,
}

impl SyncRequest {
    pub fn from_session_snapshots(request_id: u64, sessions: &[SessionInfo]) -> Self {
        Self {
            message_type: SYNC_MESSAGE_TYPE.to_string(),
            request_id,
            sessions: sessions.iter().map(SessionSnapshotPayload::from).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncUpdate {
    pub session_id: String,
    pub thought: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    pub thought_state: ThoughtState,
    pub thought_source: ThoughtSource,
    pub objective_changed: bool,
    pub bubble_precedence: BubblePrecedence,
    pub at: DateTime<Utc>,
    #[serde(default)]
    pub objective_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SyncResponse {
    pub request_id: u64,
    pub updates: Vec<SyncUpdate>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonInboundMessage {
    Hello {
        protocol: String,
    },
    #[serde(rename = "sync_response", alias = "sync_result", alias = "sync")]
    SyncResponse {
        request_id: u64,
        #[serde(default)]
        updates: Vec<SyncUpdate>,
    },
    Error {
        code: String,
        message: String,
        #[serde(default)]
        request_id: Option<u64>,
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
    use crate::types::{SessionState, ThoughtSource, ThoughtState};

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
        assert_eq!(json["request_id"], 7);
        assert_eq!(json["sessions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(json["sessions"][0]["session_id"], "sess-1");
        assert_eq!(json["sessions"][0]["state"], "busy");
    }

    #[test]
    fn inbound_message_deserializes_sync_alias() {
        let raw = r#"{
            "type": "sync_result",
            "request_id": 42,
            "updates": []
        }"#;
        let message: DaemonInboundMessage =
            serde_json::from_str(raw).expect("sync_result alias should deserialize");

        match message {
            DaemonInboundMessage::SyncResponse {
                request_id,
                updates,
            } => {
                assert_eq!(request_id, 42);
                assert!(updates.is_empty());
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
}
