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

const { __swimmersWebTest: web } = await import("./app.js?workbench-behavior-test");

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
