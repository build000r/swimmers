export function commandPaletteSessionDisplayName(session) {
  return String(session?.tmux_name || session?.name || session?.session_id || "session");
}

function escapeHtml(text) {
  return String(text || "").replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      case "'":
        return "&#39;";
      default:
        return char;
    }
  });
}

export function buildCommandPaletteItems({
  selectedSession = null,
  readOnly = false,
  sessions = [],
  copyFrameAction = null,
} = {}) {
  const selected = Boolean(selectedSession);
  const baseItems = [
    { label: "Focus terminal", meta: "terminal", actionId: "focus_terminal", disabled: !selected },
    { label: "Search terminal", meta: "Ctrl+Shift+F", actionId: "open_search", disabled: !selected },
    { label: "Send to terminal", meta: "Ctrl+Shift+S", actionId: "open_send", disabled: readOnly || !selected },
    { label: "Copy selection", meta: "Ctrl+Shift+C", actionId: "copy_selection", disabled: !selected },
    { label: "Copy visible text", meta: "frame", action: copyFrameAction, disabled: !selected },
    { label: "Toggle select mode", meta: "Ctrl+Shift+V", actionId: "toggle_select", disabled: !selected },
    { label: "Open native terminal", meta: "desktop", actionId: "open_native", disabled: !selected },
    { label: "Open Mermaid artifacts", meta: "artifacts", actionId: "open_mermaid", disabled: !selected },
    { label: "Create session", meta: "spawn", actionId: "open_create", disabled: readOnly },
    { label: "Refresh sessions", meta: "sync", actionId: "refresh" },
    { label: "Toggle follow published", meta: "selection", actionId: "toggle_follow" },
    { label: "Thought config", meta: "policy", actionId: "open_config" },
    { label: "Auth token", meta: "connection", actionId: "open_auth" },
    { label: "Toggle Trogdor atlas", meta: "overview", actionId: "toggle_trogdor_atlas" },
  ];
  const sessionItems = (Array.isArray(sessions) ? sessions : []).map((session) => ({
    label: `Switch to ${commandPaletteSessionDisplayName(session)}`,
    meta: `${session?.session_id}  ${session?.state || ""}`,
    sessionId: session?.session_id,
  }));
  return [...baseItems, ...sessionItems];
}

export function commandPaletteScore(item, query) {
  const haystack = `${item?.label || ""} ${item?.meta || ""}`.toLowerCase();
  if (!query) {
    return 1;
  }
  const exact = haystack.indexOf(query);
  if (exact >= 0) {
    return 1000 - exact;
  }
  let score = 0;
  let cursor = 0;
  for (const char of query) {
    const next = haystack.indexOf(char, cursor);
    if (next < 0) {
      return 0;
    }
    score += Math.max(1, 40 - (next - cursor));
    cursor = next + 1;
  }
  return score;
}

export function commandPaletteRecencyKey(item) {
  return String(item?.sessionId || item?.actionId || item?.label || "");
}

function commandPaletteRankCompare(a, b) {
  // Query relevance dominates; recency is a tie-breaker that also orders the
  // empty-query list "recently-used first" (frecency) without ever overriding a
  // stronger query match.
  return (
    b.score - a.score || (b.recency || 0) - (a.recency || 0) || a.label.localeCompare(b.label)
  );
}

function scoreCommandPaletteItem(item, normalizedQuery, recency) {
  return {
    ...item,
    score: commandPaletteScore(item, normalizedQuery),
    recency: recency[commandPaletteRecencyKey(item)] || 0,
  };
}

function sortedCommandPaletteItems(source, normalizedQuery, recency) {
  return source
    .map((item) => scoreCommandPaletteItem(item, normalizedQuery, recency))
    .filter((item) => !normalizedQuery || item.score > 0)
    .sort(commandPaletteRankCompare);
}

function insertBoundedCommandPaletteItem(ranked, item, limit) {
  let index = ranked.length;
  while (index > 0 && commandPaletteRankCompare(item, ranked[index - 1]) < 0) {
    index -= 1;
  }
  if (index >= limit) {
    return;
  }
  ranked.splice(index, 0, item);
  if (ranked.length > limit) {
    ranked.pop();
  }
}

export function filterCommandPaletteItems(items = [], query = "", limit = 18, recency = {}) {
  const normalizedQuery = String(query || "").trim().toLowerCase();
  const source = Array.isArray(items) ? items : [];
  const recencyMap = recency && typeof recency === "object" ? recency : {};
  if (!Number.isInteger(limit)) {
    return sortedCommandPaletteItems(source, normalizedQuery, recencyMap).slice(0, limit);
  }
  if (limit <= 0) {
    return [];
  }
  const ranked = [];
  for (const item of source) {
    const scoredItem = scoreCommandPaletteItem(item, normalizedQuery, recencyMap);
    if (normalizedQuery && scoredItem.score <= 0) {
      continue;
    }
    insertBoundedCommandPaletteItem(ranked, scoredItem, limit);
  }
  return ranked;
}

/// Record that a palette item was executed, returning a new recency map with the
/// item bumped to the most-recent rank. The map is bounded to `limit` keys so it
/// cannot grow without bound across a long session.
export function recordCommandPaletteUse(recency = {}, item, limit = 50) {
  const key = commandPaletteRecencyKey(item);
  if (!key) {
    return recency && typeof recency === "object" ? recency : {};
  }
  const base = recency && typeof recency === "object" ? recency : {};
  const values = Object.values(base);
  const nextRank = (values.length ? Math.max(...values) : 0) + 1;
  const next = { ...base, [key]: nextRank };
  const keys = Object.keys(next);
  if (keys.length <= limit) {
    return next;
  }
  const kept = keys.sort((a, b) => next[b] - next[a]).slice(0, limit);
  const capped = {};
  for (const keptKey of kept) {
    capped[keptKey] = next[keptKey];
  }
  return capped;
}

export function filteredCommandPaletteItemsForState({
  selectedSession = null,
  readOnly = false,
  sessions = [],
  copyFrameAction = null,
  query = "",
  limit = 18,
  recency = {},
} = {}) {
  return filterCommandPaletteItems(
    buildCommandPaletteItems({ selectedSession, readOnly, sessions, copyFrameAction }),
    query,
    limit,
    recency,
  );
}

export function commandPaletteExecutionPlan(item) {
  if (!item || item.disabled) {
    return { type: "none" };
  }
  if (item.sessionId) {
    return { type: "selectSession", sessionId: item.sessionId };
  }
  if (typeof item.action === "function") {
    return { type: "invokeAction", action: item.action };
  }
  if (item.actionId) {
    return { type: "dispatchAction", actionId: item.actionId };
  }
  return { type: "none" };
}

function nextEnabledPaletteIndex(from, step, count, isDisabled) {
  if (count <= 0) {
    return 0;
  }
  let i = from + step;
  while (i >= 0 && i < count) {
    if (!isDisabled(i)) {
      return i;
    }
    i += step;
  }
  // No enabled item in that direction: hold position rather than landing on a
  // disabled item or running off the ends.
  return Math.max(0, Math.min(count - 1, from));
}

// `items` accepts the palette item array (so arrow nav can skip disabled items)
// or a bare count for back-compatible callers that have no disabled metadata.
export function commandPaletteSearchKeyPlan(event, activeIndex = 0, items = 0) {
  const index = Number.isFinite(activeIndex) ? Math.trunc(activeIndex) : 0;
  const list = Array.isArray(items) ? items : [];
  const count = Array.isArray(items)
    ? items.length
    : Number.isFinite(items)
      ? Math.trunc(items)
      : 0;
  const isDisabled = (i) => Boolean(list[i]?.disabled);
  if (event?.key === "ArrowDown") {
    return {
      type: "set_index",
      index: nextEnabledPaletteIndex(index, 1, count, isDisabled),
      preventDefault: true,
    };
  }
  if (event?.key === "ArrowUp") {
    return {
      type: "set_index",
      index: nextEnabledPaletteIndex(index, -1, count, isDisabled),
      preventDefault: true,
    };
  }
  return event?.key === "Enter"
    ? { type: "run_item", preventDefault: true }
    : { type: "ignore" };
}

function boundedPaletteResultIndex(rawIndex, itemCount = 0) {
  const maxIndex = Math.max(0, (Number.isFinite(itemCount) ? Math.trunc(itemCount) : 0) - 1);
  const index = Number.isFinite(rawIndex) ? Math.trunc(rawIndex) : 0;
  return Math.max(0, Math.min(maxIndex, index));
}

export function commandPaletteResultEventPlan(eventType, target, itemCount = 0) {
  const item = target?.closest?.("[data-palette-index]");
  if (!item) {
    return { type: "ignore" };
  }
  const index = boundedPaletteResultIndex(Number(item.dataset?.paletteIndex), itemCount);
  return eventType === "click"
    ? { type: "run_item", index }
    : { type: "set_index", index };
}

export function renderCommandPaletteResultsHtml(items = [], activeIndex = 0) {
  if (!items.length) {
    return `<div class="sheet-copy">No matching commands.</div>`;
  }
  return items
    .map((item, index) => `
      <button
        class="palette-item${index === activeIndex ? " is-active" : ""}"
        type="button"
        role="option"
        aria-selected="${index === activeIndex ? "true" : "false"}"
        data-palette-index="${index}"
        ${item.disabled ? "disabled" : ""}
      >
        <span class="palette-item-title">${escapeHtml(item.label)}</span>
        <span class="palette-item-meta">${escapeHtml(item.disabled ? "unavailable" : item.meta || "")}</span>
      </button>
    `)
    .join("");
}
