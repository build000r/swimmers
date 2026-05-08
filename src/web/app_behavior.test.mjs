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

const { __swimmersWebTest: web } = await import("./app.js?behavior-test");

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
  if (web.state.terminalPaintProbeTimer) {
    clearTimeout(web.state.terminalPaintProbeTimer);
    web.state.terminalPaintProbeTimer = null;
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
  web.state.selectedSessionId = null;
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
  web.state.ws = null;
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
    artifact: null,
    gitDiff: null,
    error: "",
    requestSeq: 0,
    lastLoadedAt: 0,
  };
  web.state.readOnly = false;
  web.state.terminalFallbackActive = false;
  web.state.terminalFallbackAutoFollow = true;
  web.state.terminalMirrorText = "";
  web.state.terminalPaintVerified = false;
  web.state.terminalFrameBytesSeen = 0;
  web.state.hoveredLinkUrl = "";
  web.state.sendHistory = [];
  web.state.paletteItems = [];
  web.state.paletteIndex = 0;
  web.state.activeSheet = null;
  if (originalFetch) {
    globalThis.fetch = originalFetch;
  } else {
    delete globalThis.fetch;
  }
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

test("selecting a Trogdor swordsman forces the terminal view, not just URL state", () => {
  resetWebState();
  web.state.trogdorAtlasOpen = true;
  web.state.hoveredTrogdorSessionId = "sess_0";
  web.el.trogdorSurface.classList.remove("hidden");
  web.el.trogdorSurface.style.display = "";
  document.body.classList.add("trogdor-mode");

  web.persistSelectedSession("sess_0");

  assert.equal(web.state.selectedSessionId, "sess_0");
  assert.equal(web.state.trogdorAtlasOpen, false);
  assert.equal(web.state.hoveredTrogdorSessionId, null);
  assert.equal(web.el.trogdorSurface.classList.contains("hidden"), true);
  assert.equal(web.el.trogdorSurface.style.display, "none");
  assert.equal(web.el.trogdorSurface.getAttribute("aria-hidden"), "true");
  assert.equal(document.body.classList.contains("trogdor-mode"), false);
  assert.equal(document.body.classList.contains("terminal-focus-mode"), true);
  assert.equal(web.el.terminalStage.classList.contains("terminal-view-active"), true);
  assert.equal(window.location.search, "?session=sess_0");
});

test("Trogdor atlas renders dragon sprite assets and flames burnt swordsmen", () => {
  resetWebState();
  web.state.trogdorAtlasOpen = true;
  web.markTrogdorSessionsResponded(["sess_0"]);

  web.renderHudSurface();

  const html = web.el.trogdorSurface.innerHTML;
  assert.match(html, /class="[^"]*trogdor-dragon[^"]*is-firing/);
  assert.match(html, /\/assets\/dragon\/mouth-closed\/left\.png/);
  assert.match(html, /\/assets\/dragon\/fire-left-full\/left\.png/);
  assert.equal(web.state.trogdorBurntSessions.has("sess_0"), true);
  assert.match(html, /agent-burn-flame/);
  assert.match(html, /agent-burn-smoke/);
});

test("live terminal presentation hides the HUD canvas with class and inline state", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.hud = {};
  web.state.terminal = {};
  web.el.hudCanvas.classList.remove("hidden");
  web.el.hudCanvas.style.display = "";
  web.el.hudCanvas.style.visibility = "";
  web.el.terminalCanvas.classList.add("hidden");

  web.syncTerminalPresentation();

  assert.equal(document.body.classList.contains("terminal-focus-mode"), true);
  assert.equal(web.el.hudCanvas.classList.contains("hidden"), true);
  assert.equal(web.el.hudCanvas.style.display, "none");
  assert.equal(web.el.hudCanvas.style.visibility, "hidden");
  assert.equal(web.el.terminalCanvas.classList.contains("hidden"), false);
  assert.equal(web.el.terminalCanvas.style.display, "");
  assert.equal(web.el.terminalCanvas.style.visibility, "");
});

test("replayed terminal bytes flush protocol replies back to tmux", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalPaintVerified = true;
  const sent = [];
  let fed = [];
  web.state.terminal = {
    feed(bytes) {
      fed = Array.from(bytes);
    },
    drainEncodedInputBytes() {
      return new Uint8Array([27, 91, 99]);
    },
    render() {},
  };
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(payload);
    },
  };

  assert.equal(web.feedTerminalBytes(new Uint8Array([65, 66])), true);

  assert.deepEqual(fed, [65, 66]);
  assert.equal(sent.length, 1);
  assert.deepEqual(Array.from(sent[0]), [27, 91, 99]);
});

test("terminal text fallback keeps send input wired to the live websocket", () => {
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

  assert.equal(web.sendTerminalText("hello"), true);

  assert.equal(sent[0].type, "input_text");
  assert.equal(sent[0].data, "hello");
  assert.match(sent[0].clientMessageId, /^web-/);
});

test("terminal text fallback becomes the keyboard focus target", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  document.activeElement = web.el.terminalStage;

  web.setTerminalTextFallbackActive(true);

  assert.equal(document.activeElement, web.el.terminalFallback);
  assert.equal(web.el.terminalFallback.getAttribute("aria-hidden"), "false");
});

test("terminal text fallback keydown sends live websocket input once", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  const sent = [];
  let prevented = false;
  let stopped = false;
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  assert.equal(
    web.handleTerminalFallbackKeyEvent({
      key: "h",
      code: "KeyH",
      shiftKey: false,
      altKey: false,
      ctrlKey: false,
      metaKey: false,
      repeat: false,
      preventDefault() {
        prevented = true;
      },
      stopPropagation() {
        stopped = true;
      },
    }),
    true,
  );

  assert.equal(prevented, true);
  assert.equal(stopped, true);
  assert.equal(sent[0].type, "input_text");
  assert.equal(sent[0].data, "h");
  assert.match(sent[0].clientMessageId, /^web-/);
});

test("terminal text fallback paste sends live websocket input", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  const sent = [];
  let prevented = false;
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  assert.equal(
    web.handleTerminalFallbackPasteEvent({
      clipboardData: {
        getData(kind) {
          return kind === "text" ? "echo pasted" : "";
        },
      },
      preventDefault() {
        prevented = true;
      },
      stopPropagation() {},
    }),
    true,
  );

  assert.equal(prevented, true);
  assert.equal(sent[0].type, "input_text");
  assert.equal(sent[0].data, "echo pasted");
  assert.match(sent[0].clientMessageId, /^web-/);
});

test("terminal status strip shows live input when renderer fallback is active", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.connectionLabel = "live";
  web.state.terminalFallbackActive = true;
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send() {},
  };

  web.syncTerminalStatusStrip();

  assert.ok(web.el.terminalStatusStrip.textContent.includes("fallback live"));
  assert.equal(web.el.terminalStatusStrip.textContent.includes("snapshot fallback"), false);
});

test("terminal text fallback refreshes from live terminal frames", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  let liveText = "$ ";
  web.state.terminal = {
    feed(bytes) {
      liveText += new TextDecoder().decode(bytes);
    },
    drainEncodedInputBytes() {
      return new Uint8Array();
    },
    screenReaderMirrorText() {
      return liveText;
    },
    render() {},
  };
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send() {},
  };

  assert.equal(web.feedTerminalBytes(new TextEncoder().encode("echo hi")), true);

  assert.equal(web.el.terminalFallback.textContent, "$ echo hi");
});

test("terminal text fallback does not replace a useful snapshot with blank live frames", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  web.el.terminalFallback.textContent = "snapshot prompt\n$ cargo test";
  web.state.terminal = {
    feed() {},
    drainEncodedInputBytes() {
      return new Uint8Array();
    },
    screenReaderMirrorText() {
      return "\n\n\n\n";
    },
    render() {},
  };
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send() {},
  };

  assert.equal(web.feedTerminalBytes(new Uint8Array([10])), true);

  assert.equal(web.el.terminalFallback.textContent, "snapshot prompt\n$ cargo test");
});

test("terminal input dock appears in terminal mode and disables empty sends", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;

  web.syncTerminalInputDock();

  assert.equal(web.el.terminalInputDock.classList.contains("hidden"), false);
  assert.equal(document.body.classList.contains("terminal-input-dock-visible"), true);
  assert.equal(web.el.terminalInputSend.disabled, true);

  web.el.terminalInlineInput.value = "pwd";
  web.syncTerminalInputDock();

  assert.equal(web.el.terminalInputSend.disabled, false);
});

test("terminal input dock sends a line to tmux and keeps a local echo", async () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.terminalFallbackActive = true;
  web.el.terminalFallback.textContent = "$ ";
  web.el.terminalInlineInput.value = "echo dock";
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  assert.equal(await web.submitTerminalInputDock(), true);

  assert.equal(sent[0].type, "submit_line");
  assert.equal(sent[0].data, "echo dock");
  assert.match(sent[0].clientMessageId, /^web-/);
  assert.equal(web.el.terminalInlineInput.value, "");
  assert.equal(web.el.terminalInputEcho.textContent, "› pending: echo dock");
  assert.ok(web.el.terminalFallback.textContent.includes("› echo dock"));
});

test("terminal key strip sends Ctrl-C and navigation bytes from the dock", () => {
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

  assert.equal(web.sendTerminalControlKey("ctrl-c"), true);
  assert.equal(web.sendTerminalControlKey("arrow-up"), true);

  assert.equal(sent[0].type, "input_text");
  assert.equal(sent[0].data.charCodeAt(0), 3);
  assert.equal(sent[1].type, "input_text");
  assert.equal(sent[1].data, "\x1b[A");
  assert.ok(web.el.terminalInputEcho.textContent.includes("sent: Up"));
});

test("terminal composer turns empty-field Ctrl-C and arrows into terminal controls", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.el.terminalInlineInput.value = "";
  web.el.terminalInlineInput.selectionStart = 0;
  web.el.terminalInlineInput.selectionEnd = 0;

  assert.equal(
    web.terminalKeyActionForDomEvent({
      key: "c",
      ctrlKey: true,
      metaKey: false,
      altKey: false,
    }),
    "ctrl-c",
  );
  assert.equal(web.terminalKeyActionForDomEvent({ key: "ArrowUp", ctrlKey: false }), "arrow-up");

  web.el.terminalInlineInput.value = "edit this text";
  assert.equal(web.terminalKeyActionForDomEvent({ key: "ArrowUp", ctrlKey: false }), "");

  web.el.terminalInlineInput.selectionStart = 0;
  web.el.terminalInlineInput.selectionEnd = 4;
  assert.equal(
    web.terminalKeyActionForDomEvent({
      key: "c",
      ctrlKey: true,
      metaKey: false,
      altKey: false,
    }),
    "",
  );
});

test("input ack updates pending terminal dock delivery status", async () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.el.terminalInlineInput.value = "echo acked";
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  assert.equal(await web.submitTerminalInputDock(), true);
  assert.ok(web.el.terminalInputEcho.textContent.includes("pending: echo acked"));

  web.handleSocketText(JSON.stringify({
    type: "input_ack",
    clientMessageId: sent[0].clientMessageId,
    delivered: true,
    method: "tmux_submit_line",
  }));

  assert.ok(web.el.terminalInputEcho.textContent.includes("sent: echo acked"));
});

test("input ack failure keeps failed delivery visible", () => {
  resetWebState();
  const id = "web-test-1";
  web.state.pendingInputMessages.set(id, { text: "lost message", status: "pending", detail: "" });

  web.handleSocketText(JSON.stringify({
    type: "input_ack",
    clientMessageId: id,
    delivered: false,
    message: "tmux send-keys exited",
  }));

  assert.ok(web.el.terminalInputEcho.textContent.includes("failed: tmux send-keys exited"));
});

test("terminal workbench fetches and renders selected agent context", async () => {
  resetWebState();
  web.state.sessions = [rawSession({ tool: "Codex", cwd: "/tmp/project" })];
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  const requested = [];
  globalThis.fetch = async (path) => {
    requested.push(path);
    return jsonResponse(200, {
      session_id: "sess_0",
      available: true,
      tool: "Codex",
      cwd: "/tmp/project",
      user_task: "build the workbench",
      current_tool: { tool: "exec", detail: "cargo test agent_context" },
      recent_actions: [
        { tool: "exec", detail: "cargo test agent_context" },
        { tool: "read", detail: "src/web/app.js" },
      ],
      token_count: 777,
      context_limit: 258400,
    });
  };

  await web.refreshAgentContextForSelectedSession({ force: true });

  assert.deepEqual(requested, ["/v1/sessions/sess_0/agent-context"]);
  assert.equal(web.el.terminalWorkbenchTask.textContent, "build the workbench");
  assert.ok(web.el.terminalWorkbenchCurrent.textContent.includes("cargo test agent_context"));
  assert.ok(web.el.terminalWorkbenchPressure.textContent.includes("awaiting user"));
  assert.ok(web.el.terminalWorkbenchPressure.textContent.includes("attention"));
  assert.ok(web.el.terminalWorkbenchPressure.textContent.includes("0% context"));
  assert.ok(web.el.terminalWorkbenchActions.innerHTML.includes("src/web/app.js"));
});

test("terminal workbench toggle controls the single-terminal panel", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.terminalWorkbenchOpen = true;

  web.syncTerminalWorkbench();

  assert.equal(web.el.terminalWorkbench.classList.contains("hidden"), false);
  assert.equal(web.el.terminalWorkbenchToggle.getAttribute("aria-pressed"), "true");

  web.state.terminalWorkbenchOpen = false;
  web.syncTerminalWorkbench();

  assert.equal(web.el.terminalWorkbench.classList.contains("hidden"), true);
  assert.equal(web.el.terminalWorkbenchToggle.getAttribute("aria-pressed"), "false");
});

test("terminal Trogdor back control returns to the atlas from the workbench", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.terminalWorkbenchOpen = true;

  web.syncTerminalPresentation();

  assert.equal(web.el.terminalTrogdorBack.classList.contains("hidden"), false);
  assert.equal(web.el.terminalTrogdorBack.disabled, false);
  assert.equal(web.el.terminalTrogdorBack.getAttribute("aria-hidden"), "false");

  web.openTrogdorAtlas();

  assert.equal(web.state.trogdorAtlasOpen, true);
  assert.equal(document.body.classList.contains("trogdor-mode"), true);
  assert.equal(web.el.trogdorSurface.classList.contains("hidden"), false);
  assert.equal(web.el.terminalTrogdorBack.classList.contains("hidden"), true);
  assert.equal(web.el.terminalTrogdorBack.getAttribute("aria-hidden"), "true");
  assert.equal(web.el.terminalWorkbench.classList.contains("hidden"), true);
});

test("terminal workbench pinned widgets render pane output and artifacts from session APIs", async () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.agentContextSessionId = "sess_0";
  web.state.agentContextPayload = {
    session_id: "sess_0",
    available: true,
    current_tool: { tool: "exec", detail: "cargo test" },
    recent_actions: [{ tool: "read", detail: "src/web/app.js" }],
  };
  const requested = [];
  globalThis.fetch = async (path) => {
    requested.push(path);
    if (String(path).endsWith("/timeline")) {
      return jsonResponse(200, {
        session_id: "sess_0",
        available: true,
        cwd: "/tmp/project",
        events: [
          { id: "task", kind: "task", source: "agent-context", title: "Task", summary: "build the cockpit", order: 1 },
          { id: "current-action", kind: "tool_call", source: "agent-context", title: "exec", summary: "cargo test", order: 2 },
          { id: "pane-tail", kind: "pane_tail", source: "pane-tail", title: "Recent output", summary: "2 lines", detail: "cargo test\nfinished green\n", order: 3 },
          { id: "git-diff", kind: "diff", source: "git-diff", title: "Diffs", summary: "dirty", order: 4 },
          { id: "artifact", kind: "artifact", source: "mermaid-artifact", title: "Artifacts", summary: "2 plan files", order: 5 },
        ],
        pinned: {},
      });
    }
    if (String(path).includes("/skills?source=sbp")) {
      return jsonResponse(200, {
        session_id: "sess_0",
        source: "sbp",
        cwd: "/tmp/project",
        available: true,
        skills: [{ name: "ui", state: "ok", source_bucket: "opensource/skills" }],
        issues: [{ skill: "ui", action: "move_global_to_project", message: "ui: move_global_to_project" }],
      });
    }
    if (String(path).endsWith("/pane-tail")) {
      return jsonResponse(200, {
        session_id: "sess_0",
        text: "cargo test\nfinished green\n",
      });
    }
    if (String(path).endsWith("/mermaid-artifact")) {
      return jsonResponse(200, {
        session_id: "sess_0",
        available: true,
        path: "/tmp/project/docs/plan.mmd",
        source: "flowchart TD; A-->B",
        plan_files: ["plan.md", "WORKGRAPH.md"],
      });
    }
    if (String(path).endsWith("/git-diff")) {
      return jsonResponse(200, {
        session_id: "sess_0",
        available: true,
        cwd: "/tmp/project",
        repo_root: "/tmp/project",
        status_short: " M src/web/app.js",
        unstaged_diff: "diff --git a/src/web/app.js b/src/web/app.js\n@@ -1 +1 @@\n-old\n+new\n",
        staged_diff: "",
        truncated: false,
        files: [
          {
            path: "src/web/app.js",
            source: "unstaged",
            change: "modified",
            added_lines: 1,
            removed_lines: 1,
            hunks: [{ header: "@@ -1 +1 @@", added_lines: 1, removed_lines: 1 }],
          },
        ],
      });
    }
    return jsonResponse(404, { code: "missing" });
  };

  await web.refreshWorkbenchWidgetsForSelectedSession({ force: true });

  assert.ok(requested.includes("/v1/sessions/sess_0/timeline"));
  assert.ok(requested.includes("/v1/sessions/sess_0/skills?source=sbp"));
  assert.ok(requested.includes("/v1/sessions/sess_0/pane-tail"));
  assert.ok(requested.includes("/v1/sessions/sess_0/mermaid-artifact"));
  assert.ok(requested.includes("/v1/sessions/sess_0/git-diff"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Activity"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("finished green"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("WORKGRAPH.md"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("unstaged modified +1/-1"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("diff-line-add"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Tool calls"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("src/web/app.js"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Skills"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("ui"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Open viewer"));
});

test("terminal text fallback follows the tail unless the user scrolled up", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  web.state.terminalFallbackAutoFollow = true;
  web.el.terminalFallback.clientHeight = 100;
  web.el.terminalFallback.scrollHeight = 500;

  web.updateTerminalFallbackText("line 1\nline 2");

  assert.equal(web.el.terminalFallback.textContent, "line 1\nline 2");
  assert.equal(web.el.terminalFallback.scrollTop, 500);

  web.state.terminalFallbackAutoFollow = false;
  web.el.terminalFallback.scrollTop = 120;
  web.el.terminalFallback.scrollHeight = 800;

  web.updateTerminalFallbackText("line 3");

  assert.equal(web.el.terminalFallback.textContent, "line 3");
  assert.equal(web.el.terminalFallback.scrollTop, 120);
});

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

test("send history stores multiline prompts for recall chips", () => {
  resetWebState();

  web.rememberSendHistory("first line\nsecond line");
  web.rememberSendHistory("status");

  assert.deepEqual(web.state.sendHistory, ["status", "first line\nsecond line"]);
  assert.ok(web.el.sendHistory.innerHTML.includes("status"));
  assert.ok(web.el.sendHistory.innerHTML.includes("first line second line"));
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

test("accessibility mirror syncs FrankenTerm screen-reader text and announcements", () => {
  resetWebState();
  web.state.terminal = {
    screenReaderMirrorText() {
      return "visible terminal text";
    },
    drainAccessibilityAnnouncements() {
      return ["new terminal output"];
    },
  };

  web.syncTerminalAccessibilityMirror();

  assert.equal(web.state.terminalMirrorText, "visible terminal text");
  assert.equal(web.el.terminalA11yMirror.value, "visible terminal text");
  assert.equal(web.el.terminalAnnouncer.textContent, "new terminal output");
});

test("terminal status strip marks attention sessions in the browser chrome", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.connectionLabel = "live";

  web.syncTerminalStatusStrip();

  assert.ok(web.el.terminalStatusStrip.textContent.includes("swordsman"));
  assert.ok(web.el.terminalStatusStrip.textContent.includes("operator"));
  assert.equal(document.title, "(!) swordsman - swimmers");
  assert.equal(document.body.classList.contains("session-attention"), true);
});

test("hovered terminal links expose open and copy affordances", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.hoveredLinkUrl = "https://example.com/some/really/long/path";

  web.syncLinkTools();

  assert.equal(web.el.terminalLinkTools.classList.contains("hidden"), false);
  assert.ok(web.el.terminalLinkText.textContent.includes("https://example.com"));
});
