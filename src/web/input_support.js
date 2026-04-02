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

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}
