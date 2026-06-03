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
  // sess_0 sits at TROGDOR_REPO_POSITIONS[0] = (18, 40); dragon lands at
  // (38, 58), so the (target - dragon) vector points up-and-left, which the
  // 8-way frame picker resolves to "back-left".
  assert.match(html, /\/assets\/dragon\/mouth-closed\/back-left\.png/);
  assert.match(html, /\/assets\/dragon\/mouth-open\/back-left\.png/);
  assert.match(html, /\/assets\/dragon\/fire-left-full\/back-left\.png/);
  // Flame direction stays 2-way; with target.x < dragon.x the dragon faces left.
  assert.match(html, /class="[^"]*trogdor-dragon[^"]*is-left/);
  assert.match(html, /data-dragon-frame="back-left"/);
  assert.equal(web.state.trogdorBurntSessions.has("sess_0"), true);
  assert.match(html, /agent-burn-flame/);
  assert.match(html, /agent-burn-smoke/);
});

test("Trogdor atlas keeps deep sleep swordsmen visible and hoverable", () => {
  resetWebState();
  web.state.sessions = [
    rawSession({
      state: "idle",
      rest_state: "deep_sleep",
      thought: "parked until the operator launches it",
      action_cues: [],
    }),
  ];
  web.state.hoveredTrogdorSessionId = "sess_0";

  web.renderHudSurface();

  const html = web.el.trogdorSurface.innerHTML;
  assert.match(html, /data-trogdor-agent="true"/);
  assert.match(html, /data-session-id="sess_0"/);
  assert.match(html, /parked/);
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

test("framed terminal output records sequence and returns payload", () => {
  resetWebState();
  const ws = { sessionId: "sess_0", framedOutput: true };
  const frame = new Uint8Array([
    0x11,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    5,
    65,
    66,
  ]);

  const payload = web.terminalPayloadFromSocketBytes(frame, ws);

  assert.deepEqual(Array.from(payload), [65, 66]);
  assert.equal(web.state.lastTerminalSeqBySession.get("sess_0"), "5");
});

test("raw terminal output remains unchanged without framed opt-in", () => {
  resetWebState();
  const raw = new Uint8Array([65, 66, 67]);

  const payload = web.terminalPayloadFromSocketBytes(raw, { sessionId: "sess_0" });

  assert.equal(payload, raw);
  assert.equal(web.state.lastTerminalSeqBySession.has("sess_0"), false);
});

test("session websocket URL opts into framed resume protocol without bearer token", () => {
  resetWebState();
  web.state.token = "observer-token";
  web.state.lastTerminalSeqBySession.set("sess_0", "42");

  const url = web.sessionSocketUrl(rawSession());
  const auth = JSON.parse(web.sessionSocketAuthMessage());

  assert.equal(url.protocol, "ws:");
  assert.equal(url.pathname, "/ws/sessions/sess_0");
  assert.equal(url.searchParams.get("token"), null);
  assert.equal(url.toString().includes("observer-token"), false);
  assert.equal(url.searchParams.get("framed"), "1");
  assert.equal(url.searchParams.get("resume_from_seq"), "42");
  assert.deepEqual(auth, { type: "auth", token: "observer-token" });
});

test("session websocket auth message is omitted without a token", () => {
  resetWebState();

  assert.equal(web.sessionSocketAuthMessage(), null);
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

test("terminal paste budget rejects oversized live paste", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.terminalFallbackActive = true;
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(payload);
    },
  };

  assert.equal(web.sendTerminalText("x".repeat(786433)), false);

  assert.equal(sent.length, 0);
  assert.match(web.state.utilityLabel, /Paste blocked/);
});

test("FrankenTerm link policy only allows HTTP on loopback hosts", () => {
  resetWebState();

  window.location = new URL("http://127.0.0.1:3210/");
  assert.deepEqual(web.frankenTermLinkPolicy(), { allowHttp: true, allowHttps: true });

  window.location = new URL("http://localhost:3210/");
  assert.deepEqual(web.frankenTermLinkPolicy(), { allowHttp: true, allowHttps: true });

  window.location = new URL("http://100.64.0.1:3210/");
  assert.deepEqual(web.frankenTermLinkPolicy(), { allowHttp: false, allowHttps: true });
});

test("safe anchor href rejects active-content directory URLs", () => {
  resetWebState();
  window.location = new URL("http://swimmers.test/base/");

  assert.equal(web.safeAnchorHref("https://example.com/repo"), "https://example.com/repo");
  assert.equal(web.safeAnchorHref("/local/repo"), "http://swimmers.test/local/repo");
  assert.equal(web.safeAnchorHref("javascript:alert(1)"), "");
  assert.equal(web.safeAnchorHref("data:text/html,pwned"), "");
  assert.equal(web.safeAnchorHref("http://[::1"), "");
});

test("FrankenTerm surface validation reports missing methods", () => {
  assert.throws(
    () => web.validateFrankenTermSurface({ init() {} }, ["init", "render"], "test renderer"),
    /test renderer missing methods: render/,
  );
});

test("FrankenTerm resize waits while another surface operation is active", async () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  const calls = [];
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };
  web.state.terminal = {
    fitToContainer(width, height, dpr) {
      calls.push(["fit", width, height, dpr]);
      return { cols: 100, rows: 30 };
    },
    resize(cols, rows) {
      calls.push(["resize", cols, rows]);
    },
    render() {
      calls.push(["render"]);
    },
  };
  web.state.surfaceOperationDepth = 1;

  web.measureAndResizeSurface(true, true);

  assert.deepEqual(calls, []);
  assert.equal(web.state.resizeQueued, true);
  assert.equal(web.state.resizePushResize, true);
  assert.equal(web.state.resizeForce, true);

  web.state.surfaceOperationDepth = 0;
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(calls, [
    ["fit", 1280, 720, 1],
    ["resize", 100, 30],
    ["render"],
  ]);
  assert.deepEqual(sent, [{ type: "resize", cols: 100, rows: 30 }]);
});

test("FrankenTerm recursive borrow errors are retried instead of escaping resize", async () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  const calls = [];
  web.state.terminal = {
    fitToContainer() {
      calls.push("fit");
      if (calls.length === 1) {
        throw new Error("recursive use of an object detected which would lead to unsafe aliasing in rust");
      }
      return { cols: 92, rows: 28 };
    },
    resize(cols, rows) {
      calls.push(`resize:${cols}x${rows}`);
    },
    render() {
      calls.push("render");
    },
  };

  assert.doesNotThrow(() => web.measureAndResizeSurface(false, true));

  assert.equal(web.state.resizeQueued, true);
  assert.match(web.state.lastRendererDiagnosticError, /fitToContainer: recursive use/);

  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(calls, ["fit", "fit", "resize:92x28", "render"]);
  assert.equal(web.state.currentCols, 92);
  assert.equal(web.state.currentRows, 28);
});

test("renderer diagnostic stores JSONL line when supported", () => {
  resetWebState();
  web.state.terminal = {
    snapshotResizeStormFrameJsonl(runId, seed, timestamp, frameIndex) {
      return JSON.stringify({
        run_id: runId,
        seed,
        timestamp,
        frame_idx: frameIndex,
        hash: "abc123",
      });
    },
  };

  const line = web.captureTerminalRendererDiagnostic("painted");

  assert.match(line, /"hash":"abc123"/);
  assert.equal(web.state.lastRendererDiagnostic.reason, "painted");
  assert.equal(web.state.lastRendererDiagnostic.parsed.frame_idx, 0);
  assert.equal(web.state.rendererDiagnosticSequence, 1);
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

test("terminal status strip renders backend health degradation and recovery", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.connectionLabel = "live";

  web.applyBackendHealth({
    status: "healthy",
    thought_bridge: { status: "healthy" },
    persistence: {
      available: true,
      ok: false,
      consecutive_failures: 1,
      last_failed_operation: "save_sessions",
      last_error: "disk full",
    },
  });

  assert.equal(
    web.backendHealthWarningText(web.state.backendHealth),
    "persistence degraded: save_sessions: disk full",
  );
  assert.ok(web.el.terminalStatusStrip.textContent.includes("persistence degraded: save_sessions: disk full"));
  assert.equal(document.body.classList.contains("backend-health-degraded"), true);

  web.applyBackendHealth({
    status: "healthy",
    thought_bridge: { status: "healthy" },
    persistence: { available: true, ok: true, consecutive_failures: 0 },
  });

  assert.equal(web.backendHealthWarningText(web.state.backendHealth), "");
  assert.equal(web.el.terminalStatusStrip.textContent.includes("persistence degraded"), false);
  assert.equal(document.body.classList.contains("backend-health-degraded"), false);
});

test("terminal status strip renders thought bridge degradation", () => {
  resetWebState();
  web.applyBackendHealth({
    status: "degraded",
    thought_bridge: {
      status: "degraded",
      last_backend_error: "model timeout",
    },
    persistence: { available: true, ok: true },
  });

  assert.equal(
    web.backendHealthWarningText(web.state.backendHealth),
    "thought bridge degraded: model timeout",
  );
  assert.ok(web.el.terminalStatusStrip.textContent.includes("thought bridge degraded: model timeout"));
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

test("terminal bytes are buffered until FrankenTerm accepts input", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  const received = [];
  web.state.terminal = {
    feed(bytes) {
      received.push(new TextDecoder().decode(bytes));
    },
    drainEncodedInputBytes() {
      return new Uint8Array();
    },
    screenReaderMirrorText() {
      return received.join("");
    },
    render() {},
  };
  web.state.terminalAcceptsBytes = false;

  assert.equal(web.feedTerminalBytes(new TextEncoder().encode("boot output")), true);
  assert.equal(received.length, 0);
  assert.equal(web.state.pendingTerminalByteLength, "boot output".length);

  web.state.terminalAcceptsBytes = true;
  assert.equal(web.flushPendingTerminalBytes(), true);

  assert.deepEqual(received, ["boot output"]);
  assert.equal(web.state.pendingTerminalByteLength, 0);
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

test("terminal composer keydown handler preserves control sends and propagation", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.terminalFallbackActive = true;
  web.el.terminalInlineInput.value = "";
  const sent = [];
  web.state.ws = {
    readyState: WebSocket.OPEN,
    send(payload) {
      sent.push(JSON.parse(payload));
    },
  };

  let prevented = 0;
  let stopped = 0;
  assert.equal(web.handleTerminalInlineInputKeydown({
    key: "ArrowUp",
    preventDefault() {
      prevented += 1;
    },
    stopPropagation() {
      stopped += 1;
    },
  }), true);
  assert.equal(prevented, 1);
  assert.equal(stopped, 1);
  assert.equal(sent.at(-1).type, "input_text");
  assert.equal(sent.at(-1).data, "\x1b[A");

  web.el.terminalInlineInput.value = "edit this text";
  prevented = 0;
  stopped = 0;
  assert.equal(web.handleTerminalInlineInputKeydown({
    key: "ArrowUp",
    preventDefault() {
      prevented += 1;
    },
    stopPropagation() {
      stopped += 1;
    },
  }), false);
  assert.equal(prevented, 0);
  assert.equal(stopped, 1);
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

test("input ack cleanup preserves sent echo without keeping Node alive", () => {
  resetWebState();
  const id = "web-test-ack-cleanup";
  web.state.pendingInputMessages.set(id, { text: "cleanup me", status: "pending", detail: "" });
  const originalSetTimeout = window.setTimeout;
  let cleanupCallback = null;
  let cleanupDelay = null;
  let unrefCalled = false;
  window.setTimeout = (callback, delay) => {
    cleanupCallback = callback;
    cleanupDelay = delay;
    return {
      unref() {
        unrefCalled = true;
      },
    };
  };

  try {
    web.handleSocketText(JSON.stringify({
      type: "input_ack",
      clientMessageId: id,
      delivered: true,
      method: "tmux_submit_line",
    }));
  } finally {
    window.setTimeout = originalSetTimeout;
  }

  assert.equal(cleanupDelay, 2500);
  assert.equal(unrefCalled, true);
  assert.ok(web.el.terminalInputEcho.textContent.includes("sent: cleanup me"));
  assert.equal(web.state.pendingInputMessages.get(id).status, "sent");

  cleanupCallback();
  assert.equal(web.state.pendingInputMessages.has(id), false);
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

test("websocket control events patch live session state", () => {
  resetWebState();
  web.state.sessions = [rawSession({ state: "idle", thought: null, token_count: 0 })];
  web.state.selectedSessionId = "sess_0";

  web.handleSocketText(JSON.stringify({
    type: "control_event",
    event: "session_state",
    sessionId: "sess_0",
    payload: {
      state: "attention",
      previous_state: "idle",
      current_command: "cargo test",
      state_evidence: { confidence: "high", observed_at: "2026-05-15T10:00:00Z", cause: "poll" },
      transport_health: "degraded",
      at: "2026-05-15T10:00:01Z",
    },
  }));
  assert.equal(web.state.sessions[0].state, "attention");
  assert.equal(web.state.sessions[0].current_command, "cargo test");
  assert.equal(web.state.sessions[0].transport_health, "degraded");
  assert.ok(web.el.terminalStatusStrip.textContent.includes("attention"));

  web.handleSocketText(JSON.stringify({
    type: "control_event",
    event: "session_title",
    sessionId: "sess_0",
    payload: { title: "/tmp/new-project", at: "2026-05-15T10:00:02Z" },
  }));
  assert.equal(web.state.sessions[0].cwd, "/tmp/new-project");
  assert.equal(web.state.sessions[0].terminal_title, "/tmp/new-project");

  web.handleSocketText(JSON.stringify({
    type: "control_event",
    event: "session_skill",
    sessionId: "sess_0",
    payload: { last_skill: "ui", at: "2026-05-15T10:00:03Z" },
  }));
  assert.equal(web.state.sessions[0].last_skill, "ui");

  web.handleSocketText(JSON.stringify({
    type: "control_event",
    event: "thought_update",
    sessionId: "sess_0",
    payload: {
      thought: "operator response needed",
      token_count: 64000,
      context_limit: 128000,
      rest_state: "waiting",
      commit_candidate: true,
      action_cues: [{ kind: "awaiting_user" }],
      objective_changed: true,
      at: "2026-05-15T10:00:04Z",
    },
  }));
  assert.equal(web.state.sessions[0].thought, "operator response needed");
  assert.equal(web.state.sessions[0].token_count, 64000);
  assert.equal(web.state.sessions[0].commit_candidate, true);
  assert.equal(web.state.sessions[0].objective_changed_at, "2026-05-15T10:00:04Z");
});

test("session refresh backs off while selected event stream is open", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.ws = { readyState: WebSocket.OPEN, sessionId: "sess_0" };

  assert.equal(web.sessionEventStreamOpen(), true);
  assert.equal(web.sessionRefreshDelayMs(), 10000);

  web.state.ws = { readyState: WebSocket.OPEN, sessionId: "other" };
  assert.equal(web.sessionEventStreamOpen(), false);
  assert.equal(web.sessionRefreshDelayMs(), 2500);

  web.state.ws = { readyState: WebSocket.OPEN, sessionId: "sess_0" };
  web.state.followPublishedSelection = true;
  assert.equal(web.sessionRefreshDelayMs(), 2500);
});

test("websocket control events refresh selected workbench sidecars", async () => {
  resetWebState();
  web.state.sessions = [rawSession({ state: "idle" })];
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  const calls = [];
  globalThis.fetch = async (path) => {
    calls.push(String(path));
    return jsonResponse(200, {});
  };

  web.handleSocketText(JSON.stringify({
    type: "control_event",
    event: "session_state",
    sessionId: "sess_0",
    payload: {
      state: "attention",
      previous_state: "idle",
      state_evidence: { confidence: "high", observed_at: "2026-05-15T10:00:00Z", cause: "event" },
      transport_health: "healthy",
      at: "2026-05-15T10:00:01Z",
    },
  }));
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.ok(calls.some((path) => path.includes("/agent-context")));
  assert.ok(calls.some((path) => path.includes("/pane-tail")));
  assert.ok(!calls.some((path) => path === "/v1/sessions"));
});

test("websocket lifecycle events create and stale sessions", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";

  web.handleSocketText(JSON.stringify({
    type: "lifecycle_event",
    event: "session_created",
    sessionId: "sess_1",
    reason: "manual_tmux_adopt",
    summary: rawSession({ session_id: "sess_1", tmux_name: "archer", cwd: "/tmp/archer" }),
  }));
  assert.equal(web.state.sessions.length, 2);
  assert.equal(web.state.sessions[1].tmux_name, "archer");

  web.handleSocketText(JSON.stringify({
    type: "lifecycle_event",
    event: "session_deleted",
    sessionId: "sess_0",
    reason: "tmux_reconcile_missing",
    deleteMode: "detach_bridge",
    tmuxSessionAlive: false,
  }));
  assert.equal(web.state.sessions[0].state, "exited");
  assert.equal(web.state.sessions[0].is_stale, true);
  assert.equal(web.state.sessions[0].transport_health, "disconnected");
  assert.ok(web.el.terminalStatusStrip.textContent.includes("session ended"));
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
      turns: [{ id: "codex-turn-10", source: "Codex", text: "build the workbench", order: 1, byte_start: 10, byte_end: 80 }],
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
  assert.equal(web.el.terminalWorkbenchStatus.textContent, "structured context");
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
    turns: [{ id: "codex-turn-10", source: "Codex", text: "build the cockpit", order: 1, byte_start: 10, byte_end: 80 }],
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
    if (String(path).includes("/transcript?")) {
      return jsonResponse(200, {
        session_id: "sess_0",
        available: true,
        cwd: "/tmp/project",
        tool: "Codex",
        selected_turn_id: "codex-turn-10",
        selected_turn: { id: "codex-turn-10", source: "Codex", text: "build the cockpit", order: 1, byte_start: 10, byte_end: 80 },
        next_cursor: 240,
        turns: [
          { id: "codex-turn-10", source: "Codex", text: "build the cockpit", order: 1, byte_start: 10, byte_end: 80 },
        ],
        records: [
          {
            id: "codex-record-100",
            source: "Codex",
            kind: "function_call",
            summary: "exec: cargo test",
            raw: "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\"}}",
            byte_start: 100,
            byte_end: 160,
            truncated: false,
          },
          {
            id: "codex-record-170",
            source: "Codex",
            kind: "agent_message",
            summary: "finished green",
            raw: "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"finished green\"}}",
            byte_start: 170,
            byte_end: 240,
            truncated: false,
          },
        ],
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
  assert.ok(requested.some((path) => String(path).startsWith("/v1/sessions/sess_0/transcript?")));
  assert.ok(requested.includes("/v1/sessions/sess_0/mermaid-artifact"));
  assert.ok(requested.includes("/v1/sessions/sess_0/git-diff"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Turns"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("build the cockpit"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Post-turn JSONL"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("Activity"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("workbench-log-lens"));
  assert.ok(web.el.terminalWorkbenchWidgets.innerHTML.includes("data-log-kind=\"command\""));
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

test("terminal workbench widget event handlers preserve selection, filters, and sheets", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.workbenchWidgets.transcript = { records: [{ id: "old" }] };
  web.state.workbenchWidgets.transcriptTurnId = "old-turn";
  web.state.workbenchWidgets.transcriptNextCursor = 42;

  const target = (matches, options = {}) => ({
    value: options.value,
    closest(selector) {
      return matches[selector] ?? null;
    },
    matches(selector) {
      return Boolean(matches[selector]);
    },
  });
  const eventFor = (eventTarget) => {
    let prevented = 0;
    return {
      target: eventTarget,
      preventDefault() {
        prevented += 1;
      },
      get prevented() {
        return prevented;
      },
    };
  };

  const turnEvent = eventFor(target({
    "[data-workbench-turn-id]": { dataset: { workbenchTurnId: "turn-2" } },
  }));
  web.handleTerminalWorkbenchWidgetsClick(turnEvent);
  assert.equal(turnEvent.prevented, 1);
  assert.equal(web.state.workbenchSelectedTurnId, "turn-2");
  assert.equal(web.state.workbenchWidgets.transcript, null);
  assert.equal(web.state.workbenchWidgets.transcriptTurnId, "");
  assert.equal(web.state.workbenchWidgets.transcriptNextCursor, 0);

  const logModeEvent = eventFor(target({
    "[data-workbench-log-mode]": { dataset: { workbenchLogMode: "raw" } },
  }));
  web.handleTerminalWorkbenchWidgetsClick(logModeEvent);
  assert.equal(logModeEvent.prevented, 1);
  assert.equal(web.state.workbenchLogMode, "raw");

  const mermaidEvent = eventFor(target({ "[data-workbench-open-mermaid]": {} }));
  web.handleTerminalWorkbenchWidgetsClick(mermaidEvent);
  assert.equal(mermaidEvent.prevented, 1);
  assert.equal(web.state.activeSheet, "mermaid");

  assert.equal(web.handleTerminalWorkbenchWidgetsClick({ target: target({}), preventDefault() {} }), false);
});

test("Mermaid artifact renderer bounds source and advertised plan tabs", () => {
  resetWebState();
  const largeSource = `graph TD\n${"A-->B\n".repeat(14000)}`;
  const manyPlanFiles = Array.from({ length: 40 }, (_value, index) => `plan-${index}.md`);

  web.renderMermaidArtifact({
    session_id: "sess_0",
    available: true,
    path: "/tmp/project/docs/huge.mmd",
    source: largeSource,
    plan_files: ["../secret.txt", "plan.md", "WORKGRAPH.md", ...manyPlanFiles],
  });

  assert.ok(web.state.mermaidArtifact.source.length < largeSource.length);
  assert.ok(web.el.mermaidSource.textContent.includes("truncated after 64 KiB"));
  assert.equal(web.state.mermaidArtifact.planFiles.length, 32);
  assert.ok(!web.state.mermaidArtifact.planFiles.includes("../secret.txt"));
  assert.ok(web.state.mermaidArtifact.planFiles.includes("WORKGRAPH.md"));
  assert.ok(web.el.mermaidSummary.textContent.includes("showing first 32"));
  assert.ok(web.el.mermaidSummary.textContent.includes("unsafe name"));
});

test("Mermaid artifact preview uses image URL instead of injected SVG markup", () => {
  resetWebState();
  web.state.mermaidArtifact.svgUrl = "blob:swimmers-test-svg";

  web.renderMermaidArtifact({
    session_id: "sess_0",
    available: true,
    path: "/tmp/project/docs/diagram.mmd",
    source: "graph TD\nA-->B\n",
    plan_files: [],
  });

  const img = web.el.mermaidPreview.children.find((child) => child.id === "img");
  assert.ok(img, "preview should be rendered as an image");
  assert.equal(img.src, "blob:swimmers-test-svg");
  assert.ok(!web.el.mermaidPreview.innerHTML.includes("<script"));
});

test("Mermaid plan loader rejects path-ish names and bounds huge content", async () => {
  resetWebState();
  web.state.sessions = [rawSession({ session_id: "sess_0" })];
  web.state.selectedSessionId = "sess_0";
  web.state.mermaidArtifact.planFiles = ["plan.md"];
  const requested = [];
  globalThis.fetch = async (path) => {
    requested.push(path);
    return jsonResponse(200, {
      session_id: "sess_0",
      name: "plan.md",
      content: "x".repeat(140000),
    });
  };

  await web.loadMermaidPlanFile("../secret.txt");
  assert.equal(requested.length, 0);
  assert.ok(web.el.mermaidPlanContent.textContent.includes("not allowed"));
  assert.ok(web.el.mermaidPlanContent.classList.contains("error"));

  await web.loadMermaidPlanFile("plan.md");
  assert.equal(requested.length, 1);
  assert.ok(String(requested[0]).includes("name=plan.md"));
  assert.ok(web.state.mermaidArtifact.planContent.length < 140000);
  assert.ok(web.el.mermaidPlanContent.textContent.includes("truncated after 128 KiB"));
  assert.ok(web.el.mermaidSummary.textContent.includes("truncated to 128 KiB"));
  assert.ok(!web.el.mermaidPlanContent.classList.contains("error"));
});

test("workbench rerender preserves parent scrollTop and skips identical writes", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.workbenchWidgets.sessionId = "sess_0";
  web.state.workbenchWidgets.timeline = { events: [] };
  web.state.workbenchWidgets.paneTail = { text: "alpha\nbeta\n" };

  web.renderWorkbenchWidgets();
  const firstHtml = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(firstHtml.length > 0, "first render writes initial HTML");
  assert.equal(web.state.workbenchWidgets.lastHtml, firstHtml);

  // Operator scrolls the workbench down to read the lower widgets.
  web.el.terminalWorkbench.scrollTop = 240;

  // Polling cadence rerenders with the same payload — must not touch the DOM.
  web.el.terminalWorkbenchWidgets.innerHTML = "TAINTED";
  web.renderWorkbenchWidgets();
  assert.equal(
    web.el.terminalWorkbenchWidgets.innerHTML,
    "TAINTED",
    "identical payload skips the innerHTML write so scroll/selection survive",
  );
  assert.equal(web.el.terminalWorkbench.scrollTop, 240);

  // Real payload change rebuilds the DOM but restores the scroll position.
  web.el.terminalWorkbenchWidgets.innerHTML = firstHtml;
  web.state.workbenchWidgets.paneTail = { text: "alpha\nbeta\ngamma\n" };
  web.renderWorkbenchWidgets();
  assert.notEqual(
    web.el.terminalWorkbenchWidgets.innerHTML,
    firstHtml,
    "changed payload rewrites widget HTML",
  );
  assert.equal(
    web.el.terminalWorkbench.scrollTop,
    240,
    "scrollTop on the overflow:auto parent is restored after the rerender",
  );
});

test("workbench rerender restores details open state by title", () => {
  resetWebState();
  web.el.terminalWorkbench.scrollTop = 180;
  web.state.workbenchWidgets.lastHtml = "<details>previous</details>";

  const previousDetails = [
    mockWorkbenchDetails("Turns", false),
    mockWorkbenchDetails("Logs", true),
    mockWorkbenchDetails("Skills", false),
  ];
  const nextDetails = [
    mockWorkbenchDetails("Turns", true),
    mockWorkbenchDetails("Logs", false),
    mockWorkbenchDetails("Skills", true),
    mockWorkbenchDetails("Artifacts", true),
  ];
  const originalQuerySelectorAll = web.el.terminalWorkbenchWidgets.querySelectorAll;
  let calls = 0;
  web.el.terminalWorkbenchWidgets.querySelectorAll = (selector) => {
    assert.equal(selector, "details.workbench-widget");
    calls += 1;
    return calls === 1 ? previousDetails : nextDetails;
  };

  try {
    web.writeWorkbenchWidgetsHtml("<details class=\"workbench-widget\">next</details>");
  } finally {
    web.el.terminalWorkbenchWidgets.querySelectorAll = originalQuerySelectorAll;
  }

  assert.equal(calls, 2, "snapshots old details and restores into new details");
  assert.equal(nextDetails[0].open, false, "collapsed Turns remains collapsed");
  assert.equal(nextDetails[1].open, true, "expanded Logs remains expanded");
  assert.equal(nextDetails[2].open, false, "collapsed Skills remains collapsed");
  assert.equal(nextDetails[3].open, true, "new widgets keep their rendered default");
  assert.equal(web.el.terminalWorkbench.scrollTop, 180);
});

test("workbench transcript lens classifies, filters, searches, and preserves raw logs", async () => {
  resetWebState();
  const tailText = [
    "... truncated ...",
    "• You ran cat makefile",
    "cargo test",
    "error: failed test",
    "@@ -1 +1 @@",
    "+new line",
    "plain output",
    "<secret>",
  ].join("\n");

  const blocks = web.renderTranscriptBlocks(tailText);
  assert.deepEqual(
    blocks.map((block) => block.kind),
    ["truncation", "operator", "command", "status", "diff", "output"],
  );
  assert.equal(blocks.at(-1).lines.includes("<secret>"), true);

  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.workbenchWidgets.sessionId = "sess_0";
  web.state.workbenchWidgets.paneTail = { session_id: "sess_0", text: tailText };
  web.state.workbenchWidgets.timeline = { events: [] };
  web.renderWorkbenchWidgets();

  let html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-log-lens"));
  assert.ok(html.includes("data-log-kind=\"truncation\""));
  assert.ok(html.includes("data-log-kind=\"operator\""));
  assert.ok(html.includes("data-log-kind=\"command\""));
  assert.ok(html.includes("data-log-kind=\"status\""));
  assert.ok(html.includes("data-log-kind=\"diff\""));
  assert.ok(html.includes("data-log-kind=\"output\""));
  assert.ok(html.includes("Search logs"));

  web.state.workbenchLogFilter = "command";
  web.renderWorkbenchWidgets();
  html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("data-log-kind=\"command\""));
  assert.equal(html.includes("plain output"), false);

  web.state.workbenchLogFilter = "all";
  web.state.workbenchLogSearch = "cargo";
  web.renderWorkbenchWidgets();
  html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-log-mark"));
  assert.ok(html.includes("cargo"));
  assert.equal(html.includes("plain output"), false);

  web.state.workbenchLogSearch = "";
  web.state.workbenchLogMode = "raw";
  web.renderWorkbenchWidgets();
  html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-log-raw"));
  assert.ok(html.includes("&lt;secret&gt;"));
  assert.equal(html.includes("<secret>"), false);
});

test("workbench Turns panel is user-only and logs follow selected post-turn JSONL", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.workbenchWidgets.sessionId = "sess_0";
  web.state.agentContextSessionId = "sess_0";
  web.state.agentContextPayload = {
    available: true,
    turns: [
      { id: "turn-1", source: "Codex", text: "first user turn", order: 1, byte_start: 10, byte_end: 40 },
      { id: "turn-2", source: "Codex", text: "second user turn", order: 2, byte_start: 90, byte_end: 130 },
    ],
    recent_actions: [{ tool: "exec", detail: "assistant tool call" }],
  };
  web.state.workbenchSelectedTurnId = "turn-1";
  web.state.workbenchWidgets.transcript = {
    session_id: "sess_0",
    available: true,
    selected_turn_id: "turn-1",
    selected_turn: { id: "turn-1", source: "Codex", text: "first user turn", order: 1, byte_start: 10, byte_end: 40 },
    next_cursor: 190,
    turns: web.state.agentContextPayload.turns,
    records: [
      {
        id: "record-50",
        source: "Codex",
        kind: "function_call",
        role: null,
        summary: "exec: cargo test selected turn",
        raw: "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\"}}",
        byte_start: 50,
        byte_end: 90,
        truncated: false,
      },
      {
        id: "record-140",
        source: "Codex",
        kind: "agent_message",
        role: null,
        summary: "assistant after selected turn",
        raw: "{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"assistant after selected turn\"}}",
        byte_start: 140,
        byte_end: 190,
        truncated: false,
      },
    ],
  };

  web.renderWorkbenchWidgets();

  let html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-turn"));
  assert.ok(html.includes("first user turn"));
  assert.ok(html.includes("second user turn"));
  const turnsSection = html.slice(html.indexOf("workbench-turn-list"), html.indexOf("Post-turn JSONL"));
  assert.equal(turnsSection.includes("assistant tool call"), false);
  assert.ok(html.includes("Post-turn JSONL"));
  assert.ok(html.includes("cargo test selected turn"));

  web.state.workbenchLogMode = "raw";
  web.renderWorkbenchWidgets();
  html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-log-raw"));
  assert.ok(html.includes("&quot;function_call&quot;"));
});

test("workbench JSONL records render parsed event fields", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.workbenchWidgets.sessionId = "sess_0";
  web.state.workbenchWidgets.timeline = { events: [] };
  web.state.workbenchWidgets.transcript = {
    session_id: "sess_0",
    available: true,
    selected_turn_id: "turn-1",
    selected_turn: { id: "turn-1", source: "Codex", text: "run the checks", order: 1, byte_start: 10, byte_end: 40 },
    records: [
      {
        id: "call-1",
        source: "Codex",
        kind: "function_call",
        summary: "exec: cargo test --locked",
        raw: JSON.stringify({
          type: "response_item",
          payload: {
            type: "function_call",
            name: "exec_command",
            arguments: JSON.stringify({ cmd: "cargo test --locked", workdir: "/tmp/project" }),
          },
        }),
        byte_start: 50,
        byte_end: 130,
        truncated: false,
      },
      {
        id: "output-1",
        source: "Codex",
        kind: "function_call_output",
        summary: "finished green",
        raw: JSON.stringify({
          type: "response_item",
          payload: {
            type: "function_call_output",
            call_id: "call_1",
            output: JSON.stringify({ output: "finished green\nResult file written at target/reports/WG-002_RESULT.md\n" }),
          },
        }),
        byte_start: 131,
        byte_end: 190,
        truncated: false,
      },
      {
        id: "tokens-1",
        source: "Codex",
        kind: "token_count",
        summary: "usage update",
        raw: JSON.stringify({
          type: "event_msg",
          payload: {
            type: "token_count",
            info: { total_token_usage: { input_tokens: 150, output_tokens: 20 } },
            model_context_window: 258400,
          },
        }),
        byte_start: 191,
        byte_end: 240,
        truncated: false,
      },
    ],
  };

  web.renderWorkbenchWidgets();

  let html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-log-record"));
  assert.ok(html.includes("data-log-kind=\"command\""));
  assert.ok(html.includes("data-log-kind=\"output\""));
  assert.ok(html.includes("data-log-kind=\"status\""));
  assert.ok(html.includes("exec_command"));
  assert.ok(html.includes("cargo test --locked"));
  assert.ok(html.includes("/tmp/project"));
  assert.ok(html.includes("finished green"));
  assert.ok(html.includes("Start here"));
  assert.ok(html.includes("Outcome"));
  assert.ok(html.includes("Tool actions"));
  assert.ok(html.includes("Where to read"));
  assert.ok(html.includes("target/reports/WG-002_RESULT.md"));
  assert.ok(html.includes("Event stream"));
  assert.ok(html.includes("call_1"));
  assert.ok(html.includes("window"));
  assert.ok(html.includes("258400"));
  assert.ok(html.includes("<summary>JSON</summary>"));

  web.state.workbenchLogFilter = "output";
  web.renderWorkbenchWidgets();
  html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("data-log-kind=\"output\""));
  assert.equal(html.includes("data-log-kind=\"command\""), false);
  assert.equal(html.includes("cargo test --locked"), false);

  web.state.workbenchLogFilter = "all";
  web.state.workbenchLogSearch = "locked";
  web.renderWorkbenchWidgets();
  html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("workbench-log-mark"));
  assert.ok(html.includes("cargo test"));
  assert.equal(html.includes("finished green"), false);
});

test("workbench Claude JSONL messages render readable content instead of raw envelopes", () => {
  resetWebState();
  web.state.selectedSessionId = "sess_0";
  web.state.trogdorAtlasOpen = false;
  web.state.workbenchWidgets.sessionId = "sess_0";
  web.state.workbenchWidgets.timeline = { events: [] };
  web.state.workbenchWidgets.transcript = {
    session_id: "sess_0",
    available: true,
    selected_turn_id: "turn-1",
    selected_turn: { id: "turn-1", source: "Claude Code", text: "build release packet", order: 1, byte_start: 10, byte_end: 40 },
    records: [
      {
        id: "claude-record-1",
        source: "Claude Code",
        kind: "assistant_message",
        role: "assistant",
        summary: "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\"}}",
        raw: JSON.stringify({
          type: "assistant",
          message: {
            role: "assistant",
            content: [
              {
                type: "text",
                text: "WG-002 implementation is complete. Result file written at target/reference-board-release-spine/WG-002_RESULT.md.",
              },
            ],
          },
        }),
        byte_start: 50,
        byte_end: 120,
        truncated: false,
      },
      {
        id: "claude-record-2",
        source: "Claude Code",
        kind: "assistant_message",
        role: "assistant",
        summary: "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"}]}}",
        raw: JSON.stringify({
          type: "assistant",
          message: {
            role: "assistant",
            content: [
              {
                type: "tool_use",
                id: "toolu_1",
                name: "file-history-snapshot",
                input: { path: "/Users/b/repos/pcbcd/src/release.rs" },
              },
            ],
          },
        }),
        byte_start: 121,
        byte_end: 180,
        truncated: false,
      },
    ],
  };

  web.renderWorkbenchWidgets();

  const html = web.el.terminalWorkbenchWidgets.innerHTML;
  assert.ok(html.includes("build release packet"));
  assert.ok(html.includes("WG-002 implementation is complete"));
  assert.ok(html.includes("target/reference-board-release-spine/WG-002_RESULT.md"));
  assert.ok(html.includes("file-history-snapshot"));
  assert.ok(html.includes("/Users/b/repos/pcbcd/src/release.rs"));
  assert.ok(html.includes("data-log-kind=\"command\""));
  const briefHtml = html.slice(0, html.indexOf("Event stream"));
  assert.equal(briefHtml.includes("&quot;type&quot;:&quot;assistant&quot;,&quot;message&quot;"), false);
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
  assert.equal(web.state.terminalMirrorText, "line 1\nline 2");
  assert.equal(web.el.terminalA11yMirror.value, "line 1\nline 2");

  web.state.terminalFallbackAutoFollow = false;
  web.el.terminalFallback.scrollTop = 120;
  web.el.terminalFallback.scrollHeight = 800;

  web.updateTerminalFallbackText("line 3");

  assert.equal(web.el.terminalFallback.textContent, "line 3");
  assert.equal(web.el.terminalFallback.scrollTop, 120);
  assert.equal(web.state.terminalMirrorText, "line 3");
  assert.equal(web.el.terminalA11yMirror.value, "line 3");
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
  web.el.mobileKeyboardProxy.value = "delete sentinel";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "deleteContentBackward", data: null }), true);
  assert.equal(web.el.mobileKeyboardProxy.value, "");
  assert.equal(sent.at(-1).type, "input_text");
  assert.equal(sent.at(-1).data, "\x7f");

  web.el.mobileKeyboardProxy.value = "newline sentinel";
  assert.equal(web.handleMobileKeyboardProxyInput({ inputType: "insertLineBreak", data: null }), true);
  assert.equal(sent.at(-1).type, "input_text");
  assert.equal(sent.at(-1).data, "\r");

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
  web.state.readOnly = true;
  web.el.sendInput.disabled = true;
  web.el.tokenInput.value = "ignored";
  assert.equal(await web.handleAuthTokenButtonAction("clear"), true);
  assert.equal(web.state.token, "");
  assert.equal(storage.has("swimmers.web.token"), false);
  assert.equal(web.state.readOnly, false);
  assert.equal(web.el.sendInput.disabled, false);
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
