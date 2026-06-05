import {
  normalizeNativeDesktopOpenResponse,
  normalizeNativeDesktopStatusResponse,
} from "./contracts.js";

export function formatNativeStatus(status) {
  if (!status) {
    return "Native status unavailable.";
  }
  if (!status.supported) {
    return `Native open unavailable: ${status.reason || "unsupported host"}`;
  }
  const app = status.app || status.app_id || "available";
  const mode = status.ghostty_mode ? ` / ${String(status.ghostty_mode).toLowerCase()}` : "";
  return `Native open ready: ${app}${mode}`;
}

export function currentNativeModeLabel(nativeDesktopState = {}) {
  const mode = nativeDesktopState.status?.ghostty_mode || nativeDesktopState.status?.ghosttyMode;
  if (!mode) {
    return "swap";
  }
  return String(mode).toLowerCase();
}

export function createNativeDesktopSheetController(runtime = {}) {
  const {
    state,
    el,
    apiFetch,
    currentSession = () => null,
    refreshSessions = async () => {},
    syncSheetActionAvailability = () => {},
  } = runtime;

  function setNativeResult(message, isError = false) {
    state.nativeDesktop.result = message;
    state.nativeDesktop.error = isError ? message : "";
    if (el.nativeStatusResult) {
      el.nativeStatusResult.textContent = message || "";
      el.nativeStatusResult.classList.toggle("error", Boolean(isError));
    }
  }

  function renderNativeStatusForm(status) {
    state.nativeDesktop.status = status;
    el.nativeApp.value = String(status?.app_id || status?.app || "iterm").toLowerCase();
    el.nativeMode.value = String(status?.ghostty_mode || "swap").toLowerCase();
    el.nativeMode.disabled = String(el.nativeApp.value) !== "ghostty";
    el.nativeStatusCopy.textContent = formatNativeStatus(status);
    const lines = [
      `supported: ${Boolean(status?.supported)}`,
      status?.platform ? `platform: ${status.platform}` : null,
      status?.reason ? `reason: ${status.reason}` : null,
      status?.app ? `app: ${status.app}` : null,
      status?.ghostty_mode ? `ghostty mode: ${String(status.ghostty_mode).toLowerCase()}` : null,
    ].filter(Boolean);
    setNativeResult(lines.join("\n"));
    syncSheetActionAvailability();
  }

  async function refreshNativeStatus() {
    state.nativeDesktop.loading = true;
    try {
      const response = await apiFetch("/v1/native/status");
      const payload = normalizeNativeDesktopStatusResponse(await response.json());
      renderNativeStatusForm(payload);
      setNativeResult(formatNativeStatus(payload));
    } catch (error) {
      setNativeResult(`Failed to load native status: ${error.message}`, true);
    } finally {
      state.nativeDesktop.loading = false;
      syncSheetActionAvailability();
    }
  }

  async function saveNativeSettings() {
    const app = String(el.nativeApp.value || "iterm");
    const mode = String(el.nativeMode.value || "swap");

    state.nativeDesktop.loading = true;
    setNativeResult("Saving native settings...");
    try {
      const appResponse = await apiFetch("/v1/native/app", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ app }),
      });
      const appPayload = normalizeNativeDesktopStatusResponse(await appResponse.json());
      renderNativeStatusForm(appPayload);

      if (app === "ghostty") {
        const modeResponse = await apiFetch("/v1/native/mode", {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ mode }),
        });
        const modePayload = normalizeNativeDesktopStatusResponse(await modeResponse.json());
        renderNativeStatusForm(modePayload);
      }

      setNativeResult(`Native settings saved: ${app}${app === "ghostty" ? ` / ${mode}` : ""}`);
      await refreshSessions();
    } catch (error) {
      setNativeResult(`Failed to save native settings: ${error.message}`, true);
    } finally {
      state.nativeDesktop.loading = false;
      syncSheetActionAvailability();
    }
  }

  async function openSelectedNativeSession() {
    const session = currentSession();
    if (!session) {
      return;
    }

    setNativeResult(`Opening ${session.session_id} in the native app...`);
    try {
      const response = await apiFetch("/v1/native/open", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: session.session_id }),
      });
      const payload = normalizeNativeDesktopOpenResponse(await response.json());
      setNativeResult(`Opened ${payload.session_id} in native app${payload.pane_id ? ` (${payload.pane_id})` : ""}.`);
    } catch (error) {
      setNativeResult(`Failed to open session natively: ${error.message}`, true);
    } finally {
      syncSheetActionAvailability();
    }
  }

  async function handleNativeFormSubmit(event) {
    event.preventDefault();
    await saveNativeSettings();
  }

  async function handleNativeRefreshButtonClick() {
    await refreshNativeStatus();
  }

  async function handleNativeOpenButtonClick() {
    await openSelectedNativeSession();
  }

  function handleNativeAppChange() {
    el.nativeMode.disabled = String(el.nativeApp.value).toLowerCase() !== "ghostty";
    syncSheetActionAvailability();
  }

  function handleNativeModeChange() {
    syncSheetActionAvailability();
  }

  function controllerCurrentNativeModeLabel() {
    return currentNativeModeLabel(state.nativeDesktop);
  }

  return {
    currentNativeModeLabel: controllerCurrentNativeModeLabel,
    handleNativeAppChange,
    handleNativeFormSubmit,
    handleNativeModeChange,
    handleNativeOpenButtonClick,
    handleNativeRefreshButtonClick,
    openSelectedNativeSession,
    refreshNativeStatus,
    renderNativeStatusForm,
    saveNativeSettings,
    setNativeResult,
  };
}
