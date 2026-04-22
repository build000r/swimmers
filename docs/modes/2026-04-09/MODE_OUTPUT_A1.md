# Deductive Reasoning / Invariant Verification (A1) — Analysis of swimmers

## Thesis
A deductive invariant pass shows swimmers is generally explicit about safety boundaries (loopback gate placement, command argument boundaries, atomic rename writes), but three claimed invariants are only partially true under all reachable states: strict replay boundedness, shutdown flush guarantees, and constant-time token checking. The key result is not style-level critique; it is that specific state sequences can violate those contracts today.

## Top Findings
### §F1 — Auth invariant is order-correct but only partially sound for all address-resolution states
- **Evidence:** `src/main.rs:182-192` performs `enforce_localtrust_loopback` before `bind_listener` at `src/main.rs:242`; loopback check is string-based in `src/cli.rs:314-316`; check allows `localhost` (`src/cli.rs:315`).
- **Reasoning:** Deductively, the pre-bind ordering invariant holds for raw config strings, but the predicate is not over resolved socket addresses. Therefore the truth of “loopback-only” depends on name-resolution assumptions external to the predicate.
- **Severity:** `medium`
- **Confidence:** `0.87`
- **So What:** Change the gate to resolve `SWIMMERS_BIND` + `PORT` and require every resolved address be loopback; keep refusing on ambiguity.
- **Owner-ack?** `no`
State sequence that breaks full invariant:
1. `AUTH_MODE=local_trust`, `SWIMMERS_BIND=localhost`.
2. `localhost` resolves to a non-loopback address (host config drift/misconfig).
3. `is_loopback_bind("localhost")` passes.
4. Listener binds non-loopback; LocalTrust remains enabled.

### §F2 — URL session-id invariant holds: URL `session_id` does not flow into tmux `Command::arg`
- **Evidence:** Path params are consumed as lookup keys (`src/api/sessions.rs:104-132`, `src/api/sessions.rs:164-173`, `src/api/sessions.rs:205-209`, `src/session/supervisor.rs:821-824`). tmux command args use `tmux_name` (`src/session/actor.rs:278-285`, `src/session/actor.rs:1331-1335`, `src/session/supervisor.rs:1664-1669`, `src/session/supervisor.rs:1687-1691`). Session IDs are generated as `sess_<n>` (`src/session/supervisor.rs:1220-1223`).
- **Reasoning:** Non-ampliative dataflow check: URL `session_id` is used as a hash-map key to retrieve an actor handle; only handle `tmux_name` reaches tmux command targets.
- **Severity:** `low`
- **Confidence:** `0.95`
- **So What:** Keep this separation invariant explicit with a regression test that malicious URL IDs never trigger tmux calls.
- **Owner-ack?** `no`
State sequence checked:
1. Caller uses `/v1/sessions/abc';$\n.../input`.
2. API calls `get_session(&session_id)`.
3. Miss path returns 404; no tmux process spawn/arg construction from that URL ID.

### §F3 — Replay-ring “bounded” invariant is false for oversized single frames
- **Evidence:** `ReplayRing::push` explicitly stores a frame larger than capacity after evicting all existing frames (`src/session/replay_ring.rs:37-46`), with comment documenting this behavior (`src/session/replay_ring.rs:38`). All runtime write paths call this same method (`src/session/actor.rs:702`, `src/session/actor.rs:738`).
- **Reasoning:** The invariant “total retained bytes <= capacity in all states” is disproven by direct transition construction.
- **Severity:** `medium`
- **Confidence:** `0.99`
- **So What:** Decide policy: either hard-cap (truncate/drop oversized frame) or rename invariant to “bounded except single-frame overflow” and meter/report overflow explicitly.
- **Owner-ack?** `no`
Exact break sequence:
1. Capacity = `10`, ring empty (`total_bytes=0`).
2. Push frame of length `11`.
3. Eviction loop does nothing because ring is empty.
4. `total_bytes += 11` => retained bytes exceed configured bound.

### §F4 — State-machine backward transitions are intentional and renderer-safe in reachable actor states
- **Evidence:** Backward transitions exist (`src/state/detector.rs:346-356`, `src/state/detector.rs:289-294`, `src/state/detector.rs:186-200`, `src/state/detector.rs:207-218`). Renderer handles all `SessionState` variants and idle/attention rest-state combinations (`src/bin/swimmers_tui/render.rs:520-535`, `src/bin/swimmers_tui/entity.rs:44-58`). Actor freezes PTY input path after exit (`src/session/actor.rs:422`, `src/session/actor.rs:445-450`).
- **Reasoning:** No monotonic-state assumption is required by renderer; deduced transition graph includes reversals by design, and output mapping is total for reachable combinations.
- **Severity:** `low`
- **Confidence:** `0.90`
- **So What:** Keep this non-monotonic contract documented; add an invariant test asserting renderer accepts every `(SessionState, RestState)` pair emitted by supervisor.
- **Owner-ack?** `no`
State sequence checked:
1. `Idle -> Busy` via output/liveness.
2. `Busy -> Idle` via prompt/silence.
3. `Idle -> Attention` via timer.
4. `Attention -> Busy` via liveness (`has_children=true`).
Renderer paths exist for each resulting state/rest combination.

### §F5 — Persistence atomic-write invariant holds for corruption, with durability caveat
- **Evidence:** Writes are temp + rename in same directory (`src/persistence/file_store.rs:370-381`), temp file cleanup on rename failure (`src/persistence/file_store.rs:377-379`), and callers surface/log failures (`src/persistence/file_store.rs:184-188`, `src/persistence/file_store.rs:324`).
- **Reasoning:** A crash/panic between temp write and rename leaves old target file intact; that preserves structural integrity of last committed file.
- **Severity:** `low`
- **Confidence:** `0.86`
- **So What:** If durability matters, add fsync on temp file and parent dir after rename; current invariant is atomic replacement, not durable commit.
- **Owner-ack?** `no`
State sequence checked:
1. Write `path.tmp.<uuid>` succeeds.
2. Process panics before `rename`.
3. Original `path` remains previous valid JSON; no partial overwrite.

### §F6 — Shutdown invariant fails for guaranteed final persistence flush on SIGTERM
- **Evidence:** Server uses graceful shutdown trigger (`src/main.rs:250-252`, `src/main.rs:258-287`), but there is no final `persist_registry()`/persistence-drain call in shutdown path (`src/main.rs`, `src/session/supervisor.rs`). Persistence is periodic every 30s (`src/session/supervisor.rs:1191-1199`) and thought persistence is queued asynchronously (`src/session/supervisor.rs:1405-1426`, `src/session/supervisor.rs:1467-1488`).
- **Reasoning:** Request draining and persistence flushing are distinct invariants. Only the former is explicitly wired.
- **Severity:** `medium`
- **Confidence:** `0.93`
- **So What:** Add shutdown orchestrator: stop accepting new thought updates, drain persist queue, invoke `persist_registry`, then exit.
- **Owner-ack?** `no`
Exact break sequence:
1. Recent in-memory session/thought changes occur.
2. SIGTERM received just before next checkpoint tick.
3. Graceful HTTP drain completes.
4. Process exits without explicit final persistence barrier.

### §F7 — Token-auth constant-time invariant fails (`==` comparison)
- **Evidence:** Bearer token validation uses direct string equality (`src/auth/mod.rs:108-112`, `src/auth/mod.rs:116-120`); no `subtle::ConstantTimeEq` usage appears in `src/`.
- **Reasoning:** Deductively, `==` does not provide a constant-time guarantee; therefore the claimed invariant does not hold as stated.
- **Severity:** `low`
- **Confidence:** `0.98`
- **So What:** Switch to constant-time byte comparison for both operator and observer tokens.
- **Owner-ack?** `no`
Break sequence:
1. Endpoint in `AUTH_MODE=token`.
2. Adversary sends many token guesses with controlled prefixes.
3. Timing oracle can, in principle, leak prefix information versus fixed-time comparator.

### §F8 — `SWIMMERS_BIND` interface-only contract fails loudly (not silently) when set as `host:port`
- **Evidence:** README contract says interface only (`README.md:166`). Raw env value is accepted (`src/config.rs:119-121`). Listener always appends `:{port}` (`src/main.rs:159-162`). LocalTrust gate checks exact interface tokens (`src/cli.rs:314-316`, `src/cli.rs:374-381`). Top-level run errors exit non-zero (`src/main.rs:324-327`).
- **Reasoning:** Two deterministic failure modes exist; neither is silent.
- **Severity:** `low`
- **Confidence:** `0.96`
- **So What:** Add explicit parse/validation at config load: reject values containing a port with a direct remediation message.
- **Owner-ack?** `no`
State sequences:
1. `AUTH_MODE=local_trust`, `SWIMMERS_BIND=127.0.0.1:3210` => rejected as non-loopback string at startup gate.
2. `AUTH_MODE=token`, same bind => gate passes, bind attempts `127.0.0.1:3210:3210`, startup exits with bind error.

## Risks Identified
- `medium` / `possible`: LocalTrust bind check trusts unresolved `localhost` token; resolution mismatch can violate intended loopback-only safety.
- `medium` / `likely`: Replay ring can exceed configured byte bound on oversized frame, defeating strict memory cap assumptions.
- `medium` / `possible`: SIGTERM may lose latest in-memory state due missing final persistence barrier.
- `low` / `unlikely`: Token timing side-channel from non-constant-time comparison in token mode.
- `low` / `possible`: Operator misconfig using `SWIMMERS_BIND=host:port` causes startup refusal/crash-loop until corrected.

## Recommendations
1. `P0` — Add explicit shutdown persistence barrier (effort: `medium`, expected benefit: `high`).
2. `P1` — Make replay-capacity semantics strict (truncate/drop/split oversized frame) and test it (effort: `low`, expected benefit: `high`).
3. `P2` — Validate loopback by resolved socket addresses, not string aliases; treat `localhost` as resolved-set check (effort: `medium`, expected benefit: `medium`).
4. `P3` — Use constant-time token compare for both tokens (effort: `low`, expected benefit: `medium`).
5. `P4` — Fail fast on `SWIMMERS_BIND` containing a port with clear message (effort: `low`, expected benefit: `medium`).

## New Ideas and Extensions
- `incremental`: Add invariant-focused tests for each contract in this pass (pre-bind gate ordering, replay hard cap, shutdown flush barrier).
- `significant`: Introduce a typed bind-config parser (`InterfaceBind` vs `SocketBind`) so invalid shapes cannot reach runtime bind code.
- `radical`: Add an optional “strict durability mode” that fsyncs persistence writes and drains queues before process exit (still file-based, no DB).

## Assumptions Ledger
- Assumption used: tmux CLI arguments passed through `Command::args` are not shell-interpreted, so shell metacharacters are inert at OS command parsing level.
- Assumption used: deployment may use standard name resolution where `localhost` maps to loopback; if not, current invariant weakens.
- Assumption used: “flushes persistence” means a final explicit barrier on shutdown, not only periodic checkpoints.
- Project assumption questioned: string-level loopback checks are sufficient for network-safety invariant.
- Project assumption questioned: replay ring “bounded” wording tolerates single-frame overflow.

## Questions for the Project Owner
- Do you want `localhost` treated as trusted alias, or should loopback be verified post-resolution only?
- For replay semantics, should a single oversized frame be dropped, truncated, chunked, or stored with explicit overflow metric?
- Is best-effort persistence acceptable on SIGTERM, or do you require a final deterministic flush barrier before exit?
- Is token-mode timing resistance in scope for your threat model when exposing over Tailscale?

## Points of Uncertainty
- tmux target grammar edge-cases for unusual session names are constrained by tmux itself; this pass did not execute live tmux fuzzing.
- Real-world exploitability of token timing differences depends on network jitter and deployment topology.
- The operational importance of lost sub-30s state at shutdown depends on how much users rely on freshest thought/session metadata after restart.

## Agreements and Tensions with Other Modes
- Likely agreement with security-oriented modes: constant-time token compare and resolved-address loopback validation are straightforward hardening wins.
- Likely agreement with reliability/testing modes: shutdown flush barrier and strict replay bounds should be backed by deterministic tests.
- Likely tension with performance/minimalism modes: fsync-based durability and strict queue draining can increase shutdown latency and complexity.
- Likely tension with product/UX modes: some low-severity hardening (timing side-channel) may be deprioritized against feature velocity in a local-first tool.

## Confidence: 0.91
Calibration note: confidence rises with one integration test proving shutdown flush semantics under SIGTERM and one replay test for oversized frames; confidence falls if hidden shutdown hooks outside `main.rs`/`supervisor.rs` already enforce persistence barriers.
