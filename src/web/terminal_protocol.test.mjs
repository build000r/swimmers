import test from "node:test";
import assert from "node:assert/strict";

import {
  buildSessionSocketUrl,
  decodeTerminalOutputFrame,
  fallbackTextForKeyEvent,
  keyModifiers,
  readUint64Decimal,
  selectedSessionConnectionPlan,
  sessionSocketAuthMessageForToken,
  sessionSocketAttachPlan,
  sessionSocketAttachStatePlan,
  sessionSocketCloseExecutionPlan,
  sessionSocketClosePlan,
  sessionSocketErrorPlan,
  sessionSocketMessageExecutionPlan,
  sessionSocketMessagePlan,
  sessionSocketOpenExecutionPlan,
  sessionSocketOpenPlan,
  sessionSocketOpenStatus,
  sessionSocketOpenStatusPlan,
  sessionSocketReconnectPlan,
  sessionSocketReconnectStatus,
  terminalControlKeyEvent,
} from "./terminal_protocol.js";

test("buildSessionSocketUrl opts into framed resume without leaking auth", () => {
  const url = buildSessionSocketUrl(
    { session_id: "sess/1" },
    new URL("https://swimmers.test:3210/base"),
    "42",
  );

  assert.equal(url.protocol, "wss:");
  assert.equal(url.pathname, "/ws/sessions/sess%2F1");
  assert.equal(url.searchParams.get("framed"), "1");
  assert.equal(url.searchParams.get("resume_from_seq"), "42");
  assert.equal(url.toString().includes("token"), false);
});

test("sessionSocketAuthMessageForToken trims and omits empty tokens", () => {
  assert.equal(sessionSocketAuthMessageForToken(""), null);
  assert.equal(sessionSocketAuthMessageForToken("  "), null);
  assert.equal(sessionSocketAuthMessageForToken(" observer "), JSON.stringify({ type: "auth", token: "observer" }));
});

test("selectedSessionConnectionPlan preserves ordered setup gates", () => {
  const session = { session_id: "sess-1" };

  assert.deepEqual(selectedSessionConnectionPlan({ session: null }), { type: "teardown_terminal" });
  assert.deepEqual(selectedSessionConnectionPlan({ session }), {
    type: "setup_terminal",
    sessionId: "sess-1",
  });
  assert.deepEqual(selectedSessionConnectionPlan({
    session,
    terminalSurfaceChecked: true,
    hasTerminal: false,
    terminalFallbackActive: false,
  }), {
    type: "await_terminal_surface",
    sessionId: "sess-1",
  });
});

test("selectedSessionConnectionPlan preserves socket reuse and connect decisions", () => {
  const session = { session_id: "sess-1" };
  assert.deepEqual(selectedSessionConnectionPlan({
    session,
    terminalSurfaceChecked: true,
    hasTerminal: true,
    terminalFallbackActive: false,
    ws: { readyState: 1, sessionId: "sess-1" },
    openReadyState: 1,
  }), { type: "reuse_socket", sessionId: "sess-1" });
  assert.deepEqual(selectedSessionConnectionPlan({
    session,
    terminalSurfaceChecked: true,
    hasTerminal: true,
    terminalFallbackActive: false,
    ws: { readyState: 2, sessionId: "sess-1" },
    openReadyState: 1,
  }), { type: "connect_socket", sessionId: "sess-1" });
  assert.deepEqual(selectedSessionConnectionPlan({
    session,
    terminalSurfaceChecked: true,
    hasTerminal: false,
    terminalFallbackActive: true,
    ws: { readyState: 1, sessionId: "other" },
    openReadyState: 1,
  }), { type: "connect_socket", sessionId: "sess-1" });
});

test("sessionSocketAttachPlan preserves framed output and status copy", () => {
  const resumeUrl = new URL("ws://swimmers.test/ws/sessions/sess-1?framed=1&resume_from_seq=42");
  assert.deepEqual(sessionSocketAttachPlan(resumeUrl), {
    resumeFromSeq: "42",
    framedOutput: true,
    status: "connecting; resuming from seq 42",
  });

  const plainUrl = new URL("ws://swimmers.test/ws/sessions/sess-1?framed=0");
  assert.deepEqual(sessionSocketAttachPlan(plainUrl), {
    resumeFromSeq: "",
    framedOutput: false,
    status: "connecting; input disabled",
  });
});

test("sessionSocketAttachStatePlan preserves attach side-effect data", () => {
  assert.deepEqual(sessionSocketAttachStatePlan(
    { type: "connect_socket", sessionId: "sess-1" },
    { framedOutput: true, status: "connecting; input disabled" },
  ), {
    type: "attach_socket",
    binaryType: "arraybuffer",
    sessionId: "sess-1",
    framedOutput: true,
    readOnly: true,
    status: "connecting; input disabled",
  });
});

test("sessionSocketOpenPlan preserves stale close guard and auth status copy", () => {
  assert.deepEqual(sessionSocketOpenPlan({
    generation: 1,
    currentGeneration: 2,
    currentSocketMatches: true,
  }), { type: "close_stale" });
  assert.deepEqual(sessionSocketOpenPlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: false,
  }), { type: "close_stale" });
  assert.deepEqual(sessionSocketOpenPlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: true,
  }), { type: "attach" });
  assert.equal(sessionSocketOpenStatus(true), "authenticating; input disabled");
  assert.equal(sessionSocketOpenStatus(false), "attached");
});

test("sessionSocketOpenExecutionPlan preserves callback side-effect order flags", () => {
  assert.deepEqual(sessionSocketOpenExecutionPlan({
    generation: 1,
    currentGeneration: 2,
    currentSocketMatches: true,
  }), { type: "close_stale", closeSocket: true });
  assert.deepEqual(sessionSocketOpenExecutionPlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: true,
  }), {
    type: "attach",
    sendAuth: true,
    resizeTerminal: true,
    resetReconnectAttempt: true,
    scheduleRefresh: true,
  });
  assert.deepEqual(sessionSocketOpenStatusPlan(true), {
    type: "status",
    status: "authenticating; input disabled",
  });
});

test("sessionSocketMessagePlan preserves stale guard and text/binary routing", () => {
  assert.deepEqual(sessionSocketMessagePlan({
    generation: 1,
    currentGeneration: 2,
    currentSocketMatches: true,
    data: "hello",
  }), { type: "ignore" });
  assert.deepEqual(sessionSocketMessagePlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: false,
    data: "hello",
  }), { type: "ignore" });
  assert.deepEqual(sessionSocketMessagePlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: true,
    data: "hello",
  }), { type: "handle_text", text: "hello" });

  const data = new Uint8Array([65, 66]).buffer;
  const plan = sessionSocketMessagePlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: true,
    data,
  });
  assert.equal(plan.type, "feed_binary");
  assert.equal(plan.data, data);
});

test("sessionSocketMessageExecutionPlan materializes binary bytes for terminal routing", () => {
  assert.deepEqual(sessionSocketMessageExecutionPlan({
    generation: 1,
    currentGeneration: 2,
    currentSocketMatches: true,
    data: "hello",
  }), { type: "ignore" });
  assert.deepEqual(sessionSocketMessageExecutionPlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: true,
    data: "hello",
  }), { type: "handle_text", text: "hello" });

  const data = new Uint8Array([67, 68]).buffer;
  const plan = sessionSocketMessageExecutionPlan({
    generation: 1,
    currentGeneration: 1,
    currentSocketMatches: true,
    data,
  });
  assert.equal(plan.type, "feed_binary");
  assert.equal(plan.data, data);
  assert.deepEqual(Array.from(plan.bytes), [67, 68]);
});

test("sessionSocketClosePlan preserves reconnect guard and status rounding", () => {
  assert.deepEqual(sessionSocketClosePlan({
    generation: 1,
    currentGeneration: 2,
  }), { type: "ignore" });
  assert.deepEqual(sessionSocketClosePlan({
    generation: 1,
    currentGeneration: 1,
  }), { type: "schedule_reconnect" });
  assert.equal(sessionSocketReconnectStatus(2000), "disconnected; input disabled; retrying in 2s");
  assert.equal(sessionSocketReconnectStatus(2501), "disconnected; input disabled; retrying in 3s");
});

test("sessionSocketCloseExecutionPlan and error plan preserve status side effects", () => {
  assert.deepEqual(sessionSocketCloseExecutionPlan(2501), {
    type: "schedule_reconnect",
    incrementReconnectAttempt: true,
    status: "disconnected; input disabled; retrying in 3s",
    scheduleRefresh: true,
    delayMs: 2501,
  });
  assert.deepEqual(sessionSocketErrorPlan(), {
    type: "set_status",
    status: "attach failed; input disabled",
    muted: true,
  });
});

test("sessionSocketReconnectPlan preserves generation and selected-session gates", () => {
  assert.deepEqual(sessionSocketReconnectPlan({
    generation: 1,
    currentGeneration: 2,
    hasCurrentSession: true,
  }), { type: "ignore" });
  assert.deepEqual(sessionSocketReconnectPlan({
    generation: 1,
    currentGeneration: 1,
    hasCurrentSession: false,
  }), { type: "ignore" });
  assert.deepEqual(sessionSocketReconnectPlan({
    generation: 1,
    currentGeneration: 1,
    hasCurrentSession: true,
  }), { type: "reconnect" });
});

test("decodeTerminalOutputFrame parses opcode, sequence, and payload", () => {
  const frame = new Uint8Array([0x11, 0, 0, 0, 0, 0, 0, 0, 5, 65, 66]);
  const decoded = decodeTerminalOutputFrame(frame);

  assert.equal(decoded.seq, "5");
  assert.deepEqual(Array.from(decoded.payload), [65, 66]);
  assert.equal(decodeTerminalOutputFrame(new Uint8Array([65, 66])), null);
  assert.equal(readUint64Decimal(0, 7), "7");
});

test("fallbackTextForKeyEvent encodes printable, control, and navigation keys", () => {
  assert.equal(fallbackTextForKeyEvent({ kind: "key", phase: "down", key: "a", mods: 0 }), "a");
  assert.equal(fallbackTextForKeyEvent({ kind: "key", phase: "down", key: "c", mods: 4 }), "\x03");
  assert.equal(fallbackTextForKeyEvent({ kind: "key", phase: "down", key: "Tab", mods: 1 }), "\x1b[Z");
  assert.equal(fallbackTextForKeyEvent({ kind: "mouse", phase: "down", key: "a", mods: 0 }), "");
});

test("terminalControlKeyEvent and keyModifiers keep DOM controls stable", () => {
  assert.deepEqual(terminalControlKeyEvent("arrow-left"), {
    key: "ArrowLeft",
    code: "ArrowLeft",
    mods: 0,
    label: "Left",
  });
  assert.equal(terminalControlKeyEvent("missing"), null);
  assert.equal(keyModifiers({ shiftKey: true, altKey: true, ctrlKey: false, metaKey: true }), 11);
});
