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

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}
