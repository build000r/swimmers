# Throngterm

Mobile-first terminal manager for tmux sessions.

## Core Docs

- [QUICKSTART.md](./QUICKSTART.md) for full setup and runtime details.

## iPhone Wrapper (Capacitor)

This repo includes an iOS wrapper at `web/ios` that loads Throngterm from a
remote host URL (Tailscale-friendly).

### First-time setup

```bash
cd web
npm install
```

### Sync wrapper to a host URL

```bash
cd web
THRONGTERM_IOS_SERVER_URL=http://100.101.123.63:3210 npm run ios:sync
npm run ios:open
```

Current local defaults:

- Rust server / API / built web app: `3210`
- Vite dev server with HMR: `5175` when started through `.env-manager`
- Native Rust TUI target: `3210`

For the native TUI on localhost, `make tui` now bootstraps the local Rust API on
`3210` if it is not already running. `make tui-check` remains a pure readiness
probe and will not start the server for you.

No tmux hook setup is required for thought or rest-state updates. `throngterm`
streams session snapshots directly to `clawgs emit --stdio`.

### Fast UI dev mode (HMR)

```bash
# terminal 1
cd /Users/b/repos/throngterm
PORT=3210 cargo run

# terminal 2
cd /Users/b/repos/throngterm/web
npm run dev:host

# terminal 3 (point iPhone app to Vite)
cd /Users/b/repos/throngterm/web
THRONGTERM_IOS_SERVER_URL=http://100.101.123.63:5173 npm run ios:sync
```

If you use `.env-manager`, it starts the Vite dev server on `5175`, not `5173`.
In that path, point the iPhone wrapper to `http://<TAILNET_HOST>:5175`.

### In-app host switching

On iPhone, use the top-left `Host` button to:

- set a new server URL and reload immediately
- persist that override across relaunches
- reset to the config default

The app also includes:

- pull-to-refresh reload
- local fallback error page
- "Open in Safari" action when host is unreachable
