# Perspective-Taking (I4) — Analysis of swimmers

## Thesis

Swimmers looks coherent from the author's chair — localhost-first, tmux-native,
single binary, `cargo install` and go. But when you sit in four specific user
chairs, the gap between *what the project advertises* and *what each
stakeholder actually receives on day one* opens up in places the author has no
reason to notice. The most surprising finding: the README's headline feature
(the "Thought rail") depends on a sibling binary, `clawgs`, that is not on
crates.io, not in the prerequisites, and not checked by `swimmers config
doctor`. Every other mode will look at the code that exists; this mode looks
at who walks up to it and what they get stuck on. The author's own daily loop
(source checkout, clawgs already built) hides that cliff from them.

## Top Findings

### §F1 — Cargo-install user: the "Thought rail" headline feature is silently broken on a fresh install

- **Evidence:** `README.md:42` lists "Thought rail" as a pillar in the Why
  swimmers table. `README.md:454-456` (FAQ) and `README.md:470-471` (Design
  Philosophy) describe it as "first-class." But the default backend is
  `ThoughtBackend::Daemon` (`src/config.rs:66`), and the daemon is an external
  binary: `EmitterClient::with_bin_and_request_sequence` resolves
  `CLAWGS_BIN` → `packaged_clawgs_bin` → `adjacent_checkout_clawgs_bin` →
  literal `"clawgs"` on PATH (`src/thought/emitter_client.rs:366-437`). A
  fresh `cargo install swimmers` user has none of those. `src/main.rs:201-207`
  then logs `"continuing without daemon defaults from clawgs"` and the bridge
  runner churns silently. `clawgs` is not in the README prerequisites
  (`README.md:67-74`), not in the env-var table, and not in the doctor checks
  (`src/cli.rs:223-305`). Its only mention is `QUICKSTART.md:117-136` as an
  *optional* "Structured Transcript Snapshot" helper — which understates the
  actual role as the thought rail's data source.
- **Reasoning:** Perspective-taking exposes that the author lives in
  `/Users/b/repos/opensource/swimmers` with `opensource/clawgs` next door, so
  `adjacent_checkout_clawgs_bin` always resolves for them. The cargo-install
  user has no "adjacent checkout" and no cue that they needed one. The
  discrepancy is invisible from the author's chair.
- **Severity:** high — the thing the README leads with doesn't work on the
  install path the README also leads with.
- **Confidence:** 0.9
- **So What:** Tomorrow the owner should either (a) add a `clawgs` row to the
  prerequisites table and a `clawgs` check to `run_doctor_checks`, OR (b) make
  the TUI thought rail render a one-line hint ("install clawgs to enable the
  thought rail — see docs/clawgs.md") when `daemon_defaults` came back None,
  OR (c) default `SWIMMERS_THOUGHT_BACKEND=inproc` when clawgs is absent and
  document it. Pick one; current state is the worst option.
- **Owner-ack?** No. Not in the Limitations list. QUICKSTART mentions clawgs
  only as a snapshot extractor, not as the thought-rail dependency.

### §F2 — Remote-over-Tailscale user: `SWIMMERS_BIND=host:port` is a silent footgun

- **Evidence:** `README.md:166` documents `SWIMMERS_BIND` as "interface only,
  not `host:port`." But `src/main.rs:159-163` bind is
  `format!("{addr}:{port}")`, so a user who copy-pastes
  `SWIMMERS_BIND=0.0.0.0:3210` (the natural shape in most server tooling) ends
  up trying to bind `0.0.0.0:3210:3210` and gets a cryptic bind error.
  `src/cli.rs:223-305` (`run_doctor_checks`) never inspects the bind string
  for an embedded `:`.
- **Reasoning:** Every other server-side tool the Tailscale user runs
  (caddy, ss, nginx, ssh, wireguard, Docker `-p`) uses `host:port`. Muscle
  memory will produce the wrong value, and nothing in the README or doctor
  catches it before launch. The author wrote the rule once and never has to
  retype it.
- **Severity:** medium (time-to-first-success for the remote user).
- **Confidence:** 0.95
- **So What:** Add a single doctor finding: if
  `config.bind.contains(':')`, surface a targeted error
  ("`SWIMMERS_BIND` is the interface only; use `PORT` to change the port").
  Also a one-line refusal in `run()` before `bind_listener`.
- **Owner-ack?** No.

### §F3 — Remote-over-Tailscale user: the TUI leaks the server-side `PORT` env into its default base URL

- **Evidence:** `src/bin/swimmers_tui/api.rs:41-44`:
  ```rust
  let config = Config::from_env();
  let base_url = std::env::var("SWIMMERS_TUI_URL")
      .unwrap_or_else(|_| format!("http://127.0.0.1:{}", config.port));
  ```
  The TUI calls the *server's* `Config::from_env()`, which reads `PORT`
  (`src/config.rs:72-78`). So any `PORT` that happened to be exported in the
  TUI user's shell (from another project, a `.env`, or the server the user
  just configured on another box) silently rewrites the TUI's default URL —
  and points at `127.0.0.1:PORT`, not the remote host.
- **Reasoning:** The server and TUI are distinct binaries but share
  `Config::from_env`. The author running both from the same shell notices
  nothing. A user who set `PORT=69420` for their server and now launches
  the TUI on a different laptop (or even the same laptop with
  `SWIMMERS_TUI_URL` unset) gets a TUI pointed at the wrong loopback port.
- **Severity:** low-medium (debuggable but confusing; a surprising action at
  a distance).
- **Confidence:** 0.85
- **So What:** Split a `TuiConfig::from_env()` that only reads
  `SWIMMERS_TUI_URL`, `AUTH_MODE`, `AUTH_TOKEN` — do not touch `PORT` from the
  TUI side. Five-line refactor.
- **Owner-ack?** No.

### §F4 — Remote user: TUI silently ships no auth header under `LocalTrust` + non-loopback URL

- **Evidence:** `src/bin/swimmers_tui/api.rs:45-48`:
  ```rust
  let auth_token = match config.auth_mode {
      AuthMode::Token => config.auth_token,
      AuthMode::LocalTrust => None,
  };
  ```
  And `targets_local_backend()` (line 73) exists but is not used to warn the
  user at startup. The user gets only the 401 round-trip error from
  `startup_access_error` (`src/bin/swimmers_tui/api.rs:87-102`). That message
  *is* actionable (it tells them to set AUTH_MODE=token), so this is a soft
  finding — but a front-loaded warning ("you pointed TUI at a non-loopback
  URL but `AUTH_MODE=local_trust`; the TUI will send no auth header") would
  save a cycle.
- **Reasoning:** The author tests both local paths and remote-with-token; the
  intermediate mis-config is specifically the remote user's failure mode.
- **Severity:** low.
- **Confidence:** 0.8
- **So What:** One preflight log line in `ApiClient::from_env` when
  `!targets_local_backend() && auth_token.is_none()`.
- **Owner-ack?** No.

### §F5 — AI-coding-agent operator: the thought rail is one-way, with no programmatic ingress

- **Evidence:** The only producer of thoughts is the clawgs daemon
  (`src/thought/bridge_runner.rs:19`, `src/thought/emitter_client.rs:144`).
  There is no `POST /v1/sessions/{id}/thought` route — `src/api/mod.rs` and
  `src/api/sessions.rs` expose snapshot/pane-tail/input/attention/dismiss but
  no thought ingress. `GET /v1/thought-config` / `PUT /v1/thought-config`
  exists only for tuning the runtime, not for writing updates. The agent
  operator running Claude Code inside a tmux pane cannot push an explicit
  "about to run a migration" breadcrumb into the rail from a hook or a slash
  command — they can only hope the clawgs log-extractor catches it.
- **Reasoning:** The README frames Thoughts as "first-class" and pitches the
  rail at AI-coding-agent users. But from the operator's chair, the most
  useful thought would be *the ones they choose to emit*, not the ones
  reverse-engineered from a transcript. The rail is currently more of a
  spectator surface than a collaboration surface.
- **Severity:** medium (design gap against stated value).
- **Confidence:** 0.75 (some uncertainty: it's possible clawgs itself
  exposes an emit API the operator could shell out to, which partially
  covers this.)
- **So What:** Either (a) add a minimal authenticated
  `POST /v1/sessions/{id}/thought` that writes a `ThoughtUpdatePayload`
  directly onto the control bus, or (b) document the clawgs-side pathway
  explicitly in the README so operators know how to script it. (a) is the
  higher-leverage change.
- **Owner-ack?** No.

### §F6 — AI-coding-agent operator: attention fires without surfacing the trigger pattern

- **Evidence:** `POST /v1/sessions/{id}/attention/dismiss` exists
  (`README.md:264`), but there is no `GET` that returns *why* the session
  entered attention state. `src/state/detector.rs` classifies states but
  nothing in the API exposes the matched pattern or matching line. The
  operator is told "session dev needs attention" and must context-switch into
  the pane, read the scrollback, and decide — exactly the ambient-awareness
  loss the aquarium is supposed to fix.
- **Reasoning:** The aquarium's promise is "assess at a glance." Attention
  without context forces a glance-plus-drill-in, which breaks the promise for
  the one state that matters most to an agent operator.
- **Severity:** low-medium (the `pane-tail` endpoint partially covers this,
  but it's an extra roundtrip and the TUI must remember the attention moment).
- **Confidence:** 0.7
- **So What:** Extend the session list row (or attention event) with the
  last N characters that triggered the state transition, so the aquarium can
  render a tooltip or the thought rail can show the prompt excerpt.
- **Owner-ack?** No.

### §F7 — Future maintainer: three ~2900-line files carry most of the weight, and only one of them has the pipeline documented up front

- **Evidence:** `wc -l` across the core:
  - `src/session/actor.rs` — 2932 lines
  - `src/session/supervisor.rs` — 2896 lines
  - `src/bin/swimmers_tui/app.rs` — 2766 lines
  The order-of-operations comment for actor.rs is buried at line 715
  ("ScrollGuard → StateDetector → ReplayRing → broadcast"), but it belongs at
  the top of the file. Compare `src/scroll/guard.rs:1-16`, which has the
  ideal file-level doc comment — problem statement, strategy, and the reason
  ScrollGuard exists *at all*. A developer-six-months-from-now reading
  `actor.rs` has to scroll 700 lines before finding the pipeline they're
  about to touch.
- **Reasoning:** The author built the mental model incrementally, commit by
  commit. A returning maintainer (themselves in six months, or their own AI
  agent) only sees the final pile. The scroll guard's top-of-file comment is
  already proof the author can write load-bearing docs when they choose to;
  this finding is asking them to replicate that exactly twice more.
- **Severity:** medium (maintainability, not correctness).
- **Confidence:** 0.9
- **So What:** Add ~20-line top-of-file comments to `actor.rs`,
  `supervisor.rs`, and `app.rs` that name the stages and why each exists.
  Use `src/scroll/guard.rs:1-16` as the template. No code changes.
- **Owner-ack?** No.

### §F8 — Docs auditor: "Make Targets" sits in the cargo-install reader's flow without a clear "source checkout only" gate

- **Evidence:** `README.md:186-196` ("Make Targets") appears immediately after
  the cargo-install Quick Start and before Configuration. The single line
  "If you are working from a source checkout" is easy to skim past. A fresh
  cargo-install user reading top-to-bottom will try `make tui` and get
  "make: *** No targets specified and no makefile found."
- **Reasoning:** The author uses `make tui` every day; the source-checkout
  path is their default. For a cargo-install reader, the Make Targets block
  is noise that looks authoritative.
- **Severity:** low (cosmetic docs).
- **Confidence:** 0.8
- **So What:** Either move Make Targets under a "## Development (source
  checkout)" heading at the bottom, or fold them into a collapsed details
  block. Two-minute edit.
- **Owner-ack?** No.

## Risks Identified

| Risk | Severity | Likelihood |
|---|---|---|
| First-time cargo-install user concludes the thought rail is a marketing claim and churns | high | high (F1) |
| Remote user mis-configures `SWIMMERS_BIND`, burns time on cryptic bind errors | medium | medium (F2) |
| TUI connects to wrong loopback port because of stray `PORT` env | low-medium | low (F3) |
| TUI hits 401 under LocalTrust + non-loopback URL, one extra roundtrip to diagnose | low | medium (F4) |
| Agent operator wants to script the thought rail and finds no ingress point | medium | medium (F5) |
| Attention state fires without enough context to act on it | low-medium | medium (F6) |
| Future maintainer (or agent) wastes a session reverse-engineering 2900-line files | medium | high (F7) |
| Cargo-install reader tries `make tui` and fails | low | low (F8) |

## Recommendations

**P0 (do tomorrow):**
1. **Fix the clawgs gap.** (F1) Add a clawgs row to prerequisites + a clawgs
   check to `run_doctor_checks` + a TUI banner when
   `daemon_defaults.is_none()`. Effort: low. Benefit: high — closes the
   biggest "advertised vs. delivered" gap in the project.
2. **Reject `SWIMMERS_BIND=host:port`.** (F2) One-line preflight check in
   `run()` and a matching doctor finding. Effort: low. Benefit: medium.

**P1 (this week):**
3. **Split TUI config from server config.** (F3) Add `TuiConfig::from_env`
   so the TUI does not accidentally read the server's `PORT`. Effort: low.
4. **Preflight warning for LocalTrust + non-loopback TUI URL.** (F4) Use
   existing `targets_local_backend()`. Effort: trivial.

**P2 (next bite):**
5. **Write top-of-file pipeline docs for `actor.rs`, `supervisor.rs`,
   `app.rs`.** (F7) Use `scroll/guard.rs` as the template. Effort: medium.
   Benefit: every future maintenance session.

**P3 (design-level):**
6. **Add `POST /v1/sessions/{id}/thought`.** (F5) Lets operators script
   breadcrumbs into the rail without clawgs. Effort: medium. Benefit:
   realises the "Thoughts are first-class" claim for agent operators.
7. **Surface attention triggers in the session payload.** (F6) Effort:
   medium. Benefit: closes the "glance → drill-in" loop the aquarium claims
   to solve.

**P4 (cosmetic):**
8. **Re-home Make Targets in the README.** (F8) Effort: trivial.

All eight recommendations pass the Identity Check (tmux stays the substrate)
and the project-values filter (no new infra, no DB, no Docker, no community
solicitation).

## New Ideas and Extensions

- **incremental** — `swimmers config doctor --check clawgs` that runs a
  round-trip `hello` handshake with the daemon and reports whether thoughts
  will actually flow. Extends the existing doctor surface.
- **incremental** — TUI empty-state for the thought rail: when no thought
  has ever been received for a session, show a one-line hint with a link to
  the clawgs setup doc. Cheaper than fixing F1 at the install-path layer.
- **significant** — A thin `swimmers thought push <session> <text>` CLI
  subcommand that talks to the existing API (once F5 lands). Gives agent
  operators a tmux-pane-local way to emit breadcrumbs with no Rust
  dependency.
- **significant** — Per-file pipeline headers become a project norm, not
  just a one-off on `scroll/guard.rs`. Ties directly to F7; cheap to
  institutionalise now while the file count is still small.
- **radical** — Make the thought rail the *consent surface* for agent
  actions: an agent operator can pre-register "I am about to run X; green
  means go," and a keystroke in the TUI approves/denies. Turns swimmers
  from a dashboard into a control plane for AI sessions. Respects the
  aquarium metaphor (the fish asks permission before darting).

## Assumptions Ledger

**Unstated assumptions in my analysis:**
- That the owner actually wants cargo-install users at all. The "no
  contributions" stance suggests the audience is maybe 1–5 people; if it's
  really just the author, F1/F2/F4/F8 shrink dramatically.
- That the thought rail is supposed to work without clawgs. It's plausible
  the author considers clawgs an inseparable part of the stack, in which
  case the fix is docs, not code.
- That "remote over Tailscale" is a real user group. The code clearly
  supports it (non-loopback gate, token auth) but I have no usage data.

**Assumptions the project makes that this mode questions:**
- That the README describes the install path accurately. It does not for
  the thought rail.
- That the TUI and server can share `Config::from_env` safely because they
  ship in the same crate. They can't — the envs should be scoped.
- That a feature mentioned in "Limitations" is the only thing users might
  get stuck on. The biggest sticking points (F1, F2, F3) are not in
  Limitations.

## Questions for the Project Owner

1. Is `clawgs` intended to ship alongside `swimmers` to crates.io, bundled
   into the `cargo install` flow, or remain a separate checkout? The
   answer decides whether F1 is a docs fix or a distribution fix.
2. Who is the second user, if any? If the answer is "no one yet, this is a
   personal tool," then F1/F2/F4/F8 all drop a severity tier.
3. Is the thought rail intended as read-only spectator or as a two-way
   agent surface? This decides whether F5 is a real gap or out of scope.
4. How often do you personally return to cold code in `actor.rs` or
   `app.rs`? If the answer is "rarely, because I have context," F7 may be
   deferrable. If "monthly, and I always re-read for 20 minutes," it's
   urgent.

## Points of Uncertainty

- I did not verify whether `BridgeRunner` actually logs a *user-visible*
  failure when clawgs is missing, or whether it only emits `tracing::warn`
  that the TUI never surfaces. The severity of F1 depends on this — if the
  TUI shows a clear banner today, F1 drops to medium.
- I assumed `SWIMMERS_BIND=host:port` produces a bad bind error. It is
  possible that `TcpListener::bind` parses `host:port:port` and rejects it
  with a readable message; I did not reproduce it.
- F5's severity depends on whether clawgs itself has an API for operators
  to push thoughts. If it does, F5 is a docs finding, not a code finding.
- My claim that `PORT` leaking into the TUI (F3) actually bites in
  practice depends on whether the typical remote user exports `PORT` in
  their shell for swimmers specifically (which is likely) or only sets it
  inline on the server command (in which case F3 is theoretical).

## Agreements and Tensions with Other Modes

- **cc1 (Systems):** Will independently flag `actor.rs`/`supervisor.rs` size
  and the clawgs coupling as a systems smell. Agreement on F1, F7. May
  disagree on F5 if systems-mode sees one-way thought flow as a clean
  boundary rather than a gap.
- **cc2 (Adversarial):** Will flag F2 as a security-adjacent footgun (an
  attacker might hope for misconfig → accidental exposure). I framed it as
  usability. Both framings land the same fix.
- **cc4 (Counterfactual):** Will ask "what if clawgs was bundled?" and land
  on the same P0 as F1. Likely strong agreement.
- **cc5 (Debiasing):** Will push back on F1/F5 by asking "is the thought
  rail actually the headline, or is the author's *aquarium view* the
  headline and the thought rail is add-on?" — and they'd be right to check.
  If debiasing-mode downgrades F1, I'd reduce F1 to medium.
- **cod1 (Root-Cause):** Will locate the clawgs coupling's root cause in a
  historical monorepo split. Complementary to F1, not conflicting.
- **cod2 (Deductive):** Unlikely to surface F1–F5 at all — deductive lens
  starts from invariants, not stakeholders. My findings and cod2's should
  not overlap much.
- **cod3 (Failure-Mode):** Will independently find the
  `SWIMMERS_BIND=host:port` case as an input-validation failure mode.
  Agreement on F2.
- **cod4 (Edge-Case):** Same as cod3 for F2; may also find the `PORT`
  collision in F3.
- **cod5 (Inductive):** Will notice the pattern "README features whose
  wiring is missing from the install path" and generalize F1 + F8.

Main tension I expect: debiasing-mode vs. my F1 severity. I rated it high
because it's the README's headline; debiasing-mode will ask whether it is
*actually* the headline, and the owner's answer decides it.

## Confidence: 0.82

**What would raise this:** confirming the TUI shows no banner today when
clawgs is absent; reproducing `SWIMMERS_BIND=host:port` locally; confirming
whether clawgs exposes its own operator API (which would downgrade F5).

**What would lower this:** learning that the owner considers the thought
rail a power-user extra not a headline (downgrades F1); learning that F3
cannot happen because users always set `SWIMMERS_TUI_URL` explicitly in
practice; learning that the three large files (F7) have already been
structured by `#[region]`-style tooling the author navigates by and the
line count is a red herring.
