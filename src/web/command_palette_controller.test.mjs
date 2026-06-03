import test from "node:test";
import assert from "node:assert/strict";

import { createCommandPaletteController } from "./command_palette_controller.js";

class MockClassList {
  constructor() {
    this.values = new Set();
  }

  toggle(name, force) {
    if (force) {
      this.values.add(name);
      return true;
    }
    this.values.delete(name);
    return false;
  }

  contains(name) {
    return this.values.has(name);
  }
}

class MockElement {
  constructor(id) {
    this.id = id;
    this.classList = new MockClassList();
    this.attributes = new Map();
    this.value = "";
    this.innerHTML = "";
    this.selected = false;
    this.focused = false;
    this.firstCheckbox = null;
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  focus() {
    this.focused = true;
  }

  select() {
    this.selected = true;
  }

  querySelector(selector) {
    return selector === ".dir-row-check:not(:disabled)" ? this.firstCheckbox : null;
  }
}

function makeController() {
  const calls = [];
  const rafQueue = [];
  const state = {
    activeSheet: null,
    paletteIndex: 0,
    paletteItems: [],
    readOnly: false,
    searchQuery: "needle",
    selectedSessionId: "sess-1",
    sendTarget: { sessionId: "sess-1" },
    sessions: [
      { session_id: "sess-1", tmux_name: "alpha", state: "idle" },
      { session_id: "sess-2", tmux_name: "beta", state: "busy" },
    ],
    token: "secret-token",
    dirBrowser: { group: "batch-a" },
  };
  const el = Object.fromEntries([
    "modalRoot",
    "paletteSheet",
    "searchSheet",
    "thoughtConfigSheet",
    "nativeSheet",
    "sendSheet",
    "authSheet",
    "createSheet",
    "mermaidSheet",
    "paletteSearch",
    "paletteResults",
    "terminalSearch",
    "thoughtConfigModel",
    "nativeApp",
    "sendInput",
    "tokenInput",
    "createCwd",
    "dirsList",
    "mermaidRefreshButton",
  ].map((id) => [id, new MockElement(id)]));
  const documentRef = { body: new MockElement("body") };
  const controller = createCommandPaletteController({
    state,
    el,
    documentRef,
    requestAnimationFrameRef: (callback) => rafQueue.push(callback),
    currentSession: () => state.sessions.find((session) => session.session_id === state.selectedSessionId) || null,
    copyTerminalFrameText: () => calls.push(["copyTerminalFrameText"]),
    clampInt: (value, fallback, min, max) => Math.max(min, Math.min(max, Number.isFinite(value) ? Math.trunc(value) : fallback)),
    selectSession: async (sessionId) => calls.push(["selectSession", sessionId]),
    handleSurfaceAction: async (action) => calls.push(["handleSurfaceAction", action.actionId]),
    syncSheetActionAvailability: () => calls.push(["syncSheetActionAvailability"]),
    renderHudSurface: () => calls.push(["renderHudSurface"]),
    focusTerminalInputSurface: (options) => calls.push(["focusTerminalInputSurface", options]),
    clearCreateBatchSelection: () => calls.push(["clearCreateBatchSelection"]),
    openCreateSheet: async () => calls.push(["openCreateSheet"]),
    refreshThoughtConfig: async () => calls.push(["refreshThoughtConfig"]),
    refreshNativeStatus: async () => calls.push(["refreshNativeStatus"]),
    refreshMermaidArtifact: async () => calls.push(["refreshMermaidArtifact"]),
  });
  return { calls, controller, documentRef, el, rafQueue, state };
}

test("openCommandPalette resets search, renders filtered items, toggles modal classes, and schedules focus", () => {
  const { calls, controller, documentRef, el, rafQueue, state } = makeController();
  el.paletteSearch.value = "beta";
  state.paletteIndex = 7;

  controller.openCommandPalette();

  assert.equal(state.activeSheet, "palette");
  assert.equal(el.paletteSearch.value, "");
  assert.equal(state.paletteIndex, 0);
  assert.equal(documentRef.body.classList.contains("sheet-open"), true);
  assert.equal(el.modalRoot.classList.contains("visible"), true);
  assert.equal(el.modalRoot.getAttribute("aria-hidden"), "false");
  assert.equal(el.paletteSheet.classList.contains("hidden"), false);
  assert.equal(el.searchSheet.classList.contains("hidden"), true);
  assert.ok(el.paletteResults.innerHTML.includes("Focus terminal"));
  assert.deepEqual(calls.slice(0, 2), [["syncSheetActionAvailability"], ["renderHudSurface"]]);

  rafQueue.shift()();
  assert.equal(el.paletteSearch.focused, true);
  assert.equal(el.paletteSearch.selected, true);
});

test("openSheet runs sheet-specific side effects and focus targets", () => {
  const { calls, controller, el, rafQueue, state } = makeController();

  controller.openSheet("search");
  assert.equal(el.terminalSearch.value, "needle");
  rafQueue.shift()();
  assert.equal(el.terminalSearch.focused, true);
  assert.equal(el.terminalSearch.selected, true);

  controller.openSheet("auth");
  assert.equal(el.tokenInput.value, "secret-token");
  rafQueue.shift()();
  assert.equal(el.tokenInput.focused, true);

  controller.openSheet("create");
  rafQueue.shift()();
  assert.equal(el.createCwd.focused, true);

  controller.openSheet("thought-config");
  controller.openSheet("native");
  controller.openSheet("mermaid");

  assert.ok(calls.some((call) => call[0] === "openCreateSheet"));
  assert.ok(calls.some((call) => call[0] === "refreshThoughtConfig"));
  assert.ok(calls.some((call) => call[0] === "refreshNativeStatus"));
  assert.ok(calls.some((call) => call[0] === "refreshMermaidArtifact"));
  assert.equal(state.activeSheet, "mermaid");
});

test("closeSheets clears send and create state, hides modal, and refocuses terminal immediately", () => {
  const { calls, controller, documentRef, el, state } = makeController();

  state.activeSheet = "send";
  controller.closeSheets();
  assert.equal(state.sendTarget, null);
  assert.equal(state.activeSheet, null);
  assert.equal(documentRef.body.classList.contains("sheet-open"), false);
  assert.equal(el.modalRoot.getAttribute("aria-hidden"), "true");
  assert.deepEqual(calls.at(-1), ["focusTerminalInputSurface", { preventScroll: true }]);

  state.activeSheet = "create";
  state.dirBrowser.group = "batch-b";
  controller.closeSheets();
  assert.equal(state.dirBrowser.group, "");
  assert.ok(calls.some((call) => call[0] === "clearCreateBatchSelection"));
});

test("runCommandPaletteItem preserves disabled no-op, action, actionId, and session selection paths", async () => {
  const { calls, controller, state } = makeController();
  let actionCalls = 0;

  state.activeSheet = "palette";
  assert.equal(await controller.runCommandPaletteItem({ disabled: true, action: () => { actionCalls += 1; } }), false);
  assert.equal(actionCalls, 0);
  assert.equal(state.activeSheet, "palette");

  assert.equal(await controller.runCommandPaletteItem({ action: async () => { actionCalls += 1; } }), true);
  assert.equal(actionCalls, 1);
  assert.equal(state.activeSheet, null);

  state.activeSheet = "palette";
  assert.equal(await controller.runCommandPaletteItem({ actionId: "open_auth" }), true);
  assert.ok(calls.some((call) => call[0] === "handleSurfaceAction" && call[1] === "open_auth"));

  state.activeSheet = "palette";
  assert.equal(await controller.runCommandPaletteItem({ sessionId: "sess-2" }), true);
  assert.ok(calls.some((call) => call[0] === "selectSession" && call[1] === "sess-2"));
});
