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

export function filterCommandPaletteItems(items = [], query = "", limit = 18) {
  const normalizedQuery = String(query || "").trim().toLowerCase();
  return (Array.isArray(items) ? items : [])
    .map((item) => ({ ...item, score: commandPaletteScore(item, normalizedQuery) }))
    .filter((item) => !normalizedQuery || item.score > 0)
    .sort((a, b) => b.score - a.score || a.label.localeCompare(b.label))
    .slice(0, limit);
}

export function filteredCommandPaletteItemsForState({
  selectedSession = null,
  readOnly = false,
  sessions = [],
  copyFrameAction = null,
  query = "",
  limit = 18,
} = {}) {
  return filterCommandPaletteItems(
    buildCommandPaletteItems({ selectedSession, readOnly, sessions, copyFrameAction }),
    query,
    limit,
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
