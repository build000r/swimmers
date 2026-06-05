# Web Migration Isomorphism Ledger

Run id: `web-migration-2026-06-05`
Artifact path: `refactor/artifacts/web-migration-2026-06-05/`
Bead: `swimmers-create-web-migration-isomorphism-ledger-ceiv`

This ledger extends the baseline artifacts captured for
`swimmers-capture-web-migration-baseline-kcdh`. It is an artifact harness only:
no web source behavior, Vite, React, or route implementation changes are part
of this run.

## Baseline Snapshot

- Baseline git head: `b34160b7ddb686c3ce188a86fa412f20b919a0d6`
- Baseline branch: `main`
- Node baseline: `v22.22.2`
- Rust baseline: `rustc 1.96.0`, `cargo 1.96.0`
- Test count: `node --test src/web/*.test.mjs` passed `366` of `366`
  tests. Raw evidence: `logs/js-node-test.log`.
- Rust web tests: `cargo test web::` passed; the main web unit target reported
  `68 passed`, `0 failed`, `1121 filtered out`. Raw evidence:
  `logs/cargo-test-web.log`.
- Baseline LOC command:
  `wc -l src/web/app.js src/web/app.css src/web/*.js src/web/*.test.mjs`.
  It reported `30468` lines, with `app.js` counted twice by the explicit path
  plus glob. Raw evidence: `logs/loc-web.txt`.
- Unique web JS/test/CSS baseline LOC: `28469` lines. Raw evidence:
  `logs/loc-web-dedup.txt`.
- Route contract: `56` unique served routes, with `53` HTTP `200`, `1` HTTP
  `400`, and `2` HTTP `404`. Evidence: `served-route-contract.json`,
  `route-source-manifest.json`, and `route-checksums.txt`.
- Tooling discovery baseline: no tracked package manager, Vite config, or
  TypeScript config files matched the discovery command. Evidence:
  `logs/tooling-discovery.txt`.
- Chrome/Chromium was unavailable during the baseline capture; Chrome-dependent
  smoke and screenshot checks were skipped with evidence in
  `logs/browser-smoke-skip.txt`.

## Current Module Graph

The current ES-module graph for `src/web/*.js` is recorded in
`MODULE_GRAPH.md`.

Summary:

- JS modules: `48`
- Import declarations: `70`
- Unique dependency pairs: `68`
- Root module: `app.js`
- `app.js` directly imports `32` modules.

The served Axum asset graph remains represented by `route-source-manifest.json`
and `served-route-contract.json`; later migration beads must preserve the route
manifest or record explicit, reviewed route deltas.

## High-Risk Islands

These areas require isomorphism cards before any framework migration or cleanup
commit touches them:

- FrankenTerm runtime: `terminal_surface_controller.js`,
  `terminal_surface_setup.js`, `terminal_runtime.js`, `rendered_surface.js`,
  `rendered_surface_draw.js`, `app_behavior.test.mjs`, and the
  FrankenTerm routes in `src/web/assets.rs`.
- Trogdor atlas and reader: `trogdor_logic.js`, `trogdor_dom_logic.js`,
  `trogdor_render.js`, `trogdor_state.js`, `trogdor_surface_controller.js`,
  `trogdor_event_bindings.js`, `rendered_surface.js`, `app_trogdor.css`,
  Trogdor route assets in `src/web/assets.rs`, and the related tests.
- WebSocket and input protocol: `session_socket_controller.js`,
  `terminal_input.js`, `terminal_protocol.js`, `input_support.js`,
  `terminal_stage_controller.js`, `src/web/ws_events.rs`,
  `src/web/ws_messages.rs`, `src/web/ws_auth.rs`, and related app behavior
  tests.
- Sheets and workbench: `command_palette_controller.js`, `send_controller.js`,
  `thought_config_sheet.js`, `native_desktop_sheet.js`,
  `mermaid_artifact_controller.js`, `terminal_workbench_controller.js`,
  `workbench_render.js`, `workbench_log_lens.js`, `workbench_refresh.js`,
  `workbench_dom.js`, `app_sheets.css`, `app_sheet_results.css`, and related
  tests.
- Axum asset routes: `src/web/mod.rs`, `src/web/assets.rs`, `src/web/tests.rs`,
  route constants, `include_str!` web assets, FrankenTerm file serving, and
  embedded Trogdor PNG asset serving.

## Stop Conditions

Stop the migration or cleanup bead when any of these are true:

- A planned change requires source behavior edits before an isomorphism card
  states the current and target behavior in testable terms.
- `node --test src/web/*.test.mjs` fails, hangs, or reports a different test
  count without a documented reason.
- Route status, checksum, content type, boot payload, or asset availability
  changes without an explicit route-contract delta.
- A change touches a high-risk island without targeted test evidence for that
  island.
- The proposed simplification score is below `2.0`, or any score input is
  guessed rather than evidenced.
- Browser evidence is required but Chrome/Chromium remains unavailable; document
  the skip instead of claiming visual parity.
- The change introduces package manager, Vite, React, TypeScript, or bundler
  files in a bead scoped only to artifacts or rules.

## Isomorphism Card Template

Each later migration or cleanup commit should fill one card before editing
source:

```text
Run id: web-migration-2026-06-05
Artifact path: refactor/artifacts/web-migration-2026-06-05/
Candidate:
Touched files:
High-risk island:

Current behavior:
Target behavior:
Behavior invariants:
Route/asset invariants:
Protocol/DOM/storage invariants:

Proof before edit:
Proof after edit:
Validation commands:
Observed deltas:

LOC saved:
Confidence:
Risk:
Simplify score: LOC_saved x Confidence / Risk =

Decision: accept | reject | split | defer
Reason:
```

## Simplify Rule

Behavior identity is the first gate. A simplification must prove equivalent
behavior using tests, route contracts, or targeted command output before line
removal is considered.

Score:

```text
score = LOC_saved x Confidence / Risk
```

Threshold:

```text
implement only when score >= 2.0
```

Inputs:

- `LOC_saved`: net source lines removed after tests and generated artifacts are
  excluded.
- `Confidence`: `1.0` for direct tests and route/contract evidence, `0.75` for
  targeted tests plus manual artifact checks, `0.5` for partial indirect
  evidence. Anything lower is a rejection.
- `Risk`: `1.0` for isolated pure helpers, `2.0` for DOM/storage/state changes,
  `3.0` for WebSocket/input/protocol changes, `4.0` for FrankenTerm, Trogdor,
  workbench, sheets, or Axum route changes.

Examples:

- Remove `8` duplicate helper lines in a pure module with direct tests:
  `8 x 1.0 / 1.0 = 8.0`, eligible.
- Remove `6` DOM lines in a sheet controller with targeted tests:
  `6 x 0.75 / 2.0 = 2.25`, eligible only with the card complete.
- Remove `6` WebSocket protocol lines with partial evidence:
  `6 x 0.5 / 3.0 = 1.0`, reject or split.

## Rejection Log

- `2026-06-05`: no source simplification was accepted in this bead. This bead
  only created the proof discipline and recorded the current baseline.
- `2026-06-05`: rejected a blanket `apiJson` or raw response replacement for
  `swimmers-simplify-after-typescript-contracts-t8jb`; raw POST response bodies
  in send/launch/open/create/save flows keep their existing parse semantics.
- `2026-06-05`: rejected per-response workbench widget normalizers for
  `swimmers-simplify-after-typescript-contracts-t8jb`; workbench widget
  normalization intentionally stays at the settled aggregate result boundary.
- `2026-06-05`: rejected edits to `session_socket_controller.js`,
  `trogdor_*`, Rust routes, Vite/tooling, CSS/layout, and terminal lifecycle
  modules for `swimmers-simplify-after-typescript-contracts-t8jb`.

## Accepted Simplification Cards

### Typed HTTP Response Normalization

Run id: `web-migration-2026-06-05`
Artifact path: `refactor/artifacts/web-migration-2026-06-05/`
Bead: `swimmers-simplify-after-typescript-contracts-t8jb`

Candidate: centralize typed HTTP JSON normalization in
`src/web/api_client.js` with normalizer-aware `responseJson(response,
normalizer)` and `responseJsonOrNull(response, normalizer)`.

Touched files:

- `src/web/api_client.js`
- `src/web/api_client.test.mjs`
- `src/web/app.js`
- `src/web/terminal_workbench_controller.js`
- `src/web/agent_context_refresh.js`
- `src/web/agent_context_refresh.test.mjs`
- `src/web/session_refresh.js`
- `src/web/session_refresh.test.mjs`
- `src/web/dir_browser_controller.js`
- `src/web/mermaid_artifact.js`
- `src/web/mermaid_artifact_controller.js`
- `src/web/native_desktop_sheet.js`
- `src/web/thought_config_sheet.js`

High-risk island: sheets and workbench-adjacent refresh paths. No
FrankenTerm, Trogdor, WebSocket, Rust route/schema, CSS/layout, Vite/tooling,
or raw POST response contract changes.

Current behavior: proven HTTP callsites parsed JSON with `await
response.json()` or `await responseJsonOrNull(response)` and then immediately
applied a `normalize*Response` contract helper at the callsite.

Target behavior: the same response objects are parsed in the same order, but
the contract normalizer is passed into `responseJson` or `responseJsonOrNull`.
`responseJsonOrNull(null, normalizer)` still returns `null` without invoking
the normalizer.

Behavior invariants:

- `apiFetch` and `apiMaybeFetch` error handling and 404-to-null behavior are
  unchanged.
- `runSessionRefresh` still starts the same fetches in the same `Promise.all`,
  parses sessions before pressure/health, applies operator pressure and
  backend health before Trogdor cue sync, and applies selection before success
  side effects.
- Agent-context stale guards still run after JSON parse and before state
  replacement.
- Mermaid, directory, native desktop, thought config, and terminal snapshot
  normalizers keep the same backend vocabulary and tolerant defaults.
- Raw POST response bodies remain raw where the caller only needs operation
  output text or save/create/open results.

Route/asset invariants: no Rust route, schema, asset manifest, content type, or
served file change.

Protocol/DOM/storage invariants: no WebSocket, terminal lifecycle, DOM layout,
CSS, storage key, fetch ordering, or Trogdor behavior change.

Proof before edit:

- Start gate `git status --short --branch` reported clean `main` at
  `origin/main`; after claim the only dirty file was `.beads/issues.jsonl`.
- Static callsite scan found 13 proven normalized wrappers and 5 raw response
  parses that were intentionally left untouched.
- Before LOC command:
  `wc -l src/web/api_client.js src/web/api_client.test.mjs src/web/app.js src/web/terminal_workbench_controller.js src/web/agent_context_refresh.js src/web/session_refresh.js src/web/workbench_refresh.js src/web/dir_browser_controller.js src/web/mermaid_artifact.js src/web/mermaid_artifact_controller.js src/web/native_desktop_sheet.js src/web/thought_config_sheet.js`
  reported `4155` total lines.

Proof after edit:

- Focused validation:
  `node --test src/web/api_client.test.mjs src/web/contracts.test.mjs src/web/agent_context_refresh.test.mjs src/web/session_refresh.test.mjs src/web/workbench_refresh.test.mjs`
  passed `34/34`.
- Post-edit scan leaves only raw response bodies in intentional POST result
  paths: thought config save, directory create/batch, commit launch, and
  Mermaid open.
- After LOC command over the same files reported `4227` total lines.

Validation commands:

- Focused command above passed `34/34`.
- `npm run typecheck` passed.
- `node --test src/web/*.test.mjs` passed `385/385`.
- `npm test` passed `385/385`.
- `git diff --check` passed.
- `br dep cycles --json` reported `{"cycles":[],"count":0}`.
- `br sync --flush-only` passed.
- `rg -n 'as any|as unknown as|: any\b' src/web --glob '*.js' --glob '*.mjs' --glob '*.ts'`
  returned no matches.

Observed deltas:

- Scoped file LOC: `4155` before, `4227` after, net `+72`.
- Runtime source LOC excluding `api_client.test.mjs`: `3981` before, `4004`
  after, net `+23`.
- Test LOC: `174` before, `223` after, net `+49`.
- Centralized 13 duplicate normalizer-wrapper callsites while adding the
  explicit parser seam and focused normalizer tests.

LOC saved: scout counted 13 proven wrapper callsites centralized; measured net
source LOC is `+23` because explicit dependency injection and the shared helper
were added.
Confidence: `0.8`, backed by focused contract/refresh tests and raw-callsite
scan.
Risk: `4.0`, because the accepted callsites include sheet and workbench-adjacent
refresh paths.
Simplify score: scout recommendation `13 x 0.8 / 4.0 = 2.6`.

Decision: accept.
Reason: behavior is held at the existing contracts, parsing/null/error
semantics are centralized in `api_client.js`, and raw/aggregate response paths
that would change semantics were explicitly rejected.
