# Web Trogdor Mode

Trogdor mode is the web operator view for live Swimmers sessions. It keeps the backend vocabulary unchanged: sessions, repos, `SessionSummary`, `RestState`, `StateEvidence`, and clawgs `action_cues` remain the source of truth. The dragon/structure/villager language is presentation only.

## Map

- Repo structure: one working directory or repository with one or more active sessions.
- Villager/agent glyph: one `SessionSummary`.
- Burninate level: shared Rust `operator_pressure` score derived from existing session state and clawgs `action_cues`.
- Dragon sprite: tracked PNG frames under `assets/dragon/`, served by the web binary at `/assets/dragon/{pose}/{frame}`.
- `!`: awaiting user or attention.
- `$`: commit-ready.
- `v`: validation missing after edit.
- `d`: dirty-check missing.
- `x`: error.
- `a`: ordinary agent.

## Workflow

1. Open `make web` or `http://127.0.0.1:3210/`.
2. Use `atlas` in the rendered action rail to show or hide the Trogdor atlas while a terminal is selected.
3. Click an agent glyph to open the single-session terminal cockpit for that session.
4. Hover an agent glyph to freeze it and open the speed reader. It starts at `200 wpm`; use `-25`, `+25`, and `Pause`/`Read`.
5. Use the bottom composer in terminal focus mode to send one line without hiding terminal output.
6. Use the workbench panels for context:
   - `Turns`: user-submitted Claude/Codex turns only.
   - `Activity`: session timeline events for task, current action, and tool calls.
   - `Diffs`: structured file/hunk git summaries plus raw diff fallback.
   - `Logs`: post-turn JSONL records after the latest or selected user turn, with Raw fallback; pane-tail remains the fallback when transcript records are unavailable.
   - `Artifacts`: Mermaid and plan-file metadata from existing artifact endpoints.
   - `Skills`: passive Skillbox/SBP Skills discovery for the session cwd when `personal-workflows` and `sbp` are available.
7. Use the hover panel actions:
   - `send`: send one line to that session.
   - `batch`: send one line to ready sessions in the same existing batch.
   - `launch`: open the create-session sheet for the same repo.
   - `mmd`: open the Mermaid artifact sheet.
   - `commit`: launch the existing commit Grok flow when the shared pressure model marks the session commit-ready.
8. When an operator response resolves an awaiting-user swordsman, the atlas briefly keeps that swordsman visible as burnt and moves the dragon into a fire pose aimed at the same repo slot. This is the visual close of the loop; it does not create a new backend state.
9. In the Mermaid sheet, use plan-file tabs when the artifact reports `plan_files`; the web reads them through the existing `/v1/sessions/{id}/plan-file` API.

## Single-Session Cockpit

The cockpit is a terminal-first workbench, not a replacement for the live terminal. It reads `/v1/sessions/{id}/timeline` for ordered session events and pinned summaries, `/v1/sessions/{id}/agent-context` for user-only turn metadata, and `/v1/sessions/{id}/transcript` for JSONL records after the latest or selected user turn. The existing pane-tail, git-diff, Mermaid artifact, and plan-file endpoints remain concrete sources for expanded views. The Logs panel renders post-turn transcript summaries for scanability and keeps Raw JSONL as the fallback truth source; pane-tail is shown when transcript records are unavailable.

`/v1/sessions/{id}/git-diff` includes structured file and hunk summaries so the browser can render a useful diff overview without reparsing patches. The raw diff fields remain available for compatibility and for expanded views.

The Skills panel calls `/v1/sessions/{id}/skills?source=sbp` only when runtime `SWIMMERS_PERSONAL_WORKFLOWS=1` exposes that route; the `personal-workflows` Cargo feature only changes the default. The adapter runs a passive `sbp skills` lookup for the selected session cwd, reports unavailable states in the panel, and does not perform automatic skill hot-swap, sync, add, prune, or overlay mutation.

## Reuse Contract

Do not add separate Trogdor backend facts for agent intent, burnination state, commit readiness, user-awaiting state, workbench activity, diffs, logs, artifacts, or Skills. Add or fix facts in clawgs and the existing Swimmers session/thought/API plumbing, then let `src/operator_pressure.rs` and the single-session timeline derive presentation pressure and cockpit summaries from those facts.
