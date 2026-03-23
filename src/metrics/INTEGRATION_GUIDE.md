# Metrics Integration Guide

The metrics stack is already wired into the current TUI-first server.

## Current wiring

- `src/main.rs` initializes the Prometheus recorder with `metrics::init_metrics()`.
- `src/main.rs` merges `metrics::endpoint::metrics_router(...)`, which serves `GET /metrics`.
- `src/session/supervisor.rs` records active-session counts.
- `src/session/actor.rs` records queue depth and overload events.
- Thought lifecycle, suppression, model-call, and generation-latency metrics live in `src/metrics/mod.rs`.

## Current scope

The exported metrics describe the supported runtime:

- session counts
- session outbound queue depth and bytes
- overload events
- thought lifecycle state
- thought suppression/model-call totals
- thought generation latency

The old realtime transport metrics were removed along with the legacy non-TUI client.

## When adding metrics

- Keep new helpers inside `src/metrics/mod.rs`.
- Record them from call sites instead of importing other modules into `metrics`.
- Expose them automatically through `GET /metrics`; no extra route work is needed.

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
