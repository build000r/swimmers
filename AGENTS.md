# AGENTS.md

## Project Snapshot
- `swimmers` is a Rust 2021 crate for monitoring/managing tmux sessions as an aquarium UI.
- Main binaries:
  - `src/main.rs` -> `swimmers`, the standalone Axum HTTP/WebSocket API server.
  - `src/bin/swimmers-tui.rs` -> `swimmers-tui`, the native TUI; default mode hosts the API in-process.
- Library modules are exported from `src/lib.rs`. Core flows run through `SessionSupervisor`, per-session `SessionActor`s, state detection, thought collection, and flat-file persistence.

## Commands
- Install/build from checkout: `cargo build --release` or `cargo install --path .`
- Run TUI: `make tui` or `cargo run --bin swimmers-tui`
- Run server: `make server` or `cargo run --bin swimmers`
- Run browser surface: `make web`
- Run shared local backend plus browser URLs and TUI: `make up`
- Type-check TUI: `make tui-check`
- Rust tests: `cargo test`
- Metamorphic integration tests: `cargo test --test metamorphic`
- Perf/concurrency gates: `make ci-perf-gates`
- Coverage lcov: `make cargo-cov-lcov`
- Release workflow: `.github/workflows/release.yml` runs on `v*` tags or manual dispatch, builds the Linux `swimmers` binary, and renders notes with `scripts/release-notes-from-changelog.sh`.
- Unknown / verify first: no repo lint target is wired in Makefile or CI; check local `rustfmt`/`clippy` availability before relying on `cargo fmt --check` or `cargo clippy`.

## Important Paths
- `src/api/` - Axum routes and in-process API service helpers.
- `src/session/` - tmux discovery/supervision, session actors, replay ring, artifacts, overlays.
- `src/state/` - terminal/session state detection.
- `src/thought/` and `src/thought_ui.rs` - thought collection, daemon bridge, runtime config, UI metadata.
- `src/bin/swimmers_tui/` - native TUI app, rendering, input/event handling, picker, lifecycle, in-process API client.
- `src/web/` - browser terminal surface JS/CSS and Rust web routes.
- `src/persistence/file_store.rs` - JSON persistence with atomic writes.
- `scripts/` - run/smoke/perf/release helper scripts used by Make targets.
- `tests/metamorphic.rs` - proptest-based replay/scroll-guard invariants.
- `docs/VISION.md` - product/architecture intent; avoid duplicating it in code comments.

## Data, Config, State
- Runtime config is environment-variable based; there is no committed config file.
- Key env vars verified in README/code: `PORT`, `SWIMMERS_BIND`, `AUTH_MODE`, `AUTH_TOKEN`, `OBSERVER_TOKEN`, `SWIMMERS_DATA_DIR`, `SWIMMERS_TUI_URL`, `SWIMMERS_TUI_REUSE_SERVER`, `SWIMMERS_REPLAY_BUFFER_SIZE`, `SWIMMERS_THOUGHT_BACKEND`, `SWIMMERS_VOICE_MODEL`, `SWIMMERS_VOICE_LANGUAGE`.
- Default server bind is `127.0.0.1:3210`; non-loopback `AUTH_MODE=local_trust` is refused at startup.
- Data directory resolves from `SWIMMERS_DATA_DIR`, else platform data dir plus `swimmers`, else `./data/swimmers/`.
- Persisted files include `session_registry.json`, `thoughts.json`, and `thought_config.json` under the data dir.
- Repo theme overrides may live in `.swimmers/colors.json` inside inspected repos.

## Testing Notes
- `cargo test` is the broad local suite; many unit tests live inline across `src/`.
- `make ci-perf-gates` runs targeted thought, API hot-path, TUI bootstrap, and first-frame perf gates; expect it to be slower and environment-sensitive.
- Smoke scripts such as `scripts/test-run-tui.sh`, `scripts/test-web-live-terminal.sh`, and `scripts/stress-dirs-concurrency.sh` may require tmux, curl/lsof/bash behavior, and a local machine setup.
- `tests/artifacts/`, `target/`, `lcov.info`, `data/`, and `.swimmers/` are generated/ignored outputs.
- JS `.mjs` tests exist under `src/web/`, but no package/build file or Make target wires a JS test command; verify runner expectations before changing them.

## Coding And Architecture Conventions
- Prefer existing async/Tokio patterns; avoid blocking the runtime except through established `spawn_blocking` helpers.
- API behavior often has both HTTP handlers and TUI in-process mirror code; keep `src/api/*` and `src/bin/swimmers_tui/in_process.rs` semantics aligned.
- Flat-file persistence should remain crash-safe: use existing atomic write and lock helpers rather than direct ad hoc writes.
- Auth, bind safety, and token redaction are security boundaries; keep `AUTH_TOKEN`/`OBSERVER_TOKEN` out of logs and UI tables.
- Feature flags: `personal-workflows` enables local overlay directory/skill endpoints; `voice` adds `cpal`/`whisper-rs` and requires CMake plus a local Whisper model.

## Safety Rules
- Do not commit or print `.env`; in this checkout it is a symlink to an external env-manager path and is gitignored.
- Be careful with `make tui` and `scripts/run-tui.sh`: unless `SWIMMERS_TUI_REUSE_SERVER=1`, they may kill a stale local `swimmers` listener on the target loopback port.
- `make web` may restart an existing local `swimmers` listener if the expected web route is missing; it refuses unrelated listeners.
- Avoid destructive tmux/session actions in tests unless the test already uses the repo's test helpers.
- Do not edit generated artifacts in `target/`, `tests/artifacts/`, `data/`, `.ntm/`, or coverage files.
