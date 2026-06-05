# Web Migration Baseline - 2026-06-05

Baseline captured for Bead `swimmers-capture-web-migration-baseline-kcdh`.
Scope was artifact capture only; no product behavior or source behavior was
changed.

## Environment

- Git HEAD: `b34160b7ddb686c3ce188a86fa412f20b919a0d6`
- Branch: `main`
- Rust: `rustc 1.96.0`, `cargo 1.96.0`
- Node: `v22.22.2`
- FrankenTerm package: available during route capture; local path omitted from
  artifacts.
- Chrome/Chromium: unavailable, so Chrome-dependent screenshots were skipped.

See `logs/tool-versions.txt` for the exact captured version output.

## Validation Results

- `node --test src/web/*.test.mjs`: passed, `366` tests, `366` pass, `0` fail.
  Raw log: `logs/js-node-test.log`.
- `cargo test web::`: passed. Main web unit run reported `68 passed`, `0 failed`,
  `1121 filtered out`; the remaining filtered binaries/integration targets had
  `0` selected tests and `0` failures. Raw log: `logs/cargo-test-web.log`.
- `wc -l src/web/app.js src/web/app.css src/web/*.js src/web/*.test.mjs`:
  captured in `logs/loc-web.txt`, total `30468` lines for the exact requested
  command. This command double-counts `app.js` through the glob, so
  `logs/loc-web-dedup.txt` also records the unique file-set total: `28469`.
- `rg --files | rg '(^|/)(package.json|vite.config|tsconfig|pnpm-lock|package-lock|yarn.lock|bun.lock)$'`:
  no matches. The repo has no package manager, Vite, or TS config file in the
  tracked source set at this baseline. Raw log: `logs/tooling-discovery.txt`.

## Route And Asset Contract

Artifacts:

- Source route manifest: `route-source-manifest.json`
- Served route checksums: `route-checksums.txt`
- Served route metadata and selected HTML boot markers:
  `served-route-contract.json`
- Route capture server log, with local paths scrubbed:
  `logs/route-server.log`

The served route contract used a loopback server with `AUTH_MODE=local_trust`.
No bearer tokens or query tokens were used or logged.

Summary:

- Unique routes checked: `56`
- Status counts: `53` x `200`, `1` x `400`, `2` x `404`
- `/` and `/selected`: both `200`, include `/app.css`, `/app.js`, terminal
  stage, Trogdor surface, workbench, sheets, and sanitized boot payload fields.
- `/app.js`, `/app.css`, and web module routes: `200` with stable byte counts
  and SHA-256 checksums.
- FrankenTerm JS and WASM routes: `200`.
- FrankenTerm font route: `404`, because the sibling font file was unavailable
  in this capture environment. The boot payload still records the JS/WASM asset
  metadata.
- Trogdor sample sprite routes: `200`.
- Trogdor negative sample `/assets/dragon/mouth-closed/diagonal.png`: expected
  `404`.
- `/ws/sessions/baseline-route-probe`: plain HTTP GET without WebSocket upgrade
  returned expected `400` with `Connection header did not include 'upgrade'`.

## Smoke Evidence

- Partial existing smoke passed:
  `PORT=3322 SWIMMERS_DATA_DIR=<temp> SWIMMERS_PERSONAL_WORKFLOWS=0 bash ./scripts/test-web-live-terminal.sh`
  returned `web live terminal smoke passed`. Raw log:
  `logs/web-live-terminal-smoke.log`.
- `make web-smoke` was not run end-to-end because its
  `scripts/test-web-visible-terminal.sh` leg requires Chrome/Chromium.
- `make web-workbench-smoke` and screenshot capture were skipped for the same
  Chrome/Chromium prerequisite. Skip evidence:
  `logs/browser-smoke-skip.txt`.

## Files

- `ISOMORPHISM_LEDGER.md`
- `MODULE_GRAPH.md`
- `SCAN_COMMANDS.md`
- `VALIDATION_LEDGER.md`
- `logs/js-node-test.log`
- `logs/cargo-test-web.log`
- `logs/loc-web.txt`
- `logs/loc-web-dedup.txt`
- `logs/tooling-discovery.txt`
- `logs/tool-versions.txt`
- `logs/web-live-terminal-smoke.log`
- `logs/browser-smoke-skip.txt`
- `logs/served-route-contract.log`
- `logs/route-server.log`
- `route-source-manifest.json`
- `served-route-contract.json`
- `route-checksums.txt`
