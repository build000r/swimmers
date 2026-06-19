import {
  normalizeNativeDesktopOpenResponse,
  normalizeNativeDesktopStatusResponse,
} from "./contracts.js";
import { responseJson as defaultResponseJson } from "./api_client.js";

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

function nonEmptyString(...values) {
  for (const value of values) {
    const text = String(value ?? "").trim();
    if (text) {
      return text;
    }
  }
  return "";
}

function sessionEnvironment(session) {
  return session && typeof session.environment === "object" && session.environment
    ? session.environment
    : {};
}

function environmentSummaryForSession(session, environments = []) {
  const targetId = nonEmptyString(
    sessionEnvironment(session).target_id,
    splitRemoteSessionId(session)?.targetId,
  );
  if (!targetId || !Array.isArray(environments)) {
    return null;
  }
  return environments.find((environment) => String(environment?.id || "") === targetId) ?? null;
}

function backendModeLabel(value) {
  const text = nonEmptyString(value, "remote");
  if (text === "remote_swimmers_api") {
    return "remote Swimmers API";
  }
  return text.replace(/[_-]+/g, " ");
}

function splitRemoteSessionId(session) {
  const sessionId = nonEmptyString(session?.session_id);
  const separatorIndex = sessionId.indexOf("::");
  if (separatorIndex <= 0 || separatorIndex + 2 >= sessionId.length) {
    return null;
  }
  return {
    targetId: sessionId.slice(0, separatorIndex),
    sessionId: sessionId.slice(separatorIndex + 2),
  };
}

function handoffCwdLine(session, environment) {
  const remoteCwd = nonEmptyString(environment.remote_cwd);
  if (remoteCwd) {
    return `remote cwd: ${remoteCwd}`;
  }
  const localCwd = nonEmptyString(environment.local_cwd);
  if (localCwd) {
    return `local mapped cwd: ${localCwd}`;
  }
  const cwd = nonEmptyString(environment.canonical_cwd, session?.cwd);
  return cwd ? `cwd: ${cwd}` : "";
}

export function remoteNativeHandoffAvailable(session) {
  const scope = String(sessionEnvironment(session).scope || "local").trim().toLowerCase();
  return scope === "remote" || Boolean(splitRemoteSessionId(session));
}

export function remoteNativeHandoffMessage(session, environments = []) {
  if (!remoteNativeHandoffAvailable(session)) {
    return "";
  }
  const environment = sessionEnvironment(session);
  const remoteSession = splitRemoteSessionId(session);
  const summary = environmentSummaryForSession(session, environments);
  const targetLabel = nonEmptyString(
    environment.display_host,
    environment.target_label,
    summary?.label,
    environment.target_id,
    remoteSession?.targetId,
    "remote target",
  );
  const targetId = nonEmptyString(environment.target_id, remoteSession?.targetId);
  const mode = backendModeLabel(nonEmptyString(
    summary?.backend_mode,
    environment.launch_source,
    "remote",
  ));
  const remoteSessionId = nonEmptyString(
    environment.remote_session_id,
    remoteSession?.sessionId,
    session?.session_id,
    session?.tmux_name,
    "selected session",
  );
  const cwdLine = handoffCwdLine(session, environment);
  const targetSuffix = targetId && targetId !== targetLabel ? ` (${targetId})` : "";
  return [
    `Remote handoff: local native open cannot open this remote terminal.`,
    `Open Swimmers on ${targetLabel}${targetSuffix} to attach.`,
    `backend: ${mode}`,
    `remote session: ${remoteSessionId}`,
    cwdLine,
  ].filter(Boolean).join("\n");
}

export function formatNativeStatusCopy(status, session = null, environments = []) {
  return remoteNativeHandoffMessage(session, environments) || formatNativeStatus(status);
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
    responseJson = defaultResponseJson,
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
    el.nativeStatusCopy.textContent = formatNativeStatusCopy(status, currentSession(), state.environments);
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
      const payload = await responseJson(response, normalizeNativeDesktopStatusResponse);
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
      const appPayload = await responseJson(appResponse, normalizeNativeDesktopStatusResponse);
      renderNativeStatusForm(appPayload);

      if (app === "ghostty") {
        const modeResponse = await apiFetch("/v1/native/mode", {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ mode }),
        });
        const modePayload = await responseJson(modeResponse, normalizeNativeDesktopStatusResponse);
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
    const handoff = remoteNativeHandoffMessage(session, state.environments);
    if (handoff) {
      setNativeResult(handoff, true);
      syncSheetActionAvailability();
      return;
    }

    setNativeResult(`Opening ${session.session_id} in the native app...`);
    try {
      const response = await apiFetch("/v1/native/open", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: session.session_id }),
      });
      const payload = await responseJson(response, normalizeNativeDesktopOpenResponse);
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
