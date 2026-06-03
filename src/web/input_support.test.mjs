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
  terminalFallbackFocusPlan,
  terminalFallbackKeydownPlan,
  terminalFallbackPastePlan,
  terminalFallbackPointerFocusPlan,
  terminalKeyStripClickExecutorPlan,
  terminalKeyStripClickPlan,
  terminalStageCaptureBindings,
  terminalStageFocusExecutorPlan,
  terminalStageFocusPlan,
  terminalStageKeydownPlan,
  terminalStagePasteExecutorPlan,
  terminalStagePastePlan,
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
