# Throngterm

Mobile-first web terminal manager for tmux sessions. Animated Thronglet creatures represent sessions in a field; tap one to open its terminal. Accessed over Tailscale.

## Architecture

- **Backend** (`src/`): Rust — Axum HTTP + WebSocket, Tokio async runtime, portable-pty bridging to tmux sessions
- **Frontend** (`web/`): Preact + TypeScript + Vite — xterm.js terminal, DOM-based Thronglet renderer
- **Assets** (`web/public/assets/`): Pixel art sprites (idle.png, walking.png, beep.png, etc.)
- **Build output**: `cargo build` for server binary, `cd web && npm run build` produces `dist/`

### REST API

All routes under `/v1/`:

- `GET /v1/bootstrap` — initial client handshake: config, realtime URL, session list
- `GET /v1/sessions` — list all tmux sessions with state
- `POST /v1/sessions` — create new tmux session
- `DELETE /v1/sessions/:id` — destroy session
- `GET /v1/sessions/:id/snapshot` — terminal screen text snapshot

### Realtime: `/v1/realtime`

Single multiplexed WebSocket. Carries both JSON control messages and binary terminal frames.

**Binary frames** (terminal I/O — big-endian):

| Opcode | Name            | Direction       | Layout                                                        |
|--------|-----------------|-----------------|---------------------------------------------------------------|
| 0x10   | TERMINAL_INPUT  | client→server   | `u8 opcode \| u16 session_id_len \| session_id \| raw_input`  |
| 0x11   | TERMINAL_OUTPUT | server→client   | `u8 opcode \| u16 session_id_len \| session_id \| u64 seq \| raw_output` |

**JSON control messages** (server→client events):

| Event              | Payload struct              | Purpose                              |
|--------------------|-----------------------------|--------------------------------------|
| session_state      | SessionStatePayload         | State transition notification        |
| session_title      | SessionTitlePayload         | Terminal title change                 |
| thought_update     | ThoughtUpdatePayload        | AI agent thought + context usage      |
| session_created    | SessionCreatedPayload       | New session discovered or created     |
| session_deleted    | SessionDeletedPayload       | Session removed                       |
| session_subscription | SessionSubscriptionPayload | Subscribe/unsubscribe acknowledgment  |
| replay_truncated   | ReplayTruncatedPayload      | Requested seq fell outside replay ring|
| session_overloaded | SessionOverloadedPayload    | Backpressure signal                   |
| control_error      | ControlErrorPayload         | Error response                        |

**JSON control messages** (client→server):

| Type                 | Payload struct              | Purpose                        |
|----------------------|-----------------------------|--------------------------------|
| subscribe_session    | SubscribeSessionPayload     | Start receiving terminal output|
| unsubscribe_session  | UnsubscribeSessionPayload   | Stop receiving terminal output |
| resize               | ResizePayload               | Resize PTY                     |
| dismiss_attention    | DismissAttentionPayload     | Clear attention state          |

This table is the single source of truth for the realtime protocol.

### State Machine (StateDetector)

States: `idle`, `busy`, `error`, `attention`, `exited`
- Uses OSC 133 shell integration when available, falls back to regex prompt detection
- `busy → idle` starts a 10s timer; if idle persists, transitions to `attention`
- `error` auto-clears after 4s

### ScrollGuard

Coalesces rapid full-screen redraws from cross-client tmux scroll to prevent xterm.js artifacts. Passes through output immediately when ThrongTerm recently sent input.

## File Conventions

```
src/
  main.rs                     — binary entry point, Axum server setup
  lib.rs                      — crate root, module declarations
  config.rs                   — runtime configuration (env vars, defaults)
  types.rs                    — shared types, opcodes, control event structs
  api/
    mod.rs                    — router composition, AppState
    bootstrap.rs              — GET /v1/bootstrap
    sessions.rs               — CRUD /v1/sessions
  auth/
    mod.rs                    — Tailscale auth middleware
  realtime/
    mod.rs                    — realtime module root
    handler.rs                — WebSocket upgrade + message dispatch
    codec.rs                  — binary frame encode/decode
  session/
    mod.rs                    — session module root
    actor.rs                  — per-session async actor (PTY + state + output)
    supervisor.rs             — manages all session actors, tmux discovery
    replay_ring.rs            — bounded ring buffer for terminal output replay
  state/
    mod.rs                    — state module root
    detector.rs               — terminal state classification
  scroll/
    mod.rs                    — scroll module root
    guard.rs                  — output coalescing for cross-client redraws
  thought/
    mod.rs                    — thought module root
    loop_runner.rs            — periodic AI agent thought polling
    context.rs                — reads agent JSONL for context/token tracking
  metrics/
    mod.rs                    — metrics module root
    endpoint.rs               — Prometheus metrics endpoint
  persistence/
    mod.rs                    — persistence module root
    file_store.rs             — file-based session state persistence

web/
  src/
    main.tsx                  — Preact app entry
    app.tsx                   — top-level App component, routing
    types.ts                  — TypeScript types mirroring src/types.rs
    components/
      OverviewField.tsx       — Thronglet field / session overview
      TerminalWorkspace.tsx   — xterm.js terminal view
      ZoneManager.tsx         — zone layout and navigation
    hooks/
      useGestures.ts          — touch/pointer gesture handling
      useObserverMode.ts      — read-only observer mode
      useTerminalCache.ts     — terminal instance caching
    services/
      api.ts                  — REST API client
      realtime.ts             — WebSocket realtime client
      workspace-history.ts    — workspace navigation history
    __tests__/                — vitest frontend tests
  public/
    assets/                   — pixel art sprites
```

## Dependencies

**Backend (Cargo.toml):** axum, tokio, portable-pty, serde/serde_json, tower-http, tracing, uuid, chrono, regex, reqwest, metrics, dotenvy. No Node.js.

**Frontend (web/package.json):** preact, @preact/signals, @xterm/xterm, @xterm/addon-fit, @xterm/addon-webgl. Dev: vite, typescript, vitest, @testing-library/preact.

## Commit Format

Conventional commits: `type(scope): description`
Types: `feat`, `fix`, `chore`, `refactor`, `test`, `docs`
Scopes match file areas: `terminal`, `state`, `scroll`, `session`, `realtime`, `thought`, `api`, `metrics`, `frontend`, `thronglet`

## Error Handling

- Always log with context: `tracing::error!(session_id = %id, "message: {err}")`
- Never silently swallow errors — at minimum log them
- PTY/tmux failures must not crash the server
- Use `anyhow::Result` for fallible operations, `thiserror` for typed errors in public APIs

## Testing

- **Rust:** `cargo test` — unit tests inline in source modules
- **Frontend:** `cd web && npx vitest run` — component + service tests in `web/src/__tests__/`
