# Edge-Case / Boundary-Condition Analysis (A8) — Analysis of swimmers

## Thesis
The edge-case lens shows that swimmers is robust on many correctness boundaries (loopback gate, UTC rest-state math, replay truncation signaling), but it still has several unbounded-input and display-width boundaries where behavior degrades abruptly: high session counts trigger quadratic collision work, Unicode-heavy labels drift from terminal cell reality, large redraw/diagram/directory workloads can spike latency, and native handoff has no timeout guardrails. These are not catastrophic security failures in this localhost-first tool, but they are exactly the class of “works in demos, gets rough under stress” risks that accumulate user friction.

## Top Findings
### §F1 — High Session Counts Degrade Nonlinearly and Collapse Layout Slots
- **Input:** `N` sessions where `N` exceeds viewport capacity (especially hundreds).
- **Expected:** graceful degradation (stable frame time, deterministic overflow handling).
- **Actual code path:** every tick calls pairwise collision checks across all entities (`src/bin/swimmers_tui/app.rs:1275-1282`, `1325-1352`); resting layout clamps overflowed rows to the last row (`src/bin/swimmers_tui/entity.rs:519-537`).
- **Observed gap:** computational cost grows ~O(n^2), and overflowed resting entities visually stack into the same terminal row band instead of distributing/virtualizing.
- **Evidence:** `resolve_collisions` nested loop in `src/bin/swimmers_tui/app.rs:1325-1352`; row clamp in `bottom_rest_origin`/`top_rest_origin` at `src/bin/swimmers_tui/entity.rs:521-537`.
- **Reasoning:** boundary-condition analysis stresses scaling boundaries; this is where animation logic that feels fine at 5-20 sessions degrades sharply at 100+.
- **Severity:** medium
- **Confidence:** 0.93
- **So What:** add an entity-count cutoff strategy tomorrow (e.g., disable collision resolution above threshold, or bucketed collision checks) and cap/rest-pack overflow behavior deterministically.
- **Owner-ack?** no

### §F2 — Session Name Boundaries (Emoji/RTL/Wide Chars) Mismatch Terminal Cell Geometry
- **Input:** tmux session names with emoji, CJK/wide chars, RTL text, or mixed-width glyphs.
- **Expected:** visible labels align with hitboxes/sprites and truncate by display cell width.
- **Actual code path:** requested names are only `trim()`-normalized (`src/session/supervisor.rs:1534-1543`), while rendering/truncation use `chars().count()` rather than terminal width (`src/bin/swimmers_tui/thoughts.rs:397-399`, `src/bin/swimmers_tui/render.rs:460-472`, `src/bin/swimmers_tui/terminal.rs:190-200`).
- **Observed gap:** display-width mismatch causes alignment/truncation artifacts for non-ASCII-width labels.
- **Evidence:** `display_width` char-count logic (`src/bin/swimmers_tui/thoughts.rs:397-399`), char-by-char draw (`src/bin/swimmers_tui/terminal.rs:190-200`), char-count truncation (`src/bin/swimmers_tui/render.rs:460-472`).
- **Reasoning:** boundary lens targets representational edges where logical “character count” diverges from terminal cell occupancy.
- **Severity:** medium
- **Confidence:** 0.9
- **So What:** switch to wcwidth-aware width/truncation helpers and reuse them in labels, hit testing, and mermaid overlay text.
- **Owner-ack?** no

### §F3 — Replay Ring Boundary Can Start Snapshots Mid-Sequence
- **Input:** replay capacity near bound with PTY chunks splitting UTF-8 or ANSI sequences across frames.
- **Expected:** snapshot boundaries preserve valid semantic text/control sequence framing.
- **Actual code path:** replay ring evicts whole frames only (`src/session/replay_ring.rs:39-43`), then `snapshot()` concatenates retained bytes and decodes lossy UTF-8 (`src/session/replay_ring.rs:117-126`).
- **Observed gap:** if older frame containing prefix bytes is evicted, snapshot may begin on continuation bytes / partial escape tails, producing replacement chars or malformed control fragments.
- **Evidence:** eviction loop `src/session/replay_ring.rs:39-43`; lossy decode `src/session/replay_ring.rs:125`; ring constructed from env-driven size at `src/session/actor.rs:335` and `src/config.rs:97-108,128-130`.
- **Reasoning:** ring buffers are classic boundary-sensitive structures; correctness often breaks at chunk boundaries, not only at average-case usage.
- **Severity:** low
- **Confidence:** 0.86
- **So What:** add boundary-aware snapshot sanitization (e.g., trim leading invalid UTF-8/escape fragments) and explicit tests for split-UTF8/split-ANSI at eviction edges.
- **Owner-ack?** no

### §F4 — Scroll Guard Has No Byte Cap During Coalescing Bursts
- **Input:** rapid continuous redraw bursts above cursor-position threshold.
- **Expected:** bounded coalescing memory under sustained burst traffic.
- **Actual code path:** coalesced chunks append to in-memory buffer with no size limit (`src/scroll/guard.rs:116-121`), with flush driven by deadlines/timers (`src/session/actor.rs:658-672`, `690-708`).
- **Observed gap:** if redraw traffic remains high, buffer can grow materially between flushes; no explicit safety cap exists.
- **Evidence:** unbounded `buffered.extend_from_slice(data)` in `src/scroll/guard.rs:116-117`; timer wiring in actor `src/session/actor.rs:658-672,690-708`.
- **Reasoning:** boundary analysis on burst behavior focuses on “what happens when rate stays high longer than intended window assumptions.”
- **Severity:** low
- **Confidence:** 0.84
- **So What:** add a max coalesced byte threshold (flush-early or bypass mode) and emit a metric when threshold is hit.
- **Owner-ack?** no

### §F5 — State Detector Drops Non-ASCII Visible Bytes and Can Linger in Error on Burst Mixes
- **Input:** ANSI-heavy/non-ASCII output, no-newline streams, or single chunk containing error-like text followed by busy output.
- **Expected:** robust classification independent of ASCII-only assumptions and mixed-event bursts.
- **Actual code path:** visible stream only keeps ASCII printable + whitespace controls (`src/state/detector.rs:425-445`); first matched error pattern sets Error and breaks (`167-180`), error clears on timer (`232-241`, `710-716`).
- **Observed gap:** Unicode-rich prompt/output can be invisible to heuristics; mixed error/busy chunk can bias toward temporary Error even when command activity continues.
- **Evidence:** byte filtering `src/state/detector.rs:425-445`; error-first branch `167-180`; timer clear `232-241,710-716`; prompt heuristic constraints `580-634`.
- **Reasoning:** this mode isolates classifier behavior at signal edges where tokenization assumptions fail.
- **Severity:** medium
- **Confidence:** 0.88
- **So What:** broaden visible-text normalization beyond ASCII and add burst-order tests covering `busy -> error -> busy` in one chunk.
- **Owner-ack?** no

### §F6 — Native Handoff Handles Platform Gating but Lacks Execution Timeouts
- **Input:** macOS with slow/hung `osascript`; iTerm absent; Ghostty unavailable; Linux hosts.
- **Expected:** quick failure with bounded latency and consistent capability reporting.
- **Actual code path:** non-macOS/non-loopback is gated (`src/native/mod.rs:97-111`), Ghostty availability checked via synchronous `ProcessCommand::output()` (`301-320`), open operations await `osascript` output with no timeout (`430-454`, `607-637`); iTerm has no explicit availability preflight (`68-71`, `127-130`).
- **Observed gap:** a slow/hung osascript call can stall request handling; iTerm “supported” can be optimistic until open fails.
- **Evidence:** support gate `src/native/mod.rs:81-130`; synchronous Ghostty probe `301-320`; async open without timeout `451-454`, `634-637`; API path `src/api/native.rs:108-111,157-178`.
- **Reasoning:** edge analysis emphasizes time-boundary failures (slow external dependencies) over nominal success paths.
- **Severity:** medium
- **Confidence:** 0.91
- **So What:** wrap native probes/open in explicit timeouts and add iTerm availability probe parity with Ghostty.
- **Owner-ack?** no

### §F7 — Thought-Config PUT Boundary Is Framework-Dependent for Malformed Bodies; Concurrent Writes Are Last-Write-Wins
- **Input:** invalid JSON, empty body, oversized body, concurrent PUT requests.
- **Expected:** explicit, consistent API error contract and predictable multi-writer semantics.
- **Actual code path:** handler requires `Json<ThoughtConfig>` extraction (`src/api/thought_config.rs:43-47`) then validation (`52-64`); persistence uses atomic rename (`src/persistence/file_store.rs:315-333,370-385`) with no version/ETag conflict control.
- **Observed gap:** malformed/empty JSON handling is extractor-driven (not explicit endpoint envelope), and concurrent writes resolve by timing (last writer wins).
- **Evidence:** route handler signature at `src/api/thought_config.rs:43-47`; normalize/validate `52-64`; save path `src/persistence/file_store.rs:315-333`; no body-limit/conditional-write policy in `src/api/mod.rs:66-87`.
- **Reasoning:** boundary checks target malformed input and write races where hidden framework defaults shape user-visible behavior.
- **Severity:** low
- **Confidence:** 0.74
- **So What:** add explicit rejection mapping tests for malformed payloads and consider optimistic concurrency token if multi-client config edits are expected.
- **Owner-ack?** no

### §F8 — Large Directory and Mermaid Inputs Lack Explicit Workload Caps
- **Input:** directories with thousands of entries; huge/malformed Mermaid source.
- **Expected:** bounded per-request/per-frame work with graceful truncation/pagination.
- **Actual code path:** `/v1/dirs` scans entries, per-entry metadata, and child directory checks without pagination (`src/api/dirs.rs:489-541`, `510-525`); Mermaid preparation parses/layouts/renders source during viewer render path (`src/bin/swimmers_tui/mermaid.rs:4300-4304`, called from `4733-4737`), with no source-size limit.
- **Observed gap:** heavy filesystem or diagram inputs can produce latency spikes and interactive jank.
- **Evidence:** dirs listing loops `src/api/dirs.rs:489-541`; per-entry child scan `510-517`; mermaid parse/layout/render `src/bin/swimmers_tui/mermaid.rs:4300-4304`; render error path `4733-4737`.
- **Reasoning:** boundary-mode analysis stresses unbounded cardinality/size, where “correct” code becomes slow code.
- **Severity:** medium
- **Confidence:** 0.89
- **So What:** add `limit/offset` (or cap) to `/v1/dirs` and a max Mermaid source size with an explicit “too large to render inline” fallback.
- **Owner-ack?** no

## Risks Identified
- **Session-count quadratic collision cost** — severity: medium, likelihood: medium-high.
- **Unicode display-width drift in labels/overlays** — severity: medium, likelihood: medium.
- **Replay snapshot starts on truncated byte/control boundaries** — severity: low, likelihood: medium.
- **Scroll coalescing memory spike under sustained redraws** — severity: low, likelihood: low-medium.
- **State misclassification on non-ASCII/burst-mixed output** — severity: medium, likelihood: medium.
- **Native handoff latency stalls from unbounded osascript waits** — severity: medium, likelihood: medium.
- **Thought-config malformed-body contract inconsistency** — severity: low, likelihood: medium.
- **Directory/Mermaid heavy input latency** — severity: medium, likelihood: medium.
- **Env/auth edge papercuts (probed):** `PORT=99999` silently falls back to default (`src/config.rs:72-77`), whitespace-only env values pass non-empty check (`src/config.rs:88-95`), whitespace-only auth token can be configured if set in env (`src/config.rs:115-118`) then matched literally in token mode (`src/auth/mod.rs:108-120`). Severity: low, likelihood: low.
- **Timezone/DST boundary (probed):** rest-state timing is UTC duration-based (`src/types.rs:549-569`), so DST/midnight boundary is mostly handled. Severity: low, likelihood: low.

## Recommendations
1. **P0** — Add bounded execution timeouts around all `osascript` invocations and preflight iTerm availability. Effort: medium. Expected benefit: removes request-stall class and improves native UX predictability.
2. **P1** — Replace char-count width logic with wcwidth-aware helpers and apply uniformly to labels, Mermaid overlays, and hit calculations. Effort: medium. Expected benefit: fixes Unicode/RTL/wide-character visual correctness.
3. **P2** — Add adaptive load-shedding for high entity counts (collision throttling/bucketing + overflow policy). Effort: medium-high. Expected benefit: stable frame time at large N.
4. **P3** — Introduce explicit caps/pagination for `/v1/dirs` and max-size guardrail for Mermaid inline rendering. Effort: low-medium. Expected benefit: prevents avoidable latency spikes.
5. **P4** — Harden boundary tests: split UTF-8/ANSI replay truncation, mixed burst state transitions, malformed thought-config extractor rejections, and sustained scroll coalescing pressure. Effort: medium. Expected benefit: regression-proofing exactly where edge failures concentrate.

## New Ideas and Extensions
- **Incremental:** add an “edge diagnostics” endpoint exposing replay window start, coalescer buffer bytes, and classifier recent transitions.
- **Significant:** dynamic “performance profile mode” in TUI that auto-switches to lower-cost animation/collision behavior when session count or frame cost crosses threshold.
- **Radical:** optional off-main-thread Mermaid preparation cache keyed by artifact hash + viewport class to prevent render-path stalls while preserving tmux-first identity.

## Assumptions Ledger
- Assumed high session counts (100+) are realistic for this user base (AI-agent-heavy tmux workflows).
- Assumed terminal rendering correctness for Unicode is desired, not ASCII-only by design.
- Assumed native handoff calls are on latency-sensitive request paths where blocking matters.
- Assumed multi-client thought-config writes are possible (API exposed under token mode) even if uncommon.
- Project assumption challenged: “short coalescing window means bounded memory” is not guaranteed without a byte cap.

## Questions for the Project Owner
- Do you want to officially support full Unicode display-width correctness in TUI labels, or treat ASCII as the designed baseline?
- For native handoff, is a hard timeout (e.g., 1-2s) acceptable even if it occasionally aborts slow app launches?
- Should `/v1/dirs` prioritize completeness or responsiveness when a directory contains thousands of entries?
- Do you expect concurrent thought-config writers (multiple clients), or is last-write-wins acceptable by policy?
- At what session count should swimmers intentionally degrade animation fidelity to preserve interactivity?

## Points of Uncertainty
- Exact practical impact of replay split-boundary artifacts depends on PTY chunking patterns in real workloads.
- Axum extractor rejection payload shape for malformed thought-config bodies was inferred from handler signatures rather than live HTTP capture in this pass.
- Magnitude of scroll coalescing memory growth depends on redraw burst rate and chunk size distribution.
- iTerm absent behavior may vary by AppleScript/runtime environment; capability mismatch is inferred from code path asymmetry.

## Agreements and Tensions with Other Modes
- **Likely agreement with Performance mode:** O(n^2) collisions and unbounded workload inputs are the main throughput risks.
- **Likely agreement with UX mode:** Unicode-width mismatches and large-input jank are user-visible quality cliffs.
- **Likely agreement with Reliability mode:** missing timeout boundaries in native handoff are operationally brittle.
- **Likely tension with Security mode:** many findings are availability/quality edges, not high-severity exploit surfaces in loopback-first deployment.
- **Likely tension with Minimalism mode:** adding caps/pagination/timeout controls increases code complexity, but targets high-leverage boundaries.

## Confidence: 0.86
Calibration note: confidence rises with targeted boundary tests (high-N TUI load, malformed thought-config HTTP requests, Unicode-width golden renders, forced slow `osascript`). Confidence drops if runtime telemetry shows these boundaries are rarely hit in real usage patterns.
