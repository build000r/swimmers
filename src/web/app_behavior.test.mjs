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

  assert.deepEqual(sent, [{ type: "input_text", data: "hello" }]);
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
  assert.deepEqual(sent, [{ type: "input_text", data: "h" }]);
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
  assert.deepEqual(sent, [{ type: "input_text", data: "echo pasted" }]);
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

  assert.deepEqual(sent, [{ type: "submit_line", data: "echo dock" }]);
  assert.equal(web.el.terminalInlineInput.value, "");
  assert.equal(web.el.terminalInputEcho.textContent, "› echo dock");
  assert.ok(web.el.terminalFallback.textContent.includes("› echo dock"));
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
