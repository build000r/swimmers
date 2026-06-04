function noop() {}

export function createTerminalStatusController(runtime) {
  const {
    state,
    el,
    boot = {},
    defaultDocumentTitle = "swimmers",
    currentSession,
    sessionDisplayName,
    sessionNeedsAttention,
    backendHealthWarningText,
    shortenUrl,
    renderHudSurface = noop,
    documentRef = globalThis.document,
    setTimeoutRef = globalThis.window?.setTimeout ?? globalThis.setTimeout,
    clearTimeoutRef = globalThis.clearTimeout,
    webSocketOpenReadyState = () => 1,
  } = runtime;

  function setConnectionStatus(label, muted = false) {
    state.connectionLabel = label;
    state.connectionMuted = Boolean(muted);
    syncTerminalStatusStrip();
    renderHudSurface();
  }

  function setModeStatus(label, muted = false) {
    state.modeLabel = label;
    state.modeMuted = Boolean(muted);
    syncTerminalStatusStrip();
    renderHudSurface();
  }

  function setSearchStatus(label, muted = false) {
    state.searchLabel = label;
    state.searchMuted = Boolean(muted);
    syncTerminalStatusStrip();
    renderHudSurface();
  }

  function terminalModeLabel() {
    if (!currentSession()) {
      return "no session";
    }
    if (state.terminalFallbackActive) {
      return state.ws?.readyState === webSocketOpenReadyState() ? "fallback live" : "snapshot fallback";
    }
    if (state.terminal) {
      return "FrankenTerm live";
    }
    return boot.franken_term_available ? "attaching renderer" : "snapshot mode";
  }

  function syncTerminalStatusStrip() {
    const session = currentSession();
    const pieces = [];
    if (session) {
      pieces.push(sessionDisplayName(session));
      pieces.push(String(session.state || "unknown"));
    }
    pieces.push(state.connectionLabel || "disconnected");
    pieces.push(state.readOnly ? "observer" : "operator");
    pieces.push(terminalModeLabel());
    if (state.searchQuery) {
      pieces.push(state.searchLabel || "search active");
    }
    if (state.selectMode) {
      pieces.push("selecting");
    }
    const healthWarning = backendHealthWarningText(state.backendHealth);
    if (healthWarning) {
      pieces.push(healthWarning);
    }
    if (el.terminalStatusStrip) {
      el.terminalStatusStrip.textContent = pieces.filter(Boolean).join("  |  ");
    }
    documentRef.body.classList.toggle("backend-health-degraded", Boolean(healthWarning));
    syncDocumentLifecycleSignal();
  }

  function applyBackendHealth(payload) {
    state.backendHealth = payload && typeof payload === "object" ? payload : null;
    syncTerminalStatusStrip();
    renderHudSurface();
  }

  function syncDocumentLifecycleSignal() {
    const session = currentSession();
    const attention = sessionNeedsAttention(session);
    documentRef.body.classList.toggle("session-attention", attention);
    if (attention && session) {
      documentRef.title = `(!) ${sessionDisplayName(session)} - swimmers`;
    } else {
      documentRef.title = defaultDocumentTitle;
    }
  }

  function clearUtilityStatusTimer() {
    if (state.utilityMessageTimer) {
      clearTimeoutRef(state.utilityMessageTimer);
      state.utilityMessageTimer = null;
    }
  }

  function defaultUtilityLabel() {
    return state.hoveredLinkUrl
      ? `Cmd/Ctrl-click to open ${shortenUrl(state.hoveredLinkUrl)}.`
      : "Cmd/Ctrl-click a terminal link to open it.";
  }

  function setUtilityStatus(label, muted = false, ttlMs = 0) {
    clearUtilityStatusTimer();
    state.utilityLabel = label;
    state.utilityMuted = Boolean(muted);
    renderHudSurface();
    if (ttlMs > 0) {
      state.utilityMessageTimer = setTimeoutRef(() => {
        setUtilityStatus(defaultUtilityLabel(), !state.hoveredLinkUrl);
      }, ttlMs);
    }
  }

  return {
    applyBackendHealth,
    clearUtilityStatusTimer,
    defaultUtilityLabel,
    setConnectionStatus,
    setModeStatus,
    setSearchStatus,
    setUtilityStatus,
    syncDocumentLifecycleSignal,
    syncTerminalStatusStrip,
    terminalModeLabel,
  };
}
