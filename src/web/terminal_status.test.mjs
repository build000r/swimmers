import test from "node:test";
import assert from "node:assert/strict";

import {
  createTerminalStatusController,
} from "./terminal_status.js";

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

function createRuntime(overrides = {}) {
  const state = {
    sessions: [
      {
        session_id: "sess_0",
        tmux_name: "swordsman",
        state: "attention",
        attention: true,
      },
    ],
    selectedSessionId: "sess_0",
    connectionLabel: "disconnected",
    connectionMuted: false,
    modeLabel: "auth unknown",
    modeMuted: true,
    searchLabel: "Search idle",
    searchMuted: true,
    searchQuery: "",
    selectMode: false,
    readOnly: false,
    terminalFallbackActive: false,
    terminal: null,
    ws: null,
    backendHealth: null,
    hoveredLinkUrl: "",
    utilityLabel: "Cmd/Ctrl-click a terminal link to open it.",
    utilityMuted: true,
    utilityMessageTimer: null,
    ...overrides.state,
  };
  const el = {
    terminalStatusStrip: { textContent: "" },
    ...overrides.el,
  };
  const documentRef = {
    title: "swimmers",
    body: { classList: new MockClassList() },
    ...overrides.documentRef,
  };
  const renderCalls = [];
  const timeouts = [];
  const clearCalls = [];
  const runtime = {
    state,
    el,
    boot: { franken_term_available: false, ...overrides.boot },
    defaultDocumentTitle: "swimmers",
    currentSession() {
      return state.sessions.find((session) => session.session_id === state.selectedSessionId) ?? null;
    },
    sessionDisplayName(session) {
      return session.tmux_name || session.session_id;
    },
    sessionNeedsAttention(session) {
      return Boolean(session?.attention);
    },
    backendHealthWarningText(payload) {
      return payload?.warning || "";
    },
    shortenUrl,
    renderHudSurface() {
      renderCalls.push("render");
    },
    documentRef,
    setTimeoutRef(callback, delay) {
      const timer = `timer-${timeouts.length + 1}`;
      timeouts.push({ callback, delay, timer });
      return timer;
    },
    clearTimeoutRef(timer) {
      clearCalls.push(timer);
    },
    webSocketOpenReadyState() {
      return 1;
    },
    ...overrides.runtime,
  };

  return {
    clearCalls,
    controller: createTerminalStatusController(runtime),
    documentRef,
    el,
    renderCalls,
    state,
    timeouts,
  };
}

test("terminal status controller preserves strip ordering and document attention state", () => {
  const { controller, documentRef, el, state } = createRuntime({
    state: {
      connectionLabel: "live",
      terminalFallbackActive: true,
      ws: { readyState: 1 },
      searchQuery: "disk",
      searchLabel: "match 1",
      selectMode: true,
      backendHealth: { warning: "persistence degraded: save_sessions: disk full" },
    },
  });

  controller.syncTerminalStatusStrip();

  assert.equal(
    el.terminalStatusStrip.textContent,
    "swordsman  |  attention  |  live  |  operator  |  fallback live  |  match 1  |  selecting  |  persistence degraded: save_sessions: disk full",
  );
  assert.equal(documentRef.body.classList.contains("backend-health-degraded"), true);
  assert.equal(documentRef.body.classList.contains("session-attention"), true);
  assert.equal(documentRef.title, "(!) swordsman - swimmers");
  assert.equal(state.backendHealth.warning, "persistence degraded: save_sessions: disk full");
});

test("terminal status controller preserves fallback mode labels", () => {
  const { controller, state } = createRuntime();

  assert.equal(controller.terminalModeLabel(), "snapshot mode");

  state.terminal = {};
  assert.equal(controller.terminalModeLabel(), "FrankenTerm live");

  state.terminal = null;
  state.terminalFallbackActive = true;
  state.ws = { readyState: 0 };
  assert.equal(controller.terminalModeLabel(), "snapshot fallback");

  state.ws = { readyState: 1 };
  assert.equal(controller.terminalModeLabel(), "fallback live");

  state.selectedSessionId = "";
  assert.equal(controller.terminalModeLabel(), "no session");
});

test("terminal status setters preserve state updates, render calls, and health normalization", () => {
  const { controller, documentRef, el, renderCalls, state } = createRuntime({
    state: { searchQuery: "needle", readOnly: true },
  });

  controller.setConnectionStatus("live", true);
  controller.setModeStatus("operator", false);
  controller.setSearchStatus("match 2", false);
  controller.applyBackendHealth({ warning: "thought bridge degraded: model timeout" });

  assert.equal(state.connectionLabel, "live");
  assert.equal(state.connectionMuted, true);
  assert.equal(state.modeLabel, "operator");
  assert.equal(state.modeMuted, false);
  assert.equal(state.searchLabel, "match 2");
  assert.equal(state.searchMuted, false);
  assert.equal(state.backendHealth.warning, "thought bridge degraded: model timeout");
  assert.equal(documentRef.body.classList.contains("backend-health-degraded"), true);
  assert.ok(el.terminalStatusStrip.textContent.includes("observer"));
  assert.ok(el.terminalStatusStrip.textContent.includes("match 2"));
  assert.equal(renderCalls.length, 4);

  controller.applyBackendHealth("not an object");

  assert.equal(state.backendHealth, null);
  assert.equal(documentRef.body.classList.contains("backend-health-degraded"), false);
  assert.equal(renderCalls.length, 5);
});

test("utility status preserves default labels, timer clearing, and muted reset", () => {
  const rawUrl = `https://example.com/${"a".repeat(90)}`;
  const { clearCalls, controller, renderCalls, state, timeouts } = createRuntime({
    state: { hoveredLinkUrl: rawUrl },
  });

  assert.equal(
    controller.defaultUtilityLabel(),
    `Cmd/Ctrl-click to open ${shortenUrl(rawUrl)}.`,
  );

  controller.setUtilityStatus("Copied link.", true, 2200);

  assert.equal(state.utilityLabel, "Copied link.");
  assert.equal(state.utilityMuted, true);
  assert.equal(state.utilityMessageTimer, "timer-1");
  assert.equal(renderCalls.length, 1);
  assert.deepEqual(timeouts.map(({ delay }) => delay), [2200]);

  timeouts[0].callback();

  assert.deepEqual(clearCalls, ["timer-1"]);
  assert.equal(state.utilityLabel, `Cmd/Ctrl-click to open ${shortenUrl(rawUrl)}.`);
  assert.equal(state.utilityMuted, false);
  assert.equal(state.utilityMessageTimer, null);
  assert.equal(renderCalls.length, 2);
});
