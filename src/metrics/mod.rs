//! Performance telemetry for Swimmers.
//!
//! Uses the `metrics` facade crate with `metrics-exporter-prometheus` for
//! Prometheus-compatible exposition at `GET /metrics`.
//!
//! # Architecture
//!
//! This module is intentionally self-contained — it has **no imports** from
//! other swimmers modules. Other modules call the recording helper functions
//! defined here; the module never reaches into actor or handler internals.
//!
//! # Initialization
//!
//! Call [`init_metrics`] once during server startup (before any metrics are
//! recorded). It installs the Prometheus recorder globally and returns a
//! [`metrics_exporter_prometheus::PrometheusHandle`] that the HTTP handler
//! uses to render the scrape output.

pub mod endpoint;

use std::time::Duration;

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

// ---------------------------------------------------------------------------
// Metric names (constants to avoid typos)
// ---------------------------------------------------------------------------

/// Current outbound queue depth per session actor.
const QUEUE_DEPTH: &str = "swimmers_session_queue_depth";

/// Current outbound queue byte size per session actor.
const QUEUE_BYTES: &str = "swimmers_session_queue_bytes";

/// Number of active session actors.
const ACTIVE_SESSIONS: &str = "swimmers_active_sessions";

/// Overload events emitted.
const OVERLOAD_EVENTS: &str = "swimmers_overload_events_total";

/// Per-session lifecycle-state gauge (one-hot by `state` label).
const THOUGHT_LIFECYCLE_STATE: &str = "swimmers_thought_lifecycle_state";

/// Thought model call outcomes by generation path + cadence tier.
const THOUGHT_MODEL_CALLS: &str = "swimmers_thought_model_calls_total";

/// Thought suppression counters by reason + cadence tier.
const THOUGHT_SUPPRESSIONS: &str = "swimmers_thought_suppressions_total";

/// Thought generation latency (LLM path only).
const THOUGHT_GENERATION_LATENCY: &str = "swimmers_thought_generation_seconds";

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Install the Prometheus metrics recorder globally and register metric
/// descriptions.
///
/// Returns a [`PrometheusHandle`] whose `.render()` method produces the
/// Prometheus text exposition format for the `/metrics` endpoint.
///
/// # Panics
///
/// Panics if a global recorder has already been installed (call this exactly
/// once).
pub fn init_metrics() -> PrometheusHandle {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus metrics recorder");

    // Register descriptions so Prometheus TYPE/HELP lines are emitted.
    describe_gauge!(
        QUEUE_DEPTH,
        "Current outbound queue depth per session actor"
    );
    describe_gauge!(
        QUEUE_BYTES,
        "Current outbound queue byte size per session actor"
    );
    describe_gauge!(ACTIVE_SESSIONS, "Number of active session actors");
    describe_counter!(OVERLOAD_EVENTS, "Total overload events emitted");
    describe_gauge!(
        THOUGHT_LIFECYCLE_STATE,
        "Per-session thought lifecycle state (labels: session_id, state)"
    );
    describe_counter!(
        THOUGHT_MODEL_CALLS,
        "Thought model call outcomes by path/tier/outcome"
    );
    describe_counter!(
        THOUGHT_SUPPRESSIONS,
        "Thought suppressions by reason and cadence tier"
    );
    describe_histogram!(
        THOUGHT_GENERATION_LATENCY,
        "Thought generation latency by path and cadence tier"
    );

    handle
}

// ---------------------------------------------------------------------------
// Recording helpers
// ---------------------------------------------------------------------------

/// Update the current outbound queue depth gauge for a session.
///
/// Call site: `src/session/actor.rs` (after each broadcast, report the
/// subscriber channel's current capacity usage).
pub fn record_queue_depth(session_id: &str, depth: usize) {
    gauge!(QUEUE_DEPTH, "session_id" => session_id.to_owned()).set(depth as f64);
}

/// Update the current outbound queue byte size gauge for a session.
///
/// Call site: `src/session/actor.rs` (alongside queue depth updates).
#[allow(dead_code)]
pub fn record_queue_bytes(session_id: &str, bytes: usize) {
    gauge!(QUEUE_BYTES, "session_id" => session_id.to_owned()).set(bytes as f64);
}

/// Set the total number of active session actors.
///
/// Call site: `src/session/supervisor.rs` (after create_session, delete_session,
/// and discover_tmux_sessions — any operation that changes the session count).
pub fn set_active_sessions(count: usize) {
    gauge!(ACTIVE_SESSIONS).set(count as f64);
}

/// Increment the overload event counter for a session.
///
/// Call site: `src/session/actor.rs` (in `broadcast()` when a subscriber
/// channel is full and the client is dropped).
pub fn increment_overload(session_id: &str) {
    counter!(OVERLOAD_EVENTS, "session_id" => session_id.to_owned()).increment(1);
}

/// Set per-session lifecycle state as a one-hot gauge by state label.
pub fn set_thought_lifecycle_state(session_id: &str, state: &str) {
    for candidate in ["active", "holding", "sleeping"] {
        let value = if candidate == state { 1.0 } else { 0.0 };
        gauge!(
            THOUGHT_LIFECYCLE_STATE,
            "session_id" => session_id.to_owned(),
            "state" => candidate.to_string()
        )
        .set(value);
    }
}

/// Increment thought model-call counters by path/tier/outcome.
pub fn increment_thought_model_call(session_id: &str, path: &str, tier: &str, outcome: &str) {
    counter!(
        THOUGHT_MODEL_CALLS,
        "session_id" => session_id.to_owned(),
        "path" => path.to_owned(),
        "tier" => tier.to_owned(),
        "outcome" => outcome.to_owned()
    )
    .increment(1);
}

/// Increment thought suppression counters by reason and cadence tier.
pub fn increment_thought_suppression(session_id: &str, reason: &str, tier: &str) {
    counter!(
        THOUGHT_SUPPRESSIONS,
        "session_id" => session_id.to_owned(),
        "reason" => reason.to_owned(),
        "tier" => tier.to_owned()
    )
    .increment(1);
}

/// Record thought generation latency by path/tier.
pub fn record_thought_generation_latency(
    session_id: &str,
    path: &str,
    tier: &str,
    duration: Duration,
) {
    histogram!(
        THOUGHT_GENERATION_LATENCY,
        "session_id" => session_id.to_owned(),
        "path" => path.to_owned(),
        "tier" => tier.to_owned()
    )
    .record(duration.as_secs_f64());
}
