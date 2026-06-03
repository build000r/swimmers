import test from "node:test";
import assert from "node:assert/strict";

import { createTerminalSearchLinksController } from "./terminal_search_links.js";

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

function shortenUrl(raw) {
  if (!raw) return "";
  return raw.length > 72 ? `${raw.slice(0, 69)}...` : raw;
}

function createHarness(overrides = {}) {
  const calls = {
    schedules: 0,
    hudRenders: 0,
    searchStatuses: [],
    utilityStatuses: [],
    syncTerminalTools: 0,
    clipboardWrites: [],
    openedUrls: [],
  };
  const state = {
    terminal: null,
    searchQuery: "",
    searchState: null,
    selectMode: false,
    selectionAnchor: null,
    selectionFocus: null,
    hoveredLinkUrl: "",
    activeSheet: null,
    terminalMirrorText: "",
    ...overrides.state,
  };
  const el = {
    terminalLinkTools: { classList: new MockClassList() },
    terminalLinkText: { textContent: "" },
    terminalFallback: { textContent: "" },
    ...overrides.el,
  };
  const runtime = {
    state,
    el,
    terminalSupports(methodName) {
      return Boolean(state.terminal && typeof state.terminal[methodName] === "function");
    },
    hasLiveTerminal() {
      return Boolean(state.terminal);
    },
    scheduleRender() {
      calls.schedules += 1;
    },
    renderHudSurface() {
      calls.hudRenders += 1;
    },
    setSearchStatus(label, muted) {
      state.searchLabel = label;
      state.searchMuted = Boolean(muted);
      calls.searchStatuses.push({ label, muted: Boolean(muted) });
    },
    setUtilityStatus(label, muted, ttlMs) {
      state.utilityLabel = label;
      state.utilityMuted = Boolean(muted);
      calls.utilityStatuses.push({ label, muted: Boolean(muted), ttlMs: ttlMs ?? 0 });
    },
    defaultUtilityLabel() {
      return state.hoveredLinkUrl
        ? `Cmd/Ctrl-click to open ${shortenUrl(state.hoveredLinkUrl)}.`
        : "Cmd/Ctrl-click a terminal link to open it.";
    },
    shortenUrl,
    currentSession() {
      return overrides.currentSession === false ? null : { session_id: "sess_0" };
    },
    frankenTermLinkPolicy() {
      return { allowHttp: Boolean(overrides.allowHttp) };
    },
    surfaceBusy() {
      return Boolean(overrides.surfaceBusy);
    },
    withSurfaceOperation(_label, callback) {
      if (overrides.deferredSurfaceOperation) {
        return { deferred: true };
      }
      return { deferred: false, value: callback() };
    },
    mouseCell() {
      return overrides.mouseCell ?? { x: 2, y: 3 };
    },
    syncTerminalTools() {
      calls.syncTerminalTools += 1;
    },
    navigatorRef: {
      clipboard: {
        async writeText(text) {
          calls.clipboardWrites.push(text);
        },
      },
    },
    windowRef: {
      open(url, target, features) {
        calls.openedUrls.push({ url, target, features });
      },
    },
    URLImpl: URL,
    ...overrides.runtime,
  };

  return {
    state,
    el,
    calls,
    controller: createTerminalSearchLinksController(runtime),
  };
}

test("terminal search controller preserves query apply, clear, and match cycling", () => {
  const { state, calls, controller } = createHarness();
  const terminalCalls = [];
  state.terminal = {
    setSearchQuery(query, anchor) {
      terminalCalls.push(["setSearchQuery", query, anchor]);
      return { matchCount: 3, activeMatchIndex: 1 };
    },
    clearSearch() {
      terminalCalls.push(["clearSearch"]);
    },
    searchNext() {
      terminalCalls.push(["searchNext"]);
      return { matchCount: 3, activeMatchIndex: 2 };
    },
    searchPrev() {
      terminalCalls.push(["searchPrev"]);
      return { matchCount: 3, activeMatchIndex: 0 };
    },
  };

  controller.applySearchQuery("needle");
  controller.cycleSearchMatch(1);
  controller.cycleSearchMatch(-1);
  controller.applySearchQuery("");

  assert.deepEqual(terminalCalls, [
    ["setSearchQuery", "needle", null],
    ["searchNext"],
    ["searchPrev"],
    ["clearSearch"],
  ]);
  assert.deepEqual(calls.searchStatuses.map((item) => item.label), [
    "2/3 matches",
    "3/3 matches",
    "1/3 matches",
    "Search idle",
  ]);
  assert.equal(state.searchQuery, "");
  assert.equal(calls.schedules, 4);
  assert.equal(calls.hudRenders, 2);
});

test("terminal selection controller preserves normalized ranges and copy behavior", async () => {
  const { state, calls, controller } = createHarness({
    state: { selectMode: true, selectionAnchor: 4, selectionFocus: 9 },
  });
  const terminalCalls = [];
  state.terminal = {
    setSelectionRange(start, end) {
      terminalCalls.push(["setSelectionRange", start, end]);
    },
    clearSelection() {
      terminalCalls.push(["clearSelection"]);
    },
    copySelection() {
      terminalCalls.push(["copySelection"]);
      return "selected terminal text";
    },
  };

  controller.setTerminalSelectionRange(8, 3);
  await controller.copyTerminalSelection();
  controller.setSelectMode(false);

  assert.deepEqual(terminalCalls, [
    ["setSelectionRange", 3, 9],
    ["copySelection"],
    ["clearSelection"],
  ]);
  assert.equal(state.selectionFocus, null);
  assert.equal(state.selectionAnchor, null);
  assert.equal(state.selectMode, false);
  assert.deepEqual(calls.clipboardWrites, ["selected terminal text"]);
  assert.equal(calls.utilityStatuses.at(-1).label, "Copied 22 characters from the terminal.");
  assert.equal(calls.syncTerminalTools, 1);
});

test("terminal link controller preserves hover highlighting and link tool visibility", () => {
  const { state, el, calls, controller } = createHarness();
  const terminalCalls = [];
  state.terminal = {
    linkUrlAt(x, y) {
      terminalCalls.push(["linkUrlAt", x, y]);
      return "https://example.com/path";
    },
    linkAt(x, y) {
      terminalCalls.push(["linkAt", x, y]);
      return 42;
    },
    setHoveredLinkId(id) {
      terminalCalls.push(["setHoveredLinkId", id]);
    },
  };

  controller.updateHoveredLink({});

  assert.deepEqual(terminalCalls, [
    ["linkUrlAt", 2, 3],
    ["linkAt", 2, 3],
    ["setHoveredLinkId", 42],
  ]);
  assert.equal(state.hoveredLinkUrl, "https://example.com/path");
  assert.equal(el.terminalLinkTools.classList.contains("hidden"), false);
  assert.equal(el.terminalLinkText.textContent, "https://example.com/path");
  assert.equal(calls.utilityStatuses.at(-1).label, "Cmd/Ctrl-click to open https://example.com/path.");
  assert.equal(calls.syncTerminalTools, 1);
  assert.equal(calls.schedules, 1);
});

test("terminal link controller preserves URL policy and drained click behavior", () => {
  const { state, calls, controller } = createHarness();
  state.terminal = {
    drainLinkClicks() {
      return [
        { url: "http://public.example/path" },
        { href: "https://example.com/ok" },
        { url: "ssh://example.com/rejected" },
        { url: "https://example.com/blocked", openAllowed: false, openReason: "terminal blocked link" },
      ];
    },
  };

  controller.drainTerminalLinkClicks();

  assert.deepEqual(calls.openedUrls, [
    {
      url: "https://example.com/ok",
      target: "_blank",
      features: "noopener,noreferrer",
    },
  ]);
  assert.deepEqual(calls.utilityStatuses.map((item) => item.label), [
    "Blocked non-local HTTP link: http://public.example/path",
    "Blocked unsupported link protocol: ssh:",
    "terminal blocked link",
  ]);
});

test("terminal frame and hovered link copy helpers preserve clipboard fallbacks", async () => {
  const { state, el, calls, controller } = createHarness({
    state: {
      hoveredLinkUrl: "https://example.com/copy",
      terminalMirrorText: "",
    },
  });
  el.terminalFallback.textContent = "fallback frame text";

  assert.equal(await controller.copyHoveredLink(), true);
  assert.equal(await controller.copyTerminalFrameText(), true);

  assert.deepEqual(calls.clipboardWrites, [
    "https://example.com/copy",
    "fallback frame text",
  ]);
  assert.deepEqual(calls.utilityStatuses.map((item) => item.label), [
    "Copied https://example.com/copy.",
    "Copied 19 visible terminal characters.",
  ]);
});
