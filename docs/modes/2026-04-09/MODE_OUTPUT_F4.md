# Failure-Mode Analysis / FMEA (F4) — Analysis of swimmers

## Thesis
Swimmers is already resilient to several local-runtime failures (PTY exit, replay truncation signaling, osascript error surfacing), but its highest realistic failure pattern for a solo dev setup is *silent degradation*: transport outages, daemon sync failures, and persistence write failures often degrade behavior without a durable, user-visible degraded-state signal, which can make the aquarium look healthy while data freshness or durability has already failed.

## Top Findings
§F1. **Discovery reliability failures are tolerated, but recovery is startup-only (no periodic rediscovery)**
- **Evidence:** `src/main.rs:103-110` and `src/main.rs:214` call tmux discovery only at startup; `src/session/supervisor.rs:488-490` skips stale reconciliation when discovery is unreliable; `src/session/supervisor.rs:564-577` discovery is callable but not scheduled.
- **Reasoning:** FMEA emphasizes recovery paths after substrate faults. Here, tmux-server failure is handled as a one-time startup concern, not a continuously recoverable condition.
- **Severity:** medium
- **Confidence:** 0.92
- **So What:** Add a low-frequency background discovery/reconcile loop (opt-in or default every N seconds) so tmux restarts and externally-created sessions are re-adopted without restarting `swimmers`.
- **Owner-ack?** no

§F2. **PTY closure path is cleanly detected, but it is terminal (no reattach path)**
- **Evidence:** `src/session/actor.rs:445-452` marks exited on PTY close; `src/session/actor.rs:526-529` ignores writes to closed PTY; `src/session/supervisor.rs:23-25` and `src/session/supervisor.rs:1293-1339` reap exited sessions quickly.
- **Reasoning:** Trigger-to-recovery chain is explicit: closure -> exited -> deletion. FMEA highlights that this is robust cleanup, but not service restoration.
- **Severity:** medium
- **Confidence:** 0.95
- **So What:** Add an explicit “reattach/adopt” API action (or periodic rediscovery) so accidental tmux client/PTTY teardown does not require full server restart to recover visibility.
- **Owner-ack?** no

§F3. **Replay overflow behavior is intentional but under-signaled to snapshot consumers**
- **Evidence:** `src/session/replay_ring.rs:37-43` evicts old frames; `src/session/replay_ring.rs:71-74` returns `None` when truncated; `src/session/actor.rs:616-627` emits `ReplayTruncated`; `src/session/actor.rs:1134` always sets `TerminalSnapshot.truncated = false`.
- **Reasoning:** FMEA cares about whether failure effects are detectable by the caller. Truncation is detectable for replay subscribers, but not reflected in snapshot payloads.
- **Severity:** medium
- **Confidence:** 0.88
- **So What:** Propagate truncation/quality metadata into snapshot and pane-tail responses (for example: `source=replay_fallback`, `possibly_truncated=true`) so UIs can warn when context is incomplete.
- **Owner-ack?** no

§F4. **Persistence write failure handling is inconsistent and can silently lose durability**
- **Evidence:** `src/persistence/file_store.rs:166-189` and `src/persistence/file_store.rs:277-279` log write failures but do not return errors; `src/session/supervisor.rs:1083-1122` cannot surface those failures; `src/api/thought_config.rs:80-89` does surface thought-config persistence failure as HTTP 500.
- **Reasoning:** In FMEA, silent write failure is high-risk because effects appear only after restart. The design currently has mixed signaling semantics across persistence paths.
- **Severity:** high
- **Confidence:** 0.94
- **So What:** Return and aggregate persistence health from `save_sessions/save_thought`, then expose a degraded flag in `/health` and API responses when durability is currently compromised.
- **Owner-ack?** no

§F5. **Remote/Tailscale disconnect yields stale UI state with weak persistent indication**
- **Evidence:** `src/bin/swimmers_tui/api.rs:210-216` has no startup retry for non-local targets; `src/bin/swimmers_tui/app.rs:1144-1173` keeps previous entities on refresh failure; `src/bin/swimmers_tui/app.rs:208-217` de-duplicates identical message text and `src/bin/swimmers_tui/app.rs:221-227` expires it after TTL.
- **Reasoning:** FMEA focuses on user-facing effect: the aquarium can keep rendering old fish states while transport is down, and the error banner can disappear despite ongoing failure.
- **Severity:** high
- **Confidence:** 0.90
- **So What:** Add persistent “DATA STALE / API DISCONNECTED” header state after N consecutive refresh failures; for remote targets, add bounded startup retry/backoff similar to local preflight.
- **Owner-ack?** no

§F6. **Concurrent TUIs can race on shared mutable endpoints (selection + thought config)**
- **Evidence:** `src/api/selection.rs:85-89` is last-write-wins without revision checks; `src/bin/swimmers_tui/app.rs:1189-1210` re-syncs selection during every merge refresh; `src/api/thought_config.rs:43-98` PUT overwrites global config without versioning.
- **Reasoning:** FMEA evaluates multi-actor contention; in this local-tailnet context, simultaneous TUIs are realistic and can cause state oscillation or accidental override.
- **Severity:** medium
- **Confidence:** 0.93
- **So What:** Add optional optimistic concurrency (`version` or `If-Match`) for thought-config and selection publication, plus “last_writer” metadata for debugging races.
- **Owner-ack?** no

§F7. **Thought daemon failures degrade behavior but are not promoted to health state or fallback mode**
- **Evidence:** `src/config.rs:66` defaults to daemon backend; `src/main.rs:121-144` chooses backend once at startup; `src/thought/bridge_runner.rs:81-85` logs and continues on daemon sync failure; `src/api/health.rs:11-41` health does not include daemon sync status.
- **Reasoning:** FMEA targets detectability. “Continue looping with warn logs” is operationally safe but invisible to users consuming only API/TUI surface.
- **Severity:** medium
- **Confidence:** 0.89
- **So What:** Track `last_successful_sync_at` and consecutive sync failures in AppState, expose in `/healthz`/`/version` payloads, and surface a TUI badge when thought pipeline is degraded.
- **Owner-ack?** no

§F8. **Mermaid path still has panic assumptions and is non-interruptible during heavy render**
- **Evidence:** production `expect(...)` in `src/bin/swimmers_tui/mermaid.rs:1702`, `src/bin/swimmers_tui/mermaid.rs:3385-3386`, `src/bin/swimmers_tui/mermaid.rs:3421`, `src/bin/swimmers_tui/mermaid.rs:3440`; rendering runs inline in frame path `src/bin/swimmers_tui/mermaid.rs:4733` -> `src/bin/swimmers_tui/mermaid.rs:1552-1563` -> `src/bin/swimmers_tui/mermaid.rs:4433-4508`; event processing happens per-frame in `src/bin/swimmers_tui/mod.rs:134-141`.
- **Reasoning:** FMEA asks “what happens under stress/interrupt.” If render cost spikes or an invariant breaks, Ctrl-C/keys are delayed until frame returns; panic exits process (panic hook restores terminal but still terminates).
- **Severity:** medium
- **Confidence:** 0.81
- **So What:** Replace remaining `expect` in mermaid hot path with recoverable errors; enforce render budget (skip/reuse cache when over budget) so input remains responsive under heavy diagrams.
- **Owner-ack?** no

## Risks Identified
- **R1 — tmux server restart/crash + startup-only discovery**
  - Severity/likelihood: medium / medium
  - Trigger: tmux server goes away after startup.
  - Detection: `tmux list-sessions`/`list-panes` warnings (`src/session/supervisor.rs:327-352`, `src/session/supervisor.rs:84-87`).
  - Effect: existing actors eventually exit and are reaped; new/restarted tmux sessions are not auto-adopted.
  - Current handling: process-exit reaper removes exited sessions fast (`src/session/supervisor.rs:23-25`, `src/session/supervisor.rs:1293-1339`).
  - Recommended handling: periodic rediscovery/reconciliation loop with jitter.

- **R2 — PTY pipe closure**
  - Severity/likelihood: medium / medium
  - Trigger: child shell exits or tmux PTY closes.
  - Detection: PTY EOF/logs and state transition (`src/session/actor.rs:2138-2140`, `src/session/actor.rs:445-452`).
  - Effect: input ignored, session disappears after reaper.
  - Current handling: mark exited + cleanup.
  - Recommended handling: optional auto-reattach/adopt strategy.

- **R3 — Replay ring overflow / truncation**
  - Severity/likelihood: medium / high (under bursty output)
  - Trigger: output volume exceeds replay capacity.
  - Detection: `ReplayTruncated` for replay subscribers (`src/session/actor.rs:616-627`).
  - Effect: old history dropped; snapshot consumers may not know quality/truncation status.
  - Current handling: bounded eviction and replay truncation signal.
  - Recommended handling: propagate truncation metadata to snapshot/pane-tail API and TUI.

- **R4 — Port 3210 already in use**
  - Severity/likelihood: low / medium
  - Trigger: another process binds configured address.
  - Detection: startup bind error (`src/main.rs:159-163`, `src/main.rs:242`, `src/main.rs:324-327`).
  - Effect: server does not start.
  - Current handling: process exits with bind error text.
  - Recommended handling: friendlier hint with conflicting port guidance (`SWIMMERS_BIND`/`PORT`) and quick probe of occupant PID where available.

- **R5 — Persistence write failure (disk full / perms / missing dir)**
  - Severity/likelihood: high / low-medium
  - Trigger: write/rename failure in persistence path.
  - Detection: logs only for sessions/thought snapshots (`src/persistence/file_store.rs:184-186`, `src/persistence/file_store.rs:277-279`); HTTP 500 only for thought-config PUT (`src/api/thought_config.rs:80-89`).
  - Effect: state appears updated in-memory but durability lost across restart.
  - Current handling: partial signaling.
  - Recommended handling: unified persistence health state + surfaced API warnings.

- **R6 — HTTP client disconnect during snapshot/pane-tail request**
  - Severity/likelihood: low / medium
  - Trigger: client drops connection before actor reply arrives.
  - Detection: oneshot send failure is ignored (`src/session/actor.rs:472-483`), request timeout path in API (`src/api/sessions.rs:339-357`, `src/api/sessions.rs:405-427`).
  - Effect: wasted work for that request, but no persistent state corruption.
  - Current handling: bounded by 5s timeout and no leaked mutable state.
  - Recommended handling: optional cancellation token plumbing if these endpoints become heavier.

- **R7 — Thought daemon crash/unavailability**
  - Severity/likelihood: medium / medium
  - Trigger: daemon process exits, handshake/read failures.
  - Detection: bridge warnings (`src/thought/bridge_runner.rs:81-85`), emitter retry-once behavior (`src/thought/emitter_client.rs:188-197`).
  - Effect: thought stream may stall while core session API remains “healthy.”
  - Current handling: retry/restart daemon once per failure path, continue loop.
  - Recommended handling: expose degraded thought health in API/TUI; optional backend failover policy.

- **R8 — osascript/native open failure**
  - Severity/likelihood: low / medium
  - Trigger: AppleScript/runtime/app availability issues.
  - Detection: explicit error propagation from native module (`src/native/mod.rs:456-464`, `src/native/mod.rs:639-647`).
  - Effect: native open action fails but core session tracking unaffected.
  - Current handling: API surfaces `NATIVE_DESKTOP_OPEN_FAILED` (`src/api/native.rs:167-177`).
  - Recommended handling: include structured error codes from script output for faster remediation.

- **R9 — Tailscale disconnect / remote route loss**
  - Severity/likelihood: high / medium
  - Trigger: tailnet route flap, remote API unreachable.
  - Detection: transport error messages (`src/bin/swimmers_tui/api.rs:251-266`) shown transiently in UI.
  - Effect: stale fish/selection view can persist; no lasting degraded badge.
  - Current handling: periodic refresh keeps retrying every cycle.
  - Recommended handling: sticky “disconnected/stale” state and exponential backoff with max retry cadence.

- **R10 — Concurrent TUIs race on selection publication**
  - Severity/likelihood: medium / high
  - Trigger: two TUIs continuously publishing different selected sessions.
  - Detection: difficult from UI alone; server stores last write (`src/api/selection.rs:85-89`).
  - Effect: published selection oscillates.
  - Current handling: per-client coalescing only (`src/bin/swimmers_tui/app.rs:450-502`).
  - Recommended handling: optional lease/owner semantics or monotonic version check.

- **R11 — Concurrent TUIs race on thought-config PUT**
  - Severity/likelihood: medium / medium
  - Trigger: overlapping saves from two operators.
  - Detection: none (last write wins) (`src/api/thought_config.rs:43-98`).
  - Effect: silent override of config.
  - Current handling: validation + persistence, no CAS/version.
  - Recommended handling: revision-based optimistic concurrency.

- **R12 — Clock skew (TUI vs API host)**
  - Severity/likelihood: low / low
  - Trigger: host clock differences or backward jumps.
  - Detection: thought ordering anomalies where older `updated_at` is ignored (`src/bin/swimmers_tui/thoughts.rs:131-137`).
  - Effect: potential suppression of legitimate thought updates after time regressions.
  - Current handling: timestamp-based monotonic filter.
  - Recommended handling: include sequence-based monotonic token from server alongside timestamps.

- **R13 — tmux command pressure under session churn**
  - Severity/likelihood: medium / medium
  - Trigger: many live sessions with frequent polls.
  - Detection: command failures/timeouts in actor logs (`src/session/actor.rs:915-939`, `src/session/actor.rs:1491-1731`).
  - Effect: stale cwd/tool/liveness accuracy; noisier logs.
  - Current handling: batching active-pane lookups in supervisor (`src/session/supervisor.rs:64-112`) and per-actor refresh intervals (`src/session/actor.rs:30-32`, `src/session/actor.rs:1394-1421`).
  - Recommended handling: centralize tmux polling per tick and fan out cached results to actors.

- **R14 — Terminal resize during render burst**
  - Severity/likelihood: low / medium
  - Trigger: rapid resize while expensive frame renders.
  - Detection: resize event path (`src/bin/swimmers_tui/events.rs:407-410`) and re-render invalidation (`src/bin/swimmers_tui/mermaid.rs:1556-1563`).
  - Effect: transient visual tearing/stutter.
  - Current handling: buffer reset and full clear (`src/bin/swimmers_tui/terminal.rs:146-157`).
  - Recommended handling: optional debounce of resize-driven heavy recompute.

- **R15 — Ctrl-C during Mermaid render / residual panic surface**
  - Severity/likelihood: medium / low-medium
  - Trigger: heavy or edge-case Mermaid layout/render path; user attempts interrupt.
  - Detection: panic hook restores terminal (`src/bin/swimmers_tui/mod.rs:119-127`), but process exits; key handling is frame-driven (`src/bin/swimmers_tui/mod.rs:134-141`).
  - Effect: crash or perceived unresponsiveness during long render frame.
  - Current handling: recover terminal state on panic.
  - Recommended handling: eliminate remaining `expect` in render path + render time budget/yield strategy.

## Recommendations
- **P0 (high benefit, medium effort): Persistence health unification**
  - Make `save_sessions`/`save_thought` return error status (or set an atomic degraded flag) and expose it through `/healthz` and TUI header.
- **P1 (high benefit, medium effort): Add persistent transport/degraded-state UX**
  - Keep a sticky “API disconnected / stale data” banner after repeated refresh failures; do not rely on expiring transient messages.
- **P2 (medium-high benefit, medium effort): Introduce periodic tmux rediscovery**
  - Background `discover_tmux_sessions_with_reason("periodic_reconcile")` with low cadence and jitter, respecting current `discovery_lock`.
- **P3 (medium benefit, low-medium effort): Add optimistic concurrency to shared mutable APIs**
  - Include revision/version in `/v1/thought-config` and `/v1/selection` writes to prevent silent last-write wins across concurrent TUIs.
- **P4 (medium benefit, low effort): Remove panic assumptions from Mermaid hot path**
  - Replace `expect(...)` with recoverable errors and show inline viewer error while keeping app alive.

## New Ideas and Extensions
- **Incremental:** Add `thought_backend_status` fields (`last_ok_at`, `consecutive_failures`) to `/healthz` and optionally `/version`.
- **Incremental:** Add `snapshot_source` metadata (`tmux_capture`, `replay_fallback`) for `GET /snapshot` and `GET /pane-tail`.
- **Significant:** Central tmux telemetry worker that polls once and shares pane/cwd/liveness cache with actors, reducing command fan-out.
- **Radical:** Optional “resilience mode” profile for remote/tailnet use (sticky degraded badge, backoff policy, stricter state-age indicators) while keeping local defaults lightweight.

## Assumptions Ledger
- Unstated assumptions this analysis depends on:
  - The main production usage is a single API process with one or a few TUIs, consistent with repo context.
  - `SessionCommand::Subscribe` is not yet externally exposed as a long-lived WS route in current release.
  - Daemon sync failures are currently primarily visible via logs, not via user-facing status.
- Project assumptions this mode questions:
  - “Warnings/logs are enough observability” for local-first tooling.
  - “Last write wins” is acceptable for operator-level shared mutable state.
  - “Startup-only discovery” is sufficient once process-exit reaping exists.

## Questions for the Project Owner
- Should tmux adoption remain startup-only by design, or is periodic adoption acceptable if kept low-overhead?
- Do you want concurrent TUIs to be cooperative (shared state) or intentionally independent (each owns its own selection/config)?
- Is a persistent degraded-state indicator in TUI acceptable, or do you prefer purely ephemeral messages?
- For persistence durability: is “best effort + logs” intentional, or should failures become first-class API/TUI health signals?

## Points of Uncertainty
- I did not run runtime fault-injection (kill tmux server mid-session, fill disk, flap tailnet), so failure timing is inferred from code paths.
- The practical frequency of Mermaid hot-path panic conditions depends on real-world diagram corpus not present in this static review.
- Actual user tolerance for stale-but-visible fish during disconnect is product-preference dependent.

## Agreements and Tensions with Other Modes
- Likely agreement:
  - Reliability/ops-oriented modes should agree on persistence signaling gaps, missing degraded-state surfacing, and race-prone last-write-wins APIs.
  - UX mode should agree that stale-data transparency is currently weak for remote disconnects.
- Likely tension:
  - Performance mode may resist adding frequent discovery/polling; F4 recommends it only with low cadence/jitter and shared caches.
  - Simplicity/minimalism mode may prefer log-only failure handling; F4 argues explicit degraded states reduce false confidence.
  - Security-focused mode may down-rank some items here (correctly) because loopback-first scope reduces blast radius; F4 still rates based on user-impact reliability.

## Confidence: 0.90
Calibration note: confidence would rise with targeted fault-injection runs (tmux crash, disk-full, tailnet flap, concurrent TUIs on shared API) and fall if hidden runtime constraints intentionally disable multi-TUI/shared-state scenarios in production usage.
