//! Performance telemetry for Throngterm.
//!
//! Uses the `metrics` facade crate with `metrics-exporter-prometheus` for
//! Prometheus-compatible exposition at `GET /metrics`.
//!
//! # Architecture
//!
//! This module is intentionally self-contained — it has **no imports** from
//! other throngterm modules. Other modules call the recording helper functions
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

/// Server dispatch latency: ingress frame -> actor dispatch -> egress enqueue.
const DISPATCH_LATENCY: &str = "throngterm_dispatch_latency_seconds";

/// End-to-end keystroke echo (backend-measured dispatch portion).
const KEYSTROKE_ECHO: &str = "throngterm_keystroke_echo_seconds";

/// Current outbound queue depth per session actor.
const QUEUE_DEPTH: &str = "throngterm_session_queue_depth";

/// Current outbound queue byte size per session actor.
const QUEUE_BYTES: &str = "throngterm_session_queue_bytes";

/// Number of active session actors.
const ACTIVE_SESSIONS: &str = "throngterm_active_sessions";

/// Number of connected WebSocket clients.
const CONNECTED_CLIENTS: &str = "throngterm_connected_clients";

/// Overload events emitted.
const OVERLOAD_EVENTS: &str = "throngterm_overload_events_total";

/// Replay truncation events.
const REPLAY_TRUNCATION: &str = "throngterm_replay_truncation_total";

/// Subscription lifecycle events (subscribe/unsubscribe/idempotent_unsubscribe).
const SUBSCRIPTION_LIFECYCLE: &str = "throngterm_subscription_lifecycle_total";

/// Transport health state transitions.
const TRANSPORT_TRANSITIONS: &str = "throngterm_transport_health_transitions_total";

/// Total binary frames sent to clients.
const FRAMES_SENT: &str = "throngterm_frames_sent_total";

/// Total binary frames received from clients.
const FRAMES_RECEIVED: &str = "throngterm_frames_received_total";

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
    describe_histogram!(
        DISPATCH_LATENCY,
        "Server dispatch latency (ingress frame -> actor dispatch -> egress enqueue)"
    );
    describe_histogram!(
        KEYSTROKE_ECHO,
        "End-to-end keystroke echo latency (backend dispatch portion)"
    );
    describe_gauge!(
        QUEUE_DEPTH,
        "Current outbound queue depth per session actor"
    );
    describe_gauge!(
        QUEUE_BYTES,
        "Current outbound queue byte size per session actor"
    );
    describe_gauge!(ACTIVE_SESSIONS, "Number of active session actors");
    describe_gauge!(CONNECTED_CLIENTS, "Number of connected WebSocket clients");
    describe_counter!(OVERLOAD_EVENTS, "Total overload events emitted");
    describe_counter!(REPLAY_TRUNCATION, "Total replay truncation events");
    describe_counter!(
        SUBSCRIPTION_LIFECYCLE,
        "Total subscription lifecycle events by action"
    );
    describe_counter!(
        TRANSPORT_TRANSITIONS,
        "Total transport health state transitions"
    );
    describe_counter!(FRAMES_SENT, "Total binary frames sent to clients");
    describe_counter!(FRAMES_RECEIVED, "Total binary frames received from clients");

    handle
}

// ---------------------------------------------------------------------------
// Recording helpers
// ---------------------------------------------------------------------------

/// Record the server dispatch latency for a session.
///
/// Call sites: `src/session/actor.rs` (in the PTY output -> broadcast path)
/// and `src/realtime/handler.rs` (binary frame ingress -> actor dispatch).
pub fn record_dispatch_latency(session_id: &str, duration: Duration) {
    histogram!(DISPATCH_LATENCY, "session_id" => session_id.to_owned())
        .record(duration.as_secs_f64());
}

/// Record the keystroke echo latency (backend dispatch portion).
///
/// Call site: `src/realtime/handler.rs` (measure time from receiving a
/// TERMINAL_INPUT binary frame to dispatching WriteInput to the actor).
pub fn record_keystroke_echo(session_id: &str, duration: Duration) {
    histogram!(KEYSTROKE_ECHO, "session_id" => session_id.to_owned())
        .record(duration.as_secs_f64());
}

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

/// Set the total number of connected WebSocket clients.
///
/// Call site: `src/realtime/handler.rs` (increment on connect, decrement on
/// disconnect — or maintain an AtomicUsize and set the gauge from it).
pub fn set_connected_clients(count: usize) {
    gauge!(CONNECTED_CLIENTS).set(count as f64);
}

/// Increment the overload event counter for a session.
///
/// Call site: `src/session/actor.rs` (in `broadcast()` when a subscriber
/// channel is full and the client is dropped — SESSION_OVERLOADED) and
/// `src/realtime/handler.rs` (when emitting `session_overloaded` control event).
pub fn increment_overload(session_id: &str) {
    counter!(OVERLOAD_EVENTS, "session_id" => session_id.to_owned()).increment(1);
}

/// Increment the replay truncation counter for a session.
///
/// Call site: `src/realtime/handler.rs` (when the subscribe ack returns
/// `SubscribeOutcome::ReplayTruncated` and the `replay_truncated` control
/// event is emitted to the client).
pub fn increment_replay_truncation(session_id: &str) {
    counter!(REPLAY_TRUNCATION, "session_id" => session_id.to_owned()).increment(1);
}

/// Increment subscription lifecycle events.
///
/// `action` should be one of: "subscribe", "unsubscribe",
/// "idempotent_unsubscribe".
pub fn increment_subscription_lifecycle(session_id: &str, action: &str) {
    counter!(
        SUBSCRIPTION_LIFECYCLE,
        "session_id" => session_id.to_owned(),
        "action" => action.to_owned()
    )
    .increment(1);
}

/// Record a transport health state transition.
///
/// Call site: wherever `TransportHealth` transitions are detected (currently
/// this would be in the session actor or a future health-monitoring component).
pub fn record_transport_health_transition(from_state: &str, to_state: &str) {
    counter!(
        TRANSPORT_TRANSITIONS,
        "from_state" => from_state.to_owned(),
        "to_state" => to_state.to_owned()
    )
    .increment(1);
}

/// Increment the total binary frames sent counter.
///
/// Call site: `src/realtime/handler.rs` (each time a `Message::Binary` output
/// frame is sent to the WebSocket client).
pub fn increment_frames_sent() {
    counter!(FRAMES_SENT).increment(1);
}

/// Increment the total binary frames received counter.
///
/// Call site: `src/realtime/handler.rs` (each time a `Message::Binary` input
/// frame is received from the WebSocket client).
pub fn increment_frames_received() {
    counter!(FRAMES_RECEIVED).increment(1);
}
