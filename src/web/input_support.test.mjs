import test from "node:test";
import assert from "node:assert/strict";

import {
  authTokenButtonPlan, eventCell,
  eventClientPoint,
  globalShortcutPlan,
  mobileKeyboardInputExecutorPlan,
  mobileKeyboardInputPlan,
  mobileKeyboardKeydownPlan,
  mobileKeyboardKeyPlan,
  shouldIgnoreSyntheticClick,
  terminalComposerControlAction,
  terminalDestroyStatePatch,
  terminalFallbackActivationPlan,
  terminalFallbackFocusPlan,
  terminalFallbackKeydownPlan,
  terminalFallbackPastePlan,
  terminalFallbackPointerFocusPlan,
  terminalFallbackScrollPlan,
  terminalFallbackTextScrollPlan,
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
  terminalStageCaptureBindings,
  terminalStageFocusExecutorPlan,
  terminalStageFocusPlan,
  terminalStageKeydownPlan,
  terminalStagePasteExecutorPlan,
  terminalStagePastePlan,
  terminalAuxiliaryControlsPlan,
  terminalZoomControlsPlan,
  terminalZoomPercentLabel,
  terminalZoomPersistencePlan,
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

test("terminalStagePastePlan preserves read-only, empty, and raw text decisions", () => {
  assert.deepEqual(terminalStagePastePlan(true, "paste me\n"), { type: "ignore" });
  assert.deepEqual(terminalStagePastePlan(false, ""), { type: "ignore" });
  assert.deepEqual(terminalStagePastePlan(false, " paste me\n"), {
    type: "send_text",
    text: " paste me\n",
  });
});

test("terminalStagePasteExecutorPlan preserves ignore, send, and unknown decisions", () => {
  const ignored = { type: "ignore", preventDefault: false, sendText: false, text: "" };

  assert.deepEqual(terminalStagePasteExecutorPlan({ type: "ignore" }), ignored);
  assert.deepEqual(terminalStagePasteExecutorPlan({ type: "send_text", text: " paste me\n" }), {
    type: "send_text",
    preventDefault: true,
    sendText: true,
    text: " paste me\n",
  });
  assert.deepEqual(terminalStagePasteExecutorPlan({ type: "unknown", text: "ignored" }), ignored);
});

test("terminalFallbackPastePlan preserves gating, propagation, and exact text", () => {
  const ignored = {
    type: "ignore",
    handled: false,
    preventDefault: false,
    stopPropagation: false,
    sendText: false,
    text: "",
  };
  assert.deepEqual(terminalFallbackPastePlan({
    terminalFallbackActive: false,
    readOnly: false,
    hasCurrentSession: true,
    text: "paste",
  }), ignored);
  assert.deepEqual(terminalFallbackPastePlan({
    terminalFallbackActive: true,
    readOnly: true,
    hasCurrentSession: true,
    text: "paste",
  }), ignored);
  assert.deepEqual(terminalFallbackPastePlan({
    terminalFallbackActive: true,
    readOnly: false,
    hasCurrentSession: false,
    text: "paste",
  }), ignored);
  assert.deepEqual(terminalFallbackPastePlan({
    terminalFallbackActive: true,
    readOnly: false,
    hasCurrentSession: true,
    text: "",
  }), ignored);
  assert.deepEqual(terminalFallbackPastePlan({
    terminalFallbackActive: true,
    readOnly: false,
    hasCurrentSession: true,
    text: " paste me\n",
  }), {
    type: "send_text",
    handled: true,
    preventDefault: true,
    stopPropagation: true,
    sendText: true,
    text: " paste me\n",
  });
});

test("terminalStageFocusPlan preserves focus, blur, and ignore decisions", () => {
  assert.deepEqual(terminalStageFocusPlan("focus", { activeSheet: "send" }), { type: "ignore" });
  assert.deepEqual(terminalStageFocusPlan("focus", { activeSheet: "" }), {
    type: "forward_event",
    event: { kind: "focus", focused: true },
  });
  assert.deepEqual(terminalStageFocusPlan("blur", { mobileKeyboardOwnsFocus: true }), { type: "ignore" });
  assert.deepEqual(terminalStageFocusPlan("blur", { mobileKeyboardOwnsFocus: false }), {
    type: "forward_event",
    event: { kind: "focus", focused: false },
  });
  assert.deepEqual(terminalStageFocusPlan("click"), { type: "ignore" });
});

test("terminalFallbackFocusPlan preserves fallback focus, blur, and no-op gates", () => {
  assert.deepEqual(terminalFallbackFocusPlan("focus", { terminalFallbackActive: false }), { type: "ignore" });
  assert.deepEqual(terminalFallbackFocusPlan("focus", { terminalFallbackActive: true, activeSheet: "send" }), { type: "ignore" });
  assert.deepEqual(terminalFallbackFocusPlan("focus", { terminalFallbackActive: true, activeSheet: "" }), {
    type: "forward_event",
    event: { kind: "focus", focused: true },
  });
  assert.deepEqual(terminalFallbackFocusPlan("blur", { terminalFallbackActive: true, mobileKeyboardOwnsFocus: true }), { type: "ignore" });
  assert.deepEqual(terminalFallbackFocusPlan("blur", { terminalFallbackActive: true, mobileKeyboardOwnsFocus: false }), {
    type: "forward_event",
    event: { kind: "focus", focused: false },
  });
  assert.deepEqual(terminalFallbackFocusPlan("click", { terminalFallbackActive: true }), { type: "ignore" });
});

test("terminalFallbackActivationPlan preserves fallback activation side-effect decisions", () => {
  assert.deepEqual(terminalFallbackActivationPlan({ active: true, hasCurrentSession: true, wasActive: false, hasTerminal: true }), {
    type: "activate",
    terminalFallbackActive: true,
    hidden: false,
    ariaHidden: "false",
    updateAutoFollow: true,
    autoFollow: true,
    startSnapshotPolling: true,
    focusTerminal: true,
    clearText: false,
    stopSnapshotPolling: false,
    syncStatus: true,
  });
  assert.equal(terminalFallbackActivationPlan({ active: true, hasCurrentSession: true, wasActive: true, nearBottom: true }).autoFollow, true);
  assert.equal(terminalFallbackActivationPlan({ active: true, hasCurrentSession: true, wasActive: true, nearBottom: false }).autoFollow, false);
  assert.deepEqual(terminalFallbackActivationPlan({ active: false, hasCurrentSession: false, hasTerminal: true }), {
    type: "deactivate",
    terminalFallbackActive: false,
    hidden: true,
    ariaHidden: "true",
    updateAutoFollow: false,
    autoFollow: null,
    startSnapshotPolling: false,
    focusTerminal: false,
    clearText: true,
    stopSnapshotPolling: true,
    syncStatus: true,
  });
  assert.deepEqual(terminalFallbackActivationPlan({ active: false, hasCurrentSession: true, hasTerminal: false, clearText: false }), {
    type: "deactivate",
    terminalFallbackActive: false,
    hidden: true,
    ariaHidden: "true",
    updateAutoFollow: false,
    autoFollow: null,
    startSnapshotPolling: false,
    focusTerminal: false,
    clearText: false,
    stopSnapshotPolling: false,
    syncStatus: true,
  });
  assert.equal(terminalFallbackActivationPlan({ active: true, hasCurrentSession: false }).terminalFallbackActive, false);
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

test("terminalFallbackPointerFocusPlan preserves scheduled and immediate focus gates", () => {
  const ignored = { type: "ignore", focusTerminal: false, scheduleFrame: false };

  assert.deepEqual(terminalFallbackPointerFocusPlan("mousedown", { terminalFallbackActive: false, activeSheet: "" }), ignored);
  assert.deepEqual(terminalFallbackPointerFocusPlan("click", { terminalFallbackActive: true, activeSheet: "send" }), ignored);
  assert.deepEqual(terminalFallbackPointerFocusPlan("mousedown", { terminalFallbackActive: true, activeSheet: "" }), {
    type: "focus_terminal",
    focusTerminal: true,
    scheduleFrame: true,
  });
  assert.deepEqual(terminalFallbackPointerFocusPlan("click", { terminalFallbackActive: true, activeSheet: "" }), {
    type: "focus_terminal",
    focusTerminal: true,
    scheduleFrame: false,
  });
  assert.deepEqual(terminalFallbackPointerFocusPlan("touchend", { terminalFallbackActive: true, activeSheet: "" }), ignored);
});

test("terminalFallbackScrollPlan preserves auto-follow gates and values", () => {
  const ignored = { type: "ignore", updateAutoFollow: false, autoFollow: null };

  assert.deepEqual(terminalFallbackScrollPlan("scroll", { terminalFallbackActive: false, nearBottom: true }), ignored);
  assert.deepEqual(terminalFallbackScrollPlan("scroll", { terminalFallbackActive: true, nearBottom: true }), {
    type: "set_auto_follow",
    updateAutoFollow: true,
    autoFollow: true,
  });
  assert.deepEqual(terminalFallbackScrollPlan("scroll", { terminalFallbackActive: true, nearBottom: false }), {
    type: "set_auto_follow",
    updateAutoFollow: true,
    autoFollow: false,
  });
  assert.deepEqual(terminalFallbackScrollPlan("scroll", { terminalFallbackActive: true }), ignored);
  assert.deepEqual(terminalFallbackScrollPlan("resize", { terminalFallbackActive: true, nearBottom: true }), ignored);
  assert.deepEqual(terminalFallbackScrollPlan(), ignored);
});

test("terminalFallbackTextScrollPlan preserves follow and scroll clamp decisions", () => {
  assert.deepEqual(terminalFallbackTextScrollPlan({
    terminalFallbackAutoFollow: true,
    nearBottom: false,
    previousScrollTop: 40,
    scrollHeight: 200,
    clientHeight: 50,
  }), { type: "follow", scrollTop: 200 });
  assert.deepEqual(terminalFallbackTextScrollPlan({
    terminalFallbackAutoFollow: false,
    nearBottom: true,
    previousScrollTop: 40,
    scrollHeight: 180,
    clientHeight: 50,
  }), { type: "follow", scrollTop: 180 });
  assert.deepEqual(terminalFallbackTextScrollPlan({
    terminalFallbackAutoFollow: false,
    nearBottom: false,
    previousScrollTop: 80,
    scrollHeight: 200,
    clientHeight: 50,
  }), { type: "preserve", scrollTop: 80 });
  assert.deepEqual(terminalFallbackTextScrollPlan({
    terminalFallbackAutoFollow: false,
    nearBottom: false,
    previousScrollTop: 180,
    scrollHeight: 200,
    clientHeight: 50,
  }), { type: "preserve", scrollTop: 150 });
  assert.deepEqual(terminalFallbackTextScrollPlan({
    terminalFallbackAutoFollow: false,
    nearBottom: false,
    previousScrollTop: 10,
    scrollHeight: 20,
    clientHeight: 50,
  }), { type: "preserve", scrollTop: 0 });
});

test("terminalDestroyStatePatch returns exact fresh terminal teardown state", () => {
  const expected = {
    selectionAnchor: null,
    selectionFocus: null,
    terminal: null,
    terminalAcceptsBytes: false,
    terminalSessionId: null,
    terminalFallbackAutoFollow: true,
    terminalMirrorText: "",
    terminalPaintVerified: false,
    terminalFrameBytesSeen: 0,
  };
  const first = terminalDestroyStatePatch();
  const second = terminalDestroyStatePatch();

  assert.deepEqual(first, expected);
  assert.deepEqual(Object.keys(first), Object.keys(expected));
  assert.notStrictEqual(first, second);
  first.terminalMirrorText = "changed";
  assert.deepEqual(second, expected);
});

test("terminalStageFocusExecutorPlan preserves ignore, forward, and unknown decisions", () => {
  const event = { kind: "focus", focused: true };
  const ignored = { type: "ignore", forwardEvent: false, event: null };

  assert.deepEqual(terminalStageFocusExecutorPlan({ type: "ignore" }), ignored);
  assert.deepEqual(terminalStageFocusExecutorPlan(terminalFallbackFocusPlan("focus", { terminalFallbackActive: false })), ignored);
  assert.deepEqual(terminalStageFocusExecutorPlan({ type: "forward_event", event }), {
    type: "forward_event",
    forwardEvent: true,
    event,
  });
  assert.deepEqual(terminalStageFocusExecutorPlan({ type: "unknown", event }), ignored);
});

test("terminalStageKeydownPlan preserves shortcut, capture, and response decisions", () => {
  assert.deepEqual(terminalStageKeydownPlan({
    globalShortcutHandled: true,
    shouldCaptureKey: true,
    beginsResponse: true,
  }), { type: "prevent_default", preventDefault: true, markResponse: false, forwardKey: false });
  assert.deepEqual(terminalStageKeydownPlan({
    globalShortcutHandled: false,
    shouldCaptureKey: false,
    beginsResponse: true,
  }), { type: "ignore", preventDefault: false, markResponse: false, forwardKey: false });
  assert.deepEqual(terminalStageKeydownPlan({
    globalShortcutHandled: false,
    shouldCaptureKey: true,
    beginsResponse: false,
  }), { type: "forward_key", preventDefault: true, markResponse: false, forwardKey: true });
  assert.deepEqual(terminalStageKeydownPlan({
    globalShortcutHandled: false,
    shouldCaptureKey: true,
    beginsResponse: true,
  }), { type: "forward_key", preventDefault: true, markResponse: true, forwardKey: true });
});

test("terminalFallbackKeydownPlan preserves active, shortcut, capture, and response decisions", () => {
  assert.deepEqual(terminalFallbackKeydownPlan({
    terminalFallbackActive: false,
    globalShortcutHandled: true,
    shouldCaptureKey: true,
    beginsResponse: true,
  }), {
    type: "ignore",
    handled: false,
    preventDefault: false,
    stopPropagation: false,
    markResponse: false,
    forwardKey: false,
  });
  assert.deepEqual(terminalFallbackKeydownPlan({
    terminalFallbackActive: true,
    globalShortcutHandled: true,
    shouldCaptureKey: true,
    beginsResponse: true,
  }), {
    type: "prevent_default",
    preventDefault: true,
    markResponse: false,
    forwardKey: false,
    handled: true,
    stopPropagation: true,
  });
  assert.deepEqual(terminalFallbackKeydownPlan({
    terminalFallbackActive: true,
    globalShortcutHandled: false,
    shouldCaptureKey: false,
    beginsResponse: true,
  }), {
    type: "ignore",
    preventDefault: false,
    markResponse: false,
    forwardKey: false,
    handled: false,
    stopPropagation: false,
  });
  assert.deepEqual(terminalFallbackKeydownPlan({
    terminalFallbackActive: true,
    globalShortcutHandled: false,
    shouldCaptureKey: true,
    beginsResponse: false,
  }), {
    type: "forward_key",
    preventDefault: true,
    markResponse: false,
    forwardKey: true,
    handled: true,
    stopPropagation: true,
  });
  assert.deepEqual(terminalFallbackKeydownPlan({
    terminalFallbackActive: true,
    globalShortcutHandled: false,
    shouldCaptureKey: true,
    beginsResponse: true,
  }), {
    type: "forward_key",
    preventDefault: true,
    markResponse: true,
    forwardKey: true,
    handled: true,
    stopPropagation: true,
  });
});
