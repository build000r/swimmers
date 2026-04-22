# Systems-Thinking (F7) — Analysis of swimmers

## Thesis
Viewed as a system, swimmers is a small hub (`SessionSupervisor`) whose single
"answer every question" method — `list_sessions()` — has accidentally become
the convergence point of three unrelated concerns (rendering the TUI, feeding
the thought loop, and writing persistence checkpoints), and whose
"authoritative" state is actually split across six places whose reconciliation
rules are implicit and scattered. The mode that other lenses will miss: the
loudest behaviors in production (latency spikes, tmux-IPC chatter,
stale-session resurrection, occasional lost thought snapshots) are NOT bugs in
any one component — they are emergent properties of how the hub's couplings
compose under churn. Specifically, the 30-second persistence checkpoint calls
`list_sessions()` which calls `tmux list-panes -a`, meaning the act of
*saving state to disk* silently depends on tmux being responsive.

## Top Findings

### §F1 — `list_sessions()` is the load-bearing god-method; three unrelated pipelines all converge on it
- **Evidence:** `src/session/supervisor.rs:827-910` (`list_sessions`) and its
  callers at `src/api/sessions.rs:31`, `src/session/supervisor.rs:1092`
  (`persist_registry` → `list_sessions`), plus `collect_session_snapshots` at
  `supervisor.rs:948` which is a near-duplicate of the same pipeline for the
  thought loop (`SupervisorProvider::session_snapshots`, `supervisor.rs:1437`).
- **Reasoning (systems lens):** A single method is the convergence point of
  (a) the HTTP `GET /v1/sessions` hot path that the TUI polls, (b) the
  in-process thought loop's per-tick snapshot, and (c) the background
  persistence checkpoint every 30s. It performs a fan-out oneshot to every
  actor with a 2-second-per-actor timeout AND a synchronous `tmux list-panes
  -a` call AND per-session `discover_repo_theme(&cwd)` (walkdir on every
  call). Three callers, three cadences, one pipeline — so the worst-case
  latency of any one consumer is transitively exposed to all.
- **Severity:** medium
- **Confidence:** 0.85
- **So What:** Split `list_sessions()` into (i) a cheap in-memory
  `list_summaries_snapshot()` used by the persistence checkpoint and the
  thought loop's first pass, and (ii) a richer `list_sessions_with_tmux_join()`
  that does the list-panes + theme resolution only for the HTTP path. Theme
  resolution in particular (`resolve_repo_theme_for_summary` at `:197`) runs
  `walkdir::WalkDir` under `discover_repo_theme` for every session on every
  call — that's a filesystem scan per fish per poll.
- **Owner-ack?** no

### §F2 — Persistence accidentally depends on tmux liveness (cross-boundary coupling that shouldn't exist)
- **Evidence:** `persist_registry` at `supervisor.rs:1083-1122` calls
  `self.list_sessions().await`; `list_sessions` at `:869` calls
  `query_tmux_active_pane_session_ids` which shells out to `tmux list-panes
  -a` at `:71`. `spawn_persistence_checkpoint` at `:1192-1201` calls this
  every 30 seconds regardless of activity.
- **Reasoning:** A file-system persistence operation has no conceptual
  relationship to tmux, yet the control-flow graph shows a hard dependency.
  If the tmux server is briefly unresponsive (common after system sleep, or
  when a heavy tmux client is attached), the 30s checkpoint blocks on the
  tmux IPC instead of just dumping in-memory state. This is classic
  accidental coupling: two things that should be independent have been
  joined by a convenience reuse. It also means `persist_registry` quietly
  filters out any actor in `Exited` state (the `state != Exited` filter at
  `:846` inside `list_sessions`), so there is a window (between process
  exit and reaper firing at 250ms) where persistence can "lose" an
  Exited-but-not-yet-reaped session from `sessions.json`.
- **Severity:** medium
- **Confidence:** 0.9
- **So What:** `persist_registry` should build its `PersistedSession` list
  from an internal path that does not touch tmux. The thought snapshot cache
  is already persisted independently via `persist_thought` → `save_thought`;
  the registry only needs in-memory metadata (session_id, tmux_name, cwd,
  tool, last_activity_at, state) that the actor already holds. Breaking this
  one call removes a whole class of "why did my save stall?" incidents.
- **Owner-ack?** no

### §F3 — One authoritative state is actually six, and reconciliation rules are implicit
- **Evidence:** The session-state truth lives in:
  1. `SessionActor` fields — `state_detector`, `cwd`, `tool`, `last_activity_at`
     (`actor.rs:184-244`, **source of truth for live behavior**)
  2. `SessionSupervisor.sessions: HashMap<String, ActorHandle>` (`supervisor.rs:145`)
  3. `SessionSupervisor.stale_sessions: Vec<SessionSummary>` (`:148`, Exited mirror)
  4. `SessionSupervisor.thought_snapshots: HashMap<String, ThoughtSnapshot>` (`:168`)
  5. `SessionSupervisor.process_exit_seen_at: HashMap<String, Instant>` (`:171`, reap ledger)
  6. On-disk `FileStore` (`persistence/file_store.rs`) — `sessions.json` +
     `thoughts.json`, reloaded only at `init_persistence` (`supervisor.rs:214`).
  Reconciliation priority rules are scattered across `list_sessions`
  (`:872-901`), `collect_session_snapshots` (`:948-1060`), and
  `init_persistence` (`:229-285`), each implementing its own
  "thought_snapshot wins unless zero, else summary wins unless None" ladder.
- **Reasoning:** Whenever the same conceptual field (token_count,
  context_limit, rest_state, objective_changed_at) has three different
  storage sites with three different merge priorities, drift is inevitable
  under concurrent updates. For example, `list_sessions` at `:894` picks
  `thought_data.token_count` only if it is non-zero OR summary is zero —
  but `collect_session_snapshots` at `:1049-1054` unconditionally takes
  `thought_data` first. Two consumers of the "same" number get different
  answers within the same tick.
- **Severity:** medium
- **Confidence:** 0.8
- **So What:** Write the precedence rules down in one place: a
  `merge_session_view(summary, thought_snapshot) -> SessionSummary` helper
  used by every consumer. The three fan-out sites that currently hand-code
  the merge (`list_sessions`, `collect_session_snapshots`, `init_persistence`)
  should be reduced to mechanical calls into that helper.
- **Owner-ack?** no

### §F4 — Env-scrub inconsistency at exactly one tmux site: `list_tmux_session_names`
- **Evidence:** Of the 20 tmux `Command::new("tmux")` sites that this
  codebase invokes, 19 include `.env_remove("TMUX").env_remove("TMUX_PANE")`
  (grep confirms: `session/actor.rs` ×6, `session/supervisor.rs` ×3 others,
  `native/mod.rs`, etc.). The sole exception is
  `list_tmux_session_names` at `supervisor.rs:298-302`, which does
  `Command::new("tmux").args(["list-sessions", ...]).output()` with no env
  scrubbing.
- **Reasoning (systems/boundary lens):** The codebase's overwhelming
  convention at the tmux boundary is "always strip TMUX/TMUX_PANE" — that
  convention exists because if swimmers is launched from inside a tmux
  session (which the scrubbing implies is a supported scenario), the
  inherited `TMUX` env var routes the command to the *nested* tmux server,
  not the user's outer server. One missed site means `startup_discovery`
  can enumerate the wrong server when the operator launches swimmers from
  inside tmux. Then EVERY downstream tmux call that IS scrubbed operates
  against the outer server, and the supervisor's worldview silently diverges
  from the set it just discovered.
- **Severity:** medium (high for the specific nested-launch case;
  low-probability in solo localhost mode)
- **Confidence:** 0.95
- **So What:** Add the two `.env_remove` calls to
  `list_tmux_session_names`. One-line fix; closes a latent divergence.
- **Owner-ack?** no

### §F5 — `PROCESS_EXIT_DELETE_GRACE = Duration::ZERO` collapses a grace window that the surrounding code still treats as if it existed
- **Evidence:** `supervisor.rs:24` — `const PROCESS_EXIT_DELETE_GRACE:
  Duration = Duration::ZERO;`. The `reap_exited_sessions` loop at
  `:1293-1340` still calls `ready_process_exit_ids(&mut seen, &exited_ids,
  now, PROCESS_EXIT_DELETE_GRACE)` as though there is a grace period, and
  `process_exit_seen_at` is still a HashMap of "first seen at" timestamps.
  With grace = 0, a session becomes ready on the very first observation,
  making the entire "seen_at ledger" architecture vestigial — it's
  book-keeping that guards nothing.
- **Reasoning:** An emergent consequence of setting a tunable to zero
  without removing the ledger it governs: the reaper still does the RwLock
  write to record the timestamp, re-reads it, then immediately deletes it.
  That's contention on `process_exit_seen_at` every 250ms for no benefit,
  and a subtle invitation for future maintainers who increase the grace
  without realizing the ledger's semantics only work if each session is
  seen at a stable instant (which the summary-timeout path at `:1282` can
  violate by silently dropping).
- **Severity:** low
- **Confidence:** 0.85
- **So What:** Either (a) delete `process_exit_seen_at` entirely and reap
  on first observation, or (b) set a real grace (e.g. 500ms) and document
  *why* — e.g., to coalesce a flapping "Exited → Idle → Exited" detector.
  Current state is the worst of both.
- **Owner-ack?** no

### §F6 — The `SupervisorProvider` `std::thread::scope` + `block_on` bridge is a sign of a structural mismatch between the sync `SessionProvider` trait and the async supervisor
- **Evidence:** `supervisor.rs:1437-1513`. `session_snapshots()` and
  `thought_delivery_states()` both do `std::thread::scope(|s| s.spawn(||
  handle.block_on(...)).join())` because calling `block_on` inside an
  async context panics, and the trait is synchronous. The comment at
  `:1440-1443` explicitly names the workaround.
- **Reasoning:** This is a load-bearing boundary adapter between the
  thought loop's sync world and the supervisor's async world. It works,
  but every call spawns an OS thread, blocks it, and joins it — per
  thought-loop tick, per session scan. In a small system this is fine; it
  becomes interesting once the thought loop or any other caller runs at
  higher frequency or with many sessions, because each call is a kernel
  thread round-trip. More importantly, it's an emergent architectural
  smell: the `SessionProvider` trait was defined sync because the
  thought-loop runner is sync, but the supervisor then pays a per-call
  thread cost to honor the trait. One of them is lying about its runtime
  model.
- **Severity:** low
- **Confidence:** 0.8
- **So What:** Make `SessionProvider` async (or return borrowed snapshots
  via a sync channel that the supervisor fills from its own task). Either
  change eliminates the `thread::scope` hack and removes a subtle failure
  mode where a panic inside the scoped thread would propagate via
  `.expect("thread panicked")`.
- **Owner-ack?** no

### §F7 — The control-flow hub for "who calls tmux?" has no single choke point — 11 invocation sites, and the count is growing
- **Evidence:** `Command::new("tmux")` across
  `src/session/actor.rs` (×6), `src/session/supervisor.rs` (×4),
  `src/host_actions.rs`, `src/cli.rs`. Each site re-implements its own
  argv, env scrubbing, timeout handling, and stderr parsing (e.g. the
  "no server running" / "can't find session" string matches appear
  multiple times). `tmux_list_reports_no_sessions` lives in supervisor
  but the "can't find session" stderr check at `:1702` in
  `kill_tmux_session` is inlined separately.
- **Reasoning (systems lens, "identify the bottleneck component"):** The
  product identity is tmux — so the tmux boundary is by definition the
  most load-bearing surface swimmers has. Yet there is no
  `tmux_client::run(args)` helper. Every site is an independent
  re-implementation. That guarantees drift (see §F4) and means
  adding cross-cutting concerns (metrics, retries, structured error
  taxonomy for "not running" vs "hung") requires changing N places.
- **Severity:** medium (this is the component swimmers most depends on
  and least isolates)
- **Confidence:** 0.9
- **So What:** Introduce a thin internal `tmux_cmd(args: &[&str]) ->
  TmuxResult` helper that always env-scrubs, has one timeout, and
  classifies stderr once. Migrate sites opportunistically. This is a
  structural investment that pays down the F4-class bugs permanently and
  is in-identity (tmux is the product; isolating the tmux boundary is the
  opposite of "abstracting tmux away").
- **Owner-ack?** no

### §F8 — The actor's `tokio::select!` loop is the per-session bottleneck and the single select-arm starvation surface
- **Evidence:** `actor.rs:415-436` — one `select!` handles PTY reads,
  command channel, and timer. `ScrollGuard` at `scroll/guard.rs:93-106`
  contains an explicit comment and code path ("the coalescing window
  expired while output keeps streaming, flush on the next chunk so
  rendering keeps progressing even without timer wakeups winning the
  select race") showing the authors have already observed PTY-arm
  starvation of the timer arm under burst output.
- **Reasoning:** An emergent property: the session actor is a single
  point of sequential processing. If PTY output is a firehose (a scroll
  storm, `yes`, or a large `git log`), the timer arm can be starved, and
  the command arm (where HTTP `GetSummary` lands) queues behind output
  processing. A 2-second `GetSummary` timeout in the supervisor's
  `list_sessions` fan-out (`:845`) is therefore not arbitrary — it is
  the safety valve for the actor's single-loop bottleneck. The whole
  system's latency under load is gated by this one select. ScrollGuard's
  existence is already a mitigation for this.
- **Severity:** medium (latency under load, not correctness)
- **Confidence:** 0.75
- **So What:** Two tractable levers: (a) move the ScrollGuard burst
  processing into the PTY reader's `spawn_blocking` half so the actor's
  select loop sees pre-coalesced chunks (this is what the reader thread
  at `:402` could do cheaply), and (b) give the command channel priority
  inside the select using `biased;` so `GetSummary` does not queue behind
  a burst. Even without (a), adding `biased;` costs one line and makes
  the global `list_sessions` latency bound real rather than aspirational.
- **Owner-ack?** partial — the warnings-zero / scroll-guard work shows
  the authors know this loop is the hot spot; the specific `biased;`
  hint and the reader-side coalescing are not in the recent commit log.

## Risks Identified
| Risk | Severity | Likelihood |
|---|---|---|
| Persistence checkpoint stalls because tmux IPC stalls (§F2) | medium | medium |
| `list-sessions` run against nested tmux server when launched under tmux (§F4) | medium | low (but real when it happens) |
| Drift between `list_sessions` and `collect_session_snapshots` merge priorities (§F3) | medium | medium |
| Actor single-select starvation under output bursts (§F8) | medium | medium |
| `process_exit_seen_at` RwLock contention every 250ms for no behavioral benefit (§F5) | low | high |
| Thread-scope + block_on bridge propagating panics across async boundary (§F6) | low | low |
| Every new tmux call site re-implementing stderr parsing and env scrubbing (§F7) | medium | high over time |

## Recommendations

**P0 (ship first, small, high leverage)**
- **R1:** Add `.env_remove("TMUX").env_remove("TMUX_PANE")` to
  `list_tmux_session_names` (`supervisor.rs:298`). Effort: **low**.
  Benefit: closes the last inconsistent tmux boundary site. (§F4)
- **R2:** Add `biased;` to the actor's `tokio::select!`
  (`actor.rs:421`) so command-channel arms (notably `GetSummary`) are
  not starved by PTY bursts. Effort: **low**. Benefit: tightens the
  real-world behavior of `list_sessions`'s 2s per-actor timeout. (§F8)

**P1**
- **R3:** Decouple `persist_registry` from `list_sessions`. Build a
  dedicated in-memory path that walks `self.sessions` and asks each
  actor only for the minimal persistable fields, never touching
  `tmux list-panes`. Effort: **medium**. Benefit: removes the hidden
  tmux dependency from the 30s checkpoint and from create/delete
  hot-paths. (§F2)
- **R4:** Extract a single `tmux_cmd` helper that centralizes env
  scrubbing, timeout, stderr classification (`no server running`,
  `can't find session`, `no sessions`). Migrate sites incrementally.
  Effort: **medium**. Benefit: makes §F4-class bugs structurally
  impossible. (§F7)

**P2**
- **R5:** Factor the summary-merge precedence into one helper
  `merge_summary_with_thought_snapshot()` and call it from
  `list_sessions`, `collect_session_snapshots`, and `init_persistence`.
  Effort: **low**. Benefit: eliminates the three slightly-different
  merge ladders. (§F3)
- **R6:** Either remove `process_exit_seen_at` or set a real non-zero
  grace. Effort: **low**. Benefit: the intent of the ledger matches its
  behavior. (§F5)

**P3**
- **R7:** Make `SessionProvider` async (or re-shape it as a snapshot
  channel that the supervisor fills from its own runtime), so
  `SupervisorProvider` stops needing `std::thread::scope` + `block_on`.
  Effort: **medium**. Benefit: removes a known async/sync bridge hack.
  (§F6)

**P4**
- **R8:** Add metrics instrumentation on `list_sessions` call rate
  broken down by caller (HTTP, thought loop, persistence). This isn't
  a fix — it's the observability you need before §F1/§F2 become urgent.
  Effort: **low**. Benefit: factual evidence for the next tuning pass.

## New Ideas and Extensions
- **Incremental:** Expose a WebSocket or SSE push stream driven by the
  existing `lifecycle_tx` / `thought_tx` broadcasts (they are already
  `#[allow(dead_code)]` and have TODO markers at `supervisor.rs:119,919`).
  The infrastructure is built and unused — turning it on would let the
  TUI stop polling `GET /v1/sessions`, which directly drops the §F1
  pressure on `list_sessions`. Respects identity: it's still tmux-only,
  local-first, single binary.
- **Incremental:** A tiny "tmux liveness probe" task that pings
  `tmux list-sessions` once per N seconds and publishes a `TmuxHealth`
  status. Then `persist_registry` can skip the (to-be-removed, per §F2)
  tmux call when tmux is known-down, and the TUI can distinguish
  "no sessions" from "tmux is wedged."
- **Significant:** Invert the thought-snapshot cache → make the
  `SessionActor` the source of truth for its own `ThoughtSnapshot` too,
  and let the supervisor's cache be a pure read-through view populated
  by per-actor `event_tx`. This collapses §F3's six locations to three
  (actor, on-disk persistence, stale mirror) and makes
  `list_sessions`'s merge ladder a no-op.
- **Radical (likely out of scope but worth noting):** Replace the
  per-session tokio actor with a single-threaded event loop per
  supervisor that multiplexes all PTYs via `mio` — would eliminate
  §F8's per-session select bottleneck. Filtered out: this is a rewrite
  of the core substrate and the current actor-per-session model is
  already working; not worth the churn for a solo project at v0.1.2.

## Assumptions Ledger
**My unstated assumptions:**
- That `discover_repo_theme` (called per summary in `list_sessions`) is
  doing a non-trivial amount of work — I read the call site but not
  the implementation; if it's already memoized, the "walkdir per poll"
  concern in §F1 collapses.
- That the TUI polls `GET /v1/sessions` frequently enough that
  `list_sessions` latency matters. If the TUI uses push, §F1 is
  mostly theoretical.
- That tmux occasionally stalls on the operator's machine — §F2's
  severity depends on this being real rather than paranoia.

**Project assumptions my mode questions:**
- That the `SessionProvider` trait should be synchronous. Everything
  downstream of it lives in async, and the sync shim at
  `supervisor.rs:1437` is load-bearing for its sync-ness.
- That `list_sessions` is the right interface for persistence. The
  method's name and docstring describe an HTTP-shaped operation; using
  it as the source for a file-system dump conflates the concerns.
- That vestigial ledgers (`process_exit_seen_at` with grace=0) are
  harmless. Systems thinking says residual state with no semantic
  function is a trap waiting for a future change.

## Questions for the Project Owner
1. Is `list_tmux_session_names` (`supervisor.rs:298`) intentionally not
   env-scrubbed, or is that a bug? (I'm near-certain it's a bug — but
   worth a sanity check before fixing.)
2. Is `PROCESS_EXIT_DELETE_GRACE = Duration::ZERO` a measured choice, or
   a temporary value left in from debugging? The seen_at ledger is still
   wired up as if it matters.
3. Do you have telemetry on how often `GET /v1/sessions` gets hit per
   second from a busy TUI? I'm rating §F1 as medium on the assumption
   polling is constant; push-based would demote it.
4. Was the sync `SessionProvider` trait driven by the thought runner's
   architecture (`thought/bridge_runner.rs`, `thought/loop_runner.rs`)
   needing to run outside a tokio runtime, or is it an accident?
5. Have you observed the 30-second checkpoint stalling in practice —
   e.g., after system sleep when tmux is slow to respond?

## Points of Uncertainty
- I did not read `discover_repo_theme` or `FileStore::save_sessions`
  bodies. If either is async-cheap (no IO, or fire-and-forget),
  §F1 and §F2 severities drop.
- I did not read the full `collect_session_snapshots` consumer
  (`thought/bridge_runner.rs`, `thought/loop_runner.rs`) — my claim
  that the sync/async mismatch in §F6 is accidental rather than
  intentional is based on the shape of `SupervisorProvider`, not on
  the thought runner's own constraints.
- The §F8 "starvation" finding is backed by the authors' own comment
  in `scroll/guard.rs:93-106` acknowledging timer-arm starvation; I
  did not measure it.
- `list_sessions`'s 2-second per-actor timeout is correct per-actor
  but I'm not sure whether `futures::join_all` allows the whole call
  to exceed 2s under any condition (it shouldn't, but worth a test).

## Agreements and Tensions with Other Modes
- **Root-Cause (cod1):** Likely agrees with §F2 (tmux-in-persistence)
  and §F4 (env-scrub miss). May frame §F1 as a design choice rather
  than a root cause.
- **Deductive (cod2):** Should formalize my §F3 merge-ladder finding
  into a proof of divergence under specific inputs. Agreement expected
  on §F5 (vestigial ledger) — it's a trivial deductive contradiction.
- **Adversarial-Review (cc2):** Should push back on §F6 (the thread
  scope hack works and is localized — is the fix worth the churn?) and
  will probably argue §F5 is too minor to ship in v0.1.x. Likely to
  reinforce §F4 with specific reproduction scenarios.
- **Failure-Mode (cod3):** Should independently arrive at §F8
  (starvation) and §F2 (tmux-wedged checkpoint stall) as priority
  failure modes. May find a §F-class failure I missed around
  `persist_thought` queue full at `supervisor.rs:1467-1489` (drops
  thought snapshots silently when cap 256 is exceeded).
- **Edge-Case (cod4):** Will almost certainly hit §F4 via "what if you
  run swimmers under tmux?" — good corroboration. Will likely find the
  `join_all` + 2s timeout edge case I flagged as uncertain.
- **Perspective-Taking (cc3):** May push back on §F1 — a solo dev with
  few sessions may never feel the convergence-point pressure.
  Reasonable tension; rating medium not high reflects this.
- **Inductive (cod5):** The recent-commit pattern (warnings-zero,
  osascript sanitization, LocalTrust gate, panic hardening) inductively
  supports my §F4/§F7 framing — the codebase is in a
  "tighten-the-boundaries" mode and the missing env-scrub fits that
  remediation theme.
- **Counterfactual (cc4):** If `list_sessions` were a pure in-memory
  snapshot, §F1/§F2 collapse. That's exactly the R3/R5 recommendation.
  Strong alignment expected.
- **Debiasing (cc5):** Should catch my bias toward "this coupling
  should be broken up" — I am predisposed to see convergence points
  as bad. For a solo-dev tool, the convergence is also a simplicity win.
  Tension expected on R3 (effort:medium) being worth it now.

## Confidence: 0.8
Raising this would require: (a) actually measuring `list_sessions`
call frequency under a real TUI polling cycle; (b) reading
`discover_repo_theme` to confirm per-call cost; (c) reading
`FileStore::save_sessions` to confirm it's not already async-deferred.
Lowering this would come from learning that the thought runner's
sync constraint is real (making §F6 unfixable) or that §F4 is an
intentional choice with a reason I'm not seeing.
