import {
  sendHistoryClickPlan,
  sendSheetFailureStatus,
  sendSheetSubmitPlan,
  sendSheetSuccessStatus,
} from "./send_sheet.js";

export const DEFAULT_SEND_HISTORY_KEY = "swimmers.web.send.history";
export const DEFAULT_SEND_HISTORY_LIMIT = 8;

export function loadedSendHistory(storage, key = DEFAULT_SEND_HISTORY_KEY, limit = DEFAULT_SEND_HISTORY_LIMIT) {
  try {
    const parsed = JSON.parse(storage.getItem(key) || "[]");
    return Array.isArray(parsed)
      ? parsed.map((item) => String(item || "")).filter(Boolean).slice(0, limit)
      : [];
  } catch (_error) {
    return [];
  }
}

export function nextSendHistory(sendHistory = [], text = "", limit = DEFAULT_SEND_HISTORY_LIMIT) {
  const normalized = String(text || "").trim();
  if (!normalized) {
    return sendHistory.slice(0, limit);
  }
  return [
    normalized,
    ...sendHistory.filter((item) => item !== normalized),
  ].slice(0, limit);
}

export function sendHistoryHtml(sendHistory = [], escapeHtml = (value) => String(value)) {
  return sendHistory.slice(0, 6)
    .map((item, index) => {
      const label = String(item || "").replace(/\s+/g, " ").trim();
      const clipped = label.length > 42 ? `${label.slice(0, 39)}...` : label;
      return `<button class="ghost-button" type="button" data-send-history-index="${index}" title="${escapeHtml(label)}">${escapeHtml(clipped)}</button>`;
    })
    .join("");
}

export function deliveredGroupInputSessionIds(body, normalizeSessionId = (value) => String(value || "")) {
  if (!Array.isArray(body?.results)) {
    return [];
  }
  return body.results
    .filter((result) => result?.ok)
    .map((result) => normalizeSessionId(result?.session_id))
    .filter(Boolean);
}

export function sendModeValueFromElement(sendModeElement) {
  return String(sendModeElement?.value || "line") === "paste" ? "paste" : "line";
}

export function sendTargetReadyState({
  readOnly = false,
  sendTarget = null,
  currentSession = null,
  normalizeSessionId = (value) => String(value || ""),
} = {}) {
  if (readOnly) {
    return false;
  }
  if (!sendTarget) {
    return Boolean(currentSession);
  }
  if (sendTarget.type === "group") {
    return Array.isArray(sendTarget.sessionIds) && sendTarget.sessionIds.length >= 2;
  }
  return Boolean(normalizeSessionId(sendTarget.sessionId));
}

export function createSendController(runtime = {}) {
  const {
    state,
    el,
    apiFetch,
    responseJsonOrNull,
    currentSession,
    normalizeSessionId,
    nextInputMessageId,
    updateInputDeliveryStatus,
    sendTerminalText,
    setTerminalInputEcho,
    markTrogdorSessionsResponded,
    setUtilityStatus,
    closeSheets,
    openSheet,
    refreshSessions,
    syncSheetActionAvailability,
    escapeHtml,
    storage = globalThis.localStorage,
    WebSocketClass = globalThis.WebSocket,
    sendHistoryKey = DEFAULT_SEND_HISTORY_KEY,
    sendHistoryLimit = DEFAULT_SEND_HISTORY_LIMIT,
    ElementClass = globalThis.Element,
  } = runtime;

  function sendModeValue() {
    return sendModeValueFromElement(el.sendMode);
  }

  async function sendLine(text) {
    return sendLineToSession(state.selectedSessionId, text);
  }

  function loadSendHistory() {
    state.sendHistory = loadedSendHistory(storage, sendHistoryKey, sendHistoryLimit);
  }

  function saveSendHistory() {
    storage.setItem(sendHistoryKey, JSON.stringify(state.sendHistory.slice(0, sendHistoryLimit)));
  }

  function rememberSendHistory(text) {
    if (!String(text || "").trim()) {
      return;
    }
    state.sendHistory = nextSendHistory(state.sendHistory, text, sendHistoryLimit);
    saveSendHistory();
    renderSendHistory();
  }

  function renderSendHistory() {
    if (!el.sendHistory) {
      return;
    }
    el.sendHistory.innerHTML = sendHistoryHtml(state.sendHistory, escapeHtml);
  }

  async function sendLineToSession(sessionId, text) {
    const targetSessionId = normalizeSessionId(sessionId);
    if (!text || !targetSessionId) {
      return;
    }

    if (
      state.ws &&
      WebSocketClass &&
      state.ws.readyState === WebSocketClass.OPEN &&
      !state.readOnly &&
      state.selectedSessionId === targetSessionId
    ) {
      const clientMessageId = nextInputMessageId();
      state.pendingInputMessages.set(clientMessageId, { text, status: "pending", detail: "" });
      updateInputDeliveryStatus(clientMessageId, "pending");
      state.ws.send(JSON.stringify({ type: "submit_line", data: text, clientMessageId }));
      markTrogdorSessionsResponded([targetSessionId]);
      return;
    }

    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(targetSessionId)}/input`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text, submit: true }),
    });
    const body = await responseJsonOrNull(response);
    if (body?.delivered === false) {
      throw new Error(body.message || "input delivery failed");
    }
    setTerminalInputEcho(`sent: ${text}`);
    markTrogdorSessionsResponded([targetSessionId]);
  }

  async function sendRawTextToSession(sessionId, text) {
    const targetSessionId = normalizeSessionId(sessionId);
    if (!text || !targetSessionId) {
      return;
    }
    if (
      state.ws &&
      WebSocketClass &&
      state.ws.readyState === WebSocketClass.OPEN &&
      !state.readOnly &&
      state.selectedSessionId === targetSessionId
    ) {
      sendTerminalText(text);
      return;
    }
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(targetSessionId)}/input`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text }),
    });
    const body = await responseJsonOrNull(response);
    if (body?.delivered === false) {
      throw new Error(body.message || "input delivery failed");
    }
  }

  async function sendGroupLine(sessionIds, text) {
    const ids = Array.isArray(sessionIds)
      ? sessionIds.map(normalizeSessionId).filter(Boolean)
      : [];
    if (!text || ids.length < 2) {
      return;
    }

    const response = await apiFetch("/v1/sessions/group-input", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_ids: ids, text }),
    });
    const body = await responseJsonOrNull(response).catch(() => null);
    const deliveredSessionIds = deliveredGroupInputSessionIds(body, normalizeSessionId);
    markTrogdorSessionsResponded(deliveredSessionIds);

    const resultTotal = Array.isArray(body?.results) ? body.results.length : ids.length;
    return {
      delivered: deliveredSessionIds.length,
      skipped: Math.max(0, resultTotal - deliveredSessionIds.length),
      total: resultTotal,
      deliveredSessionIds,
      results: Array.isArray(body?.results) ? body.results : [],
    };
  }

  function updateSendHint() {
    if (!el.sendHint) {
      return;
    }
    if (state.sendTarget?.type === "group") {
      el.sendHint.textContent = "Batch sends submit the shared text to every ready agent.";
      return;
    }
    el.sendHint.textContent = sendModeValue() === "paste"
      ? "Paste only preserves text exactly for the selected live terminal."
      : "Send submits the text to the selected agent prompt.";
  }

  async function handleSendFormSubmit(event) {
    event.preventDefault();
    // Re-entrancy guard: the input is only cleared on success, so a second
    // submit while the send is in flight would resend the same line.
    if (state.sending) {
      return false;
    }
    const plan = sendSheetSubmitPlan({
      readOnly: state.readOnly,
      text: el.sendInput.value,
      sendTarget: state.sendTarget,
      selectedSessionId: state.selectedSessionId,
      sendMode: sendModeValue(),
    });
    if (plan.type === "ignore") {
      return false;
    }
    state.sending = true;
    try {
      rememberSendHistory(plan.text);
      const result = plan.type === "group"
        ? await sendGroupLine(plan.sessionIds, plan.text)
        : await (plan.type === "paste" ? sendRawTextToSession : sendLineToSession)(plan.sessionId, plan.text);
      const status = sendSheetSuccessStatus(plan, result);
      setUtilityStatus(status.label, status.muted, status.ttlMs);
      el.sendInput.value = "";
      state.sendTarget = null;
      closeSheets();
      await refreshSessions();
      return true;
    } catch (error) {
      const status = sendSheetFailureStatus(error);
      setUtilityStatus(status.label, status.muted, status.ttlMs);
      syncSheetActionAvailability();
      return false;
    } finally {
      state.sending = false;
    }
  }

  function sendTargetReady() {
    return sendTargetReadyState({
      readOnly: state.readOnly,
      sendTarget: state.sendTarget,
      currentSession: currentSession(),
      normalizeSessionId,
    });
  }

  function openSendSheet(target = null) {
    state.sendTarget = target;
    const label = target?.label || currentSession()?.tmux_name || currentSession()?.session_id || "selected session";
    if (el.sendSheetTitle) {
      el.sendSheetTitle.textContent = target?.type === "group" ? "Send Batch" : "Send To Terminal";
    }
    if (el.sendMode) {
      el.sendMode.value = "line";
      el.sendMode.disabled = target?.type === "group";
    }
    el.sendInput.value = "";
    el.sendInput.placeholder =
      target?.type === "group"
        ? `Send to ${Array.isArray(target.sessionIds) ? target.sessionIds.length : 0} batch agents.`
        : `Send to ${label}.`;
    renderSendHistory();
    updateSendHint();
    openSheet("send");
    syncSheetActionAvailability();
  }

  function handleSendHistoryClick(event) {
    const target = ElementClass && event.target instanceof ElementClass ? event.target : null;
    const plan = sendHistoryClickPlan(event.type, target, state.sendHistory);
    if (plan.type === "use_history") {
      el.sendInput.value = plan.text;
      el.sendInput.focus();
    }
  }

  return {
    handleSendFormSubmit,
    handleSendHistoryClick,
    loadSendHistory,
    openSendSheet,
    rememberSendHistory,
    renderSendHistory,
    saveSendHistory,
    sendGroupLine,
    sendLine,
    sendLineToSession,
    sendModeValue,
    sendRawTextToSession,
    sendTargetReady,
    updateSendHint,
  };
}
