import test from "node:test";
import assert from "node:assert/strict";

import {
  sendSheetFailureStatus,
  sendSheetSubmitPlan,
  sendSheetSuccessStatus,
} from "./send_sheet.js";

test("sendSheetSubmitPlan ignores read-only and blank sends without trimming payloads", () => {
  assert.deepEqual(sendSheetSubmitPlan({
    readOnly: true,
    text: "send it",
    selectedSessionId: "sess_0",
  }), { type: "ignore" });
  assert.deepEqual(sendSheetSubmitPlan({
    text: " \n\t ",
    selectedSessionId: "sess_0",
  }), { type: "ignore" });
  assert.deepEqual(sendSheetSubmitPlan({
    text: "  send it\n",
    selectedSessionId: "sess_0",
  }), {
    type: "line",
    text: "  send it\n",
    sessionId: "sess_0",
    label: "sess_0",
  });
});

test("sendSheetSubmitPlan preserves group targets and paste-vs-line routing", () => {
  assert.deepEqual(sendSheetSubmitPlan({
    text: "continue",
    sendTarget: { type: "group", sessionIds: ["a", "b"], label: "Batch" },
    sendMode: "paste",
    selectedSessionId: "ignored",
  }), {
    type: "group",
    text: "continue",
    sessionIds: ["a", "b"],
  });
  assert.deepEqual(sendSheetSubmitPlan({
    text: "draft",
    sendTarget: { type: "session", sessionId: "target", label: "Agent Target" },
    sendMode: "paste",
    selectedSessionId: "fallback",
  }), {
    type: "paste",
    text: "draft",
    sessionId: "target",
    label: "Agent Target",
  });
  assert.deepEqual(sendSheetSubmitPlan({
    text: "line",
    sendMode: "unknown",
    selectedSessionId: "fallback",
  }), {
    type: "line",
    text: "line",
    sessionId: "fallback",
    label: "fallback",
  });
});

test("sendSheetSuccessStatus preserves group, paste, and line status copy", () => {
  assert.deepEqual(sendSheetSuccessStatus(
    { type: "group", sessionIds: ["a", "b"] },
    { total: 2, skipped: 1, delivered: 1 },
  ), {
    label: "Sent batch line to 1 of 2 agents.",
    muted: false,
    ttlMs: 3200,
  });
  assert.deepEqual(sendSheetSuccessStatus(
    { type: "group", sessionIds: ["a", "b"] },
    { delivered: 0 },
  ), {
    label: "Sent batch line to 0 agents.",
    muted: true,
    ttlMs: 2400,
  });
  assert.deepEqual(sendSheetSuccessStatus({ type: "paste", label: "sess_0" }), {
    label: "Pasted text to sess_0.",
    muted: false,
    ttlMs: 2200,
  });
  assert.deepEqual(sendSheetSuccessStatus({ type: "line", label: "sess_0" }), {
    label: "Sent line to sess_0.",
    muted: false,
    ttlMs: 2200,
  });
});

test("sendSheetFailureStatus preserves failure status copy", () => {
  assert.deepEqual(sendSheetFailureStatus(new Error("input delivery failed")), {
    label: "Send failed: input delivery failed",
    muted: true,
    ttlMs: 3200,
  });
});
