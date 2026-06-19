import test from "node:test";
import assert from "node:assert/strict";

import {
  surfaceActionDispatchContextPlan,
  surfaceActionDispatchPlan,
  surfaceActionExecutionContextPlan,
  surfaceActionExecutionPlan,
  surfaceActionFocusTerminalExecutionPlan,
  surfaceActionTrogdorReaderExecutionPlan,
} from "./surface_action_plans.js";

test("surface action dispatch guards null, disabled, and direct zones", () => {
  assert.deepEqual(surfaceActionDispatchPlan(null), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ disabled: true, actionId: "refresh" }), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ type: "session", sessionId: "agent-1" }), {
    type: "select_session",
    sessionId: "agent-1",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ type: "trogdor_reader", actionId: "open_send" }), {
    type: "ignore",
  });
  assert.deepEqual(
    surfaceActionDispatchPlan({ type: "action", actionId: "fleet_filter", kind: "target", key: "skillbox" }),
    {
      type: "set_fleet_filter",
      filter: { kind: "target", key: "skillbox" },
    },
  );
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "toggle_session_grouping" }), {
    type: "toggle_session_grouping",
  });
});

test("surface action dispatch validates fleet filter payloads", () => {
  assert.deepEqual(
    surfaceActionDispatchPlan({ type: "action", actionId: "fleet_filter", kind: " TARGET ", key: " skillbox " }),
    {
      type: "set_fleet_filter",
      filter: { kind: "target", key: "skillbox" },
    },
  );
  assert.deepEqual(
    surfaceActionDispatchPlan({ type: "action", actionId: "fleet_filter", kind: "", key: "" }),
    {
      type: "set_fleet_filter",
      filter: { kind: "", key: "" },
    },
  );
  assert.deepEqual(
    surfaceActionDispatchPlan({ type: "action", actionId: "fleet_filter", kind: "target", key: "" }),
    { type: "ignore" },
  );
  assert.deepEqual(
    surfaceActionDispatchPlan({ type: "action", actionId: "fleet_filter", kind: "unknown", key: "skillbox" }),
    { type: "ignore" },
  );
});

test("surface action dispatch preserves gated current-session send behavior", () => {
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_send" }, { readOnly: true }), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_send" }, { readOnly: false }), { type: "ignore" });
  assert.deepEqual(
    surfaceActionDispatchPlan(
      { actionId: "open_send" },
      { readOnly: false, currentSession: { session_id: "agent-2", tmux_name: "" } },
    ),
    {
      type: "open_send_sheet_for_current_session",
      payload: { type: "session", sessionId: "agent-2", label: "agent-2" },
    },
  );
});

test("surface action context plans preserve data collection boundaries", () => {
  assert.deepEqual(surfaceActionDispatchContextPlan(null), {
    includeReadOnly: false,
    includeCurrentSession: false,
  });
  assert.deepEqual(surfaceActionDispatchContextPlan({ actionId: "open_send" }), {
    includeReadOnly: true,
    includeCurrentSession: true,
  });
  assert.deepEqual(surfaceActionDispatchContextPlan({ actionId: "open_create" }), {
    includeReadOnly: true,
    includeCurrentSession: false,
  });
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "open_send_sheet_for_zone" }), {
    includeZonePayload: true,
  });
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "refresh" }), {
    includeZonePayload: false,
  });
});

test("surface action execution plans preserve payload passthroughs", () => {
  assert.deepEqual(
    surfaceActionExecutionPlan(
      { type: "select_then_open_mermaid_for_zone" },
      { zonePayload: { sessionId: "agent-3" } },
    ),
    { type: "select_then_open_mermaid", sessionId: "agent-3" },
  );
  assert.deepEqual(surfaceActionExecutionPlan({ type: "open_sheet", sheetId: "search" }), {
    type: "open_sheet",
    sheetId: "search",
  });
  assert.deepEqual(surfaceActionExecutionPlan({ type: "copy_selection" }), { type: "copy_selection" });
  assert.deepEqual(
    surfaceActionExecutionPlan({ type: "set_fleet_filter", filter: { kind: "repo", key: "/tmp/repo" } }),
    { type: "set_fleet_filter", filter: { kind: "repo", key: "/tmp/repo" } },
  );
  assert.deepEqual(
    surfaceActionExecutionPlan({ type: "set_fleet_filter", filter: { kind: "", key: "" } }),
    { type: "set_fleet_filter", filter: { kind: "", key: "" } },
  );
  assert.deepEqual(
    surfaceActionExecutionPlan({ type: "set_fleet_filter", filter: { kind: "target", key: "" } }),
    { type: "ignore" },
  );
  assert.deepEqual(surfaceActionExecutionPlan({ type: "toggle_session_grouping" }), {
    type: "toggle_session_grouping",
  });
  assert.deepEqual(surfaceActionExecutionPlan({ type: "focus_terminal" }), { type: "ignore" });
});

test("surface Trogdor reader and focus execution plans preserve object shapes", () => {
  assert.deepEqual(
    surfaceActionTrogdorReaderExecutionPlan(
      { type: "trogdor_read_toggle" },
      { toggle: { session: { session_id: "agent-4" }, readAgain: true, reading: false, restartClock: true } },
    ),
    {
      type: "apply_trogdor_reader",
      session: { session_id: "agent-4" },
      readAgain: true,
      statePatch: { trogdorReading: false },
      restartClock: true,
      resetAfterWpmChange: false,
      syncReaderTimer: true,
    },
  );
  assert.deepEqual(surfaceActionTrogdorReaderExecutionPlan({ type: "trogdor_wpm" }, { nextWpm: 320 }), {
    type: "apply_trogdor_reader",
    session: null,
    readAgain: false,
    statePatch: { trogdorWpm: 320 },
    restartClock: false,
    resetAfterWpmChange: true,
    syncReaderTimer: false,
  });
  assert.deepEqual(surfaceActionFocusTerminalExecutionPlan({ message: "Ready", error: false, timeoutMs: 900 }), {
    type: "focus_terminal",
    atlasTransitionAction: "close",
    focusOptions: { preventScroll: true },
    statusMessage: "Ready",
    statusError: false,
    statusTimeoutMs: 900,
  });
});
