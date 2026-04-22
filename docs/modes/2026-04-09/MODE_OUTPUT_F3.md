# Counterfactual Reasoning (F3) — Analysis of swimmers

## Thesis

Counterfactual reasoning asks "what does the absence of X forbid, and what does
the presence of X cost?" for choices that are still cheaply reversible. Applied
to swimmers, this lens reveals that several of the current design's "small
hedges" — the `inproc` thought backend, the bifurcated `snapshot` vs `pane-tail`
endpoints, the browser surface under `src/web/mod.rs`, env-var-only
configuration — are each carrying a concrete maintenance tax without unlocking a
use case the solo-dev owner actually has. In most cases the counterfactual
("what if this were removed or replaced?") is strictly simpler than the status
quo. The opposite finding — where the absence of something is actively
blocking — is the `data/` format: flat JSON files are forbidding nothing the
owner currently needs, so the counterfactual of "migrate to sqlite" is a false
option and should be explicitly ruled out to stop it recurring in future
reviews. This mode's unique contribution is to distinguish *load-bearing
absences* (good) from *accidental absences* (actionable) from *vestigial
presences* (actionable by deletion).

## Top Findings

### §F1 — The `inproc` thought backend is already a no-op; keeping the enum costs complexity with zero upside

- **Evidence:** `src/thought/loop_runner.rs:85-125` — `ThoughtLoopRunner::spawn`
  logs `"legacy inproc thought backend selected; delegating to clawgs daemon
  bridge"` and constructs a `BridgeRunner` identical to the `Daemon` path.
  `src/config.rs:19-32, 66, 122-124` still parses `SWIMMERS_THOUGHT_BACKEND`,
  defaults unknown values to `Daemon`, and threads `ThoughtBackend` through
  `Config`.
- **Reasoning:** The counterfactual "what if `inproc` were removed?" is already
  true *at runtime* — both variants delegate to the same bridge. The only thing
  the variant still does is (a) a warn log, (b) a branch in `main.rs` wiring,
  (c) three tests that lock in "inproc stays available for compatibility," and
  (d) one public enum variant. The absence forbids nothing because the feature
  it guarded no longer exists. The presence costs: a misleading config knob
  users might still tune, a test that artificially prevents deletion, and
  continued mental load for the one maintainer.
- **Severity:** low
- **Confidence:** 0.9
- **So What:** Tomorrow: delete `ThoughtBackend::Inproc`, delete
  `SWIMMERS_THOUGHT_BACKEND` parsing in `config.rs`, collapse
  `ThoughtLoopRunner` to a thin call-site of `BridgeRunner`, drop the three
  compatibility tests at `src/config.rs:140-158`. Document in CHANGELOG that
  `SWIMMERS_THOUGHT_BACKEND` is ignored. Net: ~40 fewer lines, one fewer knob.
- **Owner-ack?** no — this is a discovery.

### §F2 — `/snapshot` and `/pane-tail` are near-duplicates; unification removes 60+ lines without losing capability

- **Evidence:** `src/api/sessions.rs:305-358` (`get_snapshot`) and
  `src/api/sessions.rs:364-428` (`get_pane_tail`). Both: require
  `SessionsRead`, fetch supervisor handle, create oneshot, send a
  `SessionCommand` variant, 5-second timeout, same three error branches. The
  only real difference is the `SessionCommand` variant (`GetSnapshot` vs
  `GetPaneTail { lines: 300 }`) and the return payload shape.
- **Reasoning:** Counterfactual: "what if there were one endpoint
  `GET /v1/sessions/{id}/view?mode=snapshot|tail&lines=N`?" It loses nothing —
  both current payloads can be produced from a tagged union — and deletes one
  full copy of the handler plumbing. More importantly, the *current* split
  costs: every future cross-cutting change (auth scope, timeout policy, error
  shape) must be made twice. The absence of a unified endpoint is currently
  forbidding a clean place to add features like "return visible text + cursor
  + scroll position atomically," which is clearly on the trajectory given the
  thought subsystem's appetite for context. A SSE/WebSocket counterfactual
  (one stream delivers both) is overkill at v0.1.2 — there is already a WS
  path at `src/web/mod.rs:48` and pulling snapshots through it would entangle
  the terminal-first browser surface with the stable API. **Pull back from the
  streaming counterfactual; commit to the unified REST counterfactual.**
- **Severity:** medium
- **Confidence:** 0.75
- **So What:** Tomorrow: add `GET /v1/sessions/{id}/view?mode=…` as the new
  canonical surface; keep `/snapshot` and `/pane-tail` as thin forwarders for
  one release, then delete. This preserves the TUI client while unlocking the
  single-handler place to extend.
- **Owner-ack?** no.

### §F3 — The browser surface (`src/web/mod.rs`, 968 lines) is the largest vestigial presence the project could cheaply delete

- **Evidence:** `src/web/mod.rs` is 968 lines — larger than `src/api/sessions.rs`
  (995) — carrying HTML/CSS/JS asset routes, Frankenterm wasm/font serving, and
  a WebSocket path (`/ws/sessions/{id}`) for live output. CLAUDE.md line 63
  classifies it as `web/mod.rs — browser surface (terminal-first)`. The README
  limitations line 130-131 in the context pack explicitly says *"browser UI is
  terminal-first; animated aquarium remains native-only"* — i.e. the browser
  surface is already conceded to be a second-class citizen.
- **Reasoning:** Counterfactual: "what if the browser surface were removed
  entirely?" The owner-acknowledged position is that the aquarium — the
  identity-bearing metaphor per context pack lines 120-121 — is native-only.
  So the browser surface cannot host the primary experience. What does it
  unlock in its absence? ~968 lines of Rust, plus Frankenterm wasm bundling,
  plus the `axum` WebSocket dependency path, plus the entire observer-token
  code path's main consumer. What does its presence currently cost? A second
  auth surface, a second rendering pipeline, a second input path, and CI
  weight. What does its absence *forbid*? One legitimate thing: the
  `PUBLISHED_VIEW_ROUTE = "/selected"` (line 31) shareable selection URL —
  that is a genuine use case (read-only remote preview over Tailscale). **So
  the counterfactual should not be "delete web entirely" but "reduce to the
  published-selection read-only view and delete everything else."** That is
  the actionable shape.
- **Severity:** high (by code-volume / maintenance-tax yardstick, not by user
  harm)
- **Confidence:** 0.7 — would rise to 0.9 if I confirmed no real user depends
  on `/ws/sessions/{id}`.
- **So What:** Tomorrow: (a) keep `selected_index` + the minimal asset routes
  it needs; (b) delete `session_ws`, `app_js`, `rendered_surface_js`,
  `input_support_js`, and the Frankenterm wasm/font routes; (c) file an issue
  `"web: collapse to read-only published view"` to track. Measure: expect
  `src/web/mod.rs` under 200 lines.
- **Owner-ack?** partial — the owner already acknowledged browser is
  terminal-first/second-class, but has not committed to shrinking it.

### §F4 — A config file is not missing; env-var-only is the correct status quo — but the DOCUMENTATION of that decision is missing

- **Evidence:** `src/config.rs:72-132` reads `PORT`, `AUTH_MODE`, `AUTH_TOKEN`,
  `OBSERVER_TOKEN`, `SWIMMERS_BIND`, `SWIMMERS_THOUGHT_BACKEND`,
  `SWIMMERS_OUTBOUND_QUEUE_BOUND`, `SWIMMERS_REPLAY_BUFFER_SIZE` directly from
  `std::env`. There is no TOML/YAML/JSON config file read at startup (only
  `.env` via `env_bootstrap.rs` which is functionally a set of env vars).
- **Reasoning:** Counterfactual: "what if there were `~/.config/swimmers/
  config.toml`?" What would it unlock? (1) multi-profile config (dev, LAN,
  Tailscale), (2) discoverability via `swimmers config show`, (3) a place to
  put non-string settings (e.g. auth token rotation, per-tool thought
  overrides). What would it cost? A dependency on `toml`/`figment`/`config`,
  a merge-precedence rule (file vs env vs CLI), a schema migration story,
  and a parser-panic surface — exactly the opposite of the "warnings-zero,
  panic-hardening" trajectory in context pack line 144. For a solo dev on
  loopback with ~8 knobs, **the absence of a config file forbids nothing that
  matters**, and its presence would introduce a third persistence format
  (after `session_registry.json` and `thought_config.json`). **The
  counterfactual here says "don't add the file."** The actionable finding is
  not about adding; it's that this *non-decision* is undocumented, so it will
  keep coming up. An ADR or `docs/decisions/001-env-only-config.md` would
  freeze the decision.
- **Severity:** low
- **Confidence:** 0.85
- **So What:** Tomorrow: write a 15-line ADR titled "Configuration is
  env-only; no config file" with the rationale (fewer formats, no merge rules,
  dev-on-laptop fit). Keep `thought_config.json` as the one exception because
  it's runtime-mutable via `PUT /v1/thought-config`.
- **Owner-ack?** no — but the direction is clearly implied by existing values.

### §F5 — Flat-file JSON persistence is load-bearing correctly; the SQLite counterfactual is a false option

- **Evidence:** `src/persistence/file_store.rs:82-144` persists
  `session_registry.json`, `thoughts.json`, `thought_config.json` via
  atomic-rename writes through `spawn_blocking`. Tests at lines 411-433
  validate concurrent atomic writes. CLAUDE.md line 60 and context pack line
  117 explicitly value "no infrastructure: no DB, no Docker."
- **Reasoning:** Counterfactual: "what if this used SQLite / WAL?" What would
  it unlock? Range queries over sessions, indexes on state, time-series of
  thought transitions, `LIKE` searches on `tmux_name`. What does the solo-dev
  maintainer actually query? The cache is loaded *in full* at startup
  (`load_sessions_from_disk` → `Vec<PersistedSession>`) and mutated in memory.
  The working set is O(number of tmux sessions on one laptop) ≈ 5-50 rows.
  At that scale SQLite's advantages are zero and its costs are (a) a
  compile-time C dependency fighting with `cargo install swimmers`, (b)
  migrations, (c) a process-level lock file fighting `atomic_write_blocking`'s
  existing crash-safety story. **The counterfactual fails cost-benefit; flat
  JSON is correctly load-bearing.** The actionable sibling finding is that
  this should also be written down as a decision because it is the #1
  recommendation any generic code reviewer will make.
- **Severity:** low (actionable only as documentation)
- **Confidence:** 0.9
- **So What:** Tomorrow: second ADR — "Persistence is flat JSON; SQLite is
  rejected" with the numbers above. This converts a recurring recommendation
  into a settled decision.
- **Owner-ack?** yes — "no DB, no Docker" is in the values list.
  **Confirmed Known Risk**.

### §F6 — `SWIMMERS_REPLAY_BUFFER_SIZE=512KB` is a load-bearing default, but the counterfactual shows it's under-specified for AI-agent sessions

- **Evidence:** `src/config.rs:64` — `replay_buffer_size: 512 * 1024`. Test at
  line 173-176 locks this in. `src/config.rs:128-130` parses
  `SWIMMERS_REPLAY_BUFFER_SIZE` as a plain `usize`, no max, no units.
- **Reasoning:** Counterfactual A (10× smaller = 52KB): on an AI-agent session
  emitting 40-100 char tool-use deltas at 10 Hz, you'd overwrite the ring in
  under a minute. The `/snapshot` and `/pane-tail` endpoints (§F2) would start
  returning text that lags the actual tmux scrollback, breaking the thought
  subsystem's replay-derived `SessionInfo.replay_text` at
  `loop_runner.rs:32-33`. **Unblocked? nothing. Broken? thought fidelity.**
  Counterfactual B (10× larger = 5MB per session): at 50 concurrent sessions
  that's 250MB RSS before any PTY activity. On a dev laptop with Claude Code +
  browsers + IDE already running, this would put the aquarium in swap
  territory. **Unblocks: longer scrollback-derived context for thought
  extraction. Breaks: the local-first "runs on my laptop" value.** The *actual*
  counterfactual worth running is "make the value mode-dependent: small
  default, override for tool sessions." But implementing that re-invents what
  tmux's own history-limit already does.
- **Severity:** medium
- **Confidence:** 0.7
- **So What:** Tomorrow: (a) cap `SWIMMERS_REPLAY_BUFFER_SIZE` parsing at say
  8 MiB to prevent foot-guns; (b) add a comment at `config.rs:64` explaining
  the 512 KiB choice links to thought-context fidelity; (c) do NOT change the
  default. The counterfactual validated the current number.
- **Owner-ack?** no.

### §F7 — Native handoff restricted to iTerm/Ghostty/macOS: the wezterm/Linux counterfactual is real and cheap at one vendor

- **Evidence:** `src/native/mod.rs:16-34` — hard-coded `ITERM_SCRIPT_RELATIVE_
  PATH`, `GHOSTTY_SCRIPT_RELATIVE_PATH`, and `NativeDesktopApp::{Iterm,
  Ghostty}`. Both paths shell out to `osascript` (macOS only). `default_
  native_app` at line 50-56 falls back to `Iterm`. Context pack line 98 says
  "macOS + Linux only (Windows is out by design)" — so *Linux is supposedly in
  scope but has no native handoff path*.
- **Reasoning:** Counterfactual A — "add wezterm via `wezterm cli spawn
  --cwd`": wezterm has a first-class CLI, no osascript, cross-platform. Cost
  is a ~100-line `wezterm.rs` sibling to the osascript scripts, plus an enum
  variant. Unlocks: Linux users get *any* native handoff, which today they
  have none of — the `NATIVE_APP_ENV` unset on Linux silently defaults to
  `Iterm` (`line 50-55`), which will fail on every invocation. **This is an
  actual bug surface exposed by counterfactual reasoning**, not a
  hypothetical. Counterfactual B — "add kitty via `kitty @ launch`": same
  shape, ~100 lines. Counterfactual C — "add gnome-terminal / alacritty":
  alacritty has no remote control; gnome-terminal's D-Bus API is unstable.
  Skip. **Actionable: add wezterm first (covers mac+linux with one
  implementation), then kitty.**
- **Severity:** high (Linux users currently have a broken default)
- **Confidence:** 0.85
- **So What:** Tomorrow: (a) at `src/native/mod.rs:50`, detect OS and return
  `None` / an error on non-macOS instead of silently selecting iTerm; (b) file
  issue "native: add wezterm handoff"; (c) add `NativeDesktopApp::Wezterm`
  behind a new `wezterm-open.scpt`-equivalent shell script using
  `wezterm cli`.
- **Owner-ack?** no — README lists Linux as supported but the native layer
  contradicts that.

### §F8 — `personal-workflows` feature flag spreads `#[cfg]` across 5+ files; the separate-binary counterfactual is cleaner

- **Evidence:** from grep — `src/api/mod.rs:76,79`, `src/api/dirs.rs:1`,
  `src/api/skills.rs:1`, `src/host_actions.rs:1`, `src/api/web_actions.rs`
  with 18+ `#[cfg(feature = "personal-workflows")]` attributes across handler
  defs, route registrations, and test modules.
- **Reasoning:** Counterfactual: "what if `personal-workflows` were a second
  binary `swimmers-personal` instead of a cargo feature?" What does it unlock?
  (a) zero `#[cfg]` noise in shared files; (b) CI build matrix collapses from
  2 to 1; (c) warnings-zero posture at line 119 becomes easier to maintain
  because you don't need the `#![cfg_attr(not(feature = "personal-workflows"),
  allow(dead_code))]` workaround at the top of `dirs.rs`, `skills.rs`,
  `host_actions.rs`; (d) the public crate `swimmers` on crates.io ships with
  a cleaner surface. What does it cost? (a) a second `[[bin]]` entry and its
  own `main.rs` that composes routes from a shared `swimmers::personal`
  module; (b) users of personal workflows now run two processes or one via a
  CLI flag. At the current scale (v0.1.2, one maintainer, 18 cfg sites), the
  cfg-flag approach is actually *more* infrastructure than a second binary.
  **However** — Identity Check caveat: "single binary per role" is called out
  at context pack line 118. A second binary is *allowed* ("per role"), and
  `personal-workflows` is clearly a different role. The counterfactual
  respects identity.
- **Severity:** medium
- **Confidence:** 0.6 — depends on how often the feature flag is toggled in
  practice, which I can't see from static analysis.
- **So What:** Tomorrow: measure first. Count how many `#[cfg(feature = …)]`
  blocks exist under `src/` (grep says ≥18). If the count keeps growing month
  over month, extract to `src/bin/swimmers-personal.rs` re-using a
  `swimmers::personal` module. If the count is stable, leave it.
- **Owner-ack?** no.

## Risks Identified

| # | Risk | Severity | Likelihood |
|---|---|---|---|
| R1 | `inproc` backend dead-code rot (§F1) | low | high |
| R2 | Double-handler drift between `/snapshot` and `/pane-tail` (§F2) | medium | medium |
| R3 | `src/web/mod.rs` silently grows into a second product surface (§F3) | high | medium |
| R4 | Env-var config decision gets re-litigated in every code review (§F4) | low | high |
| R5 | SQLite migration recommended by a drive-by reviewer, owner churns defending (§F5) | low | high |
| R6 | Replay buffer misconfiguration by user (no cap) (§F6) | medium | low |
| R7 | Linux native-handoff silently broken (§F7) | high | certain on first Linux user |
| R8 | `personal-workflows` cfg sprawl (§F8) | medium | grows monotonically |

## Recommendations

**P0 — Linux native handoff fix (§F7).** At `src/native/mod.rs:50`, make
`default_native_app` OS-aware and return a clean "no native handoff available
on this platform" error on Linux. Effort: low. Benefit: high — removes a
latent bug the context pack's own claim "macOS + Linux only" contradicts.

**P1 — Collapse `inproc` thought backend (§F1).** Delete
`ThoughtBackend::Inproc`, `SWIMMERS_THOUGHT_BACKEND`, and the compatibility
tests. Effort: low. Benefit: medium (clarity, -40 LOC).

**P1 — Unify `/snapshot` + `/pane-tail` into `/view` (§F2).** Add the new
endpoint, keep the old two as forwarders for one release. Effort: low.
Benefit: medium (deduplication, extensible).

**P2 — Shrink `src/web/mod.rs` to the published-view only (§F3).** Delete
`session_ws` and the Frankenterm asset pipeline; keep `/selected`. Effort:
medium. Benefit: high (frees ~700 LOC and a whole dependency path).

**P2 — Two ADRs in `docs/decisions/` (§F4, §F5).** "Env-only config" and
"Flat-JSON persistence, SQLite rejected." Effort: low. Benefit: medium
(prevents recurring recommendations, documents reasoning for future you).

**P3 — Cap `SWIMMERS_REPLAY_BUFFER_SIZE` parse at 8 MiB (§F6).** Effort:
trivial. Benefit: low-medium (footgun removal).

**P3 — Add wezterm native handoff (§F7 extension).** New `NativeDesktopApp::
Wezterm` variant. Effort: medium. Benefit: medium (first Linux support).

**P4 — Extract `personal-workflows` to a second binary IF cfg count keeps
growing (§F8).** Effort: medium. Benefit: medium (only if the trend continues).

## New Ideas and Extensions

**Incremental:**
- `/view?mode=…` unification (§F2).
- Parse-time cap on replay buffer size (§F6).
- OS gate on `default_native_app` (§F7).

**Significant:**
- Shrink `src/web/mod.rs` to a published-view slice (§F3). Counterfactual
  reasoning says the absence of the interactive browser is near-costless; its
  presence is expensive.
- Add wezterm handoff (§F7).

**Radical (but still within identity):**
- Delete `src/web/mod.rs` entirely. Published-selection view could move into
  a tiny `published.rs` file under `src/api/` and serve one HTML page, one
  CSS, and one poll-based JSON endpoint. No WS, no wasm. This is radical
  relative to current LOC but conservative relative to the owner's stated
  "native aquarium is the product" stance.

## Assumptions Ledger

**Assumptions my analysis depends on:**
- The `clawgs emit --stdio` daemon actually works such that `inproc` is
  functionally dead. If `inproc` has a code path I missed that falls back to
  in-process logic when the daemon is missing, §F1 softens.
- The browser surface has no user outside the owner. If there is one, §F3 is
  wrong about deletion but right about shrinkage.
- Linux users of swimmers exist or are imminent. If the practical user set is
  "build000r on macOS only," §F7 drops from P0 to P3.
- `SWIMMERS_REPLAY_BUFFER_SIZE` has never been tuned in production. If it
  has, §F6's cost-benefit changes.

**Assumptions the PROJECT makes that this mode questions:**
- That keeping `inproc` as a compatibility variant has nonzero value. It does
  not (§F1).
- That serving browser assets over HTTP is a near-free adjunct to the API. It
  is not; it's the single largest file in `src/` (§F3).
- That two near-identical read endpoints are better than one parameterized
  one. That is a style preference that costs real deduplication work (§F2).
- That "Linux supported" can remain true without any native-handoff path on
  Linux. It can't — the defaults contradict the claim (§F7).

## Questions for the Project Owner

1. Does anyone (including you) still set `SWIMMERS_THOUGHT_BACKEND=inproc`?
   If no, green-light §F1 today.
2. Is there a real consumer of `/ws/sessions/{id}` in `src/web/mod.rs`, or is
   it historical? Needed to finalize §F3.
3. On Linux, what terminal handoff (if any) do you use when attaching to a
   swimmers session? Needed to prioritize wezterm vs kitty in §F7.
4. Would a `swimmers-personal` second binary fit your mental model better
   than 18+ `#[cfg]` attributes, or is the feature flag load-bearing for
   `cargo install` users who want the lean build? (§F8)
5. What's the biggest `replay_buffer_size` you've observed in practice? The
   8 MiB cap suggestion (§F6) assumes you've never needed more.

## Points of Uncertainty

- I did not verify that the `inproc` path has no lingering in-process
  behaviour in `BridgeRunner` itself — only that `ThoughtLoopRunner::spawn`
  delegates. Confidence on §F1 would rise with a `bridge_runner.rs` read.
- I can't measure how often `/snapshot` vs `/pane-tail` are called and
  whether the 300-line tail limit is load-bearing for some client (the TUI
  tests would tell). §F2's urgency depends on that.
- `web/mod.rs`'s `PUBLISHED_VIEW_ROUTE` may depend on assets I'd be deleting
  (CSS at minimum). Confidence on the shrinkage number in §F3 is 0.7, not
  0.9.
- §F8 confidence is lowest because static `#[cfg]` counts don't tell me how
  often personal-workflows is toggled off in practice.

## Agreements and Tensions with Other Modes

**Expected agreement:**
- **cc1 Systems:** will agree on §F2 (duplication between snapshot/pane-tail
  is a systems-level maintainability issue) and on §F3 (`web/mod.rs` is a
  system-boundary that doesn't carry its weight). Likely to be more cautious
  about §F1 because systems thinkers value optionality.
- **cod1 Root-Cause:** will likely locate §F7's Linux default as a root cause
  of the README-vs-code drift.
- **cod3 Failure-Mode:** will independently flag §F6 (unbounded
  `SWIMMERS_REPLAY_BUFFER_SIZE` parse) and §F7 (silent Linux fallback).
- **cod4 Edge-Case:** should find the same Linux-default-iTerm bug in §F7.
- **cc5 Debiasing:** will reinforce §F5 — "SQLite is obviously better" is a
  classic drive-by-reviewer bias this mode rules out.

**Expected tension:**
- **cc2 Adversarial:** may argue *against* §F3's browser shrinkage because a
  browser surface is a useful defense-in-depth pivot for remote observers via
  Tailscale; adversarial thinking tends to preserve optionality. I'd respond
  that the published-view slice retains exactly that.
- **cc3 Perspective:** may argue *for* keeping `inproc` (§F1) from an
  empathy-for-legacy-users angle. I'd counter that there are no legacy users
  at v0.1.2.
- **cod2 Deductive:** may reject §F8 because "feature flags are the
  well-defined idiom" — a rules-based argument that ignores the empirical
  cfg-sprawl.
- **cod5 Inductive:** likely to agree with §F5 (flat files work, trajectory
  is fine) but may pattern-match §F2 as "normal duplication, don't touch."

## Confidence: 0.75

**What would raise this:** reading `src/thought/bridge_runner.rs` to confirm
`inproc` is truly a no-op (§F1 → 0.95); running `wc -l` on `web/mod.rs` after
a hypothetical shrink to verify the 700-LOC delete (§F3 → 0.85); confirming
with the owner whether Linux users exist (§F7 → 0.95). **What would lower
it:** discovering the browser surface has a real user; discovering `inproc`
still has in-process hooks I missed; discovering the owner has a strong
aesthetic preference for the current endpoint split.
