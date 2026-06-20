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
| **Web Trogdor cockpit** | Browser single-session terminal with Activity, Diffs, Logs, Artifacts, and Skills panels backed by real session APIs |
| **Native terminal handoff** | Open any session directly in iTerm or Ghostty from the TUI |
| **Mermaid diagrams** | Render and zoom Mermaid artifacts inline in the terminal |
| **Repo themes** | Per-repo colors plus default sprite overrides via `.swimmers/colors.json` |
| **Environment cockpit + launch targets** | Point the TUI at one remote server, or route selected directory launches to overlay-declared local, remote `swimmers` API, and SSH-only handoff targets |
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
| clawgs (thought rail) | Install/build the `clawgs` binary or set `CLAWGS_BIN=/path/to/clawgs`; verify with `swimmers config doctor` |
| CMake (voice feature only) | `brew install cmake` (macOS) or `apt install cmake` (Debian/Ubuntu) |
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

The `voice` feature builds `whisper-rs`, so you need `cmake` available on your `PATH` during the build and a local Whisper `.bin` model at runtime.

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

To start the same prompt in multiple directories from the TUI, click empty aquarium space to open the directory picker, optionally type to filter the visible rows or jump to any indexed `.git` repo under `~/repos` and `~/hard`, use `[exclude]` to mark any rows `[out]`, choose `[batch N]`, type the initial request once, and press Enter. The clawgs rail stays hidden until an agent is actually asleep/waiting for input, then shows those agents with a `[launch]` shortcut for starting the next prompt in the same repo; press `Tab` to toggle batch grouping and `>` to show all agents while the rail is open. If the rail reports `clawgs` as unavailable, run `swimmers config doctor` and set `CLAWGS_BIN=/path/to/clawgs` when the binary is not on `PATH`.

### Experimental voice input

In a source build with `--features voice`, open the initial-request composer and press `Ctrl-V` to start or stop microphone capture. Swimmers records locally, transcribes with a local Whisper model, and inserts the transcript into the composer so you can edit it before creating the hidden swimmer.

Set `SWIMMERS_VOICE_MODEL=/path/to/whisper.bin` before launching the TUI, and on macOS make sure your terminal app has Microphone permission in System Settings.

### External server mode

Set `SWIMMERS_TUI_URL` to run the API as a separate process (for multi-client access, remote access over Tailscale, or integration with the REST endpoints from `curl`/browser). The TUI switches to HTTP transport and, for loopback URLs, auto-spawns a sibling `swimmers` binary if one isn't already listening — using a readiness pipe instead of polling, so the handoff is invisible.

```bash
SWIMMERS_TUI_URL=http://127.0.0.1:3210 swimmers-tui
```

From a source checkout, use `make up` when you want the browser surface and the TUI attached to the same local backend. It builds the current checkout, requires resolvable FrankenTerm assets, replaces any stale local `swimmers` listener on `PORT`, prints the browser URLs, sets `SWIMMERS_PERSONAL_WORKFLOWS=1`, then launches the TUI with `SWIMMERS_TUI_URL` and `SWIMMERS_TUI_REUSE_SERVER=1` so it does not clear that backend. That runtime switch exposes click-to-spawn endpoints such as `/v1/dirs` and `/v1/sessions/{id}/skills`; set it to `0` to hide those local workflow routes without rebuilding. Set `SWIMMERS_UP_FEATURES` only for actual Cargo features such as `voice`. To avoid feature-set collisions in Cargo's shared `target/debug` binary path, `make up` builds into `target/swimmers-up/<features>/` by default, skips the build only when that feature-specific binary is newer than the checkout inputs, and treats a listener as reusable only when it is the same executable, `/health` says the process is alive, cheap required routes respond, and personal workflow routes are not explicitly missing. A cold `/v1/dirs` inventory can exceed the short startup probe; that is reported as a note rather than mistaken for a dead backend.

The browser Trogdor view is terminal-first. Selecting one agent opens a single-session cockpit: the live terminal remains the primary surface, the bottom composer sends input without covering model output, and the workbench panels read the session timeline, user-submitted turns, post-turn JSONL records, structured git diff, Mermaid/plan artifacts, and passive Skillbox/SBP Skills results. The Skills panel identifies relevant skills when `SWIMMERS_PERSONAL_WORKFLOWS=1` and `sbp` is available; it does not perform automatic skill hot-swap or mutate overlays.

The `make tui` wrapper clears a stale loopback `swimmers` process on the target API port before launching, so local overlay edits are reread instead of silently reusing an old server. Set `SWIMMERS_TUI_REUSE_SERVER=1` to keep an existing local backend.

To run the API explicitly as a standalone headless server:

```bash
swimmers          # same as `swimmers serve`
# or
swimmers serve    # explicit form for service managers and docs
```

`Ctrl-C` stops it. `kill $(lsof -ti:3210)` works for a backgrounded instance.

### Directory launch targets and environment cockpit

When a `skillbox-config` client overlay declares `dev_sanity.agent_launch`,
the directory picker shows a launch-target toggle next to the tool selector.
`local` keeps the existing behavior. A `kind: swimmers_api` target maps the
selected local cwd through its `path_mappings`, POSTs the create request to
the target API, and namespaces remote sessions as `target::session_id` so local
and remote `sess_0` values cannot collide. Set `auth_token_env` only for target
APIs running in `AUTH_MODE=token`; Tailnet-trusted targets do not need a
browser or launch token.

Remote targets are different from `SWIMMERS_TUI_URL`: `SWIMMERS_TUI_URL`
points the entire TUI at one backend, while launch targets let one local TUI
spawn selected directory/list runs onto another configured machine and keep
those sessions visible in the local aquarium.

V2 environment cockpit setups use three target kinds:

- `local` is the implicit loopback/in-process target.
- `swimmers_api` is a trusted remote Swimmers backend, usually reached over a
  Tailnet or token-auth URL. It can observe, launch, and send input only through
  that configured API.
- `ssh_only` is inventory and handoff only. It can show attach/bootstrap hints
  such as `ssh skillbox-devbox` or `ssh skillbox-devbox 'swimmers serve'`, but
  it is not treated as a live session source until that host has a trusted
  `swimmers_api` target.

To inventory existing SSH aliases before editing an overlay, run:

```bash
swimmers config ssh-import --dry-run --ssh-config ~/.ssh/config
```

The importer is proposal-only: it reads SSH config files, follows `Include`
patterns, emits JSON `ssh_only` target snippets, and does not connect to hosts
or write config.

Example overlay shape:

```yaml
dev_sanity:
  agent_launch:
    default_target: devbox
    group_defaults:
      swimmers: devbox
    targets:
      - id: devbox
        label: Devbox API
        kind: swimmers_api
        base_url: http://100.101.123.63:3210
        auth_token_env: SWIMMERS_DEVBOX_TOKEN
        path_mappings:
          - local_prefix: /Users/me/repos/opensource
            remote_prefix: /srv/devbox/repos/opensource
      - id: skillbox-devbox
        label: Skillbox SSH
        kind: ssh_only
        bootstrap_hint: "ssh skillbox-devbox 'swimmers serve'"

  fleet_lenses:
    - id: swimmers-on-devbox
      label: Swimmers on devbox
      matchers:
        - type: target_kind
          kind: swimmers_api
        - type: repo
          key: /Users/me/repos/opensource/swimmers
```

Built-in saved fleet lenses include `all`, `local`, `remote-api`,
`ssh-handoff`, `current-repo`, `needs-attention`, and `degraded`. Overlay
lenses can combine target id/kind, repo key, readiness, transport,
capability, degraded, and attention matchers. Lens definitions are labels and
matchers only; do not persist tokens, raw terminal output, or command history
in them.

For the multi-environment cockpit proof lane, see
[`docs/MULTI_ENV_COCKPIT_PROOF.md`](docs/MULTI_ENV_COCKPIT_PROOF.md) and run
`make multi-env-smoke`. That lane covers configured local plus
overlay-declared `swimmers_api` targets, path mappings, remote write proxy
guardrails, display grouping, fleet filters, attention inbox behavior, and
advisory badges. It does not claim arbitrary SSH fleet control or make
FrankenTerm the orchestration source of truth.

For the v2 local plus remote API plus SSH-only handoff proof lane, see
[`docs/MULTI_SSH_ENV_V2_PROOF.md`](docs/MULTI_SSH_ENV_V2_PROOF.md) and run
`make multi-ssh-env-smoke`. That smoke is fixture-backed and does not require
live SSH by default.

| Failure mode | Swimmers behavior |
|--------------|-------------------|
| Wrong-host launch risk | launch previews and receipts name `target_id`, `target_kind`, requested cwd, resolved cwd, and the selected path mapping |
| Unmapped cwd | remote launch is blocked with path-mapping guidance |
| Down remote API | cached sessions are marked degraded/stale and demoted below fresh local attention work |
| Missing token env | health/auth guidance is shown without printing token values |
| Stale c0 or NTM advisory | badge remains `external` and stale; it never becomes trusted orchestration state |
| SSH-only target | attach/bootstrap hints remain copyable, but no live session aggregation, send, or launch is implied |

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

Set `SWIMMERS_BIND` to expose the server on a non-loopback interface. The server refuses to start if `AUTH_MODE=local_trust` is paired with a non-loopback bind. For Tailscale-only exposure, bind to the machine's Tailscale IP and use `AUTH_MODE=tailnet_trust`; for other network exposure, use `AUTH_MODE=token` with `AUTH_TOKEN`. Unknown `AUTH_MODE` values and `AUTH_MODE=token` without `AUTH_TOKEN` are startup configuration errors.

```bash
# Bind to a specific Tailscale IP, trusting Tailscale for access control
SWIMMERS_BIND=100.101.123.63 \
AUTH_MODE=tailnet_trust \
swimmers

# Point the TUI at the remote server
SWIMMERS_TUI_URL=http://100.101.123.63:3210 \
swimmers-tui

# Non-tailnet exposure still requires a bearer token
SWIMMERS_BIND=0.0.0.0 \
AUTH_MODE=token \
AUTH_TOKEN=your-secret-token \
swimmers
```

From a source checkout on a Tailscale host, `make tailnet` wraps the Tailnet setup,
auto-detects the machine's Tailscale IPv4 address, enables personal workflow
endpoints, and prints the browser URL plus the local `SWIMMERS_TUI_URL=...`
command to run from another machine.

`AUTH_MODE=tailnet_trust` is accepted only for Tailscale IP ranges (`100.64.0.0/10` or `fd7a:115c:a1e0::/48`). It does not add a second browser token; Tailnet membership is the access boundary. `OBSERVER_TOKEN` is optional only for token-auth deployments where you also want a read-only credential for browser or observer clients.

In `AUTH_MODE=token`, browser HTTP requests use an `Authorization: Bearer ...` header, and browser terminal WebSockets authenticate with a first WebSocket message instead of a `?token=` URL parameter. This keeps long-lived operator and observer tokens out of WebSocket URLs, browser history, and proxy request lines; it does not encrypt the connection. Use HTTPS, SSH forwarding, or a Tailnet-only bind for non-loopback token deployments.

---

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `SWIMMERS_BIND` | `127.0.0.1` | Server bind interface. A `host:port` form is accepted, but the port is ignored — use `PORT` to set the listen port. |
| `PORT` | `3210` | Server listen port |
| `AUTH_MODE` | `local_trust` | Auth mode: `local_trust`, `tailnet_trust`, or `token` |
| `AUTH_TOKEN` | `(unset)` | Bearer token when `AUTH_MODE=token` |
| `OBSERVER_TOKEN` | `(unset)` | Read-only bearer token for token-auth deployments |
| `SWIMMERS_GROK_BIN` | `grok` | Override the Grok executable used by local spawn and commit-helper launchers |
| `SWIMMERS_PERSONAL_WORKFLOWS` | `0` | Set to `1` to expose local repo browsing, skill discovery, group editing, and commit-helper routes. Source builds made with `--features personal-workflows` default this to `1`, but the env var can still disable it. |
| `SWIMMERS_NATIVE_APP` | `iterm` | Native desktop target: `iterm` or `ghostty` |
| `SWIMMERS_GHOSTTY_MODE` | `swap` | Ghostty single-session placement: `swap`, `add`, or `window` |
| `SWIMMERS_NATIVE_SCRIPT_ROOT` | `(bundled)` | Override the root containing `scripts/iterm-focus.scpt` and `scripts/ghostty-open.scpt` for native handoff |
| `SWIMMERS_ATTENTION_GROUP_SIZE` | `6` | Number of panes to place in the managed attention group; values are clamped to `1`-`6`. |
| `SWIMMERS_ATTENTION_GROUP_LAYOUT` | `tiled` | tmux pane layout for the managed attention group: `tiled`, `even-horizontal`, `even-vertical`, `main-horizontal`, or `main-vertical`. Aliases such as `columns`, `stacked`, and `main-left` are accepted. |
| `SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED` | `(unset)` | Set to `1` to let managed attention groups include ready tmux sessions whose names are not only digits. |
| `SWIMMERS_THOUGHT_BACKEND` | `daemon` | Thought subsystem backend: `daemon` or `inproc` |
| `CLAWGS_BIN` | `(auto)` | Override path to the `clawgs` binary used by the thought rail |
| `SWIMMERS_THOUGHT_TICK_MS` | `15000` | Thought polling tick in milliseconds. Values below `250` or invalid values fall back to the default; values above `300000` are clamped. |
| `SWIMMERS_OUTBOUND_QUEUE_BOUND` | `4096` | WebSocket outbound queue bound. Values below `64` or invalid values fall back to the default; values above `65536` are clamped. |
| `SWIMMERS_REPLAY_BUFFER_SIZE` | `524288` | Replay ring size in bytes (default 512 KB). Values below `4096` or invalid values fall back to the default; values above `16777216` are clamped. |
| `SWIMMERS_DATA_DIR` | `(platform data dir)` | Override the persistence directory |
| `SWIMMERS_TUI_URL` | `(unset)` | When set, the TUI uses HTTP transport against this URL instead of hosting the API in-process. Auto-spawns a local server for loopback URLs. |
| `SWIMMERS_TUI_REUSE_SERVER` | `(unset)` | Set to `1` for `make tui` to keep an existing loopback `swimmers` backend instead of restarting it first. |
| `SWIMMERS_REPO_SEARCH_ROOTS` | `~/repos:~/hard` | Path-list roots for the spawn picker’s cached `.git` repo search. |
| `SWIMMERS_REPO_SEARCH_MAX_DEPTH` | `8` | Maximum directory depth scanned below each repo search root. |
| `SWIMMERS_VOICE_MODEL` | `(unset)` | Path to a local Whisper `.bin` model used by the experimental `voice` feature. |
| `SWIMMERS_VOICE_LANGUAGE` | `auto` | Optional language hint for the experimental `voice` feature (`en`, `fr`, `auto`, etc.). |

When `SWIMMERS_NATIVE_APP=ghostty`, the API uses Ghostty's AppleScript support to place selected tmux sessions according to `SWIMMERS_GHOSTTY_MODE`; managed attention groups always use their own Ghostty window so they remain visible without clicking a hidden split. This path requires Ghostty 1.3.0+ on macOS with automation access enabled.

Native handoff scripts are bundled into installed binaries and materialized under the Swimmers data directory when a source checkout is not present. Set `SWIMMERS_NATIVE_SCRIPT_ROOT` to point at an alternate checkout or patched script root.

For Ghostty windows or splits you open outside the TUI, run `swimmers tmux new` as the terminal command to create and attach a new tmux session using the next Swimmers numeric name. Use `swimmers tmux next-name` to preview the next number. Unnumbered sessions such as `swimmers-attention`, wave sessions, and backend support sessions do not advance that counter.

While the TUI is running, press `n` or click the top-right native-open label to switch between `iTerm` and `Ghostty` without restarting the API.

`PORT` and resource-size environment variables are parsed by the same startup path used by `swimmers config` and `swimmers config doctor`. Invalid numeric values print a config warning and use the local-first default; oversized replay, outbound queue, and thought tick values print a warning and clamp to the documented maximum.

When a selected session is stale because its tmux session disappeared, press `A` after recreating that exact tmux session to reattach the stale swimmers identity instead of creating a duplicate.

Click `[attention group]` in the TUI header to build or refresh a managed `swimmers-attention` tmux session from local numbered tmux sessions that are ready for operator input. Set `SWIMMERS_ATTENTION_GROUP_SIZE` to choose how many panes you want in the group and `SWIMMERS_ATTENTION_GROUP_LAYOUT` to control their tmux placement. Set `SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED=1` to also include ready sessions that use other naming conventions. When Ghostty is the native target on macOS, that explicit action opens the attention group in its own managed Ghostty window. On Linux or other non-native hosts, the same click refreshes the tmux group and reports `tmux attach -t swimmers-attention` for manual attach. The group prefers related project work over raw recency, preserves the managed tmux session, and refreshes the configured pane count in place as visible panes stop waiting or new ready panes can fit.

The optional browser terminal renderer reads FrankenTerm assets from `SWIMMERS_FRANKENTUI_PKG_DIR` (or `FRANKENTUI_PKG_DIR`). The source-checkout wrappers can discover and export that path for local browser workflows; standalone `swimmers` runs need one of those env vars when the package assets are outside the bundled candidate paths.

---

## Make Targets

If you are working from a source checkout, the Makefile has convenience targets:

```bash
make up                 # Start the current-checkout backend, print browser URLs, and launch the TUI against it
make tailnet            # Start the backend on this machine's Tailscale IP for remote browser/TUI clients
make tui                # Launch swimmers-tui (embedded mode by default)
make web                # Start the standalone server and print local browser URLs
make server             # Run only the standalone API server
make tui-check          # Type-check the native TUI binary
make up-smoke           # Run shell-level checks on the combined web+TUI launcher
make tui-smoke          # Run shell-level bootstrap tests on the run-tui.sh shim
make glance-smoke       # Render the 10-session Glance fixture and write proof artifacts
make remote-rust-validate-dry-run
                        # Print the remote Rust validation plan without SSH
make remote-rust-validate
                        # Run cargo validation on SWIMMERS_REMOTE_RUST_HOST
make release-acceptance # Run the default installed-binary release smoke
make release-acceptance-all
                        # Run default, source, native asset, and thought profiles
make cargo-cov-lcov     # Generate lcov coverage report
```

`make web` runs `scripts/run-web.sh`. The wrapper probes `/app.js` when it finds
an existing listener on the target port, prints both the root browser URL (`/`)
and the focused selected-session URL (`/selected`), and defaults
`SWIMMERS_PERSONAL_WORKFLOWS=1` for source-checkout browser workflows. Set that
environment variable to `0` to hide local repo browsing, skills, group editing,
and commit-helper routes.

When local disk pressure or Cargo cache churn makes Rust validation impractical,
agents and maintainers can offload source-checkout validation to an
operator-provided SSH host:

```bash
make remote-rust-validate-dry-run
SWIMMERS_REMOTE_RUST_HOST=builder.example make remote-rust-validate
SWIMMERS_REMOTE_RUST_HOST=builder.example \
  scripts/remote-rust-validate.sh -- cargo test group_membership -- --test-threads=1
```

The helper copies only tracked working-tree files into an isolated remote temp
checkout, so untracked scratch files are not sent. Add new source files to git
before treating remote validation as proof. It runs the command inside a
disposable Rust contributor validation container with an isolated
`CARGO_TARGET_DIR`, and removes remote temp directories by default. This is only
a contributor/operator validation lane; Swimmers itself remains a single-binary
tmux tool with no Docker runtime dependency.

### Browser asset routes

The Rust web server serves the Trogdor shell at `/` and the focused browser
shell at `/selected`. Browser assets keep `/app.js` as a compatibility ES module
route. When Vite build output is available, the HTML shell points at
`/assets/vite/...` files from `target/web-vite` by default, or from
`SWIMMERS_VITE_DIST_DIR` when that override is set. In debug/source workflows,
set `SWIMMERS_VITE_DEV_ORIGIN` to point module script tags at a running Vite
dev server while the Rust backend continues serving compatibility CSS from
`/app.css`.

### Release acceptance profiles

Release proof is split into profiles so optional local features do not block the default installed-binary contract:

| Profile | Command | What it proves |
|---------|---------|----------------|
| Default installed binaries | `make release-acceptance` | Installs from the checkout unless `SWIMMERS_ACCEPTANCE_BIN_DIR` is set, checks `swimmers --help`, `swimmers-tui --help`, `swimmers-tui --version`, starts a loopback `swimmers` server with an isolated data dir, and verifies `/health` plus `/v1/sessions`. |
| Source checkout personal workflows | `make release-acceptance-source` | Runs the source `make up` launcher smoke with `SWIMMERS_PERSONAL_WORKFLOWS=1` separate from crates.io/default install proof. |
| Native packaged assets | `make release-acceptance-native` | Verifies the native handoff scripts are present in the Cargo package. |
| Thought bridge | `make release-acceptance-thought` | Runs thought bridge and fake-emitter contract tests without requiring `clawgs` on the operator PATH. |
| Voice feature | `make release-acceptance-voice` | Compiles the optional `voice` TUI path; requires CMake and the voice feature build dependencies. Runtime voice use still requires `SWIMMERS_VOICE_MODEL`. |

Use `SWIMMERS_ACCEPTANCE_ARTIFACT_DIR=/path/to/evidence` to keep profile logs and HTTP payloads with a release run. The GitHub release workflow runs the native asset and default release-binary profiles for published artifacts; source, thought, and voice profiles remain explicit local/CI choices.

---

## Configuration

Swimmers reads all configuration from environment variables. There is no config file. Defaults are sane for local use:

```bash
# Minimal local usage (everything defaults)
swimmers

# Tailscale-only remote access without a browser token
SWIMMERS_BIND=100.101.123.63 \
AUTH_MODE=tailnet_trust \
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
│  FileStore (SWIMMERS_DATA_DIR / platform data dir)           │
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
| `GET` | `/v1/environments` | List local, remote API, and SSH-only environment metadata plus fleet presets |
| `GET` | `/v1/sessions` | List tmux sessions with state |
| `POST` | `/v1/sessions` | Create a new tmux session |
| `POST` | `/v1/sessions/adopt` | Adopt an externally-created tmux session; optional `session_id` reattaches a stale swimmers identity |
| `POST` | `/v1/sessions/reattach` | Alias for explicit stale-session reattachment by exact tmux name |
| `POST` | `/v1/sessions/batch` | Create one session per directory with the same initial request |
| `POST` | `/v1/sessions/group-input` | Send the same text to ready sessions in one batch |
| `DELETE` | `/v1/sessions/{id}` | Remove a session |
| `GET` | `/v1/sessions/{id}/snapshot` | Capture visible screen text |
| `GET` | `/v1/sessions/{id}/timeline` | Ordered single-session timeline with pinned task, diff, log, and artifact summaries |
| `GET` | `/v1/sessions/{id}/agent-context` | User-submitted turn metadata and current task/action context for the cockpit workbench |
| `GET` | `/v1/sessions/{id}/transcript` | User turns and post-turn JSONL records for the latest or selected turn |
| `GET` | `/v1/sessions/{id}/pane-tail` | Recent pane output |
| `GET` | `/v1/sessions/{id}/git-diff` | Raw and structured git diff summaries |
| `GET` | `/v1/sessions/{id}/mermaid-artifact` | Mermaid/plan artifact metadata and source |
| `GET` | `/v1/sessions/{id}/plan-file` | Read a plan or repo-doc artifact file |
| `GET` | `/v1/sessions/{id}/skills?source=sbp` | Passive session-scoped Skillbox/SBP Skills discovery when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `POST` | `/v1/sessions/{id}/commit-grok` | Launch the UI commit-helper flow with Grok when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `POST` | `/v1/sessions/{id}/commit-codex` | Launch the UI commit-helper flow with Codex when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `POST` | `/v1/sessions/{id}/attention/dismiss` | Clear attention state |
| `POST` | `/v1/sessions/{id}/input` | Send text input to a session |
| `GET` | `/v1/selection` | Read the published selection |
| `PUT` | `/v1/selection` | Publish the selected session |
| `GET` | `/v1/operator-pressure` | Summarize action-readiness pressure across sessions |
| `GET` | `/v1/native/status` | Native terminal support check |
| `PUT` | `/v1/native/app` | Select the native terminal app |
| `PUT` | `/v1/native/mode` | Select native terminal open behavior |
| `POST` | `/v1/native/open` | Open session in desktop terminal |
| `POST` | `/v1/native/attention-group/open` | Open or refresh the managed native attention group |
| `GET` | `/v1/dirs` | Repo/service directory browser when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `GET` | `/v1/dirs/repositories` | Cached local repository search results when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `POST` | `/v1/dirs/restart` | Restart a mapped service when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `POST` | `/v1/dirs/actions` | Start a mapped repo action such as commit assistance when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `POST` | `/v1/dirs/group-memberships` | Add, remove, or move a project in directory groups when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `GET` | `/v1/skills?tool=...` | List available skills for a tool when `SWIMMERS_PERSONAL_WORKFLOWS=1` |
| `GET` | `/v1/thought-config` | Read thought runtime config |
| `PUT` | `/v1/thought-config` | Update thought runtime config |
| `GET` | `/v1/thought/sync-preview` | Preview thought synchronization without mutating state |
| `GET` | `/health` | Health, thought bridge, and persistence status |
| `GET` | `/readyz` | Startup readiness status |
| `GET` | `/version` | Binary version metadata |
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
- You want to monitor remote sessions from a local TUI through a trusted
  Swimmers API, or keep SSH-only hosts visible as explicit handoff targets

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

The API is running with token auth. Set your credentials, or switch the remote
server to `AUTH_MODE=tailnet_trust` when it is bound to a Tailscale IP:

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
- **Browser UI is terminal-first** — the web surface keeps the live terminal primary; Trogdor presentation and cockpit panels summarize existing backend facts rather than inventing new state
- **Single-machine sessions by default** — each API manages tmux sessions on the machine it runs on. One local cockpit may aggregate explicitly configured `swimmers_api` launch targets and show `ssh_only` handoff targets, but Swimmers does not discover or control arbitrary SSH fleets
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

The `SessionActor` monitors each session's PTY output and classifies it into states (idle, busy, error, attention) based on shell activity patterns. Rest states layer on top: without transcript/thought-daemon input, elapsed idle time only ages a session into `drowsy`; `sleeping` and `deep sleep` are reserved for transcript-aware waiting/exited state.

### What is the thought rail?

A side panel in the TUI that answers one question first: which agents are waiting for input? It stays hidden until at least one agent reaches the transcript-aware sleeping state, then shows those rows with an asleep/total count and a `[launch]` shortcut that opens the normal request composer for that repo using the current tool and launch target. Press `>` to show all agents while the rail is open, or `Tab` to pivot between `pwd` and batch grouping. The rail is powered by `clawgs emit --stdio`; when `clawgs defaults` is unavailable, the TUI shows a setup hint and `swimmers config doctor` prints the fix.

### Is `LocalTrust` auth safe?

On loopback (`127.0.0.1`), yes — only processes on the same machine can reach the port. When you set `SWIMMERS_BIND` to a non-loopback address, the server refuses to start under `AUTH_MODE=local_trust`. Use `AUTH_MODE=tailnet_trust` only when binding to a Tailscale IP; use `AUTH_MODE=token` with a strong `AUTH_TOKEN` for non-tailnet exposure.

---

## Vision

See [docs/VISION.md](docs/VISION.md) for the project's mission, competitive positioning, and strategic non-goals.

---

## Design Philosophy

**Sessions are living things.** The aquarium metaphor is not decoration. It encodes session state into spatial position, animation speed, and sprite shape so you can assess a fleet of sessions with a glance instead of reading text.

**The API is the truth.** The TUI is a client. The API discovers tmux sessions, tracks their state, and serves snapshots. You can point multiple TUIs at the same API, run the API headless, or build your own client against the REST endpoints.

**No infrastructure required.** No database, no Docker, no message broker. The server binary talks to tmux directly via `portable-pty`, persists state to flat files under `SWIMMERS_DATA_DIR` or the platform data directory, and serves HTTP on a single port.

**Thoughts are first-class.** The thought subsystem streams AI agent context (from Claude Code, Codex, etc.) into a side panel. Sessions that run AI coding agents surface their internal monologue alongside the terminal output.

---

## About Contributions

Please don't take this the wrong way, but I do not accept outside contributions for any of my projects. Feel free to open issues — bug reports in particular are welcome. PRs are fine as a way to illustrate a proposed fix, but I won't merge them directly; I'll have Claude or Codex review and independently decide whether and how to address them.

---

## License

MIT. See [LICENSE](https://github.com/build000r/swimmers/blob/main/LICENSE).
