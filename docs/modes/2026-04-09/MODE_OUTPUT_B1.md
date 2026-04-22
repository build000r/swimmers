# Inductive Pattern-Mining (B1) — Analysis of swimmers

## Thesis
The repeating pattern across swimmers is a deliberately pragmatic split: strict, channel-based actor boundaries for live session control, combined with permissive parsing/serialization shortcuts at the edges (Option-fallback discovery, `serde_json::to_value(...).unwrap()`, ad-hoc string parsing). This reveals an implicit design vocabulary of “keep the hot loop robust, keep integration code lightweight,” and the blind spot is that consistency debt is accumulating mostly in boundary glue (HTTP error envelopes, shell/text parsing, and detached task lifecycle).

## Top Findings

### §F1 — Error Handling Is Layered but Boundary Shortcuts Are Drifting
- **Evidence:** `anyhow::Result` is dominant in runtime paths (`src/session/actor.rs:259`, `src/session/supervisor.rs:1664`, `src/main.rs:165`, `src/api/mod.rs:46`); typed errors are isolated to daemon client (`src/thought/emitter_client.rs:8`, `src/thought/emitter_client.rs:56`); `Box<dyn std::error::Error>` appears only at TUI binary boundary (`src/bin/swimmers-tui.rs:6`, `src/bin/swimmers_tui/mod.rs:130`). Runtime `.unwrap()` sites cluster in response serialization (`src/api/native.rs:29`, `src/api/native.rs:74`, `src/api/dirs.rs:372`, `src/api/skills.rs:214`, `src/api/web_actions.rs:66`) and invariant assumptions (`src/native/mod.rs:404`, `src/bin/swimmers_tui/app.rs:2172`).
- **Reasoning:** The repeated shape implies an implicit rule: typed errors where protocol fidelity matters (daemon bridge), ergonomic `anyhow` for server internals, and “unwrap is acceptable at trusted serialization boundaries.” The violation pattern is that boundary `unwrap` has spread to multiple API modules with slightly different styles.
- **Severity:** medium
- **Confidence:** 0.93
- **So What:** Replace response-side `serde_json::to_value(...).unwrap()` with direct `Json(T)` or fallible envelope helpers so edge failures are non-panicking and style-consistent tomorrow.
- **Owner-ack?** no

### §F2 — `.unwrap()` Usage Is Dominated by Tests, but Runtime Categories Are Clear
- **Evidence:** `rg -n "\.unwrap\(" src | wc -l` = 237 total; classification by `#[cfg(test)]`/`/tests/` boundary gives 214 test and 23 runtime. Runtime samples: lazy serialization (`src/api/native.rs:74`, `src/api/dirs.rs:588`, `src/api/skills.rs:233`), impossible-state/invariant (`src/native/mod.rs:404`, `src/native/mod.rs:424`, `src/bin/swimmers_tui/app.rs:2190`), with no material “legitimate unavoidable” category beyond invariant locks/mode checks.
- **Reasoning:** Repetition shows the team norm is “tests may unwrap aggressively; runtime unwrap only in trusted invariants or convenience spots.” The drift is that convenience unwraps are now a cross-module API pattern.
- **Severity:** medium
- **Confidence:** 0.9
- **So What:** Keep test unwrap policy, but create a lintable runtime policy: no `unwrap` in `src/api/*` and `src/native/*` outside explicitly documented invariants.
- **Owner-ack?** no

### §F3 — Async Control Plane Is Consistently Actor-Owned, but Task Lifecycles Are Detached
- **Evidence:** Session actor event loop uses `tokio::select!` across PTY, commands, and timers (`src/session/actor.rs:421`); websocket bridge uses dual-stream `select!` (`src/web/mod.rs:528`) with oneshot ack timeouts (`src/web/mod.rs:697`). Detached background tasks are common: actor run loop spawn (`src/session/actor.rs:365`), persistence/reaper loops (`src/session/supervisor.rs:1194`, `src/session/supervisor.rs:1207`), delayed input enqueue (`src/session/supervisor.rs:1598`), thought bridge infinite loop (`src/thought/bridge_runner.rs:58-89`). Main starts these without retaining handles (`src/main.rs:217-218`, `src/main.rs:130`, `src/main.rs:139`).
- **Reasoning:** Pattern repetition implies a strong owned-actor message-passing model. The blind spot is lifecycle ownership: many loops are intentionally fire-and-forget, so cancellation semantics are implicit rather than explicit.
- **Severity:** medium
- **Confidence:** 0.88
- **So What:** Introduce a lightweight runtime task registry (store handles + cooperative stop signal) for periodic loops and bridge runner to make shutdown/test harness behavior explicit.
- **Owner-ack?** no

### §F4 — API Handler Shape Is Mostly Consistent, but Response Semantics Drift
- **Evidence:** Validation/auth-first + explicit status mapping is strong in sessions and thought-config (`src/api/sessions.rs:46-92`, `src/api/sessions.rs:205-285`, `src/api/thought_config.rs:43-90`). Drift: selection returns `200` with embedded error for missing/exited (`src/api/selection.rs:31-56`), while sessions/native use status codes (`src/api/sessions.rs:229-257`, `src/api/native.rs:116-154`). Error mapping by string inspection in sessions create (`src/api/sessions.rs:69-71`) contrasts with direct typed branching elsewhere.
- **Reasoning:** Repetition teaches an implicit rule (“auth first, then domain checks”), but the violation pattern is transport-contract inconsistency across modules.
- **Severity:** medium
- **Confidence:** 0.87
- **So What:** Standardize a single HTTP error contract matrix (`NOT_FOUND`, `CONFLICT`, etc.) and align selection/native/sessions in one pass.
- **Owner-ack?** no

### §F5 — Logging Philosophy Is Coherent and Secret-Aware, with Minor Boundary Exposure Risk
- **Evidence:** Macro counts: `warn!` 41, `info!` 34, `debug!` 33, `error!` 21; `println!`/`eprintln!` only 4 each and confined to CLI/startup (`src/cli.rs:349-360`, `src/main.rs:190`, `src/main.rs:320`). Secret redaction is explicit in config table (`src/cli.rs:132-137`). Structured startup/path logs are present (`src/main.rs:79-81`).
- **Reasoning:** Repeated structured tracing indicates a consistent runtime logging model. Blind spot is not token leakage but potential noisy path/error exposure from shell/tool stderr in local logs.
- **Severity:** low
- **Confidence:** 0.91
- **So What:** Keep current philosophy; add one redaction/sanitization helper for known high-noise shell stderr fields before logging.
- **Owner-ack?** no

### §F6 — Config Access Is Uniformly Arc-Passed, No Global Config Singleton
- **Evidence:** App state carries `Arc<Config>` (`src/api/mod.rs:31-40`), supervisor/actors receive clones (`src/session/supervisor.rs:179`, `src/session/actor.rs:257`, `src/session/actor.rs:341`), websocket uses state config directly (`src/web/mod.rs:685`, `src/web/mod.rs:730-735`). No `thread_local!/OnceLock<Config>/lazy_static!` config globals found by grep.
- **Reasoning:** The repeating pattern is explicit dependency passing through state/constructor boundaries. This is a strong implicit architectural rule with little drift.
- **Severity:** low
- **Confidence:** 0.96
- **So What:** Preserve this rule; codify “no global Config” in contribution notes/tests to prevent regressions.
- **Owner-ack?** no

### §F7 — Test Coverage Is Broad, but Reusable Fixture Vocabulary Is Fragmented
- **Evidence:** `#[test]` count 507, `#[tokio::test]` 93, `#[ignore]` 0, one `proptest::proptest!` block concentrated in TUI tests (`src/bin/swimmers_tui/tests/mod.rs:8731`). Repeated `test_state()` builders across API modules (`src/api/native.rs:204`, `src/api/sessions.rs:611`, `src/api/dirs.rs:723`). Shared env mutation lock exists (`src/test_support.rs:3`) and is reused in many tests.
- **Reasoning:** Pattern implies a strong “test everything” culture. Drift appears in fixture duplication rather than missing tests.
- **Severity:** low
- **Confidence:** 0.9
- **So What:** Extract a common API test-state builder and response helpers to reduce drift and maintenance overhead without reducing coverage.
- **Owner-ack?** no

### §F8 — Parsing/Quoting and Clone Economics Show Repeated Local Solutions Instead of Shared Primitives
- **Evidence:** Typed tmux target wrappers exist (`src/tmux_target.rs:1-7`) and are used in actor/supervisor (`src/session/actor.rs:278`, `src/session/supervisor.rs:1665`, `src/session/supervisor.rs:1688`), but other command/string boundaries use local parsing/quoting (`src/native/mod.rs:567-585`, `src/native/mod.rs:653-699`, `src/api/dirs.rs:180-203`, `src/session/actor.rs:1743-1749`). Shell quote helper duplicated across modules (`src/session/supervisor.rs:1588`, `src/native/mod.rs:583`, `src/host_actions.rs:306`). Hot-path cloning repeats in replay/broadcast/render (`src/session/replay_ring.rs:80`, `src/session/actor.rs:1045`, `src/bin/swimmers_tui/render.rs:150-152`).
- **Reasoning:** The recurring shape is “small local parser/helper per module.” That accelerates shipping but increases subtle drift risk and extra allocations in high-frequency paths.
- **Severity:** medium
- **Confidence:** 0.84
- **So What:** Consolidate shared parsing/escaping primitives first; then profile replay/broadcast/render to decide whether `Bytes`/shared buffers are warranted.
- **Owner-ack?** no

## Risks Identified
- API boundary panic risk from response serialization `unwrap` in multiple handlers; severity medium, likelihood medium.
- Background loop/task lifecycle ambiguity (detached loops without explicit stop handles); severity medium, likelihood medium.
- Transport contract inconsistency across endpoints (same semantic failure rendered as different HTTP/status/body patterns); severity medium, likelihood high.
- Parsing/escaping drift across native/supervisor/host-actions/manual parsers; severity medium, likelihood medium.
- High-throughput clone overhead under many subscribers or long replay windows; severity low, likelihood medium.

## Recommendations
- **P0 (effort: low, benefit: high):** Remove runtime response `unwrap` in API modules via a shared `json_error/json_ok` helper that never panics.
- **P1 (effort: medium, benefit: high):** Add task lifecycle management for spawned loops (handles + shutdown signal), starting with persistence reaper and thought bridge.
- **P2 (effort: medium, benefit: medium):** Define one API error/status matrix and migrate `selection`, `native`, `sessions`, and `web_actions` to it.
- **P3 (effort: low, benefit: medium):** Centralize shell quoting and parser utilities (tmux target, host parsing, assoc-array parsing) to reduce duplication and drift.
- **P4 (effort: medium, benefit: medium):** Profile replay/broadcast/render allocations; if hot, move frame payloads to shared bytes and avoid per-subscriber clones where possible.

## New Ideas and Extensions
- **Incremental:** Add a `#[cfg(not(test))] deny(clippy::unwrap_used)` gate in selected runtime modules (`src/api/*`, `src/native/*`) with local allowlist for documented invariants.
- **Significant:** Introduce a typed session-event stream endpoint that consumes existing lifecycle/thought broadcasts (`SessionSupervisor::subscribe_events`) and unifies pull/push semantics.
- **Radical:** Build a binary websocket frame protocol around existing opcode placeholders (`src/types.rs:804-810`) to reduce JSON overhead while preserving tmux-first identity.

## Assumptions Ledger
- Assumed typical deployment remains localhost-first with single-user ergonomics (as stated in context pack), so medium severities reflect developer-impact not multi-tenant blast radius.
- Assumed current `#[allow(dead_code)]` blocks are intentional staging; validated by symbol grep for WS payload types showing mostly type declarations + tests (`src/types.rs:710-809`, `src/types.rs:956-988`).
- Assumed task detachment is intentional but not yet lifecycle-managed because shutdown and spawn points do not retain handles (`src/main.rs:217-218`, `src/main.rs:130-142`).
- Project-level assumption questioned by this mode: repeated local parsers/quoters are “cheap enough”; this may stop being true as feature surface grows.

## Questions for the Project Owner
- Is `GET /v1/selection` intentionally contracted to return `200` with embedded errors for missing/exited sessions, or is this historical drift?
- Do you want runtime `unwrap` in API/native modules treated as policy violations, even when “practically infallible” today?
- Should lifecycle/thought broadcast subscriptions remain latent for now, or do you want push mode promoted into first-class API behavior soon?
- For performance: do you care more about preserving simple clone-heavy code, or about early optimization of replay/broadcast under heavy agent output?

## Points of Uncertainty
- I did not run runtime load/latency profiling, so clone-cost conclusions are pattern-based, not benchmark-confirmed.
- I inferred task lifecycle risk from spawn topology; actual operational impact depends on process shutdown behavior and test harness patterns not measured here.
- Some `allow(dead_code)` symbols may be consumed by feature combinations not enabled in this scan.

## Agreements and Tensions with Other Modes
- Likely agreement with a security/threat mode: no obvious secret-token logging pattern; strongest security-adjacent work remains edge hardening and auth contract clarity.
- Likely agreement with a reliability mode: detached background loops and mixed error contracts are reliability debt despite robust actor core.
- Likely tension with a performance-first mode: this mode flags clone hotspots as medium risk; a performance mode may escalate them sooner.
- Likely tension with a formal/spec mode: this mode tolerates pragmatic response drift; a spec-driven mode will likely demand stricter endpoint uniformity immediately.

## Confidence: 0.89
Calibration note: confidence would rise with targeted microbenchmarks (replay/broadcast/render) and a quick contract test matrix for API status/body semantics. Confidence would drop if hidden feature flags substantially change handler/error paths.
