# Glance Test Fixtures

This document defines the deterministic 10-session fixture profile used to turn the Vision Glance Test into executable proof.

## Source Contract

- Product intent: `docs/VISION.md` Product Test.
- Data contract: `SessionSummary` in `src/types.rs`.
- TUI state mapping: `SpriteKind::from_session` in `src/bin/swimmers_tui/entity.rs`.
- TUI label mapping: `session_state_text` in `src/bin/swimmers_tui/render.rs`.
- Fixture manifest: `tests/fixtures/glance_state_coverage_10.json`.
- Current manifest guard: `cargo test --bin swimmers-tui fixture_manifest_matches_source_state_predicates`.
- Current first-frame proof: `bash ./scripts/test-glance-live.sh` or `make glance-smoke`.

## Profile

`glance_state_coverage_10` contains exactly 10 sessions. It covers busy compiling work, busy test work, idle drowsy, idle sleeping, awaiting-user attention, errored, exited, and stale/degraded busy state.

The test must distinguish these states from `state`, `rest_state`, `state_evidence`, `transport_health`, and `current_command`. It must not use `tmux_name` or `session_id` as the source of truth for the operator-visible state.

## Environment Prerequisites

The current harness is a simulation of ten safe tmux sessions through the TUI renderer. It requires `cargo` and `python3`; it does not require a running `swimmers` server and does not create tmux sessions.

## Non-Destructive Tmux Rule

If a later live-tmux runner is added, it must create sessions with a unique prefix such as `swimmers-glance-${RUN_ID}-NN` and must only kill sessions with that exact prefix during cleanup. It must not call broad tmux commands such as `tmux kill-server` or kill pre-existing sessions.

## Commands And Artifacts

- Manifest guard: `cargo test --bin swimmers-tui fixture_manifest_matches_source_state_predicates`
- First-frame proof: `bash ./scripts/test-glance-live.sh`
- Make target: `make glance-smoke`
- Artifact directory: `tests/artifacts/glance/`
- Required artifacts: `sessions.json`, `tui-frame.txt`, `state-observations.json`, and `native-open.json`
- Native handoff mode: simulated by default, with no real iTerm or Ghostty automation required unless a future runner explicitly opts in.
