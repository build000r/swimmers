# Throngterm AI Quickstart

> Give this file to your AI coding assistant to get throngterm running on your machine.

## What This Is

Throngterm is a mobile-first web terminal manager. It gives you a browser UI with animated "Thronglet" creatures that represent your tmux sessions. Tap a creature to open its terminal. It runs on your machine and you access it from your phone/tablet/browser over Tailscale.

**Target setup:** throngterm on port `69420`, accessible over your Tailscale network.

---

## Step 1: Prerequisites

Install these if not already present. Check first before installing.

```bash
# Check what's already installed
which node && node --version
which tmux && tmux -V
which tailscale && tailscale version
```

### Node.js (>= 18)

```bash
# macOS
brew install node

# Ubuntu/Debian
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt-get install -y nodejs
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

## Step 2: Clone & Install

```bash
git clone <REPO_URL> throngterm
cd throngterm
npm install
```

If on macOS ARM (M1/M2/M3/M4), the postinstall script handles `node-pty` spawn-helper permissions automatically.

---

## Step 3: Run on Port 69420

```bash
PORT=69420 npm start
```

You should see:

```
Throngterm running on http://0.0.0.0:69420
```

It binds to `0.0.0.0` so it's accessible on all interfaces — including your Tailscale IP.

### Run in Background (Optional)

To keep it running after you close your terminal:

```bash
# Option A: nohup
nohup env PORT=69420 node server/index.js > throngterm.log 2>&1 &

# Option B: tmux (ironic but practical)
tmux new-session -d -s throngterm 'PORT=69420 node server/index.js'

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

**Do NOT use `localhost:3000`** — the port is `69420`.

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
ExecStart=/usr/bin/node server/index.js
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
        <string>$(which node)</string>
        <string>$(pwd)/server/index.js</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PORT</key>
        <string>69420</string>
    </dict>
    <key>WorkingDirectory</key>
    <string>$(pwd)</string>
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

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| No Thronglets showing | Create tmux sessions first: `tmux new-session -d -s dev` |
| Can't access from phone | Confirm both devices are on Tailscale. Check `tailscale status` |
| Port already in use | Kill the old process: `lsof -ti:69420 \| xargs kill` |
| node-pty build error | Make sure you have Xcode CLI tools (`xcode-select --install` on macOS) or `build-essential` on Linux |
| WebSocket connection fails | If behind a reverse proxy, ensure it supports WebSocket upgrades |
| Blank terminal on connect | The session may have exited. Delete and recreate it |

---

## Architecture (For Context)

```
Browser (phone/laptop)
    │
    ├── HTTP GET /api/sessions     → Lists tmux sessions
    ├── HTTP POST /api/sessions    → Creates new session
    ├── HTTP DELETE /api/sessions/x → Kills session
    │
    └── WebSocket /ws/:sessionId   → Bidirectional terminal I/O
            │
            ├── Input:  keystrokes → node-pty → tmux session
            └── Output: tmux session → node-pty → xterm.js render
```

- **3 npm dependencies**: express, ws, node-pty
- **No database, no Docker, no build step**
- Client is vanilla JS with xterm.js
