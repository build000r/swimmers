import {
  isImeCompositionKeydown,
  terminalFallbackFocusPlan,
  terminalFallbackKeydownPlan,
  terminalFallbackPastePlan,
  terminalFallbackPointerFocusPlan,
  terminalFallbackScrollPlan,
  terminalStageFocusExecutorPlan,
  terminalStageFocusPlan,
} from "./input_support.js";

export function createTerminalFocusController(runtime = {}) {
  const {
    state,
    el,
    documentRef = globalThis.document,
    windowRef = globalThis.window,
    requestAnimationFrameRef = globalThis.requestAnimationFrame,
    currentSession,
    forwardTerminalEvent,
    forwardTerminalKeyDown,
    handleGlobalShortcut,
    keyBeginsTrogdorResponse,
    markTrogdorSessionsResponded,
    sendTerminalText,
    terminalFallbackIsNearBottom,
  } = runtime;

  function isCoarsePointer() {
    return windowRef.matchMedia?.("(pointer: coarse)")?.matches ?? false;
  }

  function syncMobileKeyboardState() {
    documentRef.body.classList.toggle("mobile-keyboard-active", state.mobileKeyboardActive);
    if (el.terminalMobileKeyboard) {
      el.terminalMobileKeyboard.setAttribute("aria-pressed", state.mobileKeyboardActive ? "true" : "false");
    }
  }

  function focusMobileKeyboard() {
    if (state.readOnly || !currentSession()) {
      return false;
    }
    state.mobileKeyboardActive = true;
    syncMobileKeyboardState();
    el.mobileKeyboardProxy.value = "";
    el.mobileKeyboardProxy.focus({ preventScroll: true });
    forwardTerminalEvent({ kind: "focus", focused: true });
    return true;
  }

  function terminalInputSurfaceHasFocus() {
    const active = documentRef.activeElement;
    return !active || active === documentRef.body || active === el.terminalStage || active === el.terminalFallback;
  }

  function focusTerminalInputSurface(options = {}) {
    if (state.activeSheet && !options.force) {
      return false;
    }
    if (options.onlyIfSurfaceFocused && !terminalInputSurfaceHasFocus()) {
      return false;
    }
    const target = state.terminalFallbackActive ? el.terminalFallback : el.terminalStage;
    if (!target || typeof target.focus !== "function") {
      return false;
    }
    target.focus({ preventScroll: Boolean(options.preventScroll) });
    return documentRef.activeElement === target;
  }

  function closeMobileKeyboard() {
    state.mobileKeyboardActive = false;
    syncMobileKeyboardState();
    if (documentRef.activeElement === el.mobileKeyboardProxy) {
      el.mobileKeyboardProxy.blur();
    }
  }

  function shouldCaptureKey(event) {
    if (!currentSession() || state.readOnly || state.activeSheet) {
      return false;
    }
    // Never capture intermediate IME composition keydowns — the committed text
    // is delivered through the input/compositionend path instead.
    if (isImeCompositionKeydown(event)) {
      return false;
    }
    if (event.metaKey) {
      return false;
    }
    return true;
  }

  function handleTerminalFallbackKeyEvent(event) {
    const fallbackActive = state.terminalFallbackActive;
    const globalShortcutHandled = fallbackActive && handleGlobalShortcut(event);
    const shouldCaptureTerminalKey = fallbackActive && !globalShortcutHandled && shouldCaptureKey(event);
    const plan = terminalFallbackKeydownPlan({
      terminalFallbackActive: fallbackActive,
      globalShortcutHandled,
      shouldCaptureKey: shouldCaptureTerminalKey,
      beginsResponse: shouldCaptureTerminalKey && keyBeginsTrogdorResponse(event),
    });
    if (plan.preventDefault) event.preventDefault();
    if (plan.stopPropagation) event.stopPropagation?.();
    if (plan.markResponse) markTrogdorSessionsResponded([state.selectedSessionId]);
    if (plan.forwardKey) forwardTerminalKeyDown(event);
    return plan.handled;
  }

  function handleTerminalFallbackPasteEvent(event) {
    const plan = terminalFallbackPastePlan({
      terminalFallbackActive: state.terminalFallbackActive, readOnly: state.readOnly,
      hasCurrentSession: Boolean(currentSession()), text: event.clipboardData?.getData("text") ?? "",
    });
    if (plan.preventDefault) event.preventDefault();
    if (plan.stopPropagation) event.stopPropagation?.();
    if (plan.sendText) sendTerminalText(plan.text);
    return plan.handled;
  }

  function runTerminalFocusAction(plan) {
    const action = terminalStageFocusExecutorPlan(plan);
    if (action.forwardEvent) forwardTerminalEvent(action.event);
  }

  function runTerminalFallbackPointerFocusAction(plan) {
    if (!plan.focusTerminal) return;
    const focus = () => focusTerminalInputSurface({ preventScroll: true });
    if (plan.scheduleFrame) requestAnimationFrameRef(focus);
    else focus();
  }

  function handleTerminalInlineInputFocus() {
    runTerminalFocusAction(terminalStageFocusPlan("focus", { activeSheet: state.activeSheet }));
  }

  function handleTerminalFallbackPointerFocus(eventType) {
    runTerminalFallbackPointerFocusAction(terminalFallbackPointerFocusPlan(eventType, { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet }));
  }

  function handleTerminalFallbackFocusEvent(eventType) {
    runTerminalFocusAction(terminalFallbackFocusPlan(eventType, eventType === "focus" ? { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet } : { terminalFallbackActive: state.terminalFallbackActive, mobileKeyboardOwnsFocus: documentRef.activeElement === el.mobileKeyboardProxy }));
  }

  function handleTerminalFallbackScroll() {
    const plan = terminalFallbackScrollPlan("scroll", { terminalFallbackActive: state.terminalFallbackActive, nearBottom: state.terminalFallbackActive ? terminalFallbackIsNearBottom() : false });
    if (plan.updateAutoFollow) state.terminalFallbackAutoFollow = plan.autoFollow;
  }

  function handleMobileKeyboardProxyFocusEvent(focused) {
    state.mobileKeyboardActive = focused;
    syncMobileKeyboardState();
    forwardTerminalEvent({ kind: "focus", focused });
  }

  function handleTerminalFallbackMousedown() {
    handleTerminalFallbackPointerFocus("mousedown");
  }

  function handleTerminalFallbackClick() {
    handleTerminalFallbackPointerFocus("click");
  }

  function handleTerminalFallbackFocus() {
    handleTerminalFallbackFocusEvent("focus");
  }

  function handleTerminalFallbackBlur() {
    handleTerminalFallbackFocusEvent("blur");
  }

  function handleTerminalStageFocusEvent(eventType) {
    runTerminalFocusAction(terminalStageFocusPlan(eventType, eventType === "focus" ? { activeSheet: state.activeSheet } : { mobileKeyboardOwnsFocus: documentRef.activeElement === el.mobileKeyboardProxy }));
  }

  return {
    closeMobileKeyboard,
    focusMobileKeyboard,
    focusTerminalInputSurface,
    handleMobileKeyboardProxyFocusEvent,
    handleTerminalFallbackBlur,
    handleTerminalFallbackClick,
    handleTerminalFallbackFocus,
    handleTerminalFallbackKeyEvent,
    handleTerminalFallbackMousedown,
    handleTerminalFallbackPasteEvent,
    handleTerminalFallbackScroll,
    handleTerminalInlineInputFocus,
    handleTerminalStageFocusEvent,
    isCoarsePointer,
    runTerminalFallbackPointerFocusAction,
    runTerminalFocusAction,
    shouldCaptureKey,
    syncMobileKeyboardState,
    terminalInputSurfaceHasFocus,
  };
}
