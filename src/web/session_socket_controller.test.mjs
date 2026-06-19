import test from "node:test";
import assert from "node:assert/strict";

import {
  createSessionSocketController,
} from "./session_socket_controller.js";

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function runtimeFixture(overrides = {}) {
  const calls = [];
  const sockets = [];
  class TestWebSocket {
    static OPEN = 1;

    constructor(url) {
      this.url = String(url);
      this.readyState = TestWebSocket.OPEN;
      this.sent = [];
      this.closed = false;
      sockets.push(this);
    }

    send(message) {
      this.sent.push(message);
    }

    close() {
      this.closed = true;
      this.readyState = 3;
      calls.push(["closeSocket", this.sessionId || ""]);
    }
  }

  const state = {
    connectionGeneration: 0,
    lastTerminalSeqBySession: new Map(),
    reconnectAttempt: 0,
    reconnectTimer: null,
    readOnly: false,
    terminal: {},
    terminalFallbackActive: false,
    ws: null,
    ...overrides.state,
  };
  let session = overrides.session ?? { session_id: "sess_b" };
  const runtime = {
    state,
    WebSocketClass: TestWebSocket,
    window: {
      location: new URL("http://swimmers.test/"),
      setTimeout(callback, delay) {
        calls.push(["setTimeout", delay]);
        return { callback, delay };
      },
    },
    setupHudSurface: async () => calls.push(["setupHudSurface"]),
    setupTerminalSurface: async () => calls.push(["setupTerminalSurface"]),
    currentSession: () => session,
    teardownTerminal: () => calls.push(["teardownTerminal"]),
    disconnectSocket() {
      calls.push(["disconnectSocket", state.ws?.sessionId || ""]);
      state.connectionGeneration += 1;
      if (state.ws) {
        state.ws.onopen = null;
        state.ws.onmessage = null;
        state.ws.onclose = null;
        state.ws.onerror = null;
        state.ws.close();
        state.ws = null;
      }
    },
    syncWriteAccess: () => calls.push(["syncWriteAccess"]),
    setConnectionStatus: (...args) => calls.push(["setConnectionStatus", ...args]),
    setModeStatus: (...args) => calls.push(["setModeStatus", ...args]),
    syncTerminalTools: () => calls.push(["syncTerminalTools"]),
    measureAndResizeSurface: (...args) => calls.push(["measureAndResizeSurface", ...args]),
    scheduleSessionRefresh: () => calls.push(["scheduleSessionRefresh"]),
    reconnectDelayMs: () => 1000,
    feedTerminalBytes: (bytes) => calls.push(["feedTerminalBytes", Array.from(bytes)]),
    mergeSummary: (summary) => calls.push(["mergeSummary", summary]),
    handleInputAck: (message) => calls.push(["handleInputAck", message]),
    applyControlEvent: (message) => calls.push(["applyControlEvent", message]),
    applyLifecycleEvent: (message) => calls.push(["applyLifecycleEvent", message]),
    refreshSessions: () => calls.push(["refreshSessions"]),
    ...overrides.runtime,
  };
  return {
    calls,
    controller: createSessionSocketController(runtime),
    runtime,
    setSession(nextSession) {
      session = nextSession;
    },
    sockets,
    state,
  };
}

test("connectSelectedSession connects a socket for the selected session", async () => {
  const { calls, controller, sockets, state } = runtimeFixture();

  await controller.connectSelectedSession();

  assert.equal(sockets.length, 1);
  assert.equal(sockets[0].url, "ws://swimmers.test/ws/sessions/sess_b?framed=1");
  assert.equal(state.ws, sockets[0]);
  assert.equal(state.ws.sessionId, "sess_b");
  assert.deepEqual(calls.slice(0, 4), [
    ["setupHudSurface"],
    ["setupTerminalSurface"],
    ["disconnectSocket", ""],
    ["syncWriteAccess"],
  ]);
});

test("connectSelectedSession disconnects stale sockets before async terminal setup", async () => {
  const gate = deferred();
  const oldSocket = {
    binaryType: "arraybuffer",
    framedOutput: false,
    onmessage() {
      runtime.feedTerminalBytes(new Uint8Array([65, 66]));
    },
    readyState: 1,
    sessionId: "sess_a",
    closed: false,
    close() {
      this.closed = true;
      this.readyState = 3;
    },
  };
  const { calls, controller, runtime, sockets, state } = runtimeFixture({
    state: {
      connectionGeneration: 7,
      ws: oldSocket,
    },
    runtime: {
      setupTerminalSurface: async () => {
        calls.push(["setupTerminalSurface:start"]);
        await gate.promise;
        calls.push(["setupTerminalSurface:done"]);
      },
    },
  });

  const connecting = controller.connectSelectedSession();

  await Promise.resolve();

  assert.equal(oldSocket.closed, true);
  assert.equal(oldSocket.onmessage, null);
  assert.equal(state.ws, null);
  assert.equal(state.connectionGeneration, 8);
  assert.deepEqual(calls.slice(0, 3), [
    ["setupHudSurface"],
    ["disconnectSocket", "sess_a"],
    ["setupTerminalSurface:start"],
  ]);

  oldSocket.onmessage?.({ data: new Uint8Array([65, 66]).buffer });
  assert.equal(calls.filter(([name]) => name === "feedTerminalBytes").length, 0);

  gate.resolve();
  await connecting;

  assert.equal(sockets.length, 1);
  assert.equal(sockets[0].sessionId, "sess_b");
  assert.equal(state.ws, sockets[0]);
  assert.equal(calls.filter(([name]) => name === "feedTerminalBytes").length, 0);
});

test("connectSelectedSession drops stale async setup after selection changes", async () => {
  const gate = deferred();
  const { calls, controller, setSession, sockets, state } = runtimeFixture({
    session: { session_id: "sess_a" },
    runtime: {
      setupTerminalSurface: async () => {
        calls.push(["setupTerminalSurface:start"]);
        await gate.promise;
        calls.push(["setupTerminalSurface:done"]);
      },
    },
  });

  const connecting = controller.connectSelectedSession();

  await Promise.resolve();
  setSession({ session_id: "sess_b" });
  gate.resolve();
  await connecting;

  assert.equal(sockets.length, 0);
  assert.equal(state.ws, null);

  await controller.connectSelectedSession();

  assert.equal(sockets.length, 1);
  assert.equal(sockets[0].sessionId, "sess_b");
  assert.equal(state.ws, sockets[0]);
});
