use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::thought::emitter_client::{EmitterClient, EmitterClientError};
use crate::thought::health::{BridgeHealthState, BridgeTiming};
use crate::thought::loop_runner::SessionInfo;
use crate::thought::loop_runner::SessionProvider;
use crate::thought::protocol::{SyncResponse, SyncUpdate, ThoughtDeliveryState};
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{ControlEvent, ThoughtSource, ThoughtUpdatePayload};

// FIXME(2026-04-21): `BridgeRunner::new` is retained for tests/simple callers;
// production startup uses `BridgeRunner::with_tick(...)`.
#[allow(dead_code)]
const DEFAULT_BRIDGE_TICK: Duration = Duration::from_secs(2);

type DeliveryStateMap = std::collections::HashMap<String, ThoughtDeliveryState>;

/// Consumes the tmux-scoped clawgs thought stream and applies accepted updates
/// through the existing SessionProvider + control bus.
pub struct BridgeRunner {
    event_tx: broadcast::Sender<ControlEvent>,
    runtime_config: Arc<RwLock<ThoughtConfig>>,
    health: Arc<BridgeHealthState>,
}

trait ThoughtMetricRecorder {
    fn set_lifecycle_state(&self, session_id: &str, state: &'static str);
    fn increment_model_call(
        &self,
        session_id: &str,
        path: &'static str,
        tier: &'static str,
        outcome: &'static str,
        count: u64,
    );
    fn increment_suppression(&self, session_id: &str, reason: &'static str, tier: &'static str);
    fn record_generation_latency(
        &self,
        session_id: &str,
        path: &'static str,
        tier: &'static str,
        duration: Duration,
    );
}

struct RuntimeThoughtMetricRecorder;

impl ThoughtMetricRecorder for RuntimeThoughtMetricRecorder {
    fn set_lifecycle_state(&self, session_id: &str, state: &'static str) {
        crate::metrics::set_thought_lifecycle_state(session_id, state);
    }

    fn increment_model_call(
        &self,
        session_id: &str,
        path: &'static str,
        tier: &'static str,
        outcome: &'static str,
        count: u64,
    ) {
        crate::metrics::increment_thought_model_call_by(session_id, path, tier, outcome, count);
    }

    fn increment_suppression(&self, session_id: &str, reason: &'static str, tier: &'static str) {
        crate::metrics::increment_thought_suppression(session_id, reason, tier);
    }

    fn record_generation_latency(
        &self,
        session_id: &str,
        path: &'static str,
        tier: &'static str,
        duration: Duration,
    ) {
        crate::metrics::record_thought_generation_latency(session_id, path, tier, duration);
    }
}

impl BridgeRunner {
    // FIXME(2026-04-21): Production wiring uses `with_tick`; this convenience ctor is currently exercised in tests.
    #[allow(dead_code)]
    pub fn new(
        event_tx: broadcast::Sender<ControlEvent>,
        runtime_config: Arc<RwLock<ThoughtConfig>>,
    ) -> Self {
        Self::with_tick(event_tx, DEFAULT_BRIDGE_TICK, runtime_config)
    }

    pub fn with_tick(
        event_tx: broadcast::Sender<ControlEvent>,
        tick: Duration,
        runtime_config: Arc<RwLock<ThoughtConfig>>,
    ) -> Self {
        Self::with_existing_health(
            event_tx,
            runtime_config,
            Arc::new(BridgeHealthState::new_with_tick(tick)),
        )
    }

    pub fn with_existing_health(
        event_tx: broadcast::Sender<ControlEvent>,
        runtime_config: Arc<RwLock<ThoughtConfig>>,
        health: Arc<BridgeHealthState>,
    ) -> Self {
        Self {
            event_tx,
            runtime_config,
            health,
        }
    }

    pub fn health(&self) -> Arc<BridgeHealthState> {
        self.health.clone()
    }

    pub fn spawn<P: SessionProvider + 'static>(
        self,
        provider: Arc<P>,
        mut emitter_client: EmitterClient,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let timing = self.health.timing();
            info!(
                tick_ms = timing.tick.as_millis() as u64,
                sync_timeout_ms = timing.sync_timeout.as_millis() as u64,
                "thought bridge runner started"
            );

            let mut delivery_states = provider.thought_delivery_states();
            let metrics = RuntimeThoughtMetricRecorder;
            loop {
                let runtime_config = self.runtime_config.read().await.clone();
                let snapshots = provider.session_snapshots().await;
                let sleep_for = run_bridge_sync_cycle(
                    BridgeSyncCycle {
                        provider: provider.as_ref(),
                        event_tx: &self.event_tx,
                        health: self.health.as_ref(),
                        metrics: &metrics,
                        timing,
                    },
                    &mut emitter_client,
                    &mut delivery_states,
                    &runtime_config,
                    &snapshots,
                )
                .await;
                tokio::time::sleep(sleep_for).await;
            }
        })
    }
}

struct BridgeSyncCycle<'a, P, M> {
    provider: &'a P,
    event_tx: &'a broadcast::Sender<ControlEvent>,
    health: &'a BridgeHealthState,
    metrics: &'a M,
    timing: BridgeTiming,
}

enum BridgeSyncOutcome {
    Success {
        response: SyncResponse,
        duration: Duration,
    },
    Error(EmitterClientError),
    Timeout,
}

enum BridgeFailureLog {
    SyncError,
    SyncTimeout,
}

async fn run_bridge_sync_cycle<P, M>(
    cycle: BridgeSyncCycle<'_, P, M>,
    emitter_client: &mut EmitterClient,
    delivery_states: &mut DeliveryStateMap,
    runtime_config: &ThoughtConfig,
    snapshots: &[SessionInfo],
) -> Duration
where
    P: SessionProvider,
    M: ThoughtMetricRecorder,
{
    prepare_bridge_sync_cycle(delivery_states, cycle.metrics, runtime_config, snapshots);
    let outcome =
        await_bridge_sync_response(emitter_client, runtime_config, snapshots, cycle.timing).await;
    handle_bridge_sync_outcome(cycle, emitter_client, delivery_states, snapshots, outcome).await
}

fn prepare_bridge_sync_cycle<M: ThoughtMetricRecorder>(
    delivery_states: &mut DeliveryStateMap,
    metrics: &M,
    runtime_config: &ThoughtConfig,
    snapshots: &[SessionInfo],
) {
    // Bound the delivery-watermark map to live sessions. It is seeded once
    // before the loop and only ever inserted into, so without this prune it
    // grows without bound across session churn (one entry per pane id the
    // daemon ever reports). Dropping watermarks for sessions tmux no longer
    // lists is safe: a stream restart re-establishes them via emission_seq==1.
    retain_live_delivery_states(delivery_states, snapshots);
    record_sync_attempt_metrics(metrics, runtime_config, snapshots);
}

async fn await_bridge_sync_response(
    emitter_client: &mut EmitterClient,
    runtime_config: &ThoughtConfig,
    snapshots: &[SessionInfo],
    timing: BridgeTiming,
) -> BridgeSyncOutcome {
    let sync_started = Instant::now();
    match tokio::time::timeout(
        timing.sync_timeout,
        emitter_client.next_sync_response(runtime_config, snapshots),
    )
    .await
    {
        Ok(Ok(response)) => BridgeSyncOutcome::Success {
            response,
            duration: sync_started.elapsed(),
        },
        Ok(Err(err)) => BridgeSyncOutcome::Error(err),
        Err(_) => BridgeSyncOutcome::Timeout,
    }
}

async fn handle_bridge_sync_outcome<P, M>(
    cycle: BridgeSyncCycle<'_, P, M>,
    emitter_client: &mut EmitterClient,
    delivery_states: &mut DeliveryStateMap,
    snapshots: &[SessionInfo],
    outcome: BridgeSyncOutcome,
) -> Duration
where
    P: SessionProvider,
    M: ThoughtMetricRecorder,
{
    match outcome {
        BridgeSyncOutcome::Success { response, duration } => {
            record_successful_bridge_sync(&cycle, delivery_states, snapshots, response, duration)
        }
        BridgeSyncOutcome::Error(err) => record_bridge_sync_error(cycle.health, cycle.metrics, err),
        BridgeSyncOutcome::Timeout => {
            record_bridge_sync_timeout(cycle.health, cycle.metrics, cycle.timing, emitter_client)
                .await
        }
    }
}

fn record_successful_bridge_sync<P, M>(
    cycle: &BridgeSyncCycle<'_, P, M>,
    delivery_states: &mut DeliveryStateMap,
    snapshots: &[SessionInfo],
    response: SyncResponse,
    sync_duration: Duration,
) -> Duration
where
    P: SessionProvider,
    M: ThoughtMetricRecorder,
{
    let last_backend_error = response.last_backend_error.clone();
    cycle.health.record_success(last_backend_error.clone());
    if let Some(error) = last_backend_error {
        warn!(
            error = %error,
            "clawgs emit daemon sync reported backend error"
        );
    }
    apply_sync_response_with_metrics(
        cycle.provider,
        cycle.event_tx,
        delivery_states,
        snapshots,
        response,
        cycle.metrics,
        Some(sync_duration),
    );
    cycle.timing.tick
}

fn record_bridge_sync_error<M: ThoughtMetricRecorder>(
    health: &BridgeHealthState,
    metrics: &M,
    err: EmitterClientError,
) -> Duration {
    record_bridge_model_error(metrics, "error");
    let retry_delay = health.next_retry_delay_for_failure();
    let error_text = err.to_string();
    record_bridge_failure(health, error_text, retry_delay, BridgeFailureLog::SyncError);
    retry_delay
}

async fn record_bridge_sync_timeout<M: ThoughtMetricRecorder>(
    health: &BridgeHealthState,
    metrics: &M,
    timing: BridgeTiming,
    emitter_client: &mut EmitterClient,
) -> Duration {
    record_bridge_model_error(metrics, "timeout");
    let retry_delay = health.next_retry_delay_for_failure();
    let error_text = bridge_sync_timeout_error(emitter_client, timing.sync_timeout).await;
    record_bridge_failure(
        health,
        error_text,
        retry_delay,
        BridgeFailureLog::SyncTimeout,
    );
    retry_delay
}

async fn bridge_sync_timeout_error(
    emitter_client: &mut EmitterClient,
    sync_timeout: Duration,
) -> String {
    let mut error_text = format!(
        "clawgs emit daemon sync timed out after {}ms",
        sync_timeout.as_millis()
    );
    if let Err(restart_err) = emitter_client.restart_daemon().await {
        error_text = format!("{error_text}; restart failed: {restart_err}");
    }
    error_text
}

fn record_bridge_failure(
    health: &BridgeHealthState,
    error_text: String,
    retry_delay: Duration,
    log: BridgeFailureLog,
) {
    health.record_failure(error_text.clone(), retry_delay);
    match log {
        BridgeFailureLog::SyncError => {
            warn!(
                error = %error_text,
                retry_delay_ms = retry_delay.as_millis() as u64,
                "clawgs emit daemon sync failed; backing off"
            );
        }
        BridgeFailureLog::SyncTimeout => {
            warn!(
                error = %error_text,
                retry_delay_ms = retry_delay.as_millis() as u64,
                "clawgs emit daemon sync timed out; backing off"
            );
        }
    }
}

/// Drop delivery watermarks for sessions that are no longer present in the
/// latest snapshot set, keeping the long-lived map bounded by the live session
/// count rather than the cumulative count of every session ever observed.
fn retain_live_delivery_states(delivery_states: &mut DeliveryStateMap, snapshots: &[SessionInfo]) {
    if delivery_states.is_empty() {
        return;
    }
    let live: std::collections::HashSet<&str> =
        snapshots.iter().map(|s| s.session_id.as_str()).collect();
    delivery_states.retain(|id, _| live.contains(id.as_str()));
}

#[cfg(test)]
fn apply_sync_response<P: SessionProvider>(
    provider: &P,
    event_tx: &broadcast::Sender<ControlEvent>,
    delivery_states: &mut DeliveryStateMap,
    session_snapshots: &[SessionInfo],
    response: SyncResponse,
) {
    apply_sync_response_with_metrics(
        provider,
        event_tx,
        delivery_states,
        session_snapshots,
        response,
        &RuntimeThoughtMetricRecorder,
        None,
    );
}

fn apply_sync_response_with_metrics<P: SessionProvider, M: ThoughtMetricRecorder>(
    provider: &P,
    event_tx: &broadcast::Sender<ControlEvent>,
    delivery_states: &mut DeliveryStateMap,
    session_snapshots: &[SessionInfo],
    response: SyncResponse,
    metrics: &M,
    generation_latency: Option<Duration>,
) {
    debug!(
        request_id = %response.request_id,
        update_count = response.updates.len(),
        "applying thought updates from daemon"
    );

    record_sync_response_metrics(metrics, session_snapshots, &response, generation_latency);

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
            session_snapshots,
            metrics,
        );
    }
}

const BRIDGE_METRIC_SESSION_ID: &str = "__bridge__";
const THOUGHT_METRIC_PATH_DAEMON: &str = "daemon";
const THOUGHT_METRIC_TIER_BATCH: &str = "batch";

fn thought_state_metric_label(state: crate::types::ThoughtState) -> &'static str {
    match state {
        crate::types::ThoughtState::Active => "active",
        crate::types::ThoughtState::Holding => "holding",
        crate::types::ThoughtState::Sleeping => "sleeping",
    }
}

fn thought_source_suppression_reason(source: ThoughtSource) -> Option<&'static str> {
    match source {
        ThoughtSource::Llm => None,
        ThoughtSource::CarryForward => Some("carry_forward"),
        ThoughtSource::StaticSleeping => Some("static_sleeping"),
    }
}

fn cadence_tier_for_session(session: &SessionInfo) -> &'static str {
    match session.state {
        crate::types::SessionState::Busy
        | crate::types::SessionState::Attention
        | crate::types::SessionState::Error => "hot",
        crate::types::SessionState::Idle => "warm",
        crate::types::SessionState::Exited => "cold",
    }
}

fn cadence_tier_for_session_id(
    session_snapshots: &[SessionInfo],
    session_id: &str,
) -> &'static str {
    session_snapshots
        .iter()
        .find(|snapshot| snapshot.session_id == session_id)
        .map(cadence_tier_for_session)
        .unwrap_or("unknown")
}

fn record_sync_attempt_metrics<M: ThoughtMetricRecorder>(
    metrics: &M,
    runtime_config: &ThoughtConfig,
    snapshots: &[SessionInfo],
) {
    if snapshots.is_empty() {
        metrics.increment_suppression(
            BRIDGE_METRIC_SESSION_ID,
            "no_sessions",
            THOUGHT_METRIC_TIER_BATCH,
        );
        return;
    }

    for snapshot in snapshots {
        let tier = cadence_tier_for_session(snapshot);
        metrics.set_lifecycle_state(
            &snapshot.session_id,
            thought_state_metric_label(snapshot.thought_state),
        );
        if !runtime_config.enabled {
            metrics.increment_suppression(&snapshot.session_id, "disabled", tier);
        }
    }
}

fn record_sync_response_metrics<M: ThoughtMetricRecorder>(
    metrics: &M,
    snapshots: &[SessionInfo],
    response: &SyncResponse,
    generation_latency: Option<Duration>,
) {
    if response.llm_calls > 0 {
        record_sync_model_metrics(metrics, response, generation_latency);
    } else if should_record_no_update_suppression(response) {
        record_no_update_suppression(metrics, snapshots);
    }
}

fn sync_model_outcome(response: &SyncResponse) -> &'static str {
    if response.last_backend_error.is_some() {
        "backend_error"
    } else {
        "success"
    }
}

fn record_sync_model_metrics<M: ThoughtMetricRecorder>(
    metrics: &M,
    response: &SyncResponse,
    generation_latency: Option<Duration>,
) {
    metrics.increment_model_call(
        BRIDGE_METRIC_SESSION_ID,
        THOUGHT_METRIC_PATH_DAEMON,
        THOUGHT_METRIC_TIER_BATCH,
        sync_model_outcome(response),
        response.llm_calls,
    );
    if let Some(duration) = generation_latency {
        metrics.record_generation_latency(
            BRIDGE_METRIC_SESSION_ID,
            THOUGHT_METRIC_PATH_DAEMON,
            THOUGHT_METRIC_TIER_BATCH,
            duration,
        );
    }
}

fn should_record_no_update_suppression(response: &SyncResponse) -> bool {
    response.updates.is_empty()
}

fn record_no_update_suppression<M: ThoughtMetricRecorder>(metrics: &M, snapshots: &[SessionInfo]) {
    if snapshots.is_empty() {
        metrics.increment_suppression(
            BRIDGE_METRIC_SESSION_ID,
            "no_updates",
            THOUGHT_METRIC_TIER_BATCH,
        );
        return;
    }

    for snapshot in snapshots {
        metrics.increment_suppression(
            &snapshot.session_id,
            "no_updates",
            cadence_tier_for_session(snapshot),
        );
    }
}

fn record_bridge_model_error<M: ThoughtMetricRecorder>(metrics: &M, outcome: &'static str) {
    metrics.increment_model_call(
        BRIDGE_METRIC_SESSION_ID,
        THOUGHT_METRIC_PATH_DAEMON,
        THOUGHT_METRIC_TIER_BATCH,
        outcome,
        1,
    );
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
        return current
            .and_then(|state| state.stream_instance_id.as_deref())
            .is_none();
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
    delivery_states: &mut DeliveryStateMap,
    prior_thoughts: &mut std::collections::HashMap<String, Option<String>>,
    batch_stream_instance_id: Option<&str>,
    update: SyncUpdate,
    session_snapshots: &[SessionInfo],
    metrics: &impl ThoughtMetricRecorder,
) {
    let original_thought_source = update.thought_source;
    let update = normalize_sleeping_update(update, prior_thoughts);
    let incoming_delivery = resolved_delivery_state(batch_stream_instance_id, &update);
    let current_delivery = delivery_states.get(&update.session_id);
    if !should_apply_delivery_state(current_delivery, incoming_delivery.as_ref()) {
        metrics.increment_suppression(
            &update.session_id,
            "stale_delivery",
            cadence_tier_for_session_id(session_snapshots, &update.session_id),
        );
        return;
    }

    let tier = cadence_tier_for_session_id(session_snapshots, &update.session_id);
    metrics.set_lifecycle_state(
        &update.session_id,
        thought_state_metric_label(update.thought_state),
    );
    if let Some(reason) = thought_source_suppression_reason(original_thought_source) {
        metrics.increment_suppression(&update.session_id, reason, tier);
    }

    let persisted_delivery = incoming_delivery
        .or_else(|| current_delivery.cloned())
        .unwrap_or_default();
    let persistence_degraded = !provider.persist_thought(
        &update.session_id,
        update.thought.as_deref(),
        update.token_count,
        update.context_limit,
        update.thought_state,
        update.thought_source,
        update.rest_state,
        update.commit_candidate,
        update.action_cues.clone(),
        update.at,
        persisted_delivery.clone(),
        update.objective_changed.then_some(update.at),
        update.objective_fingerprint.clone(),
    );
    if persistence_degraded {
        metrics.increment_suppression(&update.session_id, "persistence_degraded", tier);
    }

    // Only advance the delivery watermark once the update has durably
    // persisted. If persistence is degraded we still broadcast for the live UI
    // below, but leaving the watermark unadvanced ensures a daemon resync /
    // stream restart re-delivers this update rather than treating a
    // never-persisted update as already delivered (silent loss).
    if !persistence_degraded {
        delivery_states.insert(update.session_id.clone(), persisted_delivery);
    }
    prior_thoughts.insert(update.session_id.clone(), update.thought.clone());

    let payload = ThoughtUpdatePayload {
        thought: update.thought.clone(),
        token_count: update.token_count,
        context_limit: update.context_limit,
        thought_state: update.thought_state,
        thought_source: update.thought_source,
        rest_state: update.rest_state,
        commit_candidate: update.commit_candidate,
        action_cues: update.action_cues,
        objective_changed: update.objective_changed,
        bubble_precedence: update.bubble_precedence,
        persistence_degraded,
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
mod tests;
