# Swimmers

Native terminal session manager for tmux. The supported surface is the Rust TUI in `src/bin/swimmers_tui/`, backed by a small Axum HTTP API and optional repo-local sprite/theme overrides.

## Architecture

- **Server** (`src/`): Rust, Axum HTTP, Tokio async runtime, portable-pty bridging to tmux sessions
- **Client** (`src/bin/swimmers_tui/`): Rust TUI for session browsing, thoughts, repo actions, and native terminal handoff
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
- `GET /v1/skills?tool={tool}` — list available skills for a tool
- `GET /v1/thought-config` / `PUT /v1/thought-config` — read or update thought runtime config

## Runtime model

- `SessionSupervisor` owns session actors, tmux discovery, lifecycle broadcasts, and persistence checkpoints.
- `SessionActor` owns PTY I/O, replay buffering, scroll-guard coalescing, state detection, and session summaries.
- The thought subsystem publishes in-process updates; `src/web/` provides a browser-based remote-attach surface for convenience, though the TUI remains the flagship experience.
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
    health.rs                 — health check endpoint
    native.rs                 — native terminal handoff endpoints
    selection.rs              — published selection endpoints
    service.rs                — service management endpoints
    sessions.rs               — session CRUD and session detail endpoints
    skills.rs                 — skill listing endpoints
    thought_config.rs         — thought runtime config endpoints
    web_actions.rs            — web-surface action endpoints
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
  bin/
    swimmers_tui/
      mod.rs                  — TUI entry point
      api.rs                  — API client helpers
      app.rs                  — application state
      commit.rs               — commit flow UI
      entity.rs               — swimmer entity rendering
      events.rs               — input/event handling
      in_process.rs           — in-process server bridge
      layout.rs               — layout computation
      lifecycle.rs            — startup and shutdown
      mermaid.rs              — mermaid diagram support
      picker.rs               — interactive picker widget
      render.rs               — frame rendering
      terminal.rs             — terminal setup and teardown
      thought_config_editor.rs — thought config editing UI
      thoughts.rs             — thought display panel
      voice.rs                — voice input support
      tests/                  — TUI integration tests
  web/
    mod.rs                    — browser remote-attach surface
    app.css                   — web UI styles
    app.js                    — web UI logic
    input_support.js          — input handling helpers
    rendered_surface.js       — terminal surface renderer
  persistence/
    file_store.rs             — local persistence
```

## Commit format

Conventional commits: `type(scope): description`
Suggested scopes: `tui`, `session`, `thought`, `api`, `metrics`, `native`, `scroll`, `persistence`, `docs`

## Testing

- `cargo test`
- `bash ./scripts/test-run-tui.sh`
