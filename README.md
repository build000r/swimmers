# swimmers

<div align="center">

```
   o   .          o O  .        z z
><o)))'>        ><O)))'>          ><-)))'>
  /_/_            /_/_              \_\
      .             O   o
  active            busy           sleeping
```

</div>

A terminal aquarium for your tmux sessions. Each session becomes an animated fish whose behavior reflects its real-time state -- swimming when active, bubbling when busy, dozing when idle. Backed by a Rust API server that discovers and manages tmux sessions, with a native TUI client that renders the whole thing as a fish bowl you can navigate, inspect, and control.

```bash
git clone https://github.com/YOUR_USER/swimmers.git && cd swimmers && make tui
```

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
| **Repo themes** | Per-repo sprite and color overrides via `.swimmers/theme.json` |
| **Remote API** | Point the TUI at a remote server over Tailscale or any network |
| **Prometheus metrics** | `GET /metrics` for monitoring session counts and API health |
| **No database, no Docker** | File-based persistence, single binary, tmux is the only dependency |

---

## Quick Example

```bash
# Build and launch (starts API + TUI in one command)
make tui

# Or run API and TUI separately
make server                    # API on 127.0.0.1:3210
cargo run --bin swimmers-tui   # TUI connects to local API

# Create some tmux sessions for the aquarium
tmux new-session -d -s dev
tmux new-session -d -s logs
tmux new-session -d -s deploy

# Point TUI at a remote API (e.g., over Tailscale)
SWIMMERS_TUI_URL=http://100.101.123.63:3210 cargo run --bin swimmers-tui

# With token auth
AUTH_MODE=token AUTH_TOKEN=secret \
  SWIMMERS_TUI_URL=http://100.101.123.63:3210 \
  cargo run --bin swimmers-tui

# Custom port
PORT=69420 cargo run --bin swimmers
SWIMMERS_TUI_URL=http://127.0.0.1:69420 cargo run --bin swimmers-tui
```

---

## Design Philosophy

**Sessions are living things.** The aquarium metaphor is not decoration. It encodes session state into spatial position, animation speed, and sprite shape so you can assess a fleet of sessions with a glance instead of reading text.

**The API is the truth.** The TUI is a client. The API discovers tmux sessions, tracks their state, and serves snapshots. You can point multiple TUIs at the same API, run the API headless, or build your own client against the REST endpoints.

**No infrastructure required.** No database, no Docker, no message broker. The server binary talks to tmux directly via `portable-pty`, persists state to flat files under `data/swimmers/`, and serves HTTP on a single port.

**Thoughts are first-class.** The thought subsystem streams AI agent context (from Claude Code, Codex, etc.) into a side panel. Sessions that run AI coding agents surface their internal monologue alongside the terminal output.

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
| Setup complexity | `make tui` | Already installed | Ruby + config files | apt install |

**When to use swimmers:**
- You run many tmux sessions and want a visual overview
- You use AI coding agents and want to see their thought streams
- You want to monitor remote sessions from a local TUI

**When swimmers is not the right tool:**
- You only use one or two tmux sessions (tmux is fine on its own)
- You need a tmux session template/layout manager (use tmuxinator)

---

## Installation

### From Source (Recommended)

```bash
git clone https://github.com/YOUR_USER/swimmers.git
cd swimmers
cargo build --release
```

Binaries land in `target/release/swimmers` (API) and `target/release/swimmers-tui` (TUI).

### Prerequisites

| Dependency | Install |
|------------|---------|
| Rust toolchain | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| tmux | `brew install tmux` (macOS) or `apt install tmux` (Debian/Ubuntu) |
| Tailscale (optional) | Only needed for remote API access over a tailnet |

---

## Quick Start

1. **Clone and build**
   ```bash
   git clone https://github.com/YOUR_USER/swimmers.git
   cd swimmers
   cargo build --release
   ```

2. **Launch everything**
   ```bash
   make tui
   ```
   This starts the API on `127.0.0.1:3210` if it is not already running, waits for readiness, then launches the TUI.

3. **Create sessions** (if you don't have any tmux sessions yet)
   ```bash
   tmux new-session -d -s dev
   tmux new-session -d -s logs
   ```

4. **Navigate the aquarium** -- arrow keys to select fish, Enter to open in your terminal, and the thought rail appears on wide terminals.

---

## Commands

### Make Targets

```bash
make tui                # Start local API + TUI (recommended)
make web                # Start the server for browser/tailnet access
make server             # Run only the API server
make tui-check          # Wait for an existing API, then exit
make tui-smoke          # Run shell-level bootstrap tests
make cargo-cov-lcov     # Generate lcov coverage report
```

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `PORT` | `3210` | API listen port |
| `SWIMMERS_TUI_URL` | `http://127.0.0.1:3210` | API URL the TUI connects to |
| `AUTH_MODE` | `local_trust` | Auth mode: `local_trust` or `token` |
| `AUTH_TOKEN` | (none) | Bearer token when `AUTH_MODE=token` |
| `SWIMMERS_FRANKENTUI_PKG_DIR` | auto-detect | Path to `frankentui/pkg` for live browser terminal rendering |
| `SWIMMERS_NATIVE_APP` | `iterm` | Native desktop target: `iterm` or `ghostty` |
| `THOUGHT_BACKEND` | `daemon` | Thought subsystem: `daemon` or `inproc` |
| `THOUGHT_TICK_MS` | `15000` | Thought refresh interval in milliseconds |
| `SESSION_DELETE_MODE` | `detach_bridge` | `detach_bridge` or `kill_tmux` on session delete |
| `REPLAY_BUFFER_SIZE` | `524288` | Replay ring size in bytes (default 512KB) |

When `SWIMMERS_NATIVE_APP=ghostty`, the API uses Ghostty's AppleScript support to create or
replace a left-side preview split for the selected tmux session. This path requires Ghostty 1.3.0+
on macOS with automation access enabled.

While the TUI is running, press `n` or click the top-right native-open label to switch between
`iTerm` and `Ghostty` without restarting the API.

---

## Configuration

Swimmers reads all configuration from environment variables. There is no config file. Defaults are sane for local use:

```bash
# Minimal local usage (everything defaults)
make tui

# Browser/tailnet usage
make web

# Production-style remote API
PORT=3210 \
AUTH_MODE=token \
AUTH_TOKEN=your-secret-token \
THOUGHT_BACKEND=daemon \
cargo run --bin swimmers
```

### Repo Themes

Drop a `.swimmers/theme.json` in any repo directory to override sprite colors for sessions whose `cwd` matches that repo. The TUI discovers themes automatically.

---

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                     swimmers-tui (client)                     │
│  Aquarium view  |  Thought rail  |  Mermaid viewer           │
│  Keyboard/mouse navigation  |  Native terminal handoff       │
└───────────────────────────┬──────────────────────────────────┘
                            │ HTTP (REST JSON)
                            ▼
┌──────────────────────────────────────────────────────────────┐
│                     swimmers (API server)                     │
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

## Troubleshooting

### TUI cannot reach the API

```bash
# Check if the API is running
curl -s http://127.0.0.1:3210/v1/sessions

# Start it manually
make server
```

### TUI gets 401 or 403

The API is running with token auth. Set your credentials:

```bash
AUTH_MODE=token AUTH_TOKEN=your-token cargo run --bin swimmers-tui
```

### No sessions showing in the aquarium

Create at least one tmux session:

```bash
tmux new-session -d -s dev
```

### Port already in use

```bash
lsof -ti:3210 | xargs kill
make server
```

### Cargo build fails

```bash
rustup update stable
cargo clean
cargo build --release
```

---

## Limitations

- **tmux only** -- swimmers does not manage screen, zellij, or plain terminal sessions
- **Browser UI is terminal-first** -- the web surface is for remote attach/control; the animated aquarium remains native-only
- **Single-machine sessions** -- the API manages tmux sessions on the machine it runs on; it does not aggregate sessions across multiple hosts
- **No session templating** -- swimmers discovers existing tmux sessions but does not define layouts or startup commands (use tmuxinator for that)
- **macOS and Linux only** -- tmux does not run on Windows, so neither does swimmers

---

## FAQ

### Why "swimmers"?

Sessions are fish. The TUI is an aquarium. Fish swim. Sessions swim between states.

### Does it need Docker?

No. Single binary, flat-file persistence, talks to tmux directly.

### Can I run the API without the TUI?

Yes. `make server` runs only the API. Use the REST endpoints directly, open the browser UI with `make web`, or point a TUI at it later.

### What happens when I close the TUI?

Your tmux sessions keep running. The API keeps running (if started separately or via `make tui`). Reopen the TUI to reconnect.

### Can multiple TUIs connect to the same API?

Yes. The API is a standard HTTP server. Point multiple TUI instances at the same URL.

### How does state detection work?

The `SessionActor` monitors each session's PTY output and classifies it into states (idle, busy, error, attention) based on shell activity patterns. Rest states (drowsy, sleeping, deep sleep) layer on top based on inactivity duration.

### What is the thought rail?

A side panel in the TUI that displays AI agent thought streams. When a session runs Claude Code, Codex, or similar tools, their internal reasoning appears in the thought rail next to the aquarium view.

---

## Running in Background

### nohup

```bash
nohup env PORT=3210 ./target/release/swimmers > swimmers.log 2>&1 &
```

### systemd (Linux)

```bash
sudo tee /etc/systemd/system/swimmers.service << 'EOF'
[Unit]
Description=Swimmers Terminal Manager
After=network.target

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/path/to/swimmers
Environment=PORT=3210
ExecStart=/path/to/swimmers/target/release/swimmers
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

cat > ~/Library/LaunchAgents/com.swimmers.plist << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.swimmers</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/swimmers/target/release/swimmers</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PORT</key>
        <string>3210</string>
    </dict>
    <key>WorkingDirectory</key>
    <string>/path/to/swimmers</string>
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

## About Contributions

> *About Contributions:* Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. I simply don't have the mental bandwidth to review anything, and it's my name on the thing, so I'm responsible for any problems it causes; thus, the risk-reward is highly asymmetric from my perspective. I'd also have to worry about other "stakeholders," which seems unwise for tools I mostly make for myself for free. Feel free to submit issues, and even PRs if you want to illustrate a proposed fix, but know I won't merge them directly. Instead, I'll have Claude or Codex review submissions via `gh` and independently decide whether and how to address them. Bug reports in particular are welcome. Sorry if this offends, but I want to avoid wasted time and hurt feelings. I understand this isn't in sync with the prevailing open-source ethos that seeks community contributions, but it's the only way I can move at this velocity and keep my sanity.
