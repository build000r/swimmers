# Dueling Idea Wizards Report: Swimmers Web Reader

**Topic:** Make the web view the best place to read comprehensive agent output — including thinking traces — with hover + speed-reader affordances and click-in-to-terminal as the deep-dive path. Side goal: the web terminal view needs a significant overall UX improvement.

## Executive Summary

Three models (Claude Code, Codex, Grok) independently generated 30→5 UX ideas each, then adversarially cross-scored all 15 ideas (0–1000) and reacted to each other's scores. Convergence was unusually strong: all three independently proposed a reading-first layout, first-class thinking traces, hover preview + real speed reader, terminal as context-anchored deep-dive, and a fleet reading inbox. After the reveal, all three revised rankings agree on the same priority order: **(0) fix the backend thinking-capture gap, (1) build a source-aware reader block model, (2) ship a staged DOM reader mode, (3) wire the three-tier hover → read → terminal ladder, (4) upgrade the speed reader, (5) defer the fleet inbox.** Two ideas were killed or heavily rescoped by adversarial pressure, including one whose core mechanism the originator conceded was built on a false premise.

## Methodology

- Agents: Claude Code (NTM pane, Opus), OpenAI Codex (NTM pane, gpt-5.5 xhigh), Grok (headless sidecar via `--prompt-file`, stdout captured by the orchestrator).
- Mode: `ux`, focus = web view as agent-output reading surface. 30 ideas generated per agent, winnowed to top 5.
- Phases: study → independent ideation → adversarial cross-scoring (6 score files) → reveal with reactions → synthesis.
- Artifacts: `WIZARD_IDEAS_{CC,COD,GROK}.md`, `WIZARD_SCORES_{X}_ON_{Y}.md` (6), `WIZARD_REACTIONS_{CC,COD,GROK}.md`.

## The Unanimous #0: Thinking Traces Are Silently Dropped (backend fix)

Claude found it, Grok verified it and called it the "highest-leverage gap in my entire list," and Codex confirmed the JS-side half. In `src/thought/context.rs`, `capture_claude_assistant_block` handles `"tool_use"` and `"text"` content blocks but drops `"thinking"` at the `_ => {}` fallback. On the JS side, `workbench_records.js` can extract thinking from raw JSON but `compactRecordFields` explicitly skips the `thinking` key and `transcriptRecordDisplayKind` has no thinking category. **Every frontend reading idea hits this ceiling.** The consensus fix: emit thinking blocks as first-class transcript records (`kind: "thinking"`, watch the 4000-char `raw` truncation), plus a distinct display kind in the log lens. This was not in anyone's original top 5 — it emerged from adversarial pressure, which is the duel working as intended.

## Consensus Winners (scored 700+ by all judges)

### 1. Thinking Trace Renderer with Inline Speed-Reader (origin: CC #2) — avg 820
Codex 885 · Grok 756. Post-reveal, Claude moved it to its own #1. First-class collapsible thinking sections (collapsed preview + word count → hover card → expanded prose → RSVP speed-read with a 3-line context window). Requires the #0 backend fix. Ships independently of any layout change — Grok's decisive argument: "highest-value signal at the lowest incremental cost; should ship before or without the layout flip."

### 2. Reading-First Session Cockpit (origin: GROK #1) — avg 787
Claude 770 · Codex 805. The web view's default single-session surface becomes a full-width reading column (turns, thinking, logs, diffs as first-class panels); the terminal becomes an explicit "open terminal" deep-dive. Shared caveat from both judges, accepted by Grok: the container without a content model is "a bigger pile of truncated JSONL." Codex's delivery correction, accepted by Claude for its own similar idea: ship as a **staged reader mode** (route/toggle), not a default-flip rewrite of the canvas-first architecture.

### Post-reveal consensus addition: Source-Aware Reader Block Model (origin: COD #1 + GROK #3, revised)
Pre-reveal scores straddled 700 (CC 680/GROK 742 for Codex's feed; CC 620/COD 875 for Grok's stream), but after the reveal **all three models ranked this family #1 or #2**. The critical revision (Codex's own concession): do NOT build "one chronological feed" — thought snapshots are 15-second summary windows, transcript records are per-tool-call events, pane-tail is continuous; a naive merge "looks precise while actually being heuristic." Instead: typed reader blocks with explicit provenance labels (`transcript_record` ordered by byte offset, `thought_snapshot` labeled as latest-state summary, `timeline_summary` as synthetic, `terminal_tail` as raw evidence). `workbench_refresh.js` already parallel-fetches every needed source.

## Contested Ideas (large judge gaps — resolved by the reveal)

| Idea | Scores | Resolution |
|---|---|---|
| Unified Output Stream (GROK #3) | CC 620 vs COD 875 | Grok sided with Codex on value ("content unification is the keystone") but accepted Claude's caps: volume (240-record page cap), heuristic interleaving. Landed as the block-model consensus above. |
| Three-Tier Drill-Down (GROK #4) | CC 600 vs COD 820 | Both right: Codex on product value as the low-risk staging plan; Claude that it's an orchestration layer, not standalone. Adopted as the navigation contract for the roadmap. |
| Cross-Session Reading Inbox (GROK #5) | CC 530 vs COD 790 | Claude's localStorage-watermark critique held (per-device divergence for Tailscale operators); Codex's long-term value claim also held. Verdict: build later, with server-side read cursors. |
| Structured Turn Timeline (CC #4) | COD 850 vs GROK 648 | Grok's rendering-stack critique held (no markdown/highlighting infra, vanilla ES modules, no bundler); Claude self-revised to ~720 and noted Grok wrongly scored it against today's 360px sidebar. |
| Reading-First Layout Flip (CC #1) | COD 820 vs GROK 612 | Claude conceded Grok was "closer to right" on scope (canvas rail, HUD hit-testing, coupled state machines — a parallel rendering path, not CSS). Folded into the staged reader mode. |

## Killed or Heavily Rescoped

- **Terminal byte-offset anchoring (CC #5 mechanism, avg 527):** Claude verified its own claim was false — transcript `byte_start`/`byte_end` are JSONL file offsets; `ReplayRing` frames store only `seq` + bytes, no timestamps, no correlation. "I conflated two unrelated address spaces." Rescoped to: open terminal with a contextual breadcrumb ("From: Edit app.js, Turn 3") + "back to reader."
- **Contextual Evidence Panels (COD #5, avg 461):** lowest-scored idea in the duel; Codex conceded it is "credibility glue, not a headline feature" — a per-block proof-drawer pattern to apply after reader blocks exist.
- **Semantic Lenses as a novel subsystem (COD #3, avg 612):** Codex conceded `workbench_log_lens.js` already ships most of the taxonomy; reframed as "promote and harden the existing lens model, add Thinking as a first-class category."
- **Multi-Agent Reading Queue (COD #4, avg 544):** all three agree it's premature; sequence after single-session reading works.

## Score Matrix

| Idea | Origin | Self-Rank | CC | COD | GROK | Avg | Verdict |
|---|---|---|---|---|---|---|---|
| Thinking Trace Renderer + Speed-Reader | CC | 2 → 1 | — | 885 | 756 | 820 | **WIN** |
| Reading-First Session Cockpit | GROK | 1 → 3 | 770 | 805 | — | 787 | **WIN** (as staged reader mode) |
| Structured Turn Timeline | CC | 4 → 3 | — | 850 | 648 | 749 | Strong; fold into block model |
| Unified Agent Output Stream | GROK | 3 → 1 | 620 | 875 | — | 748 | **WIN post-reveal** (as block model) |
| Reading-First Layout Flip | CC | 1 → 2 | — | 820 | 612 | 716 | Merged into cockpit/reader mode |
| Agent Reader Feed + Hover Speed-Read | COD | 1 → 1 | 680 | — | 742 | 711 | Merged into block model |
| Three-Tier Drill-Down | GROK | 4 → 2 | 600 | 820 | — | 710 | Adopted as navigation contract |
| Hover Preview Cards | CC | 3 → 4 | — | 765 | 571 | 668 | Depends on DOM session list + #0 |
| Cross-Session Reading Inbox | GROK | 5 → 4 | 530 | 790 | — | 660 | Later; needs server-side cursors |
| Chunk-Based RSVP Speed Reader | GROK | 2 → 5 | 560 | 735 | — | 648 | Accelerator; after block model |
| Terminal Click-In Investigator | COD | 2 → 3 | 590 | — | 658 | 624 | Fold into deep-dive tier |
| Semantic Output Lenses | COD | 3 → 2 | 610 | — | 614 | 612 | Reframed: harden existing lens |
| Multi-Agent Reading Queue | COD | 4 → 5 | 520 | — | 568 | 544 | Premature |
| Terminal Context-Anchored Deep Dive | CC | 5 → 5 | — | 615 | 438 | 527 | Mechanism killed; rescope to breadcrumb |
| Contextual Evidence Panels | COD | 5 → 4 | 440 | — | 481 | 461 | **KILLED** as standalone |

## Meta-Analysis

- **Codex scores like a product architect** (range 615–885): rewards strategic correctness, forgiving on implementation. Its own ideas were the most vision-level and scored lowest with others.
- **Claude scores like a skeptical implementer** (range 440–680): penalizes unverified mechanisms and hidden dependencies. Its harshness was mostly vindicated (anchoring mechanism, localStorage watermarks, hover-on-canvas dependency) but over-penalized composable ideas by scoring them against today's layout.
- **Grok scores like a code archaeologist** (range 438–756): cited exact selectors, CSS values, and functions; corrected Claude's own factual error (the speed reader consumes full `clawgText`, not the 110-char rail label). Its idea list was judged the strongest-grounded.
- **Adversarial pressure produced the single best finding** (the `_ => {}` thinking-capture gap as priority #0) and killed a confidently-stated false mechanism — both things a single-model ideation pass would have missed.

## Recommended Next Steps

1. **Backend: capture thinking blocks** in `src/thought/context.rs` as first-class transcript records (`kind: "thinking"`), fix `compactRecordFields`/`transcriptRecordDisplayKind` suppression, handle >4000-char truncation. Small, surgical, unblocks everything.
2. **Reader block model**: typed, provenance-labeled blocks (thinking, turn, command, output, diff, artifact, snapshot) promoted from `workbench_log_lens.js` — no fake unified chronology.
3. **Staged Reader Mode**: a route/toggle that renders a DOM-first reading column (70–80% width, DOM session list) without touching the canvas terminal path.
4. **Three-tier navigation contract**: hover preview card → reader → terminal attach with breadcrumb and "back to reader"; drop byte-offset anchoring.
5. **Chunk-based RSVP speed reader** over reader blocks (hover affordance, 3-line context window, WPM persistence) — after 1–3 give it real source text.
6. **Later: fleet reading inbox** with server-exposed read cursors (not localStorage-only watermarks).
