Reading the specified docs and code to ground ideas in the current Swimmers architecture.
# Swimmers Multi-Environment Orchestration — Lane Ideas

**Scope:** Configured operator cockpit for local Mac + known Tailnet/`swimmers_api` targets. Not a cluster scheduler, not FrankenTerm-as-orchestrator. Build on overlay-declared targets, namespaced remote sessions (`target::sess_id`), path mappings, operator pressure, thought-rail pwd/batch grouping, and Trogdor repo atlas.

---

## 20 Pragmatic but Ambitious Ideas

| # | Idea | One-line pitch |
|---|------|----------------|
| 1 | **Fleet Lens** | Unified host / project / state / readiness filters across TUI aquarium, thought rail, and web session rail |
| 2 | **Remote Write Proxy** | Forward `POST /input`, `group-input`, and dismiss-attention for `target::` sessions to remote `swimmers_api` |
| 3 | **Environment Health Strip** | Active probe of configured targets; surface reachability, latency, last poll in header/TUI/`/health` |
| 4 | **Group-by-Project Aquarium** | Display-only pwd/repo clustering (c0 `--group-by-pwd` lesson); fish cluster spatially, never merge sessions |
| 5 | **Remote-First Launch Guardrails** | Overlay `group_defaults` preselect remote; local spawn requires explicit toggle + unmapped-path warnings |
| 6 | **Path Mapping Doctor** | `swimmers config doctor` validates every picker/repo-search root maps to each configured target |
| 7 | **Host-Aware Operator Pressure** | Extend `/v1/operator-pressure` and Trogdor atlas to group by `host × repo`, not repo alone |
| 8 | **Cross-Host Attention Backlog** | Keep local-only tmux attention group; add read-only cross-host “waiting queue” panel from remote polls |
| 9 | **Launch Target Autopick per cwd** | Picker resolves best target from overlay `cwd_match`, `group_defaults`, and longest `path_mappings` match |
| 10 | **Environment Presets (Lenses)** | Persist named filter combos: “Mac only”, “devbox only”, “swimmers repos”, “needs input” |
| 11 | **Remote Native Handoff Hints** | For remote sessions: copyable `ssh … tmux attach` / Tailnet URL instead of iTerm/Ghostty (external SSH, not built-in client) |
| 12 | **Container Root Labels** | Normalize display cwd (`/workspace/...` → `swimmers`) via mappings + `repo_key` logic already in attention groups |
| 13 | **NTM / Wave Session Badges** | Detect wave naming patterns; badge metadata only — **NTM orchestration stays external** |
| 14 | **Multi-Host Batch Launch** | Batch create with per-entry target from overlay `group_defaults` when launching a virtual group |
| 15 | **Thought Rail Host+Pwd Grouping** | Third grouping axis: `host → pwd → sessions` alongside existing pwd/batch toggle |
| 16 | **Degraded Remote Glance Encoding** | Stronger aquarium/Trogdor visuals for `remote_poll_degraded` / stale cache (transport already exists) |
| 17 | **Target-Scoped Directory Picker** | When launch target ≠ local, picker shows remote-mapped cwd tree preview (read-only inventory via remote `/v1/dirs` if exposed) |
| 18 | **`swimmers env` Thin CLI** | `env list`, `env probe`, `env attach-hint` — discovery/diagnostics only; **not** a second orchestrator |
| 19 | **Cross-Host Group Input (same host)** | Group-input across multiple ready sessions on *one* remote target; reject mixed-target batches with clear error |
| 20 | **Explicit Backend Mode Indicator** | Always show whether TUI is embedded-local, `SWIMMERS_TUI_URL` single-backend, or federated local+overlay poll |

**Keep external (candid):**
- **c0** — labeling/display workflows; Swimmers consumes grouping semantics, does not reimplement c0.
- **skillbox-config overlays** — source of truth for targets, mappings, group defaults.
- **SSH transport** — attach hints and copy-paste fallbacks only; no arbitrary SSH fleet management.
- **FrankenTerm** — terminal renderer for web; not the multi-env control plane.
- **NTM / cluster scheduling** — badge and filter only.

---

## Top 5 (Winnowed)

---

### 1. Fleet Lens: Host × Project × State × Readiness

**User/audience:** Parallel-agent operators juggling Mac + `skillbox-portfolio-devbox` + Tailnet backends who currently lose context between `tmux ls`, picker, thought rail, and Trogdor.

**Why indispensable:** This is the “one Swimmers thing” glance surface. Without a shared filter vocabulary, federated sessions become a bigger aquarium with the same confusion — just more fish from more hosts.

**Architecture fit:** `list_sessions_for_client` already merges local supervisor sessions + `list_remote_sessions()`. Remote fish are namespaced (`devbox::sess_3`) and labeled in `tmux_name` (`[devbox] 3`). `ThoughtFilter`, operator-pressure `repo_key`, and Trogdor repo grouping already encode project/readiness dimensions — they are not wired to a global lens.

**Likely touches:**
- `src/api/service.rs` (`list_sessions_for_client`)
- `src/types.rs` (optional `launch_target_id` / `host_label` on `SessionSummary`, derivable from namespace)
- `src/bin/swimmers_tui/app.rs`, `entity.rs`, `thoughts.rs`
- `src/web/rendered_surface.js`, `contracts.js`
- New `GET /v1/sessions?host=&project=&state=&ready=` or client-side filter with documented semantics

**Risks:**
- Filter drift between TUI, web, and API query params.
- Over-filtering hides “degraded stale” remote fish operators still need to see.
- Parsing host from `session_id` prefix is fragile if target IDs contain `::` (today they do not).

**Acceptance tests / smoke proofs:**
- Fixture: 3 local + 2 namespaced remote sessions across 2 targets; filter `host=devbox` → exactly 2 sessions; filter `project=swimmers` → correct subset on both hosts.
- TUI: apply lens, open thought rail — same subset; `Tab` pwd grouping respects active lens.
- Web: session rail chip `host:devbox` matches TUI count.
- `curl '/v1/sessions?...'` returns same IDs as TUI after refresh.

---

### 2. Remote Write Proxy (Input, Group Input, Attention)

**User/audience:** Operators who already see `devbox::sess_12` in the local aquarium but must SSH or repoint `SWIMMERS_TUI_URL` to actually respond.

**Why indispensable:** Federation today is **read-heavy**: remote list, timeline, transcript, git-diff, mermaid proxy through `denamespace_for_target`, but `send_input` and `send_group_input` only talk to local `SessionActor`. A cockpit that shows remote waiting agents but cannot answer them fails the core operator loop.

**Architecture fit:** `remote_sessions.rs` already has auth, URL building, denamespace, and remote GET helpers. Extend with `POST …/input` and batch proxy. `group_input_summary_map` must include federated summaries (same merge as list). Attention dismiss likely needs the same path.

**Likely touches:**
- `src/api/remote_sessions.rs` (new `send_remote_input`, `send_remote_group_input`)
- `src/api/sessions/core_routes.rs` (`send_input`)
- `src/api/sessions/group_input.rs`
- `src/api/service.rs` (unified summary lookup for local + remote)
- `src/bin/swimmers_tui/in_process.rs`, `api.rs` (mirror semantics)
- `src/web/send_controller.js`

**Risks:**
- Mixed local+remote group-input: must reject cross-target batches explicitly (today batch scope checks batch IDs only).
- Latency/timeouts on multi-remote group send; partial failure reporting.
- Accidentally proxying to wrong target if prefix matching regresses (existing longest-prefix tests must extend).

**Acceptance tests / smoke proofs:**
- Mock remote server: local `POST /v1/sessions/devbox::sess_1/input` forwards to remote `POST /v1/sessions/sess_1/input` with correct bearer token.
- Group-input two ready sessions on same target succeeds; local+remote mix returns `VALIDATION_FAILED` with clear message.
- TUI: select remote sleeping fish, send text via composer — remote pane receives it (pane-tail poll or integration fixture).
- Regression: `attention_group` still excludes remote from tmux pane layout (unless separately specified).

---

### 3. Environment Health Strip + Path Mapping Doctor

**User/audience:** Operators burned by “launched on Mac but meant devbox”, unmapped `~/repos/foo` → `/home/skillbox/...`, or stale remote cache after Tailscale blip.

**Why indispensable:** Overlay declares targets (`overlay.rs` `agent_launch`) but `remote_targets_health` is `probe: not_run_by_health` and poll failures silently serve 10s stale cache. Trust in the cockpit requires **prove targets before launch**, not discover failure after spawn.

**Architecture fit:** `remote_sessions` already caches poll success/failure and marks `into_remote_poll_degraded`. `swimmers config doctor` pattern exists in README. Extend doctor to validate `path_mappings` cover `SWIMMERS_REPO_SEARCH_ROOTS` and overlay `base_path` children.

**Likely touches:**
- `src/session/overlay.rs` (`remote_targets_health_snapshot`, mapping helpers)
- `src/api/remote_sessions.rs` (scheduled/active probe, expose last error)
- `src/api/health.rs` or `/readyz`
- CLI config doctor module
- `src/bin/swimmers_tui/app/health.rs`, `picker.rs` (block launch on `LAUNCH_TARGET_PATH_UNMAPPED`)
- `src/web/rendered_surface.js` (connection chips)

**Risks:**
- Probe storms if every TUI refresh hits N targets (rate-limit, parallel cap).
- False red on tailnet_trust targets without token when auth env unset.
- Doctor noise if monorepo paths legitimately map only on some targets.

**Acceptance tests / smoke proofs:**
- Doctor with fixture overlay: unmapped repo → exit non-zero + human message naming `local_prefix` to add.
- Target down: header shows `devbox · stale 42s`; sessions marked degraded; launch to that target shows warning.
- Target up: probe clears stale within one poll cycle.
- `map_cwd_for_target` unchanged behavior; doctor only reports gaps.

---

### 4. Group-by-Project Display Mode (Aquarium + Atlas)

**User/audience:** Operators who relied on c0 `--group-by-pwd` to see “everything in swimmers” vs “everything in htma” without mentally parsing 15 session names across hosts.

**Why indispensable:** Thought rail already toggles pwd/batch grouping; Trogdor atlas groups by repo. The **aquarium** — Swimmers’ differentiated glance surface — still scatters fish by physics unless filtered. Display-only clustering closes the c0 lesson inside Swimmers without session mutation.

**Architecture fit:** Reuse `attention_repo_key` / `attention_project_family` from `attention_group.rs` for cluster labels. Apply layout offsets per cluster (display only). Web Trogdor already draws repo rows; add optional host sub-row or host tint for namespaced sessions.

**Likely touches:**
- `src/api/service/attention_group.rs` (extract shared `repo_key` helpers to `src/session/` or `operator_pressure.rs`)
- `src/bin/swimmers_tui/entity.rs`, `layout.rs`, `app.rs` (toggle: free swim vs group-by-project)
- `src/bin/swimmers_tui/thoughts.rs` (align pwd labels with aquarium clusters)
- `src/web/rendered_surface.js` (atlas grouping)
- `src/operator_pressure.rs` (repo aggregation already exists)

**Risks:**
- Operators confuse display clustering with batch membership or attention-group tmux merge — UI must say “view only”.
- Layout jitter on refresh if cluster bounds recompute aggressively.
- Cross-host same repo name (same `repo_key`, different cwd roots) needs host disambiguation in label.

**Acceptance tests / smoke proofs:**
- Toggle group-by-project: sessions with cwd `/Users/b/repos/opensource/swimmers` and `devbox::` mapped `/home/skillbox/repos/opensource/swimmers` land in one cluster labeled `swimmers`, with host sublabels.
- Toggle off: positions differ; **no** `session_id` or tmux changes (metamorphic invariant).
- No effect on `POST /v1/sessions` or batch create paths.
- `make glance-smoke` equivalent with multi-repo fixture: clusters match operator-pressure `repos` list.

---

### 5. Remote-First Launch Guardrails (Explicit Local)

**User/audience:** Operators whose preference is devbox-first (c0 remote-first); local Mac sessions should be deliberate, not default drift.

**Why indispensable:** Picker already has launch-target toggle and overlay `default_target` / `group_defaults`, but the failure mode is silent wrong-host spawn. Making “where this will run” unavoidable at launch time prevents the Mac/devbox/container root confusion that motivated reconsidering FrankenTerm.

**Architecture fit:** `OverlayLaunchConfig::default_for_group` and picker `launch_target` field already exist. Extend: when entering picker from a group, preselect overlay default; `[local]` requires second confirmation if default ≠ local; show mapped remote cwd preview via `map_cwd_for_target`.

**Likely touches:**
- `src/session/overlay.rs` (`OverlayLaunchConfig`, `group_defaults`)
- `src/api/dirs.rs` (return effective default per group in `DirListResponse`)
- `src/bin/swimmers_tui/picker.rs`, `picker/render.rs`, `app/picker_actions.rs`
- `src/web/dir_browser.js`, `dir_browser_controller.js`
- `src/api/remote_sessions.rs` (`map_cwd_for_target` errors → picker UI)

**Risks:**
- Friction for genuinely local-only workflows (must stay one-click when `default_target: local`).
- Wrong `group_defaults` in overlay YAML causes persistent misrouting — doctor must catch.
- Batch launch with mixed mappability: some dirs map, some do not — need per-row target/status in batch UI.

**Acceptance tests / smoke proofs:**
- Overlay fixture: `default_target: devbox`, group `skills` → `devbox`; opening picker in that group shows `devbox` selected and mapped path preview.
- Switch to `local` shows lightweight confirm chip (“spawn on this Mac”).
- `POST /v1/sessions` with `launch_target: devbox` still namespaces `devbox::sess_N` in local list response.
- Unmapped cwd: launch disabled with same error code as `LAUNCH_TARGET_PATH_UNMAPPED`; doctor lists fix.

---

## Candid Summary

Swimmers is **closer than FrankenTerm** to the right answer because it already has the hard parts: live state, thought rail, federated read path, overlay-driven launch, and operator-pressure semantics. The gap is not another terminal renderer — it is **operability and orientation** across configured environments:

1. **See** (Fleet Lens + group-by-project display)  
2. **Trust** (health strip + path doctor)  
3. **Act** (remote write proxy)  
4. **Launch deliberately** (remote-first guardrails)

Stay inside VISION.md non-goals: no arbitrary SSH fleet, no cluster scheduler, no replacing tmux. Keep c0 and NTM as external workflows; teach Swimmers their **display and default-host lessons** instead.

**Suggested implementation order:** (3) doctor/health → (5) launch guardrails → (2) remote write proxy → (1) fleet lens → (4) group-by-project display — each step is independently shippable and testable.
