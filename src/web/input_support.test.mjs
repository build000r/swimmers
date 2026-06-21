import test from "node:test";
import assert from "node:assert/strict";

import {
  appEventListenerBindingPlan,
  authTokenButtonPlan, controlEventSessionPatchPlan, eventCell,
  eventClientPoint,
  globalShortcutPlan,
  initialStateBootPlan,
  inputAckActionPlan,
  lifecycleDeletedSessionPatchPlan,
  mobileKeyboardInputExecutorPlan,
  mobileKeyboardInputPlan,
  sheetActionAvailabilityPlan,
  mobileKeyboardKeydownPlan,
  mobileKeyboardKeyPlan,
  shouldIgnoreSyntheticClick,
  surfaceActionDispatchContextPlan,
  surfaceActionDispatchPlan,
  surfaceActionExecutionContextPlan,
  surfaceActionExecutionPlan,
  surfaceActionFocusTerminalExecutionPlan,
  surfaceActionTrogdorReaderExecutionPlan,
  terminalComposerControlAction,
  terminalInlineInputKeydownPlan,
  terminalInputDockPlan,
  terminalKeyStripClickExecutorPlan,
  terminalKeyStripClickPlan,
  terminalLiveFrameFallbackPlan,
  terminalPaintProbeSchedulePlan,
  terminalPaintVerificationPlan,
  terminalPendingByteBufferPlan,
  terminalPresentationPlan,
  terminalResizeGeometryPlan,
  terminalSurfaceInitErrorPlan,
  terminalSurfacePostInitPlan,
  terminalSurfaceRendererPlan,
  terminalSurfaceSessionPlan,
  terminalStageCaptureBindings,
  terminalAuxiliaryControlsPlan,
  normalizeTerminalZoomValue,
  terminalZoomControlsPlan,
  terminalZoomLoadValue,
  terminalZoomPercentLabel,
  terminalZoomPersistencePlan,
  terminalToolsAvailabilityPlan,
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

test("authTokenButtonPlan preserves save, clear, and ignored actions", () => {
  assert.deepEqual(authTokenButtonPlan("save", " token\n"), {
    type: "persist",
    token: " token\n",
    resetReadOnly: false,
  });
  assert.deepEqual(authTokenButtonPlan("clear", "ignored"), {
    type: "persist",
    token: "",
    resetReadOnly: true,
  });
  assert.deepEqual(authTokenButtonPlan("unknown", "secret"), { type: "ignore" });
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

test("surfaceActionDispatchPlan preserves ignored and direct zone routes", () => {
  assert.deepEqual(surfaceActionDispatchPlan(null), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ disabled: true, actionId: "refresh" }), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ type: "session", sessionId: "agent-1" }), {
    type: "select_session",
    sessionId: "agent-1",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ type: "trogdor_agent", sessionId: "agent-2" }), {
    type: "open_trogdor_agent_terminal",
    sessionId: "agent-2",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ type: "trogdor_reader", actionId: "open_send" }), { type: "ignore" });
});

test("surfaceActionDispatchPlan preserves Trogdor reader and atlas action routes", () => {
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_read_toggle" }), {
    type: "trogdor_read_toggle",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_wpm_down" }), {
    type: "trogdor_wpm",
    actionId: "trogdor_wpm_down",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_wpm_up" }), {
    type: "trogdor_wpm",
    actionId: "trogdor_wpm_up",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "toggle_trogdor_atlas" }), {
    type: "toggle_trogdor_atlas",
  });
});

test("surfaceActionDispatchPlan preserves Trogdor surface action routes", () => {
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_send" }), {
    type: "open_send_sheet_for_zone",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_group_send" }), {
    type: "open_send_sheet_for_zone",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_launch" }), {
    type: "open_create_sheet_for_zone_cwd",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_mermaid" }), {
    type: "select_then_open_mermaid_for_zone",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "trogdor_commit" }), {
    type: "select_then_launch_commit_for_zone",
  });
});

test("surfaceActionDispatchPlan preserves sheet, utility, and refresh routes", () => {
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_search" }), {
    type: "open_sheet",
    sheetId: "search",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_auth" }), {
    type: "open_sheet",
    sheetId: "auth",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_config" }), { type: "open_thought_config" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_native" }), { type: "open_native" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_mermaid" }), { type: "open_mermaid" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "launch_commit" }), { type: "launch_commit" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "toggle_follow" }), { type: "toggle_follow" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "toggle_select" }), { type: "toggle_select" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "copy_selection" }), { type: "copy_selection" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "focus_terminal" }), { type: "focus_terminal" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "refresh" }), { type: "refresh" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "unknown" }), { type: "ignore" });
});

test("surfaceActionDispatchPlan preserves open_send and open_create gates", () => {
  const session = { session_id: "agent-1", tmux_name: "codex-main" };
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_send" }, {
    readOnly: false,
    currentSession: session,
  }), {
    type: "open_send_sheet_for_current_session",
    payload: { type: "session", sessionId: "agent-1", label: "codex-main" },
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_send" }, {
    readOnly: false,
    currentSession: { session_id: "agent-2", tmux_name: "" },
  }), {
    type: "open_send_sheet_for_current_session",
    payload: { type: "session", sessionId: "agent-2", label: "agent-2" },
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_send" }, {
    readOnly: true,
    currentSession: session,
  }), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_send" }, {
    readOnly: false,
    currentSession: null,
  }), { type: "ignore" });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_create" }, { readOnly: false }), {
    type: "open_sheet",
    sheetId: "create",
  });
  assert.deepEqual(surfaceActionDispatchPlan({ actionId: "open_create" }, { readOnly: true }), { type: "ignore" });
});

test("surfaceActionDispatchContextPlan preserves direct-zone and sheet context gates", () => {
  const emptyContext = { includeReadOnly: false, includeCurrentSession: false };
  assert.deepEqual(surfaceActionDispatchContextPlan(null), emptyContext);
  assert.deepEqual(surfaceActionDispatchContextPlan({ disabled: true, actionId: "open_send" }), emptyContext);
  assert.deepEqual(surfaceActionDispatchContextPlan({ type: "session", actionId: "open_send" }), emptyContext);
  assert.deepEqual(surfaceActionDispatchContextPlan({ type: "trogdor_agent", actionId: "open_create" }), emptyContext);
  assert.deepEqual(surfaceActionDispatchContextPlan({ type: "trogdor_reader", actionId: "open_send" }), emptyContext);
  assert.deepEqual(surfaceActionDispatchContextPlan({ actionId: "open_send" }), {
    includeReadOnly: true,
    includeCurrentSession: true,
  });
  assert.deepEqual(surfaceActionDispatchContextPlan({ actionId: "open_create" }), {
    includeReadOnly: true,
    includeCurrentSession: false,
  });
  assert.deepEqual(surfaceActionDispatchContextPlan({ actionId: "refresh" }), emptyContext);
});

test("surfaceActionTrogdorReaderExecutionPlan preserves toggle side-effect decisions", () => {
  const session = { session_id: "agent-1" };
  assert.deepEqual(surfaceActionTrogdorReaderExecutionPlan(
    { type: "trogdor_read_toggle" },
    { toggle: { session, reading: null, readAgain: true, restartClock: true } },
  ), {
    type: "apply_trogdor_reader",
    session,
    readAgain: true,
    statePatch: {},
    restartClock: true,
    resetAfterWpmChange: false,
    syncReaderTimer: true,
  });
  assert.deepEqual(surfaceActionTrogdorReaderExecutionPlan(
    { type: "trogdor_read_toggle" },
    { toggle: { session: null, reading: false, readAgain: false, restartClock: false } },
  ), {
    type: "apply_trogdor_reader",
    session: null,
    readAgain: false,
    statePatch: { trogdorReading: false },
    restartClock: false,
    resetAfterWpmChange: false,
    syncReaderTimer: true,
  });
});

test("surfaceActionTrogdorReaderExecutionPlan preserves WPM and ignore decisions", () => {
  assert.deepEqual(surfaceActionTrogdorReaderExecutionPlan(
    { type: "trogdor_wpm", actionId: "trogdor_wpm_up" },
    { nextWpm: 225 },
  ), {
    type: "apply_trogdor_reader",
    session: null,
    readAgain: false,
    statePatch: { trogdorWpm: 225 },
    restartClock: false,
    resetAfterWpmChange: true,
    syncReaderTimer: false,
  });
  assert.deepEqual(surfaceActionTrogdorReaderExecutionPlan({ type: "refresh" }), { type: "ignore" });
});

test("surfaceActionExecutionContextPlan requests zone payloads only for zone-derived actions", () => {
  const payloadContext = { includeZonePayload: true };
  const emptyContext = { includeZonePayload: false };
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "open_send_sheet_for_zone" }), payloadContext);
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "open_create_sheet_for_zone_cwd" }), payloadContext);
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "select_then_open_mermaid_for_zone" }), payloadContext);
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "select_then_launch_commit_for_zone" }), payloadContext);
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "open_sheet" }), emptyContext);
  assert.deepEqual(surfaceActionExecutionContextPlan({ type: "focus_terminal" }), emptyContext);
});

test("surfaceActionExecutionPlan preserves zone payload execution decisions", () => {
  assert.deepEqual(
    surfaceActionExecutionPlan(
      { type: "open_send_sheet_for_zone" },
      { zonePayload: { type: "session", sessionId: "agent-1", label: "codex-main" } },
    ),
    { type: "open_send_sheet", payload: { type: "session", sessionId: "agent-1", label: "codex-main" } },
  );
  assert.deepEqual(
    surfaceActionExecutionPlan({ type: "open_create_sheet_for_zone_cwd" }, { zonePayload: { cwd: "/repo" } }),
    { type: "open_create_sheet_for_cwd", cwd: "/repo", launchTarget: "" },
  );
  assert.deepEqual(
    surfaceActionExecutionPlan(
      { type: "open_create_sheet_for_zone_cwd" },
      { zonePayload: { cwd: "/workspace/swimmers", launchTarget: "devbox" } },
    ),
    { type: "open_create_sheet_for_cwd", cwd: "/workspace/swimmers", launchTarget: "devbox" },
  );
  assert.deepEqual(
    surfaceActionExecutionPlan({ type: "select_then_open_mermaid_for_zone" }, { zonePayload: { sessionId: "agent-2" } }),
    { type: "select_then_open_mermaid", sessionId: "agent-2" },
  );
  assert.deepEqual(
    surfaceActionExecutionPlan({ type: "select_then_launch_commit_for_zone" }, { zonePayload: { sessionId: "agent-3" } }),
    { type: "select_then_launch_commit", sessionId: "agent-3" },
  );
});

test("surfaceActionExecutionPlan preserves simple sheet, utility, and refresh decisions", () => {
  const payload = { type: "session", sessionId: "agent-1", label: "agent-1" };
  assert.deepEqual(surfaceActionExecutionPlan({ type: "open_sheet", sheetId: "search" }), {
    type: "open_sheet",
    sheetId: "search",
  });
  assert.deepEqual(surfaceActionExecutionPlan({ type: "open_send_sheet_for_current_session", payload }), {
    type: "open_send_sheet",
    payload,
  });
  for (const type of [
    "open_thought_config",
    "open_native",
    "open_mermaid",
    "launch_commit",
    "toggle_follow",
    "toggle_select",
    "copy_selection",
    "refresh",
  ]) {
    assert.deepEqual(surfaceActionExecutionPlan({ type }), { type });
  }
  assert.deepEqual(surfaceActionExecutionPlan({ type: "focus_terminal" }), { type: "ignore" });
});

test("surfaceActionFocusTerminalExecutionPlan preserves focus side-effect data", () => {
  assert.deepEqual(surfaceActionFocusTerminalExecutionPlan({
    message: "Terminal focused.",
    error: false,
    timeoutMs: 2200,
  }), {
    type: "focus_terminal",
    atlasTransitionAction: "close",
    focusOptions: { preventScroll: true },
    statusMessage: "Terminal focused.",
    statusError: false,
    statusTimeoutMs: 2200,
  });
});

test("mobileKeyboardKeyPlan preserves special-key forwarding and no-op gates", () => {
  assert.deepEqual(mobileKeyboardKeyPlan({ key: "ArrowUp" }, { readOnly: true, hasCurrentSession: true }), {
    type: "ignore",
  });
  assert.deepEqual(mobileKeyboardKeyPlan({ key: "ArrowUp" }, { readOnly: false, hasCurrentSession: false }), {
    type: "ignore",
  });
  assert.deepEqual(mobileKeyboardKeyPlan({ key: "a" }, { hasCurrentSession: true }), { type: "ignore" });
  assert.deepEqual(mobileKeyboardKeyPlan({ key: "Escape" }, { hasCurrentSession: true }), {
    type: "close_mobile_keyboard",
  });

  for (const key of [
    "Backspace",
    "Delete",
    "Enter",
    "Tab",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
  ]) {
    assert.deepEqual(mobileKeyboardKeyPlan({ key }, { hasCurrentSession: true }), { type: "forward_key" });
  }
});

test("mobileKeyboardKeydownPlan preserves shortcut, ignore, close, and forward decisions", () => {
  assert.deepEqual(mobileKeyboardKeydownPlan({
    globalShortcutHandled: true,
    keyPlan: { type: "forward_key" },
    beginsResponse: true,
  }), {
    type: "prevent_default",
    handled: true,
    preventDefault: true,
    closeKeyboard: false,
    focusTerminal: false,
    markResponse: false,
    forwardKey: false,
  });
  assert.deepEqual(mobileKeyboardKeydownPlan({
    globalShortcutHandled: false,
    keyPlan: { type: "ignore" },
    beginsResponse: true,
  }), {
    type: "ignore",
    handled: false,
    preventDefault: false,
    closeKeyboard: false,
    focusTerminal: false,
    markResponse: false,
    forwardKey: false,
  });
  assert.deepEqual(mobileKeyboardKeydownPlan({
    globalShortcutHandled: false,
    keyPlan: { type: "close_mobile_keyboard" },
    beginsResponse: true,
  }), {
    type: "close_mobile_keyboard",
    handled: true,
    preventDefault: true,
    closeKeyboard: true,
    focusTerminal: true,
    markResponse: false,
    forwardKey: false,
  });
  assert.deepEqual(mobileKeyboardKeydownPlan({
    globalShortcutHandled: false,
    keyPlan: { type: "forward_key" },
    beginsResponse: false,
  }), {
    type: "forward_key",
    handled: true,
    preventDefault: true,
    closeKeyboard: false,
    focusTerminal: false,
    markResponse: false,
    forwardKey: true,
  });
  assert.deepEqual(mobileKeyboardKeydownPlan({
    globalShortcutHandled: false,
    keyPlan: { type: "forward_key" },
    beginsResponse: true,
  }), {
    type: "forward_key",
    handled: true,
    preventDefault: true,
    closeKeyboard: false,
    focusTerminal: false,
    markResponse: true,
    forwardKey: true,
  });
  assert.deepEqual(mobileKeyboardKeydownPlan({ globalShortcutHandled: false }), {
    type: "ignore",
    handled: false,
    preventDefault: false,
    closeKeyboard: false,
    focusTerminal: false,
    markResponse: false,
    forwardKey: false,
  });
});

test("mobileKeyboardInputPlan preserves clear, control, and inserted text decisions", () => {
  assert.deepEqual(mobileKeyboardInputPlan(
    { inputType: "insertText", data: "x" },
    { readOnly: true, hasCurrentSession: true, proxyValue: "ignored" },
  ), { type: "clear" });
  assert.deepEqual(mobileKeyboardInputPlan(
    { inputType: "insertText", data: "x" },
    { readOnly: false, hasCurrentSession: false, proxyValue: "ignored" },
  ), { type: "clear" });
  assert.deepEqual(mobileKeyboardInputPlan(
    { inputType: "deleteContentBackward" },
    { hasCurrentSession: true },
  ), {
    type: "forward_event",
    event: {
      kind: "key",
      phase: "down",
      key: "Backspace",
      code: "Backspace",
      mods: 0,
      repeat: false,
    },
  });
  assert.deepEqual(mobileKeyboardInputPlan(
    { inputType: "insertLineBreak" },
    { hasCurrentSession: true },
  ), {
    type: "forward_event",
    event: {
      kind: "key",
      phase: "down",
      key: "Enter",
      code: "Enter",
      mods: 0,
      repeat: false,
    },
  });
  assert.deepEqual(mobileKeyboardInputPlan(
    { inputType: "insertText", data: "typed" },
    { hasCurrentSession: true, proxyValue: "fallback" },
  ), { type: "send_text", text: "typed" });
  assert.deepEqual(mobileKeyboardInputPlan(
    { inputType: "insertText", data: null },
    { hasCurrentSession: true, proxyValue: "fallback" },
  ), { type: "send_text", text: "fallback" });
});

test("mobileKeyboardInputExecutorPlan preserves clear, forward, send, and unknown decisions", () => {
  const event = {
    kind: "key",
    phase: "down",
    key: "Enter",
    code: "Enter",
    mods: 0,
    repeat: false,
  };
  const ignored = { type: "ignore", handled: false, forwardEvent: null, sendText: false, text: "" };

  assert.deepEqual(mobileKeyboardInputExecutorPlan({ type: "clear" }), ignored);
  assert.deepEqual(mobileKeyboardInputExecutorPlan({ type: "forward_event", event }), {
    type: "forward_event",
    handled: true,
    forwardEvent: event,
    sendText: false,
    text: "",
  });
  assert.deepEqual(mobileKeyboardInputExecutorPlan({ type: "send_text", text: "" }), {
    type: "send_text",
    handled: true,
    forwardEvent: null,
    sendText: true,
    text: "",
  });
  assert.deepEqual(mobileKeyboardInputExecutorPlan({ type: "unknown", text: "ignored" }), ignored);
});

test("terminalComposerControlAction preserves modifier, selection, and empty-input gates", () => {
  assert.equal(terminalComposerControlAction(null), "");
  assert.equal(terminalComposerControlAction({ key: "ArrowUp", metaKey: true }), "");
  assert.equal(terminalComposerControlAction({ key: "ArrowUp", altKey: true }), "");
  assert.equal(terminalComposerControlAction({ key: "c", ctrlKey: true }, { hasSelection: true }), "");
  assert.equal(terminalComposerControlAction({ key: "C", ctrlKey: true }, { hasSelection: false }), "ctrl-c");
  assert.equal(terminalComposerControlAction({ key: "ArrowUp" }, { inputValue: "edit this text" }), "");

  assert.equal(terminalComposerControlAction({ key: "Escape" }), "escape");
  assert.equal(terminalComposerControlAction({ key: "Tab" }), "tab");
  assert.equal(terminalComposerControlAction({ key: "ArrowUp" }), "arrow-up");
  assert.equal(terminalComposerControlAction({ key: "ArrowDown" }), "arrow-down");
  assert.equal(terminalComposerControlAction({ key: "ArrowLeft" }), "arrow-left");
  assert.equal(terminalComposerControlAction({ key: "ArrowRight" }), "arrow-right");
  assert.equal(terminalComposerControlAction({ key: "Home" }), "home");
  assert.equal(terminalComposerControlAction({ key: "End" }), "end");
  assert.equal(terminalComposerControlAction({ key: "PageUp" }), "page-up");
  assert.equal(terminalComposerControlAction({ key: "PageDown" }), "page-down");
  assert.equal(terminalComposerControlAction({ key: "F1" }), "");
});

test("terminalInlineInputKeydownPlan preserves submit, send, and ignored key handling", () => {
  const ignored = {
    type: "ignore",
    handled: false,
    preventDefault: false,
    stopPropagation: true,
    submit: false,
    sendKey: false,
    actionId: "",
  };

  assert.deepEqual(terminalInlineInputKeydownPlan({ key: "Enter", shiftKey: false }), {
    type: "submit",
    handled: true,
    preventDefault: true,
    stopPropagation: true,
    submit: true,
    sendKey: false,
    actionId: "",
  });
  assert.deepEqual(terminalInlineInputKeydownPlan({ key: "Enter", shiftKey: true }), ignored);
  assert.deepEqual(terminalInlineInputKeydownPlan({ key: "ArrowUp" }, "arrow-up"), {
    type: "send_key",
    handled: true,
    preventDefault: true,
    stopPropagation: true,
    submit: false,
    sendKey: true,
    actionId: "arrow-up",
  });
  assert.deepEqual(terminalInlineInputKeydownPlan({ key: "a" }, ""), ignored);
});

test("terminalKeyStripClickPlan preserves target, disabled, and action dispatch gates", () => {
  const targetFor = (button) => ({
    closest(selector) {
      return selector === "button[data-terminal-key]" ? button : null;
    },
  });
  assert.deepEqual(terminalKeyStripClickPlan("mousemove", targetFor({
    disabled: false,
    dataset: { terminalKey: "ctrl-c" },
  })), { type: "ignore" });
  assert.deepEqual(terminalKeyStripClickPlan("click", null), { type: "ignore" });
  assert.deepEqual(terminalKeyStripClickPlan("click", targetFor({
    disabled: true,
    dataset: { terminalKey: "ctrl-c" },
  })), { type: "ignore" });
  assert.deepEqual(terminalKeyStripClickPlan("click", targetFor({
    disabled: false,
    dataset: { terminalKey: "arrow-up" },
  })), { type: "send_key", actionId: "arrow-up" });
});

test("terminalKeyStripClickExecutorPlan preserves ignore, send, and unknown decisions", () => {
  const ignored = { type: "ignore", preventDefault: false, sendKey: false, actionId: "" };

  assert.deepEqual(terminalKeyStripClickExecutorPlan({ type: "ignore" }), ignored);
  assert.deepEqual(terminalKeyStripClickExecutorPlan({ type: "send_key", actionId: "ctrl-c" }), {
    type: "send_key",
    preventDefault: true,
    sendKey: true,
    actionId: "ctrl-c",
  });
  assert.deepEqual(terminalKeyStripClickExecutorPlan({ type: "unknown", actionId: "arrow-up" }), ignored);
});

test("terminalStageCaptureBindings preserves stage event labels and options", () => {
  assert.deepEqual(terminalStageCaptureBindings(), [
    { eventType: "mousedown", action: "down", options: { capture: true } },
    { eventType: "click", action: "click", options: { capture: true } },
    { eventType: "touchend", action: "touch", options: { capture: true, passive: false } },
    { eventType: "wheel", action: "wheel", options: { capture: true, passive: false } },
  ]);
});

test("appEventListenerBindingPlan preserves binding order and special listener cases", () => {
  const key = (binding) => [
    binding.target,
    binding.eventType,
    binding.handler,
    binding.optionalListener ? "optional-listener" : "",
    binding.optionalTarget ? "optional-target" : "",
    binding.options ? JSON.stringify(binding.options) : "",
  ].filter(Boolean).join(":");
  const plan = appEventListenerBindingPlan();

  assert.deepEqual(plan.beforeTerminalStageCapture.map(key), [
    "document:keydown:handleDocumentCommandPaletteShortcut:optional-listener",
    "terminalPalette:click:handleTerminalPaletteClick",
    "terminalCopyFrame:click:handleTerminalCopyFrameClick",
    "terminalLinkOpen:click:handleTerminalLinkOpenClick",
    "terminalLinkCopy:click:handleTerminalLinkCopyClick",
    "terminalZoomOut:click:handleTerminalZoomOutClick",
    "terminalZoomReset:click:handleTerminalZoomResetClick",
    "terminalZoomIn:click:handleTerminalZoomInClick",
    "terminalMobileKeyboard:click:handleTerminalMobileKeyboardClick",
    "terminalTrogdorBack:click:handleTerminalTrogdorBackClick",
    "terminalWorkbenchToggle:click:handleTerminalWorkbenchToggleClick",
    "terminalWorkbenchRefresh:click:handleTerminalWorkbenchRefreshClick",
    "terminalWorkbenchWidgets:click:handleTerminalWorkbenchWidgetsClick",
    "terminalWorkbenchWidgets:input:handleTerminalWorkbenchWidgetsLogEvent",
    "terminalWorkbenchWidgets:change:handleTerminalWorkbenchWidgetsLogEvent",
    "terminalInputDock:submit:handleTerminalInputDockSubmit",
    "terminalInlineInput:input:handleTerminalInlineInputInput",
    "terminalInlineInput:keydown:handleTerminalInlineInputKeydown",
    "terminalKeyStrip:click:handleTerminalKeyStripClick",
    "terminalInlineInput:focus:handleTerminalInlineInputFocus",
    "terminalFallback:mousedown:handleTerminalFallbackMousedown",
    "terminalFallback:click:handleTerminalFallbackClick",
    "terminalFallback:keydown:handleTerminalFallbackKeyEvent",
    "terminalFallback:paste:handleTerminalFallbackPasteEvent",
    "terminalFallback:focus:handleTerminalFallbackFocus",
    "terminalFallback:blur:handleTerminalFallbackBlur",
    "terminalFallback:scroll:handleTerminalFallbackScroll",
    "mobileKeyboardProxy:focus:handleMobileKeyboardProxyFocus",
    "mobileKeyboardProxy:blur:handleMobileKeyboardProxyBlur",
    "mobileKeyboardProxy:keydown:handleMobileKeyboardProxyKeydown",
    "mobileKeyboardProxy:input:handleMobileKeyboardProxyInput",
    "modalBackdrop:click:closeSheets",
    "modalRoot:keydown:handleModalRootKeydown",
    "paletteSearch:input:handlePaletteSearchInput",
    "paletteSearch:keydown:handleCommandPaletteEvent",
    "paletteResults:mousemove:handleCommandPaletteEvent",
    "paletteResults:click:handleCommandPaletteEvent",
    "paletteCloseButton:click:closeSheets",
    "searchForm:submit:handleSearchFormSubmit",
    "terminalSearch:input:handleTerminalSearchInput",
    "searchPrevButton:click:handleSearchPrevButtonClick",
    "searchNextButton:click:handleSearchNextButtonClick",
    "searchClearButton:click:handleSearchClearButtonClick",
    "searchCloseButton:click:closeSheets",
    "sendMode:change:handleSendModeChange",
    "thoughtConfigForm:submit:handleThoughtConfigFormSubmit",
    "thoughtConfigBackend:change:handleThoughtConfigBackendChange",
    "thoughtConfigModel:input:handleThoughtConfigOptionChange",
    "thoughtConfigEnabled:change:handleThoughtConfigOptionChange",
    "thoughtConfigTestButton:click:handleThoughtConfigTestButtonClick",
    "thoughtConfigCloseButton:click:closeSheets",
    "nativeForm:submit:handleNativeFormSubmit",
    "nativeRefreshButton:click:handleNativeRefreshButtonClick",
    "nativeOpenButton:click:handleNativeOpenButtonClick",
    "nativeCloseButton:click:closeSheets",
    "nativeApp:change:handleNativeAppChange",
    "nativeMode:change:handleNativeModeChange",
    "sendForm:submit:handleSendFormSubmit",
    "sendCloseButton:click:handleSendCloseButtonClick",
    "sendHistory:click:handleSendHistoryClick",
    "saveTokenButton:click:handleSaveTokenButtonClick",
    "clearTokenButton:click:handleClearTokenButtonClick",
    "authCloseButton:click:closeSheets",
    "createForm:submit:handleCreateFormSubmit",
    "createCloseButton:click:closeSheets",
    "createTool:change:handleCreateToolChange",
    "createLaunchTarget:change:handleCreateLaunchTargetChange",
    "createRequest:input:handleCreateRequestInput",
    "dirsSearch:input:handleDirsSearchInput",
    "createBatchVisible:click:handleCreateBatchVisibleAction",
    "createBatchClear:click:handleCreateBatchClearClick:optional-target",
    "createCwd:input:handleCreateCwdInput",
    "dirsManagedOnly:change:handleDirsManagedOnlyChange",
    "dirsPath:input:handleDirsPathInput",
    "dirsPath:keydown:handleDirsPathKeydown",
    "dirsLoadButton:click:handleDirsLoadButtonClick",
    "dirsSpawnHere:click:handleDirsSpawnHereClick",
    "dirsUpButton:click:handleDirsUpButtonClick",
    "dirsList:change:handleDirCheckboxChange",
    "dirsList:click:handleDirsListClick",
    "dirsGroups:click:handleDirsListClick:optional-target",
    "mermaidRefreshButton:click:handleMermaidRefreshButtonClick",
    "mermaidOpenButton:click:handleMermaidOpenButtonClick",
    "mermaidPlanTabs:click:handleMermaidPlanTabsClick",
    "mermaidCloseButton:click:closeSheets",
  ]);
  assert.deepEqual(plan.afterTerminalStageCapture.map(key), [
    "terminalStage:click:handleTerminalStageClick",
    "terminalStage:touchend:handleTerminalStageTouchEnd:{\"passive\":false}",
    "terminalStage:keydown:handleTerminalStageKeydown",
    "terminalStage:paste:handleTerminalStagePaste",
    "terminalStage:focus:handleTerminalStageFocus",
    "terminalStage:blur:handleTerminalStageBlur",
    "terminalStage:mousedown:handleTerminalStageMouseDown",
    "terminalStage:mouseup:handleTerminalStageMouseUp",
    "terminalStage:mousemove:handleTerminalStageMouseMove",
    "terminalStage:wheel:handleTerminalStageWheel:{\"passive\":false}",
    "terminalStage:mouseleave:handleTerminalStageMouseleave",
  ]);
});

test("terminalPendingByteBufferPlan preserves pending byte acceptance and drops", () => {
  const ignored = { type: "ignore", accept: false, dropCount: 0, finalPendingByteLength: 5, status: "" };

  assert.deepEqual(terminalPendingByteBufferPlan({ isUint8Array: false, byteLength: 5, pendingByteLength: 5 }), ignored);
  assert.deepEqual(terminalPendingByteBufferPlan({ isUint8Array: true, byteLength: 0, pendingByteLength: 5 }), ignored);
  assert.deepEqual(terminalPendingByteBufferPlan({
    isUint8Array: true,
    byteLength: 5,
    pendingByteLength: 10,
    pendingChunkByteLengths: [4, 6],
    maxPendingBytes: 20,
  }), {
    type: "buffer",
    accept: true,
    dropCount: 0,
    finalPendingByteLength: 15,
    status: "buffering terminal; renderer attaching",
  });
  assert.deepEqual(terminalPendingByteBufferPlan({
    isUint8Array: true,
    byteLength: 5,
    pendingByteLength: 18,
    pendingChunkByteLengths: [8, 10],
    maxPendingBytes: 20,
  }), {
    type: "buffer",
    accept: true,
    dropCount: 1,
    finalPendingByteLength: 15,
    status: "buffering terminal; renderer attaching",
  });
  assert.equal(terminalPendingByteBufferPlan({
    isUint8Array: true,
    byteLength: 8,
    pendingByteLength: 25,
    pendingChunkByteLengths: [8, 7, 10],
    maxPendingBytes: 20,
  }).dropCount, 2);
  assert.deepEqual(terminalPendingByteBufferPlan({
    isUint8Array: true,
    byteLength: 25,
    pendingByteLength: 0,
    pendingChunkByteLengths: [],
    maxPendingBytes: 20,
  }), {
    type: "buffer",
    accept: true,
    dropCount: 0,
    finalPendingByteLength: 25,
    status: "buffering terminal; renderer attaching",
  });
});

test("terminalPresentationPlan preserves terminal focus and canvas visibility decisions", () => {
  assert.deepEqual(terminalPresentationPlan({
    hasCurrentSession: true,
    trogdorAtlasOpen: false,
    hasTerminal: true,
    terminalFallbackActive: true,
  }), {
    terminalFocusMode: true,
    terminalStageActive: true,
    hudHidden: true,
    hudDisplay: "none",
    hudVisibility: "hidden",
    showTerminalCanvas: true,
    terminalCanvasHidden: false,
    terminalCanvasDisplay: "",
    terminalCanvasVisibility: "",
    terminalFallbackHidden: false,
  });
  assert.deepEqual(terminalPresentationPlan({
    hasCurrentSession: true,
    trogdorAtlasOpen: true,
    hasTerminal: true,
    terminalFallbackActive: true,
  }), {
    terminalFocusMode: false,
    terminalStageActive: false,
    hudHidden: false,
    hudDisplay: "",
    hudVisibility: "",
    showTerminalCanvas: true,
    terminalCanvasHidden: false,
    terminalCanvasDisplay: "",
    terminalCanvasVisibility: "",
    terminalFallbackHidden: true,
  });
  assert.equal(terminalPresentationPlan({ hasCurrentSession: false, hasTerminal: false }).showTerminalCanvas, false);
});

test("terminalPaintProbeSchedulePlan preserves paint probe gates", () => {
  const ready = {
    terminalPaintVerified: false,
    terminalFallbackActive: false,
    hasProbeTimer: false,
    hasTerminal: true,
    hasCurrentSession: true,
    terminalFrameBytesSeen: 1,
  };

  assert.deepEqual(terminalPaintProbeSchedulePlan(ready), {
    type: "schedule_probe",
    scheduleProbe: true,
    delayMs: 180,
  });

  for (const patch of [
    { terminalPaintVerified: true },
    { terminalFallbackActive: true },
    { hasProbeTimer: true },
    { hasTerminal: false },
    { hasCurrentSession: false },
    { terminalFrameBytesSeen: 0 },
  ]) {
    assert.deepEqual(terminalPaintProbeSchedulePlan({ ...ready, ...patch }), {
      type: "ignore",
      scheduleProbe: false,
      delayMs: 180,
    });
  }
});

test("terminalPaintVerificationPlan preserves verify, refresh, and fallback decisions", () => {
  const ready = {
    hasTerminal: true,
    terminalPaintVerified: false,
    terminalFallbackActive: false,
    hasCurrentSession: true,
  };

  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, hasTerminal: false }), { type: "ignore", done: true });
  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, terminalPaintVerified: true }), { type: "ignore", done: true });
  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, terminalFallbackActive: true }), { type: "ignore", done: true });
  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, hasCurrentSession: false }), { type: "ignore", done: true });
  assert.deepEqual(terminalPaintVerificationPlan(ready), { type: "check_canvas", done: false });
  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, canvasHasVisiblePixels: true }), {
    type: "painted",
    done: true,
    fallbackActive: false,
    diagnosticReason: "painted",
  });
  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, canvasHasVisiblePixels: false }), {
    type: "refresh_snapshot",
    done: false,
  });
  assert.deepEqual(terminalPaintVerificationPlan({ ...ready, afterSnapshotRefresh: true }), {
    type: "check_canvas",
    done: false,
  });
  assert.deepEqual(terminalPaintVerificationPlan({
    ...ready,
    afterSnapshotRefresh: true,
    canvasHasVisiblePixels: false,
    hasSnapshotText: true,
  }), {
    type: "activate_fallback",
    done: true,
    fallbackActive: true,
    clearText: false,
    syncPresentation: true,
  });
  assert.deepEqual(terminalPaintVerificationPlan({
    ...ready,
    afterSnapshotRefresh: true,
    canvasHasVisiblePixels: false,
    hasSnapshotText: false,
  }), { type: "ignore", done: true });
});

test("terminalSurfaceSessionPlan preserves selected-session setup gates", () => {
  assert.deepEqual(terminalSurfaceSessionPlan({ session: null }), { type: "teardown_terminal" });
  assert.deepEqual(terminalSurfaceSessionPlan({ session: { session_id: "agent-1" } }), {
    type: "load_renderer",
    sessionId: "agent-1",
  });
  assert.deepEqual(terminalSurfaceSessionPlan({
    session: {
      session_id: "skillbox::agent-1",
      environment: {
        scope: "remote",
        target_id: "skillbox",
        target_kind: "swimmers_api",
        remote_session_id: "agent-1",
      },
    },
  }), {
    type: "activate_snapshot_fallback",
    sessionId: "skillbox::agent-1",
    clearText: false,
  });
});

test("terminalSurfaceRendererPlan preserves renderer fallback, reuse, and init decisions", () => {
  assert.deepEqual(terminalSurfaceRendererPlan({ hasRendererModule: false, sessionId: "agent-1" }), {
    type: "activate_snapshot_fallback",
    clearText: false,
  });
  assert.deepEqual(terminalSurfaceRendererPlan({
    hasRendererModule: true,
    hasTerminal: true,
    terminalSessionId: "agent-1",
    sessionId: "agent-1",
    terminalFallbackActive: true,
  }), {
    type: "reuse_terminal",
    terminalCanvasHidden: false,
    terminalFallbackHidden: false,
    loadingVisible: false,
  });
  assert.deepEqual(terminalSurfaceRendererPlan({
    hasRendererModule: true,
    hasTerminal: true,
    terminalSessionId: "agent-2",
    sessionId: "agent-1",
  }), {
    type: "initialize_terminal",
    loadingVisible: true,
    loadingLabel: "Initializing terminal...",
  });
});

test("terminalSurfaceInitErrorPlan preserves renderer error fallback status", () => {
  assert.deepEqual(terminalSurfaceInitErrorPlan("boom"), {
    type: "renderer_error_fallback",
    clearText: false,
    refreshSnapshot: true,
    loadingVisible: false,
    status: "Live terminal renderer unavailable: boom",
    statusError: true,
    statusTimeoutMs: 3600,
  });
});

test("terminalSurfacePostInitPlan preserves successful setup side-effect data", () => {
  assert.deepEqual(terminalSurfacePostInitPlan({
    sessionId: "agent-1",
    linkPolicySupported: true,
    accessibilitySupported: true,
    reducedMotion: true,
  }), {
    type: "complete_terminal_init",
    sessionId: "agent-1",
    terminalPaintVerified: false,
    terminalFrameBytesSeen: 0,
    terminalFallbackActive: false,
    setLinkOpenPolicy: true,
    setAccessibility: true,
    accessibility: { reducedMotion: true, screenReader: true },
    terminalCanvasHidden: false,
    clearSelection: true,
    refreshSearch: true,
    syncMirror: true,
    syncTools: true,
    resize: { pushResize: true, force: true },
    flushPendingBytes: true,
    loadingVisible: false,
  });
});

test("terminalResizeGeometryPlan preserves resize geometry and side-effect decisions", () => {
  assert.deepEqual(terminalResizeGeometryPlan({
    cols: 90.9,
    rows: 30.2,
    currentCols: 80,
    currentRows: 24,
    pushResize: true,
    force: false,
    hasTerminal: true,
  }), {
    cols: 90,
    rows: 30,
    dimensionsChanged: true,
    shouldResize: true,
    sendResize: true,
    captureDiagnostic: true,
    diagnosticReason: "resize",
  });
  assert.deepEqual(terminalResizeGeometryPlan({
    cols: 80,
    rows: 24,
    currentCols: 80,
    currentRows: 24,
    pushResize: true,
    force: false,
    hasTerminal: true,
  }), {
    cols: 80,
    rows: 24,
    dimensionsChanged: false,
    shouldResize: false,
    sendResize: false,
    captureDiagnostic: false,
    diagnosticReason: "resize",
  });
  assert.deepEqual(terminalResizeGeometryPlan({
    cols: 999,
    rows: 1,
    currentCols: 240,
    currentRows: 12,
    pushResize: false,
    force: true,
    hasTerminal: false,
  }), {
    cols: 240,
    rows: 12,
    dimensionsChanged: false,
    shouldResize: true,
    sendResize: false,
    captureDiagnostic: false,
    diagnosticReason: "resize",
  });
});

test("terminalLiveFrameFallbackPlan preserves live-frame fallback update decisions", () => {
  assert.deepEqual(terminalLiveFrameFallbackPlan({
    terminalFallbackActive: false,
    hasTerminal: true,
    liveText: "prompt",
    existingFallbackText: "",
  }), { type: "ignore", update: false, text: "", preserveExistingFallback: false });
  assert.deepEqual(terminalLiveFrameFallbackPlan({
    terminalFallbackActive: true,
    hasTerminal: false,
    liveText: "prompt",
    existingFallbackText: "",
  }), { type: "ignore", update: false, text: "", preserveExistingFallback: false });
  assert.deepEqual(terminalLiveFrameFallbackPlan({
    terminalFallbackActive: true,
    hasTerminal: true,
    liveText: "   \n",
    existingFallbackText: "snapshot prompt",
  }), { type: "ignore", update: false, text: "", preserveExistingFallback: true });
  assert.deepEqual(terminalLiveFrameFallbackPlan({
    terminalFallbackActive: true,
    hasTerminal: true,
    liveText: "",
    existingFallbackText: "",
  }), { type: "ignore", update: false, text: "", preserveExistingFallback: false });
  assert.deepEqual(terminalLiveFrameFallbackPlan({
    terminalFallbackActive: true,
    hasTerminal: true,
    liveText: "$ cargo test",
    existingFallbackText: "snapshot prompt",
  }), { type: "update", update: true, text: "$ cargo test", preserveExistingFallback: false });
});

test("terminalInputDockPlan preserves visibility and disabled state decisions", () => {
  assert.deepEqual(terminalInputDockPlan({
    hasCurrentSession: true,
    trogdorAtlasOpen: false,
    readOnly: false,
    inputValue: " pwd ",
  }), {
    visible: true,
    hidden: false,
    ariaHidden: "false",
    inputDisabled: false,
    keyStripButtonDisabled: false,
    sendDisabled: false,
  });
  assert.deepEqual(terminalInputDockPlan({
    hasCurrentSession: true,
    trogdorAtlasOpen: false,
    readOnly: false,
    inputValue: "  ",
  }), {
    visible: true,
    hidden: false,
    ariaHidden: "false",
    inputDisabled: false,
    keyStripButtonDisabled: false,
    sendDisabled: true,
  });
  assert.deepEqual(terminalInputDockPlan({
    hasCurrentSession: false,
    trogdorAtlasOpen: false,
    readOnly: false,
    inputValue: "pwd",
  }), {
    visible: false,
    hidden: true,
    ariaHidden: "true",
    inputDisabled: true,
    keyStripButtonDisabled: true,
    sendDisabled: true,
  });
  assert.equal(terminalInputDockPlan({
    hasCurrentSession: true,
    trogdorAtlasOpen: true,
    readOnly: false,
    inputValue: "pwd",
  }).visible, false);
  assert.equal(terminalInputDockPlan({
    hasCurrentSession: true,
    trogdorAtlasOpen: false,
    readOnly: true,
    inputValue: "pwd",
  }).sendDisabled, true);
});

test("terminalZoomControlsPlan preserves support gates, bounds, and labels", () => {
  assert.equal(terminalZoomPercentLabel(1.25), "125%");
  assert.deepEqual(terminalZoomControlsPlan({
    zoomSupported: true,
    hasTerminal: true,
    zoom: 1,
    minZoom: 0.5,
    maxZoom: 2,
  }), {
    supported: true,
    zoomOutDisabled: false,
    zoomInDisabled: false,
    zoomResetDisabled: true,
    zoomResetLabel: "100%",
  });
  assert.deepEqual(terminalZoomControlsPlan({
    zoomSupported: false,
    hasTerminal: true,
    zoom: 1.5,
    minZoom: 0.5,
    maxZoom: 2,
  }), {
    supported: false,
    zoomOutDisabled: true,
    zoomInDisabled: true,
    zoomResetDisabled: true,
    zoomResetLabel: "150%",
  });
  assert.deepEqual(terminalZoomControlsPlan({
    zoomSupported: false,
    hasTerminal: false,
    zoom: 0.5,
    minZoom: 0.5,
    maxZoom: 2,
  }), {
    supported: true,
    zoomOutDisabled: true,
    zoomInDisabled: false,
    zoomResetDisabled: false,
    zoomResetLabel: "50%",
  });
  assert.equal(terminalZoomControlsPlan({
    zoomSupported: true,
    hasTerminal: true,
    zoom: 2,
    minZoom: 0.5,
    maxZoom: 2,
  }).zoomInDisabled, true);
});

test("normalizeTerminalZoomValue preserves parse, step, and clamp behavior", () => {
  const config = { minZoom: 0.5, maxZoom: 2, step: 0.1 };
  assert.equal(normalizeTerminalZoomValue("nope", config), 1);
  assert.equal(normalizeTerminalZoomValue("1.26", config), 1.3);
  assert.equal(normalizeTerminalZoomValue("1.24", config), 1.2000000000000002);
  assert.equal(normalizeTerminalZoomValue("3", config), 2);
  assert.equal(normalizeTerminalZoomValue("0.1", config), 0.5);
});

test("terminalZoomLoadValue preserves URL precedence over stored zoom", () => {
  const config = { minZoom: 0.5, maxZoom: 2, step: 0.1 };
  assert.equal(terminalZoomLoadValue({ urlZoom: "1.6", storedZoom: "0.7" }, config), 1.6);
  assert.equal(terminalZoomLoadValue({ urlZoom: null, storedZoom: "0.7" }, config), 0.7000000000000001);
  assert.equal(terminalZoomLoadValue({ urlZoom: null, storedZoom: "" }, config), 1);
  assert.equal(terminalZoomLoadValue({ urlZoom: "bad", storedZoom: "0.7" }, config), 1);
});

test("terminalZoomPersistencePlan preserves URL and storage values", () => {
  assert.deepEqual(terminalZoomPersistencePlan(1), {
    storageValue: "1.00",
    urlParamAction: "delete",
    urlParamValue: "",
  });
  assert.deepEqual(terminalZoomPersistencePlan(0.9995), {
    storageValue: "1.00",
    urlParamAction: "delete",
    urlParamValue: "",
  });
  assert.deepEqual(terminalZoomPersistencePlan(1.25), {
    storageValue: "1.25",
    urlParamAction: "set",
    urlParamValue: "1.25",
  });
  assert.deepEqual(terminalZoomPersistencePlan(0.5), {
    storageValue: "0.50",
    urlParamAction: "set",
    urlParamValue: "0.50",
  });
});

test("terminalAuxiliaryControlsPlan preserves mobile keyboard and copy-frame gates", () => {
  assert.deepEqual(terminalAuxiliaryControlsPlan({
    hasCurrentSession: true,
    readOnly: false,
    mobileKeyboardActive: false,
    hasCopyFrame: true,
  }), {
    mobileKeyboardDisabled: false,
    mobileKeyboardAriaPressed: "false",
    copyFrameAvailable: true,
    copyFrameDisabled: false,
  });
  assert.deepEqual(terminalAuxiliaryControlsPlan({
    hasCurrentSession: true,
    readOnly: true,
    mobileKeyboardActive: true,
    hasCopyFrame: true,
  }), {
    mobileKeyboardDisabled: true,
    mobileKeyboardAriaPressed: "true",
    copyFrameAvailable: true,
    copyFrameDisabled: false,
  });
  assert.deepEqual(terminalAuxiliaryControlsPlan({
    hasCurrentSession: false,
    readOnly: false,
    mobileKeyboardActive: true,
    hasCopyFrame: true,
  }), {
    mobileKeyboardDisabled: true,
    mobileKeyboardAriaPressed: "true",
    copyFrameAvailable: true,
    copyFrameDisabled: true,
  });
  assert.deepEqual(terminalAuxiliaryControlsPlan({
    hasCurrentSession: true,
    readOnly: false,
    mobileKeyboardActive: false,
    hasCopyFrame: false,
  }), {
    mobileKeyboardDisabled: false,
    mobileKeyboardAriaPressed: "false",
    copyFrameAvailable: false,
    copyFrameDisabled: true,
  });
});

test("terminalToolsAvailabilityPlan preserves control disabled states", () => {
  assert.deepEqual(terminalToolsAvailabilityPlan({
    searchReady: true,
    liveTerminal: true,
    frankenTermAvailable: true,
    searchQuery: "needle",
    readOnly: false,
    sendTargetType: "session",
    hasCurrentSession: true,
  }), {
    searchDisabled: false,
    sendInputDisabled: false,
    sendModeDisabled: false,
    sendSubmitDisabled: false,
    createFormElementsDisabled: false,
    searchStatus: null,
  });
  assert.deepEqual(terminalToolsAvailabilityPlan({
    searchReady: false,
    liveTerminal: true,
    frankenTermAvailable: true,
    searchQuery: "",
    readOnly: true,
    sendTargetType: "group",
    hasCurrentSession: false,
  }), {
    searchDisabled: true,
    sendInputDisabled: true,
    sendModeDisabled: true,
    sendSubmitDisabled: true,
    createFormElementsDisabled: true,
    searchStatus: { label: "Search unavailable in this FrankenTerm build", muted: true },
  });
  assert.equal(terminalToolsAvailabilityPlan({
    searchReady: true,
    liveTerminal: true,
    readOnly: false,
    sendTargetType: "group",
    hasCurrentSession: true,
  }).sendModeDisabled, true);
});

test("initialStateBootPlan preserves token, session, directory, and desktop defaults", () => {
  assert.deepEqual(initialStateBootPlan({
    searchParams: new URLSearchParams("token=query-token&session=url-session&preset=remote-api"),
    storedToken: "stored-token",
    selectedFromStorage: "stored-session",
    rawStoredDirPath: " /repo/swimmers ",
    rawStoredManagedOnly: "true",
    rawStoredFleetFilter: JSON.stringify({ kind: "TARGET", key: "skillbox" }),
    rawStoredFleetPresetId: "local",
    rawStoredSessionGroupMode: "project",
    bootFollowPublishedSelection: false,
    terminalWorkbenchMobile: false,
  }), {
    queryToken: "query-token",
    tokenToPersist: "query-token",
    selectedSessionId: "url-session",
    followFromUrl: false,
    followPublishedSelection: false,
    storedDirPath: "/repo/swimmers",
    clearStoredDirPath: false,
    storedManagedOnly: true,
    storedFleetFilter: { kind: "target", key: "skillbox" },
    storedFleetPresetId: "remote-api",
    storedSessionGroupMode: "project",
    terminalWorkbenchOpen: true,
  });
});

test("initialStateBootPlan preserves stored fallbacks and root directory cleanup", () => {
  assert.deepEqual(initialStateBootPlan({
    searchParams: new URLSearchParams(""),
    storedToken: "stored-token",
    selectedFromStorage: "stored-session",
    rawStoredDirPath: "/",
    rawStoredManagedOnly: "false",
    rawStoredFleetFilter: "{not json",
    rawStoredFleetPresetId: "ssh-handoff",
    rawStoredSessionGroupMode: "unknown",
    bootFollowPublishedSelection: false,
    terminalWorkbenchMobile: true,
  }), {
    queryToken: "",
    tokenToPersist: "stored-token",
    selectedSessionId: "stored-session",
    followFromUrl: false,
    followPublishedSelection: false,
    storedDirPath: "",
    clearStoredDirPath: true,
    storedManagedOnly: false,
    storedFleetFilter: { kind: "", key: "" },
    storedFleetPresetId: "ssh-handoff",
    storedSessionGroupMode: "flat",
    terminalWorkbenchOpen: false,
  });
});

test("initialStateBootPlan preserves follow-published selection override", () => {
  assert.deepEqual(initialStateBootPlan({
    searchParams: new URLSearchParams("follow=published&session=url-session"),
    storedToken: "",
    selectedFromStorage: "stored-session",
    rawStoredDirPath: " / ",
    rawStoredManagedOnly: null,
    bootFollowPublishedSelection: false,
    terminalWorkbenchMobile: false,
  }), {
    queryToken: "",
    tokenToPersist: "",
    selectedSessionId: null,
    followFromUrl: true,
    followPublishedSelection: true,
    storedDirPath: "",
    clearStoredDirPath: true,
    storedManagedOnly: false,
    storedFleetFilter: { kind: "", key: "" },
    storedFleetPresetId: "",
    storedSessionGroupMode: "flat",
    terminalWorkbenchOpen: true,
  });

  assert.equal(initialStateBootPlan({
    searchParams: new URLSearchParams("session=url-session"),
    selectedFromStorage: "stored-session",
    bootFollowPublishedSelection: true,
  }).selectedSessionId, null);
});

test("controlEventSessionPatchPlan preserves session_state payload semantics", () => {
  const evidence = { source: "bridge" };
  const session = { session_id: "sess_1", state: "idle", cwd: "/old" };
  const plan = controlEventSessionPatchPlan(session, {
    event: "session_state",
    payload: {
      state: "busy",
      previous_state: null,
      current_command: "",
      state_evidence: evidence,
      transport_health: "healthy",
      exit_reason: "done",
      at: "2026-06-03T13:00:00Z",
    },
  });

  assert.equal(plan.event, "session_state");
  assert.deepEqual(plan.session, {
    session_id: "sess_1",
    state: "busy",
    cwd: "/old",
    last_control_event: "session_state",
    previous_state: null,
    current_command: "",
    state_evidence: evidence,
    transport_health: "healthy",
    exit_reason: "done",
    last_activity_at: "2026-06-03T13:00:00Z",
  });
  assert.notEqual(plan.session, session);
});

test("controlEventSessionPatchPlan preserves title and skill event semantics", () => {
  assert.deepEqual(controlEventSessionPatchPlan({
    session_id: "sess_1",
    cwd: "/old",
  }, {
    event: "session_title",
    payload: { title: " /repo/swimmers " },
  }).session, {
    session_id: "sess_1",
    cwd: "/repo/swimmers",
    last_control_event: "session_title",
    terminal_title: "/repo/swimmers",
  });

  assert.deepEqual(controlEventSessionPatchPlan({
    session_id: "sess_1",
    cwd: "/old",
  }, {
    event: "session_title",
    payload: { title: "agent shell" },
  }).session, {
    session_id: "sess_1",
    cwd: "/old",
    last_control_event: "session_title",
    terminal_title: "agent shell",
  });

  assert.deepEqual(controlEventSessionPatchPlan({
    session_id: "sess_1",
    last_skill: "old",
  }, {
    event: "session_skill",
    payload: { last_skill: null },
  }).session, {
    session_id: "sess_1",
    last_skill: null,
    last_control_event: "session_skill",
  });
});

test("controlEventSessionPatchPlan preserves thought update payload semantics", () => {
  const actionCues = ["needs_input"];
  assert.deepEqual(controlEventSessionPatchPlan({
    session_id: "sess_1",
    commit_candidate: false,
  }, {
    event: "thought_update",
    payload: {
      thought: "",
      token_count: 0,
      context_limit: null,
      thought_state: "ready",
      thought_source: "daemon",
      rest_state: "awake",
      commit_candidate: "yes",
      action_cues: actionCues,
      at: "2026-06-03T13:01:00Z",
      objective_changed: true,
    },
  }).session, {
    session_id: "sess_1",
    commit_candidate: true,
    last_control_event: "thought_update",
    thought: "",
    token_count: 0,
    context_limit: null,
    thought_state: "ready",
    thought_source: "daemon",
    rest_state: "awake",
    action_cues: actionCues,
    thought_updated_at: "2026-06-03T13:01:00Z",
    objective_changed_at: "2026-06-03T13:01:00Z",
  });
});

test("controlEventSessionPatchPlan preserves unknown event and non-object payload behavior", () => {
  assert.deepEqual(controlEventSessionPatchPlan({
    session_id: "sess_1",
    state: "old",
  }, {
    event: "mystery",
    payload: "not an object",
  }).session, {
    session_id: "sess_1",
    state: "old",
    last_control_event: "mystery",
  });

  assert.deepEqual(controlEventSessionPatchPlan({
    session_id: "sess_1",
    state: "old",
  }, {
    event: "session_state",
    payload: {
      state: "",
      state_evidence: null,
      transport_health: "",
      exit_reason: "",
      at: "",
    },
  }).session, {
    session_id: "sess_1",
    state: "old",
    last_control_event: "session_state",
  });
});

test("lifecycleDeletedSessionPatchPlan preserves deleted-session patch semantics", () => {
  const session = {
    session_id: "sess_1",
    state: "running",
    is_stale: false,
    transport_health: "healthy",
    cwd: "/repo",
  };
  const patch = lifecycleDeletedSessionPatchPlan(session, {
    reason: "closed",
    deleteMode: "kill-pane",
    delete_mode: "fallback",
    tmuxSessionAlive: false,
    tmux_session_alive: true,
  });

  assert.deepEqual(patch, {
    session_id: "sess_1",
    state: "exited",
    is_stale: true,
    transport_health: "disconnected",
    cwd: "/repo",
    delete_reason: "closed",
    delete_mode: "kill-pane",
    tmux_session_alive: false,
  });
  assert.notEqual(patch, session);
});

test("lifecycleDeletedSessionPatchPlan preserves fallback and default deleted-session fields", () => {
  assert.deepEqual(lifecycleDeletedSessionPatchPlan({
    session_id: "sess_1",
    delete_reason: "old",
    delete_mode: "old",
    tmux_session_alive: true,
  }, {
    reason: "",
    delete_mode: "server-delete",
    tmux_session_alive: 1,
  }), {
    session_id: "sess_1",
    state: "exited",
    is_stale: true,
    transport_health: "disconnected",
    delete_reason: "",
    delete_mode: "server-delete",
    tmux_session_alive: true,
  });

  assert.deepEqual(lifecycleDeletedSessionPatchPlan({
    session_id: "sess_2",
  }, {}), {
    session_id: "sess_2",
    state: "exited",
    is_stale: true,
    transport_health: "disconnected",
    delete_reason: "",
    delete_mode: "",
    tmux_session_alive: false,
  });
});

test("inputAckActionPlan preserves delivered ack semantics", () => {
  assert.deepEqual(inputAckActionPlan({
    clientMessageId: "client-1",
    client_message_id: "fallback",
    delivered: true,
    method: "send_keys",
    message: "ignored",
  }), {
    action: "update",
    id: "client-1",
    status: "sent",
    detail: "send_keys",
    expectedStatus: "sent",
    delayMs: 2500,
  });

  assert.deepEqual(inputAckActionPlan({
    client_message_id: "client-2",
    delivered: 1,
    method: "",
  }), {
    action: "update",
    id: "client-2",
    status: "sent",
    detail: "",
    expectedStatus: "sent",
    delayMs: 2500,
  });
});

test("inputAckActionPlan preserves failed ack semantics", () => {
  assert.deepEqual(inputAckActionPlan({
    clientMessageId: "client-3",
    delivered: false,
    message: "not attached",
  }), {
    action: "update",
    id: "client-3",
    status: "failed",
    detail: "not attached",
    expectedStatus: "failed",
    delayMs: 8000,
  });

  assert.deepEqual(inputAckActionPlan({
    client_message_id: "client-4",
    delivered: 0,
    message: "",
  }), {
    action: "update",
    id: "client-4",
    status: "failed",
    detail: "input delivery failed",
    expectedStatus: "failed",
    delayMs: 8000,
  });
});

test("inputAckActionPlan preserves missing id ignore semantics", () => {
  assert.deepEqual(inputAckActionPlan({
    delivered: true,
    method: "send_keys",
  }), {
    action: "ignore",
    id: "",
    status: "",
    detail: "",
    expectedStatus: "",
    delayMs: 0,
  });

  assert.deepEqual(inputAckActionPlan({
    clientMessageId: 0,
    client_message_id: "",
    delivered: false,
  }), {
    action: "ignore",
    id: "",
    status: "",
    detail: "",
    expectedStatus: "",
    delayMs: 0,
  });
});

test("sheetActionAvailabilityPlan preserves enabled action state", () => {
  assert.deepEqual(sheetActionAvailabilityPlan({
    writeDisabled: false,
    hasSession: true,
    batchReady: true,
    hasSinglePath: false,
    visibleSelectableCount: 1,
    hasBrowserPath: true,
    hasThoughtConfig: true,
    hasNativeStatus: true,
    nativeSupported: true,
    hasMermaidPath: true,
    hasDirsPath: true,
    hasParentDir: true,
    sendTargetType: "session",
    sendTargetReady: true,
  }), {
    createButtonDisabled: false,
    createBatchSubmitDisabled: false,
    createBatchVisibleDisabled: false,
    dirsSpawnHereDisabled: false,
    thoughtConfigTestDisabled: false,
    thoughtConfigSaveDisabled: false,
    nativeSaveDisabled: false,
    nativeOpenDisabled: false,
    nativeRefreshDisabled: false,
    mermaidOpenDisabled: false,
    mermaidRefreshDisabled: false,
    dirsLoadDisabled: false,
    dirsUpDisabled: false,
    sendModeDisabled: false,
    sendSubmitDisabled: false,
  });
});

test("sheetActionAvailabilityPlan blocks batch submit for unmapped remote rows", () => {
  const plan = sheetActionAvailabilityPlan({
    writeDisabled: false,
    hasSession: true,
    batchReady: true,
    batchBlocked: true,
    hasSinglePath: true,
    visibleSelectableCount: 1,
    hasBrowserPath: true,
    hasThoughtConfig: true,
    hasNativeStatus: true,
    nativeSupported: true,
    hasMermaidPath: true,
    hasDirsPath: true,
    hasParentDir: true,
    sendTargetType: "session",
    sendTargetReady: true,
  });

  assert.equal(plan.createButtonDisabled, false);
  assert.equal(plan.createBatchSubmitDisabled, true);
});

test("sheetActionAvailabilityPlan blocks single create for unmapped remote cwd", () => {
  const plan = sheetActionAvailabilityPlan({
    writeDisabled: false,
    hasSession: true,
    batchReady: false,
    singleBlocked: true,
    hasSinglePath: true,
    visibleSelectableCount: 1,
    hasBrowserPath: true,
    hasThoughtConfig: true,
    hasNativeStatus: true,
    nativeSupported: true,
    hasMermaidPath: true,
    hasDirsPath: true,
    hasParentDir: true,
    sendTargetType: "session",
    sendTargetReady: true,
  });

  assert.equal(plan.createButtonDisabled, true);
  assert.equal(plan.createBatchSubmitDisabled, true);
});

test("sheetActionAvailabilityPlan keeps remote handoff reachable without local native support", () => {
  const plan = sheetActionAvailabilityPlan({
    writeDisabled: false,
    hasSession: true,
    nativeSupported: false,
    nativeHandoffAvailable: true,
  });

  assert.equal(plan.nativeOpenDisabled, false);
});

test("sheetActionAvailabilityPlan preserves read-only and missing-resource disabled state", () => {
  assert.deepEqual(sheetActionAvailabilityPlan({
    writeDisabled: true,
    hasSession: false,
    batchReady: false,
    hasSinglePath: false,
    visibleSelectableCount: 0,
    hasBrowserPath: false,
    hasThoughtConfig: false,
    hasNativeStatus: false,
    nativeSupported: false,
    hasMermaidPath: false,
    hasDirsPath: false,
    hasParentDir: false,
    sendTargetType: "group",
    sendTargetReady: false,
  }), {
    createButtonDisabled: true,
    createBatchSubmitDisabled: true,
    createBatchVisibleDisabled: true,
    dirsSpawnHereDisabled: true,
    thoughtConfigTestDisabled: true,
    thoughtConfigSaveDisabled: true,
    nativeSaveDisabled: true,
    nativeOpenDisabled: true,
    nativeRefreshDisabled: false,
    mermaidOpenDisabled: true,
    mermaidRefreshDisabled: true,
    dirsLoadDisabled: true,
    dirsUpDisabled: true,
    sendModeDisabled: true,
    sendSubmitDisabled: true,
  });
});

test("sheetActionAvailabilityPlan preserves path, group, and artifact fallback gates", () => {
  const plan = sheetActionAvailabilityPlan({
    writeDisabled: false,
    hasSession: true,
    batchReady: false,
    hasSinglePath: true,
    visibleSelectableCount: 0,
    hasBrowserPath: true,
    hasThoughtConfig: true,
    hasNativeStatus: true,
    nativeSupported: false,
    hasMermaidPath: false,
    hasDirsPath: true,
    hasParentDir: true,
    sendTargetType: "group",
    sendTargetReady: true,
  });
  assert.equal(plan.createButtonDisabled, false);
  assert.equal(plan.createBatchSubmitDisabled, true);
  assert.equal(plan.createBatchVisibleDisabled, true);
  assert.equal(plan.dirsSpawnHereDisabled, false);
  assert.equal(plan.nativeOpenDisabled, true);
  assert.equal(plan.mermaidOpenDisabled, true);
  assert.equal(plan.mermaidRefreshDisabled, false);
  assert.equal(plan.dirsLoadDisabled, false);
  assert.equal(plan.dirsUpDisabled, false);
  assert.equal(plan.sendModeDisabled, true);
  assert.equal(plan.sendSubmitDisabled, false);
});

test("terminalToolsAvailabilityPlan preserves search status copy", () => {
  assert.deepEqual(terminalToolsAvailabilityPlan({
    searchReady: true,
    liveTerminal: false,
    frankenTermAvailable: true,
    searchQuery: "",
  }).searchStatus, { label: "Search waits for terminal attach", muted: true });
  assert.deepEqual(terminalToolsAvailabilityPlan({
    searchReady: true,
    liveTerminal: false,
    frankenTermAvailable: false,
    searchQuery: "",
  }).searchStatus, { label: "Search needs FrankenTerm assets", muted: true });
  assert.deepEqual(terminalToolsAvailabilityPlan({
    searchReady: true,
    liveTerminal: true,
    frankenTermAvailable: true,
    searchQuery: "",
  }).searchStatus, { label: "Search idle", muted: true });
});
