# Trogdor Web Workgraph

Status as of 2026-05-08.

## Complete

- `WG-01` Carry clawgs `action_cues` through Swimmers session/thought persistence and protocol.
- `WG-02` Derive operator pressure in shared Rust code at `src/operator_pressure.rs`.
- `WG-03` Expose `/v1/operator-pressure` for web clients.
- `WG-04` Render repo structures, agent glyphs, burninate banner, pressure rows, and hover speed reader in the web surface.
- `WG-05` Keep atlas available after terminal selection through the `atlas` toggle.
- `WG-06` Wire hover-panel actions to existing session input, group input, create session, Mermaid, plan-file, and commit flows.
- `WG-07` Harden live web input and web smoke terminal replies.
- `WG-08` Document the operator workflow and reuse contract in `docs/TROGDOR_WEB.md`.
- `WG-09` Use tracked dragon PNG frames for the walking/fire atlas dragon and render a visible flame/smoke burn state for resolved swordsmen.
- `WG-10` Add the V3 single-session cockpit spine: `/timeline`, structured git diff summaries, passive Skillbox/SBP Skills discovery, and timeline-backed browser Activity, Diffs, Logs, Artifacts, and Skills panels.
- `WG-11` Extend the live workbench smoke to capture desktop and mobile screenshots and assert no overlap between cockpit panels, terminal controls, Trogdor back control, and the bottom composer.

## Verification

- `cargo check --all-targets`
- `cargo test --lib`
- `cargo test --test metamorphic`
- `node --check src/web/app.js`
- `node --test src/web/rendered_surface.test.mjs src/web/input_support.test.mjs src/web/app_behavior.test.mjs`
- `make web-smoke`
- `scripts/test-web-workbench.sh`

## Reuse Contract

Trogdor presentation must continue to consume existing backend facts. The single-session cockpit can summarize timeline, diff, log, artifact, and Skills data, but it must not invent separate Trogdor-only state for agent intent, backend facts, commit readiness, user-awaiting state, or skill policy.

## Deferred Only With New Scope

- Native voice dictation parity. The web flow supports typed send/create operations, but browser microphone capture is not part of the current web-only Trogdor scope.
- Automatic skill hot-swap. The shipped Skills panel is passive discovery only; any future add/sync/prune/hot-swap behavior needs an explicit operator action and a separate proof gate.
- A broader browser automation suite beyond the focused live workbench smoke. Current coverage is unit-level rendered-surface tests, behavior tests, live terminal smoke, and the desktop/mobile workbench screenshot smoke.
