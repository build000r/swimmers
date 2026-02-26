use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::thought::emitter_client::EmitterClient;
use crate::thought::loop_runner::SessionProvider;
use crate::thought::protocol::{SyncResponse, SyncUpdate};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{ControlEvent, ThoughtUpdatePayload};

const DEFAULT_BRIDGE_TICK: Duration = Duration::from_secs(2);

/// Periodically syncs session snapshots to the clawgs emit daemon and applies
/// returned thought updates through the existing SessionProvider + control bus.
pub struct BridgeRunner {
    tick: Duration,
    event_tx: broadcast::Sender<ControlEvent>,
    runtime_config: Arc<RwLock<ThoughtConfig>>,
}

impl BridgeRunner {
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

            let mut interval = tokio::time::interval(self.tick);
            loop {
                interval.tick().await;

                let snapshots = provider.session_snapshots();
                let runtime_config = self.runtime_config.read().await.clone();
                match emitter_client
                    .sync_sessions(&snapshots, &runtime_config)
                    .await
                {
                    Ok(response) => {
                        apply_sync_response(provider.as_ref(), &self.event_tx, response);
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
    response: SyncResponse,
) {
    debug!(
        request_id = response.request_id,
        update_count = response.updates.len(),
        "applying thought updates from daemon"
    );

    for update in response.updates {
        apply_update(provider, event_tx, update);
    }
}

fn apply_update<P: SessionProvider>(
    provider: &P,
    event_tx: &broadcast::Sender<ControlEvent>,
    update: SyncUpdate,
) {
    provider.persist_thought(
        &update.session_id,
        update.thought.as_deref(),
        update.token_count,
        update.context_limit,
        update.thought_state,
        update.thought_source,
        update.objective_fingerprint.clone(),
    );

    let payload = ThoughtUpdatePayload {
        thought: update.thought.clone(),
        token_count: update.token_count,
        context_limit: update.context_limit,
        thought_state: update.thought_state,
        thought_source: update.thought_source,
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
    use chrono::Utc;
    use std::sync::Mutex;

    use crate::thought::loop_runner::{SessionInfo, SessionProvider};
    use crate::thought::protocol::SyncUpdate;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{
        BubblePrecedence, SessionState, ThoughtSource, ThoughtState, ThoughtUpdatePayload,
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
        objective_fingerprint: Option<String>,
    }

    #[derive(Default)]
    struct RecordingProvider {
        snapshots: Vec<SessionInfo>,
        persisted: Mutex<Vec<PersistCall>>,
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
                    objective_fingerprint,
                });
        }
    }

    #[tokio::test]
    async fn apply_sync_response_persists_and_broadcasts() {
        let provider = RecordingProvider::default();
        let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
        let now = Utc::now();

        let response = SyncResponse {
            request_id: 11,
            updates: vec![SyncUpdate {
                session_id: "sess-bridge".to_string(),
                thought: Some("Applying bridge update".to_string()),
                token_count: 88,
                context_limit: 120,
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                objective_changed: true,
                bubble_precedence: BubblePrecedence::ThoughtFirst,
                at: now,
                objective_fingerprint: Some("obj-bridge".to_string()),
            }],
        };

        apply_sync_response(&provider, &event_tx, response);

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

        apply_sync_response(
            &provider,
            &event_tx,
            SyncResponse {
                request_id: 1,
                updates: Vec::new(),
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
                objective_fingerprint: None,
                thought_updated_at: None,
                token_count: 0,
                context_limit: 0,
                last_activity_at: Utc::now(),
            }],
            persisted: Mutex::new(Vec::new()),
        };

        let snapshots = provider.session_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].session_id, "sess-a");
    }
}
