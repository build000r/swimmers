export {
  surfaceActionDispatchContextPlan,
  surfaceActionDispatchPlan,
  surfaceActionExecutionContextPlan,
  surfaceActionExecutionPlan,
  surfaceActionFocusTerminalExecutionPlan,
  surfaceActionTrogdorReaderExecutionPlan,
} from "./surface_action_plans.js";
import { sessionUsesRemoteSnapshotFallback } from "./remote_session.js";

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

export function appEventListenerBindingPlan() {
  return {
    beforeTerminalStageCapture: [
      { target: "document", eventType: "keydown", handler: "handleDocumentCommandPaletteShortcut", optionalListener: true },
      { target: "terminalPalette", eventType: "click", handler: "handleTerminalPaletteClick" },
      { target: "terminalCopyFrame", eventType: "click", handler: "handleTerminalCopyFrameClick" },
      { target: "terminalLinkOpen", eventType: "click", handler: "handleTerminalLinkOpenClick" },
      { target: "terminalLinkCopy", eventType: "click", handler: "handleTerminalLinkCopyClick" },
      { target: "terminalZoomOut", eventType: "click", handler: "handleTerminalZoomOutClick" },
      { target: "terminalZoomReset", eventType: "click", handler: "handleTerminalZoomResetClick" },
      { target: "terminalZoomIn", eventType: "click", handler: "handleTerminalZoomInClick" },
      { target: "terminalMobileKeyboard", eventType: "click", handler: "handleTerminalMobileKeyboardClick" },
      { target: "terminalTrogdorBack", eventType: "click", handler: "handleTerminalTrogdorBackClick" },
      { target: "terminalWorkbenchToggle", eventType: "click", handler: "handleTerminalWorkbenchToggleClick" },
      { target: "terminalWorkbenchRefresh", eventType: "click", handler: "handleTerminalWorkbenchRefreshClick" },
      { target: "terminalWorkbenchWidgets", eventType: "click", handler: "handleTerminalWorkbenchWidgetsClick" },
      { target: "terminalWorkbenchWidgets", eventType: "input", handler: "handleTerminalWorkbenchWidgetsLogEvent" },
      { target: "terminalWorkbenchWidgets", eventType: "change", handler: "handleTerminalWorkbenchWidgetsLogEvent" },
      { target: "terminalInputDock", eventType: "submit", handler: "handleTerminalInputDockSubmit" },
      { target: "terminalInlineInput", eventType: "input", handler: "handleTerminalInlineInputInput" },
      { target: "terminalInlineInput", eventType: "keydown", handler: "handleTerminalInlineInputKeydown" },
      { target: "terminalKeyStrip", eventType: "click", handler: "handleTerminalKeyStripClick" },
      { target: "terminalInlineInput", eventType: "focus", handler: "handleTerminalInlineInputFocus" },
      { target: "terminalFallback", eventType: "mousedown", handler: "handleTerminalFallbackMousedown" },
      { target: "terminalFallback", eventType: "click", handler: "handleTerminalFallbackClick" },
      { target: "terminalFallback", eventType: "keydown", handler: "handleTerminalFallbackKeyEvent" },
      { target: "terminalFallback", eventType: "paste", handler: "handleTerminalFallbackPasteEvent" },
      { target: "terminalFallback", eventType: "focus", handler: "handleTerminalFallbackFocus" },
      { target: "terminalFallback", eventType: "blur", handler: "handleTerminalFallbackBlur" },
      { target: "terminalFallback", eventType: "scroll", handler: "handleTerminalFallbackScroll" },
      { target: "mobileKeyboardProxy", eventType: "focus", handler: "handleMobileKeyboardProxyFocus" },
      { target: "mobileKeyboardProxy", eventType: "blur", handler: "handleMobileKeyboardProxyBlur" },
      { target: "mobileKeyboardProxy", eventType: "keydown", handler: "handleMobileKeyboardProxyKeydown" },
      { target: "mobileKeyboardProxy", eventType: "input", handler: "handleMobileKeyboardProxyInput" },
      { target: "modalBackdrop", eventType: "click", handler: "closeSheets" },
      { target: "modalRoot", eventType: "keydown", handler: "handleModalRootKeydown" },
      { target: "paletteSearch", eventType: "input", handler: "handlePaletteSearchInput" },
      { target: "paletteSearch", eventType: "keydown", handler: "handleCommandPaletteEvent" },
      { target: "paletteResults", eventType: "mousemove", handler: "handleCommandPaletteEvent" },
      { target: "paletteResults", eventType: "click", handler: "handleCommandPaletteEvent" },
      { target: "paletteCloseButton", eventType: "click", handler: "closeSheets" },
      { target: "searchForm", eventType: "submit", handler: "handleSearchFormSubmit" },
      { target: "terminalSearch", eventType: "input", handler: "handleTerminalSearchInput" },
      { target: "searchPrevButton", eventType: "click", handler: "handleSearchPrevButtonClick" },
      { target: "searchNextButton", eventType: "click", handler: "handleSearchNextButtonClick" },
      { target: "searchClearButton", eventType: "click", handler: "handleSearchClearButtonClick" },
      { target: "searchCloseButton", eventType: "click", handler: "closeSheets" },
      { target: "sendMode", eventType: "change", handler: "handleSendModeChange" },
      { target: "thoughtConfigForm", eventType: "submit", handler: "handleThoughtConfigFormSubmit" },
      { target: "thoughtConfigBackend", eventType: "change", handler: "handleThoughtConfigBackendChange" },
      { target: "thoughtConfigModel", eventType: "input", handler: "handleThoughtConfigOptionChange" },
      { target: "thoughtConfigEnabled", eventType: "change", handler: "handleThoughtConfigOptionChange" },
      { target: "thoughtConfigTestButton", eventType: "click", handler: "handleThoughtConfigTestButtonClick" },
      { target: "thoughtConfigCloseButton", eventType: "click", handler: "closeSheets" },
      { target: "nativeForm", eventType: "submit", handler: "handleNativeFormSubmit" },
      { target: "nativeRefreshButton", eventType: "click", handler: "handleNativeRefreshButtonClick" },
      { target: "nativeOpenButton", eventType: "click", handler: "handleNativeOpenButtonClick" },
      { target: "nativeCloseButton", eventType: "click", handler: "closeSheets" },
      { target: "nativeApp", eventType: "change", handler: "handleNativeAppChange" },
      { target: "nativeMode", eventType: "change", handler: "handleNativeModeChange" },
      { target: "sendForm", eventType: "submit", handler: "handleSendFormSubmit" },
      { target: "sendCloseButton", eventType: "click", handler: "handleSendCloseButtonClick" },
      { target: "sendHistory", eventType: "click", handler: "handleSendHistoryClick" },
      { target: "saveTokenButton", eventType: "click", handler: "handleSaveTokenButtonClick" },
      { target: "clearTokenButton", eventType: "click", handler: "handleClearTokenButtonClick" },
      { target: "authCloseButton", eventType: "click", handler: "closeSheets" },
      { target: "createForm", eventType: "submit", handler: "handleCreateFormSubmit" },
      { target: "createCloseButton", eventType: "click", handler: "closeSheets" },
      { target: "createTool", eventType: "change", handler: "handleCreateToolChange" },
      { target: "createLaunchTarget", eventType: "change", handler: "handleCreateLaunchTargetChange" },
      { target: "createRequest", eventType: "input", handler: "handleCreateRequestInput" },
      { target: "dirsSearch", eventType: "input", handler: "handleDirsSearchInput" },
      { target: "createBatchVisible", eventType: "click", handler: "handleCreateBatchVisibleAction" },
      { target: "createBatchClear", eventType: "click", handler: "handleCreateBatchClearClick", optionalTarget: true },
      { target: "createCwd", eventType: "input", handler: "handleCreateCwdInput" },
      { target: "dirsManagedOnly", eventType: "change", handler: "handleDirsManagedOnlyChange" },
      { target: "dirsPath", eventType: "input", handler: "handleDirsPathInput" },
      { target: "dirsPath", eventType: "keydown", handler: "handleDirsPathKeydown" },
      { target: "dirsLoadButton", eventType: "click", handler: "handleDirsLoadButtonClick" },
      { target: "dirsSpawnHere", eventType: "click", handler: "handleDirsSpawnHereClick" },
      { target: "dirsUpButton", eventType: "click", handler: "handleDirsUpButtonClick" },
      { target: "dirsList", eventType: "change", handler: "handleDirCheckboxChange" },
      { target: "dirsList", eventType: "click", handler: "handleDirsListClick" },
      // Group-filter chips render into #dirs-groups (a sibling of #dirs-list),
      // so clicks there never bubble to the dirsList handler. Bind the same
      // handler (it delegates to the group-chip path first) to dirsGroups.
      { target: "dirsGroups", eventType: "click", handler: "handleDirsListClick", optionalTarget: true },
      { target: "mermaidRefreshButton", eventType: "click", handler: "handleMermaidRefreshButtonClick" },
      { target: "mermaidOpenButton", eventType: "click", handler: "handleMermaidOpenButtonClick" },
      { target: "mermaidPlanTabs", eventType: "click", handler: "handleMermaidPlanTabsClick" },
      { target: "mermaidCloseButton", eventType: "click", handler: "closeSheets" },
    ],
    afterTerminalStageCapture: [
      { target: "terminalStage", eventType: "click", handler: "handleTerminalStageClick" },
      { target: "terminalStage", eventType: "touchend", handler: "handleTerminalStageTouchEnd", options: { passive: false } },
      { target: "terminalStage", eventType: "keydown", handler: "handleTerminalStageKeydown" },
      { target: "terminalStage", eventType: "paste", handler: "handleTerminalStagePaste" },
      { target: "terminalStage", eventType: "focus", handler: "handleTerminalStageFocus" },
      { target: "terminalStage", eventType: "blur", handler: "handleTerminalStageBlur" },
      { target: "terminalStage", eventType: "mousedown", handler: "handleTerminalStageMouseDown" },
      { target: "terminalStage", eventType: "mouseup", handler: "handleTerminalStageMouseUp" },
      { target: "terminalStage", eventType: "mousemove", handler: "handleTerminalStageMouseMove" },
      { target: "terminalStage", eventType: "wheel", handler: "handleTerminalStageWheel", options: { passive: false } },
      { target: "terminalStage", eventType: "mouseleave", handler: "handleTerminalStageMouseleave" },
    ],
  };
}

function terminalStagePointerIgnore(type = "ignore") {
  return {
    type,
    preventDefault: false,
    handleAction: false,
    action: null,
    focusTerminal: false,
    focusMobileThenTerminal: false,
    suppressClick: false,
  };
}

export function terminalStageClickPlan(context = {}) {
  const hit = context.hit || {};
  if (context.fallbackOwnsPointer) {
    return {
      ...terminalStagePointerIgnore("fallback_pointer"),
      focusTerminal: !context.activeSheet,
    };
  }
  if (hit.action) {
    if (context.ignoreSyntheticClick) {
      return {
        ...terminalStagePointerIgnore("synthetic_click_ignored"),
        preventDefault: true,
      };
    }
    return {
      ...terminalStagePointerIgnore("surface_action"),
      preventDefault: true,
      handleAction: true,
      action: hit.action,
    };
  }
  return {
    ...terminalStagePointerIgnore(context.activeSheet ? "ignore" : "focus_terminal"),
    focusTerminal: !context.activeSheet,
  };
}

export function terminalStageTouchEndPlan(context = {}) {
  const hit = context.hit || {};
  if (context.fallbackOwnsPointer) {
    return terminalStagePointerIgnore("fallback_pointer");
  }
  if (hit.action) {
    return {
      ...terminalStagePointerIgnore("surface_action"),
      preventDefault: true,
      handleAction: true,
      action: hit.action,
      suppressClick: true,
    };
  }
  if (hit.consume) {
    return {
      ...terminalStagePointerIgnore("consume"),
      preventDefault: true,
    };
  }
  return {
    ...terminalStagePointerIgnore(context.activeSheet ? "ignore" : "focus_mobile_then_terminal"),
    focusMobileThenTerminal: !context.activeSheet,
  };
}

function terminalStageMouseIgnore(type = "ignore", mouseKind = "") {
  return {
    type,
    preventDefault: false,
    suppressClick: false,
    handleAction: false,
    action: null,
    openHoveredLink: false,
    startSelection: false,
    completeSelection: false,
    forwardMouse: false,
    mouseKind,
  };
}

export function terminalStageMouseDownPlan(context = {}) {
  const hit = context.hit || {};
  if (context.fallbackOwnsPointer) {
    return terminalStageMouseIgnore("fallback_pointer", "down");
  }
  if (hit.action) {
    return {
      ...terminalStageMouseIgnore("surface_action", "down"),
      preventDefault: true,
      suppressClick: true,
      handleAction: true,
      action: hit.action,
    };
  }
  if (hit.consume || !context.hasTerminal) {
    return { ...terminalStageMouseIgnore("blocked", "down"), preventDefault: true };
  }
  if (context.modifierKey && context.hoveredLinkUrl) {
    return { ...terminalStageMouseIgnore("link_modifier", "down"), preventDefault: true };
  }
  if (context.selectMode && context.button === 0) {
    return { ...terminalStageMouseIgnore("select_start", "down"), preventDefault: true, startSelection: true };
  }
  if (context.readOnly) {
    return terminalStageMouseIgnore("read_only", "down");
  }
  return { ...terminalStageMouseIgnore("forward_mouse", "down"), forwardMouse: true };
}

export function terminalStageMouseUpPlan(context = {}) {
  const hit = context.hit || {};
  if (context.fallbackOwnsPointer) {
    return terminalStageMouseIgnore("fallback_pointer", "up");
  }
  if (hit.action || hit.consume || !context.hasTerminal) {
    return {
      ...terminalStageMouseIgnore("blocked", "up"),
      preventDefault: Boolean(hit.action || hit.consume),
    };
  }
  if (context.modifierKey && context.hoveredLinkUrl) {
    return { ...terminalStageMouseIgnore("link_modifier", "up"), preventDefault: true, openHoveredLink: true };
  }
  if (context.selectMode && context.selectionAnchor !== null && context.button === 0) {
    return { ...terminalStageMouseIgnore("select_complete", "up"), preventDefault: true, completeSelection: true };
  }
  if (context.readOnly) {
    return terminalStageMouseIgnore("read_only", "up");
  }
  return { ...terminalStageMouseIgnore("forward_mouse", "up"), forwardMouse: true };
}

function terminalStageMouseMoveIgnore(type = "ignore") {
  return {
    type,
    preventDefault: false,
    updateTrogdorSurface: false,
    trogdorZone: null,
    clearHoveredLink: false,
    updateHoveredLink: false,
    updateSelectionRange: false,
    forwardMouse: false,
  };
}

export function terminalStageMouseMovePlan(context = {}) {
  const hit = context.hit || {};
  if (context.fallbackOwnsPointer) {
    return terminalStageMouseMoveIgnore("fallback_pointer");
  }
  const base = {
    ...terminalStageMouseMoveIgnore("hover_update"),
    updateTrogdorSurface: true,
    trogdorZone: hit.action,
  };
  if (hit.consume || !context.hasTerminal) {
    return { ...base, type: "blocked", clearHoveredLink: Boolean(hit.consume) };
  }
  if (context.selectMode && context.selectionAnchor !== null && (context.buttons & 1) === 1) {
    return { ...base, type: "select_drag", preventDefault: true, updateSelectionRange: true };
  }
  if (context.readOnly) {
    return { ...base, type: "read_only", updateHoveredLink: true };
  }
  return { ...base, type: "forward_mouse", updateHoveredLink: true, forwardMouse: true };
}

function terminalStageWheelIgnore(type = "ignore") {
  return {
    type,
    preventDefault: false,
    forwardWheel: false,
  };
}

export function terminalStageWheelPlan(context = {}) {
  const hit = context.hit || {};
  if (context.fallbackOwnsPointer) {
    return terminalStageWheelIgnore("fallback_pointer");
  }
  if (hit.consume) {
    return { ...terminalStageWheelIgnore("consume"), preventDefault: true };
  }
  if (context.readOnly || !context.hasTerminal || context.selectMode) {
    return terminalStageWheelIgnore("blocked");
  }
  return { ...terminalStageWheelIgnore("forward_wheel"), preventDefault: true, forwardWheel: true };
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

export function terminalSurfaceSessionPlan(context = {}) {
  const session = context.session || null;
  if (!session) {
    return { type: "teardown_terminal" };
  }
  if (sessionUsesRemoteSnapshotFallback(session)) {
    return { type: "activate_snapshot_fallback", sessionId: session.session_id, clearText: false };
  }
  return { type: "load_renderer", sessionId: session.session_id };
}

export function terminalSurfaceRendererPlan(context = {}) {
  if (!context.hasRendererModule) {
    return { type: "activate_snapshot_fallback", clearText: false };
  }
  if (context.hasTerminal && context.terminalSessionId === context.sessionId) {
    return {
      type: "reuse_terminal",
      terminalCanvasHidden: false,
      terminalFallbackHidden: !context.terminalFallbackActive,
      loadingVisible: false,
    };
  }
  return { type: "initialize_terminal", loadingVisible: true, loadingLabel: "Initializing terminal..." };
}

export function terminalSurfaceInitErrorPlan(message) {
  return {
    type: "renderer_error_fallback",
    clearText: false,
    refreshSnapshot: true,
    loadingVisible: false,
    status: `Live terminal renderer unavailable: ${message}`,
    statusError: true,
    statusTimeoutMs: 3600,
  };
}

export function terminalSurfacePostInitPlan(context = {}) {
  return {
    type: "complete_terminal_init",
    sessionId: context.sessionId,
    terminalPaintVerified: false,
    terminalFrameBytesSeen: 0,
    terminalFallbackActive: false,
    setLinkOpenPolicy: Boolean(context.linkPolicySupported),
    setAccessibility: Boolean(context.accessibilitySupported),
    accessibility: {
      reducedMotion: Boolean(context.reducedMotion),
      screenReader: true,
    },
    terminalCanvasHidden: false,
    clearSelection: true,
    refreshSearch: true,
    syncMirror: true,
    syncTools: true,
    resize: { pushResize: true, force: true },
    flushPendingBytes: true,
    loadingVisible: false,
  };
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
  // Guard the step divisor: a 0/undefined step would make numeric/step NaN and
  // propagate NaN into setZoom()/the resize path.
  const step = Number.isFinite(config.step) && config.step > 0 ? config.step : 0;
  const stepped = step > 0 ? Math.round(numeric / step) * step : numeric;
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

export function initialStateBootPlan(context = {}) {
  const param = (name, fallback = null) => context.searchParams?.get?.(name) ?? fallback;
  const queryToken = param("token", "");
  const selectedFromUrl = param("session");
  const presetFromUrl = param("preset", "") || param("fleet_preset", "");
  const selectedFromStorage = context.selectedFromStorage ?? null;
  const followFromUrl = param("follow") === "published";
  const rawStoredDirPath = String(context.rawStoredDirPath ?? "");
  const trimmedStoredDirPath = rawStoredDirPath.trim();
  const storedDirPath = trimmedStoredDirPath === "/" ? "" : trimmedStoredDirPath;
  const followPublishedSelection = Boolean(context.bootFollowPublishedSelection || followFromUrl);
  return {
    queryToken,
    tokenToPersist: queryToken || (context.storedToken ?? ""),
    selectedSessionId: followPublishedSelection ? null : selectedFromUrl || selectedFromStorage || null,
    followFromUrl,
    followPublishedSelection,
    storedDirPath,
    clearStoredDirPath: Boolean(rawStoredDirPath && !storedDirPath),
    storedManagedOnly: context.rawStoredManagedOnly === "true",
    storedFleetFilter: parseStoredFleetFilter(context.rawStoredFleetFilter),
    storedFleetPresetId: parseStoredFleetPresetId(presetFromUrl || context.rawStoredFleetPresetId),
    storedSessionGroupMode: parseStoredSessionGroupMode(context.rawStoredSessionGroupMode),
    terminalWorkbenchOpen: !Boolean(context.terminalWorkbenchMobile),
  };
}

function parseStoredFleetFilter(raw) {
  if (!raw) {
    return { kind: "", key: "" };
  }
  try {
    const parsed = JSON.parse(String(raw));
    const kind = String(parsed?.kind || "").trim().toLowerCase();
    const key = String(parsed?.key || "").trim();
    return kind && key ? { kind, key } : { kind: "", key: "" };
  } catch {
    return { kind: "", key: "" };
  }
}

function parseStoredFleetPresetId(raw) {
  return String(raw || "").trim().toLowerCase();
}

function parseStoredSessionGroupMode(raw) {
  return String(raw || "").trim().toLowerCase() === "project" ? "project" : "flat";
}

export function controlEventSessionPatchPlan(session = {}, message = {}) {
  const payload = message.payload && typeof message.payload === "object" ? message.payload : {};
  const event = String(message.event || "");
  const nextSession = { ...session, last_control_event: event };

  if (event === "session_state") {
    if (payload.state) nextSession.state = payload.state;
    if ("previous_state" in payload) nextSession.previous_state = payload.previous_state;
    if ("current_command" in payload) nextSession.current_command = payload.current_command;
    if (payload.state_evidence && typeof payload.state_evidence === "object") {
      nextSession.state_evidence = payload.state_evidence;
    }
    if (payload.transport_health) nextSession.transport_health = payload.transport_health;
    if (payload.exit_reason) nextSession.exit_reason = payload.exit_reason;
    if (payload.at) nextSession.last_activity_at = payload.at;
  } else if (event === "session_title") {
    const title = String(payload.title || "").trim();
    if (title) {
      nextSession.terminal_title = title;
      if (title.startsWith("/")) {
        nextSession.cwd = title;
      }
    }
  } else if (event === "session_skill") {
    if ("last_skill" in payload) {
      nextSession.last_skill = payload.last_skill;
    }
  } else if (event === "thought_update") {
    if ("thought" in payload) nextSession.thought = payload.thought;
    if ("token_count" in payload) nextSession.token_count = payload.token_count;
    if ("context_limit" in payload) nextSession.context_limit = payload.context_limit;
    if ("thought_state" in payload) nextSession.thought_state = payload.thought_state;
    if ("thought_source" in payload) nextSession.thought_source = payload.thought_source;
    if ("rest_state" in payload) nextSession.rest_state = payload.rest_state;
    if ("commit_candidate" in payload) nextSession.commit_candidate = Boolean(payload.commit_candidate);
    if (Array.isArray(payload.action_cues)) nextSession.action_cues = payload.action_cues;
    if (payload.at) nextSession.thought_updated_at = payload.at;
    if (payload.objective_changed && payload.at) nextSession.objective_changed_at = payload.at;
  }

  return { event, session: nextSession };
}

export function lifecycleDeletedSessionPatchPlan(session = {}, message = {}) {
  return {
    ...session,
    state: "exited",
    is_stale: true,
    transport_health: "disconnected",
    delete_reason: message.reason || "",
    delete_mode: message.deleteMode || message.delete_mode || "",
    tmux_session_alive: Boolean(message.tmuxSessionAlive ?? message.tmux_session_alive),
  };
}

export function inputAckActionPlan(message = {}) {
  const id = message.clientMessageId || message.client_message_id || "";
  if (!id) {
    return { action: "ignore", id: "", status: "", detail: "", expectedStatus: "", delayMs: 0 };
  }

  if (message.delivered) {
    return {
      action: "update",
      id,
      status: "sent",
      detail: message.method || "",
      expectedStatus: "sent",
      delayMs: 2500,
    };
  }

  return {
    action: "update",
    id,
    status: "failed",
    detail: message.message || "input delivery failed",
    expectedStatus: "failed",
    delayMs: 8000,
  };
}

export function sheetActionAvailabilityPlan(context = {}) {
  const writeDisabled = Boolean(context.writeDisabled);
  const hasSession = Boolean(context.hasSession);
  const nativeHandoffAvailable = Boolean(context.nativeHandoffAvailable);
  return {
    createButtonDisabled: writeDisabled || Boolean(context.singleBlocked) || (!context.batchReady && !context.hasSinglePath),
    createBatchSubmitDisabled: writeDisabled || !context.batchReady || Boolean(context.batchBlocked),
    createBatchVisibleDisabled: writeDisabled || context.visibleSelectableCount < 1,
    dirsSpawnHereDisabled: writeDisabled || !context.hasBrowserPath,
    thoughtConfigTestDisabled: writeDisabled || !context.hasThoughtConfig,
    thoughtConfigSaveDisabled: writeDisabled || !context.hasThoughtConfig,
    nativeSaveDisabled: writeDisabled || !context.hasNativeStatus,
    nativeOpenDisabled: writeDisabled || !hasSession || (!context.nativeSupported && !nativeHandoffAvailable),
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
