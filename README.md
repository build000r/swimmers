# swimmers

<div align="center">

```
   o   .          o O  .        z z
><o)))'>        ><O)))'>          ><-)))'>
  /_/_            /_/_              \_\
      .             O   o
  active            busy           sleeping
```

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/build000r/swimmers/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)](https://www.rust-lang.org/)

**Quick Start**

```bash
cargo install swimmers
swimmers-tui        # opens the aquarium — no server process to manage
```

</div>

A terminal aquarium for your tmux sessions. Each session becomes an animated fish whose behavior reflects its real-time state — swimming when active, bubbling when busy, dozing when idle. Backed by a Rust API server that discovers and manages tmux sessions, with a native TUI client that renders the whole thing as a fish bowl you can navigate, inspect, and control.

---

## TL;DR

**The Problem**: You have a dozen tmux sessions running across a machine. Listing them with `tmux ls` gives you cryptic one-liners. You can't tell at a glance which sessions are busy, which are idle, which need attention, and which have errored out. Switching between them is a context-destroying exercise in remembering session names.

**The Solution**: Swimmers turns your tmux sessions into a visual fish bowl. Each session is an animated ASCII fish. Active sessions swim, busy ones blow bubbles, sleeping ones sink to the bottom, errored ones show `x` eyes. Select a fish to inspect its pane output, open it in your desktop terminal, or read the thought stream from your AI coding agents.

### Why swimmers?

| Feature | What It Does |
|---------|--------------|
| **Aquarium view** | Sessions rendered as animated ASCII fish with state-driven sprites |
| **Live state detection** | Idle, busy, error, attention, drowsy, sleeping, deep sleep, exited |
| **Thought rail** | Side panel showing AI agent thought streams per session |
| **Native terminal handoff** | Open any session directly in iTerm or Ghostty from the TUI |
| **Mermaid diagrams** | Render and zoom Mermaid artifacts inline in the terminal |
| **Repo themes** | Per-repo colors plus default sprite overrides via `.swimmers/colors.json` |
| **Remote API** | Point the TUI at a remote server over Tailscale or any network |
| **Prometheus metrics** | `GET /metrics` for monitoring session counts and API health |
| **No database, no Docker** | File-based persistence, single binary, tmux is the only dependency |

---

## Installation

### From crates.io

```bash
cargo install swimmers
```

That installs **two binaries** on your `PATH`:

- `swimmers-tui` — the aquarium TUI. By default it hosts the API in-process, so one command is enough to get started.
- `swimmers` — the standalone Axum HTTP/WebSocket API server. Use it when you want a long-running headless server, multiple TUI clients against one backend, or remote access over Tailscale. Run it as `swimmers` or, equivalently, `swimmers serve`.

No repo checkout required.

### Prerequisites

| Dependency | Install |
|------------|---------|
| Rust toolchain (1.82+) | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| tmux | `brew install tmux` (macOS) or `apt install tmux` (Debian/Ubuntu) |
| Tailscale (optional) | Only needed for remote API access over a tailnet |

### From Source

```bash
git clone https://github.com/build000r/swimmers.git
cd swimmers
cargo build --release
```

Binaries land in `target/release/swimmers` (API server) and `target/release/swimmers-tui` (TUI client). You can also run `cargo install --path .` from inside the checkout.

To build the experimental local voice-input path for the TUI, enable the `voice` feature:

```bash
cargo build --release --features voice
SWIMMERS_VOICE_MODEL=~/models/ggml-base.en.bin target/release/swimmers-tui
```

---

## Quick Start

After `cargo install swimmers`, both `swimmers` and `swimmers-tui` are on your PATH. No clone required.

1. **Open the TUI**

   ```bash
   swimmers-tui
   ```

   The TUI hosts the API in-process by default — no separate server to start, no port to worry about, no handshake message while you wait for something to boot. Quit with `q` and the whole thing exits cleanly.

2. **Create some tmux sessions** if you don't have any yet

   ```bash
   tmux new-session -d -s dev
   tmux new-session -d -s logs
   tmux new-session -d -s deploy
   ```

   They appear in the aquarium within seconds.

3. **Navigate** — arrow keys to select a fish, Enter to open the session in your terminal, `q` to quit the TUI.

### Experimental voice input

In a source build with `--features voice`, open the initial-request composer and press `Ctrl-V` to start or stop microphone capture. Swimmers records locally, transcribes with a local Whisper model, and inserts the transcript into the composer so you can edit it before creating the hidden swimmer.

### External server mode

Set `SWIMMERS_TUI_URL` to run the API as a separate process (for multi-client access, remote access over Tailscale, or integration with the REST endpoints from `curl`/browser). The TUI switches to HTTP transport and, for loopback URLs, auto-spawns a sibling `swimmers` binary if one isn't already listening — using a readiness pipe instead of polling, so the handoff is invisible.

```bash
SWIMMERS_TUI_URL=http://127.0.0.1:3210 swimmers-tui
```

To run the API explicitly as a standalone headless server:

```bash
swimmers          # same as `swimmers serve`
# or
swimmers serve    # explicit form for service managers and docs
```

`Ctrl-C` stops it. `kill $(lsof -ti:3210)` works for a backgrounded instance.

---

## Bind Address and Network Access

Bind addresses only apply to the standalone `swimmers` / `swimmers serve` binary. Embedded mode (`swimmers-tui` with no `SWIMMERS_TUI_URL`) binds no socket at all.

By default the server binds to **`127.0.0.1:3210`** (loopback only).

### Loopback (default, no auth required)

```bash
swimmers                                            # binds 127.0.0.1:3210
SWIMMERS_TUI_URL=http://127.0.0.1:3210 swimmers-tui # opt into external HTTP transport
```

### External / Tailscale access

Set `SWIMMERS_BIND` to expose the server on a non-loopback interface. The server refuses to start if you pair a non-loopback bind with `AUTH_MODE=local_trust`; for external exposure, switch to `AUTH_MODE=token` and set `AUTH_TOKEN`.

```bash
# Bind to all interfaces (e.g., for Tailscale access from another machine)
SWIMMERS_BIND=0.0.0.0 \
AUTH_MODE=token \
AUTH_TOKEN=your-secret-token \
swimmers

# Bind to a specific Tailscale IP
SWIMMERS_BIND=100.101.123.63 \
AUTH_MODE=token \
AUTH_TOKEN=your-secret-token \
swimmers

# Point the TUI at the remote server
SWIMMERS_TUI_URL=http://100.101.123.63:3210 \
AUTH_MODE=token \
AUTH_TOKEN=your-secret-token \
swimmers-tui
```

For any non-loopback bind, use `AUTH_MODE=token` with `AUTH_TOKEN`. `OBSERVER_TOKEN` is optional when you also want a read-only credential for browser or observer clients.

---

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `SWIMMERS_BIND` | `127.0.0.1` | Server bind address (interface only, not `host:port`) |
| `PORT` | `3210` | Server listen port |
| `AUTH_MODE` | `local_trust` | Auth mode: `local_trust` or `token` |
| `AUTH_TOKEN` | `(unset)` | Bearer token when `AUTH_MODE=token` |
| `OBSERVER_TOKEN` | `(unset)` | Read-only bearer token for token-auth deployments |
| `SWIMMERS_NATIVE_APP` | `iterm` | Native desktop target: `iterm` or `ghostty` |
| `SWIMMERS_THOUGHT_BACKEND` | `daemon` | Thought subsystem backend: `daemon` or `inproc` |
| `SWIMMERS_REPLAY_BUFFER_SIZE` | `524288` | Replay ring size in bytes (default 512 KB) |
| `SWIMMERS_DATA_DIR` | `(platform data dir)` | Override the persistence directory |
| `SWIMMERS_TUI_URL` | `(unset)` | When set, the TUI uses HTTP transport against this URL instead of hosting the API in-process. Auto-spawns a local server for loopback URLs. |
| `SWIMMERS_VOICE_MODEL` | `(unset)` | Path to a local Whisper `.bin` model used by the experimental `voice` feature. |
| `SWIMMERS_VOICE_LANGUAGE` | `auto` | Optional language hint for the experimental `voice` feature (`en`, `fr`, `auto`, etc.). |

When `SWIMMERS_NATIVE_APP=ghostty`, the API uses Ghostty's AppleScript support to create or replace a left-side preview split for the selected tmux session. This path requires Ghostty 1.3.0+ on macOS with automation access enabled.

While the TUI is running, press `n` or click the top-right native-open label to switch between `iTerm` and `Ghostty` without restarting the API.

The optional browser terminal renderer also honors `SWIMMERS_FRANKENTUI_PKG_DIR` (or `FRANKENTUI_PKG_DIR`) to override the auto-detected `frankentui/pkg` asset path.

---

## Make Targets

If you are working from a source checkout, the Makefile has convenience targets:

```bash
make tui                # Launch swimmers-tui (embedded mode by default)
make web                # Start the standalone server and print local browser URLs
make server             # Run only the standalone API server
make tui-check          # Wait for an existing API (external mode only), then exit
make tui-smoke          # Run shell-level bootstrap tests on the run-tui.sh shim
make cargo-cov-lcov     # Generate lcov coverage report
```

---

## Configuration

Swimmers reads all configuration from environment variables. There is no config file. Defaults are sane for local use:

```bash
# Minimal local usage (everything defaults)
swimmers

# External access with token auth
SWIMMERS_BIND=0.0.0.0 \
AUTH_MODE=token \
AUTH_TOKEN=your-secret-token \
swimmers
```

### Repo Themes

Drop a `.swimmers/colors.json` in any repo directory to override session colors and the repo's default sprite. The TUI discovers themes automatically, and the header `[auto]` sprite mode uses the repo default before falling back to the built-in default.

```json
{
  "sprite": "jelly",
  "palette": {
    "body": "#B89875",
    "outline": "#3D2F24",
    "accent": "#1D1914",
    "shirt": "#AA9370"
  }
}
```

Valid sprite values are `fish`, `balls`, and `jelly`. The header sprite toggle can still force a global override for the current TUI session.

---

## Architecture

### Default: embedded mode

`swimmers-tui` hosts the API in the same process. No port, no handshake, no second binary. All the subsystems below run in one Tokio runtime alongside the render loop.

```
┌──────────────────────────────────────────────────────────────┐
│                         swimmers-tui                         │
│  Aquarium view  |  Thought rail  |  Mermaid viewer           │
│  Keyboard/mouse navigation  |  Native terminal handoff       │
│──────────────────────────────────────────────────────────────│
│  InProcessApi (TuiApi trait) — zero HTTP, zero JSON          │
│──────────────────────────────────────────────────────────────│
│  SessionSupervisor, SessionActor, Thought subsystem,         │
│  FileStore — the same subsystems the standalone server       │
│  runs, just in the same process.                             │
└───────────────────────────┬──────────────────────────────────┘
                            │ PTY / shell exec
                            ▼
┌──────────────────────────────────────────────────────────────┐
│                         tmux server                          │
└──────────────────────────────────────────────────────────────┘
```

### External mode (`SWIMMERS_TUI_URL`)

Set `SWIMMERS_TUI_URL` to split the API into its own process. Multiple TUIs, headless setups, remote access, and direct REST/`curl` use all take this path. For loopback URLs the TUI auto-spawns a sibling `swimmers` binary using a readiness-pipe handshake (no `curl` polling).

```
┌──────────────────────────────────────────────────────────────┐
│                     swimmers-tui (client)                     │
│  Aquarium view  |  Thought rail  |  Mermaid viewer           │
│  Keyboard/mouse navigation  |  Native terminal handoff       │
└───────────────────────────┬──────────────────────────────────┘
                            │ HTTP (REST JSON)
                            │ auto-spawns `swimmers` for loopback
                            │ URLs via readiness-pipe handshake
                            ▼
┌──────────────────────────────────────────────────────────────┐
│            swimmers  (a.k.a. `swimmers serve`)                │
│  Axum router  |  Auth middleware  |  Prometheus /metrics      │
├──────────────────────────────────────────────────────────────┤
│  SessionSupervisor                                           │
│    ├─ tmux discovery loop                                    │
│    ├─ lifecycle broadcasts                                   │
│    └─ persistence checkpoints                                │
│  SessionActor (per session)                                  │
│    ├─ PTY I/O via portable-pty                               │
│    ├─ replay ring buffer                                     │
│    ├─ state detection (idle/busy/error/attention)             │
│    └─ ScrollGuard (redraw burst coalescing)                  │
│  Thought subsystem                                           │
│    ├─ bridge runner (daemon mode)                             │
│    └─ loop runner (in-process mode)                           │
├──────────────────────────────────────────────────────────────┤
│  FileStore (data/swimmers/)  — flat-file persistence         │
└───────────────────────────┬──────────────────────────────────┘
                            │ PTY / shell exec
                            ▼
┌──────────────────────────────────────────────────────────────┐
│                         tmux server                          │
│  Sessions  |  Windows  |  Panes                              │
└──────────────────────────────────────────────────────────────┘
```

### API Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/v1/sessions` | List tmux sessions with state |
| `POST` | `/v1/sessions` | Create a new tmux session |
| `DELETE` | `/v1/sessions/{id}` | Remove a session |
| `GET` | `/v1/sessions/{id}/snapshot` | Capture visible screen text |
| `GET` | `/v1/sessions/{id}/pane-tail` | Recent pane output |
| `POST` | `/v1/sessions/{id}/attention/dismiss` | Clear attention state |
| `POST` | `/v1/sessions/{id}/input` | Send text input to a session |
| `GET` | `/v1/selection` | Read the published selection |
| `POST` | `/v1/selection` | Publish the selected session |
| `GET` | `/v1/native/status` | Native terminal support check |
| `POST` | `/v1/native/open` | Open session in desktop terminal |
| `GET` | `/v1/dirs` | Repo/service directory browser |
| `POST` | `/v1/dirs/restart` | Restart a mapped service |
| `GET` | `/v1/skills/{tool}` | List available skills for a tool |
| `GET` | `/v1/thought-config` | Read thought runtime config |
| `PUT` | `/v1/thought-config` | Update thought runtime config |
| `GET` | `/metrics` | Prometheus metrics |

---

## Running in Background

### nohup

```bash
nohup swimmers > swimmers.log 2>&1 &
```

### systemd (Linux)

```bash
sudo tee /etc/systemd/system/swimmers.service << 'EOF'
[Unit]
Description=Swimmers Terminal Manager
After=network.target

[Service]
Type=simple
User=your-username
Environment=SWIMMERS_BIND=127.0.0.1
Environment=PORT=3210
ExecStart=/home/your-username/.cargo/bin/swimmers
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now swimmers
```

### macOS LaunchAgent

```bash
mkdir -p ~/Library/LaunchAgents

cat > ~/Library/LaunchAgents/com.swimmers.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.swimmers</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Users/your-username/.cargo/bin/swimmers</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/swimmers.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/swimmers.err</string>
</dict>
</plist>
EOF

launchctl load ~/Library/LaunchAgents/com.swimmers.plist
```

---

## How swimmers Compares

| Feature | swimmers | tmux ls | tmuxinator | byobu |
|---------|----------|---------|------------|-------|
| Visual session overview | Session-state-driven animated sprites | Text list | Text list | Status bar |
| State detection (busy/idle/error) | Automatic | Manual | None | Partial |
| AI thought stream | Built-in side panel | None | None | None |
| Remote access | REST API, any network | SSH + tmux attach | Local only | SSH + byobu |
| Native terminal handoff | One keypress from TUI | `tmux attach -t` | Manual | Manual |
| Metrics/observability | Prometheus `/metrics` | None | None | None |
| Setup complexity | `cargo install swimmers` | Already installed | Ruby + config files | apt install |

**When to use swimmers:**
- You run many tmux sessions and want a visual overview
- You use AI coding agents and want to see their thought streams
- You want to monitor remote sessions from a local TUI

**When swimmers is not the right tool:**
- You only use one or two tmux sessions (tmux is fine on its own)
- You need a tmux session template/layout manager (use tmuxinator)

---

## Troubleshooting

### TUI cannot reach the API

This only applies to **external mode** (`SWIMMERS_TUI_URL` set). Embedded mode (the default) does not talk to a network at all.

```bash
# Check if the API is running
curl -s http://127.0.0.1:3210/v1/sessions

# Start it
swimmers          # or: swimmers serve
```

If you want to avoid the external-mode setup entirely, unset `SWIMMERS_TUI_URL` and run `swimmers-tui` directly — it hosts the API in-process.

### TUI gets 401 or 403

The API is running with token auth. Set your credentials:

```bash
AUTH_MODE=token AUTH_TOKEN=your-token swimmers-tui
```

### No sessions showing in the aquarium

Create at least one tmux session:

```bash
tmux new-session -d -s dev
```

### Port already in use

```bash
lsof -ti:3210 | xargs kill
swimmers
```

### Cargo build fails

```bash
rustup update stable
cargo clean
cargo build --release
```

---

## Limitations

- **tmux only** — swimmers does not manage screen, zellij, or plain terminal sessions
- **Browser UI is terminal-first** — the web surface is for remote attach/control; the animated aquarium remains native-only
- **Single-machine sessions** — the API manages tmux sessions on the machine it runs on; it does not aggregate sessions across multiple hosts
- **No session templating** — swimmers discovers existing tmux sessions but does not define layouts or startup commands (use tmuxinator for that)
- **macOS and Linux only** — tmux does not run on Windows, so neither does swimmers

---

## FAQ

### Why "swimmers"?

Sessions are fish. The TUI is an aquarium. Fish swim. Sessions swim between states.

### Does it need Docker?

No. Single binary, flat-file persistence, talks to tmux directly.

### Can I run the API without the TUI?

Yes. Run `swimmers` (or `swimmers serve`) on its own and use the REST endpoints directly, open the browser UI, or point a TUI at it via `SWIMMERS_TUI_URL`.

### What happens when I close the TUI?

Your tmux sessions always keep running — the aquarium observes tmux, it doesn't own it.

In **embedded mode** (default), closing the TUI tears down its in-process API too, since they're the same process. The next `swimmers-tui` launch rediscovers tmux from scratch. In **external mode** (`SWIMMERS_TUI_URL` set), the standalone `swimmers` server keeps running independently; reopen the TUI to reconnect.

### Can multiple TUIs connect to the same API?

Yes, via external mode. Run a standalone `swimmers` server and point each TUI at its URL via `SWIMMERS_TUI_URL`. Embedded mode is single-tenant by design — the API only exists inside that TUI process.

### How does state detection work?

The `SessionActor` monitors each session's PTY output and classifies it into states (idle, busy, error, attention) based on shell activity patterns. Rest states (drowsy, sleeping, deep sleep) layer on top based on inactivity duration.

### What is the thought rail?

A side panel in the TUI that displays AI agent thought streams. When a session runs Claude Code, Codex, or similar tools, their internal reasoning appears in the thought rail next to the aquarium view.

### Is `LocalTrust` auth safe?

On loopback (`127.0.0.1`), yes — only processes on the same machine can reach the port. When you set `SWIMMERS_BIND` to a non-loopback address, the server refuses to start under `AUTH_MODE=local_trust`. Use `AUTH_MODE=token` with a strong `AUTH_TOKEN` for any external exposure.

---

## Vision

See [docs/VISION.md](docs/VISION.md) for the project's mission, competitive positioning, and strategic non-goals.

---

## Design Philosophy

**Sessions are living things.** The aquarium metaphor is not decoration. It encodes session state into spatial position, animation speed, and sprite shape so you can assess a fleet of sessions with a glance instead of reading text.

**The API is the truth.** The TUI is a client. The API discovers tmux sessions, tracks their state, and serves snapshots. You can point multiple TUIs at the same API, run the API headless, or build your own client against the REST endpoints.

**No infrastructure required.** No database, no Docker, no message broker. The server binary talks to tmux directly via `portable-pty`, persists state to flat files under `data/swimmers/`, and serves HTTP on a single port.

**Thoughts are first-class.** The thought subsystem streams AI agent context (from Claude Code, Codex, etc.) into a side panel. Sessions that run AI coding agents surface their internal monologue alongside the terminal output.

---

## About Contributions

Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. Feel free to open issues — bug reports in particular are welcome. PRs are fine as a way to illustrate a proposed fix, but I won't merge them directly; I'll have Claude or Codex review and independently decide whether and how to address them.

---

## License

MIT. See [LICENSE](https://github.com/build000r/swimmers/blob/main/LICENSE).
