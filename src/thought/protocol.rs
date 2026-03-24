use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use clawgs::emit::protocol::{
    DaemonInboundMessage, EMIT_PROTOCOL_V1, SYNC_MESSAGE_TYPE,
};

use crate::thought::loop_runner::SessionInfo;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{BubblePrecedence, RestState, SessionState, ThoughtSource, ThoughtState};

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

pub fn build_sync_request(
    request_id: u64,
    runtime_config: &ThoughtConfig,
    sessions: &[SessionInfo],
) -> clawgs::emit::protocol::SyncRequest {
    build_sync_request_with_now(request_id, Utc::now(), runtime_config, sessions)
}

pub fn build_sync_request_with_now(
    request_id: u64,
    now: DateTime<Utc>,
    runtime_config: &ThoughtConfig,
    sessions: &[SessionInfo],
) -> clawgs::emit::protocol::SyncRequest {
    clawgs::emit::protocol::SyncRequest::new(
        request_id.to_string(),
        now,
        runtime_config.clone(),
        sessions.iter().map(shared_session_snapshot).collect(),
    )
}

impl From<clawgs::emit::protocol::SyncUpdate> for SyncUpdate {
    fn from(value: clawgs::emit::protocol::SyncUpdate) -> Self {
        Self {
            session_id: value.session_id,
            stream_instance_id: value.stream_instance_id,
            emission_seq: value.emission_seq,
            thought: value.thought,
            token_count: value.token_count,
            context_limit: value.context_limit,
            thought_state: thought_state_from_shared(value.thought_state),
            thought_source: thought_source_from_shared(value.thought_source),
            rest_state: rest_state_from_shared(value.rest_state),
            objective_changed: value.objective_changed,
            bubble_precedence: bubble_precedence_from_shared(value.bubble_precedence),
            at: value.at,
            objective_fingerprint: value.objective_fingerprint,
        }
    }
}

impl From<clawgs::emit::protocol::SyncResponse> for SyncResponse {
    fn from(value: clawgs::emit::protocol::SyncResponse) -> Self {
        Self {
            request_id: value.request_id,
            stream_instance_id: value.stream_instance_id,
            updates: value.updates.into_iter().map(SyncUpdate::from).collect(),
        }
    }
}

fn shared_session_snapshot(session: &SessionInfo) -> clawgs::emit::protocol::SessionSnapshot {
    clawgs::emit::protocol::SessionSnapshot {
        session_id: session.session_id.clone(),
        state: session_state_to_shared(session.state),
        exited: session.exited,
        tool: session.tool.clone(),
        cwd: session.cwd.clone(),
        replay_text: session.replay_text.clone(),
        thought: session.thought.clone(),
        thought_state: thought_state_to_shared(session.thought_state),
        thought_source: thought_source_to_shared(session.thought_source),
        rest_state: rest_state_to_shared(session.rest_state),
        objective_fingerprint: session.objective_fingerprint.clone(),
        thought_updated_at: session.thought_updated_at,
        token_count: session.token_count,
        context_limit: session.context_limit,
        last_activity_at: session.last_activity_at,
    }
}

fn session_state_to_shared(state: SessionState) -> clawgs::emit::protocol::SessionState {
    match state {
        SessionState::Idle => clawgs::emit::protocol::SessionState::Idle,
        SessionState::Busy => clawgs::emit::protocol::SessionState::Busy,
        SessionState::Error => clawgs::emit::protocol::SessionState::Error,
        SessionState::Attention => clawgs::emit::protocol::SessionState::Attention,
        SessionState::Exited => clawgs::emit::protocol::SessionState::Exited,
    }
}

fn thought_state_to_shared(state: ThoughtState) -> clawgs::emit::protocol::ThoughtState {
    match state {
        ThoughtState::Active => clawgs::emit::protocol::ThoughtState::Active,
        ThoughtState::Holding => clawgs::emit::protocol::ThoughtState::Holding,
        ThoughtState::Sleeping => clawgs::emit::protocol::ThoughtState::Sleeping,
    }
}

fn thought_source_to_shared(source: ThoughtSource) -> clawgs::emit::protocol::ThoughtSource {
    match source {
        ThoughtSource::CarryForward => clawgs::emit::protocol::ThoughtSource::CarryForward,
        ThoughtSource::Llm => clawgs::emit::protocol::ThoughtSource::Llm,
        ThoughtSource::StaticSleeping => clawgs::emit::protocol::ThoughtSource::StaticSleeping,
    }
}

fn rest_state_to_shared(state: RestState) -> clawgs::emit::protocol::RestState {
    match state {
        RestState::Active => clawgs::emit::protocol::RestState::Active,
        RestState::Drowsy => clawgs::emit::protocol::RestState::Drowsy,
        RestState::Sleeping => clawgs::emit::protocol::RestState::Sleeping,
        RestState::DeepSleep => clawgs::emit::protocol::RestState::DeepSleep,
    }
}

fn thought_state_from_shared(state: clawgs::emit::protocol::ThoughtState) -> ThoughtState {
    match state {
        clawgs::emit::protocol::ThoughtState::Active => ThoughtState::Active,
        clawgs::emit::protocol::ThoughtState::Holding => ThoughtState::Holding,
        clawgs::emit::protocol::ThoughtState::Sleeping => ThoughtState::Sleeping,
    }
}

fn thought_source_from_shared(source: clawgs::emit::protocol::ThoughtSource) -> ThoughtSource {
    match source {
        clawgs::emit::protocol::ThoughtSource::CarryForward => ThoughtSource::CarryForward,
        clawgs::emit::protocol::ThoughtSource::Llm => ThoughtSource::Llm,
        clawgs::emit::protocol::ThoughtSource::StaticSleeping => ThoughtSource::StaticSleeping,
    }
}

fn rest_state_from_shared(state: clawgs::emit::protocol::RestState) -> RestState {
    match state {
        clawgs::emit::protocol::RestState::Active => RestState::Active,
        clawgs::emit::protocol::RestState::Drowsy => RestState::Drowsy,
        clawgs::emit::protocol::RestState::Sleeping => RestState::Sleeping,
        clawgs::emit::protocol::RestState::DeepSleep => RestState::DeepSleep,
    }
}

fn bubble_precedence_from_shared(
    value: clawgs::emit::protocol::BubblePrecedence,
) -> BubblePrecedence {
    match value {
        clawgs::emit::protocol::BubblePrecedence::ThoughtFirst => BubblePrecedence::ThoughtFirst,
    }
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
            objective_fingerprint: Some("obj-1".to_string()),
            thought_updated_at: Some(now),
            token_count: 12,
            context_limit: 100,
            last_activity_at: now,
        }
    }

    #[test]
    fn sync_request_serializes_expected_shape() {
        let request = build_sync_request_with_now(7, Utc::now(), &ThoughtConfig::default(), &[sample_session()]);
        let json = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(json["type"], SYNC_MESSAGE_TYPE);
        assert_eq!(json["id"], "7");
        assert_eq!(json["config"]["agent_prompt"], "");
        assert_eq!(json["config"]["terminal_prompt"], "");
        assert_eq!(json["sessions"][0]["session_id"], "sess-1");
        assert_eq!(json["sessions"][0]["state"], "busy");
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
}
