import test from "node:test";
import assert from "node:assert/strict";

import {
  createSendController,
  deliveredGroupInputSessionIds,
  loadedSendHistory,
  nextSendHistory,
  sendHistoryHtml,
  sendModeValueFromElement,
  sendTargetReadyState,
} from "./send_controller.js";

function memoryStorage(initial = {}) {
  const entries = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return entries.has(key) ? entries.get(key) : null;
    },
    setItem(key, value) {
      entries.set(key, String(value));
    },
    removeItem(key) {
      entries.delete(key);
    },
    dump() {
      return Object.fromEntries(entries.entries());
    },
  };
}

function jsonResponse(payload) {
  return {
    async json() {
      return payload;
    },
  };
}

function fixture(overrides = {}) {
  const calls = [];
  const state = {
    pendingInputMessages: new Map(),
    readOnly: false,
    selectedSessionId: "sess_0",
    sendHistory: [],
    sendTarget: null,
    ws: null,
    ...overrides.state,
  };
  const el = {
    sendHistory: { innerHTML: "" },
    sendHint: { textContent: "" },
    sendInput: {
      disabled: false,
      placeholder: "",
      value: "",
      focus() {
        calls.push(["focusSendInput"]);
      },
    },
    sendMode: { disabled: false, value: "line" },
    sendSheetTitle: { textContent: "" },
    ...overrides.el,
  };
  const storage = overrides.storage || memoryStorage();
  const controller = createSendController({
    state,
    el,
    storage,
    sendHistoryKey: "history",
    sendHistoryLimit: 3,
    WebSocketClass: { OPEN: 1 },
    ElementClass: class TestElement {},
    apiFetch: async (path, init = {}) => {
      calls.push(["apiFetch", path, init.body ? JSON.parse(init.body) : null]);
      return jsonResponse(overrides.apiBody ?? { delivered: true });
    },
    responseJsonOrNull: async (response) => response.json(),
    currentSession: () => overrides.currentSession ?? { session_id: "sess_0", tmux_name: "Agent Zero" },
    normalizeSessionId: (value) => String(value || "").trim(),
    nextInputMessageId: () => "input-1",
    updateInputDeliveryStatus: (...args) => calls.push(["updateInputDeliveryStatus", ...args]),
    sendTerminalText: (text) => calls.push(["sendTerminalText", text]),
    setTerminalInputEcho: (text) => calls.push(["setTerminalInputEcho", text]),
    markTrogdorSessionsResponded: (ids) => calls.push(["markTrogdorSessionsResponded", ids]),
    setUtilityStatus: (...args) => calls.push(["setUtilityStatus", ...args]),
    closeSheets: () => calls.push(["closeSheets"]),
    openSheet: (sheet) => calls.push(["openSheet", sheet]),
    refreshSessions: async () => calls.push(["refreshSessions"]),
    syncSheetActionAvailability: () => calls.push(["syncSheetActionAvailability"]),
    escapeHtml: (value) => String(value).replaceAll("<", "&lt;").replaceAll(">", "&gt;"),
    ...overrides.runtime,
  });
  return { calls, controller, el, state, storage };
}

test("send history helpers preserve load, de-dupe, limit, display order, and escaping", () => {
  const storage = memoryStorage({
    history: JSON.stringify([" one ", "", 2, "three", "four"]),
  });
  assert.deepEqual(loadedSendHistory(storage, "history", 3), [" one ", "2", "three"]);
  assert.deepEqual(nextSendHistory(["one", "two", "three"], " two ", 3), ["two", "one", "three"]);
  assert.deepEqual(nextSendHistory(["one", "two", "three"], "four", 3), ["four", "one", "two"]);
  assert.deepEqual(nextSendHistory(["one"], "   ", 3), ["one"]);
  assert.equal(
    sendHistoryHtml(["launch   <repo>", "abcdefghijklmnopqrstuvwxyz0123456789abcdefghi"], (value) => String(value).replaceAll("<", "&lt;").replaceAll(">", "&gt;")),
    '<button class="ghost-button" type="button" data-send-history-index="0" title="launch &lt;repo&gt;">launch &lt;repo&gt;</button>' +
      '<button class="ghost-button" type="button" data-send-history-index="1" title="abcdefghijklmnopqrstuvwxyz0123456789abcdefghi">abcdefghijklmnopqrstuvwxyz0123456789abc...</button>',
  );
});

test("send target and mode helpers preserve read-only, selected, group, and paste gates", () => {
  assert.equal(sendModeValueFromElement({ value: "paste" }), "paste");
  assert.equal(sendModeValueFromElement({ value: "raw" }), "line");
  assert.equal(sendTargetReadyState({ readOnly: true, currentSession: {} }), false);
  assert.equal(sendTargetReadyState({ currentSession: { session_id: "sess_0" } }), true);
  assert.equal(sendTargetReadyState({ sendTarget: { type: "group", sessionIds: ["a"] } }), false);
  assert.equal(sendTargetReadyState({ sendTarget: { type: "group", sessionIds: ["a", "b"] } }), true);
  assert.equal(sendTargetReadyState({ sendTarget: { type: "session", sessionId: " sess_0 " }, normalizeSessionId: (value) => String(value || "").trim() }), true);
});

test("controller uses websocket fast path for selected line and paste sends", async () => {
  const sent = [];
  const { calls, controller, state } = fixture({
    state: {
      ws: {
        readyState: 1,
        send(message) {
          sent.push(JSON.parse(message));
        },
      },
    },
  });

  await controller.sendLineToSession("sess_0", "continue");
  await controller.sendRawTextToSession("sess_0", "draft");

  assert.deepEqual(sent, [{ type: "submit_line", data: "continue", clientMessageId: "input-1" }]);
  assert.deepEqual(state.pendingInputMessages.get("input-1"), { text: "continue", status: "pending", detail: "" });
  assert.deepEqual(calls, [
    ["updateInputDeliveryStatus", "input-1", "pending"],
    ["markTrogdorSessionsResponded", ["sess_0"]],
    ["sendTerminalText", "draft"],
  ]);
});

test("controller falls back to API sends and summarizes successful group deliveries", async () => {
  const { calls, controller } = fixture({
    apiBody: {
      results: [
        { session_id: "ready", ok: true },
        { session_id: "stale", ok: false },
      ],
    },
  });

  await controller.sendLineToSession("sess_0", "continue");
  await controller.sendRawTextToSession("sess_0", "draft");
  const group = await controller.sendGroupLine(["ready", "", "stale"], "continue");

  assert.deepEqual(calls.filter((call) => call[0] === "apiFetch"), [
    ["apiFetch", "/v1/sessions/sess_0/input", { text: "continue", submit: true }],
    ["apiFetch", "/v1/sessions/sess_0/input", { text: "draft" }],
    ["apiFetch", "/v1/sessions/group-input", { session_ids: ["ready", "stale"], text: "continue" }],
  ]);
  assert.deepEqual(calls.filter((call) => call[0] === "markTrogdorSessionsResponded"), [
    ["markTrogdorSessionsResponded", ["sess_0"]],
    ["markTrogdorSessionsResponded", ["ready"]],
  ]);
  assert.deepEqual(group, {
    delivered: 1,
    skipped: 1,
    total: 2,
    deliveredSessionIds: ["ready"],
    results: [
      { session_id: "ready", ok: true },
      { session_id: "stale", ok: false },
    ],
  });
  assert.deepEqual(deliveredGroupInputSessionIds({ results: [{ session_id: " a ", ok: true }, { session_id: "b", ok: false }] }, (value) => String(value || "").trim()), ["a"]);
});

test("form submit preserves history, statuses, close, refresh, and failure availability sync", async () => {
  const success = fixture();
  success.state.selectedSessionId = "sess_0";
  success.el.sendInput.value = "continue\n";

  assert.equal(await success.controller.handleSendFormSubmit({ preventDefault() {} }), true);

  assert.deepEqual(success.state.sendHistory, ["continue"]);
  assert.equal(success.storage.dump().history, JSON.stringify(["continue"]));
  assert.equal(success.el.sendInput.value, "");
  assert.equal(success.state.sendTarget, null);
  assert.deepEqual(success.calls.slice(-3), [
    ["setUtilityStatus", "Sent line to sess_0.", false, 2200],
    ["closeSheets"],
    ["refreshSessions"],
  ]);

  const failure = fixture({
    runtime: {
      apiFetch: async () => jsonResponse({ delivered: false, message: "not ready" }),
    },
  });
  failure.el.sendInput.value = "continue";

  assert.equal(await failure.controller.handleSendFormSubmit({ preventDefault() {} }), false);
  assert.deepEqual(failure.calls.slice(-2), [
    ["setUtilityStatus", "Send failed: not ready", true, 3200],
    ["syncSheetActionAvailability"],
  ]);
});

test("open send sheet resets line mode, renders history, updates hints, and opens sheet", () => {
  const { calls, controller, el, state } = fixture();
  state.sendHistory = ["previous"];

  controller.openSendSheet({ type: "group", sessionIds: ["a", "b"], label: "Batch" });

  assert.equal(state.sendTarget.type, "group");
  assert.equal(el.sendSheetTitle.textContent, "Send Batch");
  assert.equal(el.sendMode.value, "line");
  assert.equal(el.sendMode.disabled, true);
  assert.equal(el.sendInput.value, "");
  assert.equal(el.sendInput.placeholder, "Send to 2 batch agents.");
  assert.equal(el.sendHint.textContent, "Batch sends submit the shared text to every ready agent.");
  assert.ok(el.sendHistory.innerHTML.includes("previous"));
  assert.deepEqual(calls.slice(-2), [["openSheet", "send"], ["syncSheetActionAvailability"]]);
});

test("history click adapter recalls stored text and focuses the send input", () => {
  class TestElement {}
  const button = Object.assign(new TestElement(), {
    dataset: { sendHistoryIndex: "1" },
    closest(selector) {
      return selector === "[data-send-history-index]" ? this : null;
    },
  });
  const { calls, controller, el, state } = fixture({
    runtime: { ElementClass: TestElement },
  });
  state.sendHistory = ["first", "second"];

  controller.handleSendHistoryClick({ type: "click", target: button });

  assert.equal(el.sendInput.value, "second");
  assert.deepEqual(calls.slice(-1), [["focusSendInput"]]);
});
