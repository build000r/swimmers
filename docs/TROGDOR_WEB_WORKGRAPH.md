# Trogdor Web Workgraph

Status as of 2026-05-03.

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

## Verification

- `cargo check --all-targets`
- `cargo test --lib`
- `cargo test --test metamorphic`
- `node --check src/web/app.js`
- `node --test src/web/rendered_surface.test.mjs src/web/input_support.test.mjs src/web/app_behavior.test.mjs`
- `make web-smoke`

## Deferred Only With New Scope

- Native voice dictation parity. The web flow supports typed send/create operations, but browser microphone capture is not part of the current web-only Trogdor scope.
- A full browser automation screenshot suite. Current coverage is unit-level rendered-surface tests plus live terminal smoke.
