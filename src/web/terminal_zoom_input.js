import {
  normalizeTerminalZoomValue,
  terminalAuxiliaryControlsPlan,
  terminalInputDockPlan,
  terminalZoomControlsPlan,
  terminalZoomLoadValue,
  terminalZoomPercentLabel,
  terminalZoomPersistencePlan,
} from "./input_support.js";

export const DEFAULT_TERMINAL_ZOOM_STORAGE_KEY = "swimmers.web.terminalZoom";

export function createTerminalZoomInputController(runtime = {}) {
  const {
    state,
    el,
    storage = globalThis.localStorage,
    windowRef = globalThis.window,
    documentRef = globalThis.document,
    URLImpl = globalThis.URL,
    surfaceSupports = (surface, methodName) => typeof surface?.[methodName] === "function",
    terminalSupports = (methodName) => surfaceSupports(state?.terminal, methodName),
    currentSession = () => null,
    updateTerminalFallbackText = () => {},
    sendLineToSession = async () => {},
    rememberSendHistory = () => {},
    refreshSessions = () => {},
    setConnectionStatus = () => {},
    setUtilityStatus = () => {},
    measureAndResizeSurface = () => {},
    focusTerminalInputSurface = () => {},
    terminalZoomStorageKey = DEFAULT_TERMINAL_ZOOM_STORAGE_KEY,
    minZoom = 0.65,
    maxZoom = 2.4,
    step = 0.1,
  } = runtime;

  function zoomConfig() {
    return { minZoom, maxZoom, step };
  }

  function terminalZoomSupported() {
    return terminalSupports("setZoom") || surfaceSupports(state.hud, "setZoom");
  }

  function normalizeTerminalZoom(value) {
    return normalizeTerminalZoomValue(value, zoomConfig());
  }

  function loadTerminalZoom(url) {
    return terminalZoomLoadValue({
      urlZoom: url.searchParams.get("zoom"),
      storedZoom: storage.getItem(terminalZoomStorageKey),
    }, zoomConfig());
  }

  function syncTerminalZoomControls() {
    if (!el.terminalControlStrip) {
      return;
    }
    const plan = terminalZoomControlsPlan({
      zoomSupported: terminalZoomSupported(),
      hasTerminal: Boolean(state.terminal),
      zoom: state.terminalZoom,
      minZoom,
      maxZoom,
    });
    el.terminalZoomOut.disabled = plan.zoomOutDisabled;
    el.terminalZoomIn.disabled = plan.zoomInDisabled;
    el.terminalZoomReset.disabled = plan.zoomResetDisabled;
    el.terminalZoomReset.textContent = plan.zoomResetLabel;
    const auxiliaryPlan = terminalAuxiliaryControlsPlan({
      hasCurrentSession: Boolean(currentSession()),
      readOnly: state.readOnly,
      mobileKeyboardActive: state.mobileKeyboardActive,
      hasCopyFrame: Boolean(el.terminalCopyFrame),
    });
    el.terminalMobileKeyboard.disabled = auxiliaryPlan.mobileKeyboardDisabled;
    el.terminalMobileKeyboard.setAttribute("aria-pressed", auxiliaryPlan.mobileKeyboardAriaPressed);
    syncTerminalInputDock();
    if (auxiliaryPlan.copyFrameAvailable) el.terminalCopyFrame.disabled = auxiliaryPlan.copyFrameDisabled;
  }

  function syncTerminalInputDock() {
    if (!el.terminalInputDock) {
      return;
    }
    const plan = terminalInputDockPlan({
      hasCurrentSession: Boolean(currentSession()),
      trogdorAtlasOpen: state.trogdorAtlasOpen,
      readOnly: state.readOnly,
      inputValue: el.terminalInlineInput.value,
    });
    documentRef.body.classList.toggle("terminal-input-dock-visible", plan.visible);
    el.terminalInputDock.classList.toggle("hidden", plan.hidden);
    el.terminalInputDock.setAttribute("aria-hidden", plan.ariaHidden);
    el.terminalInlineInput.disabled = plan.inputDisabled;
    if (el.terminalKeyStrip) {
      for (const button of el.terminalKeyStrip.querySelectorAll("button[data-terminal-key]")) {
        button.disabled = plan.keyStripButtonDisabled;
      }
    }
    el.terminalInputSend.disabled = plan.sendDisabled;
  }

  function resizeTerminalInlineInput() {
    if (!el.terminalInlineInput) {
      return;
    }
    el.terminalInlineInput.style.height = "auto";
    const nextHeight = Math.max(40, Math.min(86, el.terminalInlineInput.scrollHeight || 40));
    el.terminalInlineInput.style.height = `${nextHeight}px`;
  }

  function setTerminalInputEcho(text) {
    if (!el.terminalInputEcho) {
      return;
    }
    const normalized = String(text || "").replace(/\r/g, "").replace(/\n+$/, "");
    el.terminalInputEcho.textContent = normalized ? `› ${normalized.replace(/\s+/g, " ")}` : "";
  }

  function projectTerminalInputIntoFallback(text) {
    if (!state.terminalFallbackActive || !el.terminalFallback) {
      return;
    }
    const normalized = String(text || "").replace(/\r/g, "").replace(/\n+$/, "");
    if (!normalized.trim()) {
      return;
    }
    const existing = el.terminalFallback.textContent || "";
    const separator = existing && !existing.endsWith("\n") ? "\n" : "";
    updateTerminalFallbackText(`${existing}${separator}› ${normalized}\n`);
  }

  async function submitTerminalInputDock() {
    if (state.readOnly || !currentSession()) {
      return false;
    }
    const text = String(el.terminalInlineInput.value || "");
    if (!text.trim()) {
      syncTerminalInputDock();
      return false;
    }
    setTerminalInputEcho(`pending: ${text}`);
    projectTerminalInputIntoFallback(text);
    try {
      await sendLineToSession(state.selectedSessionId, text);
      rememberSendHistory(text);
      el.terminalInlineInput.value = "";
      resizeTerminalInlineInput();
      syncTerminalInputDock();
      void refreshSessions();
      return true;
    } catch (error) {
      setTerminalInputEcho(`failed: ${error?.message || "input delivery failed"}`);
      setConnectionStatus("input failed; stream may be disconnected", true);
      return false;
    }
  }

  function applyZoomToSurface(surface) {
    if (surfaceSupports(surface, "setZoom")) {
      surface.setZoom(state.terminalZoom);
      return true;
    }
    return false;
  }

  function persistTerminalZoomToUrl(plan) {
    const url = new URLImpl(windowRef.location.href);
    if (plan.urlParamAction === "delete") url.searchParams.delete("zoom");
    else url.searchParams.set("zoom", plan.urlParamValue);
    windowRef.history.replaceState({}, "", url);
  }

  function applyTerminalZoom(options = {}) {
    const previous = state.terminalZoom;
    state.terminalZoom = normalizeTerminalZoom(state.terminalZoom);
    const changed = Math.abs(previous - state.terminalZoom) > 0.001;
    const applied = applyZoomToSurface(state.hud) || applyZoomToSurface(state.terminal);
    if (state.terminal) {
      applyZoomToSurface(state.terminal);
    }
    if (options.persist !== false) {
      const persistencePlan = terminalZoomPersistencePlan(state.terminalZoom);
      storage.setItem(terminalZoomStorageKey, persistencePlan.storageValue);
      persistTerminalZoomToUrl(persistencePlan);
    }
    syncTerminalZoomControls();
    if ((changed || options.forceResize) && (applied || state.terminal || state.hud)) {
      measureAndResizeSurface(true, true);
    }
    if (options.announce) {
      setUtilityStatus(`Terminal zoom ${terminalZoomPercentLabel(state.terminalZoom)}.`, false, 1600);
    }
  }

  function setTerminalZoom(nextZoom, options = {}) {
    state.terminalZoom = normalizeTerminalZoom(nextZoom);
    applyTerminalZoom(options);
  }

  function setTerminalZoomAndRefocus(nextZoom) {
    setTerminalZoom(nextZoom, { announce: true });
    focusTerminalInputSurface({ preventScroll: true });
  }

  function handleTerminalZoomOutClick() {
    setTerminalZoomAndRefocus(state.terminalZoom - step);
  }

  function handleTerminalZoomResetClick() {
    setTerminalZoomAndRefocus(1);
  }

  function handleTerminalZoomInClick() {
    setTerminalZoomAndRefocus(state.terminalZoom + step);
  }

  function handleTerminalInputDockSubmit(event) {
    event.preventDefault();
    void submitTerminalInputDock();
  }

  function handleTerminalInlineInputInput() {
    resizeTerminalInlineInput();
    syncTerminalInputDock();
  }

  return {
    terminalZoomSupported,
    normalizeTerminalZoom,
    loadTerminalZoom,
    syncTerminalZoomControls,
    syncTerminalInputDock,
    resizeTerminalInlineInput,
    setTerminalInputEcho,
    projectTerminalInputIntoFallback,
    submitTerminalInputDock,
    applyZoomToSurface,
    persistTerminalZoomToUrl,
    applyTerminalZoom,
    setTerminalZoom,
    setTerminalZoomAndRefocus,
    handleTerminalZoomOutClick,
    handleTerminalZoomResetClick,
    handleTerminalZoomInClick,
    handleTerminalInputDockSubmit,
    handleTerminalInlineInputInput,
  };
}
