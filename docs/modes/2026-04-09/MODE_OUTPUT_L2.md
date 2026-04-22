# Debiasing / Calibration / Meta-Reasoning (L2) — Analysis of swimmers

## Thesis

This mode looks past *what the code does* and asks *where reasoning about the
code is most likely to be wrong*. Swimmers is a careful, opinionated project
whose recent trajectory (warnings-zero, loopback default, panic-surface
hardening, osascript sanitization) is self-aware — but self-awareness has a
shape, and that shape is visible. The hardening is reactive-symmetric: the
project fixes classes of issue that *became visible* during bootstrap
(panic, warnings, local shell injection, bind-address footgun) while leaving
quieter cognitive traps intact — silent env-var fallbacks, a test suite that
rehearses the author's mental model instead of challenging it, and a
`personal-workflows` feature gate that looks like discipline but is actually
the visible edge of an unresolved identity question. The debiasing lens
reveals these as *reasoning* defects, not implementation defects: they are the
places where a future reviewer (or the author three months from now) is most
likely to form a confident-but-wrong conclusion about the system.

## Top Findings

### §F1 — Silent env-var fallback codified as a test (`SWIMMERS_THOUGHT_BACKEND`)

- **Evidence:** `src/config.rs:25-31` `ThoughtBackend::from_env_value` maps
  any unrecognized value (including typos like `deamon`) to `Daemon`. The test
  at `src/config.rs:139-145`, `unknown_backend_defaults_to_daemon`, *locks in*
  this behavior — it asserts "an unrecognized input gives you Daemon" rather
  than asserting "an unrecognized input produces a warning *and then* falls
  back."
- **Reasoning:** This is an asymmetric risk dressed as a symmetric choice.
  The default was clearly migrated from `Inproc` → `Daemon` (the sibling test
  is named `inproc_backend_stays_available_for_compatibility`). A typo does
  not land a user on "the old thing," it lands them on "the new thing that the
  author is most confident will work for *their* workflow." The test is
  confirmation-biased: it encodes the author's mental model ("fall through to
  the new default") as truth instead of asking "is silent migration of a typo
  acceptable?" A log warning at parse time costs nothing and is absent.
- **Severity:** low
- **Confidence:** 0.85
- **So What:** Add a `tracing::warn!` in `from_env_value` on the unknown
  branch, citing the raw value, and flip the test to assert the warning fires.
  Do the same for `apply_env_port` and `apply_env_usize` (see §F2).
- **Owner-ack?** no

### §F2 — Base-rate neglect in env-var parse errors (`PORT`, `SWIMMERS_*`)

- **Evidence:** `src/config.rs:72-108`. `apply_env_port` swallows a parse
  error and silently keeps `3210`. `apply_env_usize` silently drops zero or
  unparseable values. `apply_env_non_empty_string` silently drops empty
  strings. None of these emit a log line.
- **Reasoning:** Typos in env var values ("PORT=32l0") are common — base rate
  is nonzero. The current design optimizes for the rare case (clean input) and
  fails silently on the common case (typo, empty override, pasted whitespace).
  The user believes they set PORT=3200; the server binds 3210; they waste time
  debugging why the TUI can't connect. This is base-rate neglect: a "can't
  happen" assumption embedded in code that will happen.
- **Severity:** low
- **Confidence:** 0.9
- **So What:** One `tracing::warn!` line per fallback path, citing the
  variable name and raw value. Zero behavioral change, large debuggability
  win.
- **Owner-ack?** no

### §F3 — Confirmation bias in TUI test design (happy-path saturation)

- **Evidence:** `src/bin/swimmers_tui/tests/mod.rs` is 9,500 lines with 216
  test functions. The `MockApi` architecture has per-method `VecDeque`s that
  support both `Ok` and `Err` results (e.g. lines 44-67), but a grep for
  `push_.*Err\(` returns only 9 hits. Two-hundred-plus tests, nine error-path
  injections.
- **Reasoning:** The mocks were built to exercise error branches; the tests
  were not. This is the canonical shape of confirmation bias in test design:
  the author built the adversarial harness, then spent their energy
  rehearsing the scenarios they already believed worked. The TUI's most
  interesting behavior (what does the user see when `fetch_sessions` returns
  `Err`? when the daemon is unreachable? when `open_session` fails mid-flight?)
  is under-covered relative to the mocks' capabilities.
- **Severity:** medium
- **Confidence:** 0.8 (grep-based, may under-count via different idioms)
- **So What:** Pick the 5 most load-bearing `fetch_*` / `open_*` / `publish_*`
  flows and add one error-path test each. Do not try to backfill all 200+.
  The ratio is the signal, not the absolute count.
- **Owner-ack?** no

### §F4 — Availability bias in hardening trajectory (what got fixed vs what didn't)

- **Evidence:** Recent commits `7ae8ea0` ("harden api panic surface, add
  health/version, graceful shutdown, sanitize osascript args"), `00d1941`
  ("default to loopback bind"), `cbf6122` ("drop workstation path
  fallbacks and mermaid panic"). Meanwhile, `src/auth/mod.rs:108-122` compares
  the bearer token with `provided == expected` — not a constant-time
  comparison. The README documents a first-class Tailscale exposure path
  (README lines 134-158).
- **Reasoning:** Availability bias in maintenance: the items that got
  hardened are the ones that *became visible* during the bootstrap push
  (panics crashed the server, warnings cluttered builds, the osascript path
  has a concrete injection story in the author's head). The bearer-token
  comparison is invisible — no one panics, no warning — so it isn't on the
  list even though the documented deployment includes network exposure.
  This is not a high-severity timing attack (the attacker would need
  Tailscale-level network adjacency) but the *reasoning asymmetry* matters:
  the project is hardening the paths it can *see* and trusting the rest.
- **Severity:** low (threat model: same-tailnet adversary; not a loopback
  concern)
- **Confidence:** 0.75
- **So What:** Swap `==` for `subtle::ConstantTimeEq` or `constant_time_eq`
  in token compare (single-file change, tiny dep). More importantly, adopt
  a rule: "every time we harden a visible path, ask what *invisible*
  siblings of this path exist." Keep a one-liner in CHANGELOG.md naming the
  invisible-sibling audit.
- **Owner-ack?** no

### §F5 — Anchor-validating tests, not calibration tests (`outbound_queue_bound`)

- **Evidence:** `src/config.rs:161-170`
  `burst_of_600_frames_fits_in_default_outbound_queue` asserts
  `outbound_queue_bound >= 600`. The actual default is 4096 (line 47). The
  test gives no justification for "600" beyond a comment saying "e.g. rapid
  AI agent output."
- **Reasoning:** This test is reverse-anchoring on a number someone chose
  once. If 4096 was picked first and 600 was invented later to justify it,
  the test is a tautology ("the number I picked ≥ the threshold I invented").
  If 600 was measured from a real burst and 4096 is 6.8× headroom, the test
  should cite *where* 600 came from (a profiling run? an incident?). Without
  that provenance, any future change to the default cannot be reasoned about
  — you can't tell whether you're breaking a calibrated margin or breaking
  nothing. This is anchoring bias institutionalized as a unit test.
- **Severity:** low
- **Confidence:** 0.7
- **So What:** Either (a) add a one-line comment citing the provenance of
  600 (a specific scenario, a commit, a metric), or (b) convert the test to
  a property: "the bound must exceed the largest observed burst in
  `metrics/mod.rs`." Option (a) is low effort; do it when next touching the
  file.
- **Owner-ack?** no

### §F6 — Narrative closure in the README (features framed as complete)

- **Evidence:** `README.md:48` — "**No database, no Docker** | File-based
  persistence, single binary, tmux is the only dependency" listed as a
  feature. `README.md:26` — "Backed by a Rust API server that discovers and
  manages tmux sessions." The Known Limitations section (`README.md` further
  down) lists *scope* limits (tmux only, mac+Linux, single machine) but not
  *maturity* limits (v0.1.x, flat-file durability under concurrent writes not
  proven, token mode path has less mileage than loopback).
- **Reasoning:** This is narrative closure: the README reads as "here is a
  finished thing, these are the features, these are the scope boundaries,"
  when the commit trajectory says "this is a month or two past bootstrap and
  still finding panic-surface bugs." Readers (and the author, re-reading in
  six months) will anchor on the finished-product tone and under-weight the
  early-stage caveats. A one-paragraph "what 'early release' means for you"
  section would calibrate readers without changing any behavior.
- **Severity:** low
- **Confidence:** 0.8
- **So What:** Add one paragraph near the top of README.md or as a
  "Maturity" subsection: v0.1.x, loopback is the tested path, token mode
  has less mileage, expect rough edges and file issues. This is a
  documentation diff, not code.
- **Owner-ack?** partial — CHANGELOG bootstrap commit (`cf97874`) implicitly
  acknowledges early-release posture; README tone does not reflect it.

### §F7 — Feature gate as unresolved identity, not discipline (`personal-workflows`)

- **Evidence:** `Cargo.toml:63-68` defines a `personal-workflows` feature,
  off by default. `src/api/web_actions.rs` is almost entirely behind
  `#[cfg(feature = "personal-workflows")]`. Commit `cb99681` frames the
  gate as "feature-gate personal workstation endpoints" to keep the
  crates.io build lean.
- **Reasoning:** A feature gate is the right *mechanism* when you have two
  real audiences. Swimmers has one audience (per README: solo dev, no
  outside contributions). So what is the gate actually doing? It's
  preserving code the author isn't ready to delete. That's endowment bias
  wearing discipline's clothes: the "lean crates.io build" framing is true,
  but the underlying question — *do these endpoints belong in swimmers at
  all, or do they belong in a sibling tool?* — hasn't been answered. The
  gate lets the question stay open indefinitely. The meta risk is identity
  drift by accretion: next year there are three feature gates, and the
  project's identity is whatever survives after flipping them all off.
- **Severity:** medium (identity, not functional)
- **Confidence:** 0.7
- **So What:** Answer the question *now* while the gate is still young.
  Either (a) delete `web_actions.rs` and move it to a private sibling repo
  that depends on swimmers, or (b) commit to the endpoints as first-class
  and delete the gate. The worst outcome is "leave the gate in place and
  forget why." If answering takes more than 15 minutes of thought, write
  a decision memo in the repo (not `MEMORY.md` — a committed ADR).
- **Owner-ack?** no

### §F8 — Unwrap in the error-response path itself (`dirs.rs`)

- **Evidence:** `src/api/dirs.rs:372, 588, 649` — `error_response()` calls
  `serde_json::to_value(ErrorResponse { ... }).unwrap()`. The struct has
  two `String` fields, so the serialization genuinely cannot fail under any
  normal condition.
- **Reasoning:** This is where base-rate neglect hides. "Serializing a
  two-field struct can't fail" is true *today*. The next person to add a
  field (say, a `details: serde_json::Value` that happens to contain a
  non-UTF-8 map key) re-enters the "can't fail" assumption without checking.
  Worse: the unwrap is in the *error handling path*, so a bug there panics
  the error handler — the one place you least want a panic, because it
  obscures the original error. Recent commit `7ae8ea0` specifically hardened
  the panic surface; this is the kind of residual that survives such a pass
  because it reads as "can't happen."
- **Severity:** low
- **Confidence:** 0.85
- **So What:** Replace with `serde_json::to_value(...).unwrap_or_else(|_|
  serde_json::json!({"code": code, "message": "serialization failed"}))`.
  Three-line change, removes a whole class of "error-path panics the
  error-handler" bugs from the file.
- **Owner-ack?** partial — the panic-surface hardening commit targeted
  visible panics; cold-path unwraps in error constructors were not in scope.

## Risks Identified

| Risk | Severity | Likelihood |
|---|---|---|
| Silent env-var typo masks config mistakes | low | high |
| Happy-path test saturation hides regressions in error UI | medium | medium |
| Invisible-sibling gaps (token compare, cold-path unwraps) accrue between hardening sprints | low | medium |
| README narrative closure leads users/reviewers to overestimate maturity | low | medium |
| `personal-workflows` gate entrenches an unresolved scope question | medium | medium |
| Anchor-validating tests mask drift in calibration-sensitive defaults | low | low |

## Recommendations

- **P1 — Log every env-var fallback.** One `tracing::warn!` per parse-miss
  (`PORT`, `SWIMMERS_THOUGHT_BACKEND`, `SWIMMERS_REPLAY_BUFFER_SIZE`,
  `SWIMMERS_OUTBOUND_QUEUE_BOUND`, `SWIMMERS_BIND`). Effort: low. Benefit:
  kills an entire class of "why didn't my env var take effect" support
  questions and shrinks §F1 + §F2. Do it in one commit.
- **P1 — Resolve `personal-workflows` identity question.** Either delete the
  gate (code lands as first-class) or delete the gated code (lands in a
  sibling repo). Effort: low if the answer is obvious, medium if an ADR is
  needed. Benefit: removes §F7 as a latent scope risk.
- **P2 — Five error-path TUI tests.** Not a backfill, a targeted injection:
  pick the 5 API calls the user sees most, push one `Err` into each, assert
  the TUI produces a sensible state. Effort: low. Benefit: reduces §F3 from
  "structural bias" to "conscious coverage gap."
- **P2 — Constant-time token compare + invisible-sibling audit note.**
  Add `constant_time_eq` dep (or `subtle`), replace the `==` in
  `src/auth/mod.rs:108-122`, and append one line to CHANGELOG.md naming the
  practice ("invisible-sibling audit after each hardening pass"). Effort:
  low. Benefit: addresses §F4 at low cost without over-rotating on a weak
  threat model.
- **P3 — Maturity paragraph in README.** Calibrate the narrative. Effort:
  very low. Benefit: downstream — anyone filing an issue has accurate
  expectations.
- **P3 — Cite provenance for the `600` anchor.** One comment or convert to
  a provenance-aware property test. Effort: low. Benefit: future-proofs
  reasoning about the default.
- **P3 — Replace error-constructor unwrap in `dirs.rs`.** Three lines.
  Effort: very low. Benefit: removes a "panic in the panic handler" class.

## New Ideas and Extensions

- **Incremental — "Config doctor" extension.** The existing `config doctor`
  CLI subcommand (commit `bcc0e8c`) is the right place to surface env-var
  parse fallbacks. Make `config doctor` print every env var the server
  would read, its raw value, its parsed value, and a ⚠ if they differ.
  Addresses §F1/§F2 at the diagnostic layer instead of the runtime layer.
- **Incremental — ADR folder.** Commit-driven ADRs (`docs/adr/NNNN-*.md`)
  would give the `personal-workflows` question (and future scope
  questions) a home. Not process overhead; just a 200-word file per
  decision. Respects "no infrastructure."
- **Significant — Test-coverage gradient for error paths.** A `make
  test-error-paths` target that greps for `push_*_Err(` in test files and
  prints a ratio vs `push_*(` success pushes. Once the ratio is visible,
  the bias corrects itself. Respects warnings-zero ethos (observability
  over policing).
- **Radical — None.** This mode does not have a radical recommendation for
  swimmers. The project is identity-stable; the debiasing work is all in
  the margins.

## Assumptions Ledger

- **My assumptions:**
  - The grep-based count of `push_*_Err(` is a reasonable proxy for
    error-path coverage; if error paths are exercised via a different
    idiom, §F3 is weaker.
  - `600` in the outbound-queue test does not appear elsewhere in the repo
    as a measured threshold (spot-checked, not exhaustive).
  - The token-compare timing-attack risk is theoretical at Tailscale
    adjacency; I rated §F4 accordingly and did not escalate.
  - The README's "Quick Start" prominence reflects tone, not maturity
    claims; I read tone as the risk, not the literal text.
- **Project's assumptions I question:**
  - "Silent fallback to a sane default is always safer than failing
    loud." On a solo dev's localhost tool, failing loud has no blast
    radius and high debug value.
  - "The hardening push found everything worth finding." Hardening
    sprints find visible items; invisible siblings persist until the next
    incident.
  - "A feature gate is a decision." A feature gate is a *deferred*
    decision; swimmers treats it as equivalent.

## Questions for the Project Owner

1. Was the thought-backend default migration (inproc → daemon) intended to
   silently re-home users whose configs had typos, or was that a
   side-effect of the parse-fallback design?
2. Where did `outbound_queue_bound = 4096` and the `600`-frame assertion
   come from — a measured burst, a guess, or a copy-paste anchor?
3. Do the `web_actions.rs` endpoints belong in swimmers long-term, or are
   they a sibling tool waiting to be extracted? If you don't know, is an
   ADR worth 20 minutes?
4. Post-hardening, what is the owner's mental ranking of *invisible*
   risks — paths that won't panic, won't log, won't fail tests, but might
   silently misbehave? §F4 and §F8 live there.
5. Is the TUI's error-state rendering (what the user sees when
   `fetch_sessions` returns `Err`) something you've eyeballed recently, or
   is it running on faith?

## Points of Uncertainty

- **§F3 (test bias) confidence depends on grep idiom.** I counted
  `push_*_Err(` call sites. If the tests use a different error-injection
  pattern (e.g. `.push_back(Err(...))` directly), I under-counted. The
  *ratio* is likely still skewed but the magnitude could be off.
- **§F5 (anchor test) is low-confidence because I didn't trace the git
  blame.** If commit history shows 600 came from a measured burst and
  4096 was calibrated afterward, the test is fine and the finding is just
  "add a comment." Didn't run `git log -S 600 -- src/config.rs`.
- **§F4 severity is load-bearing on the Tailscale threat model.** If the
  owner's reading is "token mode is for close friends on a tailnet, not
  defense-in-depth," my low rating holds. If token mode is meant to be a
  real perimeter, bump to medium.
- **§F7 (identity) is inherently a judgment call.** The author might have
  a clear answer I can't see from the code.

## Agreements and Tensions with Other Modes

- **Systems (F7):** Will find architectural seams (supervisor/actor/thought);
  likely agrees on §F7 if it spots the feature-gate seam. Tension: may
  propose abstractions I'd reject.
- **Root-Cause (F5):** Will converge on a specific bug; may contradict §F4
  by arguing the token compare is fine *because* the tailnet is trusted.
  That's a reasonable disagreement and the owner should weigh it.
- **Deductive (A1):** Strong on §F2 (logical "this env var parse path has
  no error edge") and §F8 (unwrap reasoning). Will agree on low-severity
  findings.
- **Adversarial (H2):** Will escalate §F4 to high/critical. Down-weight
  per deployment context.
- **Failure-Mode (F4):** Will catalogue PTY/tmux failure modes. Orthogonal
  to meta findings; no overlap.
- **Edge-Case (A8):** Will find the unicode/CJK edges in the replay ring.
  Partial agreement with §F3 (tests are happy-path).
- **Perspective (I4):** May push accessibility or multi-user angles;
  tensions with identity. Partial agreement with §F6 (narrative framing).
- **Inductive (B1):** May spot repeated patterns that suggest refactors;
  agreement with §F3 (pattern in mocks vs tests).
- **Counterfactual (F3):** Will ask "what if swimmers didn't wrap tmux?"
  Direct identity violation — filter.

## Confidence: 0.74

Calibration note: raises to ~0.85 if I trace git blame on the
`outbound_queue_bound` default and the thought-backend migration commit
(confirms §F1 and §F5 framing). Lowers to ~0.6 if an owner response
shows the `personal-workflows` gate has a committed deletion plan I
didn't see (§F7) or if the test-file error-injection idiom differs
from my grep (§F3).

---

## Sibling Mode Calibration

Predicted *most likely bogus finding* from each of the other nine modes in
this ensemble, so the lead agent can down-weight accordingly. These are
predictions about failure shape, not dismissals — each mode will also
surface real findings.

1. **Systems (F7).** Likely bogus: "extract a `SessionBackend` trait so
   tmux can be swapped for zellij/screen." Direct Identity-Check
   violation — tmux is the product. Filter.
2. **Root-Cause (F5).** Likely bogus: finds one concrete bug (say, a race
   in `ScrollGuard` or `SessionSupervisor`) and over-generalizes to "this
   is a class of synchronization bugs throughout the codebase." Trust the
   specific finding; discount the generalization.
3. **Deductive (A1).** Likely bogus: reasons from "this is an HTTP API"
   to "it needs pagination, rate limiting, idempotency keys, request IDs,
   and OpenAPI." None of those belong in a single-user loopback tool.
4. **Adversarial (H2).** Likely bogus: rates the localhost tool with a
   public-SaaS threat model — "the pane-tail endpoint leaks sensitive
   data!" Yes, to the user who owns the pane. Down-rate severity whenever
   the attacker model requires same-machine user privileges.
5. **Failure-Mode (F4).** Likely bogus: enumerates PTY/tmux death
   scenarios (tmux server crash, zombie PTYs, SIGCHLD reaping) that
   `SessionSupervisor` and `SessionActor` already handle. Cross-check
   with actual code before taking seriously.
6. **Edge-Case (A8).** Likely bogus: finds a unicode / combining-mark /
   emoji-cluster issue in the replay ring or render path and rates it
   critical. For a personal aquarium UI, broken emoji is a medium paper
   cut, not a critical. Down-rate severity.
7. **Perspective (I4).** Likely bogus: recommends collaborative/multi-user
   features, accessibility modes incompatible with an ASCII aquarium, or
   "what about Windows?" — all identity violations. Filter.
8. **Inductive (B1).** Likely bogus: sees three `apply_env_*` helpers in
   `config.rs` and proposes extracting a generic `EnvVarParser<T>` trait.
   Premature abstraction — three lines of similar code is the right
   amount.
9. **Counterfactual (F3).** Likely bogus: "what if swimmers were a plain
   terminal multiplexer dashboard without the aquarium metaphor?" The
   aquarium is load-bearing per the context pack. Filter unless the
   counterfactual stays *inside* identity (e.g., "what if sprites were
   data-driven from themes earlier?").

General weighting rule for the lead agent: modes 4 (Adversarial) and 7
(Perspective) are the most likely to overclaim severity on this specific
project because their natural threat/user models don't match a
solo-dev-loopback tool. Modes 1 (Systems) and 9 (Counterfactual) are the
most likely to propose identity-violating rewrites. Modes 2 (Root-Cause)
and 8 (Inductive) are the most likely to surface *real* findings *and*
over-generalize them — keep the finding, trim the generalization.
