# Throngterm Quickstart

> Give this file to your AI coding assistant to get throngterm running on your machine.

## What This Is

Throngterm is a native terminal UI backed by a Rust API that manages tmux
sessions. The supported path is `API + TUI`.

**Target setup:** local API on `3210` with the native TUI attached to it.

---

## Step 1: Prerequisites

Install these if not already present. Check first before installing:

```bash
rustc --version
cargo --version
which tmux && tmux -V
```

### Rust toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

### tmux

```bash
# macOS
brew install tmux

# Ubuntu/Debian
sudo apt-get install -y tmux
```

### Tailscale (Optional)

Only needed if your TUI will talk to a remote API over your tailnet.

---

## Step 2: Clone & Build

```bash
git clone <REPO_URL> throngterm
cd throngterm
cargo build --release
```

---

## Step 3: Run Locally

Recommended:

```bash
make tui
```

That will:

- start the local API on `127.0.0.1:3210` if it is not already running
- wait for readiness
- launch the native TUI

If you want separate processes:

```bash
make server
cargo run --bin throngterm-tui
```

Useful variants:

- `make tui-check`: wait for an existing API and exit
- `PORT=69420 cargo run --bin throngterm`: run the API on a custom port
- `THRONGTERM_TUI_URL=http://127.0.0.1:69420 cargo run --bin throngterm-tui`: point the TUI at that custom API

You should see the API start and begin discovering tmux sessions.

No tmux hook setup is required for thought or rest-state updates. `throngterm`
streams session snapshots directly to `clawgs emit --stdio`.

The API binds to `0.0.0.0`, so you can also point a TUI at it from another
machine if you expose the port intentionally.

### Structured Transcript Snapshot (Optional)

If you also have the sibling `skills` repo with `clawgs`, you can extract a
normalized Claude/Codex JSON snapshot from the same machine:

```bash
bash scripts/clawgs-extract.sh
```

Override the binary path when needed:

```bash
CLAWGS_BIN=/custom/path/clawgs bash scripts/clawgs-extract.sh
```

Pass a specific cwd (used for JSONL discovery) plus extra extractor flags:

```bash
bash scripts/clawgs-extract.sh /path/to/project --pretty --include-raw
```

## Step 4: Connect to a Remote API (Optional)

If the API runs on another machine:

```bash
THRONGTERM_TUI_URL=http://100.x.y.z:3210 cargo run --bin throngterm-tui
```

For token-protected APIs:

```bash
AUTH_MODE=token AUTH_TOKEN=your-token \
THRONGTERM_TUI_URL=http://100.x.y.z:3210 \
cargo run --bin throngterm-tui
```

### Run in Background (Optional)

To keep only the API running after you close your terminal:

```bash
# Option A: nohup
nohup env PORT=3210 ./target/release/throngterm > throngterm.log 2>&1 &

# Option B: tmux (ironic but practical)
tmux new-session -d -s throngterm 'PORT=3210 /path/to/throngterm/target/release/throngterm'

# Option C: systemd (Linux, persistent across reboots)
# See "Systemd Service" section below
```

---

## Step 5: Create tmux Sessions

Throngterm manages tmux sessions. You need at least one for anything to show
up. Create them either from the TUI or directly with tmux:

```bash
tmux new-session -d -s dev
tmux new-session -d -s logs
tmux new-session -d -s scratch
```

They will appear in the TUI session list.

---

## Optional: Systemd Service (Linux)

For a persistent setup that survives reboots:

```bash
sudo tee /etc/systemd/system/throngterm.service << 'EOF'
[Unit]
Description=Throngterm Terminal Manager
After=network.target

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/path/to/throngterm
Environment=PORT=3210
ExecStart=/path/to/throngterm/target/release/throngterm
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now throngterm
```

Replace `YOUR_USERNAME` and `/path/to/throngterm` with actual values.

---

## Optional: macOS LaunchAgent

For persistence on macOS:

```bash
mkdir -p ~/Library/LaunchAgents

cat > ~/Library/LaunchAgents/com.throngterm.plist << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.throngterm</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/throngterm/target/release/throngterm</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PORT</key>
        <string>69420</string>
    </dict>
    <key>WorkingDirectory</key>
    <string>/path/to/throngterm</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/throngterm.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/throngterm.err</string>
</dict>
</plist>
EOF

launchctl load ~/Library/LaunchAgents/com.throngterm.plist
```

Replace `/path/to/throngterm` with the actual path.

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| TUI cannot reach the API | Run `make tui` or start the API with `make server` |
| TUI gets `401` or `403` | Set `AUTH_MODE=token` and `AUTH_TOKEN` to match the API |
| No sessions showing | Create tmux sessions first: `tmux new-session -d -s dev` |
| Port already in use | Kill the old process: `lsof -ti:3210 \| xargs kill` |
| Cargo build fails | Ensure Rust toolchain is installed: `rustup update stable` |
| Blank terminal on connect | The session may have exited. Recreate it or restart the shell |

---

## Architecture (For Context)

```
Native TUI
    |
    |-- GET /v1/bootstrap         -> Config + session list
    |-- GET /v1/sessions          -> List tmux sessions
    |-- POST /v1/sessions         -> Create new session
    |-- DELETE /v1/sessions/:id   -> Remove session
    |
    '-- WebSocket /v1/realtime    -> Multiplexed terminal I/O + control events
            |
            |-- Binary: keystrokes -> PTY -> tmux session
            '-- Binary: tmux session -> PTY -> TUI renderer
```

- **Backend**: Rust (axum + tokio + portable-pty)
- **Client**: Rust TUI (`throngterm-tui`)
- **No database, no Docker**
