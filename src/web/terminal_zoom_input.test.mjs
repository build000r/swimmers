import test from "node:test";
import assert from "node:assert/strict";

import { createTerminalZoomInputController } from "./terminal_zoom_input.js";

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

function mockElement(overrides = {}) {
  return {
    classList: new MockClassList(),
    attributes: new Map(),
    disabled: false,
    style: {},
    textContent: "",
    value: "",
    scrollHeight: 0,
    setAttribute(name, value) {
      this.attributes.set(name, String(value));
    },
    getAttribute(name) {
      return this.attributes.get(name) ?? null;
    },
    querySelectorAll() {
      return [];
    },
    ...overrides,
  };
}

function memoryStorage(initial = {}) {
  const entries = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return entries.has(key) ? entries.get(key) : null;
    },
    setItem(key, value) {
      entries.set(key, String(value));
    },
  };
}

function fixture(overrides = {}) {
  const calls = [];
  const keyStripButton = mockElement();
  const state = {
    hud: null,
    terminal: null,
    terminalZoom: 1,
    terminalFallbackActive: false,
    trogdorAtlasOpen: false,
    readOnly: false,
    mobileKeyboardActive: false,
    selectedSessionId: "sess_0",
    ...overrides.state,
  };
  const el = {
    terminalControlStrip: mockElement(),
    terminalZoomOut: mockElement(),
    terminalZoomReset: mockElement(),
    terminalZoomIn: mockElement(),
    terminalMobileKeyboard: mockElement(),
    terminalCopyFrame: mockElement(),
    terminalInputDock: mockElement(),
    terminalInlineInput: mockElement(),
    terminalInputSend: mockElement(),
    terminalInputEcho: mockElement(),
    terminalFallback: mockElement(),
    terminalKeyStrip: mockElement({
      querySelectorAll(selector) {
        return selector === "button[data-terminal-key]" ? [keyStripButton] : [];
      },
    }),
    ...overrides.el,
  };
  const storage = overrides.storage || memoryStorage({ zoom: "0.80" });
  const windowRef = overrides.windowRef || {
    location: { href: "http://localhost/pond?zoom=1.40&panel=terminal" },
    history: {
      replaceState(_state, _title, url) {
        calls.push(["replaceState", String(url)]);
      },
    },
  };
  const documentRef = overrides.documentRef || { body: mockElement() };
  const controller = createTerminalZoomInputController({
    state,
    el,
    storage,
    windowRef,
    documentRef,
    URLImpl: URL,
    currentSession: () => overrides.currentSession ?? { session_id: "sess_0" },
    updateTerminalFallbackText: (text) => {
      calls.push(["updateTerminalFallbackText", text]);
      el.terminalFallback.textContent = text;
    },
    sendLineToSession: async (sessionId, text) => {
      calls.push(["sendLineToSession", sessionId, text, el.terminalInputEcho.textContent, el.terminalFallback.textContent]);
      return overrides.sendLineResult;
    },
    rememberSendHistory: (text) => calls.push(["rememberSendHistory", text, el.terminalInlineInput.value]),
    refreshSessions: () => calls.push(["refreshSessions"]),
    setConnectionStatus: (...args) => calls.push(["setConnectionStatus", ...args]),
    setUtilityStatus: (...args) => calls.push(["setUtilityStatus", ...args]),
    measureAndResizeSurface: (...args) => calls.push(["measureAndResizeSurface", ...args]),
    focusTerminalInputSurface: (...args) => calls.push(["focusTerminalInputSurface", ...args]),
    terminalZoomStorageKey: "zoom",
    minZoom: 0.5,
    maxZoom: 2,
    step: 0.1,
    ...overrides.runtime,
  });
  return { calls, controller, documentRef, el, keyStripButton, state, storage, windowRef };
}

test("terminal zoom controller preserves URL precedence, storage persistence, and setZoom dispatch", () => {
  const { calls, controller, el, state, storage } = fixture();
  state.hud = { setZoom: (zoom) => calls.push(["hud.setZoom", zoom]) };
  state.terminal = { setZoom: (zoom) => calls.push(["terminal.setZoom", zoom]) };

  assert.equal(controller.loadTerminalZoom(new URL("http://localhost/pond?zoom=1.40")), 1.4000000000000001);

  controller.setTerminalZoom(1.2, { announce: true });

  assert.equal(state.terminalZoom, 1.2000000000000002);
  assert.deepEqual(calls.slice(0, 3), [
    ["hud.setZoom", 1.2000000000000002],
    ["terminal.setZoom", 1.2000000000000002],
    ["replaceState", "http://localhost/pond?zoom=1.20&panel=terminal"],
  ]);
  assert.equal(storage.getItem("zoom"), "1.20");
  assert.equal(el.terminalZoomReset.textContent, "120%");
  assert.equal(el.terminalMobileKeyboard.getAttribute("aria-pressed"), "false");
  assert.deepEqual(calls.at(-1), ["setUtilityStatus", "Terminal zoom 120%.", false, 1600]);
});

test("terminal input dock controller preserves fallback projection and send order", async () => {
  const { calls, controller, documentRef, el, keyStripButton, state } = fixture({
    state: { terminalFallbackActive: true },
  });
  el.terminalFallback.textContent = "$ ";
  el.terminalInlineInput.value = "echo dock";
  el.terminalInlineInput.scrollHeight = 72;

  assert.equal(await controller.submitTerminalInputDock(), true);

  assert.deepEqual(calls, [
    ["updateTerminalFallbackText", "$ \n› echo dock\n"],
    ["sendLineToSession", "sess_0", "echo dock", "› pending: echo dock", "$ \n› echo dock\n"],
    ["rememberSendHistory", "echo dock", "echo dock"],
    ["refreshSessions"],
  ]);
  assert.equal(el.terminalInlineInput.value, "");
  assert.equal(el.terminalInlineInput.style.height, "72px");
  assert.equal(el.terminalInputSend.disabled, true);
  assert.equal(keyStripButton.disabled, false);
  assert.equal(documentRef.body.classList.contains("terminal-input-dock-visible"), true);
});

test("terminal input dock guards against double-send while a send is in flight", async () => {
  let resolveSend;
  const sendCalls = [];
  const { controller, el, state } = fixture({
    runtime: {
      sendLineToSession: (sessionId, text) => {
        sendCalls.push([sessionId, text]);
        return new Promise((resolve) => {
          resolveSend = resolve;
        });
      },
    },
  });
  el.terminalInlineInput.value = "echo once";

  const first = controller.submitTerminalInputDock();
  // The input is only cleared on success, so a second submit before the first
  // resolves must be rejected rather than resending the same line.
  assert.equal(await controller.submitTerminalInputDock(), false);
  assert.equal(sendCalls.length, 1);
  assert.equal(state.sending, true);

  resolveSend();
  assert.equal(await first, true);
  assert.equal(state.sending, false);
});
