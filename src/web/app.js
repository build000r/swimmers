import { buildSurfaceFrame, surfaceActionAt, surfaceConsumesPointer } from "/rendered_surface.js";
import { eventCell, shouldIgnoreSyntheticClick } from "/input_support.js";

const boot = window.__SWIMMERS_BOOT__ ?? {
  franken_term_available: false,
  franken_term_js_url: "",
  franken_term_wasm_url: "",
  follow_published_selection: false,
  focus_layout: false,
};

const TOKEN_STORAGE_KEY = "swimmers.web.token";
const SESSION_STORAGE_KEY = "swimmers.web.session";
const DIR_BROWSER_PATH_KEY = "swimmers.web.dirs.path";
const DIR_BROWSER_MANAGED_ONLY_KEY = "swimmers.web.dirs.managed";
const SESSION_REFRESH_MS = 2500;
const SNAPSHOT_REFRESH_MS = 900;
const SURFACE_CLICK_SUPPRESS_MS = 450;
const FALLBACK_THOUGHT_BACKENDS = [
  { key: "", label: "auto" },
  { key: "openrouter", label: "openrouter" },
  { key: "codex", label: "codex" },
];

const state = {
  token: "",
  sessions: [],
  selectedSessionId: null,
  publishedSelection: null,
  followPublishedSelection: Boolean(boot.follow_published_selection),
  readOnly: false,
  frankenModule: null,
  frankenInit: null,
  hud: null,
  terminal: null,
  terminalSessionId: null,
  ws: null,
  connectionGeneration: 0,
  refreshTimer: null,
  snapshotTimer: null,
  renderQueued: false,
  currentCols: 80,
  currentRows: 24,
  searchQuery: "",
  searchState: null,
  selectMode: false,
  selectionAnchor: null,
  selectionFocus: null,
  hoveredLinkUrl: "",
  utilityMessageTimer: null,
  connectionLabel: "disconnected",
  connectionMuted: false,
  modeLabel: "auth unknown",
  modeMuted: true,
  searchLabel: "Search idle",
  searchMuted: true,
  utilityLabel: "Cmd/Ctrl-click a terminal link to open it.",
  utilityMuted: true,
  surfaceZones: [],
  surfaceMasks: [],
  surfaceClickSuppressUntil: 0,
  activeSheet: null,
  thoughtConfig: {
    loading: false,
    config: null,
    ui: null,
    result: "",
    error: "",
  },
  nativeDesktop: {
    loading: false,
    status: null,
    result: "",
    error: "",
  },
  dirBrowser: {
    loading: false,
    path: "",
    managedOnly: false,
    entries: [],
    status: "",
    error: "",
  },
  mermaidArtifact: {
    loading: false,
    sessionId: null,
    artifact: null,
    svg: "",
    source: "",
    status: "",
    error: "",
  },
};

const el = {
  terminalStage: document.getElementById("terminal-stage"),
  terminalCanvas: document.getElementById("terminal-canvas"),
  hudCanvas: document.getElementById("hud-canvas"),
  terminalFallback: document.getElementById("terminal-fallback"),
  loadingOverlay: document.getElementById("loading-overlay"),
  loadingLabel: document.getElementById("loading-label"),
  modalRoot: document.getElementById("modal-root"),
  modalBackdrop: document.getElementById("modal-backdrop"),
  searchSheet: document.getElementById("search-sheet"),
  searchForm: document.getElementById("search-form"),
  terminalSearch: document.getElementById("terminal-search"),
  searchPrevButton: document.getElementById("search-prev-button"),
  searchNextButton: document.getElementById("search-next-button"),
  searchClearButton: document.getElementById("search-clear-button"),
  searchCloseButton: document.getElementById("search-close-button"),
  thoughtConfigSheet: document.getElementById("thought-config-sheet"),
  thoughtConfigForm: document.getElementById("thought-config-form"),
  thoughtConfigEnabled: document.getElementById("thought-config-enabled"),
  thoughtConfigBackend: document.getElementById("thought-config-backend"),
  thoughtConfigModel: document.getElementById("thought-config-model"),
  thoughtConfigModelPresets: document.getElementById("thought-config-model-presets"),
  thoughtConfigHint: document.getElementById("thought-config-hint"),
  thoughtConfigSummary: document.getElementById("thought-config-summary"),
  thoughtConfigDaemon: document.getElementById("thought-config-daemon"),
  thoughtConfigResult: document.getElementById("thought-config-result"),
  thoughtConfigTestButton: document.getElementById("thought-config-test-button"),
  thoughtConfigCloseButton: document.getElementById("thought-config-close-button"),
  thoughtConfigSaveButton: document.getElementById("thought-config-save-button"),
  nativeSheet: document.getElementById("native-sheet"),
  nativeForm: document.getElementById("native-form"),
  nativeStatusCopy: document.getElementById("native-status-copy"),
  nativeApp: document.getElementById("native-app"),
  nativeMode: document.getElementById("native-mode"),
  nativeStatusResult: document.getElementById("native-status-result"),
  nativeRefreshButton: document.getElementById("native-refresh-button"),
  nativeOpenButton: document.getElementById("native-open-button"),
  nativeCloseButton: document.getElementById("native-close-button"),
  nativeSaveButton: document.getElementById("native-save-button"),
  sendSheet: document.getElementById("send-sheet"),
  sendForm: document.getElementById("send-form"),
  sendInput: document.getElementById("send-input"),
  sendSubmitButton: document.getElementById("send-submit-button"),
  sendCloseButton: document.getElementById("send-close-button"),
  authSheet: document.getElementById("auth-sheet"),
  tokenInput: document.getElementById("token-input"),
  saveTokenButton: document.getElementById("save-token-button"),
  clearTokenButton: document.getElementById("clear-token-button"),
  authCloseButton: document.getElementById("auth-close-button"),
  createSheet: document.getElementById("create-sheet"),
  createForm: document.getElementById("create-form"),
  createCwd: document.getElementById("create-cwd"),
  createTool: document.getElementById("create-tool"),
  createRequest: document.getElementById("create-request"),
  createButton: document.getElementById("create-button"),
  createCloseButton: document.getElementById("create-close-button"),
  dirsSummary: document.getElementById("dirs-summary"),
  dirsManagedOnly: document.getElementById("dirs-managed-only"),
  dirsPath: document.getElementById("dirs-path"),
  dirsLoadButton: document.getElementById("dirs-load-button"),
  dirsUpButton: document.getElementById("dirs-up-button"),
  dirsList: document.getElementById("dirs-list"),
  mermaidSheet: document.getElementById("mermaid-sheet"),
  mermaidSummary: document.getElementById("mermaid-summary"),
  mermaidPreview: document.getElementById("mermaid-preview"),
  mermaidSource: document.getElementById("mermaid-source"),
  mermaidRefreshButton: document.getElementById("mermaid-refresh-button"),
  mermaidOpenButton: document.getElementById("mermaid-open-button"),
  mermaidCloseButton: document.getElementById("mermaid-close-button"),
};

function currentSession() {
  return state.sessions.find((session) => session.session_id === state.selectedSessionId) ?? null;
}

function surfaceSupports(surface, methodName) {
  return Boolean(surface && typeof surface[methodName] === "function");
}

function terminalSupports(methodName) {
  return surfaceSupports(state.terminal, methodName);
}

function hasLiveTerminal() {
  return Boolean(state.terminal);
}

function apiHeaders(extra = {}) {
  const headers = { ...extra };
  if (state.token) {
    headers.Authorization = `Bearer ${state.token}`;
  }
  return headers;
}

async function apiFetch(path, init = {}) {
  const headers = apiHeaders(init.headers ?? {});
  const response = await fetch(path, { ...init, headers });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const json = await response.json();
      if (json?.message) {
        message = json.message;
      } else if (json?.code) {
        message = json.code;
      }
    } catch (_) {
      // Keep the HTTP fallback message.
    }
    const error = new Error(message);
    error.status = response.status;
    throw error;
  }
  return response;
}

async function apiMaybeFetch(path, init = {}) {
  try {
    return await apiFetch(path, init);
  } catch (error) {
    if (error?.status === 404) {
      return null;
    }
    throw error;
  }
}

async function responseJsonOrNull(response) {
  if (!response) {
    return null;
  }
  return response.json();
}

function fallbackThoughtBackendMetadata() {
  return FALLBACK_THOUGHT_BACKENDS.map((backend) => ({
    key: backend.key,
    label: backend.label,
    model_presets_hint: backend.key === ""
      ? "auto backend uses daemon default model"
      : backend.key === "openrouter"
        ? "presets: auto  router  cached free models"
        : backend.key === "codex"
          ? "presets: auto  5.1-mini  5.3-codex  5.4"
          : "auto backend uses daemon default model",
    model_presets: backend.key === "openrouter"
      ? ["", "openrouter/free", "nvidia/nemotron-3-super-120b-a12b:free", "arcee-ai/trinity-large-preview:free"]
      : backend.key === "codex"
        ? ["", "gpt-5.1-codex-mini", "gpt-5.3-codex", "gpt-5.4"]
        : [""],
  }));
}

function thoughtBackendMetadata() {
  const backends = state.thoughtConfig.ui?.backends;
  return Array.isArray(backends) && backends.length ? backends : fallbackThoughtBackendMetadata();
}

function selectedThoughtBackendMetadata() {
  const backends = thoughtBackendMetadata();
  const backend = state.thoughtConfig.config?.backend || "";
  return backends.find((entry) => normalizeBackendKey(entry.key) === normalizeBackendKey(backend)) ?? backends[0] ?? null;
}

function normalizeBackendKey(value) {
  const key = String(value || "").trim().toLowerCase();
  if (!key) return "";
  if (key === "claude" || key === "claude-cli" || key === "claude_cli") return "openrouter";
  if (key === "codex-cli" || key === "codex_cli") return "codex";
  return key;
}

function normalizeThoughtModelForBackend(backend, model) {
  const key = normalizeBackendKey(backend);
  const trimmed = String(model || "").trim();
  if (!trimmed) {
    return "";
  }
  if (key === "openrouter") {
    return trimmed.includes("/") ? trimmed : "";
  }
  if (key === "codex") {
    return trimmed.startsWith("gpt-") ? trimmed : "";
  }
  if (!key) {
    return "";
  }
  return trimmed;
}

function currentNativeModeLabel() {
  const mode = state.nativeDesktop.status?.ghostty_mode || state.nativeDesktop.status?.ghosttyMode;
  if (!mode) {
    return "swap";
  }
  return String(mode).toLowerCase();
}

function setConnectionStatus(label, muted = false) {
  state.connectionLabel = label;
  state.connectionMuted = Boolean(muted);
  renderHudSurface();
}

function setModeStatus(label, muted = false) {
  state.modeLabel = label;
  state.modeMuted = Boolean(muted);
  renderHudSurface();
}

function setSearchStatus(label, muted = false) {
  state.searchLabel = label;
  state.searchMuted = Boolean(muted);
  renderHudSurface();
}

function clearUtilityStatusTimer() {
  if (state.utilityMessageTimer) {
    clearTimeout(state.utilityMessageTimer);
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
    state.utilityMessageTimer = window.setTimeout(() => {
      setUtilityStatus(defaultUtilityLabel(), !state.hoveredLinkUrl);
    }, ttlMs);
  }
}

function setLoadingState(visible, label = "Loading FrankenTerm...") {
  el.loadingLabel.textContent = label;
  el.loadingOverlay.classList.toggle("visible", Boolean(visible));
}

function persistToken(token) {
  state.token = token.trim();
  el.tokenInput.value = state.token;
  if (state.token) {
    localStorage.setItem(TOKEN_STORAGE_KEY, state.token);
  } else {
    localStorage.removeItem(TOKEN_STORAGE_KEY);
  }
}

function normalizeSessionId(sessionId) {
  const trimmed = typeof sessionId === "string" ? sessionId.trim() : "";
  return trimmed || null;
}

function syncUrlState() {
  const url = new URL(window.location.href);
  const publishedRoute = window.location.pathname === "/selected";
  url.searchParams.delete("token");
  if (state.followPublishedSelection) {
    if (publishedRoute) {
      url.searchParams.delete("follow");
    } else {
      url.searchParams.set("follow", "published");
    }
    url.searchParams.delete("session");
  } else if (state.selectedSessionId) {
    url.searchParams.delete("follow");
    url.searchParams.set("session", state.selectedSessionId);
  } else {
    url.searchParams.delete("follow");
    url.searchParams.delete("session");
  }
  window.history.replaceState({}, "", url);
}

function persistSelectedSession(sessionId, options = {}) {
  const normalized = normalizeSessionId(sessionId);
  state.selectedSessionId = normalized;
  if (normalized) {
    localStorage.setItem(SESSION_STORAGE_KEY, normalized);
  } else {
    localStorage.removeItem(SESSION_STORAGE_KEY);
  }

  if (options.syncUrl ?? true) {
    syncUrlState();
  }
}

function setFollowPublishedSelection(enabled, options = {}) {
  state.followPublishedSelection = Boolean(enabled);
  document.body.classList.toggle("following-published", state.followPublishedSelection);
  if (!options.skipUrlSync) {
    syncUrlState();
  }
  renderHudSurface();
}

function syncTerminalTools() {
  const searchReady = terminalSupports("setSearchQuery");
  const selectionReady = terminalSupports("copySelection") || terminalSupports("extractSelectionText");
  const liveTerminal = hasLiveTerminal();

  el.terminalSearch.disabled = !searchReady;
  el.searchPrevButton.disabled = !searchReady;
  el.searchNextButton.disabled = !searchReady;
  el.searchClearButton.disabled = !searchReady;
  el.sendInput.disabled = state.readOnly;
  el.sendSubmitButton.disabled = state.readOnly || !currentSession();
  Array.from(el.createForm.elements).forEach((element) => {
    element.disabled = state.readOnly;
  });

  el.terminalStage.classList.toggle("select-mode", state.selectMode);
  el.terminalStage.classList.toggle("link-hot", Boolean(state.hoveredLinkUrl) && !state.selectMode);

  if (!liveTerminal) {
    if (boot.franken_term_available) {
      setSearchStatus("Search waits for terminal attach", true);
    } else {
      setSearchStatus("Search needs FrankenTerm assets", true);
    }
  } else if (!searchReady) {
    setSearchStatus("Search unavailable in this FrankenTerm build", true);
  } else if (!state.searchQuery) {
    setSearchStatus("Search idle", true);
  }
}

function syncSheetActionAvailability() {
  const hasSession = Boolean(currentSession());
  const writeDisabled = Boolean(state.readOnly);
  const nativeSupported = Boolean(state.nativeDesktop.status?.supported);
  const mermaidPath = state.mermaidArtifact.artifact?.path;

  el.createButton.disabled = writeDisabled || !el.createCwd.value.trim();
  el.thoughtConfigTestButton.disabled = writeDisabled || !state.thoughtConfig.config;
  el.thoughtConfigSaveButton.disabled = writeDisabled || !state.thoughtConfig.config;
  el.nativeSaveButton.disabled = writeDisabled || !state.nativeDesktop.status;
  el.nativeOpenButton.disabled = writeDisabled || !hasSession || !nativeSupported;
  el.nativeRefreshButton.disabled = false;
  el.mermaidOpenButton.disabled = writeDisabled || !hasSession || !mermaidPath;
  el.mermaidRefreshButton.disabled = !hasSession;
  el.dirsLoadButton.disabled = !el.dirsPath.value.trim();
  el.dirsUpButton.disabled = !parentDir(el.dirsPath.value.trim());
}

function loadInitialState() {
  const url = new URL(window.location.href);
  const queryToken = url.searchParams.get("token") ?? "";
  const storedToken = localStorage.getItem(TOKEN_STORAGE_KEY) ?? "";
  const selectedFromUrl = url.searchParams.get("session");
  const selectedFromStorage = localStorage.getItem(SESSION_STORAGE_KEY);
  const followFromUrl = url.searchParams.get("follow") === "published";
  const storedDirPath = localStorage.getItem(DIR_BROWSER_PATH_KEY) ?? "";
  const storedManagedOnly = localStorage.getItem(DIR_BROWSER_MANAGED_ONLY_KEY) === "true";

  persistToken(queryToken || storedToken);
  setFollowPublishedSelection(boot.follow_published_selection || followFromUrl, { skipUrlSync: true });
  state.dirBrowser.path = storedDirPath;
  state.dirBrowser.managedOnly = storedManagedOnly;
  el.dirsPath.value = storedDirPath;
  el.dirsManagedOnly.checked = storedManagedOnly;
  el.createCwd.value = storedDirPath;
  persistSelectedSession(
    state.followPublishedSelection ? null : selectedFromUrl || selectedFromStorage || null,
    { syncUrl: false },
  );
  syncUrlState();
}

function sessionExists(sessionId) {
  return state.sessions.some((session) => session.session_id === sessionId);
}

function summarizeThought(session) {
  const thought = (session?.thought || "").trim();
  if (!thought) {
    return "No thought snapshot yet.";
  }
  return thought.length > 110 ? `${thought.slice(0, 107)}...` : thought;
}

function relativeCwd(cwd) {
  if (!cwd) return "unknown cwd";
  const parts = cwd.split("/").filter(Boolean);
  if (!parts.length) return cwd;
  return parts.slice(-2).join("/");
}

function formatTime(raw) {
  if (!raw) return "unknown";
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) {
    return raw;
  }
  return date.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function shortenUrl(raw) {
  if (!raw) return "";
  return raw.length > 72 ? `${raw.slice(0, 69)}...` : raw;
}

function surfaceSession(session, options = {}) {
  return {
    sessionId: session.session_id,
    name: session.tmux_name || session.session_id,
    state: String(session.state || "unknown"),
    restLabel: String(session.rest_state || "unknown"),
    transportLabel: String(session.transport_health || "unknown"),
    toolLabel: session.tool || "shell",
    cwdLabel: relativeCwd(session.cwd),
    fullCwd: session.cwd || "",
    thoughtLabel: options.detail ? session.thought || "No thought snapshot yet." : summarizeThought(session),
    contextLabel: `${session.token_count ?? 0} / ${session.context_limit ?? 0}`,
    skillLabel: session.last_skill || "none",
    activityLabel: formatTime(session.last_activity_at),
    commandLabel: session.current_command || "idle",
    attachedLabel: String(session.attached_clients ?? 0),
    commitCandidate: Boolean(session.commit_candidate),
  };
}

function parentDir(path) {
  const trimmed = String(path || "").trim().replace(/\/+$/g, "");
  if (!trimmed || trimmed === "/") {
    return "";
  }
  const index = trimmed.lastIndexOf("/");
  if (index <= 0) {
    return "/";
  }
  return trimmed.slice(0, index);
}

function formatNativeStatus(status) {
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

function setThoughtConfigResult(message, isError = false) {
  state.thoughtConfig.result = message;
  state.thoughtConfig.error = isError ? message : "";
  if (el.thoughtConfigResult) {
    el.thoughtConfigResult.textContent = message || "";
    el.thoughtConfigResult.classList.toggle("error", Boolean(isError));
  }
}

function setNativeResult(message, isError = false) {
  state.nativeDesktop.result = message;
  state.nativeDesktop.error = isError ? message : "";
  if (el.nativeStatusResult) {
    el.nativeStatusResult.textContent = message || "";
    el.nativeStatusResult.classList.toggle("error", Boolean(isError));
  }
}

function setDirStatus(message, isError = false) {
  state.dirBrowser.status = message;
  state.dirBrowser.error = isError ? message : "";
  if (el.dirsSummary) {
    el.dirsSummary.textContent = message || "";
    el.dirsSummary.classList.toggle("error", Boolean(isError));
  }
}

function setMermaidStatus(message, isError = false) {
  state.mermaidArtifact.status = message;
  state.mermaidArtifact.error = isError ? message : "";
  if (el.mermaidSummary) {
    el.mermaidSummary.textContent = message || "";
    el.mermaidSummary.classList.toggle("error", Boolean(isError));
  }
}

function renderThoughtConfigOptions() {
  const backends = thoughtBackendMetadata();
  const currentBackend = normalizeBackendKey(el.thoughtConfigBackend.value || state.thoughtConfig.config?.backend || "");
  el.thoughtConfigBackend.innerHTML = "";
  for (const backend of backends) {
    const option = document.createElement("option");
    option.value = backend.key;
    option.textContent = backend.label || backend.key || "auto";
    if (normalizeBackendKey(backend.key) === currentBackend) {
      option.selected = true;
    }
    el.thoughtConfigBackend.appendChild(option);
  }

  const selected = backends.find((backend) => normalizeBackendKey(backend.key) === currentBackend) ?? backends[0];
  el.thoughtConfigHint.textContent = selected?.model_presets_hint || "";
  const presets = Array.isArray(selected?.model_presets) ? selected.model_presets : [""];
  el.thoughtConfigModelPresets.innerHTML = "";
  for (const preset of presets) {
    const option = document.createElement("option");
    option.value = preset;
    el.thoughtConfigModelPresets.appendChild(option);
  }
}

function applyThoughtConfigToForm(payload) {
  const rawConfig = payload?.config || payload || null;
  const daemonDefaults = payload?.daemon_defaults ?? null;
  const ui = payload?.ui ?? null;
  const backend = normalizeBackendKey(rawConfig?.backend || "");
  const config = rawConfig
    ? {
        ...rawConfig,
        backend,
        model: normalizeThoughtModelForBackend(backend, rawConfig.model || ""),
      }
    : null;

  state.thoughtConfig.config = config;
  state.thoughtConfig.ui = ui;
  el.thoughtConfigEnabled.checked = Boolean(config?.enabled ?? true);
  el.thoughtConfigBackend.value = String(config?.backend || "");
  el.thoughtConfigModel.value = String(config?.model || "");
  renderThoughtConfigOptions();
  const backend = selectedThoughtBackendMetadata();
  el.thoughtConfigSummary.textContent = backend
    ? `${backend.label || backend.key || "auto"} backend selected.`
    : "Thought config loaded.";
  const daemonBackend = normalizeBackendKey(daemonDefaults?.backend || "");
  el.thoughtConfigDaemon.textContent = daemonDefaults
    ? `daemon default: ${daemonBackend || "auto"} / ${daemonDefaults.model || "(empty)"}`
    : "daemon default: unavailable";
  syncSheetActionAvailability();
}

function draftThoughtConfig() {
  if (!state.thoughtConfig.config) {
    return null;
  }
  return {
    ...state.thoughtConfig.config,
    enabled: Boolean(el.thoughtConfigEnabled.checked),
    backend: String(el.thoughtConfigBackend.value || "").trim(),
    model: String(el.thoughtConfigModel.value || "").trim(),
  };
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

function renderDirEntries(response) {
  const entries = Array.isArray(response?.entries) ? response.entries : [];
  state.dirBrowser.entries = entries;
  const path = String(response?.path || el.createCwd.value || "").trim();
  state.dirBrowser.path = path;
  localStorage.setItem(DIR_BROWSER_PATH_KEY, path);
  localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(Boolean(el.dirsManagedOnly.checked)));
  el.dirsPath.value = path;
  el.createCwd.value = path;
  el.dirsList.innerHTML = "";

  if (!entries.length) {
    const empty = document.createElement("div");
    empty.className = "browser-empty";
    empty.textContent = "No child directories found.";
    el.dirsList.appendChild(empty);
  } else {
    for (const entry of entries) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "browser-entry";
      row.dataset.path = joinPath(path, entry.name);
      row.dataset.hasChildren = String(Boolean(entry.has_children));
      row.innerHTML = `
        <span class="browser-entry-name">${escapeHtml(entry.name)}</span>
        <span class="browser-entry-meta">${entry.has_children ? "dir" : "leaf"}${entry.is_running ? " · running" : ""}</span>
      `;
      el.dirsList.appendChild(row);
    }
  }

  const managed = Boolean(el.dirsManagedOnly.checked);
  const summary = response?.path
    ? `${entries.length} entries at ${response.path}${managed ? " (managed)" : ""}`
    : "Select a directory to continue.";
  setDirStatus(summary);
  syncSheetActionAvailability();
}

function renderMermaidArtifact(payload) {
  state.mermaidArtifact.artifact = payload;
  const available = Boolean(payload?.available);
  const path = payload?.path || "(unknown path)";
  const updatedAt = payload?.updated_at ? formatTime(payload.updated_at) : "unknown";
  const source = payload?.source || "";
  state.mermaidArtifact.source = source;
  el.mermaidSource.textContent = source || "Mermaid source unavailable.";
  el.mermaidPreview.innerHTML = "";

  if (available && state.mermaidArtifact.svg) {
    el.mermaidPreview.innerHTML = state.mermaidArtifact.svg;
  }

  const lines = [
    `available: ${available}`,
    `path: ${path}`,
    `updated: ${updatedAt}`,
    payload?.error ? `error: ${payload.error}` : null,
  ].filter(Boolean);
  setMermaidStatus(lines.join("\n"));
  syncSheetActionAvailability();
}

function joinPath(base, name) {
  const root = String(base || "").replace(/\/+$/g, "");
  const child = String(name || "").replace(/^\/+/, "");
  if (!root) {
    return child ? `/${child}` : "/";
  }
  if (!child) {
    return root || "/";
  }
  if (root === "/") {
    return `/${child}`;
  }
  return `${root}/${child}`;
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

async function refreshThoughtConfig() {
  state.thoughtConfig.loading = true;
  try {
    const response = await apiFetch("/v1/thought-config");
    const payload = await response.json();
    applyThoughtConfigToForm(payload);
    setThoughtConfigResult("Thought config loaded.");
  } catch (error) {
    setThoughtConfigResult(`Failed to load thought config: ${error.message}`, true);
  } finally {
    state.thoughtConfig.loading = false;
    syncSheetActionAvailability();
  }
}

async function testThoughtConfig() {
  const draft = draftThoughtConfig();
  if (!draft) {
    return;
  }

  state.thoughtConfig.loading = true;
  setThoughtConfigResult("Testing thought config...");
  try {
    const response = await apiFetch("/v1/thought-config/test", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(draft),
    });
    const payload = await response.json();
    const message = payload?.message || "Thought config probe succeeded.";
    setThoughtConfigResult(
      `${message}\n` +
        `ok: ${Boolean(payload?.ok)}\n` +
        `llm_calls: ${payload?.llm_calls ?? 0}\n` +
        (payload?.last_backend_error ? `backend error: ${payload.last_backend_error}` : ""),
    );
  } catch (error) {
    setThoughtConfigResult(`Thought config test failed: ${error.message}`, true);
  } finally {
    state.thoughtConfig.loading = false;
    syncSheetActionAvailability();
  }
}

async function saveThoughtConfig() {
  const draft = draftThoughtConfig();
  if (!draft) {
    return;
  }

  state.thoughtConfig.loading = true;
  setThoughtConfigResult("Saving thought config...");
  try {
    const response = await apiFetch("/v1/thought-config", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(draft),
    });
    await response.json();
    state.thoughtConfig.config = draft;
    renderThoughtConfigOptions();
    setThoughtConfigResult("Thought config saved.");
    await refreshSessions();
  } catch (error) {
    setThoughtConfigResult(`Thought config save failed: ${error.message}`, true);
  } finally {
    state.thoughtConfig.loading = false;
    syncSheetActionAvailability();
  }
}

async function refreshNativeStatus() {
  state.nativeDesktop.loading = true;
  try {
    const response = await apiFetch("/v1/native/status");
    const payload = await response.json();
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
    const appPayload = await appResponse.json();
    renderNativeStatusForm(appPayload);

    if (app === "ghostty") {
      const modeResponse = await apiFetch("/v1/native/mode", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ mode }),
      });
      const modePayload = await modeResponse.json();
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
    const payload = await response.json();
    setNativeResult(`Opened ${payload.session_id} in native app${payload.pane_id ? ` (${payload.pane_id})` : ""}.`);
  } catch (error) {
    setNativeResult(`Failed to open session natively: ${error.message}`, true);
  } finally {
    syncSheetActionAvailability();
  }
}

async function loadDirListing(path = el.dirsPath.value, managedOnly = el.dirsManagedOnly.checked) {
  const targetPath = String(path || "").trim();
  const managed = Boolean(managedOnly);
  if (!targetPath && !state.dirBrowser.path) {
    return;
  }

  state.dirBrowser.loading = true;
  state.dirBrowser.managedOnly = managed;
  el.dirsManagedOnly.checked = managed;
  localStorage.setItem(DIR_BROWSER_PATH_KEY, targetPath || state.dirBrowser.path || "");
  localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(managed));
  try {
    const url = new URL("/v1/dirs", window.location.origin);
    if (targetPath) {
      url.searchParams.set("path", targetPath);
    }
    url.searchParams.set("managed_only", String(managed));
    const response = await apiFetch(url.pathname + url.search);
    const payload = await response.json();
    renderDirEntries(payload);
  } catch (error) {
    setDirStatus(`Failed to load directories: ${error.message}`, true);
  } finally {
    state.dirBrowser.loading = false;
    syncSheetActionAvailability();
  }
}

async function refreshMermaidArtifact() {
  const session = currentSession();
  if (!session) {
    return;
  }

  state.mermaidArtifact.loading = true;
  state.mermaidArtifact.sessionId = session.session_id;
  state.mermaidArtifact.artifact = null;
  state.mermaidArtifact.svg = "";
  state.mermaidArtifact.source = "";
  el.mermaidPreview.innerHTML = "";
  el.mermaidSource.textContent = "";
  try {
    const artifactResponse = await apiMaybeFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/mermaid-artifact`);
    const artifact = await responseJsonOrNull(artifactResponse);
    state.mermaidArtifact.artifact = artifact;
    if (artifact?.available) {
      const svgResponse = await apiMaybeFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/mermaid-artifact/svg`);
      if (svgResponse) {
        state.mermaidArtifact.svg = await svgResponse.text();
      } else {
        state.mermaidArtifact.svg = "";
      }
    } else {
      state.mermaidArtifact.svg = "";
    }
    renderMermaidArtifact(artifact);
  } catch (error) {
    setMermaidStatus(`Failed to load Mermaid artifact: ${error.message}`, true);
  } finally {
    state.mermaidArtifact.loading = false;
    syncSheetActionAvailability();
  }
}

async function openMermaidArtifactHost() {
  const session = currentSession();
  if (!session) {
    return;
  }

  try {
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/mermaid-artifact/open`, {
      method: "POST",
    });
    const payload = await response.json();
    setMermaidStatus(`Opened Mermaid artifact${payload?.path ? `: ${payload.path}` : ""}.`);
  } catch (error) {
    setMermaidStatus(`Failed to open Mermaid artifact: ${error.message}`, true);
  }
}

async function launchCommitCodex() {
  const session = currentSession();
  if (!session) {
    return;
  }

  setUtilityStatus(`Launching commit Codex for ${session.session_id}...`, false, 1800);
  try {
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/commit-codex`, {
      method: "POST",
    });
    const payload = await response.json();
    setUtilityStatus(
      `Commit Codex launched: ${payload.session_name} / ${payload.watch_command}`,
      false,
      3800,
    );
  } catch (error) {
    setUtilityStatus(`Failed to launch commit Codex: ${error.message}`, true, 3800);
  }
}

async function refreshSessions() {
  try {
    const requests = [apiFetch("/v1/sessions")];
    if (state.followPublishedSelection) {
      requests.push(apiFetch("/v1/selection"));
    }

    const [response, publishedResponse] = await Promise.all(requests);
    const payload = await response.json();
    state.sessions = Array.isArray(payload.sessions) ? payload.sessions : [];

    if (publishedResponse) {
      state.publishedSelection = await publishedResponse.json();
      const publishedSessionId = normalizeSessionId(state.publishedSelection?.session_id);
      if (publishedSessionId && sessionExists(publishedSessionId)) {
        persistSelectedSession(publishedSessionId);
      } else {
        persistSelectedSession(null);
      }
    } else {
      state.publishedSelection = null;
      if (!state.selectedSessionId || !sessionExists(state.selectedSessionId)) {
        persistSelectedSession(state.sessions[0]?.session_id ?? null);
      }
    }

    await setupHudSurface();
    renderHudSurface();
    syncTerminalTools();
    await connectSelectedSession();
    if (state.followPublishedSelection && !state.selectedSessionId) {
      setConnectionStatus("waiting", true);
    } else {
      setConnectionStatus(state.selectedSessionId ? "live" : "idle");
    }
    setModeStatus(state.readOnly ? "observer" : "operator", !state.token);
  } catch (error) {
    state.sessions = [];
    state.publishedSelection = null;
    persistSelectedSession(null);
    renderHudSurface();
    if (error?.status === 401 || error?.status === 403) {
      setConnectionStatus("auth required", true);
      setModeStatus("token needed", false);
    } else {
      setConnectionStatus("backend unavailable", true);
      setModeStatus("offline", true);
    }
  }
}

function scheduleSessionRefresh() {
  if (state.refreshTimer) {
    clearInterval(state.refreshTimer);
  }
  state.refreshTimer = window.setInterval(refreshSessions, SESSION_REFRESH_MS);
}

async function ensureFrankenTerm() {
  if (!boot.franken_term_available) {
    return null;
  }

  if (!state.frankenInit) {
    state.frankenInit = import(boot.franken_term_js_url).then(async (mod) => {
      await mod.default();
      state.frankenModule = mod;
      return mod;
    });
  }

  return state.frankenInit;
}

async function setupHudSurface() {
  const mod = await ensureFrankenTerm();
  if (!mod) {
    return null;
  }

  if (state.hud) {
    return state.hud;
  }

  setLoadingState(true, "Loading rendered control surface...");
  state.hud = new mod.FrankenTermWeb();
  await state.hud.init(el.hudCanvas, undefined);
  if (surfaceSupports(state.hud, "setAccessibility")) {
    state.hud.setAccessibility({
      reducedMotion: window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false,
    });
  }
  el.hudCanvas.classList.remove("hidden");
  measureAndResizeSurface(false, true);
  renderHudSurface();
  setLoadingState(false);
  return state.hud;
}

function destroyTerminalInstance() {
  state.selectionAnchor = null;
  state.selectionFocus = null;
  clearHoveredLink(false);
  if (state.terminal) {
    state.terminal.destroy();
    state.terminal = null;
  }
  state.terminalSessionId = null;
  el.terminalCanvas.classList.add("hidden");
}

async function setupTerminalSurface() {
  stopSnapshotPolling();

  const session = currentSession();
  if (!session) {
    teardownTerminal();
    return;
  }

  const mod = await ensureFrankenTerm();
  if (!mod) {
    teardownTerminal();
    el.terminalFallback.classList.remove("hidden");
    await refreshSnapshotFallback();
    startSnapshotPolling();
    return;
  }

  if (state.terminal && state.terminalSessionId === session.session_id) {
    el.terminalCanvas.classList.remove("hidden");
    el.terminalFallback.classList.add("hidden");
    refreshTerminalSearch();
    syncTerminalTools();
    setLoadingState(false);
    return;
  }

  destroyTerminalInstance();
  setLoadingState(true, "Initializing terminal...");
  state.terminal = new mod.FrankenTermWeb();
  await state.terminal.init(el.terminalCanvas, undefined);
  state.terminalSessionId = session.session_id;
  if (terminalSupports("setLinkOpenPolicy")) {
    state.terminal.setLinkOpenPolicy({
      allowHttp: true,
      allowHttps: true,
    });
  }
  if (terminalSupports("setAccessibility")) {
    state.terminal.setAccessibility({
      reducedMotion: window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false,
    });
  }
  el.terminalCanvas.classList.remove("hidden");
  el.terminalFallback.classList.add("hidden");
  clearTerminalSelection();
  refreshTerminalSearch();
  syncTerminalTools();
  measureAndResizeSurface(true, true);
  setLoadingState(false);
}

function teardownTerminal() {
  disconnectSocket();
  stopSnapshotPolling();
  destroyTerminalInstance();
  el.terminalFallback.classList.add("hidden");
  el.terminalFallback.textContent = "";
  syncTerminalTools();
  renderHudSurface();
}

function disconnectSocket() {
  state.connectionGeneration += 1;
  if (state.ws) {
    state.ws.onopen = null;
    state.ws.onmessage = null;
    state.ws.onclose = null;
    state.ws.onerror = null;
    state.ws.close();
    state.ws = null;
  }
}

function scheduleRender() {
  if (state.renderQueued) {
    return;
  }
  if (!state.terminal && !state.hud) {
    return;
  }
  state.renderQueued = true;
  requestAnimationFrame(() => {
    state.renderQueued = false;
    if (state.terminal) {
      state.terminal.render();
    }
    if (state.hud) {
      state.hud.render();
    }
  });
}

function sendResize() {
  if (!state.ws || state.ws.readyState !== WebSocket.OPEN || !state.selectedSessionId) {
    return;
  }
  state.ws.send(JSON.stringify({ type: "resize", cols: state.currentCols, rows: state.currentRows }));
}

function measureAndResizeSurface(pushResize = false, force = false) {
  const referenceSurface = state.terminal || state.hud;
  if (!referenceSurface) {
    return;
  }

  const rect = el.terminalStage.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const geo = referenceSurface.fitToContainer(rect.width, rect.height, dpr);
  const cols = clampInt(geo?.cols, 80, 24, 240);
  const rows = clampInt(geo?.rows, 24, 12, 120);

  if (!force && cols === state.currentCols && rows === state.currentRows && !pushResize) {
    return;
  }

  state.currentCols = cols;
  state.currentRows = rows;
  if (state.hud) {
    state.hud.resize(cols, rows);
  }
  if (state.terminal) {
    state.terminal.resize(cols, rows);
  }
  renderHudSurface();
  scheduleRender();

  if (pushResize) {
    sendResize();
  }
}

function buildSurfaceModel() {
  const selectedSession = currentSession();
  const surfaceSessions = state.sessions.map((session) => surfaceSession(session));
  const terminalReady = Boolean(state.terminal && state.ws && state.ws.readyState === WebSocket.OPEN);
  return {
    cols: state.currentCols,
    rows: state.currentRows,
    focusLayout: Boolean(boot.focus_layout && state.followPublishedSelection),
    followPublishedSelection: state.followPublishedSelection,
    connectionLabel: state.connectionLabel,
    connectionMuted: state.connectionMuted,
    modeLabel: state.modeLabel,
    modeMuted: state.modeMuted,
    searchLabel: state.searchLabel,
    searchMuted: state.searchMuted,
    utilityLabel: state.utilityLabel,
    utilityMuted: state.utilityMuted,
    searchQuery: state.searchQuery,
    selectMode: state.selectMode,
    readOnly: state.readOnly,
    frankenTermAvailable: boot.franken_term_available,
    terminalReady,
    snapshotFallback: !boot.franken_term_available,
    activeSheet: state.activeSheet,
    hoveredLinkUrl: state.hoveredLinkUrl,
    sessions: surfaceSessions,
    selectedSessionId: state.selectedSessionId,
    publishedSessionId: normalizeSessionId(state.publishedSelection?.session_id),
    publishedAtLabel: formatTime(state.publishedSelection?.published_at),
    currentSession: selectedSession ? surfaceSession(selectedSession, { detail: true }) : null,
  };
}

function renderHudSurface() {
  if (!state.hud) {
    return;
  }
  const frame = buildSurfaceFrame(buildSurfaceModel());
  state.surfaceZones = frame.zones ?? [];
  state.surfaceMasks = frame.masks ?? [];
  state.hud.applyPatchBatchFlat(frame.spans, frame.cells);
  scheduleRender();
}

async function connectSelectedSession() {
  await setupHudSurface();

  const session = currentSession();
  if (!session) {
    teardownTerminal();
    return;
  }

  await setupTerminalSurface();
  if (!state.terminal) {
    return;
  }

  if (state.ws && state.ws.readyState <= WebSocket.OPEN && state.ws.sessionId === session.session_id) {
    return;
  }

  disconnectSocket();
  const generation = state.connectionGeneration;
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = new URL(`${protocol}//${window.location.host}/ws/sessions/${encodeURIComponent(session.session_id)}`);
  if (state.token) {
    url.searchParams.set("token", state.token);
  }

  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  ws.sessionId = session.session_id;
  state.ws = ws;
  setConnectionStatus("connecting");

  ws.onopen = () => {
    if (generation !== state.connectionGeneration || state.ws !== ws) {
      ws.close();
      return;
    }
    measureAndResizeSurface(true, true);
    setConnectionStatus("attached");
  };

  ws.onmessage = (event) => {
    if (generation !== state.connectionGeneration || state.ws !== ws) {
      return;
    }

    if (typeof event.data === "string") {
      handleSocketText(event.data);
      return;
    }

    if (state.terminal) {
      state.terminal.feed(new Uint8Array(event.data));
      if (state.searchQuery) {
        refreshTerminalSearch();
      }
      scheduleRender();
    }
  };

  ws.onclose = () => {
    if (generation !== state.connectionGeneration) {
      return;
    }
    setConnectionStatus("detached", true);
    window.setTimeout(() => {
      if (generation !== state.connectionGeneration || !currentSession()) {
        return;
      }
      connectSelectedSession();
    }, 1400);
  };

  ws.onerror = () => {
    setConnectionStatus("attach failed", true);
  };
}

function handleSocketText(raw) {
  try {
    const message = JSON.parse(raw);
    switch (message.type) {
      case "ready":
        state.readOnly = Boolean(message.readOnly);
        setModeStatus(state.readOnly ? "observer" : "operator", !state.token);
        syncWriteAccess();
        syncTerminalTools();
        if (message.summary) {
          mergeSummary(message.summary);
        }
        break;
      case "replay_truncated":
        setConnectionStatus("partial replay", true);
        break;
      case "error":
        setConnectionStatus(message.code || "error", true);
        break;
      case "pong":
        break;
      default:
        break;
    }
  } catch (_) {
    // Ignore malformed transport diagnostics.
  }
}

function mergeSummary(summary) {
  const index = state.sessions.findIndex((session) => session.session_id === summary.session_id);
  if (index >= 0) {
    state.sessions[index] = summary;
  }
  renderHudSurface();
}

function syncWriteAccess() {
  el.sendInput.disabled = state.readOnly;
  el.sendSubmitButton.disabled = state.readOnly;
  el.createButton.disabled = state.readOnly || !el.createCwd.value.trim();
  el.thoughtConfigTestButton.disabled = state.readOnly || !state.thoughtConfig.config;
  el.thoughtConfigSaveButton.disabled = state.readOnly || !state.thoughtConfig.config;
  el.nativeSaveButton.disabled = state.readOnly || !state.nativeDesktop.status;
  el.nativeOpenButton.disabled = state.readOnly || !currentSession();
  el.mermaidOpenButton.disabled = state.readOnly || !currentSession();
  el.dirsLoadButton.disabled = state.readOnly || !el.dirsPath.value.trim();
  syncTerminalTools();
  syncSheetActionAvailability();
}

function flushEncodedInputBytes() {
  if (!state.terminal || !state.ws || state.ws.readyState !== WebSocket.OPEN || state.readOnly) {
    return;
  }

  const payload = state.terminal.drainEncodedInputBytes();
  if (!payload) {
    return;
  }

  const chunks = Array.isArray(payload) ? payload : [payload];
  for (const chunk of chunks) {
    const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
    if (bytes.byteLength > 0) {
      state.ws.send(bytes);
    }
  }
}

function forwardTerminalEvent(event) {
  if (!state.terminal || state.readOnly) {
    return;
  }
  state.terminal.input(event);
  flushEncodedInputBytes();
}

function keyModifiers(event) {
  return (event.shiftKey ? 1 : 0) | (event.altKey ? 2 : 0) | (event.ctrlKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

function shouldCaptureKey(event) {
  if (!currentSession() || state.readOnly || state.activeSheet) {
    return false;
  }
  if (event.metaKey) {
    return false;
  }
  return true;
}

function mouseCell(event) {
  const rect = el.terminalStage.getBoundingClientRect();
  return eventCell(event, rect, state.currentCols, state.currentRows);
}

function cellOffset(cell) {
  return cell.y * Math.max(1, state.currentCols) + cell.x;
}

function setTerminalSelectionRange(start, end) {
  if (!terminalSupports("setSelectionRange")) {
    return;
  }
  const normalizedStart = Math.min(start, end);
  const normalizedEnd = Math.max(start, end) + 1;
  state.selectionFocus = end;
  state.terminal.setSelectionRange(normalizedStart, normalizedEnd);
  scheduleRender();
}

function clearTerminalSelection() {
  state.selectionAnchor = null;
  state.selectionFocus = null;
  if (terminalSupports("clearSelection")) {
    state.terminal.clearSelection();
    scheduleRender();
  }
}

function setSelectMode(enabled) {
  state.selectMode = Boolean(enabled);
  if (!state.selectMode) {
    clearTerminalSelection();
  }
  syncTerminalTools();
}

function updateSearchUi(searchState) {
  state.searchState = searchState ?? null;
  if (!state.searchQuery) {
    setSearchStatus("Search idle", true);
    return;
  }

  const matchCount = Number(state.searchState?.matchCount ?? 0);
  if (matchCount > 0) {
    const activeIndex = Number(state.searchState?.activeMatchIndex ?? 0) + 1;
    setSearchStatus(`${activeIndex}/${matchCount} matches`, false);
  } else {
    setSearchStatus("No matches", true);
  }
}

function refreshTerminalSearch() {
  if (!state.searchQuery || !terminalSupports("setSearchQuery")) {
    updateSearchUi(null);
    return;
  }
  updateSearchUi(state.terminal.setSearchQuery(state.searchQuery, null));
  scheduleRender();
}

function applySearchQuery(rawQuery) {
  state.searchQuery = typeof rawQuery === "string" ? rawQuery : "";
  if (!state.searchQuery) {
    if (terminalSupports("clearSearch")) {
      state.terminal.clearSearch();
      scheduleRender();
    }
    updateSearchUi(null);
    renderHudSurface();
    return;
  }

  if (!terminalSupports("setSearchQuery")) {
    setSearchStatus("Search unavailable in this FrankenTerm build", true);
    return;
  }

  refreshTerminalSearch();
  renderHudSurface();
}

function cycleSearchMatch(direction) {
  if (!state.searchQuery || !hasLiveTerminal()) {
    return;
  }

  if (direction < 0 && terminalSupports("searchPrev")) {
    updateSearchUi(state.terminal.searchPrev());
    scheduleRender();
    return;
  }

  if (direction > 0 && terminalSupports("searchNext")) {
    updateSearchUi(state.terminal.searchNext());
    scheduleRender();
  }
}

async function copyTerminalSelection() {
  if (!hasLiveTerminal()) {
    return;
  }

  const text =
    (terminalSupports("copySelection") && state.terminal.copySelection()) ||
    (terminalSupports("extractSelectionText") && state.terminal.extractSelectionText()) ||
    "";

  if (!text) {
    setUtilityStatus("No terminal selection to copy.", true, 2200);
    return;
  }

  if (!navigator.clipboard?.writeText) {
    setUtilityStatus("Clipboard write is unavailable in this browser context.", true, 3000);
    return;
  }

  try {
    await navigator.clipboard.writeText(text);
    setUtilityStatus(`Copied ${text.length} characters from the terminal.`, false, 2200);
  } catch (error) {
    setUtilityStatus(`Clipboard write failed: ${error.message}`, true, 3000);
  }
}

function safeOpenUrl(rawUrl) {
  try {
    const url = new URL(rawUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      setUtilityStatus(`Blocked unsupported link protocol: ${url.protocol}`, true, 2600);
      return;
    }
    window.open(url.toString(), "_blank", "noopener,noreferrer");
  } catch (error) {
    setUtilityStatus(`Invalid link: ${error.message}`, true, 2600);
  }
}

function clearHoveredLink(updateUi = true) {
  state.hoveredLinkUrl = "";
  if (terminalSupports("setHoveredLinkId")) {
    state.terminal.setHoveredLinkId(0);
    scheduleRender();
  }
  if (updateUi) {
    setUtilityStatus(defaultUtilityLabel(), true);
    syncTerminalTools();
  }
}

function updateHoveredLink(event) {
  if (!hasLiveTerminal() || !terminalSupports("linkUrlAt") || state.selectMode) {
    clearHoveredLink(true);
    return;
  }

  const cell = mouseCell(event);
  const url = state.terminal.linkUrlAt(cell.x, cell.y) ?? "";
  state.hoveredLinkUrl = url;
  if (terminalSupports("linkAt") && terminalSupports("setHoveredLinkId")) {
    const linkId = state.terminal.linkAt(cell.x, cell.y);
    state.terminal.setHoveredLinkId(linkId);
    scheduleRender();
  }

  if (url) {
    setUtilityStatus(`Cmd/Ctrl-click to open ${shortenUrl(url)}.`, false);
  } else {
    setUtilityStatus("Cmd/Ctrl-click a terminal link to open it.", true);
  }
  syncTerminalTools();
}

async function sendLine(text) {
  if (!text || !currentSession()) {
    return;
  }

  const payload = text.endsWith("\n") ? text : `${text}\n`;

  if (state.ws && state.ws.readyState === WebSocket.OPEN && !state.readOnly) {
    state.ws.send(JSON.stringify({ type: "input_text", data: payload }));
    return;
  }

  await apiFetch(`/v1/sessions/${encodeURIComponent(state.selectedSessionId)}/input`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text: payload }),
  });
}

async function createSessionFromSheet() {
  if (state.readOnly) {
    return;
  }

  const cwd = el.createCwd.value.trim();
  const initialRequest = el.createRequest.value.trim();
  const spawnTool = el.createTool.value;

  const response = await apiFetch("/v1/sessions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      cwd: cwd || null,
      spawn_tool: spawnTool,
      initial_request: initialRequest || null,
    }),
  });

  const payload = await response.json();
  const created = payload?.session;
  if (created?.session_id) {
    closeSheets();
    await refreshSessions();
    await selectSession(created.session_id);
  }
}

async function refreshSnapshotFallback() {
  const session = currentSession();
  if (!session) {
    return;
  }

  try {
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/snapshot`);
    const payload = await response.json();
    el.terminalFallback.textContent = payload.screen_text || "";
    syncTerminalTools();
  } catch (error) {
    el.terminalFallback.textContent = `Snapshot unavailable: ${error.message}`;
  }
}

function startSnapshotPolling() {
  stopSnapshotPolling();
  state.snapshotTimer = window.setInterval(refreshSnapshotFallback, SNAPSHOT_REFRESH_MS);
}

function stopSnapshotPolling() {
  if (state.snapshotTimer) {
    clearInterval(state.snapshotTimer);
    state.snapshotTimer = null;
  }
}

async function openCreateSheet() {
  const selected = currentSession();
  const preferredPath = String(el.createCwd.value || state.dirBrowser.path || selected?.cwd || "").trim();
  const initialPath = preferredPath || "/";
  el.createCwd.value = initialPath;
  el.dirsPath.value = initialPath;
  if (typeof state.dirBrowser.managedOnly !== "boolean") {
    state.dirBrowser.managedOnly = false;
  }
  el.dirsManagedOnly.checked = state.dirBrowser.managedOnly;
  await loadDirListing(initialPath, state.dirBrowser.managedOnly);
}

function openThoughtConfigSheet() {
  openSheet("thought-config");
}

function openNativeSheet() {
  openSheet("native");
}

function openMermaidSheet() {
  if (!currentSession()) {
    setUtilityStatus("Select a session before opening Mermaid artifacts.", true, 2600);
    return;
  }
  openSheet("mermaid");
}

function setActiveSheet(sheetId) {
  state.activeSheet = sheetId;
  document.body.classList.toggle("sheet-open", Boolean(sheetId));
  el.modalRoot.classList.toggle("visible", Boolean(sheetId));
  el.modalRoot.setAttribute("aria-hidden", sheetId ? "false" : "true");
  el.searchSheet.classList.toggle("hidden", sheetId !== "search");
  el.thoughtConfigSheet.classList.toggle("hidden", sheetId !== "thought-config");
  el.nativeSheet.classList.toggle("hidden", sheetId !== "native");
  el.sendSheet.classList.toggle("hidden", sheetId !== "send");
  el.authSheet.classList.toggle("hidden", sheetId !== "auth");
  el.createSheet.classList.toggle("hidden", sheetId !== "create");
  el.mermaidSheet.classList.toggle("hidden", sheetId !== "mermaid");
  syncSheetActionAvailability();
  renderHudSurface();
}

function focusActiveSheet() {
  requestAnimationFrame(() => {
    switch (state.activeSheet) {
      case "search":
        el.terminalSearch.focus();
        el.terminalSearch.select();
        break;
      case "thought-config":
        el.thoughtConfigModel.focus();
        el.thoughtConfigModel.select();
        break;
      case "native":
        el.nativeApp.focus();
        break;
      case "send":
        el.sendInput.focus();
        break;
      case "auth":
        el.tokenInput.focus();
        el.tokenInput.select();
        break;
      case "create":
        el.createCwd.focus();
        break;
      case "mermaid":
        el.mermaidRefreshButton.focus();
        break;
      default:
        el.terminalStage.focus();
        break;
    }
  });
}

function openSheet(sheetId) {
  setActiveSheet(sheetId);
  if (sheetId === "search") {
    el.terminalSearch.value = state.searchQuery;
  }
  if (sheetId === "create") {
    void openCreateSheet();
  }
  if (sheetId === "thought-config") {
    void refreshThoughtConfig();
  }
  if (sheetId === "native") {
    void refreshNativeStatus();
  }
  if (sheetId === "mermaid") {
    void refreshMermaidArtifact();
  }
  if (sheetId === "auth") {
    el.tokenInput.value = state.token;
  }
  focusActiveSheet();
}

function closeSheets() {
  setActiveSheet(null);
  el.terminalStage.focus();
}

async function selectSession(sessionId) {
  if (!normalizeSessionId(sessionId)) {
    return;
  }
  if (state.followPublishedSelection) {
    setFollowPublishedSelection(false);
  }
  persistSelectedSession(sessionId);
  renderHudSurface();
  await connectSelectedSession();
  if (state.activeSheet === "mermaid") {
    await refreshMermaidArtifact();
  }
}

async function toggleFollowPublished() {
  setFollowPublishedSelection(!state.followPublishedSelection);
  if (!state.followPublishedSelection && !state.selectedSessionId) {
    persistSelectedSession(state.sessions[0]?.session_id ?? null);
  }
  renderHudSurface();
  syncTerminalTools();
  await refreshSessions();
}

async function handleSurfaceAction(zone) {
  if (!zone || zone.disabled) {
    return;
  }

  if (zone.type === "session") {
    await selectSession(zone.sessionId);
    return;
  }

  switch (zone.actionId) {
    case "open_search":
      openSheet("search");
      break;
    case "open_send":
      if (!state.readOnly && currentSession()) {
        openSheet("send");
      }
      break;
    case "open_auth":
      openSheet("auth");
      break;
    case "open_config":
      openThoughtConfigSheet();
      break;
    case "open_native":
      openNativeSheet();
      break;
    case "open_mermaid":
      openMermaidSheet();
      break;
    case "launch_commit":
      await launchCommitCodex();
      break;
    case "open_create":
      if (!state.readOnly) {
        openSheet("create");
      }
      break;
    case "toggle_follow":
      await toggleFollowPublished();
      break;
    case "toggle_select":
      setSelectMode(!state.selectMode);
      break;
    case "copy_selection":
      await copyTerminalSelection();
      break;
    case "focus_terminal":
      el.terminalStage.focus();
      setUtilityStatus(
        currentSession()
          ? "Terminal focused. Type directly or use the terminal actions below."
          : "Select a session row to attach its terminal first.",
        !currentSession(),
        2200,
      );
      break;
    case "refresh":
      await refreshSessions();
      break;
    default:
      break;
  }
}

function surfaceHit(event) {
  const cell = mouseCell(event);
  return {
    cell,
    action: surfaceActionAt(state.surfaceZones, cell),
    consume: surfaceConsumesPointer(state.surfaceMasks, cell),
  };
}

function handleGlobalShortcut(event) {
  if (event.key === "Escape") {
    if (state.activeSheet) {
      closeSheets();
      return true;
    }
    if (state.selectMode) {
      setSelectMode(false);
      return true;
    }
    return false;
  }

  if (!(event.ctrlKey && event.shiftKey) || event.metaKey || event.altKey) {
    return false;
  }

  switch (event.code) {
    case "KeyF":
      openSheet("search");
      return true;
    case "KeyS":
      if (!state.readOnly && currentSession()) {
        openSheet("send");
      }
      return true;
    case "KeyA":
      openSheet("auth");
      return true;
    case "KeyT":
      openThoughtConfigSheet();
      return true;
    case "KeyO":
      openNativeSheet();
      return true;
    case "KeyN":
      if (!state.readOnly) {
        openSheet("create");
      }
      return true;
    case "KeyM":
      openMermaidSheet();
      return true;
    case "KeyP":
      void toggleFollowPublished();
      return true;
    case "KeyV":
      setSelectMode(!state.selectMode);
      return true;
    case "KeyC":
      void copyTerminalSelection();
      return true;
    case "KeyR":
      void refreshSessions();
      return true;
    default:
      return false;
  }
}

function bindEvents() {
  el.modalBackdrop.addEventListener("click", closeSheets);
  el.modalRoot.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeSheets();
    }
  });

  el.searchForm.addEventListener("submit", (event) => {
    event.preventDefault();
    closeSheets();
  });
  el.terminalSearch.addEventListener("input", (event) => {
    applySearchQuery(event.target.value);
  });
  el.searchPrevButton.addEventListener("click", () => {
    cycleSearchMatch(-1);
  });
  el.searchNextButton.addEventListener("click", () => {
    cycleSearchMatch(1);
  });
  el.searchClearButton.addEventListener("click", () => {
    el.terminalSearch.value = "";
    applySearchQuery("");
  });
  el.searchCloseButton.addEventListener("click", closeSheets);

  el.thoughtConfigForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    await saveThoughtConfig();
  });
  el.thoughtConfigBackend.addEventListener("change", () => {
    el.thoughtConfigModel.value = normalizeThoughtModelForBackend(
      el.thoughtConfigBackend.value,
      el.thoughtConfigModel.value,
    );
    renderThoughtConfigOptions();
    syncSheetActionAvailability();
  });
  el.thoughtConfigModel.addEventListener("input", () => {
    syncSheetActionAvailability();
  });
  el.thoughtConfigEnabled.addEventListener("change", () => {
    syncSheetActionAvailability();
  });
  el.thoughtConfigTestButton.addEventListener("click", async () => {
    await testThoughtConfig();
  });
  el.thoughtConfigCloseButton.addEventListener("click", closeSheets);

  el.nativeForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    await saveNativeSettings();
  });
  el.nativeRefreshButton.addEventListener("click", async () => {
    await refreshNativeStatus();
  });
  el.nativeOpenButton.addEventListener("click", async () => {
    await openSelectedNativeSession();
  });
  el.nativeCloseButton.addEventListener("click", closeSheets);
  el.nativeApp.addEventListener("change", () => {
    el.nativeMode.disabled = String(el.nativeApp.value).toLowerCase() !== "ghostty";
    syncSheetActionAvailability();
  });
  el.nativeMode.addEventListener("change", () => {
    syncSheetActionAvailability();
  });

  el.sendForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (state.readOnly) {
      return;
    }
    const text = el.sendInput.value.trim();
    if (!text) {
      return;
    }
    await sendLine(text);
    el.sendInput.value = "";
    closeSheets();
  });
  el.sendCloseButton.addEventListener("click", closeSheets);

  el.saveTokenButton.addEventListener("click", async () => {
    persistToken(el.tokenInput.value);
    closeSheets();
    await refreshSessions();
  });
  el.clearTokenButton.addEventListener("click", async () => {
    persistToken("");
    state.readOnly = false;
    syncWriteAccess();
    closeSheets();
    await refreshSessions();
  });
  el.authCloseButton.addEventListener("click", closeSheets);

  el.createForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    await createSessionFromSheet();
  });
  el.createCloseButton.addEventListener("click", closeSheets);
  el.createCwd.addEventListener("input", () => {
    el.dirsPath.value = el.createCwd.value;
    localStorage.setItem(DIR_BROWSER_PATH_KEY, String(el.createCwd.value || "").trim());
    syncSheetActionAvailability();
  });
  el.dirsManagedOnly.addEventListener("change", () => {
    state.dirBrowser.managedOnly = Boolean(el.dirsManagedOnly.checked);
    localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(state.dirBrowser.managedOnly));
    syncSheetActionAvailability();
    void loadDirListing(el.dirsPath.value, state.dirBrowser.managedOnly);
  });
  el.dirsPath.addEventListener("input", () => {
    localStorage.setItem(DIR_BROWSER_PATH_KEY, String(el.dirsPath.value || "").trim());
    syncSheetActionAvailability();
  });
  el.dirsPath.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void loadDirListing(el.dirsPath.value, el.dirsManagedOnly.checked);
    }
  });
  el.dirsLoadButton.addEventListener("click", async () => {
    await loadDirListing(el.dirsPath.value, el.dirsManagedOnly.checked);
  });
  el.dirsUpButton.addEventListener("click", async () => {
    const parent = parentDir(el.dirsPath.value);
    if (parent) {
      el.dirsPath.value = parent;
      el.createCwd.value = parent;
      await loadDirListing(parent, el.dirsManagedOnly.checked);
    }
  });
  el.dirsList.addEventListener("click", async (event) => {
    const button = event.target instanceof Element ? event.target.closest(".browser-entry") : null;
    if (!button) {
      return;
    }
    const path = String(button.dataset.path || "").trim();
    if (!path) {
      return;
    }
    el.dirsPath.value = path;
    el.createCwd.value = path;
    if (button.dataset.hasChildren === "true") {
      await loadDirListing(path, el.dirsManagedOnly.checked);
    } else {
      setDirStatus(`Selected ${path}`);
      syncSheetActionAvailability();
    }
  });

  el.mermaidRefreshButton.addEventListener("click", async () => {
    await refreshMermaidArtifact();
  });
  el.mermaidOpenButton.addEventListener("click", async () => {
    await openMermaidArtifactHost();
  });
  el.mermaidCloseButton.addEventListener("click", closeSheets);

  el.terminalStage.addEventListener("click", (event) => {
    const hit = surfaceHit(event);
    if (hit.action) {
      if (shouldIgnoreSyntheticClick(performance.now(), state.surfaceClickSuppressUntil)) {
        event.preventDefault();
        return;
      }
      event.preventDefault();
      void handleSurfaceAction(hit.action);
      return;
    }
    if (!state.activeSheet) {
      el.terminalStage.focus();
    }
  });

  el.terminalStage.addEventListener(
    "touchend",
    (event) => {
      const hit = surfaceHit(event);
      if (hit.action) {
        state.surfaceClickSuppressUntil = performance.now() + SURFACE_CLICK_SUPPRESS_MS;
        event.preventDefault();
        void handleSurfaceAction(hit.action);
        return;
      }
      if (hit.consume) {
        event.preventDefault();
        return;
      }
      if (!state.activeSheet) {
        el.terminalStage.focus();
      }
    },
    { passive: false },
  );

  el.terminalStage.addEventListener("keydown", (event) => {
    if (handleGlobalShortcut(event)) {
      event.preventDefault();
      return;
    }
    if (!shouldCaptureKey(event)) {
      return;
    }
    event.preventDefault();
    forwardTerminalEvent({
      kind: "key",
      phase: "down",
      key: typeof event.key === "string" ? event.key : "",
      code: typeof event.code === "string" ? event.code : "",
      mods: keyModifiers(event),
      repeat: Boolean(event.repeat),
    });
  });

  el.terminalStage.addEventListener("paste", (event) => {
    if (state.readOnly) {
      return;
    }
    const text = event.clipboardData?.getData("text") ?? "";
    if (!text) {
      return;
    }
    event.preventDefault();
    if (terminalSupports("pasteText")) {
      state.terminal.pasteText(text);
      flushEncodedInputBytes();
      return;
    }
    forwardTerminalEvent({ kind: "paste", data: text });
  });

  el.terminalStage.addEventListener("focus", () => {
    if (!state.activeSheet) {
      forwardTerminalEvent({ kind: "focus", focused: true });
    }
  });

  el.terminalStage.addEventListener("blur", () => {
    forwardTerminalEvent({ kind: "focus", focused: false });
  });

  el.terminalStage.addEventListener("mousedown", (event) => {
    const hit = surfaceHit(event);
    if (hit.action) {
      state.surfaceClickSuppressUntil = performance.now() + SURFACE_CLICK_SUPPRESS_MS;
      event.preventDefault();
      void handleSurfaceAction(hit.action);
      return;
    }
    if (hit.consume || !state.terminal) {
      event.preventDefault();
      return;
    }

    updateHoveredLink(event);
    if ((event.metaKey || event.ctrlKey) && state.hoveredLinkUrl) {
      event.preventDefault();
      return;
    }

    if (state.selectMode && event.button === 0) {
      event.preventDefault();
      const anchor = cellOffset(hit.cell);
      state.selectionAnchor = anchor;
      setTerminalSelectionRange(anchor, anchor);
      return;
    }

    if (state.readOnly) {
      return;
    }

    forwardTerminalEvent({
      kind: "mouse",
      phase: "down",
      button: clampInt(event.button, 0, 0, 2),
      x: hit.cell.x,
      y: hit.cell.y,
      mods: keyModifiers(event),
    });
  });

  el.terminalStage.addEventListener("mouseup", (event) => {
    const hit = surfaceHit(event);
    if (hit.action || hit.consume || !state.terminal) {
      if (hit.action || hit.consume) {
        event.preventDefault();
      }
      return;
    }

    updateHoveredLink(event);
    if ((event.metaKey || event.ctrlKey) && state.hoveredLinkUrl) {
      event.preventDefault();
      safeOpenUrl(state.hoveredLinkUrl);
      return;
    }

    if (state.selectMode && state.selectionAnchor !== null && event.button === 0) {
      event.preventDefault();
      const focus = cellOffset(hit.cell);
      setTerminalSelectionRange(state.selectionAnchor, focus);
      state.selectionAnchor = null;
      return;
    }

    if (state.readOnly) {
      return;
    }

    forwardTerminalEvent({
      kind: "mouse",
      phase: "up",
      button: clampInt(event.button, 0, 0, 2),
      x: hit.cell.x,
      y: hit.cell.y,
      mods: keyModifiers(event),
    });
  });

  el.terminalStage.addEventListener("mousemove", (event) => {
    const hit = surfaceHit(event);
    if (hit.consume || !state.terminal) {
      if (hit.consume) {
        clearHoveredLink(true);
      }
      return;
    }

    if (state.selectMode && state.selectionAnchor !== null && (event.buttons & 1) === 1) {
      event.preventDefault();
      setTerminalSelectionRange(state.selectionAnchor, cellOffset(hit.cell));
      return;
    }

    updateHoveredLink(event);

    if (state.readOnly) {
      return;
    }

    forwardTerminalEvent({
      kind: "mouse",
      phase: "move",
      button: 0,
      x: hit.cell.x,
      y: hit.cell.y,
      mods: keyModifiers(event),
    });
  });

  el.terminalStage.addEventListener(
    "wheel",
    (event) => {
      const hit = surfaceHit(event);
      if (hit.consume) {
        event.preventDefault();
        return;
      }
      if (state.readOnly || !state.terminal || state.selectMode) {
        return;
      }
      event.preventDefault();
      forwardTerminalEvent({
        kind: "wheel",
        x: hit.cell.x,
        y: hit.cell.y,
        dx: Math.round(event.deltaX),
        dy: Math.round(event.deltaY),
        mods: keyModifiers(event),
      });
    },
    { passive: false },
  );

  el.terminalStage.addEventListener("mouseleave", () => {
    clearHoveredLink(true);
  });

  const resizeObserver = new ResizeObserver(() => {
    measureAndResizeSurface(true, true);
  });
  resizeObserver.observe(el.terminalStage);
}

async function init() {
  loadInitialState();
  bindEvents();
  setUtilityStatus(defaultUtilityLabel(), true);
  syncWriteAccess();
  setLoadingState(boot.franken_term_available, boot.franken_term_available ? "Loading rendered control surface..." : "Snapshot fallback mode");
  await setupHudSurface();
  renderHudSurface();
  scheduleSessionRefresh();
  await refreshSessions();
  if (boot.franken_term_available) {
    setLoadingState(false);
  }
}

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

init().catch((error) => {
  console.error("[swimmers-web] failed to initialize", error);
  setConnectionStatus("init failed", true);
  setLoadingState(false);
});
