# Web Migration Scan Commands

Run id: `web-migration-2026-06-05`
Artifact path: `refactor/artifacts/web-migration-2026-06-05/`

These commands define the fallback scan harness for later migration beads.
They are intentionally read-only and do not require Vite, React, TypeScript, or
package-manager setup.

## Tool Availability

Command used:

```sh
command -v jscpd || true
command -v scc || true
command -v tokei || true
command -v cloc || true
command -v rg || true
command -v wc || true
command -v node || true
```

Result on `2026-06-05`:

- `jscpd`: unavailable, skipped.
- `scc`: unavailable, skipped.
- `tokei`: unavailable, skipped.
- `cloc`: unavailable, skipped.
- `rg`: available; executable path omitted for OSS hygiene.
- `wc`: available; executable path omitted for OSS hygiene.
- `node`: available; executable path omitted for OSS hygiene.

## LOC

Baseline command already captured:

```sh
wc -l src/web/app.js src/web/app.css src/web/*.js src/web/*.test.mjs
```

Baseline result:

- Requested command total: `30468` lines, with `app.js` counted twice.
- Unique file-set total: `28469` lines, recorded in `logs/loc-web-dedup.txt`.

Current web tracked file counts:

- `96` tracked files under the scanned web source surface.
- `48` `src/web/*.js` files.
- `34` `src/web/*.test.mjs` files.
- `8` `src/web/*.css` files.
- `6` `src/web/*.rs` files.

Exact fallback count command:

```sh
git ls-files 'src/web/*.js' 'src/web/*.test.mjs' 'src/web/*.css' 'src/web/*.rs' | sort -u | wc -l
```

## Module Graph

Exact fallback command used:

```sh
node -e "const fs=require('fs'); const path=require('path'); const files=fs.readdirSync('src/web').filter(f=>f.endsWith('.js')).sort(); for (const f of files){ const s=fs.readFileSync(path.join('src/web',f),'utf8'); const deps=[...s.matchAll(/from\\s+['\"](\\.\\/[^'\"]+)['\"]|import\\s*\\(\\s*['\"](\\.\\/[^'\"]+)['\"]/g)].map(m=>(m[1]||m[2]).replace(/^\\.\\//,'')); console.log(f+' -> '+(deps.length ? deps.join(', ') : '(none)')); }"
```

Result: see `MODULE_GRAPH.md`.

## Duplication

Preferred tool if later installed: `jscpd`.

Exact fallback command used on `2026-06-05`:

```sh
node -e 'const fs=require("fs"); const path=require("path"); const files=fs.readdirSync("src/web").filter(f=>/\.(js|mjs|css)$/.test(f)).map(f=>path.join("src/web",f)).sort(); const n=8; const seen=new Map(); for (const file of files){ const raw=fs.readFileSync(file,"utf8").split(/\r?\n/); const lines=raw.map(l=>l.trim()).filter(l=>l && !l.startsWith("import ")); for (let i=0;i+n<=lines.length;i++){ const block=lines.slice(i,i+n).join("\n"); if (!seen.has(block)) seen.set(block,[]); seen.get(block).push(`${file}:${i+1}`); } } const dupes=[...seen.entries()].filter(([,v])=>v.length>1).sort((a,b)=>b[1].length-a[1].length); console.log(`files=${files.length}`); console.log(`window_lines=${n}`); console.log(`duplicate_windows=${dupes.length}`); for (const [block, locs] of dupes.slice(0,20)){ console.log(`${locs.length}x ${locs.slice(0,6).join(", ")}`); console.log(block.split("\n").slice(0,2).join(" / ")); }'
```

Result snapshot:

- Files scanned: `90`.
- Window size: `8` normalized nonblank lines.
- Duplicate windows: `245`.
- Highest-signal duplicate family: repeated `escapeHtml` helpers across
  `app.js`, `command_palette.js`, `dir_browser.js`, `trogdor_render.js`,
  `trogdor_surface_controller.js`, and `workbench_log_lens.js`.
- This fallback is conservative and exact-text oriented; it is not a semantic
  clone detector.

## Slop

Exact fallback command used:

```sh
rg -n "\b(TODO|FIXME|HACK|XXX|workaround|temporary|temp|debugger|console\.log|eslint-disable)\b" src/web --glob '*.{js,mjs,css,rs}'
```

Result snapshot:

- Matches: `1`.
- Match: `src/web/tests.rs:815`, a test assertion containing the string
  `temp path`; recorded as a false-positive slop hit, not cleanup permission.

## Callsite Surface

Exact fallback command used:

```sh
rg -o "addEventListener|removeEventListener|querySelectorAll?\(|dataset\.|fetch\(|new WebSocket|\.send\(|localStorage|sessionStorage|innerHTML|insertAdjacentHTML|classList\.|style\." src/web/*.js src/web/*.test.mjs | sed 's/.*://' | sort | uniq -c | sort -nr
```

Result snapshot:

```text
108 classList.
 65 innerHTML
 42 dataset.
 18 localStorage
 16 style.
 15 addEventListener
  9 querySelectorAll(
  6 sessionStorage
  5 .send(
  2 fetch(
```

Use these counts to decide where later cards need DOM, storage, WebSocket, or
fetch-specific invariants.

## Axum Asset Route Surface

Exact fallback command used:

```sh
rg -o "route\(|APP_JS_ROUTE|APP_CSS_ROUTE|FRANKENTERM|TROGDOR|include_str|include_bytes|asset|Assets|Router" src/web/*.rs | sed 's/.*://' | sort | uniq -c | sort -nr
```

Result snapshot:

```text
81 asset
54 route(
45 include_str
31 FRANKENTERM
21 TROGDOR
14 APP_JS_ROUTE
 8 include_bytes
 6 Router
 5 Assets
 3 APP_CSS_ROUTE
```

Any route, `include_str!`, `include_bytes!`, FrankenTerm, or Trogdor asset
change must cite `route-source-manifest.json`, `served-route-contract.json`,
and targeted Rust tests before it can be accepted.
