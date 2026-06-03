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
