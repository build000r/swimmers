Using dueling-idea-wizards to produce one independent Swimmers multi-environment orchestration lane.

**20 Candidate Ideas**
1. Fleet Lens: one host/cwd/project/state/readiness filter model across TUI, thought rail, and web.
2. Remote Action Proxy: send input, dismiss attention, and same-target group input to `target::session`.
3. Launch Target Doctor: prove target health, auth, self-target skips, and path mappings before launch.
4. Remote-First Launch Sheet: mapped cwd preview, target defaulting, and explicit local override.
5. Display-Only Grouped Atlas: group by host/project/pwd without renaming, moving, or merging sessions.
6. Cross-Host Attention Inbox: ready remote sessions in a queue; keep tmux attention groups local.
7. SSH Fallback Sheet: copyable `ssh ... tmux attach` and safe launch commands for remote targets.
8. Canonical Repo Labels: derive display repo keys from path mappings across Mac/devbox/container roots.
9. Target-Scoped Batch Preview: dry-run a batch by target and reject mixed unmapped dirs early.
10. Remote Skills Handoff: make remote SBP/skills status explicit, with "query on target host" handling.
11. Session Provenance: show where a session was launched, mapped cwd, target, tool, and batch.
12. Degraded Remote Glance Encoding: stronger stale/degraded transport visuals in aquarium and Trogdor.
13. Environment Registry Pane: overlay-declared hosts, base URLs, defaults, auth status, mapped roots.
14. Passive c0/NTM Metadata Import: consume labels/status as metadata only; keep c0 external.
15. Devbox Capacity Advisory: surface NTM/load-guard status as advisory, not scheduling.
16. Host-Aware Published Selection: published session includes target identity and remote attach hint.
17. Remote Directory Inventory: optionally read remote `/v1/dirs` for target-scoped picker previews.
18. Work Queue Snapshots: one screen of "needs user / validate / dirty / commit-ready" by host.
19. Overlay Schema Linter: validate `agent_launch`, `groups`, and path mapping contract in config doctor.
20. FrankenTerm Boundary Hardening: keep it as a renderer, not the orchestration product surface.

**Best 5**

1. **Fleet Lens: Host x Project x Readiness**
- User/audience: The single operator juggling local Mac, `skillbox-portfolio-devbox`, Tailnet APIs, and container-root sessions.
- Why indispensable: It makes Swimmers the orientation layer: "what matters, where is it, and is it ready for me?" without reading `tmux ls`, c0 logs, or separate launchers.
- Architecture fit: `list_sessions_for_client` already merges local and remote sessions; remote IDs are namespaced; `ThoughtFilter`, Trogdor repo grouping, and `operator_pressure` already carry the state/readiness material.
- Touches: `src/api/remote_sessions.rs`, `src/types.rs`, `src/bin/swimmers_tui/app/session_entities.rs`, `src/bin/swimmers_tui/thoughts.rs`, `src/bin/swimmers_tui/render.rs`, `src/web/surface_model.js`, `src/web/rendered_surface.js`.
- Risks: Over-filtering can hide stale remote sessions; inferred repo keys can be wrong across mapped roots; visual density could fight the fish-bowl glance test.
- Proofs: Fixture with 3 local and 3 remote sessions across two targets; filter by host/project/readiness; stale remote sessions remain visible under a degraded bucket; Trogdor and thought rail show the same subset.

2. **Remote Action Proxy With Same-Target Guardrails**
- User/audience: Operators who can see a remote waiting agent locally and need to answer it immediately.
- Why indispensable: Visibility without action breaks the cockpit loop. Today remote list/context/diff/transcript paths exist, but input/group-input are local actor paths.
- Architecture fit: `remote_sessions.rs` already handles auth, URL building, denamespacing, remote GET proxies, and secret redaction. Extend that pattern to `POST /input`, attention dismiss, and group input.
- Touches: `src/api/remote_sessions.rs`, `src/api/sessions/core_routes.rs`, `src/api/sessions/group_input.rs`, `src/bin/swimmers_tui/api.rs`, `src/bin/swimmers_tui/in_process.rs`, web send-sheet paths.
- Risks: Mixed local/remote group input must be rejected; partial remote failures need honest per-session results; token errors must not leak secrets.
- Proofs: Mock remote server verifies `devbox::sess_1` forwards to `/v1/sessions/sess_1/input`; same-target remote batch succeeds; mixed-target batch returns a clear validation error; bearer tokens are redacted in error bodies.

3. **Launch Target Doctor + Mapped CWD Preview**
- User/audience: Remote-first operators burned by launching on Mac when they meant devbox, or by `/Users/b/...` versus `/srv/skillbox/...` mismatches.
- Why indispensable: Wrong-host launch is the core product failure for this use case. Swimmers already has overlay launch targets; it needs to make the target and mapped cwd impossible to miss.
- Architecture fit: `OverlayLaunchConfig`, `group_defaults`, and `map_cwd_for_target` are already present; health already has an overlay/remote-target ledger slot.
- Touches: `src/session/overlay.rs`, `src/api/remote_sessions.rs`, `src/api/health.rs`, `src/cli.rs`, `src/bin/swimmers_tui/picker.rs`, `src/bin/swimmers_tui/app/initial_request.rs`.
- Risks: Probes can slow startup; auth-missing and remote-down states must degrade without blocking local usage; false confidence if API is up but repo path is absent.
- Proofs: Overlay fixture with mapped and unmapped paths; picker disables or warns on unmapped remote launch; doctor reports target count, auth env presence, mapping coverage, self-target skip, and last poll error without printing secrets.

4. **Display-Only Grouped Atlas**
- User/audience: Operators who liked `c0 --group-by-pwd once` because it made remote work legible as a tree.
- Why indispensable: It imports the useful lesson from c0 without mutating tmux state. Grouping is presentation; sessions stay exactly where they are.
- Architecture fit: Thought rail already toggles pwd/batch grouping; Trogdor groups by repo; attention grouping already has repo-family logic. Make grouping a first-class display mode across aquarium/session rail/web atlas.
- Touches: `src/bin/swimmers_tui/thoughts.rs`, `src/bin/swimmers_tui/render.rs`, `src/api/service/attention_group.rs` for shared repo-key helpers, `src/web/rendered_surface.js`, `src/web/surface_model.js`.
- Risks: Users may mistake grouped display for session movement; inferred project labels may collapse unrelated repos; selection can jump if groups reorder too aggressively.
- Proofs: c0-style fixture proves grouping by pwd/project does not change session IDs, tmux names, batches, or cwd; toggling grouped/flat preserves selection; local and remote sessions under the same repo are grouped visually but retain host badges.

5. **Cross-Host Attention Inbox, Not Cross-Host tmux Group**
- User/audience: The operator who wants one queue of "agents waiting on me" across known environments.
- Why indispensable: This gives the daily operator workflow a home in Swimmers while respecting the non-goal: no cluster scheduler, no remote pane manager.
- Architecture fit: `attention_group` deliberately excludes remote sessions from native tmux layout. Keep that. Add a read-only inbox sourced from `list_sessions_for_client` and `operator_pressure`, with local open for local sessions and SSH/Tailnet hints for remote ones.
- Touches: `src/api/service/attention_group.rs`, `src/operator_pressure.rs`, `src/api/native.rs`, `src/bin/swimmers_tui/app.rs`, `src/bin/swimmers_tui/render.rs`, `src/web/rendered_surface.js`.
- Risks: Remote stale cache can create false urgency; SSH attach hints need explicit overlay metadata; users may expect native iTerm/Ghostty open to work for remote sessions.
- Proofs: Local attention group still excludes remote sessions; inbox includes remote awaiting-user/commit-ready sessions; degraded remote sessions sort lower and show stale age; clicking local opens native, clicking remote shows attach command or target URL only.

Candid ranking: build 3 and 4 first because they prevent wrong-host confusion quickly. Then 2 closes the action loop. Then 1 and 5 make the unified cockpit feel inevitable rather than just possible. Keep c0/NTM load guards external helpers for now; Swimmers should consume their outputs passively, not become their scheduler.

