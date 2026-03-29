# Swimmers

Native terminal session manager for tmux. The supported surface is the Rust TUI in `src/bin/swimmers-tui.rs`, backed by a small Axum HTTP API and optional repo-local sprite/theme overrides.

## Architecture

- **Server** (`src/`): Rust, Axum HTTP, Tokio async runtime, portable-pty bridging to tmux sessions
- **Client** (`src/bin/swimmers-tui.rs`): Rust TUI for session browsing, thoughts, repo actions, and native terminal handoff
- **Persistence** (`data/swimmers/`): file-based session snapshots and thought runtime config
- **Metrics**: Prometheus exposition at `GET /metrics`

## API

All supported routes live under `/v1/`:

- `GET /v1/sessions` — list tmux sessions with current state
- `POST /v1/sessions` — create a tmux session
- `DELETE /v1/sessions/{session_id}` — remove a session
- `GET /v1/sessions/{session_id}/snapshot` — capture visible screen text
- `GET /v1/sessions/{session_id}/pane-tail` — capture recent pane output
- `POST /v1/sessions/{session_id}/attention/dismiss` — clear attention state
- `POST /v1/sessions/{session_id}/input` — send text input into a session
- `GET /v1/selection` / `POST /v1/selection` — read or publish the selected session
- `GET /v1/native/status` / `POST /v1/native/open` — desktop terminal handoff
- `GET /v1/dirs` / `POST /v1/dirs/restart` — repo browsing and mapped service restarts
- `GET /v1/skills/{tool}` — list available skills for a tool
- `GET /v1/thought-config` / `PUT /v1/thought-config` — read or update thought runtime config

## Runtime model

- `SessionSupervisor` owns session actors, tmux discovery, lifecycle broadcasts, and persistence checkpoints.
- `SessionActor` owns PTY I/O, replay buffering, scroll-guard coalescing, state detection, and session summaries.
- The thought subsystem publishes in-process updates; there is no supported browser/realtime transport layer.
- `ScrollGuard` coalesces redraw bursts from multi-client tmux scrolling so the TUI sees the final frame instead of intermediate garbage.

## File map

```
src/
  main.rs                     — binary entry point, startup, router assembly
  lib.rs                      — crate exports
  config.rs                   — runtime configuration
  types.rs                    — shared API and event types
  api/
    mod.rs                    — router composition and AppState
    dirs.rs                   — directory browser + service restart endpoints
    native.rs                 — native terminal handoff endpoints
    selection.rs              — published selection endpoints
    sessions.rs               — session CRUD and session detail endpoints
    skills.rs                 — skill listing endpoints
    thought_config.rs         — thought runtime config endpoints
  session/
    actor.rs                  — per-session PTY actor
    supervisor.rs             — tmux discovery, lifecycle, persistence hooks
    replay_ring.rs            — bounded replay buffer
  scroll/
    guard.rs                  — redraw coalescing
  state/
    detector.rs               — shell state classification
  thought/
    bridge_runner.rs          — daemon-backed thought loop
    loop_runner.rs            — in-process compatibility runner
    context.rs                — thought/context extraction
    runtime_config.rs         — persisted runtime tuning
  metrics/
    mod.rs                    — metric helpers and registration
    endpoint.rs               — `/metrics`
  persistence/
    file_store.rs             — local persistence
```

## Commit format

Conventional commits: `type(scope): description`
Suggested scopes: `tui`, `session`, `thought`, `api`, `metrics`, `native`, `scroll`, `persistence`, `docs`

## Testing

- `cargo test`
- `bash ./scripts/test-run-tui.sh`
