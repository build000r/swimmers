import test from "node:test";
import assert from "node:assert/strict";

import {
  eventCell,
  eventClientPoint,
  globalShortcutPlan,
  shouldIgnoreSyntheticClick,
} from "./input_support.js";

test("eventClientPoint uses direct pointer coordinates when present", () => {
  assert.deepEqual(eventClientPoint({ clientX: 120, clientY: 48 }), {
    x: 120,
    y: 48,
  });
});

test("eventClientPoint falls back to changedTouches for mobile taps", () => {
  assert.deepEqual(
    eventClientPoint({
      changedTouches: [{ clientX: 33, clientY: 77 }],
    }),
    {
      x: 33,
      y: 77,
    },
  );
});

test("eventCell maps touch coordinates into the rendered grid", () => {
  const cell = eventCell(
    {
      changedTouches: [{ clientX: 75, clientY: 90 }],
    },
    { left: 0, top: 0, width: 160, height: 160 },
    16,
    16,
  );
  assert.deepEqual(cell, { x: 7, y: 9 });
});

test("shouldIgnoreSyntheticClick suppresses the follow-up click after a handled press", () => {
  assert.equal(shouldIgnoreSyntheticClick(100, 120), true);
  assert.equal(shouldIgnoreSyntheticClick(140, 120), false);
});

test("globalShortcutPlan preserves modifier precedence and escape ordering", () => {
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, code: "KeyK" }), { type: "open_palette" });
  assert.deepEqual(globalShortcutPlan({ metaKey: true, shiftKey: true, code: "Equal" }), { type: "zoom_in" });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, code: "NumpadSubtract" }), { type: "zoom_out" });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, code: "Digit0" }), { type: "zoom_reset" });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, altKey: true, code: "KeyK" }), { type: "unhandled" });
  assert.deepEqual(globalShortcutPlan(
    { key: "Escape" },
    { activeSheet: "send", trogdorAtlasOpen: true, selectMode: true },
  ), { type: "close_sheets" });
  assert.deepEqual(globalShortcutPlan({ key: "Escape" }, { trogdorAtlasOpen: true, selectMode: true }), {
    type: "close_trogdor_atlas",
  });
  assert.deepEqual(globalShortcutPlan({ key: "Escape" }, { selectMode: true }), { type: "exit_select_mode" });
  assert.deepEqual(globalShortcutPlan({ key: "Escape" }), { type: "unhandled" });
});

test("globalShortcutPlan preserves ctrl-shift commands and gated handled no-ops", () => {
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyF" }), {
    type: "open_sheet",
    sheetId: "search",
  });
  assert.deepEqual(globalShortcutPlan(
    { ctrlKey: true, shiftKey: true, code: "KeyS" },
    { readOnly: true, hasCurrentSession: true },
  ), { type: "handled" });
  assert.deepEqual(globalShortcutPlan(
    { ctrlKey: true, shiftKey: true, code: "KeyS" },
    { readOnly: false, hasCurrentSession: false },
  ), { type: "handled" });
  assert.deepEqual(globalShortcutPlan(
    { ctrlKey: true, shiftKey: true, code: "KeyS" },
    { readOnly: false, hasCurrentSession: true },
  ), { type: "open_sheet", sheetId: "send" });
  assert.deepEqual(globalShortcutPlan(
    { ctrlKey: true, shiftKey: true, code: "KeyN" },
    { readOnly: true },
  ), { type: "handled" });
  assert.deepEqual(globalShortcutPlan(
    { ctrlKey: true, shiftKey: true, code: "KeyN" },
    { readOnly: false },
  ), { type: "open_sheet", sheetId: "create" });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyA" }), {
    type: "open_sheet",
    sheetId: "auth",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyT" }), {
    type: "open_thought_config",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyO" }), {
    type: "open_native",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyM" }), {
    type: "open_mermaid",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyP" }), {
    type: "toggle_follow",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyV" }), {
    type: "toggle_select",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyC" }), {
    type: "copy_selection",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyR" }), {
    type: "refresh_sessions",
  });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, code: "KeyL" }), { type: "handled" });
  assert.deepEqual(globalShortcutPlan(
    { ctrlKey: true, shiftKey: true, code: "KeyL" },
    { hoveredLinkUrl: "http://127.0.0.1:3210" },
  ), { type: "copy_hovered_link" });
  assert.deepEqual(globalShortcutPlan({ metaKey: true, shiftKey: true, code: "KeyF" }), { type: "unhandled" });
  assert.deepEqual(globalShortcutPlan({ ctrlKey: true, shiftKey: true, altKey: true, code: "KeyF" }), {
    type: "unhandled",
  });
});
