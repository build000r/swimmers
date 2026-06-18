# Swimmers Multi-Environment Cockpit Duel

Date: 2026-06-18

Scope: decide whether Swimmers should become the primary operator surface for local, SSH, devbox, and configured remote `swimmers_api` environments, and turn the strongest direction into implementation work.

## Inputs

- Cass/memory evidence: repeated local-vs-devbox-vs-container confusion, `c0 --group-by-pwd once` success, remote-first `c0` defaults, and prior Swimmers devl/devbox launcher work.
- Repo evidence: `README.md`, `docs/VISION.md`, remote session polling, launch target mappings, TUI picker launch targets, Trogdor repo atlas, and local-only attention group behavior.
- Agent lanes:
  - `WIZARD_IDEAS_GROK.md`: completed.
  - `WIZARD_IDEAS_COD.md`: completed.
  - `WIZARD_IDEAS_CC.md`: abandoned after a bounded wait with no artifact.

## Verdict

Swimmers is closer than FrankenTerm to the product the operator needs. FrankenTerm can remain a terminal-rendering or pane-hypervisor dependency, but the confusing part is not terminal rendering. The confusing part is orientation and action across explicitly configured environments: which host, which cwd, which repo, which backend, which agent needs input, and where a new session will launch.

The product should be a configured multi-environment cockpit:

1. See all known local and remote sessions through one environment lens.
2. Trust target health and path mappings before launching.
3. Launch deliberately with remote-first defaults and explicit local overrides.
4. Act on remote sessions through same-target proxy paths.
5. Group by host/project/pwd as display only, borrowing the useful `c0 --group-by-pwd` lesson without mutating sessions.

Do not turn Swimmers into a general SSH fleet manager, arbitrary cluster scheduler, or NTM replacement. Keep remote scope to overlay-declared targets and known `swimmers_api` backends.

## Scorecard

Scores are orchestrator scores over the two completed independent idea packets, weighted for user pain, repo fit, testability, and VISION.md alignment.

| Rank | Idea | Score | Why |
|---:|---|---:|---|
| 1 | Environment Health Strip + Path Mapping Doctor | 950 | Directly prevents wrong-host and wrong-root launches; already has overlay and remote health surfaces. |
| 2 | Remote-First Launch Guardrails | 930 | Matches the `c0` remote-first lesson and turns target choice into an unavoidable launch affordance. |
| 3 | Fleet Lens / Environment Registry | 910 | Creates the shared host/project/state/readiness vocabulary needed by TUI and web. |
| 4 | Remote Write Proxy with same-target guardrails | 890 | Closes the observe-then-act loop for remote waiting sessions. |
| 5 | Display-Only Grouped Atlas / Aquarium | 875 | Imports `group-by-pwd` legibility while preserving tmux/session identity. |
| 6 | Cross-Host Attention Inbox | 850 | Provides one waiting queue without violating local-only tmux attention group boundaries. |
| 7 | Canonical Repo Labels | 820 | Necessary glue for mapped roots across Mac, devbox, and container paths. |
| 8 | SSH / Native Handoff Hints | 790 | Valuable fallback, but must stay copyable/explicit rather than an SSH client. |
| 9 | Passive c0 / NTM metadata import | 760 | Useful as badges and advisory metadata; dangerous if it becomes orchestration. |
| 10 | Environment Presets / saved lenses | 720 | Good workflow polish after the core model is stable. |

## Design Contract

- Environment metadata is first-class: target id, host label, backend mode, base URL, auth status, cwd mapping status, last poll, and stale/degraded state.
- Session metadata carries provenance: local vs remote, target id, original remote session id, canonical repo label, remote cwd, local mapped cwd where known, launch source, and batch/group hints.
- Grouping is view state, not session state. It must not rename, move, merge, or retarget tmux sessions.
- Cross-host action is explicit and guarded. Same-target remote writes can be proxied. Mixed local/remote or mixed-target group sends must fail with a precise validation error.
- Attention groups remain local tmux layouts. A cross-host attention inbox can aggregate waiting sessions, but remote open actions should show target URLs or SSH attach hints unless a real remote API action exists.
- c0 and NTM stay external. Swimmers can consume labels/status as passive metadata and mirror useful grouping/default-host behavior.

## Implementation Order

1. Environment inventory and canonical session provenance.
2. Target health, config doctor, and path mapping proof.
3. Remote-first launch guardrails in TUI and web.
4. Remote write proxy with same-target group validation.
5. Fleet lens filters across API, TUI, thoughts, and web.
6. Display-only host/project/pwd grouping.
7. Cross-host attention inbox and handoff hints.
8. Passive c0/NTM metadata badges and capacity advisory.
9. Saved lenses and workflow polish.
10. End-to-end fixture and smoke proof package.

## Beads Graph

Created with `br` on 2026-06-18 and synced to `.beads/issues.jsonl`.

Epic:

- `swimmers-multi-env-cockpit-1pmw`: configured multi-environment Swimmers cockpit.

Ready entry point:

- `swimmers-multi-env-foundation-p4q4`: environment inventory and session provenance.

Implementation slices:

- `swimmers-canonical-repo-cwd-labels-s89j`: canonical repo and cwd labels across local, remote, and container roots.
- `swimmers-target-health-path-doctor-tyou`: target health strip and path mapping doctor.
- `swimmers-remote-first-launch-guardrails-lgwh`: remote-first launch guardrails and mapped cwd preview.
- `swimmers-remote-write-proxy-brn9`: remote write proxy with same-target guardrails.
- `swimmers-fleet-lens-filters-tw50`: fleet lens filters across API, TUI, thought rail, and web.
- `swimmers-display-only-grouped-atlas-as51`: display-only host/project/pwd grouping for aquarium, thought rail, and Trogdor.
- `swimmers-cross-host-attention-inbox-iiut`: cross-host attention inbox without cross-host tmux merging.
- `swimmers-remote-handoff-backend-mode-jkek`: remote handoff hints and explicit backend-mode indicator.
- `swimmers-passive-c0-ntm-metadata-xg59`: passive c0, NTM, SBP, and capacity advisory metadata.
- `swimmers-environment-presets-dir-inventory-kndl`: environment presets, saved lenses, and target-scoped directory inventory.
- `swimmers-multi-env-fixtures-proof-af5q`: multi-environment fixtures, docs, smoke proof, and release checklist.

Graph validation:

- `br dep cycles --json`: no cycles.
- `br lint --json`: new multi-env graph clean; remaining warnings are pre-existing Wave22 beads.
- `br ready --json`: `swimmers-multi-env-cockpit-1pmw` and `swimmers-multi-env-foundation-p4q4` are ready alongside pre-existing Wave22 work.
- `br sync --status`: in sync after JSONL export.

## Evidence Notes

- `src/api/remote_sessions.rs` already namespaces remote IDs, maps cwd, polls remote sessions, and proxies read-heavy artifacts.
- `src/session/overlay.rs` already carries `agent_launch`, target defaults, and path mappings, but health currently reports target counts without a real probe.
- `src/bin/swimmers_tui/picker.rs` already has launch target toggles and batch launch wiring.
- `src/api/service/attention_group.rs` deliberately excludes remote sessions from native attention groups; the new inbox should preserve that boundary.
- `src/web/rendered_surface.js` already has Trogdor repo grouping and operator pressure concepts that can become environment-aware.
