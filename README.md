# Throngterm

Native terminal UI for tmux-backed sessions.

## Core Docs

- [QUICKSTART.md](./QUICKSTART.md) for full setup and runtime details.

## Primary Commands

```bash
make tui
```

Starts the local API on `127.0.0.1:3210` if needed, then launches the native
TUI.

```bash
make server
```

Runs only the Rust API/backend.

```bash
make tui-check
```

Waits for an existing API and exits without launching the TUI.

## Remote API Use

No tmux hook setup is required for thought or rest-state updates. `throngterm`
streams session snapshots directly to `clawgs emit --stdio`.

Point the TUI at a non-local API with `THRONGTERM_TUI_URL`:

```bash
THRONGTERM_TUI_URL=http://100.101.123.63:3210 cargo run --bin throngterm-tui
```

For token-protected APIs:

```bash
AUTH_MODE=token AUTH_TOKEN=your-token \
THRONGTERM_TUI_URL=http://100.101.123.63:3210 \
cargo run --bin throngterm-tui
```

The repo still contains older `web/` assets, but the supported workflow is the
native TUI.
