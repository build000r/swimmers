# Metrics Integration Guide

This document describes exactly how to wire the `metrics` module into the
existing throngterm codebase. The metrics module is fully self-contained and
compiles today; this guide covers the integration steps that require modifying
files owned by other agents.

## 1. Module Declaration

Add `mod metrics;` to **both** `src/main.rs` and `src/lib.rs`:

### `src/main.rs` (line 1 area)

```rust
mod metrics;  // Add alongside the other mod declarations
```

### `src/lib.rs`

```rust
pub mod metrics;  // Add alongside the other pub mod declarations
```

## 2. Initialize the Exporter in `main.rs`

In `src/main.rs`, call `metrics::init_metrics()` **after** tracing init but
**before** any actors are spawned (so the recorder is installed before any
metrics are recorded).

### `src/main.rs` ~line 28 (after tracing init, before Config)

```rust
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Initialize Prometheus metrics exporter
    let prom_handle = metrics::init_metrics();

    let config = Config::from_env();
```

## 3. Register the `/metrics` Route

In `src/main.rs` ~line 59, merge the metrics router into the app. The metrics
router carries its own state (`MetricsState`) so it does not conflict with
`AppState`.

```rust
    // Build router
    let app = Router::new()
        .merge(api::api_router())
        .merge(metrics::endpoint::metrics_router(prom_handle))   // <-- ADD THIS
        .nest("/v1/realtime", realtime::handler::ws_router())
        .with_state(state);
```

## 4. Instrument Call Sites

Below are the specific locations where each helper function should be called.

### 4a. Frames Sent / Received (`src/realtime/handler.rs`)

**`increment_frames_received()`** -- call when a binary frame arrives from the client.

File: `src/realtime/handler.rs`, inside `handle_socket()`, line ~93:
```rust
Some(Ok(Message::Binary(data))) => {
    crate::metrics::increment_frames_received();  // <-- ADD
    handle_binary_frame(&data, &state, &mut ws_sink).await;
}
```

**`increment_frames_sent()`** -- call when an output frame is sent to the client.

File: `src/realtime/handler.rs`, inside `handle_socket()`, line ~151:
```rust
PollEvent::Output { session_id, frame } => {
    let binary = codec::encode_output_frame(&session_id, frame.seq, &frame.data);
    crate::metrics::increment_frames_sent();  // <-- ADD
    if ws_sink.send(Message::Binary(binary.into())).await.is_err() {
```

### 4b. Dispatch Latency (`src/realtime/handler.rs`)

Measure the time from receiving a binary input frame to completing the actor
dispatch.

File: `src/realtime/handler.rs`, in `handle_binary_frame()`, line ~246:
```rust
async fn handle_binary_frame(
    data: &[u8],
    state: &Arc<AppState>,
    ws_sink: &mut (impl SinkExt<Message, Error = axum::Error> + Unpin),
) {
    let dispatch_start = std::time::Instant::now();  // <-- ADD
    match codec::decode_input_frame(data) {
        Ok((session_id, input_bytes)) => {
            // ... existing code ...
            if let Err(e) = handle.send(SessionCommand::WriteInput(input_bytes)).await {
                // ... existing error handling ...
            } else {
                crate::metrics::record_dispatch_latency(  // <-- ADD
                    &session_id,
                    dispatch_start.elapsed(),
                );
                crate::metrics::record_keystroke_echo(    // <-- ADD
                    &session_id,
                    dispatch_start.elapsed(),
                );
            }
```

### 4c. Overload Events (`src/session/actor.rs` and `src/realtime/handler.rs`)

**In the actor broadcast** -- when a subscriber is dropped due to a full channel.

File: `src/session/actor.rs`, in `broadcast()`, line ~398:
```rust
Err(mpsc::error::TrySendError::Full(_)) => {
    warn!(/* ... */);
    crate::metrics::increment_overload(&self.session_id);  // <-- ADD
    to_remove.push(client_id);
}
```

**In the WS handler** -- when the overload control event is emitted.

File: `src/realtime/handler.rs`, line ~156:
```rust
PollEvent::Overloaded { session_id } => {
    crate::metrics::increment_overload(&session_id);  // <-- ADD
    if state.supervisor.get_session(&session_id).await.is_some() {
```

### 4d. Replay Truncation (`src/realtime/handler.rs`)

File: `src/realtime/handler.rs`, in `handle_subscribe()`, line ~428:
```rust
Ok(Ok(SubscribeOutcome::ReplayTruncated {
    requested_resume_from_seq,
    replay_window_start_seq,
    latest_seq,
})) => {
    crate::metrics::increment_replay_truncation(&session_id);  // <-- ADD
    let event = ControlEvent { /* ... */ };
```

### 4e. Active Sessions (`src/session/supervisor.rs`)

Update the gauge after any operation that changes the session count.

File: `src/session/supervisor.rs`

After `create_session()` inserts into the map, line ~177:
```rust
sessions.insert(session_id.clone(), handle);
crate::metrics::set_active_sessions(sessions.len());  // <-- ADD
```

After `delete_session()` removes from the map, line ~196:
```rust
sessions
    .remove(session_id)
    .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?
// After the block that holds the write lock:
crate::metrics::set_active_sessions(/* new count */);  // <-- ADD
```

After `discover_tmux_sessions()` completes discovery, line ~145:
```rust
let sessions = self.sessions.read().await;
crate::metrics::set_active_sessions(sessions.len());  // <-- ADD
info!(count = sessions.len(), "tmux session discovery complete");
```

### 4f. Connected Clients (`src/realtime/handler.rs`)

The simplest approach is to use an `AtomicUsize` counter at module scope and
update the gauge on connect/disconnect.

File: `src/realtime/handler.rs`, add near the top (after the existing `NEXT_CLIENT_ID`):
```rust
use std::sync::atomic::AtomicUsize;
static CONNECTED_CLIENTS: AtomicUsize = AtomicUsize::new(0);
```

In `handle_socket()`, after line ~83 (`"realtime client connected"`):
```rust
tracing::info!("realtime client connected");
let client_count = CONNECTED_CLIENTS.fetch_add(1, Ordering::Relaxed) + 1;
crate::metrics::set_connected_clients(client_count);
```

At function exit (before the final `"realtime client cleanup complete"` log), line ~200:
```rust
let client_count = CONNECTED_CLIENTS.fetch_sub(1, Ordering::Relaxed) - 1;
crate::metrics::set_connected_clients(client_count);
tracing::info!("realtime client cleanup complete");
```

### 4g. Queue Depth and Bytes (`src/session/actor.rs`)

After each `broadcast()` call in the actor, report the aggregate queue state
for all subscribers. This is approximate (uses channel capacity as a proxy).

File: `src/session/actor.rs`, in `handle_pty_output()` after broadcasting, line ~385:
```rust
self.broadcast(frame).await;

// Report aggregate queue depth across subscribers
let total_depth: usize = self.subscribers.values()
    .map(|tx| tx.max_capacity() - tx.capacity())
    .sum();
crate::metrics::record_queue_depth(&self.session_id, total_depth);
```

Note: Exact byte tracking would require wrapping `OutputFrame` sizes. As a
first pass, depth-only is sufficient. Byte tracking can be added by
accumulating `frame.data.len()` in the actor state.

### 4h. Transport Health Transitions

Currently `TransportHealth` is set statically in `build_summary()`. When a
transport health state machine is implemented, call:

```rust
crate::metrics::record_transport_health_transition("healthy", "degraded");
```

at the point where the transition is detected. This is a placeholder for
future work.

## 5. Verification

After wiring, start the server and verify:

```bash
curl http://localhost:3210/metrics
```

You should see Prometheus text output with `# HELP` and `# TYPE` lines for
all defined metrics, plus any that have been recorded.

## Summary of Files to Modify

| File | Changes |
|------|---------|
| `src/main.rs` | Add `mod metrics;`, call `init_metrics()`, merge `metrics_router` |
| `src/lib.rs` | Add `pub mod metrics;` |
| `src/realtime/handler.rs` | frames_sent, frames_received, dispatch_latency, keystroke_echo, overload, replay_truncation, connected_clients |
| `src/session/actor.rs` | overload (in broadcast), queue_depth, queue_bytes |
| `src/session/supervisor.rs` | active_sessions gauge |
