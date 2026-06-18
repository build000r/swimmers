import test from "node:test";
import assert from "node:assert/strict";

import {
  terminalDestroyStatePatch,
  terminalFallbackActivationPlan,
  terminalFallbackFocusPlan,
  terminalFallbackKeydownPlan,
  terminalFallbackPastePlan,
  terminalFallbackPointerFocusPlan,
  terminalFallbackScrollPlan,
  terminalFallbackTextScrollPlan,
  terminalStageClickPlan,
  terminalStageFocusExecutorPlan,
  terminalStageFocusPlan,
  terminalStageKeydownPlan,
  terminalStageMouseDownPlan,
  terminalStageMouseMovePlan,
  terminalStageMouseUpPlan,
  terminalStagePasteExecutorPlan,
  terminalStagePastePlan,
  terminalStageTouchEndPlan,
  terminalStageWheelPlan,
} from "./input_support.js";

test("terminalStageClickPlan preserves fallback, action, suppression, and focus decisions", () => {
  const action = { type: "action", actionId: "terminal" };
  const ignored = {
    type: "synthetic_click_ignored",
    preventDefault: true,
    handleAction: false,
    action: null,
    focusTerminal: false,
    focusMobileThenTerminal: false,
    suppressClick: false,
  };

  assert.deepEqual(terminalStageClickPlan({
    fallbackOwnsPointer: true,
    activeSheet: "",
  }), {
    type: "fallback_pointer",
    preventDefault: false,
    handleAction: false,
    action: null,
    focusTerminal: true,
    focusMobileThenTerminal: false,
    suppressClick: false,
  });
  assert.deepEqual(terminalStageClickPlan({
    fallbackOwnsPointer: true,
    activeSheet: "send",
  }).focusTerminal, false);
  assert.deepEqual(terminalStageClickPlan({
    hit: { action },
    ignoreSyntheticClick: true,
  }), ignored);
  assert.deepEqual(terminalStageClickPlan({
    hit: { action },
    ignoreSyntheticClick: false,
  }), {
    type: "surface_action",
    preventDefault: true,
    handleAction: true,
    action,
    focusTerminal: false,
    focusMobileThenTerminal: false,
    suppressClick: false,
  });
  assert.deepEqual(terminalStageClickPlan({
    hit: { consume: true },
    activeSheet: "",
  }), {
    type: "focus_terminal",
    preventDefault: false,
    handleAction: false,
    action: null,
    focusTerminal: true,
    focusMobileThenTerminal: false,
    suppressClick: false,
  });
  assert.deepEqual(terminalStageClickPlan({
    hit: {},
    activeSheet: "send",
  }).type, "ignore");
});

test("terminalStageTouchEndPlan preserves fallback, action, consume, and focus decisions", () => {
  const action = { type: "trogdor_agent", sessionId: "sess_1" };

  assert.deepEqual(terminalStageTouchEndPlan({
    fallbackOwnsPointer: true,
    hit: { action },
    activeSheet: "",
  }), {
    type: "fallback_pointer",
    preventDefault: false,
    handleAction: false,
    action: null,
    focusTerminal: false,
    focusMobileThenTerminal: false,
    suppressClick: false,
  });
  assert.deepEqual(terminalStageTouchEndPlan({
    hit: { action },
    activeSheet: "",
  }), {
    type: "surface_action",
    preventDefault: true,
    handleAction: true,
    action,
    focusTerminal: false,
    focusMobileThenTerminal: false,
    suppressClick: true,
  });
  assert.deepEqual(terminalStageTouchEndPlan({
    hit: { consume: true },
    activeSheet: "",
  }), {
    type: "consume",
    preventDefault: true,
    handleAction: false,
    action: null,
    focusTerminal: false,
    focusMobileThenTerminal: false,
    suppressClick: false,
  });
  assert.deepEqual(terminalStageTouchEndPlan({
    hit: {},
    activeSheet: "",
  }), {
    type: "focus_mobile_then_terminal",
    preventDefault: false,
    handleAction: false,
    action: null,
    focusTerminal: false,
    focusMobileThenTerminal: true,
    suppressClick: false,
  });
  assert.deepEqual(terminalStageTouchEndPlan({
    hit: {},
    activeSheet: "send",
  }).type, "ignore");
});

test("terminalStageMouseDownPlan preserves action, link, selection, and forwarding decisions", () => {
  const action = { type: "action", actionId: "terminal" };
  const ignored = {
    type: "fallback_pointer",
    preventDefault: false,
    suppressClick: false,
    handleAction: false,
    action: null,
    openHoveredLink: false,
    startSelection: false,
    completeSelection: false,
    forwardMouse: false,
    mouseKind: "down",
  };

  assert.deepEqual(terminalStageMouseDownPlan({ fallbackOwnsPointer: true, hit: { action } }), ignored);
  assert.deepEqual(terminalStageMouseDownPlan({ hit: { action }, hasTerminal: true }), {
    ...ignored,
    type: "surface_action",
    preventDefault: true,
    suppressClick: true,
    handleAction: true,
    action,
  });
  assert.deepEqual(terminalStageMouseDownPlan({ hit: { consume: true }, hasTerminal: true }), {
    ...ignored,
    type: "blocked",
    preventDefault: true,
  });
  assert.deepEqual(terminalStageMouseDownPlan({ hit: {}, hasTerminal: false }), {
    ...ignored,
    type: "blocked",
    preventDefault: true,
  });
  assert.deepEqual(terminalStageMouseDownPlan({
    hit: {},
    hasTerminal: true,
    modifierKey: true,
    hoveredLinkUrl: "https://example.test",
  }), {
    ...ignored,
    type: "link_modifier",
    preventDefault: true,
  });
  assert.deepEqual(terminalStageMouseDownPlan({
    hit: {},
    hasTerminal: true,
    selectMode: true,
    button: 0,
  }), {
    ...ignored,
    type: "select_start",
    preventDefault: true,
    startSelection: true,
  });
  assert.deepEqual(terminalStageMouseDownPlan({ hit: {}, hasTerminal: true, readOnly: true }), {
    ...ignored,
    type: "read_only",
  });
  assert.deepEqual(terminalStageMouseDownPlan({ hit: {}, hasTerminal: true, button: 2 }), {
    ...ignored,
    type: "forward_mouse",
    forwardMouse: true,
  });
});

test("terminalStageMouseUpPlan preserves blocked, link, selection, and forwarding decisions", () => {
  const action = { type: "action", actionId: "terminal" };
  const ignored = {
    type: "fallback_pointer",
    preventDefault: false,
    suppressClick: false,
    handleAction: false,
    action: null,
    openHoveredLink: false,
    startSelection: false,
    completeSelection: false,
    forwardMouse: false,
    mouseKind: "up",
  };

  assert.deepEqual(terminalStageMouseUpPlan({ fallbackOwnsPointer: true, hit: { action } }), ignored);
  assert.deepEqual(terminalStageMouseUpPlan({ hit: { action }, hasTerminal: true }), {
    ...ignored,
    type: "blocked",
    preventDefault: true,
  });
  assert.deepEqual(terminalStageMouseUpPlan({ hit: { consume: true }, hasTerminal: true }), {
    ...ignored,
    type: "blocked",
    preventDefault: true,
  });
  assert.deepEqual(terminalStageMouseUpPlan({ hit: {}, hasTerminal: false }), {
    ...ignored,
    type: "blocked",
  });
  assert.deepEqual(terminalStageMouseUpPlan({
    hit: {},
    hasTerminal: true,
    modifierKey: true,
    hoveredLinkUrl: "https://example.test",
  }), {
    ...ignored,
    type: "link_modifier",
    preventDefault: true,
    openHoveredLink: true,
  });
  assert.deepEqual(terminalStageMouseUpPlan({
    hit: {},
    hasTerminal: true,
    selectMode: true,
    selectionAnchor: 12,
    button: 0,
  }), {
    ...ignored,
    type: "select_complete",
    preventDefault: true,
    completeSelection: true,
  });
  assert.deepEqual(terminalStageMouseUpPlan({ hit: {}, hasTerminal: true, readOnly: true }), {
    ...ignored,
    type: "read_only",
  });
  assert.deepEqual(terminalStageMouseUpPlan({ hit: {}, hasTerminal: true, button: 2 }), {
    ...ignored,
    type: "forward_mouse",
    forwardMouse: true,
  });
});

test("terminalStageMouseMovePlan preserves hover, selection, read-only, and forwarding decisions", () => {
  const action = { type: "trogdor_agent", sessionId: "sess-1" };
  const ignored = {
    type: "fallback_pointer",
    preventDefault: false,
    updateTrogdorSurface: false,
    trogdorZone: null,
    clearHoveredLink: false,
    updateHoveredLink: false,
    updateSelectionRange: false,
    forwardMouse: false,
  };
  const hoverBase = {
    ...ignored,
    type: "hover_update",
    updateTrogdorSurface: true,
    trogdorZone: undefined,
  };

  assert.deepEqual(terminalStageMouseMovePlan({ fallbackOwnsPointer: true, hit: { action } }), ignored);
  assert.deepEqual(terminalStageMouseMovePlan({ hit: { consume: true }, hasTerminal: true }), {
    ...hoverBase,
    type: "blocked",
    clearHoveredLink: true,
  });
  assert.deepEqual(terminalStageMouseMovePlan({ hit: {}, hasTerminal: false }), {
    ...hoverBase,
    type: "blocked",
  });
  assert.deepEqual(terminalStageMouseMovePlan({
    hit: { action },
    hasTerminal: true,
    selectMode: true,
    selectionAnchor: 12,
    buttons: 1,
  }), {
    ...hoverBase,
    type: "select_drag",
    trogdorZone: action,
    preventDefault: true,
    updateSelectionRange: true,
  });
  assert.deepEqual(terminalStageMouseMovePlan({ hit: {}, hasTerminal: true, readOnly: true }), {
    ...hoverBase,
    type: "read_only",
    updateHoveredLink: true,
  });
  assert.deepEqual(terminalStageMouseMovePlan({
    hit: {},
    hasTerminal: true,
    selectMode: true,
    selectionAnchor: null,
    buttons: 1,
  }), {
    ...hoverBase,
    type: "forward_mouse",
    updateHoveredLink: true,
    forwardMouse: true,
  });
});

test("terminalStageWheelPlan preserves fallback, consume, blocked, and forwarding decisions", () => {
  const ignored = {
    type: "fallback_pointer",
    preventDefault: false,
    forwardWheel: false,
  };

  assert.deepEqual(terminalStageWheelPlan({ fallbackOwnsPointer: true, hit: { consume: true } }), ignored);
  assert.deepEqual(terminalStageWheelPlan({ hit: { consume: true }, hasTerminal: true }), {
    ...ignored,
    type: "consume",
    preventDefault: true,
  });
  assert.deepEqual(terminalStageWheelPlan({ hit: {}, hasTerminal: true, readOnly: true }), {
    ...ignored,
    type: "blocked",
  });
  assert.deepEqual(terminalStageWheelPlan({ hit: {}, hasTerminal: false }), {
    ...ignored,
    type: "blocked",
  });
  assert.deepEqual(terminalStageWheelPlan({ hit: {}, hasTerminal: true, selectMode: true }), {
    ...ignored,
    type: "blocked",
  });
  assert.deepEqual(terminalStageWheelPlan({ hit: {}, hasTerminal: true }), {
    ...ignored,
    type: "forward_wheel",
    preventDefault: true,
    forwardWheel: true,
  });
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
