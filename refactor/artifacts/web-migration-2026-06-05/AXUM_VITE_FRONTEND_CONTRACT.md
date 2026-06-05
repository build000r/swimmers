# Axum Vite TypeScript Frontend Contract

Run id: `web-migration-2026-06-05`
Bead: `swimmers-define-axum-vite-frontend-contract-zszf`
Scope: contract only. This artifact does not add package tooling, Vite,
React, TypeScript, lockfiles, or source behavior changes.

## Source Evidence

- `src/web/mod.rs` currently owns the web document routes, boot payload, and
  WebSocket route merge.
- `src/web/assets.rs` currently owns static JS/CSS routes, debug filesystem
  asset reads, release `include_str!`/`include_bytes!` asset embedding,
  FrankenTerm external asset routes, and Trogdor PNG routes.
- `refactor/artifacts/web-migration-2026-06-05/served-route-contract.json`
  captured `56` served routes: documents, JS/CSS assets, FrankenTerm assets,
  sampled Trogdor assets, one negative asset route, and the WebSocket upgrade
  expectation.
- `refactor/artifacts/web-migration-2026-06-05/route-source-manifest.json`
  maps current route constants and handlers back to source files.
- Tooling discovery in the baseline found no tracked `package.json`,
  `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`,
  `vite.config*`, or `tsconfig.json`.
- `AGENTS.md` records JS `.mjs` tests under `src/web/` but no package/build
  file or Make target for a JS package runner.

## Package Manager

Chosen package manager: `npm`.

Rationale:

- The repo has no existing JavaScript package manager or lockfile convention.
- The baseline already depends on system Node for
  `node --test src/web/*.test.mjs`; `npm` ships with Node and avoids adding a
  Corepack, pnpm, yarn, or bun prerequisite before the repo demonstrates a need
  for one.
- The next tooling bead should add `package.json` and `package-lock.json`
  together. It should not introduce `pnpm-lock.yaml`, `yarn.lock`, `bun.lock`,
  or alternate package manager metadata unless a new bead explicitly changes
  this contract.
- Rust build and install flows must not require the Vite dev server. Any npm
  build step is a deliberate web asset preparation step, not an implicit side
  effect of `cargo build`.

## Ownership Boundary

`src/web/mod.rs` remains the owner of browser document routes and runtime boot
data. Vite and TypeScript may own the frontend module graph, but they do not
own routing, API facts, WebSocket semantics, or boot payload synthesis.

`src/web/assets.rs` remains the owner of served asset policy. After Vite is
introduced, this file should be the Rust boundary that decides whether the
document uses development asset URLs or embedded/built asset URLs, and it
continues to serve backend-owned external assets.

React, if introduced in a later bead, may render app UI inside the existing web
surface. It must not take ownership of FrankenTerm internals, the WebSocket
protocol, or Trogdor backend facts.

## Boot Payload Contract

`render_index` must continue to serialize `window.__SWIMMERS_BOOT__` before
loading the frontend entry script. TypeScript may define a matching type and
runtime validator, but it must read the Rust-provided payload instead of
reconstructing these values from `import.meta.env` or compile-time constants.

The boot payload fields are stable:

- `franken_term_available`
- `franken_term_js_url`
- `franken_term_wasm_url`
- `franken_term_font_url`
- `franken_term_asset_info`
- `follow_published_selection`
- `focus_layout`

Route-specific values:

- `/` sets `follow_published_selection: false` and `focus_layout: false`.
- `/selected` sets `follow_published_selection: true` and
  `focus_layout: true`.
- FrankenTerm URL fields keep pointing at the backend routes under
  `/assets/frankenterm/`.
- `franken_term_asset_info` stays optional and continues to describe backend
  resolved external files with route, size, and checksum data when available.

## Dev Asset Flow

Development flow keeps Axum as the browser origin for documents, API routes,
and WebSockets.

Future Vite development integration should use this shape:

1. `src/web/mod.rs` renders `/` and `/selected` through the same
   `render_index(focus_layout)` path that currently sets the boot payload.
2. `src/web/mod.rs` asks `src/web/assets.rs` for the active frontend entry
   tags or URLs.
3. In debug builds with an explicit Vite dev origin, `src/web/assets.rs`
   returns Vite dev module URLs for the Vite client and TypeScript entry.
   The expected local entry is the future TypeScript app entry, not a
   FrankenTerm bundle entry.
4. The Vite dev server handles TypeScript transforms, hot reload, and dev
   source maps. Axum continues to handle `/v1/*`, `/ws/sessions/{session_id}`,
   `/assets/frankenterm/*`, and `/assets/dragon/{pose}/{frame}`.
5. If the Vite dev origin is not configured or not reachable, the behavior
   must be explicit: either fail with a clear developer error or use the
   checked-in/built fallback. Do not silently serve a half-migrated page.

The existing `dev_asset` pattern is the behavior to preserve conceptually:
debug browser refreshes should pick up frontend edits without rebuilding the
Rust binary. Vite should replace the source-file reads for app JS/CSS, not the
backend document/boot ownership.

## Release And Source Asset Flow

Release binaries must keep embedding web assets. A release build must not
depend on a running Vite server, local `node_modules`, or host filesystem paths
for app JS/CSS.

Future Vite release integration should use this shape:

1. `npm ci` installs the locked frontend toolchain.
2. `npm run build` produces a Vite production build and manifest in a
   repo-declared web dist directory.
3. The dist directory is available before Rust release/package builds. The
   exact later implementation can use checked-in dist assets or a documented
   release preparation step, but `cargo build --release` and
   `cargo install --path .` must produce a binary that can serve the web UI
   without a Vite dev server.
4. `src/web/assets.rs` embeds the built app entry, chunks, CSS, and manifest
   through Rust asset embedding in release builds.
5. `src/web/mod.rs` renders the same `/` and `/selected` documents and injects
   the manifest-selected built entry routes.
6. Missing built assets are a build or startup error with a targeted message,
   not a runtime page with missing scripts.

FrankenTerm JS/WASM/font files are not Vite build outputs. They remain backend
external asset routes resolved from `SWIMMERS_FRANKENTUI_PKG_DIR` or
`FRANKENTUI_PKG_DIR`.

Trogdor PNG sprite files remain backend embedded assets under
`/assets/dragon/{pose}/{frame}` unless a later route-delta bead explicitly
moves them.

## Route Mapping

Document routes:

| Current route | Current owner | Future dev path | Future release path |
| --- | --- | --- | --- |
| `/` | `src/web/mod.rs::index` | Axum-rendered document, Vite dev entry injected | Axum-rendered document, built Vite entry injected |
| `/selected` | `src/web/mod.rs::selected_index` | Axum-rendered document, Vite dev entry injected, focus boot fields true | Axum-rendered document, built Vite entry injected, focus boot fields true |
| `/ws/sessions/{session_id}` | `src/web/mod.rs::session_ws` | unchanged Axum WebSocket route | unchanged Axum WebSocket route |

App JS/CSS routes:

| Current route group | Current owner | Future dev path | Future release path |
| --- | --- | --- | --- |
| `/app.js` | `src/web/assets.rs::app_js` | Vite TypeScript entry replaces document use; compatibility route may stay no-store until a route delta retires it | manifest-selected built entry route; `/app.js` may stay as a no-store compatibility alias until a route delta retires it |
| `/app.css` | `src/web/assets.rs::app_css` | Vite CSS graph replaces document use; compatibility route may stay no-store until a route delta retires it | manifest-selected built CSS route; `/app.css` may stay as a no-store compatibility alias until a route delta retires it |
| Current individual JS module routes | `src/web/assets.rs` route constants and `javascript_route!` handlers | imported from the Vite dev module graph by source path | bundled or chunked into Vite dist assets; old routes require an explicit route-contract delta before removal |

Current individual JS module routes are:

```text
/api_client.js
/session_persistence.js
/app_event_handlers.js
/app_event_bindings.js
/trogdor_event_bindings.js
/rendered_surface.js
/rendered_surface_draw.js
/input_support.js
/surface_action_plans.js
/terminal_stage_controller.js
/send_sheet.js
/send_controller.js
/thought_config_sheet.js
/native_desktop_sheet.js
/terminal_surface_setup.js
/terminal_surface_controller.js
/terminal_focus.js
/terminal_zoom_input.js
/terminal_resize.js
/global_shortcut_dispatch.js
/session_refresh.js
/agent_context_refresh.js
/mermaid_artifact.js
/mermaid_artifact_controller.js
/terminal_safety.js
/terminal_search_links.js
/terminal_status.js
/terminal_protocol.js
/terminal_input.js
/session_socket_controller.js
/dir_browser.js
/dir_browser_controller.js
/command_palette.js
/command_palette_controller.js
/trogdor_logic.js
/trogdor_state.js
/trogdor_dom_logic.js
/trogdor_render.js
/trogdor_surface_controller.js
/workbench_dom.js
/workbench_render.js
/workbench_log_lens.js
/workbench_refresh.js
/workbench_records.js
/terminal_workbench_controller.js
```

Backend-owned static asset routes:

| Current route | Current owner | Future dev path | Future release path |
| --- | --- | --- | --- |
| `/assets/frankenterm/FrankenTerm.js` | `src/web/assets.rs::franken_term_js` | unchanged backend external file route | unchanged backend external file route |
| `/assets/frankenterm/FrankenTerm_bg.wasm` | `src/web/assets.rs::franken_term_wasm` | unchanged backend external file route | unchanged backend external file route |
| `/assets/frankenterm/pragmasevka-nf-subset.woff2` | `src/web/assets.rs::franken_term_font` | unchanged backend external file route | unchanged backend external file route |
| `/assets/dragon/{pose}/{frame}` | `src/web/assets.rs::trogdor_dragon_asset` | unchanged backend embedded PNG route | unchanged backend embedded PNG route |

## Cache Policy

- `/` and `/selected`: keep `Cache-Control: no-store`.
- Vite dev assets: no-store/dev-server defaults are acceptable; do not add
  long-lived caching to dev assets.
- Current compatibility routes such as `/app.js`, `/app.css`, and individual
  module routes: keep `Cache-Control: no-store` while they exist.
- Vite release assets with content hashes: serve with
  `Cache-Control: public, max-age=31536000, immutable`.
- Non-hashed release aliases, if any: serve with `Cache-Control: no-store` or
  short cache only. Do not make `/app.js` or `/app.css` immutable unless their
  URLs contain content hashes.
- FrankenTerm JS/WASM/font routes: keep `Cache-Control: no-store` because they
  resolve to host-provided external files whose current bytes are reported via
  `franken_term_asset_info`.
- Trogdor PNG routes: keep
  `Cache-Control: public, max-age=31536000, immutable` while they remain
  compile-time embedded assets.

## Source Map Policy

- Current baseline routes have no source map references.
- Vite dev source maps are allowed through the Vite dev server.
- Production source maps must not be browser-fetchable by default. If generated
  for debugging, keep them out of served routes or guard them behind an
  explicit opt-in route policy captured in a later route-contract delta.
- Release JS/CSS served by Axum must not include `sourceMappingURL` comments
  unless the corresponding source-map route is intentionally served and tested.

## Test Strategy

Current parity command:

```bash
node --test src/web/*.test.mjs
```

This remains the required JavaScript parity check until the migration has
ported or replaced each covered behavior with equivalent TypeScript tests.
The next tooling bead may add an npm script that wraps the exact command, but
the direct command remains valid during migration.

Backend route and boot validation:

```bash
cargo test web::
```

This remains the preferred Rust validation for `src/web/mod.rs`,
`src/web/assets.rs`, boot payload behavior, route content types, cache headers,
FrankenTerm asset errors, and Trogdor asset routing.

Future TypeScript validation:

```bash
npm run typecheck
```

The expected implementation is `tsc --noEmit` or an equivalent TypeScript
typecheck that does not write build output.

Future Vite/React validation:

```bash
npm run test:unit
```

The expected implementation is a Vite-compatible unit test runner for React
components and browser-facing modules. It must complement, not immediately
replace, `node --test src/web/*.test.mjs`. Existing pure logic, storage,
protocol, and DOM-contract tests should move only when the replacement test is
at least as specific as the current test.

Route-contract validation after any frontend serving change:

- Re-capture `/`, `/selected`, app asset routes, FrankenTerm routes, and
  sampled Trogdor routes.
- Compare boot payload fields, content types, cache headers, status codes, and
  source-map references.
- Record every intentional delta in the migration artifact directory before
  closing the source-changing bead.

## Non-Goals

- No Electron.
- No SSR framework.
- No React ownership of FrankenTerm internals.
- No new Trogdor backend facts. Keep `docs/TROGDOR_WEB.md` as the reuse
  contract: fix or add facts in existing session/thought/API plumbing, then
  derive presentation state from those facts.
- No package manager, lockfile, Vite config, TypeScript config, or source
  behavior change in this bead.
