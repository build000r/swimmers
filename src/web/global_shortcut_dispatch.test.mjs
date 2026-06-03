import test from "node:test";
import assert from "node:assert/strict";

import { runGlobalShortcutAction } from "./global_shortcut_dispatch.js";

function buildRuntime(overrides = {}) {
  const calls = [];
  const runtime = {
    state: {
      terminalZoom: 1,
      selectMode: false,
      ...overrides.state,
    },
    terminalZoomStep: 0.1,
    openCommandPalette: () => calls.push(["openCommandPalette"]),
    setTerminalZoom: (value, options) => calls.push(["setTerminalZoom", value, options]),
    closeSheets: () => calls.push(["closeSheets"]),
    trogdorAtlasTransitionState: (mode) => ({ trogdorAtlasOpen: mode !== "close" }),
    renderHudSurface: () => calls.push(["renderHudSurface"]),
    setSelectMode: (enabled) => calls.push(["setSelectMode", enabled]),
    openSheet: (sheetId) => calls.push(["openSheet", sheetId]),
    openThoughtConfigSheet: () => calls.push(["openThoughtConfigSheet"]),
    openNativeSheet: () => calls.push(["openNativeSheet"]),
    openMermaidSheet: () => calls.push(["openMermaidSheet"]),
    toggleFollowPublished: () => calls.push(["toggleFollowPublished"]),
    copyTerminalSelection: () => calls.push(["copyTerminalSelection"]),
    copyHoveredLink: () => calls.push(["copyHoveredLink"]),
    refreshSessions: () => calls.push(["refreshSessions"]),
  };
  return { calls, runtime };
}

test("runGlobalShortcutAction dispatches palette, zoom, sheet, and noop actions", () => {
  const { calls, runtime } = buildRuntime();
  runGlobalShortcutAction({ type: "open_palette" }, runtime);
  runGlobalShortcutAction({ type: "zoom_in" }, runtime);
  runGlobalShortcutAction({ type: "zoom_out" }, runtime);
  runGlobalShortcutAction({ type: "zoom_reset" }, runtime);
  runGlobalShortcutAction({ type: "open_sheet", sheetId: "search" }, runtime);
  runGlobalShortcutAction({ type: "handled" }, runtime);

  assert.deepEqual(calls, [
    ["openCommandPalette"],
    ["setTerminalZoom", 1.1, { announce: true }],
    ["setTerminalZoom", 0.9, { announce: true }],
    ["setTerminalZoom", 1, { announce: true }],
    ["openSheet", "search"],
  ]);
});

test("runGlobalShortcutAction dispatches escape and ctrl-shift side effects", () => {
  const { calls, runtime } = buildRuntime({ state: { selectMode: true, trogdorAtlasOpen: true } });
  runGlobalShortcutAction({ type: "close_sheets" }, runtime);
  runGlobalShortcutAction({ type: "close_trogdor_atlas" }, runtime);
  runGlobalShortcutAction({ type: "exit_select_mode" }, runtime);
  runGlobalShortcutAction({ type: "open_thought_config" }, runtime);
  runGlobalShortcutAction({ type: "open_native" }, runtime);
  runGlobalShortcutAction({ type: "open_mermaid" }, runtime);
  runGlobalShortcutAction({ type: "toggle_follow" }, runtime);
  runGlobalShortcutAction({ type: "toggle_select" }, runtime);
  runGlobalShortcutAction({ type: "copy_selection" }, runtime);
  runGlobalShortcutAction({ type: "copy_hovered_link" }, runtime);
  runGlobalShortcutAction({ type: "refresh_sessions" }, runtime);

  assert.deepEqual(runtime.state.trogdorAtlasOpen, false);
  assert.deepEqual(calls, [
    ["closeSheets"],
    ["renderHudSurface"],
    ["setSelectMode", false],
    ["openThoughtConfigSheet"],
    ["openNativeSheet"],
    ["openMermaidSheet"],
    ["toggleFollowPublished"],
    ["setSelectMode", false],
    ["copyTerminalSelection"],
    ["copyHoveredLink"],
    ["refreshSessions"],
  ]);
});
