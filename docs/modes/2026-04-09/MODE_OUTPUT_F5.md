# Root-Cause Analysis (F5) — Analysis of swimmers

## Thesis
The deepest structural driver in swimmers is a boundary mismatch: core behavior is inferred from presentation-layer terminal bytes and evolved in a personal-workstation context, but the project is now shipping as a public crate with broader runtime environments. That mismatch explains both the recent hardening churn and recurring fragility patterns (state heuristics, redraw coalescing, feature gating seams, and persistence semantics) better than any individual bug fix does.

## Top Findings
### §F1 — The hardening burst is primarily a late boundary-codification cycle, not independent bugs
- **Symptom:** A dense sequence of fixes (loopback default, LocalTrust gate, panic-surface hardening, osascript sanitization, workstation fallback removal, feature gates) landed right around first public release.
- **5-Whys:**
1. Why so many fixes at once? Because release blockers and should-fix issues surfaced together during publish prep (`CHANGELOG.md:59-76`).
2. Why did publish prep surface them? Because crates.io distribution exposed non-author environments and trust boundaries (`CHANGELOG.md:17-18`, `CHANGELOG.md:61-63`).
3. Why were those boundaries not already encoded? Because code paths carried local/workstation assumptions (personal workflows and path assumptions later gated/removed: `Cargo.toml:64-68`, `CHANGELOG.md:70-72`).
4. Why did assumptions persist? The architecture optimized for solo local iteration first, with enforcement added after (`src/main.rs:184-192`, `src/cli.rs:374-385`).
5. Why does this recur as churn? There is no single explicit pre-release contract asserting environment/trust invariants before features merge.
- **Root Cause:** Product-scope expansion (private tool -> public crate) outran explicit boundary contracts.
- **Root vs Symptom Handling:** Mixed. Loopback enforcement and feature-gating address root-boundary codification (`src/config.rs:56-58`, `src/main.rs:184-192`, `Cargo.toml:64-68`), while several fixes are instance-level patches (`src/native/mod.rs:587-598`, `CHANGELOG.md:72-73`).
- **Evidence:** `CHANGELOG.md:59-76`, `src/main.rs:184-192`, `src/cli.rs:374-385`, `Cargo.toml:64-68`, `src/native/mod.rs:587-598`.
- **Reasoning:** The same class of issue appears across security, packaging, crash behavior, and docs because the shared driver is boundary formalization lag.
- **Severity:** high
- **Confidence:** 0.89
- **So What:** Add a single “release boundary contract” checklist enforced in CI (auth/bind invariant, feature matrix, endpoint availability, packaging assumptions) so future hardening is preemptive instead of bursty.
- **Owner-ack?** yes — Confirmed Known Risk (`CHANGELOG.md:59-76`).

### §F2 — State misclassification risk is rooted in semantic inference from output bytes, not in any one regex
- **Symptom:** Session state (idle/busy/error/attention) can still be wrong in edge terminal streams despite many fixes.
- **5-Whys:**
1. Why can state be wrong? Because fallback classification still relies on prompt/error heuristics (`src/state/detector.rs:167-219`, `src/state/detector.rs:580-634`).
2. Why rely on heuristics? Because OSC 133 markers are optional and not universal (`src/state/detector.rs:124-165`, `src/state/detector.rs:183-206`).
3. Why not rely only on process truth? Liveness is periodic reconciliation, not primary signal (`src/session/actor.rs:903-942`, `src/state/detector.rs:330-359`).
4. Why is logic still fragile? Prompt-like logic exists in multiple places (`src/state/detector.rs:580-634`, `src/session/actor.rs:1440-1487`).
5. Why duplicate logic? No single typed classification contract is shared across state + activity pipelines.
- **Root Cause:** Semantic state is inferred from lossy terminal presentation streams, with duplicated heuristics across subsystems.
- **Root vs Symptom Handling:** Partially root-addressed (OSC 133 + liveness reconciliation), but still largely symptom-managed by heuristic hardening.
- **Evidence:** `src/state/detector.rs:124-219`, `src/state/detector.rs:330-359`, `src/state/detector.rs:580-634`, `src/session/actor.rs:903-942`, `src/session/actor.rs:1440-1487`.
- **Reasoning:** Repeated heuristic tuning and fallback layering indicate the driver is information quality + classifier fragmentation, not missing patterns.
- **Severity:** high
- **Confidence:** 0.86
- **So What:** Consolidate all prompt/activity classification into one shared classifier module with explicit confidence/cause codes; have both state transitions and meaningful-activity logic consume it.
- **Owner-ack?** no.

### §F3 — ScrollGuard exists because tmux redraw behavior and per-client rendering goals are structurally misaligned
- **Symptom:** Multi-client tmux scrolling can produce flicker/garbage unless redraw bursts are coalesced.
- **5-Whys:**
1. Why flicker? Other client scrolls emit redraw bursts into shared PTY output (`src/scroll/guard.rs:4-7`).
2. Why do bursts break rendering? They arrive as dense cursor-position streams that look like partial frames (`src/scroll/guard.rs:6-15`, `src/scroll/guard.rs:108-123`).
3. Why can’t swimmers ignore them cleanly? The actor consumes a raw byte stream and must preserve sequence integrity (`src/session/actor.rs:714-723`, `src/scroll/guard.rs:114-116`).
4. Why coalescing window/timers? There is no upstream per-client semantic redraw boundary, so timing heuristics approximate frame end (`src/scroll/guard.rs:21-23`, `src/scroll/guard.rs:94-106`, `src/scroll/guard.rs:145-151`).
5. Why is that inherent? tmux is shared-state terminal multiplexing; swimmers is downstream rendering over that substrate.
- **Root Cause:** Substrate impedance mismatch between shared tmux redraw streams and client-specific smooth rendering expectations.
- **Root vs Symptom Handling:** Symptom mitigation only; it reduces artifact frequency but cannot remove the underlying shared redraw behavior.
- **Evidence:** `src/scroll/guard.rs:1-15`, `src/scroll/guard.rs:79-131`, `src/session/actor.rs:714-723`.
- **Reasoning:** The guard’s design is explicitly heuristic and temporal, signaling unavoidable downstream compensation.
- **Severity:** medium
- **Confidence:** 0.92
- **So What:** Keep ScrollGuard, but treat it as a permanent compatibility layer; invest in better observability (coalesced burst counters, false-positive metrics) rather than chasing elimination.
- **Owner-ack?** yes — Confirmed Known Risk (`src/scroll/guard.rs:1-15`).

### §F4 — Flat-file persistence assumes single-process best-effort durability; corruption first breaks continuity, not live runtime
- **Symptom:** On corruption/read failure, persistence silently starts fresh; historical/stale continuity disappears first.
- **5-Whys:**
1. Why does continuity disappear? Corrupt registry load returns empty vector (`src/persistence/file_store.rs:191-203`).
2. Why is there no recovery path? Corruption handling chooses availability over rollback/journal replay (`src/persistence/file_store.rs:200-213`, `src/persistence/file_store.rs:297-305`).
3. Why is corruption still possible? Atomic rename is used, but no fsync on file/dir for durability boundaries (`src/persistence/file_store.rs:370-385`).
4. Why are concurrent writers risky? Locks are in-process only; no OS-level cross-process file lock (`src/persistence/file_store.rs:87-95`, `src/persistence/file_store.rs:167`, `src/persistence/file_store.rs:242`).
5. Why this model? Project values favor no infrastructure + simple local files.
- **Root Cause:** Persistence model is intentionally minimal and single-instance oriented, without crash-recovery or multi-process coordination semantics.
- **Root vs Symptom Handling:** Mostly symptom-level (atomic tmp+rename, in-process mutexes) but not root-level for durability/recovery across process boundaries.
- **Evidence:** `src/persistence/file_store.rs:87-95`, `src/persistence/file_store.rs:191-213`, `src/persistence/file_store.rs:370-385`, `src/session/supervisor.rs:1192-1200`.
- **Reasoning:** Existing safeguards protect common write races inside one process, but the absence of recovery primitives determines failure mode shape.
- **Severity:** high
- **Confidence:** 0.84
- **So What:** Add lightweight integrity envelope: checksum + last-known-good file fallback + fsync(file+parent) + advisory lockfile for single-writer enforcement.
- **Owner-ack?** no.

### §F5 — Dual thought backends persist as migration debt: “two knobs, one behavior”
- **Symptom:** `daemon` and `inproc` are both exposed, but `inproc` delegates to daemon bridge.
- **5-Whys:**
1. Why two backends? Config still parses both (`src/config.rs:19-31`, `src/config.rs:122-124`).
2. Why keep `inproc`? Compatibility promise during transition (`src/thought/loop_runner.rs:1-7`, `src/thought/loop_runner.rs:85-113`).
3. Why not remove now? Existing startup wiring/tests still preserve branch behavior (`src/main.rs:121-144`, `src/config.rs:148-153`).
4. Why preserve branch if behavior converged? To avoid breaking user envs/docs abruptly.
5. Why does this become structural risk? Ambiguous knobs create false operational choices and extra maintenance paths.
- **Root Cause:** Incomplete deprecation lifecycle after architectural migration to daemon boundary.
- **Root vs Symptom Handling:** Symptom push-around; compatibility branch avoids breakage but keeps conceptual complexity.
- **Evidence:** `src/thought/loop_runner.rs:1-7`, `src/thought/loop_runner.rs:85-124`, `src/main.rs:121-144`, `src/config.rs:19-31`.
- **Reasoning:** The code itself labels `inproc` as legacy while preserving it as a first-class config branch.
- **Severity:** medium
- **Confidence:** 0.94
- **So What:** Publish deprecation window and end-state date; then remove `Inproc` variant and env knob or make it explicit alias-with-warning only.
- **Owner-ack?** yes — Confirmed Known Risk (`src/main.rs:123`, `src/thought/loop_runner.rs:1-7`).

### §F6 — `personal-workflows` gate fixes packaging bleed at compile time, but runtime/docs still leak optional surfaces
- **Symptom:** Personal endpoints were gated for crates.io, but web UI and docs still present actions/routes that can 404 in default builds.
- **5-Whys:**
1. Why did personal endpoints bleed initially? Maintainer-local workflows lived in the core API surface (`src/host_actions.rs:11-15`, `src/api/dirs.rs:1-2`, `src/api/skills.rs:1-2`).
2. Why was a feature gate added? To keep crates.io build lean and avoid workstation-specific behavior by default (`Cargo.toml:64-68`, `CHANGELOG.md:70`).
3. Why do users still hit seams? Browser UI still invokes optional endpoints like `/v1/dirs` and commit launch actions (`src/web/app.js:917-940`, `src/web/app.js:1007-1018`, `src/web/rendered_surface.js:346-353`).
4. Why is that possible? No explicit capability handshake from server to UI for feature availability.
5. Why is there also doc leak? README endpoint table still lists gated routes unconditionally (`README.md:273-275`).
- **Root Cause:** Product boundary (core vs maintainer-local workflows) is encoded only at compile-time routing, not end-to-end contract/UI/docs.
- **Root vs Symptom Handling:** Partially root-addressed; compile-time gating solved crate payload bleed, but contract/UI/documentation seams remain.
- **Evidence:** `Cargo.toml:64-68`, `src/api/mod.rs:76-80`, `src/web/app.js:917-940`, `src/web/app.js:1007-1018`, `README.md:273-275`.
- **Reasoning:** A true boundary requires the same distinction across build, runtime capability signaling, and user-facing docs.
- **Severity:** medium
- **Confidence:** 0.9
- **So What:** Add `/v1/capabilities` and hide disabled actions/routes in web UI; annotate README endpoint table with feature flags.
- **Owner-ack?** yes — Confirmed Known Risk (`CHANGELOG.md:70`).

### §F7 — Prompt-shape logic is duplicated verbatim across subsystems, creating drift pressure
- **Symptom:** Prompt heuristics must stay synchronized in two independent functions.
- **5-Whys:**
1. Why duplicate? State detection and activity detection each need “prompt-like” judgments (`src/state/detector.rs:580-634`, `src/session/actor.rs:1440-1487`).
2. Why not shared? Each subsystem evolved locally for its own purpose.
3. Why is that risky? Heuristic tweaks can land in one path and not the other.
4. Why does that matter? State and activity timestamps diverge, producing inconsistent behavior and harder debugging (`src/session/actor.rs:765-773`, `src/session/actor.rs:1423-1437`).
5. Why recurring? No single authoritative heuristic API with tests enforcing parity.
- **Root Cause:** Missing shared abstraction for terminal prompt classification.
- **Root vs Symptom Handling:** Not root-addressed; current test coverage validates each copy separately rather than equivalence.
- **Evidence:** `src/state/detector.rs:580-634`, `src/session/actor.rs:1423-1487`, `src/session/actor.rs:2337-2343`.
- **Reasoning:** Structural duplication in heuristic-heavy code is a known churn amplifier and directly tied to classification brittleness.
- **Severity:** medium
- **Confidence:** 0.87
- **So What:** Extract one shared `prompt_like(line)` helper used by both modules; add parity tests that fail if behavior diverges.
- **Owner-ack?** no.

## Risks Identified
- **High / Medium likelihood:** Misclassification drift (idle/busy/attention) causing misleading fish state and stale activity semantics (`§F2`, `§F7`).
- **High / Low-Medium likelihood:** Persistence continuity loss after corruption/process contention; first visible issue is missing historical/stale session continuity on restart (`§F4`).
- **Medium / High likelihood:** Optional-feature seams causing runtime 404/error UX in default crates build (`§F6`).
- **Medium / Medium likelihood:** Scroll redraw artifacts under multi-client tmux workloads remain a recurring class even with coalescing (`§F3`).
- **Medium / Medium likelihood:** Continued boundary-hardening bursts near release milestones, reducing predictability and increasing regression risk (`§F1`).
- **Medium / Low likelihood:** Operational confusion from `inproc` backend knob that is functionally a compatibility alias (`§F5`).

## Recommendations
- **P0 (effort: medium, benefit: high):** Unify terminal-state classification into one shared module with explicit `cause` and confidence outputs; consume it in both `StateDetector` and actor meaningful-activity logic.
- **P1 (effort: medium, benefit: high):** Harden flat-file durability semantics: checksum + last-good fallback + fsync(file+parent) + advisory lockfile to enforce single writer across processes.
- **P2 (effort: low-medium, benefit: medium-high):** Add runtime capability discovery (`/v1/capabilities`) and make web actions/docs conditional on feature availability (`personal-workflows`, commit helpers, dirs/skills endpoints).
- **P3 (effort: low, benefit: medium):** Execute a dated deprecation plan for `SWIMMERS_THOUGHT_BACKEND=inproc`; collapse to one backend path after window closes.
- **P4 (effort: low, benefit: medium):** Institutionalize a release-boundary checklist (security bind/auth invariant, endpoint matrix, packaging assumptions, optional dependency checks) as CI gating.

## New Ideas and Extensions
- **Incremental:** Add metrics for classifier disagreement events (heuristic vs liveness correction) and ScrollGuard coalescing rates to target real hot spots.
- **Significant:** Offer an explicit shell-integration bootstrap command that standardizes OSC 133 markers for managed sessions, reducing fallback heuristics exposure.
- **Radical:** Introduce an append-only local event journal (still flat-file, no DB) for session/thought state replay, then materialize snapshots from it for stronger crash recovery semantics.

## Assumptions Ledger
- This analysis assumes typical single-owner usage, but allows occasional multi-process or remote/token deployments as documented trajectories.
- It assumes `tmux` behavior described in comments reflects real production behavior under multi-client attachments.
- It assumes feature-gated build (`personal-workflows` off) is the default user path for crates.io installs.
- It assumes persistence files can be externally affected (abrupt shutdowns, filesystem quirks, accidental dual process), not only ideal single-process runs.
- Project assumption questioned: “local-first” is enough boundary definition for release quality; findings suggest explicit feature/deployment contracts are still required.
- Project assumption questioned: atomic rename alone is sufficient persistence safety; it is necessary but not complete for durability/recovery semantics.

## Questions for the Project Owner
- Do you want to keep any user-visible reason for `SWIMMERS_THOUGHT_BACKEND=inproc`, or should it become a hard alias and then be removed?
- Is default crates.io UX expected to include directory browser + commit helper actions, or should those be explicitly hidden unless `personal-workflows` is enabled?
- Are you willing to enforce a single-process lock on the data dir, or do you intentionally support multiple swimmers processes sharing one persistence path?
- Would you accept a short-term “classifier reason code” in session summaries to debug misclassifications before any larger refactor?
- Should README endpoint tables reflect feature flags inline to prevent API expectation drift?

## Points of Uncertainty
- I did not run long-lived tmux multi-client stress tests in this pass; ScrollGuard conclusions are code- and comment-based.
- I did not verify behavior under network filesystems or abrupt power loss; persistence risk is inferred from write semantics.
- The exact frequency of 404 seams in browser UI depends on whether users rely on those specific actions in default builds.
- The practical user impact of `inproc` compatibility debt depends on how many existing environments still set that variable.

## Agreements and Tensions with Other Modes
- Likely agreement with **F7 (systems-thinking)** that cross-cutting couplings (state, persistence, snapshot paths) are the real risk multipliers.
- Likely agreement with **B1 (pattern-mining)** and **L2 (debiasing)** on duplicated heuristics and migration/compatibility drift as recurring patterns.
- Likely agreement with **F4 (FMEA)** and **A8 (edge-case)** on persistence recovery behavior and optional-endpoint seams.
- Likely agreement with **I4 (perspective-taking)** that release/install context exposes hidden assumptions not visible in maintainer-local workflows.
- Tension with **A1 (invariant verification):** A1 may judge several invariants as “holding” today, while F5 highlights deeper class-level drivers that still produce churn even when local invariants pass.
- Tension with **H2 (red-team):** H2 will likely rate some issues lower due loopback-first threat model; F5 still treats boundary codification debt as high leverage for future regressions.

## Confidence: 0.88
Calibration note: Confidence rises with targeted runtime telemetry (classifier disagreement counters, ScrollGuard burst stats) and restart-corruption drills. Confidence drops if production usage proves much narrower (strictly single-process, no browser usage, no optional-feature expectations), which would reduce practical impact of several findings.
