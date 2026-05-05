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
3. Hover an agent glyph to freeze it and open the speed reader. It starts at `200 wpm`; use `-25`, `+25`, and `Pause`/`Read`.
4. Use the hover panel actions:
   - `send`: send one line to that session.
   - `batch`: send one line to ready sessions in the same existing batch.
   - `launch`: open the create-session sheet for the same repo.
   - `mmd`: open the Mermaid artifact sheet.
   - `commit`: launch the existing commit Codex flow when the shared pressure model marks the session commit-ready.
5. When an operator response resolves an awaiting-user swordsman, the atlas briefly keeps that swordsman visible as burnt and moves the dragon into a fire pose aimed at the same repo slot. This is the visual close of the loop; it does not create a new backend state.
6. In the Mermaid sheet, use plan-file tabs when the artifact reports `plan_files`; the web reads them through the existing `/v1/sessions/{id}/plan-file` API.

## Reuse Contract

Do not add separate Trogdor backend facts for agent intent, burnination state, commit readiness, or user-awaiting state. Add or fix facts in clawgs and the existing Swimmers session/thought plumbing, then let `src/operator_pressure.rs` derive presentation pressure from those facts.
