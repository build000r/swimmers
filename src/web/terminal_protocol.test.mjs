import test from "node:test";
import assert from "node:assert/strict";

import {
  buildSessionSocketUrl,
  decodeTerminalOutputFrame,
  fallbackTextForKeyEvent,
  keyModifiers,
  readUint64Decimal,
  sessionSocketAuthMessageForToken,
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
