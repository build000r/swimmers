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
