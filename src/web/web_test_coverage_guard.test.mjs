import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const webDir = path.join(repoRoot, "src/web");

async function readRepoFile(relativePath) {
  return readFile(path.join(repoRoot, relativePath), "utf8");
}

async function readRepoJson(relativePath) {
  return JSON.parse(await readRepoFile(relativePath));
}

async function webTestPaths() {
  const entries = await readdir(webDir);
  return entries
    .filter((entry) => entry.endsWith(".test.mjs"))
    .map((entry) => `src/web/${entry}`)
    .sort();
}

test("web test TypeScript config pins every Node web behavior test", async () => {
  const config = await readRepoJson("tsconfig.web-tests.json");
  const configured = [...config.files].sort();

  assert.deepEqual(configured, await webTestPaths());
});

test("package scripts keep web behavior tests and TypeScript guard wired", async () => {
  const packageJson = await readRepoJson("package.json");

  assert.equal(packageJson.scripts.test, "node --test src/web/*.test.mjs");
  assert.match(packageJson.scripts.typecheck, /tsc -p tsconfig\.web-tests\.json/);
});

test("app behavior suite keeps migration-critical app.js coverage topics", async () => {
  const source = await readRepoFile("src/web/app_behavior.test.mjs");
  const requiredSnippets = [
    'await import("./app.js?behavior-test")',
    "FrankenTerm surface validation reports missing methods",
    "FrankenTerm resize waits while another surface operation is active",
    "terminal bytes are buffered until FrankenTerm accepts input",
    "Trogdor atlas renders dragon sprite assets and flames burnt swordsmen",
    "terminal workbench fetches and renders selected agent context",
    "send form submit handler preserves line send side effects and cleanup",
    "auth token button action preserves save, clear, and refresh side effects",
    "accessibility mirror syncs FrankenTerm screen-reader text and announcements",
    "visible directory batch action preserves selection, cwd fallback, and status",
    "command palette filters existing actions without touching terminal input",
  ];

  for (const snippet of requiredSnippets) {
    assert.ok(source.includes(snippet), `missing app_behavior guard: ${snippet}`);
  }
});

test("focused helper suites keep migration-critical behavior coverage topics", async () => {
  const requiredSnippetsByFile = new Map([
    [
      "src/web/input_support.test.mjs",
      [
        "eventCell maps touch coordinates into the rendered grid",
        "surfaceActionDispatchPlan preserves open_send and open_create gates",
        "terminalResizeGeometryPlan preserves resize geometry and side-effect decisions",
        "terminalPendingByteBufferPlan preserves pending byte acceptance and drops",
      ],
    ],
    [
      "src/web/rendered_surface.test.mjs",
      [
        "surface frame emits flat patch payload invariants",
        "surface exposes parity actions for the selected session",
        "surface renders trogdor pressure atlas with hover speed reader",
      ],
    ],
    [
      "src/web/terminal_protocol.test.mjs",
      [
        "buildSessionSocketUrl opts into framed resume without leaking auth",
        "decodeTerminalOutputFrame parses opcode, sequence, and payload",
      ],
    ],
    [
      "src/web/terminal_safety.test.mjs",
      [
        "FrankenTerm link policy allows HTTP only for loopback hosts",
        "terminal paste budget measures UTF-8 bytes",
      ],
    ],
    [
      "src/web/dir_browser.test.mjs",
      [
        "directory path helpers preserve legacy root joining and explicit paths",
        "visibleDirBatchPlan preserves paths, fallbacks, and status copy",
      ],
    ],
    [
      "src/web/dir_browser_controller.test.mjs",
      [
        "directory browser controller delegates dynamic view rendering while preserving state ownership",
      ],
    ],
    [
      "src/web/dir_browser_view_island.test.mjs",
      [
        "directory browser view island preserves group chip DOM contract",
        "directory browser view island preserves row, action, badge, and link contract",
        "directory browser view island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/auth_sheet_island.test.mjs",
      [
        "auth sheet island preserves sheet host and child DOM contract",
        "auth sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/command_palette.test.mjs",
      [
        "command palette state helper combines built-in commands, sessions, and scores",
        "command palette execution plan helper preserves no-ops and dispatch ordering",
      ],
    ],
    [
      "src/web/command_palette_controller.test.mjs",
      [
        "openSheet runs sheet-specific side effects and focus targets",
        "renderCommandPalette delegates results to a mounted React island when present",
        "closeSheets clears send and create state, hides modal, and refocuses terminal immediately",
      ],
    ],
    [
      "src/web/command_palette_island.test.mjs",
      [
        "command palette island preserves sheet host and child DOM contract",
        "command palette island mounts, rerenders results, and guards stable nodes",
      ],
    ],
    [
      "src/web/create_sheet_island.test.mjs",
      [
        "create sheet island preserves sheet host and child DOM contract",
        "create sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/search_sheet_island.test.mjs",
      [
        "search sheet island preserves sheet host and child DOM contract",
        "search sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/send_sheet_island.test.mjs",
      [
        "send sheet island preserves sheet host and child DOM contract",
        "send sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/thought_config_sheet_island.test.mjs",
      [
        "thought config sheet island preserves sheet host and child DOM contract",
        "thought config sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/native_desktop_sheet_island.test.mjs",
      [
        "native desktop sheet island preserves sheet host and child DOM contract",
        "native desktop sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
    [
      "src/web/mermaid_sheet_island.test.mjs",
      [
        "mermaid sheet island preserves sheet host and child DOM contract",
        "mermaid sheet island mounts, rerenders, and guards stable nodes",
      ],
    ],
  ]);

  for (const [relativePath, snippets] of requiredSnippetsByFile) {
    const source = await readRepoFile(relativePath);
    for (const snippet of snippets) {
      assert.ok(source.includes(snippet), `missing ${relativePath} guard: ${snippet}`);
    }
  }
});

test("Vite can transform the app.js behavior-test entry with public test exports", async (t) => {
  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/app.js?behavior-test");

  assert.ok(transformed?.code, "Vite did not transform app.js?behavior-test");
  assert.match(transformed.code, /__swimmersWebTest/);
});

test("Vite transforms the React shell path that owns React imports", async (t) => {
  const source = await readRepoFile("src/web/react_shell.js");
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/react_shell.js");

  assert.ok(transformed?.code, "Vite did not transform react_shell.js");
  assert.match(transformed.code, /react/);
  assert.match(transformed.code, /react-dom/);
});

test("Vite transforms the auth sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/auth_sheet_island.js");
  assert.match(appSource, /import\("\.\/auth_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/auth_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform auth_sheet_island.js");
  assert.match(transformed.code, /AuthSheet/);
});

test("Vite transforms the command palette React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/command_palette_island.js");
  assert.match(appSource, /import\("\.\/command_palette_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/command_palette_island.js");

  assert.ok(transformed?.code, "Vite did not transform command_palette_island.js");
  assert.match(transformed.code, /CommandPaletteSheet/);
});

test("Vite transforms the search sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/search_sheet_island.js");
  assert.match(appSource, /import\("\.\/search_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/search_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform search_sheet_island.js");
  assert.match(transformed.code, /SearchSheet/);
});

test("Vite transforms the send sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/send_sheet_island.js");
  assert.match(appSource, /import\("\.\/send_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/send_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform send_sheet_island.js");
  assert.match(transformed.code, /SendSheet/);
});

test("Vite transforms the thought config sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/thought_config_sheet_island.js");
  assert.match(appSource, /import\("\.\/thought_config_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/thought_config_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform thought_config_sheet_island.js");
  assert.match(transformed.code, /ThoughtConfigSheet/);
});

test("Vite transforms the native desktop sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/native_desktop_sheet_island.js");
  assert.match(appSource, /import\("\.\/native_desktop_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/native_desktop_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform native_desktop_sheet_island.js");
  assert.match(transformed.code, /NativeDesktopSheet/);
});

test("Vite transforms the Mermaid sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/mermaid_sheet_island.js");
  assert.match(appSource, /import\("\.\/mermaid_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/mermaid_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform mermaid_sheet_island.js");
  assert.match(transformed.code, /MermaidSheet/);
});

test("Vite transforms the create sheet React island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/create_sheet_island.js");
  assert.match(appSource, /import\("\.\/create_sheet_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/create_sheet_island.js");

  assert.ok(transformed?.code, "Vite did not transform create_sheet_island.js");
  assert.match(transformed.code, /CreateSheet/);
});

test("Vite transforms the directory browser React view island path", async (t) => {
  const appSource = await readRepoFile("src/web/app.js");
  const source = await readRepoFile("src/web/dir_browser_view_island.js");
  assert.match(appSource, /import\("\.\/dir_browser_view_island\.js"\)/);
  assert.match(source, /from "react"/);
  assert.match(source, /from "react-dom"/);
  assert.match(source, /from "react-dom\/client"/);

  const server = await createServer({
    configFile: path.join(repoRoot, "vite.config.js"),
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  t.after(() => server.close());

  const transformed = await server.transformRequest("/src/web/dir_browser_view_island.js");

  assert.ok(transformed?.code, "Vite did not transform dir_browser_view_island.js");
  assert.match(transformed.code, /DirBrowserList/);
});
