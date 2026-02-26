# Throngterm Quickstart

> Give this file to your AI coding assistant to get throngterm running on your machine.

## What This Is

Throngterm is a mobile-first web terminal manager. It gives you a browser UI with animated "Thronglet" creatures that represent your tmux sessions. Tap a creature to open its terminal. It runs on your machine and you access it from your phone/tablet/browser over Tailscale.

**Target setup:** throngterm on port `69420`, accessible over your Tailscale network.

---

## Step 1: Prerequisites

Install these if not already present. Check first before installing.

```bash
# Check what's already installed
rustc --version
cargo --version
which tmux && tmux -V
which tailscale && tailscale version
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

### Tailscale

```bash
# macOS
brew install --cask tailscale
# Then open Tailscale.app and sign in

# Linux
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

Confirm Tailscale is connected:

```bash
tailscale status
```

Note your Tailscale IP (e.g. `100.x.y.z`) — you'll use it to access throngterm from other devices.

---

## Step 2: Clone & Build

```bash
git clone <REPO_URL> throngterm
cd throngterm
cargo build --release
```

Build the frontend:

```bash
cd web
npm install
npm run build
cd ..
```

---

## Step 3: Run on Port 69420

```bash
PORT=69420 ./target/release/throngterm
```

Or during development:

```bash
PORT=69420 cargo run
```

You should see the server start and begin discovering tmux sessions.

It binds to `0.0.0.0` so it's accessible on all interfaces — including your Tailscale IP.

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

### Frontend Dev Server (Optional)

For frontend development with hot reload:

```bash
cd web
npm run dev
```

This runs the Vite dev server with HMR, proxying API requests to the Rust backend.

### Run in Background (Optional)

To keep it running after you close your terminal:

```bash
# Option A: nohup
nohup env PORT=69420 ./target/release/throngterm > throngterm.log 2>&1 &

# Option B: tmux (ironic but practical)
tmux new-session -d -s throngterm 'PORT=69420 /path/to/throngterm'

# Option C: systemd (Linux, persistent across reboots)
# See "Systemd Service" section below
```

---

## Step 4: Access It

From any device on your Tailscale network:

```
http://<YOUR_TAILSCALE_IP>:69420
```

From the machine itself:

```
http://localhost:69420
```

---

## Step 5: Create tmux Sessions

Throngterm manages tmux sessions. You need at least one for anything to show up.

You can create sessions from the web UI (tap the `+` button) or from the terminal:

```bash
tmux new-session -d -s dev
tmux new-session -d -s logs
tmux new-session -d -s scratch
```

Each session appears as an animated Thronglet in the browser. Tap one to open its terminal.

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
Environment=PORT=69420
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
| No Thronglets showing | Create tmux sessions first: `tmux new-session -d -s dev` |
| Can't access from phone | Confirm both devices are on Tailscale. Check `tailscale status` |
| Port already in use | Kill the old process: `lsof -ti:69420 \| xargs kill` |
| Cargo build fails | Ensure Rust toolchain is installed: `rustup update stable` |
| WebSocket connection fails | If behind a reverse proxy, ensure it supports WebSocket upgrades |
| Blank terminal on connect | The session may have exited. Delete and recreate it |
| Frontend not loading | Rebuild: `cd web && npm run build` |

---

## Architecture (For Context)

```
Browser (phone/laptop)
    |
    |-- GET /v1/bootstrap         -> Config + session list
    |-- GET /v1/sessions          -> List tmux sessions
    |-- POST /v1/sessions         -> Create new session
    |-- DELETE /v1/sessions/:id   -> Remove session
    |
    '-- WebSocket /v1/realtime    -> Multiplexed terminal I/O + control events
            |
            |-- Binary: keystrokes -> PTY -> tmux session
            '-- Binary: tmux session -> PTY -> xterm.js render
```

- **Backend**: Rust (axum + tokio + portable-pty)
- **Frontend**: Preact + TypeScript + Vite (built to `dist/`)
- **No database, no Docker**
