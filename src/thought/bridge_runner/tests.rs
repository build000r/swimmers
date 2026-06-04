use super::*;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tempfile::tempdir;

use crate::thought::loop_runner::{SessionInfo, SessionProvider};
use crate::thought::protocol::{SyncUpdate, ThoughtDeliveryState};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{
    ActionCue, ActionCueConfidence, ActionCueKind, ActionCueSource, ActionCueStatus,
    BubblePrecedence, RestState, SessionState, ThoughtSource, ThoughtState, ThoughtUpdatePayload,
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
    action_cues: Vec<ActionCue>,
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
    reject_persists: bool,
}

impl SessionProvider for RecordingProvider {
    async fn session_snapshots(&self) -> Vec<SessionInfo> {
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
        action_cues: Vec<ActionCue>,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_changed_at: Option<DateTime<Utc>>,
        objective_fingerprint: Option<String>,
    ) -> bool {
        if self.reject_persists {
            return false;
        }
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
                action_cues,
                updated_at,
                delivery: delivery.clone(),
                objective_changed_at,
                objective_fingerprint,
            });
        self.delivery_states
            .lock()
            .expect("delivery_states mutex should lock")
            .insert(session_id.to_string(), delivery);
        true
    }

    fn thought_delivery_states(&self) -> HashMap<String, ThoughtDeliveryState> {
        self.delivery_states
            .lock()
            .expect("delivery_states mutex should lock")
            .clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum MetricRecord {
    Lifecycle {
        session_id: String,
        state: &'static str,
    },
    ModelCall {
        session_id: String,
        path: &'static str,
        tier: &'static str,
        outcome: &'static str,
        count: u64,
    },
    Suppression {
        session_id: String,
        reason: &'static str,
        tier: &'static str,
    },
    Latency {
        session_id: String,
        path: &'static str,
        tier: &'static str,
        duration: Duration,
    },
}

#[derive(Default)]
struct RecordingMetrics {
    records: Mutex<Vec<MetricRecord>>,
}

impl RecordingMetrics {
    fn records(&self) -> Vec<MetricRecord> {
        self.records
            .lock()
            .expect("metrics mutex should lock")
            .clone()
    }
}

impl ThoughtMetricRecorder for RecordingMetrics {
    fn set_lifecycle_state(&self, session_id: &str, state: &'static str) {
        self.records
            .lock()
            .expect("metrics mutex should lock")
            .push(MetricRecord::Lifecycle {
                session_id: session_id.to_string(),
                state,
            });
    }

    fn increment_model_call(
        &self,
        session_id: &str,
        path: &'static str,
        tier: &'static str,
        outcome: &'static str,
        count: u64,
    ) {
        self.records
            .lock()
            .expect("metrics mutex should lock")
            .push(MetricRecord::ModelCall {
                session_id: session_id.to_string(),
                path,
                tier,
                outcome,
                count,
            });
    }

    fn increment_suppression(&self, session_id: &str, reason: &'static str, tier: &'static str) {
        self.records
            .lock()
            .expect("metrics mutex should lock")
            .push(MetricRecord::Suppression {
                session_id: session_id.to_string(),
                reason,
                tier,
            });
    }

    fn record_generation_latency(
        &self,
        session_id: &str,
        path: &'static str,
        tier: &'static str,
        duration: Duration,
    ) {
        self.records
            .lock()
            .expect("metrics mutex should lock")
            .push(MetricRecord::Latency {
                session_id: session_id.to_string(),
                path,
                tier,
                duration,
            });
    }
}

fn commit_ready_cue() -> ActionCue {
    ActionCue {
        kind: ActionCueKind::CommitReady,
        status: ActionCueStatus::Active,
        source: ActionCueSource::Transcript,
        confidence: ActionCueConfidence::Deterministic,
        evidence: ActionCue::expected_evidence(ActionCueKind::CommitReady)
            .iter()
            .map(|item| item.to_string())
            .collect(),
    }
}

fn metric_session(
    session_id: &str,
    state: SessionState,
    thought_state: ThoughtState,
) -> SessionInfo {
    SessionInfo {
        session_id: session_id.to_string(),
        state,
        exited: state == SessionState::Exited,
        tool: Some("Codex".to_string()),
        cwd: "/tmp/project".to_string(),
        replay_text: "cargo test".to_string(),
        thought: Some("working".to_string()),
        thought_state,
        thought_source: ThoughtSource::CarryForward,
        rest_state: RestState::Drowsy,
        commit_candidate: false,
        action_cues: Vec::new(),
        objective_fingerprint: Some("obj-metric".to_string()),
        thought_updated_at: None,
        token_count: 42,
        context_limit: 100,
        last_activity_at: Utc::now(),
    }
}

#[test]
fn retain_live_delivery_states_drops_absent_sessions() {
    let mut states: HashMap<String, ThoughtDeliveryState> = HashMap::new();
    states.insert("sess_live".to_string(), ThoughtDeliveryState::default());
    states.insert("sess_gone".to_string(), ThoughtDeliveryState::default());

    let snapshots = vec![metric_session(
        "sess_live",
        SessionState::Busy,
        ThoughtState::Active,
    )];
    retain_live_delivery_states(&mut states, &snapshots);

    assert!(states.contains_key("sess_live"));
    assert!(
        !states.contains_key("sess_gone"),
        "watermark for a session no longer in snapshots must be pruned"
    );

    // Empty snapshots clear the map entirely rather than panicking or leaking.
    retain_live_delivery_states(&mut states, &[]);
    assert!(states.is_empty());
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
            action_cues: vec![commit_ready_cue()],
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
    assert_eq!(persisted[0].action_cues, vec![commit_ready_cue()]);
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
    assert_eq!(payload.action_cues, vec![commit_ready_cue()]);
    assert!(payload.objective_changed);
    assert_eq!(payload.bubble_precedence, BubblePrecedence::ThoughtFirst);
    assert!(!payload.persistence_degraded);
    assert_eq!(payload.at, now);
}

#[test]
fn apply_sync_response_broadcasts_degraded_update_without_advancing_watermark() {
    let provider = RecordingProvider {
        reject_persists: true,
        ..RecordingProvider::default()
    };
    let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
    let mut delivery_states = provider.thought_delivery_states();

    apply_sync_response(
        &provider,
        &event_tx,
        &mut delivery_states,
        &[],
        SyncResponse {
            request_id: "tmux-reject".to_string(),
            stream_instance_id: Some("stream-reject".to_string()),
            updates: vec![SyncUpdate {
                session_id: "sess-reject".to_string(),
                stream_instance_id: None,
                emission_seq: Some(1),
                thought: Some("Do not publish before persist".to_string()),
                token_count: 12,
                context_limit: 100,
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: false,
                action_cues: Vec::new(),
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
    // The update was never persisted, so the in-memory delivery watermark
    // must NOT advance. Leaving it absent ensures a stream restart / daemon
    // resync re-delivers this update instead of silently dropping it.
    assert!(
        !delivery_states.contains_key("sess-reject"),
        "degraded persistence must not advance the delivery watermark"
    );
    assert!(provider
        .delivery_states
        .lock()
        .expect("delivery states")
        .is_empty());
    let event = event_rx
        .try_recv()
        .expect("degraded event should broadcast");
    assert_eq!(event.event, "thought_update");
    assert_eq!(event.session_id, "sess-reject");
    let payload: ThoughtUpdatePayload =
        serde_json::from_value(event.payload).expect("payload should deserialize");
    assert_eq!(
        payload.thought.as_deref(),
        Some("Do not publish before persist")
    );
    assert!(payload.persistence_degraded);
}

#[test]
fn sync_response_records_lifecycle_model_and_latency_metrics() {
    let provider = RecordingProvider {
        snapshots: vec![metric_session(
            "sess-bridge",
            SessionState::Busy,
            ThoughtState::Holding,
        )],
        persisted: Mutex::new(Vec::new()),
        delivery_states: Mutex::new(HashMap::new()),
        reject_persists: false,
    };
    let (event_tx, _) = broadcast::channel::<ControlEvent>(8);
    let metrics = RecordingMetrics::default();
    let now = Utc::now();
    let snapshots = provider.snapshots.clone();
    let mut delivery_states = provider.thought_delivery_states();

    record_sync_attempt_metrics(&metrics, &ThoughtConfig::default(), &snapshots);
    apply_sync_response_with_metrics(
        &provider,
        &event_tx,
        &mut delivery_states,
        &snapshots,
        SyncResponse {
            request_id: "tmux-metrics".to_string(),
            stream_instance_id: Some("stream-metrics".to_string()),
            updates: vec![SyncUpdate {
                session_id: "sess-bridge".to_string(),
                stream_instance_id: None,
                emission_seq: Some(1),
                thought: Some("Thinking".to_string()),
                token_count: 64,
                context_limit: 128,
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed: false,
                bubble_precedence: BubblePrecedence::ThoughtFirst,
                at: now,
                objective_fingerprint: None,
            }],
            llm_calls: 2,
            last_backend_error: None,
        },
        &metrics,
        Some(Duration::from_millis(42)),
    );

    let records = metrics.records();
    assert!(records.contains(&MetricRecord::Lifecycle {
        session_id: "sess-bridge".to_string(),
        state: "holding",
    }));
    assert!(records.contains(&MetricRecord::Lifecycle {
        session_id: "sess-bridge".to_string(),
        state: "active",
    }));
    assert!(records.contains(&MetricRecord::ModelCall {
        session_id: "__bridge__".to_string(),
        path: "daemon",
        tier: "batch",
        outcome: "success",
        count: 2,
    }));
    assert!(records.contains(&MetricRecord::Latency {
        session_id: "__bridge__".to_string(),
        path: "daemon",
        tier: "batch",
        duration: Duration::from_millis(42),
    }));
}

#[test]
fn sync_metrics_record_suppression_reasons() {
    let provider = RecordingProvider {
        snapshots: vec![metric_session(
            "sess-suppress",
            SessionState::Idle,
            ThoughtState::Holding,
        )],
        persisted: Mutex::new(Vec::new()),
        delivery_states: Mutex::new(HashMap::new()),
        reject_persists: false,
    };
    let (event_tx, _) = broadcast::channel::<ControlEvent>(8);
    let metrics = RecordingMetrics::default();
    let mut config = ThoughtConfig::default();
    config.enabled = false;
    let snapshots = provider.snapshots.clone();
    let now = Utc::now();
    let mut delivery_states = provider.thought_delivery_states();

    record_sync_attempt_metrics(&metrics, &config, &snapshots);
    apply_sync_response_with_metrics(
        &provider,
        &event_tx,
        &mut delivery_states,
        &snapshots,
        SyncResponse {
            request_id: "tmux-suppression".to_string(),
            stream_instance_id: Some("stream-suppression".to_string()),
            updates: vec![SyncUpdate {
                session_id: "sess-suppress".to_string(),
                stream_instance_id: None,
                emission_seq: Some(1),
                thought: Some("Sleeping.".to_string()),
                token_count: 64,
                context_limit: 128,
                thought_state: ThoughtState::Sleeping,
                thought_source: ThoughtSource::StaticSleeping,
                rest_state: RestState::Sleeping,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed: false,
                bubble_precedence: BubblePrecedence::ThoughtFirst,
                at: now,
                objective_fingerprint: None,
            }],
            llm_calls: 0,
            last_backend_error: None,
        },
        &metrics,
        None,
    );

    let records = metrics.records();
    assert!(records.contains(&MetricRecord::Suppression {
        session_id: "sess-suppress".to_string(),
        reason: "disabled",
        tier: "warm",
    }));
    assert!(records.contains(&MetricRecord::Suppression {
        session_id: "sess-suppress".to_string(),
        reason: "static_sleeping",
        tier: "warm",
    }));
}

#[test]
fn stale_delivery_records_suppression_metric() {
    let provider = RecordingProvider {
        snapshots: vec![metric_session(
            "sess-stale",
            SessionState::Attention,
            ThoughtState::Active,
        )],
        persisted: Mutex::new(Vec::new()),
        delivery_states: Mutex::new(HashMap::new()),
        reject_persists: false,
    };
    provider
        .delivery_states
        .lock()
        .expect("delivery states")
        .insert(
            "sess-stale".to_string(),
            ThoughtDeliveryState {
                stream_instance_id: Some("stream-a".to_string()),
                emission_seq: 3,
            },
        );
    let (event_tx, _) = broadcast::channel::<ControlEvent>(8);
    let metrics = RecordingMetrics::default();
    let snapshots = provider.snapshots.clone();
    let mut delivery_states = provider.thought_delivery_states();

    apply_sync_response_with_metrics(
        &provider,
        &event_tx,
        &mut delivery_states,
        &snapshots,
        SyncResponse {
            request_id: "tmux-stale".to_string(),
            stream_instance_id: Some("stream-a".to_string()),
            updates: vec![SyncUpdate {
                session_id: "sess-stale".to_string(),
                stream_instance_id: None,
                emission_seq: Some(2),
                thought: Some("Late update".to_string()),
                token_count: 64,
                context_limit: 128,
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed: false,
                bubble_precedence: BubblePrecedence::ThoughtFirst,
                at: Utc::now(),
                objective_fingerprint: None,
            }],
            llm_calls: 0,
            last_backend_error: None,
        },
        &metrics,
        None,
    );

    assert!(metrics.records().contains(&MetricRecord::Suppression {
        session_id: "sess-stale".to_string(),
        reason: "stale_delivery",
        tier: "hot",
    }));
}

#[test]
fn backend_error_response_records_model_outcome() {
    let metrics = RecordingMetrics::default();
    record_sync_response_metrics(
        &metrics,
        &[],
        &SyncResponse {
            request_id: "tmux-error".to_string(),
            stream_instance_id: None,
            updates: Vec::new(),
            llm_calls: 1,
            last_backend_error: Some("rate limited".to_string()),
        },
        Some(Duration::from_millis(17)),
    );

    let records = metrics.records();
    assert!(records.contains(&MetricRecord::ModelCall {
        session_id: "__bridge__".to_string(),
        path: "daemon",
        tier: "batch",
        outcome: "backend_error",
        count: 1,
    }));
    assert!(records.contains(&MetricRecord::Latency {
        session_id: "__bridge__".to_string(),
        path: "daemon",
        tier: "batch",
        duration: Duration::from_millis(17),
    }));
}

#[test]
fn no_update_response_without_snapshots_records_bridge_suppression() {
    let metrics = RecordingMetrics::default();
    record_sync_response_metrics(
        &metrics,
        &[],
        &SyncResponse {
            request_id: "tmux-empty".to_string(),
            stream_instance_id: None,
            updates: Vec::new(),
            llm_calls: 0,
            last_backend_error: None,
        },
        Some(Duration::from_millis(17)),
    );

    assert_eq!(
        metrics.records(),
        vec![MetricRecord::Suppression {
            session_id: "__bridge__".to_string(),
            reason: "no_updates",
            tier: "batch",
        }]
    );
}

#[test]
fn no_update_response_with_snapshots_records_session_suppressions() {
    let metrics = RecordingMetrics::default();
    let snapshots = vec![
        metric_session("sess-busy", SessionState::Busy, ThoughtState::Active),
        metric_session("sess-exited", SessionState::Exited, ThoughtState::Sleeping),
    ];

    record_sync_response_metrics(
        &metrics,
        &snapshots,
        &SyncResponse {
            request_id: "tmux-no-updates".to_string(),
            stream_instance_id: None,
            updates: Vec::new(),
            llm_calls: 0,
            last_backend_error: None,
        },
        Some(Duration::from_millis(17)),
    );

    assert_eq!(
        metrics.records(),
        vec![
            MetricRecord::Suppression {
                session_id: "sess-busy".to_string(),
                reason: "no_updates",
                tier: "hot",
            },
            MetricRecord::Suppression {
                session_id: "sess-exited".to_string(),
                reason: "no_updates",
                tier: "cold",
            },
        ]
    );
}

#[test]
fn bridge_runner_defaults_to_two_second_tick() {
    let (event_tx, _) = broadcast::channel::<ControlEvent>(8);
    let runtime_config = Arc::new(RwLock::new(ThoughtConfig::default()));
    let runner = BridgeRunner::new(event_tx, runtime_config);
    assert_eq!(runner.health().timing().tick, Duration::from_secs(2));
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

#[tokio::test]
async fn recording_provider_session_snapshots_roundtrip() {
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
            action_cues: Vec::new(),
            objective_fingerprint: None,
            thought_updated_at: None,
            token_count: 0,
            context_limit: 0,
            last_activity_at: Utc::now(),
        }],
        persisted: Mutex::new(Vec::new()),
        delivery_states: Mutex::new(HashMap::new()),
        reject_persists: false,
    };

    let snapshots = provider.session_snapshots().await;
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
                action_cues: Vec::new(),
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
fn unsequenced_update_after_watermark_is_ignored() {
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
            request_id: "tmux-unsequenced".to_string(),
            stream_instance_id: Some("stream-a".to_string()),
            updates: vec![SyncUpdate {
                session_id: "tmux:work:1.0:%1".to_string(),
                stream_instance_id: None,
                emission_seq: None,
                thought: Some("Late legacy update".to_string()),
                token_count: 10,
                context_limit: 100,
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                action_cues: Vec::new(),
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
        2
    );
    match event_rx.try_recv() {
        Err(broadcast::error::TryRecvError::Empty) => {}
        other => panic!("expected no event for unsequenced update, got: {other:?}"),
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
                action_cues: Vec::new(),
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
                action_cues: Vec::new(),
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
            action_cues: Vec::new(),
            objective_fingerprint: None,
            thought_updated_at: Some(now),
            token_count: 12,
            context_limit: 100,
            last_activity_at: now,
        }],
        persisted: Mutex::new(Vec::new()),
        delivery_states: Mutex::new(HashMap::new()),
        reject_persists: false,
    };
    let (event_tx, mut event_rx) = broadcast::channel::<ControlEvent>(8);
    let mut delivery_states = provider.thought_delivery_states();

    let snapshots = provider.session_snapshots().await;
    apply_sync_response(
        &provider,
        &event_tx,
        &mut delivery_states,
        &snapshots,
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
                action_cues: Vec::new(),
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
                action_cues: Vec::new(),
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
                action_cues: Vec::new(),
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

#[tokio::test]
async fn bridge_runner_waits_for_tick_between_sync_requests() {
    let temp = tempdir().expect("tempdir");
    let request_log = temp.path().join("requests.log");
    let fake_bin = write_fake_bridge_daemon_script(temp.path(), &request_log);
    let (event_tx, _) = broadcast::channel::<ControlEvent>(8);
    let runtime_config = Arc::new(RwLock::new(ThoughtConfig::default()));
    let runner = BridgeRunner::with_tick(event_tx, Duration::from_millis(60), runtime_config);

    let handle = runner.spawn(
        Arc::new(RecordingProvider::default()),
        EmitterClient::with_bin(fake_bin.to_string_lossy().into_owned()),
    );

    wait_for_log_lines(&request_log, 2).await;
    handle.abort();
    let _ = handle.await;

    let timestamps: Vec<u128> = fs::read_to_string(&request_log)
        .expect("request log")
        .lines()
        .map(|line| line.parse::<u128>().expect("nanoseconds timestamp"))
        .collect();
    assert!(
        timestamps.len() >= 2,
        "expected at least two bridge sync requests, got {:?}",
        timestamps
    );
    let diff_ns = timestamps[1].saturating_sub(timestamps[0]);
    assert!(
        diff_ns >= 40_000_000,
        "expected bridge tick delay, got only {}ns between sync requests",
        diff_ns
    );
}

async fn wait_for_log_lines(path: &Path, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let count = fs::read_to_string(path)
            .ok()
            .map(|content| content.lines().count())
            .unwrap_or(0);
        if count >= expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} log lines in {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn write_fake_bridge_daemon_script(temp_root: &Path, log_path: &Path) -> PathBuf {
    let script_path = temp_root.join("fake-clawgs-bridge.py");
    let script = format!(
        r#"#!/usr/bin/env python3
import json
import pathlib
import sys
import time

log_path = pathlib.Path({log_path:?})
print(json.dumps({{"type": "hello", "protocol": "clawgs.emit.v1"}}), flush=True)
request_id = 0
for line in sys.stdin:
    if not line.strip():
        continue
    request_id += 1
    with log_path.open("a", encoding="utf-8") as handle:
        handle.write(f"{{time.time_ns()}}\n")
    print(
        json.dumps(
            {{
                "type": "sync_result",
                "id": str(request_id),
                "stream_instance_id": "stream-a",
                "updates": [],
                "metrics": {{"llm_calls": 0, "last_backend_error": None}},
            }}
        ),
        flush=True,
    )
"#,
        log_path = log_path.to_string_lossy()
    );
    fs::write(&script_path, script).expect("write fake bridge daemon");
    let mut perms = fs::metadata(&script_path)
        .expect("script metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).expect("set script permissions");
    script_path
}
