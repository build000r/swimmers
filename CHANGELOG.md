# Changelog

All notable changes to swimmers are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.3] — 2026-04-15

- Added bridge health state, `/readyz`, retry backoff, and self-fencing so the daemon-backed thought loop can surface degraded/unhealthy state instead of failing silently.
- Made `SWIMMERS_THOUGHT_TICK_MS` configurable and wired the bridge health snapshot through the API response surface.
- Reworked the TUI thought rail so only the bracketed session label is clickable, tagged rows split cleanly into metadata/body lines, and badges inherit the session color.
- Removed a dead overlay helper so `cargo build --release` stays warning-free under the publish gate.

## [0.1.2] — 2026-04-08

- Rebuilt the GitHub release workflow inside `rust:1-bookworm` so published `swimmers-linux-amd64` assets stay compatible with Debian/Ubuntu environments that ship glibc 2.36.
- This fixes the `GLIBC_2.38` / `GLIBC_2.39` runtime failure from `0.1.1` when skillbox boxes install swimmers from GitHub releases.

## [0.1.1] — 2026-04-08

- Added a GitHub Actions release workflow that publishes a native `swimmers-linux-amd64` binary and companion SHA-256 asset for box installs.
- This release exists to remove the slow local `linux/amd64` emulation path from skillbox provisioning and recovery.

## [0.1.0] — 2026-04-07

First public release on crates.io. swimmers ships two binaries:

- `swimmers` — Axum HTTP server that discovers and manages tmux sessions
- `swimmers-tui` — terminal UI client that renders the aquarium and talks to the server

The project began life as **throngterm** (a mobile terminal manager with thronglet companions), was rewritten from a Node.js stack to a Rust + Tokio actor-per-session backend, and was renamed to swimmers in [`5bc4c03`](https://github.com/build000r/swimmers/commit/5bc4c03) before this release.

### Aquarium and session lifecycle

- Tmux session discovery loop with broadcast lifecycle events and persistence checkpoints ([`a040490`](https://github.com/build000r/swimmers/commit/a040490), [`61f4de9`](https://github.com/build000r/swimmers/commit/61f4de9))
- `SessionActor` per session with PTY I/O via `portable-pty`, replay ring buffer, and ScrollGuard redraw coalescing ([`8fdb541`](https://github.com/build000r/swimmers/commit/8fdb541), [`2cbcb12`](https://github.com/build000r/swimmers/commit/2cbcb12))
- Process-tree liveness reconciliation for sessions ([`c13adc7`](https://github.com/build000r/swimmers/commit/c13adc7))
- Duration-driven rest states (drowsy → sleeping → deep sleep) for idle swimmers ([`f9a81eb`](https://github.com/build000r/swimmers/commit/f9a81eb))
- State detector with idle/busy/error/attention classification, ANSI-strip false-positive guards, and prompt-recovery error clearing ([`98dcf5c`](https://github.com/build000r/swimmers/commit/98dcf5c), [`a74b14a`](https://github.com/build000r/swimmers/commit/a74b14a), [`27ccfe0`](https://github.com/build000r/swimmers/commit/27ccfe0))
- Batched tmux active-pane lookups via single `list-panes` call ([`97acc8a`](https://github.com/build000r/swimmers/commit/97acc8a))
- CWD-aware session spawn and OSC 7 cwd refresh events ([`0fa717d`](https://github.com/build000r/swimmers/commit/0fa717d), [`39ddfd0`](https://github.com/build000r/swimmers/commit/39ddfd0))

### Terminal UI (swimmers-tui)

- Aquarium view with state-driven animated ASCII sprites ([`83f9937`](https://github.com/build000r/swimmers/commit/83f9937), [`a75ce93`](https://github.com/build000r/swimmers/commit/a75ce93))
- Modular TUI split out of monolithic binary ([`876229c`](https://github.com/build000r/swimmers/commit/876229c))
- Mermaid viewer with semantic zoom, ER schema views, and pipeline rendering ([`3eeb324`](https://github.com/build000r/swimmers/commit/3eeb324), [`18d23ff`](https://github.com/build000r/swimmers/commit/18d23ff), [`ddd98df`](https://github.com/build000r/swimmers/commit/ddd98df), [`74fb30e`](https://github.com/build000r/swimmers/commit/74fb30e))
- Per-session mermaid + repo theme caching ([`d86c112`](https://github.com/build000r/swimmers/commit/d86c112))
- Domain plan viewer and session flow expansion ([`3028e26`](https://github.com/build000r/swimmers/commit/3028e26), [`ee71dc8`](https://github.com/build000r/swimmers/commit/ee71dc8))
- Startup wait with background retry against the API ([`9361f2c`](https://github.com/build000r/swimmers/commit/9361f2c))
- Skip native-status polls when sessions endpoint fails ([`7873e5b`](https://github.com/build000r/swimmers/commit/7873e5b))

### Thought subsystem

- Context-aware thought pipeline reading agent JSONL files ([`2385c8b`](https://github.com/build000r/swimmers/commit/2385c8b))
- OpenRouter backend with ANSI-strip dedup and purpose-driven prompts ([`a8cf1ef`](https://github.com/build000r/swimmers/commit/a8cf1ef))
- Token tracking, input-token extraction, and objective-shift timestamps ([`f977893`](https://github.com/build000r/swimmers/commit/f977893), [`458e53a`](https://github.com/build000r/swimmers/commit/458e53a), [`ebb28ce`](https://github.com/build000r/swimmers/commit/ebb28ce))
- Runtime-config protocol with backend selection and TUI editor ([`bfceb14`](https://github.com/build000r/swimmers/commit/bfceb14), [`199d672`](https://github.com/build000r/swimmers/commit/199d672))
- Sync preview endpoint and idle-inference noise reduction ([`cf486cd`](https://github.com/build000r/swimmers/commit/cf486cd), [`d5bdb61`](https://github.com/build000r/swimmers/commit/d5bdb61))

### Web surface and native handoff

- Browser-facing web surface and helpers ([`84cc6f1`](https://github.com/build000r/swimmers/commit/84cc6f1), [`edc21fa`](https://github.com/build000r/swimmers/commit/edc21fa))
- Native iTerm and Ghostty handoff for selected sessions
- Buffered output frames until snapshot loads to prevent garbled display ([`bad4b27`](https://github.com/build000r/swimmers/commit/bad4b27))
- Realtime framing alignment and push-first fallback ([`65f7854`](https://github.com/build000r/swimmers/commit/65f7854))

### Crate hardening for v0.1.0

The final stretch closed three publish blockers and nine should-fixes flagged by a pre-publish review.

- **Loopback by default.** Server now binds `127.0.0.1:3210`; non-loopback bind via `SWIMMERS_BIND` emits a stderr warning that LocalTrust auth is insecure off-loopback ([`00d1941`](https://github.com/build000r/swimmers/commit/00d1941))
- **MIT LICENSE** added at repo root and `license = "MIT"` set in `Cargo.toml` ([`0f5b48b`](https://github.com/build000r/swimmers/commit/0f5b48b))
- **README rewritten** around `cargo install swimmers`, both binaries documented, all links absolute ([`e1e2ee4`](https://github.com/build000r/swimmers/commit/e1e2ee4))
- **`Cargo.toml` polished** with description, repository, homepage, documentation, keywords, categories, authors, `rust-version = "1.75"`, exclude list, and `personal-workflows` feature flag ([`101b33e`](https://github.com/build000r/swimmers/commit/101b33e))
- **reqwest switched to rustls-tls** so `cargo install swimmers` works on clean Linux without OpenSSL headers ([`101b33e`](https://github.com/build000r/swimmers/commit/101b33e))
- **README env vars aligned** with code: `SWIMMERS_THOUGHT_BACKEND`, `SWIMMERS_REPLAY_BUFFER_SIZE`; phantom `THOUGHT_TICK_MS` and `SESSION_DELETE_MODE` rows removed ([`758acfa`](https://github.com/build000r/swimmers/commit/758acfa))
- **XDG data dir** via `dirs::data_dir()` with `SWIMMERS_DATA_DIR` override; no more writing to cwd ([`a0eafa2`](https://github.com/build000r/swimmers/commit/a0eafa2))
- **Personal-workstation endpoints feature-gated** behind `personal-workflows` (off by default): `.env-manager` browsing, skill scanning, commit-codex helpers ([`cb99681`](https://github.com/build000r/swimmers/commit/cb99681))
- **Workstation path fallbacks removed.** `/Users/b/...` frankentui defaults dropped; only `SWIMMERS_FRANKENTUI_PKG_DIR` env var is honored ([`cbf6122`](https://github.com/build000r/swimmers/commit/cbf6122))
- **Mermaid panic replaced** with `(0.0, 0.0)` fallback + `tracing::warn!` ([`cbf6122`](https://github.com/build000r/swimmers/commit/cbf6122))
- **Zero release-build warnings** after `cargo fix` and targeted `#[allow(dead_code)]` annotations ([`deb5da2`](https://github.com/build000r/swimmers/commit/deb5da2))
- **Clap CLI shell** with `swimmers config doctor` and explicit LocalTrust loopback gate ([`bcc0e8c`](https://github.com/build000r/swimmers/commit/bcc0e8c))
- **API panic surface hardened** with `/health`, `/version`, graceful shutdown, and sanitized osascript args ([`7ae8ea0`](https://github.com/build000r/swimmers/commit/7ae8ea0))

### Project history before the rename

- Project bootstrapped as **throngterm** in [`4ce3246`](https://github.com/build000r/swimmers/commit/4ce3246)
- Rust/Tokio rewrite from the original Node.js stack ([`a040490`](https://github.com/build000r/swimmers/commit/a040490), [`8aded72`](https://github.com/build000r/swimmers/commit/8aded72))
- Auth middleware, performance telemetry, and observer mode ([`dafda1c`](https://github.com/build000r/swimmers/commit/dafda1c), [`1185530`](https://github.com/build000r/swimmers/commit/1185530))
- Renamed throngterm → swimmers ([`5bc4c03`](https://github.com/build000r/swimmers/commit/5bc4c03))
- Legacy Node.js stack removed and docs updated for the Rust/Preact world ([`5d9c3ed`](https://github.com/build000r/swimmers/commit/5d9c3ed))

[0.1.1]: https://github.com/build000r/swimmers/releases/tag/v0.1.1
[0.1.3]: https://github.com/build000r/swimmers/releases/tag/v0.1.3
[0.1.2]: https://github.com/build000r/swimmers/releases/tag/v0.1.2
[0.1.0]: https://github.com/build000r/swimmers/releases/tag/v0.1.0
