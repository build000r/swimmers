# Throngterm

Mobile-first web terminal manager for tmux sessions. Animated Thronglet creatures represent sessions in a field; tap one to open its terminal. Accessed over Tailscale. No build step.

## Architecture

- **Server** (`server/`): Node.js — Express HTTP + WebSocket (ws) + node-pty bridging to tmux sessions
- **Client** (`public/`): Vanilla JS — xterm.js terminal, DOM-based Thronglet renderer, app glue
- **Assets** (`public/assets/`): Pixel art sprites (idle.png, walking.png, beep.png, etc.)
- **No build step.** Client libs (xterm.js, FitAddon) loaded from CDN in `public/index.html`

### REST API

- `GET /api/sessions` — list all tmux sessions with state
- `POST /api/sessions` — create new tmux session
- `DELETE /api/sessions/:id` — destroy session (detaches PTY, does not kill tmux)

### WebSocket: `/ws/:sessionId`

Binary protocol. First byte is the message type, remainder is payload.

| Byte | Name              | Direction       | Payload                          |
|------|-------------------|-----------------|----------------------------------|
| 0x01 | resize            | client->server  | JSON `{cols, rows}`              |
| 0x02 | state             | server->client  | JSON `{state, currentCommand}`   |
| 0x03 | exit              | server->client  | (none)                           |
| 0x04 | dismiss-attention | client->server  | (none)                           |
| 0x05 | thought           | server->client  | JSON `{sessionId, thought}`      |
| 0x30 | terminal-data     | bidirectional   | raw terminal bytes               |

This table is the single source of truth for the binary protocol.

### State Machine (StateDetector)

States: `idle`, `busy`, `error`, `attention`
- Uses OSC 133 shell integration when available, falls back to regex prompt detection
- `busy -> idle` starts a 10s timer; if idle persists, transitions to `attention`
- `error` auto-clears after 4s

### ScrollGuard

Coalesces rapid full-screen redraws from cross-client tmux scroll to prevent xterm.js artifacts. Passes through output immediately when ThrongTerm recently sent input.

## File Conventions

```
server/index.js          — HTTP + WS server, REST routes
server/session-manager.js — Session/SessionManager classes, tmux discovery, thought loop
server/state-detector.js  — Terminal state classification (idle/busy/error/attention)
server/scroll-guard.js    — Output coalescing for cross-client tmux redraws
public/index.html         — SPA shell, CDN imports
public/js/app.js          — View routing, zone management, session polling
public/js/terminal.js     — TerminalWrapper (xterm.js + WebSocket lifecycle)
public/js/thronglet.js    — ThrongletRenderer (DOM sprite creatures, wander, drag)
public/assets/            — Pixel art sprites
test/                     — vitest unit tests
```

## Dependencies

Production: `express`, `ws`, `node-pty` only. No frameworks, no bundlers, no transpilers.
Dev/test: `vitest`. Frontend: xterm.js + FitAddon via CDN (no npm frontend deps).

## Commit Format

Conventional commits: `type(scope): description`
Types: `feat`, `fix`, `chore`, `refactor`, `test`, `docs`
Scopes match file areas: `terminal`, `state-detector`, `scroll-guard`, `layout`, `thronglet`, `api`

## Error Handling

- Always log with context: `console.error(\`[session ${name}] message\`, error)`
- Never silently swallow errors — at minimum log them
- PTY/tmux failures must not crash the server

## Testing

- Framework: vitest
- Test files go in `test/` directory
- Run: `npx vitest run`
- Focus on server-side logic (StateDetector, ScrollGuard, SessionManager)