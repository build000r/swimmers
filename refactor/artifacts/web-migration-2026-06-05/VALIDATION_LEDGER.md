# Web Migration Validation Ledger

Run id: `web-migration-2026-06-05`
Artifact path: `refactor/artifacts/web-migration-2026-06-05/`
Bead: `swimmers-create-web-migration-isomorphism-ledger-ceiv`

This validation ledger records the artifact-harness checks for the local
isomorphism ledger run. It does not authorize source behavior edits.

## Baseline Commands

Previously captured baseline commands:

```sh
node --test src/web/*.test.mjs
cargo test web::
wc -l src/web/app.js src/web/app.css src/web/*.js src/web/*.test.mjs
rg --files | rg '(^|/)(package.json|vite.config|tsconfig|pnpm-lock|package-lock|yarn.lock|bun.lock)$'
```

Evidence:

- JS tests: `logs/js-node-test.log`.
- Rust web tests: `logs/cargo-test-web.log`.
- LOC: `logs/loc-web.txt` and `logs/loc-web-dedup.txt`.
- Tooling discovery: `logs/tooling-discovery.txt`.
- Route contract: `route-source-manifest.json`,
  `served-route-contract.json`, and `route-checksums.txt`.

## Local Validation

Commands run for this ledger update:

```sh
node --test src/web/*.test.mjs
git diff --check
test -f refactor/artifacts/web-migration-2026-06-05/ISOMORPHISM_LEDGER.md && test -f refactor/artifacts/web-migration-2026-06-05/MODULE_GRAPH.md && test -f refactor/artifacts/web-migration-2026-06-05/SCAN_COMMANDS.md && test -f refactor/artifacts/web-migration-2026-06-05/VALIDATION_LEDGER.md && test -f refactor/artifacts/web-migration-2026-06-05/SUMMARY.md
rg -n "web-migration-2026-06-05|Baseline|baseline|Stop Conditions|stop conditions|score = LOC_saved x Confidence / Risk|score >= 2\\.0" refactor/artifacts/web-migration-2026-06-05/ISOMORPHISM_LEDGER.md refactor/artifacts/web-migration-2026-06-05/MODULE_GRAPH.md refactor/artifacts/web-migration-2026-06-05/SCAN_COMMANDS.md refactor/artifacts/web-migration-2026-06-05/VALIDATION_LEDGER.md refactor/artifacts/web-migration-2026-06-05/SUMMARY.md
```

Results:

- `node --test src/web/*.test.mjs`: passed, `366` tests, `366` pass,
  `0` fail.
- `git diff --check`: passed.
- Artifact existence check: passed.
- Artifact content check: passed; artifacts mention the run id, baseline
  commands or baseline evidence, stop conditions, and simplify score rule.

## Stop Conditions Checked

- No source behavior files were edited.
- No Vite, React, TypeScript, package manager, or bundler files were added.
- No route, asset, WebSocket, DOM, storage, FrankenTerm, Trogdor, sheet, or
  workbench implementation was changed.
- Browser smoke was not claimed because Chrome/Chromium was unavailable in the
  baseline capture.
- Simplification proposals remain rejected until a later card proves behavior
  identity and reaches `score >= 2.0`.

## Skipped Tools

- `jscpd`: skipped because `command -v jscpd` found no executable.
- `scc`: skipped because `command -v scc` found no executable.
- `tokei`: skipped because `command -v tokei` found no executable.
- `cloc`: skipped because `command -v cloc` found no executable.
- Chrome/Chromium smoke and screenshots: not rerun; the baseline artifact
  already records Chrome/Chromium as unavailable in
  `logs/browser-smoke-skip.txt`.
