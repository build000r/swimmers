use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::thought::emitter_client::EmitterClient;
use crate::thought::loop_runner::SessionInfo;
use crate::thought::loop_runner::SessionProvider;
use crate::thought::protocol::{SyncResponse, SyncUpdate, ThoughtDeliveryState};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{ControlEvent, ThoughtSource, ThoughtUpdatePayload};

// TODO: re-evaluate when BridgeRunner::new is used outside tests
#[allow(dead_code)]
const DEFAULT_BRIDGE_TICK: Duration = Duration::from_secs(2);

/// Consumes the tmux-scoped clawgs thought stream and applies accepted updates
/// through the existing SessionProvider + control bus.
pub struct BridgeRunner {
    tick: Duration,
    event_tx: broadcast::Sender<ControlEvent>,
    runtime_config: Arc<RwLock<ThoughtConfig>>,
}

impl BridgeRunner {
    // TODO: re-evaluate when direct BridgeRunner construction is needed outside tests
    #[allow(dead_code)]
    pub fn new(
        event_tx: broadcast::Sender<ControlEvent>,
        runtime_config: Arc<RwLock<ThoughtConfig>>,
    ) -> Self {
        Self {
            tick: DEFAULT_BRIDGE_TICK,
            event_tx,
            runtime_config,
        }
    }

    pub fn with_tick(
        event_tx: broadcast::Sender<ControlEvent>,
        tick: Duration,
        runtime_config: Arc<RwLock<ThoughtConfig>>,
    ) -> Self {
        Self {
            tick,
            event_tx,
            runtime_config,
        }
    }

    pub fn spawn<P: SessionProvider + 'static>(
        self,
        provider: Arc<P>,
        mut emitter_client: EmitterClient,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                tick_ms = self.tick.as_millis() as u64,
                "thought bridge runner started"
            );

            let mut delivery_states = provider.thought_delivery_states();
            loop {
                let runtime_config = self.runtime_config.read().await.clone();
                let snapshots = provider.session_snapshots();
                match emitter_client
                    .next_sync_response(&runtime_config, &snapshots)
                    .await
                {
                    Ok(response) => {
                        apply_sync_response(
                            provider.as_ref(),
                            &self.event_tx,
                            &mut delivery_states,
                            &snapshots,
                            response,
                        );
                    }
                    Err(err) => {
                        warn!(
                            error = %err,
                            "clawgs emit daemon sync failed; continuing bridge loop"
                        );
                    }
                }
            }
        })
    }
}

fn apply_sync_response<P: SessionProvider>(
    provider: &P,
    event_tx: &broadcast::Sender<ControlEvent>,
    delivery_states: &mut std::collections::HashMap<String, ThoughtDeliveryState>,
    session_snapshots: &[SessionInfo],
    response: SyncResponse,
) {
    debug!(
        request_id = %response.request_id,
        update_count = response.updates.len(),
        "applying thought updates from daemon"
    );

    let batch_stream_instance_id = response.stream_instance_id.clone();
    let mut prior_thoughts = session_snapshots
        .iter()
        .map(|snapshot| (snapshot.session_id.clone(), snapshot.thought.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    for update in response.updates {
        apply_update(
            provider,
            event_tx,
            delivery_states,
            &mut prior_thoughts,
            batch_stream_instance_id.as_deref(),
            update,
        );
    }
}

fn is_sleeping_placeholder(thought: &str) -> bool {
    matches!(
        thought.trim().to_ascii_lowercase().as_str(),
        "sleeping" | "sleeping."
    )
}

fn normalize_sleeping_update(
    mut update: SyncUpdate,
    prior_thoughts: &std::collections::HashMap<String, Option<String>>,
) -> SyncUpdate {
    if update.thought_source != ThoughtSource::StaticSleeping {
        return update;
    }

    update.thought = prior_thoughts
        .get(&update.session_id)
        .and_then(|thought| thought.as_deref())
        .map(str::trim)
        .filter(|thought| !thought.is_empty() && !is_sleeping_placeholder(thought))
        .map(ToString::to_string);
    update.thought_source = ThoughtSource::CarryForward;
    update
}

fn resolved_delivery_state(
    batch_stream_instance_id: Option<&str>,
    update: &SyncUpdate,
) -> Option<ThoughtDeliveryState> {
    let stream_instance_id = update
        .stream_instance_id
        .clone()
        .or_else(|| batch_stream_instance_id.map(ToString::to_string));
    let emission_seq = update.emission_seq?;
    if emission_seq == 0 {
        return None;
    }

    Some(ThoughtDeliveryState {
        stream_instance_id,
        emission_seq,
    })
}

fn should_apply_delivery_state(
    current: Option<&ThoughtDeliveryState>,
    incoming: Option<&ThoughtDeliveryState>,
) -> bool {
    let Some(incoming) = incoming else {
        return true;
    };
    let Some(incoming_stream) = incoming.stream_instance_id.as_deref() else {
        return true;
    };

    let Some(current) = current else {
        return true;
    };
    let Some(current_stream) = current.stream_instance_id.as_deref() else {
        return true;
    };

    if current_stream == incoming_stream {
        incoming.emission_seq > current.emission_seq
    } else {
        incoming.emission_seq == 1
    }
}

fn apply_update<P: SessionProvider>(
    provider: &P,
    event_tx: &broadcast::Sender<ControlEvent>,
    delivery_states: &mut std::collections::HashMap<String, ThoughtDeliveryState>,
    prior_thoughts: &mut std::collections::HashMap<String, Option<String>>,
    batch_stream_instance_id: Option<&str>,
    update: SyncUpdate,
) {
    let update = normalize_sleeping_update(update, prior_thoughts);
    let incoming_delivery = resolved_delivery_state(batch_stream_instance_id, &update);
    let current_delivery = delivery_states.get(&update.session_id);
    if !should_apply_delivery_state(current_delivery, incoming_delivery.as_ref()) {
        return;
    }

    let persisted_delivery = incoming_delivery
        .clone()
        .or_else(|| current_delivery.cloned())
        .unwrap_or_default();
    delivery_states.insert(update.session_id.clone(), persisted_delivery.clone());
    prior_thoughts.insert(update.session_id.clone(), update.thought.clone());

    provider.persist_thought(
        &update.session_id,
        update.thought.as_deref(),
        update.token_count,
        update.context_limit,
        update.thought_state,
        update.thought_source,
        update.rest_state,
        update.commit_candidate,
        update.at,
        persisted_delivery,
        update.objective_changed.then_some(update.at),
        update.objective_fingerprint.clone(),
    );

    let payload = ThoughtUpdatePayload {
        thought: update.thought.clone(),
        token_count: update.token_count,
        context_limit: update.context_limit,
        thought_state: update.thought_state,
        thought_source: update.thought_source,
        rest_state: update.rest_state,
        commit_candidate: update.commit_candidate,
        objective_changed: update.objective_changed,
        bubble_precedence: update.bubble_precedence,
        at: update.at,
    };

    let event = ControlEvent {
        event: "thought_update".to_string(),
        session_id: update.session_id,
        payload: serde_json::to_value(&payload).unwrap_or_default(),
    };

    let _ = event_tx.send(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::thought::loop_runner::{SessionInfo, SessionProvider};
    use crate::thought::protocol::{SyncUpdate, ThoughtDeliveryState};
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{
        BubblePrecedence, RestState, SessionState, ThoughtSource, ThoughtState,
        ThoughtUpdatePayload,
    };
    use tokio::sync::RwLock;

    #[derive(Debug, Clone)]
    struct PersistCall {
        session_id: String,
        thought: Option<String>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        rest_state: RestState,
        commit_candidate: bool,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_changed_at: Option<DateTime<Utc>>,
        objective_fingerprint: Option<String>,
    }

    #[derive(Default)]
    struct RecordingProvider {
        snapshots: Vec<SessionInfo>,
        persisted: Mutex<Vec<PersistCall>>,
        delivery_states: Mutex<HashMap<String, ThoughtDeliveryState>>,
    }

    impl SessionProvider for RecordingProvider {
        fn session_snapshots(&self) -> Vec<SessionInfo> {
            self.snapshots.clone()
        }

        fn persist_thought(
            &self,
            session_id: &str,
            thought: Option<&str>,
            token_count: u64,
            context_limit: u64,
            thought_state: ThoughtState,
            thought_source: ThoughtSource,
            rest_state: RestState,
            commit_candidate: bool,
            updated_at: DateTime<Utc>,
            delivery: ThoughtDeliveryState,
            objective_changed_at: Option<DateTime<Utc>>,
            objective_fingerprint: Option<String>,
        ) {
            self.persisted
                .lock()
                .expect("persisted mutex should lock")
                .push(PersistCall {
                    session_id: session_id.to_string(),
                    thought: thought.map(ToString::to_string),
                    token_count,
                    context_limit,
                    thought_state,
                    thought_source,
                    rest_state,
                    commit_candidate,
                    updated_at,
                    delivery: delivery.clone(),
                    objective_changed_at,
                    objective_fingerprint,
                });
            self.delivery_states
                .lock()
                .expect("delivery_states mutex should lock")
                .insert(session_id.to_string(), delivery);
        }

        fn thought_delivery_states(&self) -> HashMap<String, ThoughtDeliveryState> {
            self.delivery_states
                .lock()
                .expect("delivery_states mutex should lock")
                .clone()
        }
    }

    #[tokio::test]
    async fn apply_sync_response_persists_and_broadcasts() {
        let provider = RecordingProvider::default();
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
        let now = Utc::now();

        let response = SyncResponse {
            request_id: "tmux-1".to_string(),
            stream_instance_id: Some("stream-a".to_string()),
            updates: vec![SyncUpdate {
                session_id: "sess-bridge".to_string(),
                stream_instance_id: None,
                emission_seq: Some(1),
                thought: Some("Applying bridge update".to_string()),
                token_count: 88,
                context_limit: 120,
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                objective_changed: true,
                bubble_precedence: BubblePrecedence::ThoughtFirst,
                at: now,
                objective_fingerprint: Some("obj-bridge".to_string()),
            }],
            llm_calls: 0,
            last_backend_error: None,
        };

        let mut delivery_states = provider.thought_delivery_states();
        apply_sync_response(&provider, &event_tx, &mut delivery_states, &[], response);

        let persisted = provider
            .persisted
            .lock()
            .expect("persisted mutex should lock");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].session_id, "sess-bridge");
        assert_eq!(
            persisted[0].thought.as_deref(),
            Some("Applying bridge update")
        );
        assert_eq!(persisted[0].token_count, 88);
        assert_eq!(persisted[0].context_limit, 120);
        assert_eq!(persisted[0].thought_state, ThoughtState::Active);
        assert_eq!(persisted[0].thought_source, ThoughtSource::Llm);
        assert_eq!(persisted[0].rest_state, RestState::Active);
        assert!(persisted[0].commit_candidate);
        assert_eq!(persisted[0].updated_at, now);
        assert_eq!(persisted[0].objective_changed_at, Some(now));
        assert_eq!(
            persisted[0].delivery.stream_instance_id.as_deref(),
            Some("stream-a")
        );
        assert_eq!(persisted[0].delivery.emission_seq, 1);
        assert_eq!(
            persisted[0].objective_fingerprint.as_deref(),
            Some("obj-bridge")
        );
        drop(persisted);

        let event = event_rx.recv().await.expect("event should be broadcast");
        assert_eq!(event.event, "thought_update");
        assert_eq!(event.session_id, "sess-bridge");

        let payload: ThoughtUpdatePayload =
            serde_json::from_value(event.payload).expect("payload should deserialize");
        assert_eq!(payload.thought.as_deref(), Some("Applying bridge update"));
        assert_eq!(payload.token_count, 88);
        assert_eq!(payload.context_limit, 120);
        assert_eq!(payload.thought_state, ThoughtState::Active);
        assert_eq!(payload.thought_source, ThoughtSource::Llm);
        assert_eq!(payload.rest_state, RestState::Active);
        assert!(payload.commit_candidate);
        assert!(payload.objective_changed);
        assert_eq!(payload.bubble_precedence, BubblePrecedence::ThoughtFirst);
        assert_eq!(payload.at, now);
    }

    #[test]
    fn bridge_runner_defaults_to_two_second_tick() {
        let (event_tx, _) = broadcast::channel::<ControlEvent>(8);
        let runtime_config = Arc::new(RwLock::new(ThoughtConfig::default()));
        let runner = BridgeRunner::new(event_tx, runtime_config);
        assert_eq!(runner.tick, Duration::from_secs(2));
    }

    #[test]
    fn apply_sync_response_handles_empty_update_list() {
        let provider = RecordingProvider::default();
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
        let mut delivery_states = provider.thought_delivery_states();

        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &[],
            SyncResponse {
                request_id: "tmux-1".to_string(),
                stream_instance_id: Some("stream-a".to_string()),
                updates: Vec::new(),
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        let persisted = provider
            .persisted
            .lock()
            .expect("persisted mutex should lock");
        assert!(persisted.is_empty());

        match event_rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected no event for empty update list, got: {other:?}"),
        }
    }

    #[test]
    fn recording_provider_session_snapshots_roundtrip() {
        let provider = RecordingProvider {
            snapshots: vec![SessionInfo {
                session_id: "sess-a".to_string(),
                state: SessionState::Idle,
                exited: false,
                tool: None,
                cwd: "/tmp".to_string(),
                replay_text: String::new(),
                thought: None,
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::CarryForward,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                objective_fingerprint: None,
                thought_updated_at: None,
                token_count: 0,
                context_limit: 0,
                last_activity_at: Utc::now(),
            }],
            persisted: Mutex::new(Vec::new()),
            delivery_states: Mutex::new(HashMap::new()),
        };

        let snapshots = provider.session_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].session_id, "sess-a");
    }

    #[test]
    fn duplicate_event_is_ignored() {
        let provider = RecordingProvider::default();
        provider
            .delivery_states
            .lock()
            .expect("delivery states")
            .insert(
                "tmux:work:1.0:%1".to_string(),
                ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 2,
                },
            );
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);

        let mut delivery_states = provider.thought_delivery_states();
        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &[],
            SyncResponse {
                request_id: "tmux-2".to_string(),
                stream_instance_id: Some("stream-a".to_string()),
                updates: vec![SyncUpdate {
                    session_id: "tmux:work:1.0:%1".to_string(),
                    stream_instance_id: None,
                    emission_seq: Some(2),
                    thought: Some("Patching sidebar".to_string()),
                    token_count: 10,
                    context_limit: 100,
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    commit_candidate: false,
                    objective_changed: false,
                    bubble_precedence: BubblePrecedence::ThoughtFirst,
                    at: DateTime::parse_from_rfc3339("2026-03-08T14:00:07Z")
                        .expect("timestamp")
                        .with_timezone(&Utc),
                    objective_fingerprint: None,
                }],
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        assert!(provider.persisted.lock().expect("persisted").is_empty());
        assert_eq!(
            delivery_states
                .get("tmux:work:1.0:%1")
                .expect("delivery state")
                .emission_seq,
            2
        );
        match event_rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected no event for duplicate update, got: {other:?}"),
        }
    }

    #[test]
    fn stale_event_in_same_stream_is_ignored() {
        let provider = RecordingProvider::default();
        provider
            .delivery_states
            .lock()
            .expect("delivery states")
            .insert(
                "tmux:work:1.0:%1".to_string(),
                ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 4,
                },
            );
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);

        let mut delivery_states = provider.thought_delivery_states();
        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &[],
            SyncResponse {
                request_id: "tmux-3".to_string(),
                stream_instance_id: Some("stream-a".to_string()),
                updates: vec![SyncUpdate {
                    session_id: "tmux:work:1.0:%1".to_string(),
                    stream_instance_id: None,
                    emission_seq: Some(3),
                    thought: Some("Running tests".to_string()),
                    token_count: 10,
                    context_limit: 100,
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    commit_candidate: false,
                    objective_changed: false,
                    bubble_precedence: BubblePrecedence::ThoughtFirst,
                    at: Utc::now(),
                    objective_fingerprint: None,
                }],
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        assert!(provider.persisted.lock().expect("persisted").is_empty());
        assert_eq!(
            delivery_states
                .get("tmux:work:1.0:%1")
                .expect("delivery state")
                .emission_seq,
            4
        );
        match event_rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected no event for stale update, got: {other:?}"),
        }
    }

    #[test]
    fn stream_restart_accepts_seq_one_and_resets_watermark() {
        let provider = RecordingProvider::default();
        provider
            .delivery_states
            .lock()
            .expect("delivery states")
            .insert(
                "tmux:work:1.0:%1".to_string(),
                ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 4,
                },
            );
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
        let now = DateTime::parse_from_rfc3339("2026-03-08T14:05:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);

        let mut delivery_states = provider.thought_delivery_states();
        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &[],
            SyncResponse {
                request_id: "tmux-4".to_string(),
                stream_instance_id: Some("stream-b".to_string()),
                updates: vec![SyncUpdate {
                    session_id: "tmux:work:1.0:%1".to_string(),
                    stream_instance_id: None,
                    emission_seq: Some(1),
                    thought: Some("Reconnected and resuming".to_string()),
                    token_count: 10,
                    context_limit: 100,
                    thought_state: ThoughtState::Active,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Active,
                    commit_candidate: false,
                    objective_changed: true,
                    bubble_precedence: BubblePrecedence::ThoughtFirst,
                    at: now,
                    objective_fingerprint: None,
                }],
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        let persisted = provider.persisted.lock().expect("persisted");
        assert_eq!(persisted.len(), 1);
        assert_eq!(
            persisted[0].delivery.stream_instance_id.as_deref(),
            Some("stream-b")
        );
        assert_eq!(persisted[0].delivery.emission_seq, 1);
        drop(persisted);

        let event = event_rx.try_recv().expect("event");
        assert_eq!(event.session_id, "tmux:work:1.0:%1");
    }

    #[tokio::test]
    async fn static_sleeping_update_keeps_last_real_thought() {
        let now = DateTime::parse_from_rfc3339("2026-03-08T14:05:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let provider = RecordingProvider {
            snapshots: vec![SessionInfo {
                session_id: "sess-sleep".to_string(),
                state: SessionState::Idle,
                exited: false,
                tool: Some("Codex".to_string()),
                cwd: "/tmp".to_string(),
                replay_text: String::new(),
                thought: Some("Reviewing the patch".to_string()),
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                objective_fingerprint: None,
                thought_updated_at: Some(now),
                token_count: 12,
                context_limit: 100,
                last_activity_at: now,
            }],
            persisted: Mutex::new(Vec::new()),
            delivery_states: Mutex::new(HashMap::new()),
        };
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
        let mut delivery_states = provider.thought_delivery_states();

        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &provider.session_snapshots(),
            SyncResponse {
                request_id: "tmux-sleep".to_string(),
                stream_instance_id: Some("stream-a".to_string()),
                updates: vec![SyncUpdate {
                    session_id: "sess-sleep".to_string(),
                    stream_instance_id: None,
                    emission_seq: Some(1),
                    thought: Some("Sleeping.".to_string()),
                    token_count: 12,
                    context_limit: 100,
                    thought_state: ThoughtState::Sleeping,
                    thought_source: ThoughtSource::StaticSleeping,
                    rest_state: RestState::Sleeping,
                    commit_candidate: true,
                    objective_changed: false,
                    bubble_precedence: BubblePrecedence::ThoughtFirst,
                    at: now,
                    objective_fingerprint: None,
                }],
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        let persisted = provider
            .persisted
            .lock()
            .expect("persisted mutex should lock");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].thought.as_deref(), Some("Reviewing the patch"));
        assert_eq!(persisted[0].thought_state, ThoughtState::Sleeping);
        assert_eq!(persisted[0].thought_source, ThoughtSource::CarryForward);
        assert_eq!(persisted[0].rest_state, RestState::Sleeping);
        assert!(persisted[0].commit_candidate);
        drop(persisted);

        let event = event_rx.recv().await.expect("event should be broadcast");
        let payload: ThoughtUpdatePayload =
            serde_json::from_value(event.payload).expect("payload should deserialize");
        assert_eq!(payload.thought.as_deref(), Some("Reviewing the patch"));
        assert_eq!(payload.thought_state, ThoughtState::Sleeping);
        assert_eq!(payload.thought_source, ThoughtSource::CarryForward);
        assert_eq!(payload.rest_state, RestState::Sleeping);
        assert!(payload.commit_candidate);
    }

    #[test]
    fn old_stream_after_restart_is_ignored() {
        let provider = RecordingProvider::default();
        provider
            .delivery_states
            .lock()
            .expect("delivery states")
            .insert(
                "tmux:work:1.0:%1".to_string(),
                ThoughtDeliveryState {
                    stream_instance_id: Some("stream-b".to_string()),
                    emission_seq: 1,
                },
            );
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);

        let mut delivery_states = provider.thought_delivery_states();
        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &[],
            SyncResponse {
                request_id: "tmux-5".to_string(),
                stream_instance_id: Some("stream-a".to_string()),
                updates: vec![SyncUpdate {
                    session_id: "tmux:work:1.0:%1".to_string(),
                    stream_instance_id: None,
                    emission_seq: Some(99),
                    thought: Some("late old stream".to_string()),
                    token_count: 10,
                    context_limit: 100,
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    commit_candidate: false,
                    objective_changed: false,
                    bubble_precedence: BubblePrecedence::ThoughtFirst,
                    at: Utc::now(),
                    objective_fingerprint: None,
                }],
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        assert!(provider.persisted.lock().expect("persisted").is_empty());
        assert_eq!(
            delivery_states
                .get("tmux:work:1.0:%1")
                .expect("delivery state")
                .stream_instance_id
                .as_deref(),
            Some("stream-b")
        );
        match event_rx.try_recv() {
            Err(broadcast::error::TryRecvError::Empty) => {}
            other => panic!("expected no event for old stream update, got: {other:?}"),
        }
    }

    #[test]
    fn same_cwd_sessions_keep_independent_watermarks() {
        let provider = RecordingProvider::default();
        provider
            .delivery_states
            .lock()
            .expect("delivery states")
            .extend([
                (
                    "tmux:work:1.0:%1".to_string(),
                    ThoughtDeliveryState {
                        stream_instance_id: Some("stream-a".to_string()),
                        emission_seq: 3,
                    },
                ),
                (
                    "tmux:work:1.1:%2".to_string(),
                    ThoughtDeliveryState {
                        stream_instance_id: Some("stream-a".to_string()),
                        emission_seq: 7,
                    },
                ),
            ]);
        let (event_tx, _) = broadcast::channel::<ControlEvent>(8);

        let mut delivery_states = provider.thought_delivery_states();
        apply_sync_response(
            &provider,
            &event_tx,
            &mut delivery_states,
            &[],
            SyncResponse {
                request_id: "tmux-6".to_string(),
                stream_instance_id: Some("stream-a".to_string()),
                updates: vec![SyncUpdate {
                    session_id: "tmux:work:1.0:%1".to_string(),
                    stream_instance_id: None,
                    emission_seq: Some(4),
                    thought: Some("pane one advanced".to_string()),
                    token_count: 10,
                    context_limit: 100,
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    commit_candidate: false,
                    objective_changed: false,
                    bubble_precedence: BubblePrecedence::ThoughtFirst,
                    at: Utc::now(),
                    objective_fingerprint: None,
                }],
                llm_calls: 0,
                last_backend_error: None,
            },
        );

        assert_eq!(
            delivery_states
                .get("tmux:work:1.0:%1")
                .expect("pane one")
                .emission_seq,
            4
        );
        assert_eq!(
            delivery_states
                .get("tmux:work:1.1:%2")
                .expect("pane two")
                .emission_seq,
            7
        );
    }
}
