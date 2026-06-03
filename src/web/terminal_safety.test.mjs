import test from "node:test";
import assert from "node:assert/strict";

import {
  MAX_TERMINAL_PASTE_BYTES,
  frankenTermLinkPolicy,
  isLoopbackHostname,
  safeAnchorHref,
  terminalTextWithinPasteBudget,
  utf8ByteLength,
} from "./terminal_safety.js";

test("FrankenTerm link policy allows HTTP only for loopback hosts", () => {
  globalThis.window = { location: new URL("http://localhost:3210/") };
  assert.equal(isLoopbackHostname("localhost"), true);
  assert.equal(isLoopbackHostname("dev.localhost"), true);
  assert.equal(isLoopbackHostname("127.9.8.7"), true);
  assert.deepEqual(frankenTermLinkPolicy(), { allowHttp: true, allowHttps: true });

  globalThis.window = { location: new URL("http://100.64.0.1:3210/") };
  assert.equal(isLoopbackHostname("100.64.0.1"), false);
  assert.deepEqual(frankenTermLinkPolicy(), { allowHttp: false, allowHttps: true });
});

test("safeAnchorHref rejects active-content URLs", () => {
  globalThis.window = { location: new URL("http://swimmers.test/base/") };

  assert.equal(safeAnchorHref("https://example.com/repo"), "https://example.com/repo");
  assert.equal(safeAnchorHref("/local/repo"), "http://swimmers.test/local/repo");
  assert.equal(safeAnchorHref("javascript:alert(1)"), "");
  assert.equal(safeAnchorHref("data:text/html,pwned"), "");
  assert.equal(safeAnchorHref("http://[::1"), "");
});

test("terminal paste budget measures UTF-8 bytes", () => {
  assert.equal(utf8ByteLength("abc"), 3);
  assert.equal(utf8ByteLength("\u{1f525}"), 4);
  assert.equal(terminalTextWithinPasteBudget("x".repeat(MAX_TERMINAL_PASTE_BYTES)), true);
  assert.equal(terminalTextWithinPasteBudget("x".repeat(MAX_TERMINAL_PASTE_BYTES + 1)), false);
});
