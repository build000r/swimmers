import test from "node:test";
import assert from "node:assert/strict";

class MockClassList {
  constructor() {
    this.values = new Set();
  }

  add(...names) {
    for (const name of names) {
      this.values.add(name);
    }
  }

  remove(...names) {
    for (const name of names) {
      this.values.delete(name);
    }
  }

  toggle(name, force) {
    if (force === undefined) {
      if (this.values.has(name)) {
        this.values.delete(name);
        return false;
      }
      this.values.add(name);
      return true;
    }
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
    this.dataset = {};
    this.style = {};
    this.attributes = new Map();
    this.listeners = new Map();
    this.value = "";
    this.checked = false;
    this.disabled = false;
    this.textContent = "";
    this.innerHTML = "";
    this.children = [];
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.clientHeight = 0;
  }

  addEventListener(name, listener) {
    const listeners = this.listeners.get(name) || [];
    listeners.push(listener);
    this.listeners.set(name, listeners);
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  focus() {
    document.activeElement = this;
  }

  blur() {
    if (document.activeElement === this) {
      document.activeElement = null;
    }
  }

  select() {}

  appendChild(child) {
    this.children.push(child);
    return child;
  }

  getBoundingClientRect() {
    return { left: 0, top: 0, width: 1280, height: 720 };
  }

  querySelector() {
    return null;
  }

  querySelectorAll() {
    return [];
  }

  contains(target) {
    return target === this;
  }

  closest() {
    return null;
  }
}

const elements = new Map();
const storage = new Map();
const originalFetch = globalThis.fetch;

function element(id) {
  if (!elements.has(id)) {
    elements.set(id, new MockElement(id));
  }
  return elements.get(id);
}

globalThis.Element = MockElement;
globalThis.document = {
  activeElement: null,
  title: "swimmers",
  body: new MockElement("body"),
  getElementById: element,
  createElement(tagName) {
    return new MockElement(String(tagName));
  },
};
if (!globalThis.navigator) {
  Object.defineProperty(globalThis, "navigator", {
    value: {},
    configurable: true,
  });
}
globalThis.localStorage = {
  getItem(key) {
    return storage.has(key) ? storage.get(key) : null;
  },
  setItem(key, value) {
    storage.set(key, String(value));
  },
  removeItem(key) {
    storage.delete(key);
  },
  clear() {
    storage.clear();
  },
};
globalThis.window = {
  __SWIMMERS_BOOT__: {
    franken_term_available: false,
    franken_term_js_url: "",
    franken_term_wasm_url: "",
    franken_term_font_url: "",
    franken_term_asset_info: null,
    follow_published_selection: false,
    focus_layout: false,
  },
  __SWIMMERS_DISABLE_AUTO_INIT__: true,
  location: new URL("http://swimmers.test/"),
  history: {
    replaceState(_state, _title, url) {
      window.location = new URL(String(url), window.location.href);
    },
  },
  matchMedia() {
    return { matches: false };
  },
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval,
};
globalThis.requestAnimationFrame = (callback) => {
  callback();
  return 1;
};
globalThis.cancelAnimationFrame = () => {};
globalThis.WebSocket = globalThis.WebSocket || { OPEN: 1 };

const { __swimmersWebTest: web } = await import("./app.js?interaction-behavior-test");

function rawSession(overrides = {}) {
  return {
    session_id: "sess_0",
    tmux_name: "swordsman",
    state: "attention",
    state_evidence: {
      confidence: "high",
      observed_at: "2026-05-04T00:00:00Z",
      cause: "test",
    },
    rest_state: "active",
    transport_health: "healthy",
    tool: "shell",
    cwd: "/tmp/swimmers",
    thought: "waiting for operator input",
    action_cues: [{ kind: "awaiting_user" }],
    attached_clients: 1,
    stale_attached_clients: 0,
    token_count: 0,
    context_limit: 0,
    ...overrides,
  };
}

function resetWebState() {
  if (web.state.snapshotTimer) {
    clearInterval(web.state.snapshotTimer);
    web.state.snapshotTimer = null;
  }
  if (web.state.refreshTimer) {
    clearTimeout(web.state.refreshTimer);
    web.state.refreshTimer = null;
  }
  if (web.state.terminalPaintProbeTimer) {
    clearTimeout(web.state.terminalPaintProbeTimer);
    web.state.terminalPaintProbeTimer = null;
  }
  if (web.state.resizeRetryTimer) {
    clearTimeout(web.state.resizeRetryTimer);
    web.state.resizeRetryTimer = null;
  }
  storage.clear();
  window.location = new URL("http://swimmers.test/");
  document.title = "swimmers";
  document.activeElement = null;
  document.body.classList = new MockClassList();
  for (const node of elements.values()) {
    node.classList = new MockClassList();
    node.style = {};
    node.attributes.clear();
    node.value = "";
    node.checked = false;
    node.disabled = false;
    node.textContent = "";
    node.innerHTML = "";
    node.scrollTop = 0;
    node.scrollHeight = 0;
    node.clientHeight = 0;
  }
  web.state.sessions = [rawSession()];
  web.state.token = "";
  web.state.selectedSessionId = null;
  web.state.followPublishedSelection = false;
  web.state.trogdorAtlasOpen = true;
  web.state.hoveredTrogdorSessionId = null;
  web.state.trogdorReaderStartedAt = 0;
  web.state.trogdorReaderStartIndex = 0;
  web.state.trogdorReaderClawgKey = "";
  web.state.trogdorSurfaceSignature = "";
  web.state.trogdorReadProgress = {};
  web.state.trogdorDismissedClawgs = {};
  web.state.trogdorBurntSessions = new Map();
  web.state.trogdorAwaitingSessionIds = new Set();
  web.state.hud = null;
  web.state.terminal = null;
  web.state.terminalAcceptsBytes = true;
  web.state.pendingTerminalByteChunks = [];
  web.state.pendingTerminalByteLength = 0;
  web.state.frankenModule = null;
  web.state.frankenInit = null;
  web.state.frankenFontInit = null;
  web.state.frankenLoadError = "";
  web.state.frankenAssetSummary = "";
  web.state.ws = null;
  web.state.lastTerminalSeqBySession = new Map();
  web.state.reconnectTimer = null;
  web.state.reconnectAttempt = 0;
  web.state.pendingInputMessages = new Map();
  web.state.terminalWorkbenchOpen = true;
  web.state.agentContextSessionId = null;
  web.state.agentContextLoading = false;
  web.state.agentContextPayload = null;
  web.state.agentContextError = "";
  web.state.agentContextRequestSeq = 0;
  web.state.agentContextLastLoadedAt = 0;
  web.state.workbenchWidgets = {
    sessionId: null,
    loading: false,
    timeline: null,
    skills: null,
    paneTail: null,
    transcript: null,
    transcriptTurnId: "",
    transcriptNextCursor: 0,
    artifact: null,
    gitDiff: null,
    error: "",
    requestSeq: 0,
    lastLoadedAt: 0,
    lastHtml: "",
  };
  web.state.workbenchLogMode = "lens";
  web.state.workbenchLogFilter = "all";
  web.state.workbenchLogSearch = "";
  web.state.workbenchSelectedTurnId = "";
  web.state.readOnly = false;
  web.state.backendHealth = null;
  web.state.fleetFilter = { kind: "", key: "" };
  web.state.fleetPresetId = "";
  web.state.fleetPresets = [];
  web.state.sessionGroupMode = "flat";
  web.state.terminalFallbackActive = false;
  web.state.terminalFallbackAutoFollow = true;
  web.state.terminalMirrorText = "";
  web.state.terminalPaintVerified = false;
  web.state.terminalFrameBytesSeen = 0;
  web.state.renderQueued = false;
  web.state.renderRetryQueued = false;
  web.state.surfaceInitInProgress = 0;
  web.state.surfaceOperationDepth = 0;
  web.state.hudRenderQueued = false;
  web.state.resizeQueued = false;
  web.state.resizePushResize = false;
  web.state.resizeForce = false;
  web.state.currentCols = 80;
  web.state.currentRows = 24;
  web.state.rendererDiagnosticSequence = 0;
  web.state.lastRendererDiagnostic = null;
  web.state.lastRendererDiagnosticError = "";
  web.state.hoveredLinkUrl = "";
  web.state.sendHistory = [];
  web.state.paletteItems = [];
  web.state.paletteIndex = 0;
  web.state.activeSheet = null;
  web.state.dirBrowser = {
    loading: false,
    path: "",
    managedOnly: false,
    entries: [],
    groups: [],
    group: "",
    search: "",
    overlayLabel: "",
    launchTargets: [],
    launchTarget: "local",
    batchSelected: new Set(),
    status: "",
    error: "",
  };
  web.state.mermaidArtifact = {
    loading: false,
    sessionId: null,
    artifact: null,
    svgUrl: "",
    source: "",
    planFiles: [],
    activePlanFile: "",
    planContent: "",
    status: "",
    error: "",
  };
  if (originalFetch) {
    globalThis.fetch = originalFetch;
  } else {
    delete globalThis.fetch;
  }
}

function mockWorkbenchDetails(title, open) {
  return {
    open,
    querySelector(selector) {
      return selector === ".workbench-widget-title" ? { textContent: title } : null;
    },
  };
}

function jsonResponse(status, body) {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 207 ? "Multi-Status" : "OK",
    async json() {
      return body;
    },
  };
}


test("command palette filters existing actions without touching terminal input", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.sessions = [
    rawSession(),
    rawSession({ session_id: "sess_1", tmux_name: "beta", state: "idle" }),
  ];

  web.openCommandPalette();
  web.el.paletteSearch.value = "beta";
  web.renderCommandPalette();

  assert.equal(web.state.activeSheet, "palette");
  assert.ok(web.el.paletteResults.innerHTML.includes("Switch to beta"));
  assert.equal(web.el.terminalStage.classList.contains("terminal-view-active"), true);
});

test("command palette run path preserves disabled no-op, actions, and actionId dispatch", async () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.activeSheet = "palette";
  let actionCalls = 0;

  assert.equal(await web.runCommandPaletteItem({ disabled: true, action: () => { actionCalls += 1; } }), false);
  assert.equal(actionCalls, 0);
  assert.equal(web.state.activeSheet, "palette");

  assert.equal(await web.runCommandPaletteItem({ action: async () => { actionCalls += 1; } }), true);
  assert.equal(actionCalls, 1);
  assert.equal(web.state.activeSheet, null);

  web.state.activeSheet = "palette";
  assert.equal(await web.runCommandPaletteItem({ actionId: "open_auth" }), true);
  assert.equal(web.state.activeSheet, "auth");
});

test("surface fleet lens actions persist active filter and grouping mode", async () => {
  resetWebState();

  await web.handleSurfaceAction({
    type: "action",
    actionId: "fleet_filter",
    kind: "target",
    key: "skillbox",
  });
  assert.deepEqual(web.state.fleetFilter, { kind: "target", key: "skillbox" });
  assert.equal(
    storage.get("swimmers.web.fleet.filter"),
    JSON.stringify({ kind: "target", key: "skillbox" }),
  );

  await web.handleSurfaceAction({
    type: "action",
    actionId: "fleet_filter",
    kind: "target",
    key: "skillbox",
  });
  assert.deepEqual(web.state.fleetFilter, { kind: "", key: "" });
  assert.equal(storage.has("swimmers.web.fleet.filter"), false);

  await web.handleSurfaceAction({
    type: "action",
    actionId: "toggle_session_grouping",
  });
  assert.equal(web.state.sessionGroupMode, "project");
  assert.equal(storage.get("swimmers.web.sessionGroupMode"), "project");
});

test("surface fleet preset actions persist URL-safe preset state", async () => {
  resetWebState();

  await web.handleSurfaceAction({
    type: "action",
    actionId: "fleet_preset",
    presetId: "remote-api",
  });

  assert.equal(web.state.fleetPresetId, "remote-api");
  assert.deepEqual(web.state.fleetFilter, { kind: "", key: "" });
  assert.equal(storage.get("swimmers.web.fleet.preset"), "remote-api");
  assert.equal(window.location.search, "?preset=remote-api");

  await web.handleSurfaceAction({
    type: "action",
    actionId: "fleet_preset",
    presetId: "remote-api",
  });

  assert.equal(web.state.fleetPresetId, "");
  assert.equal(storage.has("swimmers.web.fleet.preset"), false);
  assert.equal(window.location.search, "");
});

test("surface environment hint actions copy command text", async () => {
  resetWebState();
  const writes = [];
  globalThis.navigator.clipboard = {
    async writeText(text) {
      writes.push(text);
    },
  };

  await web.handleSurfaceAction({
    type: "environment_hint",
    actionId: "copy_environment_hint",
    kind: "bootstrap",
    key: "skillbox-api",
    copyText: "ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'",
  });

  assert.deepEqual(writes, ["ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'"]);
  assert.equal(web.state.utilityLabel, "Copied bootstrap.");
  delete globalThis.navigator.clipboard;
});

test("fleet filter reconciler clears persisted filters for unavailable targets", () => {
  resetWebState();
  web.state.fleetFilter = { kind: "target", key: "missing-devbox" };
  storage.set("swimmers.web.fleet.filter", JSON.stringify(web.state.fleetFilter));
  web.state.sessions = [rawSession({
    session_id: "local",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/tmp/swimmers",
    },
  })];

  web.reconcileFleetFilterForSessions();

  assert.deepEqual(web.state.fleetFilter, { kind: "", key: "" });
  assert.equal(storage.has("swimmers.web.fleet.filter"), false);
  assert.equal(web.state.sessions.length, 1);
});

test("command palette event handlers preserve navigation and result dispatch", () => {
  resetWebState();
  web.state.activeSheet = "palette";
  let actionCalls = 0;
  web.state.paletteItems = [
    { label: "Alpha", action: () => { actionCalls += 1; } },
    { label: "Beta", action: () => { actionCalls += 1; } },
    { label: "Gamma", action: () => { actionCalls += 1; } },
  ];
  web.state.paletteIndex = 0;

  let prevented = 0;
  assert.equal(web.handleCommandPaletteEvent({
    type: "keydown",
    key: "ArrowDown",
    preventDefault() {
      prevented += 1;
    },
  }), true);
  assert.equal(prevented, 1);
  assert.equal(web.state.paletteIndex, 1);

  web.state.paletteItems = [
    { label: "Alpha", action: () => { actionCalls += 1; } },
    { label: "Beta", action: () => { actionCalls += 1; } },
    { label: "Gamma", action: () => { actionCalls += 1; } },
  ];
  web.state.paletteIndex = 1;
  prevented = 0;
  assert.equal(web.handleCommandPaletteEvent({
    type: "keydown",
    key: "Enter",
    preventDefault() {
      prevented += 1;
    },
  }), true);
  assert.equal(prevented, 1);
  assert.equal(actionCalls, 1);

  const item = new MockElement("palette-item");
  item.closest = (selector) => selector === "[data-palette-index]" ? { dataset: { paletteIndex: "2" } } : null;
  const clickEvent = { type: "click", target: item, preventDefault() { prevented += 1; } };
  assert.equal(web.handleCommandPaletteEvent(clickEvent), true);
  assert.equal(web.state.paletteIndex, 2);
  assert.equal(prevented, 1);
  assert.equal(actionCalls, 2);
});

test("global shortcut handler preserves side effects and handled no-ops", () => {
  resetWebState();
  web.state.trogdorAtlasOpen = false;
  web.state.terminalZoom = 1;

  assert.equal(web.handleGlobalShortcut({ ctrlKey: true, code: "KeyK" }), true);
  assert.equal(web.state.activeSheet, "palette");

  assert.equal(web.handleGlobalShortcut({ ctrlKey: true, code: "Equal" }), true);
  assert.equal(web.state.terminalZoom, 1.1);

  web.state.activeSheet = "auth";
  assert.equal(web.handleGlobalShortcut({ key: "Escape" }), true);
  assert.equal(web.state.activeSheet, null);

  web.state.trogdorAtlasOpen = true;
  assert.equal(web.handleGlobalShortcut({ key: "Escape" }), true);
  assert.equal(web.state.trogdorAtlasOpen, false);

  web.el.createForm.elements = [];
  web.state.selectMode = true;
  assert.equal(web.handleGlobalShortcut({ key: "Escape" }), true);
  assert.equal(web.state.selectMode, false);

  web.state.selectedSessionId = "sess_0";
  web.state.readOnly = true;
  assert.equal(web.handleGlobalShortcut({ ctrlKey: true, shiftKey: true, code: "KeyS" }), true);
  assert.equal(web.state.activeSheet, null);

  web.state.readOnly = false;
  assert.equal(web.handleGlobalShortcut({ ctrlKey: true, shiftKey: true, code: "KeyS" }), true);
  assert.equal(web.state.activeSheet, "send");

  web.state.activeSheet = null;
  assert.equal(web.handleGlobalShortcut({ ctrlKey: true, shiftKey: true, code: "KeyZ" }), false);
});

test("document command-palette shortcut delegates the table only when focus is off-surface", () => {
  resetWebState();
  web.state.activeSheet = null;

  // Off-surface focus (target not inside a dispatching surface): the full table
  // is delegated, so Ctrl+Shift+A opens the auth sheet.
  web.handleDocumentCommandPaletteShortcut({
    ctrlKey: true,
    shiftKey: true,
    code: "KeyA",
    target: { closest: () => null },
    preventDefault() {},
  });
  assert.equal(web.state.activeSheet, "auth");

  // On-surface focus (target inside a dispatching surface): that surface's own
  // listener already handled it, so the document handler must not double-fire.
  web.state.activeSheet = null;
  web.handleDocumentCommandPaletteShortcut({
    ctrlKey: true,
    shiftKey: true,
    code: "KeyA",
    target: { closest: () => ({}) },
    preventDefault() {},
  });
  assert.equal(web.state.activeSheet, null);

  // Ctrl+K opens the palette regardless of the focus target.
  web.handleDocumentCommandPaletteShortcut({
    ctrlKey: true,
    code: "KeyK",
    target: { closest: () => ({}) },
    preventDefault() {},
  });
  assert.equal(web.state.activeSheet, "palette");
});

test("mobile keyboard key handler preserves shortcut precedence, close, and forwarding", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.terminalFallbackActive = true;
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  let prevented = 0;
  assert.equal(web.handleMobileKeyboardProxyKeydown({
    ctrlKey: true,
    code: "KeyK",
    preventDefault() {
      prevented += 1;
    },
  }), true);
  assert.equal(prevented, 1);
  assert.equal(web.state.activeSheet, "palette");
  assert.equal(sent.length, 0);

  web.state.activeSheet = null;
  web.state.readOnly = true;
  prevented = 0;
  assert.equal(web.handleMobileKeyboardProxyKeydown({
    key: "ArrowUp",
    code: "ArrowUp",
    preventDefault() {
      prevented += 1;
    },
  }), false);
  assert.equal(prevented, 0);
  assert.equal(sent.length, 0);

  web.state.readOnly = false;
  web.state.mobileKeyboardActive = true;
  document.activeElement = web.el.mobileKeyboardProxy;
  prevented = 0;
  assert.equal(web.handleMobileKeyboardProxyKeydown({
    key: "Escape",
    code: "Escape",
    preventDefault() {
      prevented += 1;
    },
  }), true);
  assert.equal(prevented, 1);
  assert.equal(web.state.mobileKeyboardActive, false);
  assert.equal(document.activeElement, web.el.terminalFallback);

  prevented = 0;
  assert.equal(web.handleMobileKeyboardProxyKeydown({
    key: "Backspace",
    code: "Backspace",
    preventDefault() {
      prevented += 1;
    },
  }), true);
  assert.equal(prevented, 1);
  assert.equal(sent.at(-1).type, "input_text");
  assert.equal(sent.at(-1).data, "\x7f");
  assert.equal(web.state.trogdorBurntSessions.has("sess_0"), true);
});

test("mobile keyboard input handler preserves clears, control events, and text sends", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  web.state.readOnly = true;
  web.el.mobileKeyboardProxy.value = "stale";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "insertText", data: "ignored" }), false);
  assert.equal(web.el.mobileKeyboardProxy.value, "");
  assert.equal(sent.length, 0);

  web.state.readOnly = false;
  // Enter/Backspace are owned exclusively by the keydown channel now, so the
  // matching input-event control types are no-ops here. This prevents the PTY
  // from receiving the keystroke twice on browsers that fire both a named
  // keydown and the corresponding input event.
  const sentBeforeControl = sent.length;
  web.el.mobileKeyboardProxy.value = "delete sentinel";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "deleteContentBackward", data: null }), false);
  assert.equal(web.el.mobileKeyboardProxy.value, "");
  assert.equal(sent.length, sentBeforeControl);

  web.el.mobileKeyboardProxy.value = "newline sentinel";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "insertLineBreak", data: null }), false);
  assert.equal(sent.length, sentBeforeControl);

  web.el.mobileKeyboardProxy.value = "fallback";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "insertText", data: "typed" }), true);
  assert.equal(web.el.mobileKeyboardProxy.value, "");
  assert.equal(sent.at(-1).type, "input_text");
  assert.equal(sent.at(-1).data, "typed");
  assert.equal(web.state.trogdorBurntSessions.has("sess_0"), true);

  web.el.mobileKeyboardProxy.value = "fallback";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "insertText", data: null }), true);
  assert.equal(sent.at(-1).type, "input_text");
  assert.equal(sent.at(-1).data, "fallback");
});

test("send history stores multiline prompts for recall chips", () => {
  resetWebState();

  web.rememberSendHistory("first line\nsecond line");
  web.rememberSendHistory("status");

  assert.deepEqual(web.state.sendHistory, ["status", "first line\nsecond line"]);
  assert.ok(web.el.sendHistory.innerHTML.includes("status"));
  assert.ok(web.el.sendHistory.innerHTML.includes("first line second line"));
});

test("auth token button action preserves save, clear, and refresh side effects", async () => {
  resetWebState();
  const requests = [];
  globalThis.fetch = async (path, init = {}) => {
    requests.push({ path: String(path), authorization: init.headers?.Authorization ?? null });
    return path === "/v1/sessions"
      ? jsonResponse(200, { sessions: [rawSession()] })
      : jsonResponse(404, { message: "not found" });
  };

  web.state.activeSheet = "auth";
  web.el.tokenInput.value = " operator-token ";
  assert.equal(await web.handleAuthTokenButtonAction("save"), true);
  assert.equal(web.state.token, "operator-token");
  assert.equal(web.el.tokenInput.value, "operator-token");
  assert.equal(storage.get("swimmers.web.token"), "operator-token");
  assert.equal(web.state.activeSheet, null);
  assert.equal(requests.find((request) => request.path === "/v1/sessions")?.authorization, "Bearer operator-token");

  requests.length = 0;
  web.state.activeSheet = "auth";
  // Currently a writer (operator token granted write). Clearing the token revokes
  // that privilege, so it must NOT optimistically keep/grant write — it drops to
  // observer (read-only) pending the server-derived flag from the next refresh.
  web.state.readOnly = false;
  web.el.sendInput.disabled = false;
  web.el.tokenInput.value = "ignored";
  assert.equal(await web.handleAuthTokenButtonAction("clear"), true);
  assert.equal(web.state.token, "");
  assert.equal(storage.has("swimmers.web.token"), false);
  assert.equal(web.state.readOnly, true);
  assert.equal(web.el.sendInput.disabled, true);
  assert.equal(web.state.activeSheet, null);
  assert.equal(requests.find((request) => request.path === "/v1/sessions")?.authorization, null);
});

test("visible directory batch action preserves selection, cwd fallback, and status", () => {
  resetWebState();
  Object.assign(web.state.dirBrowser, {
    path: "/srv/repos",
    search: "dirty",
    overlayLabel: "managed",
    entries: [
      { name: "swimmers", repo_dirty: true },
      { name: "other", repo_dirty: true },
      { name: "clients", group: "clients", repo_dirty: true },
      { name: "clean" },
    ],
    batchSelected: new Set(["/old"]),
  });
  web.el.dirsPath.value = "/typed";
  web.el.createCwd.value = "";

  web.handleCreateBatchVisibleAction();

  assert.deepEqual(Array.from(web.state.dirBrowser.batchSelected), ["/srv/repos/swimmers", "/srv/repos/other"]);
  assert.equal(web.el.createCwd.value, "/srv/repos/swimmers");
  assert.equal(web.state.dirBrowser.status, "Batching 2 visible directories.");
  assert.equal(web.state.dirBrowser.error, "");
  assert.equal(web.el.createBatchCount.textContent, "2 selected");

  web.state.dirBrowser.search = "missing";
  web.state.dirBrowser.batchSelected = new Set(["/old"]);
  web.el.createCwd.value = "";

  web.handleCreateBatchVisibleAction();

  assert.equal(web.state.dirBrowser.batchSelected.size, 0);
  assert.equal(web.el.createCwd.value, "/srv/repos");
  assert.equal(web.state.dirBrowser.status, "No visible directories to batch.");
  assert.equal(web.state.dirBrowser.error, "No visible directories to batch.");
});

test("directory checkbox change action preserves add, remove, reset, and ignore paths", () => {
  resetWebState();
  const checkbox = (path, checked = true) => {
    const node = new MockElement("dir-checkbox");
    node.dataset.path = path;
    node.checked = checked;
    node.closest = (selector) => selector === ".dir-row-check" ? node : null;
    return node;
  };

  const added = checkbox("/srv/repos/added", true);
  assert.equal(web.handleDirCheckboxChange({ type: "change", target: added }), true);
  assert.deepEqual(Array.from(web.state.dirBrowser.batchSelected), ["/srv/repos/added"]);
  assert.equal(web.el.createCwd.value, "/srv/repos/added");

  const removed = checkbox("/srv/repos/added", false);
  assert.equal(web.handleDirCheckboxChange({ type: "change", target: removed }), true);
  assert.equal(web.state.dirBrowser.batchSelected.size, 0);
  assert.equal(web.el.createCwd.value, "/srv/repos/added");

  const invalid = checkbox(" ", true);
  assert.equal(web.handleDirCheckboxChange({ type: "change", target: invalid }), true);
  assert.equal(invalid.checked, false);
  assert.equal(web.handleDirCheckboxChange({ type: "change", target: new MockElement("ignored") }), false);
});

test("directory group chip click action preserves filters, storage, and load path", async () => {
  resetWebState();
  const chip = (dataset) => {
    const node = new MockElement("dir-group-chip");
    node.dataset = { ...dataset };
    node.closest = (selector) => selector === ".dir-group-chip" ? node : null;
    return node;
  };
  const requests = [];
  globalThis.fetch = async (path) => {
    requests.push(String(path));
    return jsonResponse(200, { path: "/srv/repos", entries: [], groups: ["clients"] });
  };

  web.state.dirBrowser.path = "/srv/repos";
  web.state.dirBrowser.group = "old";
  web.state.dirBrowser.managedOnly = false;
  web.state.dirBrowser.batchSelected = new Set(["/srv/repos/old"]);
  web.el.dirsPath.value = "/typed";
  web.el.dirsManagedOnly.checked = false;

  assert.equal(await web.handleDirGroupChipClick({ type: "click", target: chip({ filter: "managed", group: "clients" }) }), true);
  assert.equal(web.state.dirBrowser.group, "");
  assert.equal(web.state.dirBrowser.managedOnly, true);
  assert.equal(web.el.dirsManagedOnly.checked, true);
  assert.equal(web.state.dirBrowser.batchSelected.size, 0);
  assert.equal(storage.get("swimmers.web.dirs.managed"), "true");
  assert.equal(requests.at(-1), "/v1/dirs?path=%2Fsrv%2Frepos&managed_only=true");

  web.state.dirBrowser.path = "";
  web.state.dirBrowser.batchSelected = new Set(["/old"]);
  web.el.dirsPath.value = "/typed";
  web.el.dirsManagedOnly.checked = true;

  assert.equal(await web.handleDirGroupChipClick({ type: "click", target: chip({ filter: "group", group: " clients " }) }), true);
  assert.equal(web.state.dirBrowser.group, "clients");
  assert.equal(web.state.dirBrowser.managedOnly, true);
  assert.equal(web.state.dirBrowser.batchSelected.size, 0);
  assert.equal(requests.at(-1), "/v1/dirs?path=%2Ftyped&managed_only=true&group=clients");
  assert.equal(await web.handleDirGroupChipClick({ type: "click", target: new MockElement("ignored") }), false);
});

test("send form submit handler preserves line send side effects and cleanup", async () => {
  resetWebState();
  web.state.trogdorAtlasOpen = false;
  web.state.selectedSessionId = "sess_0";
  web.state.activeSheet = "send";
  web.el.sendInput.value = "continue\n";
  web.el.sendMode.value = "line";

  const requests = [];
  globalThis.fetch = async (path, init = {}) => {
    requests.push({ path, body: init.body ? JSON.parse(init.body) : null });
    if (path === "/v1/sessions/sess_0/input") {
      return jsonResponse(200, { ok: true, delivered: true });
    }
    if (path === "/v1/sessions") {
      return jsonResponse(200, { sessions: [rawSession({ session_id: "sess_0" })] });
    }
    return jsonResponse(404, { message: "not found" });
  };

  let prevented = 0;
  assert.equal(await web.handleSendFormSubmit({
    preventDefault() {
      prevented += 1;
    },
  }), true);

  assert.equal(prevented, 1);
  assert.deepEqual(requests[0], {
    path: "/v1/sessions/sess_0/input",
    body: { text: "continue\n", submit: true },
  });
  assert.deepEqual(web.state.sendHistory, ["continue"]);
  assert.equal(web.el.sendInput.value, "");
  assert.equal(web.state.sendTarget, null);
  assert.equal(web.state.activeSheet, null);
  assert.equal(web.state.utilityLabel, "Sent line to sess_0.");
  assert.equal(web.state.utilityMuted, false);
  clearTimeout(web.state.utilityMessageTimer);
  web.state.utilityMessageTimer = null;
});

test("batch send burns only successful group-input results", async () => {
  resetWebState();
  web.state.sessions = [
    rawSession({ session_id: "ready", tmux_name: "ready" }),
    rawSession({ session_id: "stale", tmux_name: "stale" }),
  ];
  const requests = [];
  globalThis.fetch = async (path, init) => {
    requests.push({ path, body: JSON.parse(init.body) });
    return jsonResponse(207, {
      delivered: 1,
      skipped: 1,
      results: [
        { session_id: "ready", ok: true },
        {
          session_id: "stale",
          ok: false,
          error: { code: "SESSION_NOT_READY" },
        },
      ],
    });
  };

  const result = await web.sendGroupLine(["ready", "stale"], "continue");

  assert.deepEqual(requests, [
    {
      path: "/v1/sessions/group-input",
      body: { session_ids: ["ready", "stale"], text: "continue" },
    },
  ]);
  assert.deepEqual(result.deliveredSessionIds, ["ready"]);
  assert.equal(result.delivered, 1);
  assert.equal(result.skipped, 1);
  assert.equal(result.total, 2);
  assert.equal(web.state.trogdorBurntSessions.has("ready"), true);
  assert.equal(web.state.trogdorBurntSessions.has("stale"), false);
});

test("paste-only HTTP sends stage text without burning Trogdor response state", async () => {
  resetWebState();
  web.state.sessions = [rawSession({ session_id: "sess_0" })];
  const requests = [];
  globalThis.fetch = async (path, init) => {
    requests.push({ path, body: JSON.parse(init.body) });
    return jsonResponse(200, { ok: true, session_id: "sess_0" });
  };

  await web.sendRawTextToSession("sess_0", "draft only");

  assert.deepEqual(requests, [
    {
      path: "/v1/sessions/sess_0/input",
      body: { text: "draft only" },
    },
  ]);
  assert.equal(web.state.trogdorBurntSessions.has("sess_0"), false);
});
