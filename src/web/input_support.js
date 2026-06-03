export function eventClientPoint(event) {
  if (Number.isFinite(event?.clientX) && Number.isFinite(event?.clientY)) {
    return {
      x: event.clientX,
      y: event.clientY,
    };
  }

  const touch = event?.changedTouches?.[0] ?? event?.touches?.[0] ?? null;
  if (touch && Number.isFinite(touch.clientX) && Number.isFinite(touch.clientY)) {
    return {
      x: touch.clientX,
      y: touch.clientY,
    };
  }

  return {
    x: 0,
    y: 0,
  };
}

export function eventCell(event, rect, cols, rows) {
  const point = eventClientPoint(event);
  const safeCols = Math.max(1, cols);
  const safeRows = Math.max(1, rows);
  const cellWidth = Math.max(1, rect.width / safeCols);
  const cellHeight = Math.max(1, rect.height / safeRows);

  return {
    x: clampInt(Math.floor((point.x - rect.left) / cellWidth), 0, 0, safeCols - 1),
    y: clampInt(Math.floor((point.y - rect.top) / cellHeight), 0, 0, safeRows - 1),
  };
}

export function shouldIgnoreSyntheticClick(nowMs, suppressUntilMs) {
  return Number.isFinite(suppressUntilMs) && nowMs <= suppressUntilMs;
}

export function authTokenButtonPlan(action, tokenValue = "") {
  switch (action) {
    case "save":
      return { type: "persist", token: String(tokenValue ?? ""), resetReadOnly: false };
    case "clear":
      return { type: "persist", token: "", resetReadOnly: true };
    default:
      return { type: "ignore" };
  }
}

export function globalShortcutPlan(event, context = {}) {
  if ((event?.ctrlKey || event?.metaKey) && !event?.altKey) {
    switch (event.code) {
      case "KeyK":
        return { type: "open_palette" };
      case "Equal":
      case "NumpadAdd":
        return { type: "zoom_in" };
      case "Minus":
      case "NumpadSubtract":
        return { type: "zoom_out" };
      case "Digit0":
      case "Numpad0":
        return { type: "zoom_reset" };
      default:
        break;
    }
  }

  if (event?.key === "Escape") {
    if (context.activeSheet) {
      return { type: "close_sheets" };
    }
    if (context.trogdorAtlasOpen) {
      return { type: "close_trogdor_atlas" };
    }
    if (context.selectMode) {
      return { type: "exit_select_mode" };
    }
    return { type: "unhandled" };
  }

  if (!(event?.ctrlKey && event?.shiftKey) || event?.metaKey || event?.altKey) {
    return { type: "unhandled" };
  }

  switch (event.code) {
    case "KeyF":
      return { type: "open_sheet", sheetId: "search" };
    case "KeyS":
      return context.readOnly || !context.hasCurrentSession
        ? { type: "handled" }
        : { type: "open_sheet", sheetId: "send" };
    case "KeyA":
      return { type: "open_sheet", sheetId: "auth" };
    case "KeyT":
      return { type: "open_thought_config" };
    case "KeyO":
      return { type: "open_native" };
    case "KeyN":
      return context.readOnly
        ? { type: "handled" }
        : { type: "open_sheet", sheetId: "create" };
    case "KeyM":
      return { type: "open_mermaid" };
    case "KeyP":
      return { type: "toggle_follow" };
    case "KeyV":
      return { type: "toggle_select" };
    case "KeyC":
      return { type: "copy_selection" };
    case "KeyL":
      return context.hoveredLinkUrl ? { type: "copy_hovered_link" } : { type: "handled" };
    case "KeyR":
      return { type: "refresh_sessions" };
    default:
      return { type: "unhandled" };
  }
}

const MOBILE_KEYBOARD_SPECIAL_KEYS = new Set([
  "Backspace",
  "Delete",
  "Enter",
  "Tab",
  "Escape",
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "Home",
  "End",
  "PageUp",
  "PageDown",
]);

export function mobileKeyboardKeyPlan(event, context = {}) {
  if (context.readOnly || !context.hasCurrentSession) {
    return { type: "ignore" };
  }
  if (!MOBILE_KEYBOARD_SPECIAL_KEYS.has(event?.key)) {
    return { type: "ignore" };
  }
  if (event.key === "Escape") {
    return { type: "close_mobile_keyboard" };
  }
  return { type: "forward_key" };
}

export function mobileKeyboardKeydownPlan(context = {}) {
  if (context.globalShortcutHandled) {
    return { type: "prevent_default", handled: true, preventDefault: true, closeKeyboard: false, focusTerminal: false, markResponse: false, forwardKey: false };
  }
  if (context.keyPlan?.type === "ignore") {
    return { type: "ignore", handled: false, preventDefault: false, closeKeyboard: false, focusTerminal: false, markResponse: false, forwardKey: false };
  }
  if (context.keyPlan?.type === "close_mobile_keyboard") {
    return { type: "close_mobile_keyboard", handled: true, preventDefault: true, closeKeyboard: true, focusTerminal: true, markResponse: false, forwardKey: false };
  }
  if (context.keyPlan?.type !== "forward_key") {
    return { type: "ignore", handled: false, preventDefault: false, closeKeyboard: false, focusTerminal: false, markResponse: false, forwardKey: false };
  }
  return {
    type: "forward_key",
    handled: true,
    preventDefault: true,
    closeKeyboard: false,
    focusTerminal: false,
    markResponse: Boolean(context.beginsResponse),
    forwardKey: true,
  };
}

function mobileKeyboardInputKeyEvent(key) {
  return {
    kind: "key",
    phase: "down",
    key,
    code: key,
    mods: 0,
    repeat: false,
  };
}

export function mobileKeyboardInputPlan(event, context = {}) {
  if (context.readOnly || !context.hasCurrentSession) {
    return { type: "clear" };
  }
  const inputType = String(event?.inputType || "");
  const text = typeof event?.data === "string" ? event.data : String(context.proxyValue ?? "");
  if (inputType === "deleteContentBackward") {
    return { type: "forward_event", event: mobileKeyboardInputKeyEvent("Backspace") };
  }
  if (inputType === "insertLineBreak") {
    return { type: "forward_event", event: mobileKeyboardInputKeyEvent("Enter") };
  }
  return { type: "send_text", text };
}

export function mobileKeyboardInputExecutorPlan(plan = {}) {
  if (plan.type === "clear") {
    return { type: "ignore", handled: false, forwardEvent: null, sendText: false, text: "" };
  }
  if (plan.type === "forward_event") {
    return { type: "forward_event", handled: true, forwardEvent: plan.event, sendText: false, text: "" };
  }
  if (plan.type === "send_text") {
    return { type: "send_text", handled: true, forwardEvent: null, sendText: true, text: plan.text };
  }
  return { type: "ignore", handled: false, forwardEvent: null, sendText: false, text: "" };
}

export function terminalKeyStripClickPlan(eventType, target) {
  if (eventType !== "click") {
    return { type: "ignore" };
  }
  const button = target?.closest?.("button[data-terminal-key]") ?? null;
  if (!button || button.disabled) {
    return { type: "ignore" };
  }
  return { type: "send_key", actionId: button.dataset.terminalKey };
}

export function terminalKeyStripClickExecutorPlan(plan = {}) {
  if (plan.type !== "send_key") {
    return { type: "ignore", preventDefault: false, sendKey: false, actionId: "" };
  }
  return { type: "send_key", preventDefault: true, sendKey: true, actionId: plan.actionId };
}

export function terminalComposerControlAction(event, context = {}) {
  if (!event || event.metaKey || event.altKey) {
    return "";
  }
  const key = String(event.key || "");
  if (event.ctrlKey && key.toLowerCase() === "c") {
    return context.hasSelection ? "" : "ctrl-c";
  }
  if (String(context.inputValue || "").length > 0) {
    return "";
  }
  switch (key) {
    case "Escape":
      return "escape";
    case "Tab":
      return "tab";
    case "ArrowUp":
      return "arrow-up";
    case "ArrowDown":
      return "arrow-down";
    case "ArrowLeft":
      return "arrow-left";
    case "ArrowRight":
      return "arrow-right";
    case "Home":
      return "home";
    case "End":
      return "end";
    case "PageUp":
      return "page-up";
    case "PageDown":
      return "page-down";
    default:
      return "";
  }
}

export function terminalInlineInputKeydownPlan(event, actionId = "") {
  const normalizedActionId = String(actionId || "");
  if (event?.key === "Enter" && !event?.shiftKey) {
    return {
      type: "submit",
      handled: true,
      preventDefault: true,
      stopPropagation: true,
      submit: true,
      sendKey: false,
      actionId: "",
    };
  }
  if (normalizedActionId) {
    return {
      type: "send_key",
      handled: true,
      preventDefault: true,
      stopPropagation: true,
      submit: false,
      sendKey: true,
      actionId: normalizedActionId,
    };
  }
  return {
    type: "ignore",
    handled: false,
    preventDefault: false,
    stopPropagation: true,
    submit: false,
    sendKey: false,
    actionId: "",
  };
}

export function terminalStageCaptureBindings() {
  return [
    { eventType: "mousedown", action: "down", options: { capture: true } },
    { eventType: "click", action: "click", options: { capture: true } },
    { eventType: "touchend", action: "touch", options: { capture: true, passive: false } },
    { eventType: "wheel", action: "wheel", options: { capture: true, passive: false } },
  ];
}

export function terminalStagePastePlan(readOnly, text) {
  if (readOnly || !text) {
    return { type: "ignore" };
  }
  return { type: "send_text", text };
}

export function terminalStagePasteExecutorPlan(plan = {}) {
  if (plan.type !== "send_text") {
    return { type: "ignore", preventDefault: false, sendText: false, text: "" };
  }
  return { type: "send_text", preventDefault: true, sendText: true, text: plan.text };
}

export function terminalFallbackPastePlan(context = {}) {
  if (!context.terminalFallbackActive || context.readOnly || !context.hasCurrentSession || !context.text) {
    return { type: "ignore", handled: false, preventDefault: false, stopPropagation: false, sendText: false, text: "" };
  }
  return {
    type: "send_text",
    handled: true,
    preventDefault: true,
    stopPropagation: true,
    sendText: true,
    text: context.text,
  };
}

export function terminalStageFocusPlan(eventType, context = {}) {
  if (eventType === "focus") {
    return context.activeSheet
      ? { type: "ignore" }
      : { type: "forward_event", event: { kind: "focus", focused: true } };
  }
  if (eventType === "blur") {
    return context.mobileKeyboardOwnsFocus
      ? { type: "ignore" }
      : { type: "forward_event", event: { kind: "focus", focused: false } };
  }
  return { type: "ignore" };
}

export function terminalFallbackFocusPlan(eventType, context = {}) {
  if (!context.terminalFallbackActive) {
    return { type: "ignore" };
  }
  if (eventType === "focus") {
    return context.activeSheet
      ? { type: "ignore" }
      : { type: "forward_event", event: { kind: "focus", focused: true } };
  }
  if (eventType === "blur") {
    return context.mobileKeyboardOwnsFocus
      ? { type: "ignore" }
      : { type: "forward_event", event: { kind: "focus", focused: false } };
  }
  return { type: "ignore" };
}

export function terminalFallbackActivationPlan(context = {}) {
  const terminalFallbackActive = Boolean(context.active && context.hasCurrentSession);
  return {
    type: terminalFallbackActive ? "activate" : "deactivate",
    terminalFallbackActive,
    hidden: !terminalFallbackActive,
    ariaHidden: terminalFallbackActive ? "false" : "true",
    updateAutoFollow: terminalFallbackActive,
    autoFollow: terminalFallbackActive ? (context.wasActive ? context.nearBottom : true) : null,
    startSnapshotPolling: terminalFallbackActive,
    focusTerminal: terminalFallbackActive && !context.wasActive,
    clearText: !terminalFallbackActive && context.clearText !== false,
    stopSnapshotPolling: !terminalFallbackActive && (context.hasTerminal || !context.hasCurrentSession),
    syncStatus: true,
  };
}

export function terminalPendingByteBufferPlan(context = {}) {
  const pendingChunkByteLengths = Array.isArray(context.pendingChunkByteLengths) ? context.pendingChunkByteLengths : [];
  const currentLength = Number.isFinite(context.pendingByteLength) ? context.pendingByteLength : 0;
  const byteLength = Number.isFinite(context.byteLength) ? context.byteLength : 0;
  if (!context.isUint8Array || byteLength === 0) {
    return { type: "ignore", accept: false, dropCount: 0, finalPendingByteLength: currentLength, status: "" };
  }
  const maxPendingBytes = Number.isFinite(context.maxPendingBytes) ? context.maxPendingBytes : Number.POSITIVE_INFINITY;
  let finalPendingByteLength = currentLength + byteLength;
  let dropCount = 0;
  while (finalPendingByteLength > maxPendingBytes && pendingChunkByteLengths.length + 1 - dropCount > 1) {
    finalPendingByteLength -= pendingChunkByteLengths[dropCount] || 0;
    dropCount += 1;
  }
  return { type: "buffer", accept: true, dropCount, finalPendingByteLength, status: "buffering terminal; renderer attaching" };
}

export function terminalPresentationPlan(context = {}) {
  const terminalFocusMode = Boolean(context.hasCurrentSession && !context.trogdorAtlasOpen);
  return {
    terminalFocusMode,
    terminalStageActive: terminalFocusMode,
    hudHidden: terminalFocusMode,
    hudDisplay: terminalFocusMode ? "none" : "",
    hudVisibility: terminalFocusMode ? "hidden" : "",
    showTerminalCanvas: Boolean(context.hasTerminal),
    terminalCanvasHidden: false,
    terminalCanvasDisplay: "",
    terminalCanvasVisibility: "",
    terminalFallbackHidden: !(terminalFocusMode && context.terminalFallbackActive),
  };
}

export function terminalPaintProbeSchedulePlan(context = {}) {
  const scheduleProbe = Boolean(
    !context.terminalPaintVerified &&
      !context.terminalFallbackActive &&
      !context.hasProbeTimer &&
      context.hasTerminal &&
      context.hasCurrentSession &&
      context.terminalFrameBytesSeen !== 0,
  );
  return { type: scheduleProbe ? "schedule_probe" : "ignore", scheduleProbe, delayMs: 180 };
}

export function terminalPaintVerificationPlan(context = {}) {
  if (!context.hasTerminal || context.terminalPaintVerified || context.terminalFallbackActive || !context.hasCurrentSession) {
    return { type: "ignore", done: true };
  }
  if (typeof context.canvasHasVisiblePixels !== "boolean") {
    return { type: "check_canvas", done: false };
  }
  if (context.canvasHasVisiblePixels) {
    return { type: "painted", done: true, fallbackActive: false, diagnosticReason: "painted" };
  }
  if (!context.afterSnapshotRefresh) {
    return { type: "refresh_snapshot", done: false };
  }
  if (context.hasSnapshotText) {
    return { type: "activate_fallback", done: true, fallbackActive: true, clearText: false, syncPresentation: true };
  }
  return { type: "ignore", done: true };
}

export function terminalResizeGeometryPlan(context = {}) {
  const cols = clampInt(context.cols, 80, 24, 240);
  const rows = clampInt(context.rows, 24, 12, 120);
  const dimensionsChanged = cols !== context.currentCols || rows !== context.currentRows;
  const shouldResize = Boolean(context.force || dimensionsChanged);
  return {
    cols,
    rows,
    dimensionsChanged,
    shouldResize,
    sendResize: Boolean(context.pushResize && shouldResize),
    captureDiagnostic: Boolean(context.hasTerminal && shouldResize),
    diagnosticReason: "resize",
  };
}

export function terminalLiveFrameFallbackPlan(context = {}) {
  if (!context.terminalFallbackActive || !context.hasTerminal) {
    return { type: "ignore", update: false, text: "", preserveExistingFallback: false };
  }
  const liveText = String(context.liveText || "");
  const liveTextHasContent = textHasContent(liveText);
  const existingFallbackHasContent = textHasContent(context.existingFallbackText);
  if (!liveTextHasContent) {
    return { type: "ignore", update: false, text: "", preserveExistingFallback: existingFallbackHasContent };
  }
  return { type: "update", update: true, text: liveText, preserveExistingFallback: false };
}

export function terminalInputDockPlan(context = {}) {
  const visible = Boolean(context.hasCurrentSession && !context.trogdorAtlasOpen);
  const controlsDisabled = !visible || Boolean(context.readOnly);
  const hasText = Boolean(String(context.inputValue || "").trim());
  return {
    visible,
    hidden: !visible,
    ariaHidden: visible ? "false" : "true",
    inputDisabled: controlsDisabled,
    keyStripButtonDisabled: controlsDisabled,
    sendDisabled: controlsDisabled || !hasText,
  };
}

export function terminalZoomPercentLabel(zoom) {
  return `${Math.round(zoom * 100)}%`;
}

export function normalizeTerminalZoomValue(value, config = {}) {
  const numeric = Number.parseFloat(value);
  if (!Number.isFinite(numeric)) {
    return 1;
  }
  const stepped = Math.round(numeric / config.step) * config.step;
  return Math.max(config.minZoom, Math.min(config.maxZoom, stepped));
}

export function terminalZoomLoadValue(context = {}, config = {}) {
  return normalizeTerminalZoomValue(context.urlZoom !== null ? context.urlZoom : context.storedZoom || "1", config);
}

export function terminalZoomControlsPlan(context = {}) {
  const supported = Boolean(context.zoomSupported || !context.hasTerminal);
  return {
    supported,
    zoomOutDisabled: !supported || context.zoom <= context.minZoom + 0.001,
    zoomInDisabled: !supported || context.zoom >= context.maxZoom - 0.001,
    zoomResetDisabled: !supported || Math.abs(context.zoom - 1) < 0.001,
    zoomResetLabel: terminalZoomPercentLabel(context.zoom),
  };
}

export function terminalZoomPersistencePlan(zoom) {
  const zoomValue = zoom.toFixed(2);
  const deleteUrlParam = Math.abs(zoom - 1) < 0.001;
  return {
    storageValue: zoomValue,
    urlParamAction: deleteUrlParam ? "delete" : "set",
    urlParamValue: deleteUrlParam ? "" : zoomValue,
  };
}

export function terminalAuxiliaryControlsPlan(context = {}) {
  const hasCurrentSession = Boolean(context.hasCurrentSession);
  const copyFrameAvailable = Boolean(context.hasCopyFrame);
  return {
    mobileKeyboardDisabled: Boolean(context.readOnly) || !hasCurrentSession,
    mobileKeyboardAriaPressed: context.mobileKeyboardActive ? "true" : "false",
    copyFrameAvailable,
    copyFrameDisabled: !copyFrameAvailable || !hasCurrentSession,
  };
}

export function terminalToolsAvailabilityPlan(context = {}) {
  let searchStatus = null;
  if (!context.liveTerminal) {
    searchStatus = {
      label: context.frankenTermAvailable ? "Search waits for terminal attach" : "Search needs FrankenTerm assets",
      muted: true,
    };
  } else if (!context.searchReady) {
    searchStatus = { label: "Search unavailable in this FrankenTerm build", muted: true };
  } else if (!context.searchQuery) {
    searchStatus = { label: "Search idle", muted: true };
  }
  return {
    searchDisabled: !context.searchReady,
    sendInputDisabled: Boolean(context.readOnly),
    sendModeDisabled: Boolean(context.readOnly) || context.sendTargetType === "group",
    sendSubmitDisabled: Boolean(context.readOnly) || !context.hasCurrentSession,
    createFormElementsDisabled: Boolean(context.readOnly),
    searchStatus,
  };
}

export function sheetActionAvailabilityPlan(context = {}) {
  const writeDisabled = Boolean(context.writeDisabled);
  const hasSession = Boolean(context.hasSession);
  return {
    createButtonDisabled: writeDisabled || (!context.batchReady && !context.hasSinglePath),
    createBatchSubmitDisabled: writeDisabled || !context.batchReady,
    createBatchVisibleDisabled: writeDisabled || context.visibleSelectableCount < 1,
    dirsSpawnHereDisabled: writeDisabled || !context.hasBrowserPath,
    thoughtConfigTestDisabled: writeDisabled || !context.hasThoughtConfig,
    thoughtConfigSaveDisabled: writeDisabled || !context.hasThoughtConfig,
    nativeSaveDisabled: writeDisabled || !context.hasNativeStatus,
    nativeOpenDisabled: writeDisabled || !hasSession || !context.nativeSupported,
    nativeRefreshDisabled: false,
    mermaidOpenDisabled: writeDisabled || !hasSession || !context.hasMermaidPath,
    mermaidRefreshDisabled: !hasSession,
    dirsLoadDisabled: !context.hasDirsPath,
    dirsUpDisabled: !context.hasParentDir,
    sendModeDisabled: writeDisabled || context.sendTargetType === "group",
    sendSubmitDisabled: writeDisabled || !context.sendTargetReady,
  };
}

export function terminalFallbackPointerFocusPlan(eventType, context = {}) {
  if (!context.terminalFallbackActive || context.activeSheet) {
    return { type: "ignore", focusTerminal: false, scheduleFrame: false };
  }
  if (eventType === "mousedown") {
    return { type: "focus_terminal", focusTerminal: true, scheduleFrame: true };
  }
  if (eventType === "click") {
    return { type: "focus_terminal", focusTerminal: true, scheduleFrame: false };
  }
  return { type: "ignore", focusTerminal: false, scheduleFrame: false };
}

export function terminalFallbackScrollPlan(eventType, context = {}) {
  if (eventType !== "scroll" || !context.terminalFallbackActive || typeof context.nearBottom !== "boolean") {
    return { type: "ignore", updateAutoFollow: false, autoFollow: null };
  }
  return { type: "set_auto_follow", updateAutoFollow: true, autoFollow: context.nearBottom };
}

export function terminalFallbackTextScrollPlan(context = {}) {
  if (context.terminalFallbackAutoFollow || context.nearBottom) {
    return { type: "follow", scrollTop: context.scrollHeight };
  }
  return {
    type: "preserve",
    scrollTop: Math.min(context.previousScrollTop, Math.max(0, context.scrollHeight - context.clientHeight)),
  };
}

export function terminalDestroyStatePatch() {
  return {
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
}

export function terminalStageFocusExecutorPlan(plan = {}) {
  if (plan.type !== "forward_event") {
    return { type: "ignore", forwardEvent: false, event: null };
  }
  return { type: "forward_event", forwardEvent: true, event: plan.event };
}

export function terminalStageKeydownPlan(context = {}) {
  if (context.globalShortcutHandled) {
    return { type: "prevent_default", preventDefault: true, markResponse: false, forwardKey: false };
  }
  if (!context.shouldCaptureKey) {
    return { type: "ignore", preventDefault: false, markResponse: false, forwardKey: false };
  }
  return {
    type: "forward_key",
    preventDefault: true,
    markResponse: Boolean(context.beginsResponse),
    forwardKey: true,
  };
}

export function terminalFallbackKeydownPlan(context = {}) {
  if (!context.terminalFallbackActive) {
    return {
      type: "ignore",
      handled: false,
      preventDefault: false,
      stopPropagation: false,
      markResponse: false,
      forwardKey: false,
    };
  }
  const plan = terminalStageKeydownPlan(context);
  return {
    ...plan,
    handled: plan.type !== "ignore",
    stopPropagation: Boolean(plan.preventDefault),
  };
}

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

function textHasContent(text) {
  return /\S/.test(String(text || ""));
}
