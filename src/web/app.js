import { buildSurfaceFrame, surfaceActionAt, surfaceConsumesPointer } from "./rendered_surface.js";
import { eventCell, shouldIgnoreSyntheticClick } from "./input_support.js";

const boot = window.__SWIMMERS_BOOT__ ?? {
  franken_term_available: false,
  franken_term_js_url: "",
  franken_term_wasm_url: "",
  franken_term_font_url: "",
  franken_term_asset_info: null,
  follow_published_selection: false,
  focus_layout: false,
};

const TOKEN_STORAGE_KEY = "swimmers.web.token";
const SESSION_STORAGE_KEY = "swimmers.web.session";
const DIR_BROWSER_PATH_KEY = "swimmers.web.dirs.path";
const DIR_BROWSER_MANAGED_ONLY_KEY = "swimmers.web.dirs.managed";
const TERMINAL_ZOOM_STORAGE_KEY = "swimmers.web.terminalZoom";
const SEND_HISTORY_KEY = "swimmers.web.send.history";
const SESSION_REFRESH_MS = 2500;
const SESSION_REFRESH_STREAMING_MS = 10000;
const SNAPSHOT_REFRESH_MS = 900;
const AGENT_CONTEXT_REFRESH_MS = 5000;
const SURFACE_CLICK_SUPPRESS_MS = 450;
const TROGDOR_READ_PROGRESS_KEY = "swimmers.web.trogdor.readProgress";
const TROGDOR_BURN_MS = 1100;
const TROGDOR_DRAGON_ASSET_BASE = "/assets/dragon";
const TROGDOR_DRAGON_TARGET = { x: 56, y: 64 };
const TROGDOR_DRAGON_FIRE_STAGES = ["short", "mid", "full"];
// 8-way body sprite filenames (without `.png`). Match the on-disk assets.
const TROGDOR_DRAGON_BODY_FRAMES = [
  "front",
  "3q-right",
  "right",
  "back-right",
  "back",
  "back-left",
  "left",
  "3q-left",
];
// atan2 sector → body frame. Sector 0 = +x (right), sector 2 = +y on canvas
// (down / toward-camera = "front"). Two sectors map to "left" because the angle
// ±π collapses. Mirrors the prototype's `dirIndexFromVec`.
const TROGDOR_DRAGON_FRAME_BY_SECTOR = {
  "2": "front",
  "1": "3q-right",
  "0": "right",
  "-1": "back-right",
  "-2": "back",
  "-3": "back-left",
  "-4": "left",
  "4": "left",
  "3": "3q-left",
};
const TERMINAL_ZOOM_MIN = 0.65;
const TERMINAL_ZOOM_MAX = 2.4;
const TERMINAL_ZOOM_STEP = 0.1;
const SEND_HISTORY_LIMIT = 8;
const MAX_TERMINAL_PASTE_BYTES = 786432;
const MAX_PENDING_TERMINAL_BYTES = 524288;
const MERMAID_SOURCE_DISPLAY_MAX_CHARS = 64 * 1024;
const MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS = 128 * 1024;
const MERMAID_PLAN_FILES_MAX = 32;
const TERMINAL_OUTPUT_OPCODE = 0x11;
const FRANKENTERM_REQUIRED_INSTANCE_METHODS = [
  "init",
  "destroy",
  "fitToContainer",
  "resize",
  "render",
];
const FRANKENTERM_HUD_METHODS = [
  ...FRANKENTERM_REQUIRED_INSTANCE_METHODS,
  "applyPatchBatchFlat",
];
const FRANKENTERM_TERMINAL_METHODS = [
  ...FRANKENTERM_REQUIRED_INSTANCE_METHODS,
  "feed",
  "input",
  "drainEncodedInputBytes",
];
const FALLBACK_THOUGHT_BACKENDS = [
  { key: "", label: "auto" },
  { key: "openrouter", label: "openrouter" },
  { key: "grok", label: "grok" },
];

const state = {
  token: "",
  sessions: [],
  operatorPressureBySession: new Map(),
  selectedSessionId: null,
  publishedSelection: null,
  followPublishedSelection: Boolean(boot.follow_published_selection),
  readOnly: false,
  frankenModule: null,
  frankenInit: null,
  frankenFontInit: null,
  frankenLoadError: "",
  frankenAssetSummary: "",
  hud: null,
  terminal: null,
  terminalAcceptsBytes: true,
  terminalSessionId: null,
  ws: null,
  lastTerminalSeqBySession: new Map(),
  pendingTerminalByteChunks: [],
  pendingTerminalByteLength: 0,
  connectionGeneration: 0,
  reconnectTimer: null,
  reconnectAttempt: 0,
  inputSequence: 0,
  pendingInputMessages: new Map(),
  terminalWorkbenchOpen: true,
  agentContextSessionId: null,
  agentContextLoading: false,
  agentContextPayload: null,
  agentContextError: "",
  agentContextRequestSeq: 0,
  agentContextLastLoadedAt: 0,
  workbenchWidgets: {
    sessionId: null,
    loading: false,
    timeline: null,
    skills: null,
    paneTail: null,
    transcript: null,
    transcriptTurnId: "",
    transcriptNextCursor: 0,
    artifact: null,
    gitDiff: null,
    error: "",
    requestSeq: 0,
    lastLoadedAt: 0,
    lastHtml: "",
  },
  workbenchLogMode: "lens",
  workbenchLogFilter: "all",
  workbenchLogSearch: "",
  workbenchSelectedTurnId: "",
  refreshTimer: null,
  snapshotTimer: null,
  terminalPaintProbeTimer: null,
  renderQueued: false,
  terminalFallbackActive: false,
  terminalFallbackAutoFollow: true,
  terminalMirrorText: "",
  terminalPaintVerified: false,
  terminalFrameBytesSeen: 0,
  rendererDiagnosticSequence: 0,
  lastRendererDiagnostic: null,
  lastRendererDiagnosticError: "",
  currentCols: 80,
  currentRows: 24,
  terminalZoom: 1,
  mobileKeyboardActive: false,
  searchQuery: "",
  searchState: null,
  selectMode: false,
  selectionAnchor: null,
  selectionFocus: null,
  hoveredLinkUrl: "",
  sendHistory: [],
  paletteItems: [],
  paletteIndex: 0,
  utilityMessageTimer: null,
  connectionLabel: "disconnected",
  connectionMuted: false,
  modeLabel: "auth unknown",
  modeMuted: true,
  searchLabel: "Search idle",
  searchMuted: true,
  utilityLabel: "Cmd/Ctrl-click a terminal link to open it.",
  utilityMuted: true,
  backendHealth: null,
  surfaceZones: [],
  surfaceMasks: [],
  surfaceClickSuppressUntil: 0,
  hoveredTrogdorSessionId: null,
  trogdorAtlasOpen: true,
  trogdorWpm: 200,
  trogdorReading: true,
  trogdorReaderStartedAt: 0,
  trogdorReaderStartIndex: 0,
  trogdorReaderClawgKey: "",
  trogdorReaderTimer: null,
  trogdorSurfaceSignature: "",
  trogdorReadProgress: {},
  trogdorDismissedClawgs: {},
  trogdorBurntSessions: new Map(),
  trogdorAwaitingSessionIds: new Set(),
  sendTarget: null,
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
    groups: [],
    group: "",
    search: "",
    overlayLabel: "",
    launchTargets: [],
    launchTarget: "local",
    batchSelected: new Set(),
    status: "",
    error: "",
  },
  mermaidArtifact: {
    loading: false,
    sessionId: null,
    artifact: null,
    svg: "",
    source: "",
    planFiles: [],
    activePlanFile: "",
    planContent: "",
    status: "",
    error: "",
  },
};

const defaultDocumentTitle = document.title || "swimmers";

const el = {
  terminalStage: document.getElementById("terminal-stage"),
  terminalCanvas: document.getElementById("terminal-canvas"),
  hudCanvas: document.getElementById("hud-canvas"),
  terminalFallback: document.getElementById("terminal-fallback"),
  terminalA11yMirror: document.getElementById("terminal-a11y-mirror"),
  terminalAnnouncer: document.getElementById("terminal-announcer"),
  terminalStatusStrip: document.getElementById("terminal-status-strip"),
  terminalLinkTools: document.getElementById("terminal-link-tools"),
  terminalLinkText: document.getElementById("terminal-link-text"),
  terminalLinkOpen: document.getElementById("terminal-link-open"),
  terminalLinkCopy: document.getElementById("terminal-link-copy"),
  loadingOverlay: document.getElementById("loading-overlay"),
  loadingLabel: document.getElementById("loading-label"),
  mobileKeyboardProxy: document.getElementById("mobile-kb-proxy"),
  terminalControlStrip: document.getElementById("terminal-control-strip"),
  terminalPalette: document.getElementById("terminal-palette"),
  terminalCopyFrame: document.getElementById("terminal-copy-frame"),
  terminalZoomOut: document.getElementById("terminal-zoom-out"),
  terminalZoomReset: document.getElementById("terminal-zoom-reset"),
  terminalZoomIn: document.getElementById("terminal-zoom-in"),
  terminalMobileKeyboard: document.getElementById("terminal-mobile-keyboard"),
  terminalTrogdorBack: document.getElementById("terminal-trogdor-back"),
  terminalWorkbenchToggle: document.getElementById("terminal-workbench-toggle"),
  terminalWorkbench: document.getElementById("terminal-workbench"),
  terminalWorkbenchTitle: document.getElementById("terminal-workbench-title"),
  terminalWorkbenchMeta: document.getElementById("terminal-workbench-meta"),
  terminalWorkbenchStatus: document.getElementById("terminal-workbench-status"),
  terminalWorkbenchTask: document.getElementById("terminal-workbench-task"),
  terminalWorkbenchCurrent: document.getElementById("terminal-workbench-current"),
  terminalWorkbenchPressure: document.getElementById("terminal-workbench-pressure"),
  terminalWorkbenchActions: document.getElementById("terminal-workbench-actions"),
  terminalWorkbenchWidgets: document.getElementById("terminal-workbench-widgets"),
  terminalWorkbenchRefresh: document.getElementById("terminal-workbench-refresh"),
  terminalInputDock: document.getElementById("terminal-input-dock"),
  terminalInlineInput: document.getElementById("terminal-inline-input"),
  terminalInputSend: document.getElementById("terminal-input-send"),
  terminalInputEcho: document.getElementById("terminal-input-echo"),
  terminalKeyStrip: document.getElementById("terminal-key-strip"),
  trogdorSurface: document.getElementById("trogdor-surface"),
  trogdorLauncher: document.getElementById("trogdor-launcher"),
  modalRoot: document.getElementById("modal-root"),
  modalBackdrop: document.getElementById("modal-backdrop"),
  paletteSheet: document.getElementById("palette-sheet"),
  paletteSearch: document.getElementById("palette-search"),
  paletteResults: document.getElementById("palette-results"),
  paletteCloseButton: document.getElementById("palette-close-button"),
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
  sendSheetTitle: document.getElementById("send-sheet-title"),
  sendForm: document.getElementById("send-form"),
  sendMode: document.getElementById("send-mode"),
  sendInput: document.getElementById("send-input"),
  sendHistory: document.getElementById("send-history"),
  sendHint: document.getElementById("send-hint"),
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
  createLaunchTarget: document.getElementById("create-launch-target"),
  createRequest: document.getElementById("create-request"),
  createButton: document.getElementById("create-button"),
  createCloseButton: document.getElementById("create-close-button"),
  dirsSummary: document.getElementById("dirs-summary"),
  dirsManagedOnly: document.getElementById("dirs-managed-only"),
  dirsSearch: document.getElementById("dirs-search"),
  dirsPath: document.getElementById("dirs-path"),
  dirsLoadButton: document.getElementById("dirs-load-button"),
  dirsUpButton: document.getElementById("dirs-up-button"),
  dirsSpawnHere: document.getElementById("dirs-spawn-here"),
  dirsList: document.getElementById("dirs-list"),
  createBatchBar: document.getElementById("create-batch-bar"),
  createBatchCount: document.getElementById("create-batch-count"),
  createBatchTool: document.getElementById("create-batch-tool"),
  createBatchPreview: document.getElementById("create-batch-preview"),
  createBatchClear: document.getElementById("create-batch-clear"),
  createBatchVisible: document.getElementById("create-batch-visible"),
  createBatchSubmit: document.getElementById("create-batch-submit"),
  mermaidSheet: document.getElementById("mermaid-sheet"),
  mermaidSummary: document.getElementById("mermaid-summary"),
  mermaidPreview: document.getElementById("mermaid-preview"),
  mermaidSource: document.getElementById("mermaid-source"),
  mermaidPlanTabs: document.getElementById("mermaid-plan-tabs"),
  mermaidPlanContent: document.getElementById("mermaid-plan-content"),
  mermaidRefreshButton: document.getElementById("mermaid-refresh-button"),
  mermaidOpenButton: document.getElementById("mermaid-open-button"),
  mermaidCloseButton: document.getElementById("mermaid-close-button"),
};

function currentSession() {
  return state.sessions.find((session) => session.session_id === state.selectedSessionId) ?? null;
}

function sessionDisplayName(session) {
  return String(session?.tmux_name || session?.name || session?.session_id || "session");
}

function sessionNeedsAttention(session) {
  if (!session) {
    return false;
  }
  const stateLabel = String(session.state || "").toLowerCase();
  return stateLabel === "attention" || rawSessionAwaitingUser(session);
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

function assertFrankenTermModule(mod) {
  if (!mod || typeof mod.default !== "function") {
    throw new Error("FrankenTerm module is missing its wasm initializer");
  }
  if (typeof mod.FrankenTermWeb !== "function") {
    throw new Error("FrankenTerm module is missing FrankenTermWeb");
  }
  return mod;
}

function validateFrankenTermSurface(surface, requiredMethods, label = "FrankenTerm surface") {
  const missing = requiredMethods.filter((methodName) => !surfaceSupports(surface, methodName));
  if (missing.length) {
    throw new Error(`${label} missing methods: ${missing.join(", ")}`);
  }
  return surface;
}

function frankenTermAssetSummary() {
  const info = boot.franken_term_asset_info;
  if (!info || typeof info !== "object") {
    return "";
  }
  const pieces = [];
  for (const key of ["js", "wasm", "font"]) {
    const item = info[key];
    if (!item) {
      continue;
    }
    const checksum = item.checksum ? ` ${item.checksum}` : "";
    const size = Number.isFinite(item.size_bytes) ? ` ${item.size_bytes}b` : "";
    pieces.push(`${key}${checksum}${size}`);
  }
  return pieces.join("; ");
}

function isLoopbackHostname(hostname) {
  const host = String(hostname || "").trim().toLowerCase().replace(/^\[|\]$/g, "");
  if (!host) {
    return false;
  }
  if (host === "localhost" || host.endsWith(".localhost") || host === "::1") {
    return true;
  }
  const ipv4 = host.split(".");
  return (
    ipv4.length === 4 &&
    ipv4[0] === "127" &&
    ipv4.every((part) => /^\d+$/.test(part) && Number(part) >= 0 && Number(part) <= 255)
  );
}

function frankenTermLinkPolicy() {
  return {
    allowHttp: isLoopbackHostname(window.location?.hostname),
    allowHttps: true,
  };
}

function utf8ByteLength(text) {
  const value = String(text ?? "");
  if (typeof TextEncoder !== "undefined") {
    return new TextEncoder().encode(value).byteLength;
  }
  let count = 0;
  for (const char of value) {
    const code = char.codePointAt(0);
    if (code <= 0x7f) {
      count += 1;
    } else if (code <= 0x7ff) {
      count += 2;
    } else if (code <= 0xffff) {
      count += 3;
    } else {
      count += 4;
    }
  }
  return count;
}

function terminalTextWithinPasteBudget(text) {
  return utf8ByteLength(text) <= MAX_TERMINAL_PASTE_BYTES;
}

function rejectOversizeTerminalText(text, label = "Paste") {
  const bytes = utf8ByteLength(text);
  if (bytes <= MAX_TERMINAL_PASTE_BYTES) {
    return false;
  }
  setUtilityStatus(`${label} blocked: ${bytes} bytes exceeds ${MAX_TERMINAL_PASTE_BYTES}.`, true, 3200);
  return true;
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
        : backend.key === "grok"
          ? "uses Grok CLI default unless a model is set"
          : "auto backend uses daemon default model",
    model_presets: backend.key === "openrouter"
      ? ["", "openrouter/free", "nvidia/nemotron-3-super-120b-a12b:free", "arcee-ai/trinity-large-preview:free"]
      : backend.key === "grok"
        ? [""]
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
  if (key === "claude" || key === "claude-cli" || key === "claude_cli") return "grok";
  if (key === "codex" || key === "codex-cli" || key === "codex_cli") return "grok";
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
  if (key === "grok") {
    return trimmed;
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
  syncTerminalStatusStrip();
  renderHudSurface();
}

function nextInputMessageId() {
  state.inputSequence += 1;
  return `web-${Date.now()}-${state.inputSequence}`;
}

function updateInputDeliveryStatus(id, status, detail = "") {
  if (!id) {
    return;
  }
  const pending = state.pendingInputMessages.get(id) ?? {};
  state.pendingInputMessages.set(id, { ...pending, status, detail });
  if (status === "pending") {
    setTerminalInputEcho(`pending: ${pending.text || ""}`);
    return;
  }
  if (status === "sent") {
    setTerminalInputEcho(`sent: ${pending.text || ""}`);
    return;
  }
  setTerminalInputEcho(`failed: ${detail || pending.text || "input not delivered"}`);
}

function clearReconnectTimer() {
  if (state.reconnectTimer) {
    window.clearTimeout(state.reconnectTimer);
    state.reconnectTimer = null;
  }
}

function reconnectDelayMs() {
  const attempt = Math.max(0, state.reconnectAttempt);
  return Math.min(10000, 1000 * 2 ** Math.min(attempt, 3));
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
    return state.ws?.readyState === WebSocket.OPEN ? "fallback live" : "snapshot fallback";
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
  document.body.classList.toggle("backend-health-degraded", Boolean(healthWarning));
  syncDocumentLifecycleSignal();
}

function conciseHealthDetail(value) {
  const text = String(value || "").trim();
  if (!text) {
    return "";
  }
  return text.length > 64 ? `${text.slice(0, 61)}...` : text;
}

function backendHealthWarningText(health) {
  if (!health || typeof health !== "object") {
    return "";
  }
  const persistence = health.persistence || {};
  if (!persistence.available) {
    return "persistence unavailable";
  }
  if (!persistence.ok) {
    const operation = persistence.last_failed_operation || "write";
    const detail = conciseHealthDetail(persistence.last_error);
    return `persistence degraded: ${operation}${detail ? `: ${detail}` : ""}`;
  }
  const thought = health.thought_bridge || {};
  const status = String(thought.status || "").toLowerCase();
  if (!status || status === "healthy") {
    return "";
  }
  if (status === "degraded") {
    const detail = conciseHealthDetail(thought.last_backend_error || thought.last_error);
    return `thought bridge degraded${detail ? `: ${detail}` : ""}`;
  }
  if (status === "unhealthy") {
    const detail = conciseHealthDetail(thought.shutdown_reason || thought.last_error);
    return `thought bridge unhealthy${detail ? `: ${detail}` : ""}`;
  }
  return `thought bridge ${status}`;
}

function applyBackendHealth(payload) {
  state.backendHealth = payload && typeof payload === "object" ? payload : null;
  syncTerminalStatusStrip();
  renderHudSurface();
}

function syncDocumentLifecycleSignal() {
  const session = currentSession();
  const attention = sessionNeedsAttention(session);
  document.body.classList.toggle("session-attention", attention);
  if (attention && session) {
    document.title = `(!) ${sessionDisplayName(session)} - swimmers`;
  } else {
    document.title = defaultDocumentTitle;
  }
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
  const previous = state.selectedSessionId;
  state.selectedSessionId = normalized;
  if (previous !== normalized) {
    resetAgentContextForSession(normalized);
    resetWorkbenchWidgetsForSession(normalized);
  }
  if (normalized) {
    localStorage.setItem(SESSION_STORAGE_KEY, normalized);
    closeTrogdorAtlasForTerminal();
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

function terminalZoomSupported() {
  return terminalSupports("setZoom") || surfaceSupports(state.hud, "setZoom");
}

function normalizeTerminalZoom(value) {
  const numeric = Number.parseFloat(value);
  if (!Number.isFinite(numeric)) {
    return 1;
  }
  const stepped = Math.round(numeric / TERMINAL_ZOOM_STEP) * TERMINAL_ZOOM_STEP;
  return Math.max(TERMINAL_ZOOM_MIN, Math.min(TERMINAL_ZOOM_MAX, stepped));
}

function loadTerminalZoom(url) {
  const fromUrl = url.searchParams.get("zoom");
  if (fromUrl !== null) {
    return normalizeTerminalZoom(fromUrl);
  }
  return normalizeTerminalZoom(localStorage.getItem(TERMINAL_ZOOM_STORAGE_KEY) || "1");
}

function terminalZoomLabel() {
  return `${Math.round(state.terminalZoom * 100)}%`;
}

function syncTerminalZoomControls() {
  if (!el.terminalControlStrip) {
    return;
  }
  const supported = terminalZoomSupported() || !state.terminal;
  el.terminalZoomOut.disabled = !supported || state.terminalZoom <= TERMINAL_ZOOM_MIN + 0.001;
  el.terminalZoomIn.disabled = !supported || state.terminalZoom >= TERMINAL_ZOOM_MAX - 0.001;
  el.terminalZoomReset.disabled = !supported || Math.abs(state.terminalZoom - 1) < 0.001;
  el.terminalZoomReset.textContent = terminalZoomLabel();
  el.terminalMobileKeyboard.disabled = state.readOnly || !currentSession();
  el.terminalMobileKeyboard.setAttribute("aria-pressed", state.mobileKeyboardActive ? "true" : "false");
  syncTerminalInputDock();
  if (el.terminalCopyFrame) {
    el.terminalCopyFrame.disabled = !currentSession();
  }
}

function terminalInputDockVisible() {
  return Boolean(currentSession() && !state.trogdorAtlasOpen);
}

function syncTerminalInputDock() {
  if (!el.terminalInputDock) {
    return;
  }
  const visible = terminalInputDockVisible();
  document.body.classList.toggle("terminal-input-dock-visible", visible);
  el.terminalInputDock.classList.toggle("hidden", !visible);
  el.terminalInputDock.setAttribute("aria-hidden", visible ? "false" : "true");
  el.terminalInlineInput.disabled = !visible || state.readOnly;
  if (el.terminalKeyStrip) {
    for (const button of el.terminalKeyStrip.querySelectorAll("button[data-terminal-key]")) {
      button.disabled = !visible || state.readOnly;
    }
  }
  const hasText = Boolean(String(el.terminalInlineInput.value || "").trim());
  el.terminalInputSend.disabled = !visible || state.readOnly || !hasText;
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

function resetAgentContextForSession(sessionId) {
  state.agentContextSessionId = normalizeSessionId(sessionId);
  state.agentContextLoading = false;
  state.agentContextPayload = null;
  state.agentContextError = "";
  state.agentContextLastLoadedAt = 0;
  renderTerminalWorkbench();
}

function resetWorkbenchWidgetsForSession(sessionId) {
  state.workbenchWidgets.sessionId = normalizeSessionId(sessionId);
  state.workbenchWidgets.loading = false;
  state.workbenchWidgets.timeline = null;
  state.workbenchWidgets.skills = null;
  state.workbenchWidgets.paneTail = null;
  state.workbenchWidgets.transcript = null;
  state.workbenchWidgets.transcriptTurnId = "";
  state.workbenchWidgets.transcriptNextCursor = 0;
  state.workbenchWidgets.artifact = null;
  state.workbenchWidgets.gitDiff = null;
  state.workbenchWidgets.error = "";
  state.workbenchWidgets.lastLoadedAt = 0;
  state.workbenchWidgets.lastHtml = "";
  state.workbenchLogMode = "lens";
  state.workbenchLogFilter = "all";
  state.workbenchLogSearch = "";
  state.workbenchSelectedTurnId = "";
  renderWorkbenchWidgets();
}

function terminalWorkbenchVisible() {
  return Boolean(currentSession() && !state.trogdorAtlasOpen && state.terminalWorkbenchOpen);
}

function syncTrogdorBackButton() {
  if (!el.terminalTrogdorBack) {
    return;
  }
  const visible = Boolean(currentSession() && !state.trogdorAtlasOpen);
  el.terminalTrogdorBack.classList.toggle("hidden", !visible);
  el.terminalTrogdorBack.disabled = !visible;
  el.terminalTrogdorBack.setAttribute("aria-hidden", visible ? "false" : "true");
}

function syncTerminalWorkbench() {
  const hasSession = Boolean(currentSession() && !state.trogdorAtlasOpen);
  const visible = terminalWorkbenchVisible();
  document.body.classList.toggle("terminal-workbench-open", visible);
  if (el.terminalWorkbenchToggle) {
    el.terminalWorkbenchToggle.disabled = !hasSession;
    el.terminalWorkbenchToggle.setAttribute("aria-pressed", visible ? "true" : "false");
  }
  if (el.terminalWorkbench) {
    el.terminalWorkbench.classList.toggle("hidden", !visible);
    el.terminalWorkbench.setAttribute("aria-hidden", visible ? "false" : "true");
  }
  renderTerminalWorkbench();
}

function setTerminalWorkbenchOpen(open) {
  state.terminalWorkbenchOpen = Boolean(open);
  syncTerminalWorkbench();
  if (state.terminalWorkbenchOpen) {
    void refreshAgentContextForSelectedSession({ force: true });
    void refreshWorkbenchWidgetsForSelectedSession({ force: true });
  }
}

function selectedAgentContextPayload() {
  return state.agentContextSessionId === state.selectedSessionId
    ? state.agentContextPayload
    : null;
}

function agentActionLabel(action) {
  if (!action) {
    return "";
  }
  const tool = String(action.tool || "action").trim() || "action";
  const detail = String(action.detail || "").trim();
  return detail ? `${tool}: ${detail}` : tool;
}

function truncateWorkbenchText(value, max = 180) {
  const normalized = String(value || "").replace(/\s+/g, " ").trim();
  if (normalized.length <= max) {
    return normalized;
  }
  return `${normalized.slice(0, Math.max(0, max - 3))}...`;
}

function operatorPressureSummary(session, payload) {
  if (!session) {
    return "No pressure cues.";
  }
  const cues = [];
  const actionCues = Array.isArray(session.action_cues) ? session.action_cues : [];
  for (const cue of actionCues.slice(0, 3)) {
    const kind = String(cue?.kind || "").replace(/_/g, " ").trim();
    if (kind) {
      cues.push(kind);
    }
  }
  if (session.state && session.state !== "idle") {
    cues.push(String(session.state).replace(/_/g, " "));
  }
  if (session.transport_health && session.transport_health !== "healthy") {
    cues.push(`transport ${String(session.transport_health).replace(/_/g, " ")}`);
  }
  if (session.is_stale) {
    cues.push("stale registry");
  }
  const attached = Number(session.attached_clients || 0);
  const staleAttached = Number(session.stale_attached_clients || 0);
  if (attached || staleAttached) {
    cues.push(`${attached} attached${staleAttached ? `, ${staleAttached} stale` : ""}`);
  }
  const tokens = Number(payload?.token_count ?? session.token_count ?? 0);
  const limit = Number(payload?.context_limit ?? session.context_limit ?? 0);
  if (tokens > 0 && limit > 0) {
    const pct = Math.min(999, Math.round((tokens / limit) * 100));
    cues.push(`${pct}% context`);
  }
  return cues.length ? cues.slice(0, 5).join(" · ") : "No pressure cues.";
}

function renderTerminalWorkbench() {
  if (!el.terminalWorkbench) {
    return;
  }

  const session = currentSession();
  const payload = selectedAgentContextPayload();
  const tool = payload?.tool || session?.tool || "unknown";
  const cwd = payload?.cwd || session?.cwd || "";
  const status = state.agentContextLoading
    ? "loading context"
    : state.agentContextError
      ? state.agentContextError
      : payload?.available
        ? "structured context"
        : payload?.message || "waiting for context";
  const task = payload?.user_task || summarizeThought(session);
  const current = agentActionLabel(payload?.current_tool) || "No current action.";
  const pressure = operatorPressureSummary(session, payload);
  const actions = Array.isArray(payload?.recent_actions) ? payload.recent_actions : [];

  el.terminalWorkbenchTitle.textContent = session ? sessionDisplayName(session) : "No session";
  el.terminalWorkbenchMeta.textContent = session ? `${tool} · ${cwd}` : "";
  el.terminalWorkbenchStatus.textContent = status;
  el.terminalWorkbenchTask.textContent = truncateWorkbenchText(task || "No task context.");
  el.terminalWorkbenchCurrent.textContent = truncateWorkbenchText(current, 140);
  el.terminalWorkbenchPressure.textContent = truncateWorkbenchText(pressure, 160);
  el.terminalWorkbenchRefresh.disabled = !session || state.agentContextLoading;

  if (!actions.length) {
    el.terminalWorkbenchActions.innerHTML = `<li class="workbench-action"><span class="workbench-action-detail">${escapeHtml(payload?.available ? "No recent actions." : "No structured actions.")}</span></li>`;
    renderWorkbenchWidgets();
    return;
  }

  el.terminalWorkbenchActions.innerHTML = actions
    .slice(0, 6)
    .map((action) => {
      const toolLabel = truncateWorkbenchText(action?.tool || "action", 44);
      const detail = truncateWorkbenchText(action?.detail || "", 160);
      return `
        <li class="workbench-action">
          <span class="workbench-action-tool">${escapeHtml(toolLabel)}</span>
          <span class="workbench-action-detail">${escapeHtml(detail || "No detail.")}</span>
        </li>
      `;
    })
    .join("");
  renderWorkbenchWidgets();
}

async function refreshAgentContextForSelectedSession(options = {}) {
  const session = currentSession();
  if (!session || state.trogdorAtlasOpen) {
    state.agentContextLoading = false;
    renderTerminalWorkbench();
    return;
  }

  const sessionId = session.session_id;
  const now = Date.now();
  const hasCurrentPayload =
    state.agentContextSessionId === sessionId && Boolean(state.agentContextPayload);
  if (
    options.throttle &&
    hasCurrentPayload &&
    now - state.agentContextLastLoadedAt < AGENT_CONTEXT_REFRESH_MS
  ) {
    return;
  }
  if (state.agentContextLoading && !options.force) {
    return;
  }

  const requestSeq = state.agentContextRequestSeq + 1;
  state.agentContextRequestSeq = requestSeq;
  state.agentContextSessionId = sessionId;
  state.agentContextError = "";
  state.agentContextLoading = !options.silent || !hasCurrentPayload;
  renderTerminalWorkbench();

  try {
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(sessionId)}/agent-context`);
    const payload = await response.json();
    if (requestSeq !== state.agentContextRequestSeq || state.selectedSessionId !== sessionId) {
      return;
    }
    state.agentContextPayload = payload;
    state.agentContextError = "";
    state.agentContextLastLoadedAt = Date.now();
  } catch (error) {
    if (requestSeq !== state.agentContextRequestSeq || state.selectedSessionId !== sessionId) {
      return;
    }
    state.agentContextPayload = null;
    state.agentContextError = error?.message || "context unavailable";
  } finally {
    if (requestSeq === state.agentContextRequestSeq) {
      state.agentContextLoading = false;
      renderTerminalWorkbench();
    }
  }
}

function selectedWorkbenchWidgets() {
  return state.workbenchWidgets.sessionId === state.selectedSessionId
    ? state.workbenchWidgets
    : {
        sessionId: null,
        loading: false,
        timeline: null,
        skills: null,
        paneTail: null,
        transcript: null,
        transcriptTurnId: "",
        transcriptNextCursor: 0,
        artifact: null,
        gitDiff: null,
        error: "",
        requestSeq: state.workbenchWidgets.requestSeq,
        lastLoadedAt: state.workbenchWidgets.lastLoadedAt,
      };
}

function tailLineCount(text) {
  const trimmed = String(text || "").trimEnd();
  return trimmed ? trimmed.split(/\n/).length : 0;
}

function widgetTextExcerpt(text, max = 4200) {
  const normalized = String(text || "").replace(/\r/g, "");
  if (normalized.length <= max) {
    return normalized;
  }
  return `... truncated ...\n${normalized.slice(-max)}`;
}

const WORKBENCH_LOG_KIND_LABELS = {
  all: "All",
  operator: "Chat",
  command: "Command",
  status: "Status",
  diff: "Diff",
  output: "Output",
  truncation: "Trimmed",
};

const WORKBENCH_LOG_FILTERS = ["all", "operator", "command", "status", "diff", "output", "truncation"];

const WORKBENCH_LOG_COMMAND_RE =
  /^(?:cargo|make|git|node|bun|npm|pnpm|yarn|python3?|pytest|uv|xcodebuild|swift|curl|tmux|cat|sed|rg|grep|ls|cd|cp|mv|mkdir|touch|chmod|ssh|docker|kubectl)\b/;

function transcriptLineKind(line) {
  const trimmed = String(line || "").trim();
  if (!trimmed) {
    return "output";
  }
  if (/^\.\.\. truncated \.\.\.$/i.test(trimmed) || /^truncated[:\s]/i.test(trimmed)) {
    return "truncation";
  }
  if (/^(?:[•*]\s+|[-]\s+You\b|You ran\b|Using [a-z][\w-]*\b)/i.test(trimmed)) {
    return "operator";
  }
  if (
    /^(?:diff --git|index [0-9a-f]+\.\.|@@\s|---\s|\+\+\+\s)/.test(trimmed) ||
    /^[+][^+]/.test(trimmed) ||
    /^-[^\-\s]/.test(trimmed)
  ) {
    return "diff";
  }
  if (
    /(?:\berror\b|\bfailed\b|\bfatal\b|\bpanic\b|\bwarning\b|\bdenied\b|\brefused\b|\btimed out\b|\bunavailable\b|\bblocked\b)/i.test(trimmed) ||
    /^(?:Finished|Running|Compiling|Waiting|Worked for|Validation|Evidence|PASS|FAIL)\b/i.test(trimmed) ||
    /^[-]\s+(?:Worked for|Evidence)\b/i.test(trimmed)
  ) {
    return "status";
  }
  if (/^(?:[$#❯>]\s+|[A-Za-z0-9_.~/-]+[$#]\s+)/.test(trimmed) || WORKBENCH_LOG_COMMAND_RE.test(trimmed)) {
    return "command";
  }
  return "output";
}

function renderTranscriptBlocks(text) {
  const lines = String(text || "").replace(/\r/g, "").split("\n");
  const blocks = [];
  let current = null;

  lines.forEach((line, index) => {
    if (!line.trim() && !current) {
      return;
    }
    const kind = transcriptLineKind(line);
    if (current && current.kind === kind) {
      current.lines.push(line);
      current.endLine = index + 1;
      return;
    }
    current = {
      kind,
      label: WORKBENCH_LOG_KIND_LABELS[kind] || "Output",
      lines: [line],
      startLine: index + 1,
      endLine: index + 1,
    };
    blocks.push(current);
  });

  return blocks.filter((block) => block.lines.some((line) => line.trim()));
}

function blockMatchesSearch(block, query) {
  const needle = String(query || "").trim().toLowerCase();
  if (!needle) {
    return true;
  }
  return block.lines.join("\n").toLowerCase().includes(needle);
}

function renderHighlightedLogLine(line, query) {
  const text = String(line || "");
  const needle = String(query || "").trim();
  if (!needle) {
    return escapeHtml(text || " ");
  }

  const lower = text.toLowerCase();
  const lowerNeedle = needle.toLowerCase();
  let cursor = 0;
  let html = "";
  while (cursor < text.length) {
    const index = lower.indexOf(lowerNeedle, cursor);
    if (index < 0) {
      html += escapeHtml(text.slice(cursor));
      break;
    }
    html += escapeHtml(text.slice(cursor, index));
    html += `<mark class="workbench-log-mark">${escapeHtml(text.slice(index, index + needle.length))}</mark>`;
    cursor = index + needle.length;
  }
  return html || escapeHtml(text || " ");
}

function workbenchLogCounts(blocks) {
  return blocks.reduce((counts, block) => {
    counts[block.kind] = (counts[block.kind] || 0) + 1;
    return counts;
  }, {});
}

function parseJsonObject(text) {
  try {
    const parsed = JSON.parse(String(text || ""));
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

function parseNestedJsonObject(value) {
  if (value && typeof value === "object") {
    return value;
  }
  if (typeof value !== "string" || !value.trim().startsWith("{")) {
    return null;
  }
  return parseJsonObject(value);
}

function compactJsonValue(value, limit = 360) {
  if (value === undefined || value === null) {
    return "";
  }
  try {
    const text = typeof value === "string" ? value : JSON.stringify(value);
    return truncateWorkbenchText(String(text || "").replace(/\r/g, "").trim(), limit);
  } catch {
    return "";
  }
}

function payloadTextContent(value) {
  if (typeof value === "string") {
    return value;
  }
  if (Array.isArray(value)) {
    return value
      .map((block) => {
        if (typeof block === "string") {
          return block;
        }
        if (block?.type === "tool_use") {
          const input = compactJsonValue(block.input, 300);
          return [block.name || "tool_use", input].filter(Boolean).join(": ");
        }
        if (block?.type === "tool_result") {
          return payloadTextContent(block.content) || compactJsonValue(block, 300);
        }
        if (block?.type === "thinking") {
          return block.thinking || "";
        }
        if (typeof block?.text === "string") {
          return block.text;
        }
        if (typeof block?.content === "string") {
          return block.content;
        }
        return payloadTextContent(block?.content);
      })
      .map((part) => String(part || "").trim())
      .filter(Boolean)
      .join("\n");
  }
  if (value && typeof value === "object") {
    return compactJsonValue(value);
  }
  return "";
}

function transcriptRecordEnvelope(record) {
  const raw = String(record?.raw || "").trim();
  const parsed = parseJsonObject(raw);
  const message = parsed?.message && typeof parsed.message === "object" ? parsed.message : null;
  const payload = parsed?.payload && typeof parsed.payload === "object"
    ? parsed.payload
    : message || (parsed && typeof parsed === "object" ? parsed : {});
  return { raw, parsed, payload, message };
}

function payloadToolUseBlock(payload) {
  const content = Array.isArray(payload?.content) ? payload.content : [];
  return content.find((block) => block && typeof block === "object" && block.type === "tool_use") || null;
}

function payloadToolResultBlock(payload) {
  const content = Array.isArray(payload?.content) ? payload.content : [];
  return content.find((block) => block && typeof block === "object" && block.type === "tool_result") || null;
}

function readableRecordSummary(record, raw) {
  const summary = String(record?.summary || "").trim();
  if (!summary || summary === raw || /^[\[{]/.test(summary)) {
    return "";
  }
  return summary;
}

function compactRecordFields(value) {
  if (!value || typeof value !== "object") {
    return "";
  }
  const skipped = new Set(["payload", "message", "content", "signature", "thinking"]);
  return Object.entries(value)
    .filter(([key, entry]) => !skipped.has(key) && entry !== undefined && entry !== null && entry !== "")
    .slice(0, 5)
    .map(([key, entry]) => `${key}: ${compactJsonValue(entry, 160)}`)
    .join("\n");
}

function payloadMessageText(payload) {
  if (typeof payload?.message === "string") {
    return payload.message;
  }
  if (payload?.message && typeof payload.message === "object") {
    return payloadTextContent(payload.message.content) || compactRecordFields(payload.message);
  }
  return "";
}

function transcriptRecordIsCall(kind) {
  return /^(function_call|custom_tool_call)$/.test(String(kind || ""));
}

function transcriptRecordIsCallOutput(kind) {
  return /^(function_call_output|custom_tool_call_output)$/.test(String(kind || ""));
}

function transcriptRecordDisplayKind(record, payload) {
  const kind = String(record?.kind || payload?.type || "record");
  if (payloadToolResultBlock(payload)) {
    return "output";
  }
  if (transcriptRecordIsCallOutput(kind)) {
    return "output";
  }
  if (payloadToolUseBlock(payload)) {
    return "command";
  }
  if (transcriptRecordIsCall(kind)) {
    return "command";
  }
  if (/agent_message|assistant_message|message|user_message/.test(kind)) {
    return "operator";
  }
  if (/token_count|session_meta|turn_context|compacted|patch_apply/.test(kind)) {
    return "status";
  }
  if (/diff|patch/.test(kind)) {
    return "diff";
  }
  if (record?.truncated) {
    return "truncation";
  }
  return transcriptLineKind(record?.summary || record?.raw || "");
}

function transcriptRecordMeta(record, parsed) {
  const pieces = [];
  const source = String(record?.source || "").trim();
  const rawType = String(parsed?.type || "").trim();
  const cursor = Number(record?.byte_start || 0);
  if (source) {
    pieces.push(source);
  }
  if (rawType) {
    pieces.push(rawType);
  }
  if (cursor > 0) {
    pieces.push(`@${cursor}`);
  }
  if (record?.truncated) {
    pieces.push("trimmed");
  }
  return pieces.join(" · ");
}

function transcriptRecordDisplay(record) {
  const { raw, parsed, payload } = transcriptRecordEnvelope(record);
  const kind = String(record?.kind || payload?.type || parsed?.type || "record");
  const displayKind = transcriptRecordDisplayKind(record, payload);
  const title = kind.replace(/_/g, " ");
  const summary = readableRecordSummary(record, raw);
  const role = record?.role || payload?.role || "";
  const toolUse = payloadToolUseBlock(payload);
  const toolResult = payloadToolResultBlock(payload);
  const fields = [];
  let body = "";

  if (toolUse || transcriptRecordIsCall(kind)) {
    const name = payload?.name || toolUse?.name || summary.split(":")[0] || "tool";
    const args = parseNestedJsonObject(payload?.arguments || payload?.input || toolUse?.input);
    const command = args?.cmd || args?.command || "";
    const workdir = args?.workdir || args?.cwd || "";
    body = command || payloadTextContent(payload?.input || toolUse?.input) || summary || name;
    fields.push(["tool", name]);
    if (workdir) {
      fields.push(["cwd", workdir]);
    }
    if (toolUse?.id) {
      fields.push(["call", toolUse.id]);
    }
  } else if (toolResult || transcriptRecordIsCallOutput(kind)) {
    const output = parseNestedJsonObject(payload?.output || toolResult?.content);
    body =
      output?.output ||
      output?.error ||
      payloadTextContent(payload?.output || toolResult?.content) ||
      summary ||
      "Tool output";
    if (payload?.call_id || toolResult?.tool_use_id) {
      fields.push(["call", payload?.call_id || toolResult.tool_use_id]);
    }
  } else if (/agent_message|assistant_message|message|user_message/.test(kind)) {
    body = payloadMessageText(payload) || payloadTextContent(payload?.content) || summary || "Message";
    if (role) {
      fields.push(["role", role]);
    }
  } else if (/token_count/.test(kind)) {
    const usage = payload?.info?.total_token_usage || payload?.usage || {};
    const contextWindow = payload?.model_context_window || payload?.info?.model_context_window;
    if (usage?.input_tokens !== undefined) {
      fields.push(["input", String(usage.input_tokens)]);
    }
    if (usage?.output_tokens !== undefined) {
      fields.push(["output", String(usage.output_tokens)]);
    }
    if (contextWindow !== undefined) {
      fields.push(["window", String(contextWindow)]);
    }
    body = summary || "Token usage update";
  } else if (/patch_apply/.test(kind)) {
    body = payloadMessageText(payload) || summary || kind.replace(/_/g, " ");
  } else {
    body = payloadMessageText(payload) || payloadTextContent(payload?.content) || summary || compactRecordFields(payload) || kind;
  }

  return {
    id: record?.id || `${kind}-${record?.byte_start || 0}`,
    kind: displayKind,
    label: WORKBENCH_LOG_KIND_LABELS[displayKind] || "Output",
    title,
    meta: transcriptRecordMeta(record, parsed),
    fields,
    body: truncateWorkbenchText(String(body || "").replace(/\r/g, ""), 1400),
    raw,
  };
}

function recordMatchesSearch(record, query) {
  const needle = String(query || "").trim().toLowerCase();
  if (!needle) {
    return true;
  }
  return [record.label, record.title, record.meta, record.body, record.raw]
    .join("\n")
    .toLowerCase()
    .includes(needle);
}

const WORKBENCH_LOG_PATH_RE = /(?:^|[\s"'`([])((?:~\/|\.{1,2}\/|\/)?[A-Za-z0-9_@%+=:.-][A-Za-z0-9_@%+=:./-]*\.(?:c|cc|cpp|css|h|html|js|jsx|json|jsonl|lock|log|md|mjs|mmd|mmdx|py|rs|sh|toml|ts|tsx|txt|wasm|yaml|yml))(?:$|[\s"'`),\]])/g;

function normalizeWorkbenchBriefText(text, limit = 260) {
  return truncateWorkbenchText(String(text || "").replace(/\r/g, "").replace(/\s+/g, " ").trim(), limit);
}

function uniqueNonEmpty(values) {
  const seen = new Set();
  const result = [];
  for (const value of values) {
    const text = String(value || "").trim();
    if (!text || seen.has(text)) {
      continue;
    }
    seen.add(text);
    result.push(text);
  }
  return result;
}

function extractWorkbenchPaths(text) {
  const paths = [];
  const source = String(text || "");
  for (const match of source.matchAll(WORKBENCH_LOG_PATH_RE)) {
    const path = String(match[1] || "").replace(/[;:.,]+$/, "");
    if (path && !path.startsWith("http")) {
      paths.push(path);
    }
  }
  return paths;
}

function workbenchPathScore(path) {
  const text = String(path || "").toLowerCase();
  let score = 0;
  if (text.includes("result")) {
    score += 80;
  }
  if (text.endsWith(".md") || text.endsWith(".mmd") || text.endsWith(".mmdx")) {
    score += 30;
  }
  if (text.startsWith("target/") || text.includes("/target/")) {
    score += 20;
  }
  if (text.startsWith("/")) {
    score += 10;
  }
  return score;
}

function workbenchRecordBody(record) {
  const body = String(record?.body || "").trim();
  if (!body || body === "Message" || body === "Tool output") {
    return "";
  }
  return body;
}

function workbenchRecordRole(record) {
  const roleField = record?.fields?.find?.(([key]) => key === "role");
  return String(roleField?.[1] || "").trim();
}

function workbenchBriefItems(records, options = {}) {
  const items = [];
  const selectedTurnText = normalizeWorkbenchBriefText(options.selectedTurn?.text || "", 220);
  const userRecord = [...records].reverse().find((record) => workbenchRecordRole(record) === "user");
  const userText = selectedTurnText || normalizeWorkbenchBriefText(workbenchRecordBody(userRecord), 220);
  const outcomeRecord = [...records].reverse().find((record) => {
    const body = workbenchRecordBody(record);
    return body && /baked|blocked|complete|done|error|fail|pass|result|summary|written/i.test(body);
  });
  const assistantRecord = [...records].reverse().find((record) => {
    const role = workbenchRecordRole(record);
    const body = workbenchRecordBody(record);
    return body && (role === "assistant" || /assistant|agent/.test(record?.title || ""));
  });
  const fallbackRecord = [...records].reverse().find((record) => workbenchRecordBody(record));
  const outcomeText = normalizeWorkbenchBriefText(
    workbenchRecordBody(outcomeRecord) || workbenchRecordBody(assistantRecord) || workbenchRecordBody(fallbackRecord),
    280,
  );
  const commands = uniqueNonEmpty(
    records
      .filter((record) => record.kind === "command")
      .map((record) => normalizeWorkbenchBriefText(workbenchRecordBody(record), 120)),
  ).slice(0, 3);
  const paths = uniqueNonEmpty(
    records.flatMap((record) => [
      ...extractWorkbenchPaths(record.body),
      ...extractWorkbenchPaths(record.raw),
      ...(record.fields || []).flatMap(([, value]) => extractWorkbenchPaths(value)),
    ]),
  )
    .sort((left, right) => workbenchPathScore(right) - workbenchPathScore(left))
    .slice(0, 4);

  if (userText) {
    items.push(["User turn", userText]);
  }
  if (outcomeText) {
    items.push(["Outcome", outcomeText]);
  }
  if (commands.length) {
    items.push(["Tool actions", commands.join("\n")]);
  }
  if (paths.length) {
    items.push(["Where to read", paths.join("\n")]);
  }
  return items;
}

function renderWorkbenchLogBrief(records, options = {}) {
  const items = workbenchBriefItems(records, options);
  if (!items.length) {
    return "";
  }
  return `
    <section class="workbench-log-brief" aria-label="Log summary">
      <div class="workbench-log-brief-title">Start here</div>
      <div class="workbench-log-brief-items">
        ${items
          .map(
            ([label, value]) => `
              <div class="workbench-log-brief-item">
                <div class="workbench-log-brief-label">${escapeHtml(label)}</div>
                <div class="workbench-log-brief-value">${String(value)
                  .split("\n")
                  .map((line) => `<span>${escapeHtml(line)}</span>`)
                  .join("")}</div>
              </div>
            `,
          )
          .join("")}
      </div>
    </section>
  `;
}

function renderWorkbenchRecordLens(records, options = {}) {
  const parsedRecords = Array.isArray(records) ? records.map(transcriptRecordDisplay) : [];
  const rawText = options.rawText ?? transcriptRecordsToRawText(records);
  const rawExcerpt = widgetTextExcerpt(rawText);
  const rawHasText = Boolean(rawExcerpt.trim());
  const title = options.title || "Post-turn JSONL";
  const emptyText = options.emptyText || "No JSONL records after this turn yet.";
  const counts = workbenchLogCounts(parsedRecords);
  const mode = state.workbenchLogMode === "raw" ? "raw" : "lens";
  const filter = WORKBENCH_LOG_FILTERS.includes(state.workbenchLogFilter) ? state.workbenchLogFilter : "all";
  const query = String(state.workbenchLogSearch || "");
  const filteredRecords = parsedRecords.filter((record) => {
    const kindMatches = filter === "all" || record.kind === filter;
    return kindMatches && recordMatchesSearch(record, query);
  });

  const controls = renderWorkbenchLogControls(filter, query, mode);
  if (mode === "raw") {
    return `
      <div class="workbench-action-detail">${escapeHtml(title)}</div>
      ${controls}
      ${rawHasText ? `<pre class="workbench-log-raw">${escapeHtml(rawExcerpt)}</pre>` : `<div>${escapeHtml(emptyText)}</div>`}
    `;
  }

  const countChips = renderWorkbenchLogCountChips(counts);
  const briefRecords = filter === "all" && !query.trim() ? parsedRecords : filteredRecords;
  const briefHtml = renderWorkbenchLogBrief(briefRecords, options);
  const recordsHtml = !parsedRecords.length
    ? `<div class="workbench-log-empty">${escapeHtml(emptyText)}</div>`
    : filteredRecords.length
      ? filteredRecords.map((record) => renderWorkbenchLogRecord(record, query)).join("")
      : `<div class="workbench-log-empty">No JSONL records match.</div>`;
  const evidenceOpen = query.trim() || filter !== "all" ? "open" : "";
  const evidenceMeta = parsedRecords.length
    ? `${filteredRecords.length}/${parsedRecords.length} shown`
    : "empty";

  return `
    <div class="workbench-action-detail">${escapeHtml(title)}</div>
    <div class="workbench-log-lens">
      ${briefHtml}
      ${controls}
      ${countChips ? `<div class="workbench-log-chips">${countChips}</div>` : ""}
      <details class="workbench-log-evidence" ${evidenceOpen}>
        <summary>
          <span>Event stream</span>
          <span>${escapeHtml(evidenceMeta)}</span>
        </summary>
        <div class="workbench-log-records">${recordsHtml}</div>
      </details>
    </div>
  `;
}

function renderWorkbenchLogRecord(record, query) {
  const fields = record.fields
    .filter(([, value]) => String(value || "").trim())
    .map(
      ([key, value]) => `
        <span class="workbench-log-field">
          <span class="workbench-log-field-key">${escapeHtml(key)}</span>
          <span class="workbench-log-field-value">${escapeHtml(String(value))}</span>
        </span>
      `,
    )
    .join("");
  const bodyLines = String(record.body || "")
    .split("\n")
    .slice(0, 24)
    .map((line) => `<div class="workbench-log-line">${renderHighlightedLogLine(line, query)}</div>`)
    .join("");
  return `
    <article class="workbench-log-record workbench-log-block workbench-log-block-${record.kind}" data-log-kind="${escapeHtml(record.kind)}">
      <div class="workbench-log-block-header">
        <span>${escapeHtml(record.label)} · ${escapeHtml(record.title)}</span>
        <span>${escapeHtml(record.meta)}</span>
      </div>
      ${fields ? `<div class="workbench-log-fields">${fields}</div>` : ""}
      <div class="workbench-log-block-body">${bodyLines || `<div class="workbench-log-line">${escapeHtml(record.title)}</div>`}</div>
      ${record.raw ? `
        <details class="workbench-log-json">
          <summary>JSON</summary>
          <pre>${escapeHtml(widgetTextExcerpt(record.raw, 2200))}</pre>
        </details>
      ` : ""}
    </article>
  `;
}

function transcriptRecordsToLensText(records) {
  if (!Array.isArray(records) || !records.length) {
    return "";
  }
  return records
    .map((record) => {
      const kind = String(record?.kind || "record").replace(/_/g, " ");
      const summary = String(record?.summary || "").trim();
      if (/function call|tool call/i.test(kind) && /^exec:\s+/i.test(summary)) {
        return summary.replace(/^exec:\s+/i, "");
      }
      return `${kind}: ${summary || "(empty record)"}`;
    })
    .join("\n");
}

function transcriptRecordsToRawText(records) {
  if (!Array.isArray(records) || !records.length) {
    return "";
  }
  return records
    .map((record) => String(record?.raw || "").trim())
    .filter(Boolean)
    .join("\n");
}

function renderWorkbenchLogCountChips(counts) {
  return WORKBENCH_LOG_FILTERS.filter((kind) => kind !== "all" && counts[kind])
    .map(
      (kind) => `
        <span class="workbench-log-chip workbench-log-chip-${kind}">
          <span>${escapeHtml(WORKBENCH_LOG_KIND_LABELS[kind])}</span>
          <span class="workbench-log-chip-count">${counts[kind]}</span>
        </span>
      `,
    )
    .join("");
}

function renderWorkbenchLogControls(filter, query, mode) {
  const filterOptions = WORKBENCH_LOG_FILTERS.map(
    (kind) => `<option value="${escapeHtml(kind)}" ${filter === kind ? "selected" : ""}>${escapeHtml(WORKBENCH_LOG_KIND_LABELS[kind])}</option>`,
  ).join("");

  return `
    <div class="workbench-log-toolbar">
      <div class="workbench-log-view-toggle" role="group" aria-label="Log view">
        <button type="button" class="workbench-log-view-button" data-workbench-log-mode="lens" aria-pressed="${mode === "lens" ? "true" : "false"}">Lens</button>
        <button type="button" class="workbench-log-view-button" data-workbench-log-mode="raw" aria-pressed="${mode === "raw" ? "true" : "false"}">Raw</button>
      </div>
      <select class="workbench-log-filter" name="workbench-log-filter" aria-label="Filter log blocks" data-workbench-log-filter>
        ${filterOptions}
      </select>
      <input class="workbench-log-search" type="search" name="workbench-log-search" aria-label="Search logs" placeholder="Search logs" value="${escapeHtml(query)}" data-workbench-log-search />
    </div>
  `;
}

function renderWorkbenchLogLens(tailText, options = {}) {
  if (Array.isArray(options.records)) {
    return renderWorkbenchRecordLens(options.records, options);
  }

  const excerpt = widgetTextExcerpt(tailText);
  const rawExcerpt = widgetTextExcerpt(options.rawText ?? tailText);
  const hasText = Boolean(excerpt.trim());
  const rawHasText = Boolean(rawExcerpt.trim());
  const title = options.title || "Recent output";
  const emptyText = options.emptyText || "No recent pane output.";
  const blocks = hasText ? renderTranscriptBlocks(excerpt) : [];
  const counts = workbenchLogCounts(blocks);
  const mode = state.workbenchLogMode === "raw" ? "raw" : "lens";
  const filter = WORKBENCH_LOG_FILTERS.includes(state.workbenchLogFilter) ? state.workbenchLogFilter : "all";
  const query = String(state.workbenchLogSearch || "");
  const filteredBlocks = blocks.filter((block) => {
    const kindMatches = filter === "all" || block.kind === filter;
    return kindMatches && blockMatchesSearch(block, query);
  });
  const countChips = renderWorkbenchLogCountChips(counts);
  const controls = renderWorkbenchLogControls(filter, query, mode);

  if (mode === "raw") {
    return `
      <div class="workbench-action-detail">${escapeHtml(title)}</div>
      ${controls}
      ${rawHasText ? `<pre class="workbench-log-raw">${escapeHtml(rawExcerpt)}</pre>` : `<div>${escapeHtml(emptyText)}</div>`}
    `;
  }

  const blocksHtml = !hasText
    ? `<div class="workbench-log-empty">${escapeHtml(emptyText)}</div>`
    : filteredBlocks.length
    ? filteredBlocks
        .map((block) => {
          const lineRange = block.startLine === block.endLine ? `L${block.startLine}` : `L${block.startLine}-${block.endLine}`;
          const lines = block.lines
            .map((line) => `<div class="workbench-log-line">${renderHighlightedLogLine(line, query)}</div>`)
            .join("");
          return `
            <article class="workbench-log-block workbench-log-block-${block.kind}" data-log-kind="${escapeHtml(block.kind)}">
              <div class="workbench-log-block-header">
                <span>${escapeHtml(block.label)}</span>
                <span>${escapeHtml(lineRange)}</span>
              </div>
              <div class="workbench-log-block-body">${lines}</div>
            </article>
          `;
        })
        .join("")
    : `<div class="workbench-log-empty">No log blocks match.</div>`;

  return `
    <div class="workbench-action-detail">${escapeHtml(title)}</div>
    <div class="workbench-log-lens">
      ${controls}
      ${countChips ? `<div class="workbench-log-chips">${countChips}</div>` : ""}
      <div class="workbench-log-blocks">${blocksHtml}</div>
    </div>
  `;
}

function renderDiffHtml(diffText) {
  const text = widgetTextExcerpt(diffText, 6400);
  if (!text.trim()) {
    return "";
  }
  return text
    .split("\n")
    .map((line) => {
      let klass = "diff-line";
      if (line.startsWith("+") && !line.startsWith("+++")) {
        klass += " diff-line-add";
      } else if (line.startsWith("-") && !line.startsWith("---")) {
        klass += " diff-line-del";
      } else if (line.startsWith("@@")) {
        klass += " diff-line-hunk";
      }
      return `<span class="${klass}">${escapeHtml(line || " ")}</span>`;
    })
    .join("\n");
}

function timelineEventsByKind(timeline, kinds) {
  const wanted = new Set(Array.isArray(kinds) ? kinds : [kinds]);
  return Array.isArray(timeline?.events)
    ? timeline.events.filter((event) => wanted.has(event?.kind))
    : [];
}

function renderTimelineEvents(events, emptyText = "No timeline events.") {
  if (!events.length) {
    return `<div>${escapeHtml(emptyText)}</div>`;
  }
  return `
    <ul class="workbench-actions">
      ${events
        .slice(0, 8)
        .map(
          (event) => `
            <li class="workbench-action">
              <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(event?.title || event?.kind || "event", 44))}</span>
              <span class="workbench-action-detail">${escapeHtml(truncateWorkbenchText(event?.summary || "No summary.", 220))}</span>
            </li>
          `,
        )
        .join("")}
    </ul>
  `;
}

function renderTurnsPanel(turns, selectedTurnId) {
  if (!Array.isArray(turns) || !turns.length) {
    return `<div>No user-submitted turns found.</div>`;
  }
  return `
    <div class="workbench-turn-list" role="list">
      ${turns
        .slice(-20)
        .map((turn) => {
          const id = String(turn?.id || "");
          const selected = id && id === selectedTurnId;
          const label = `Turn ${turn?.order || "?"}`;
          const text = truncateWorkbenchText(turn?.text || "", 180);
          const meta = [turn?.source, turn?.timestamp].filter(Boolean).join(" · ");
          return `
            <button class="workbench-turn ${selected ? "is-selected" : ""}" type="button" data-workbench-turn-id="${escapeHtml(id)}" aria-pressed="${selected ? "true" : "false"}">
              <span class="workbench-turn-label">${escapeHtml(label)}</span>
              <span class="workbench-turn-text">${escapeHtml(text || "Empty turn")}</span>
              ${meta ? `<span class="workbench-turn-meta">${escapeHtml(meta)}</span>` : ""}
            </button>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderDiffFileSummaries(files) {
  if (!Array.isArray(files) || !files.length) {
    return "";
  }
  return `
    <ul class="workbench-actions workbench-diff-files">
      ${files
        .slice(0, 8)
        .map((file) => {
          const hunks = Array.isArray(file?.hunks) ? file.hunks.length : 0;
          const meta = `${file?.source || "diff"} ${file?.change || "modified"} +${file?.added_lines || 0}/-${file?.removed_lines || 0}${hunks ? `, ${hunks} hunks` : ""}`;
          return `
            <li class="workbench-action">
              <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(file?.path || "unknown file", 72))}</span>
              <span class="workbench-action-detail">${escapeHtml(meta)}</span>
            </li>
          `;
        })
        .join("")}
    </ul>
  `;
}

function renderSkillsPanel(skillsPayload) {
  if (!skillsPayload) {
    return `<div>Skillbox skills have not loaded.</div>`;
  }
  if (!skillsPayload.available) {
    return `<div>${escapeHtml(skillsPayload.message || "Skillbox skills unavailable.")}</div>`;
  }
  const skills = Array.isArray(skillsPayload.skills) ? skillsPayload.skills : [];
  const issues = Array.isArray(skillsPayload.issues) ? skillsPayload.issues : [];
  const skillsHtml = skills.length
    ? `
      <ul class="workbench-actions">
        ${skills
          .slice(0, 8)
          .map(
            (skill) => `
              <li class="workbench-action">
                <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(skill?.name || "skill", 44))}</span>
                <span class="workbench-action-detail">${escapeHtml(truncateWorkbenchText(skill?.description || skill?.source_bucket || skill?.state || "available", 180))}</span>
              </li>
            `,
          )
          .join("")}
      </ul>
    `
    : `<div>No matching skills.</div>`;
  const issueHtml = issues.length
    ? `<div class="workbench-action-detail">${escapeHtml(`${issues.length} policy issue${issues.length === 1 ? "" : "s"} reported`)}</div>`
    : `<div class="workbench-action-detail">No policy issues reported.</div>`;
  return `${skillsHtml}${issueHtml}`;
}

function writeWorkbenchWidgetsHtml(nextHtml) {
  if (!el.terminalWorkbenchWidgets) {
    return;
  }
  if (state.workbenchWidgets.lastHtml === nextHtml) {
    // No payload change since the last render — skip the DOM swap entirely so
    // the operator's scroll position, text selection, and <details> toggles
    // are not collateral damage of the polling cadence.
    return;
  }
  const scroller = el.terminalWorkbench;
  const prevScrollTop =
    scroller && typeof scroller.scrollTop === "number" ? scroller.scrollTop : 0;
  const openByTitle = new Map();
  if (typeof el.terminalWorkbenchWidgets.querySelectorAll === "function") {
    for (const node of el.terminalWorkbenchWidgets.querySelectorAll(
      "details.workbench-widget",
    )) {
      const titleEl =
        typeof node.querySelector === "function"
          ? node.querySelector(".workbench-widget-title")
          : null;
      const key = titleEl ? titleEl.textContent ?? "" : "";
      if (key) {
        openByTitle.set(key, Boolean(node.open));
      }
    }
  }
  el.terminalWorkbenchWidgets.innerHTML = nextHtml;
  state.workbenchWidgets.lastHtml = nextHtml;
  if (
    openByTitle.size &&
    typeof el.terminalWorkbenchWidgets.querySelectorAll === "function"
  ) {
    for (const node of el.terminalWorkbenchWidgets.querySelectorAll(
      "details.workbench-widget",
    )) {
      const titleEl =
        typeof node.querySelector === "function"
          ? node.querySelector(".workbench-widget-title")
          : null;
      const key = titleEl ? titleEl.textContent ?? "" : "";
      if (openByTitle.has(key)) {
        node.open = openByTitle.get(key);
      }
    }
  }
  if (scroller && typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => {
      scroller.scrollTop = prevScrollTop;
    });
  } else if (scroller) {
    scroller.scrollTop = prevScrollTop;
  }
}

function renderWorkbenchWidgets() {
  if (!el.terminalWorkbenchWidgets) {
    return;
  }

  const session = currentSession();
  const widgets = selectedWorkbenchWidgets();
  if (!session) {
    writeWorkbenchWidgetsHtml(
      `<div class="workbench-action-detail">No session selected.</div>`,
    );
    return;
  }

  const timeline = widgets.timeline;
  const timelineEvents = Array.isArray(timeline?.events) ? timeline.events : [];
  const paneEvent = timelineEventsByKind(timeline, "pane_tail")[0];
  const artifactEvent = timelineEventsByKind(timeline, "artifact")[0];
  const diffEvent = timelineEventsByKind(timeline, "diff")[0];
  const tailText = widgets.paneTail?.text || paneEvent?.detail || "";
  const contextPayload = selectedAgentContextPayload();
  const transcript = widgets.transcript;
  const transcriptRecords = Array.isArray(transcript?.records) ? transcript.records : [];
  const turns = Array.isArray(transcript?.turns) && transcript.turns.length
    ? transcript.turns
    : Array.isArray(contextPayload?.turns)
      ? contextPayload.turns
      : [];
  const selectedTurnId =
    state.workbenchSelectedTurnId ||
    transcript?.selected_turn_id ||
    turns.at(-1)?.id ||
    "";
  const transcriptAvailable = Boolean(transcript?.available);
  const transcriptText = transcriptRecordsToLensText(transcriptRecords);
  const transcriptRawText = transcriptRecordsToRawText(transcriptRecords);
  const useTranscriptLogs = transcriptAvailable && (Boolean(transcript?.selected_turn) || transcriptRecords.length > 0);
  const lines = useTranscriptLogs ? transcriptRecords.length : tailLineCount(tailText);
  const artifact = widgets.artifact;
  const gitDiff = widgets.gitDiff;
  const toolActions = [
    contextPayload?.current_tool,
    ...(Array.isArray(contextPayload?.recent_actions) ? contextPayload.recent_actions : []),
  ].filter(Boolean);
  const activityEvents = timelineEventsByKind(timeline, ["task", "tool_call", "context"]);
  const planFiles = Array.isArray(artifact?.plan_files) ? artifact.plan_files : [];
  const artifactAvailable = Boolean(artifact?.available);
  const artifactMeta = artifactAvailable
    ? `${planFiles.length} plan file${planFiles.length === 1 ? "" : "s"}`
    : "unavailable";
  const diffAvailable = Boolean(gitDiff?.available);
  const unstagedDiff = gitDiff?.unstaged_diff || "";
  const stagedDiff = gitDiff?.staged_diff || "";
  const diffText = [stagedDiff, unstagedDiff].filter((part) => String(part || "").trim()).join("\n");
  const diffMeta = diffAvailable
    ? diffText.trim()
      ? gitDiff?.truncated
        ? "truncated"
        : "dirty"
      : "clean"
    : diffEvent?.summary || "unavailable";
  const status = widgets.loading
    ? `<div class="workbench-action-detail">Loading pinned widgets...</div>`
    : widgets.error
      ? `<div class="workbench-action-detail">${escapeHtml(widgets.error)}</div>`
      : "";
  const outputBody = useTranscriptLogs
    ? renderWorkbenchLogLens(transcriptText, {
        title: "Post-turn JSONL",
        rawText: transcriptRawText,
        records: transcriptRecords,
        selectedTurn: transcript?.selected_turn,
        emptyText: "No JSONL records after this turn yet.",
      })
    : renderWorkbenchLogLens(tailText);
  const activityBody = activityEvents.length
    ? `${activityEvents.some((event) => event?.kind === "tool_call") ? `<div class="workbench-action-detail">Tool calls</div>` : ""}${renderTimelineEvents(activityEvents, "No structured activity.")}`
    : toolActions.length
      ? `
        <div class="workbench-action-detail">Tool calls</div>
        <ul class="workbench-actions">
          ${toolActions
            .slice(0, 8)
            .map(
              (action) => `
                <li class="workbench-action">
                  <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(action?.tool || "action", 44))}</span>
                  <span class="workbench-action-detail">${escapeHtml(truncateWorkbenchText(action?.detail || "No detail.", 180))}</span>
                </li>
              `,
            )
            .join("")}
        </ul>
      `
      : `<div>No structured activity or Tool calls.</div>`;
  const artifactBody = artifactAvailable
    ? `
      <div>${escapeHtml(artifact.path || "Artifact path unavailable.")}</div>
      ${planFiles.length ? `<div>${escapeHtml(planFiles.join(", "))}</div>` : `<div>No plan files advertised.</div>`}
      <button class="workbench-widget-action" type="button" data-workbench-open-mermaid="true">Open viewer</button>
    `
    : `<div>${escapeHtml(artifact?.error || artifactEvent?.summary || "No Mermaid or plan artifact found.")}</div>`;
  const diffBody = diffAvailable
    ? diffText.trim()
      ? `
        <div>${escapeHtml(gitDiff.status_short || "dirty tree")}</div>
        ${renderDiffFileSummaries(gitDiff.files)}
        <pre class="workbench-diff">${renderDiffHtml(diffText)}</pre>
      `
      : `<div>${escapeHtml(gitDiff.repo_root || gitDiff.cwd || "Repository")} is clean.</div>`
    : `<div>${escapeHtml(gitDiff?.message || diffEvent?.summary || "No git diff available.")}</div>`;
  const skills = widgets.skills;
  const skillsMeta = skills?.available
    ? `${Array.isArray(skills.skills) ? skills.skills.length : 0} skills`
    : "unavailable";

  writeWorkbenchWidgetsHtml(`
    ${status}
    <details class="workbench-widget" open>
      <summary>
        <span class="workbench-widget-title">Turns</span>
        <span class="workbench-widget-meta">${turns.length ? `${turns.length} user` : "empty"}</span>
      </summary>
      <div class="workbench-widget-body">${renderTurnsPanel(turns, selectedTurnId)}</div>
    </details>
    <details class="workbench-widget" open>
      <summary>
        <span class="workbench-widget-title">Logs</span>
        <span class="workbench-widget-meta">${useTranscriptLogs ? `${lines} records` : lines ? `${lines} lines` : "empty"}</span>
      </summary>
      <div class="workbench-widget-body">${outputBody}</div>
    </details>
    <details class="workbench-widget" ${activityEvents.length || toolActions.length ? "open" : ""}>
      <summary>
        <span class="workbench-widget-title">Activity</span>
        <span class="workbench-widget-meta">${timelineEvents.length ? `${timelineEvents.length} events` : "snapshot"}</span>
      </summary>
      <div class="workbench-widget-body">${activityBody}</div>
    </details>
    <details class="workbench-widget" ${diffAvailable && diffText.trim() ? "open" : ""}>
      <summary>
        <span class="workbench-widget-title">Diffs</span>
        <span class="workbench-widget-meta">${escapeHtml(diffMeta)}</span>
      </summary>
      <div class="workbench-widget-body">${diffBody}</div>
    </details>
    <details class="workbench-widget">
      <summary>
        <span class="workbench-widget-title">Artifacts</span>
        <span class="workbench-widget-meta">${escapeHtml(artifactMeta)}</span>
      </summary>
      <div class="workbench-widget-body">${artifactBody}</div>
    </details>
    <details class="workbench-widget">
      <summary>
        <span class="workbench-widget-title">Skills</span>
        <span class="workbench-widget-meta">${escapeHtml(skillsMeta)}</span>
      </summary>
      <div class="workbench-widget-body">${renderSkillsPanel(skills)}</div>
    </details>
  `);
}

async function refreshWorkbenchWidgetsForSelectedSession(options = {}) {
  const session = currentSession();
  if (!session || state.trogdorAtlasOpen) {
    state.workbenchWidgets.loading = false;
    renderWorkbenchWidgets();
    return;
  }

  const sessionId = session.session_id;
  const now = Date.now();
  const hasCurrentWidgets =
    state.workbenchWidgets.sessionId === sessionId &&
    (Boolean(state.workbenchWidgets.timeline) ||
      Boolean(state.workbenchWidgets.skills) ||
      Boolean(state.workbenchWidgets.paneTail) ||
      Boolean(state.workbenchWidgets.transcript) ||
      Boolean(state.workbenchWidgets.artifact));
  if (
    options.throttle &&
    hasCurrentWidgets &&
    now - state.workbenchWidgets.lastLoadedAt < AGENT_CONTEXT_REFRESH_MS
  ) {
    return;
  }
  if (state.workbenchWidgets.loading && !options.force) {
    return;
  }

  const requestSeq = state.workbenchWidgets.requestSeq + 1;
  state.workbenchWidgets.requestSeq = requestSeq;
  state.workbenchWidgets.sessionId = sessionId;
  state.workbenchWidgets.error = "";
  state.workbenchWidgets.loading = !options.silent;
  renderWorkbenchWidgets();

  const timelinePath = `/v1/sessions/${encodeURIComponent(sessionId)}/timeline`;
  const skillsPath = `/v1/sessions/${encodeURIComponent(sessionId)}/skills?source=sbp`;
  const tailPath = `/v1/sessions/${encodeURIComponent(sessionId)}/pane-tail`;
  const transcriptParams = new URLSearchParams();
  const requestedTurnId = state.workbenchSelectedTurnId || "";
  const canDeltaTranscript =
    !options.force &&
    state.workbenchWidgets.sessionId === sessionId &&
    state.workbenchWidgets.transcript &&
    state.workbenchWidgets.transcriptTurnId === requestedTurnId &&
    state.workbenchWidgets.transcriptNextCursor > 0;
  if (requestedTurnId) {
    transcriptParams.set("turn_id", requestedTurnId);
  }
  if (canDeltaTranscript) {
    transcriptParams.set("after", String(state.workbenchWidgets.transcriptNextCursor));
  }
  transcriptParams.set("limit", canDeltaTranscript ? "80" : "160");
  const transcriptPath = `/v1/sessions/${encodeURIComponent(sessionId)}/transcript?${transcriptParams.toString()}`;
  const artifactPath = `/v1/sessions/${encodeURIComponent(sessionId)}/mermaid-artifact`;
  const diffPath = `/v1/sessions/${encodeURIComponent(sessionId)}/git-diff`;
  const [timelineResult, skillsResult, tailResult, transcriptResult, artifactResult, diffResult] = await Promise.allSettled([
    apiMaybeFetch(timelinePath).then(responseJsonOrNull),
    apiMaybeFetch(skillsPath).then(responseJsonOrNull),
    apiMaybeFetch(tailPath).then(responseJsonOrNull),
    apiMaybeFetch(transcriptPath).then(responseJsonOrNull),
    apiMaybeFetch(artifactPath).then(responseJsonOrNull),
    apiMaybeFetch(diffPath).then(responseJsonOrNull),
  ]);

  if (requestSeq !== state.workbenchWidgets.requestSeq || state.selectedSessionId !== sessionId) {
    return;
  }

  const errors = [];
  if (timelineResult.status === "fulfilled") {
    state.workbenchWidgets.timeline = timelineResult.value;
  } else {
    state.workbenchWidgets.timeline = null;
    errors.push(`timeline: ${timelineResult.reason?.message || "unavailable"}`);
  }

  if (skillsResult.status === "fulfilled") {
    state.workbenchWidgets.skills = skillsResult.value;
  } else {
    state.workbenchWidgets.skills = null;
    errors.push(`skills: ${skillsResult.reason?.message || "unavailable"}`);
  }

  if (tailResult.status === "fulfilled") {
    state.workbenchWidgets.paneTail = tailResult.value;
  } else {
    state.workbenchWidgets.paneTail = null;
    errors.push(`output: ${tailResult.reason?.message || "unavailable"}`);
  }

  if (transcriptResult.status === "fulfilled") {
    const nextTranscript = transcriptResult.value;
    if (nextTranscript) {
      const previous = state.workbenchWidgets.transcript;
      const previousRecords = Array.isArray(previous?.records) ? previous.records : [];
      const nextRecords = Array.isArray(nextTranscript?.records) ? nextTranscript.records : [];
      const mergeDelta =
        canDeltaTranscript &&
        previous &&
        (nextTranscript?.selected_turn_id || "") === (previous?.selected_turn_id || "");
      if (mergeDelta) {
        const byId = new Map();
        for (const record of previousRecords.concat(nextRecords)) {
          if (record?.id) {
            byId.set(record.id, record);
          }
        }
        nextTranscript.records = Array.from(byId.values())
          .sort((left, right) => Number(left?.byte_start || 0) - Number(right?.byte_start || 0))
          .slice(-240);
      }
      state.workbenchWidgets.transcript = nextTranscript;
      state.workbenchWidgets.transcriptTurnId = requestedTurnId || nextTranscript?.selected_turn_id || "";
      state.workbenchWidgets.transcriptNextCursor = Number(nextTranscript?.next_cursor || 0);
      if (!state.workbenchSelectedTurnId && nextTranscript?.selected_turn_id) {
        state.workbenchSelectedTurnId = nextTranscript.selected_turn_id;
      }
    } else {
      state.workbenchWidgets.transcript = null;
      state.workbenchWidgets.transcriptTurnId = "";
      state.workbenchWidgets.transcriptNextCursor = 0;
    }
  } else {
    state.workbenchWidgets.transcript = null;
    state.workbenchWidgets.transcriptTurnId = "";
    state.workbenchWidgets.transcriptNextCursor = 0;
    errors.push(`transcript: ${transcriptResult.reason?.message || "unavailable"}`);
  }

  if (artifactResult.status === "fulfilled") {
    state.workbenchWidgets.artifact = artifactResult.value;
  } else {
    state.workbenchWidgets.artifact = null;
    errors.push(`artifacts: ${artifactResult.reason?.message || "unavailable"}`);
  }

  if (diffResult.status === "fulfilled") {
    state.workbenchWidgets.gitDiff = diffResult.value;
  } else {
    state.workbenchWidgets.gitDiff = null;
    errors.push(`diffs: ${diffResult.reason?.message || "unavailable"}`);
  }

  state.workbenchWidgets.error = errors.join("; ");
  state.workbenchWidgets.loading = false;
  state.workbenchWidgets.lastLoadedAt = Date.now();
  renderWorkbenchWidgets();
}

function applyZoomToSurface(surface) {
  if (surfaceSupports(surface, "setZoom")) {
    surface.setZoom(state.terminalZoom);
    return true;
  }
  return false;
}

function persistTerminalZoomToUrl() {
  const url = new URL(window.location.href);
  if (Math.abs(state.terminalZoom - 1) < 0.001) {
    url.searchParams.delete("zoom");
  } else {
    url.searchParams.set("zoom", state.terminalZoom.toFixed(2));
  }
  window.history.replaceState({}, "", url);
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
    localStorage.setItem(TERMINAL_ZOOM_STORAGE_KEY, state.terminalZoom.toFixed(2));
    persistTerminalZoomToUrl();
  }
  syncTerminalZoomControls();
  if ((changed || options.forceResize) && (applied || state.terminal || state.hud)) {
    measureAndResizeSurface(true, true);
  }
  if (options.announce) {
    setUtilityStatus(`Terminal zoom ${terminalZoomLabel()}.`, false, 1600);
  }
}

function setTerminalZoom(nextZoom, options = {}) {
  state.terminalZoom = normalizeTerminalZoom(nextZoom);
  applyTerminalZoom(options);
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
  if (el.sendMode) {
    el.sendMode.disabled = state.readOnly || state.sendTarget?.type === "group";
  }
  el.sendSubmitButton.disabled = state.readOnly || !currentSession();
  Array.from(el.createForm.elements).forEach((element) => {
    element.disabled = state.readOnly;
  });

  el.terminalStage.classList.toggle("select-mode", state.selectMode);
  el.terminalStage.classList.toggle("link-hot", Boolean(state.hoveredLinkUrl) && !state.selectMode);
  syncTerminalZoomControls();
  if ((state.readOnly || !currentSession()) && state.mobileKeyboardActive) {
    closeMobileKeyboard();
  }
  syncLinkTools();
  syncTerminalStatusStrip();

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
  const batchCount = state.dirBrowser.batchSelected instanceof Set ? state.dirBrowser.batchSelected.size : 0;
  const hasSinglePath = Boolean(el.createCwd.value.trim());
  const batchReady = batchCount > 0;
  const visibleSelectableCount = visibleSelectableDirPaths().length;
  const hasBrowserPath = Boolean((state.dirBrowser.path || el.dirsPath.value || "").trim());

  el.createButton.disabled = writeDisabled || (!batchReady && !hasSinglePath);
  if (el.createBatchSubmit) {
    el.createBatchSubmit.disabled = writeDisabled || !batchReady;
  }
  if (el.createBatchVisible) {
    el.createBatchVisible.disabled = writeDisabled || visibleSelectableCount < 1;
  }
  if (el.dirsSpawnHere) {
    el.dirsSpawnHere.disabled = writeDisabled || !hasBrowserPath;
  }
  el.thoughtConfigTestButton.disabled = writeDisabled || !state.thoughtConfig.config;
  el.thoughtConfigSaveButton.disabled = writeDisabled || !state.thoughtConfig.config;
  el.nativeSaveButton.disabled = writeDisabled || !state.nativeDesktop.status;
  el.nativeOpenButton.disabled = writeDisabled || !hasSession || !nativeSupported;
  el.nativeRefreshButton.disabled = false;
  el.mermaidOpenButton.disabled = writeDisabled || !hasSession || !mermaidPath;
  el.mermaidRefreshButton.disabled = !hasSession;
  el.dirsLoadButton.disabled = !el.dirsPath.value.trim();
  el.dirsUpButton.disabled = !parentDir(el.dirsPath.value.trim());
  if (el.sendMode) {
    el.sendMode.disabled = writeDisabled || state.sendTarget?.type === "group";
  }
  el.sendSubmitButton.disabled = writeDisabled || !sendTargetReady();
  updateSendHint();
  renderCreateBatchBar();
}

function loadInitialState() {
  const url = new URL(window.location.href);
  const queryToken = url.searchParams.get("token") ?? "";
  const storedToken = localStorage.getItem(TOKEN_STORAGE_KEY) ?? "";
  const selectedFromUrl = url.searchParams.get("session");
  const selectedFromStorage = localStorage.getItem(SESSION_STORAGE_KEY);
  const followFromUrl = url.searchParams.get("follow") === "published";
  const rawStoredDirPath = localStorage.getItem(DIR_BROWSER_PATH_KEY) ?? "";
  const storedDirPath = rawStoredDirPath.trim() === "/" ? "" : rawStoredDirPath;
  const storedManagedOnly = localStorage.getItem(DIR_BROWSER_MANAGED_ONLY_KEY) === "true";
  if (rawStoredDirPath && !storedDirPath) {
    localStorage.removeItem(DIR_BROWSER_PATH_KEY);
  }

  state.terminalZoom = loadTerminalZoom(url);
  state.terminalWorkbenchOpen = !(window.matchMedia?.("(max-width: 700px)")?.matches ?? false);
  state.trogdorReadProgress = loadTrogdorReadProgress();
  loadSendHistory();
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
  state.trogdorAtlasOpen = !(state.followPublishedSelection || state.selectedSessionId);
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

function loadTrogdorReadProgress() {
  if (typeof localStorage === "undefined") {
    return {};
  }
  try {
    const parsed = JSON.parse(localStorage.getItem(TROGDOR_READ_PROGRESS_KEY) || "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const progress = {};
    for (const [key, value] of Object.entries(parsed)) {
      const index = Number(value);
      if (key && Number.isFinite(index) && index >= 0) {
        progress[key] = Math.floor(index);
      }
    }
    return progress;
  } catch (_error) {
    return {};
  }
}

function saveTrogdorReadProgress() {
  if (typeof localStorage === "undefined") {
    return;
  }
  try {
    localStorage.setItem(TROGDOR_READ_PROGRESS_KEY, JSON.stringify(state.trogdorReadProgress || {}));
  } catch (_error) {
    // Best effort only; losing the cursor should not block the operator UI.
  }
}

function stableTextHash(text) {
  let hash = 5381;
  for (let index = 0; index < text.length; index += 1) {
    hash = ((hash << 5) + hash + text.charCodeAt(index)) >>> 0;
  }
  return hash.toString(36);
}

function rawActionCueKinds(session) {
  return (Array.isArray(session?.action_cues) ? session.action_cues : [])
    .map((cue) => String(cue?.kind || "").toLowerCase())
    .filter(Boolean);
}

function rawHasActionCue(session, kind) {
  return rawActionCueKinds(session).includes(kind);
}

function rawSessionAwaitingUser(session) {
  const pressure = operatorPressureSnapshot(session?.session_id)?.pressure || {};
  const reasonKind = String(pressure.reason_kind || "").toLowerCase();
  const stateLabel = String(session?.state || "").toLowerCase();
  return rawHasActionCue(session, "awaiting_user") || reasonKind === "awaiting_user" || stateLabel === "attention";
}

function rawSessionIsSleepingOrDeepSleep(session) {
  const rest = String(session?.rest_state || "").toLowerCase();
  return rest === "sleeping" || rest === "deep_sleep";
}

function trogdorClawgText(session) {
  return String(session?.clawgText || session?.thoughtLabel || session?.commandLabel || session?.name || "waiting");
}

function trogdorClawgWords(session) {
  return trogdorClawgText(session)
    .split(/\s+/)
    .map((word) => word.trim())
    .filter(Boolean);
}

function trogdorClawgKey(session) {
  const sessionId = String(session?.sessionId || "");
  if (!sessionId) {
    return "";
  }
  const updated = String(session?.thoughtUpdatedAt || session?.objectiveChangedAt || "");
  const text = trogdorClawgText(session);
  return `${sessionId}:${updated}:${stableTextHash(text)}`;
}

function trogdorClawgReadIndex(session) {
  const words = trogdorClawgWords(session);
  const key = trogdorClawgKey(session);
  if (!key) {
    return 0;
  }
  return clampInt(state.trogdorReadProgress?.[key], 0, 0, words.length);
}

function setTrogdorClawgReadIndex(session, index) {
  const key = trogdorClawgKey(session);
  if (!key) {
    return false;
  }
  const words = trogdorClawgWords(session);
  const nextIndex = clampInt(index, 0, 0, words.length);
  if (state.trogdorReadProgress?.[key] === nextIndex) {
    return false;
  }
  state.trogdorReadProgress = {
    ...(state.trogdorReadProgress || {}),
    [key]: nextIndex,
  };
  saveTrogdorReadProgress();
  return true;
}

function markTrogdorClawgComplete(session) {
  setTrogdorClawgReadIndex(session, trogdorClawgWords(session).length);
}

function trogdorClawgReadComplete(session) {
  const words = trogdorClawgWords(session);
  return words.length > 0 && trogdorClawgReadIndex(session) >= words.length;
}

function trogdorClawgDismissed(session) {
  const key = trogdorClawgKey(session);
  return Boolean(key && state.trogdorDismissedClawgs?.[key]);
}

function dismissTrogdorClawg(session) {
  const key = trogdorClawgKey(session);
  if (!key) {
    return false;
  }
  state.trogdorDismissedClawgs = {
    ...(state.trogdorDismissedClawgs || {}),
    [key]: true,
  };
  return true;
}

function trogdorSessionBurnt(sessionOrId) {
  const sessionId = typeof sessionOrId === "string" ? sessionOrId : sessionOrId?.sessionId;
  const until = state.trogdorBurntSessions.get(String(sessionId || ""));
  if (!until) {
    return false;
  }
  if (until <= performance.now()) {
    state.trogdorBurntSessions.delete(String(sessionId || ""));
    return false;
  }
  return true;
}

function pruneTrogdorBurntSessions() {
  const now = performance.now();
  let changed = false;
  for (const [sessionId, until] of state.trogdorBurntSessions.entries()) {
    if (until <= now) {
      state.trogdorBurntSessions.delete(sessionId);
      changed = true;
    }
  }
  return changed;
}

function markTrogdorSessionsBurnt(sessionIds, options = {}) {
  const ids = Array.isArray(sessionIds) ? sessionIds.map(normalizeSessionId).filter(Boolean) : [];
  if (!ids.length) {
    return;
  }
  const until = performance.now() + TROGDOR_BURN_MS;
  for (const sessionId of ids) {
    state.trogdorBurntSessions.set(sessionId, until);
  }
  window.setTimeout(() => {
    if (pruneTrogdorBurntSessions()) {
      state.trogdorSurfaceSignature = "";
      renderHudSurface();
    }
  }, TROGDOR_BURN_MS + 40);
  if (options.render !== false) {
    state.trogdorSurfaceSignature = "";
    renderHudSurface();
  }
}

function currentTrogdorSurfaceSession() {
  const sessionId = normalizeSessionId(state.hoveredTrogdorSessionId);
  if (!sessionId) {
    return null;
  }
  const raw = state.sessions.find((item) => item.session_id === sessionId);
  return raw ? surfaceSession(raw) : null;
}

function trogdorSessionAwaitingUser(session) {
  const reasonKind = String(session?.operatorPressure?.reason_kind || "").toLowerCase();
  const stateLabel = String(session?.state || "").toLowerCase();
  return trogdorHasActionCue(session, "awaiting_user") || reasonKind === "awaiting_user" || stateLabel === "attention";
}

function trogdorSessionHasReadyClawg(session) {
  const reasonKind = String(session?.operatorPressure?.reason_kind || "").toLowerCase();
  return (
    trogdorDomActionCueKinds(session).length > 0 ||
    ["awaiting_user", "commit_ready", "validation_missing_after_edit", "dirty_check_missing"].includes(reasonKind) ||
    String(session?.state || "").toLowerCase() === "attention"
  );
}

function trogdorSessionIsSleepingOrDeepSleep(session) {
  const rest = String(session?.restLabel || "").toLowerCase();
  return rest === "sleeping" || rest === "deep_sleep";
}

function trogdorSwordsmanVisible(session) {
  if (trogdorSessionBurnt(session)) {
    return true;
  }
  return (
    (trogdorSessionHasReadyClawg(session) && !trogdorClawgDismissed(session)) ||
    trogdorSessionIsSleepingOrDeepSleep(session)
  );
}

function trogdorSessionCanRead(session) {
  return (
    !trogdorSessionBurnt(session) &&
    (
      (trogdorSessionHasReadyClawg(session) && !trogdorClawgDismissed(session)) ||
      trogdorSessionIsSleepingOrDeepSleep(session)
    )
  );
}

function trogdorReaderBaseIndex(session) {
  const words = trogdorClawgWords(session);
  const key = trogdorClawgKey(session);
  if (key && key === state.trogdorReaderClawgKey) {
    return clampInt(state.trogdorReaderStartIndex, 0, 0, words.length);
  }
  return trogdorClawgReadIndex(session);
}

function trogdorReaderWordIndex(session, wpm) {
  const words = trogdorClawgWords(session);
  if (!words.length) {
    return -1;
  }
  const baseIndex = trogdorReaderBaseIndex(session);
  if (baseIndex >= words.length) {
    return words.length;
  }
  if (state.trogdorReading === false) {
    return baseIndex;
  }
  const elapsed = state.hoveredTrogdorSessionId
    ? Math.max(0, performance.now() - state.trogdorReaderStartedAt)
    : 0;
  const msPerWord = Math.max(60, 60000 / Math.max(1, wpm));
  return Math.min(words.length, baseIndex + Math.floor(elapsed / msPerWord));
}

function advanceTrogdorReaderProgressForCurrentHover() {
  const session = currentTrogdorSurfaceSession();
  if (!session || !trogdorSessionCanRead(session)) {
    return;
  }
  if (trogdorClawgKey(session) !== state.trogdorReaderClawgKey) {
    startTrogdorReaderForSession(session);
  }
  if (state.trogdorReading === false) {
    return;
  }
  const words = trogdorClawgWords(session);
  if (!words.length) {
    return;
  }
  const wordIndex = trogdorReaderWordIndex(session, state.trogdorWpm);
  if (wordIndex < 0) {
    return;
  }
  const nextReadIndex = Math.min(words.length, wordIndex + 1);
  setTrogdorClawgReadIndex(session, nextReadIndex);
  if (nextReadIndex >= words.length) {
    state.trogdorReading = false;
  }
}

function startTrogdorReaderForSession(session, options = {}) {
  const words = trogdorClawgWords(session);
  const key = trogdorClawgKey(session);
  if (options.readAgain && key) {
    const { [key]: _dismissed, ...remainingDismissed } = state.trogdorDismissedClawgs || {};
    state.trogdorDismissedClawgs = remainingDismissed;
    setTrogdorClawgReadIndex(session, 0);
  }
  const startIndex = options.readAgain ? 0 : trogdorClawgReadIndex(session);
  state.trogdorReaderClawgKey = key;
  state.trogdorReaderStartIndex = clampInt(startIndex, 0, 0, words.length);
  state.trogdorReaderStartedAt = performance.now();
  state.trogdorReading = state.trogdorReaderStartIndex < words.length;
}

function markTrogdorSessionsResponded(sessionIds) {
  const ids = Array.isArray(sessionIds) ? sessionIds.map(normalizeSessionId).filter(Boolean) : [];
  const burntIds = [];
  for (const sessionId of ids) {
    const raw = state.sessions.find((item) => item.session_id === sessionId);
    if (!raw) {
      continue;
    }
    const session = surfaceSession(raw);
    if (!trogdorSessionAwaitingUser(session)) {
      continue;
    }
    dismissTrogdorClawg(session);
    markTrogdorClawgComplete(session);
    burntIds.push(sessionId);
  }
  if (burntIds.length) {
    if (burntIds.includes(normalizeSessionId(state.hoveredTrogdorSessionId))) {
      state.hoveredTrogdorSessionId = null;
      state.trogdorReaderStartedAt = 0;
      state.trogdorReaderStartIndex = 0;
      state.trogdorReaderClawgKey = "";
      syncTrogdorReaderTimer();
    }
    markTrogdorSessionsBurnt(burntIds);
  }
}

function keyBeginsTrogdorResponse(event) {
  if (event.repeat || event.metaKey || event.ctrlKey || event.altKey) {
    return false;
  }
  if (typeof event.key !== "string") {
    return false;
  }
  return event.key.length === 1 || event.key === "Enter" || event.key === "Backspace";
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

function sessionStateConfidence(session) {
  return String(session?.state_evidence?.confidence || "low").toLowerCase();
}

function sessionStateObserved(session) {
  return Boolean(session?.state_evidence?.observed_at);
}

function sessionStateDisplay(session) {
  const label = String(session?.state || "unknown");
  if (sessionStateConfidence(session) !== "high" || !sessionStateObserved(session)) {
    return `${label}?`;
  }
  return label;
}

function sessionStateTrustLabel(session) {
  const evidence = session?.state_evidence || {};
  const confidence = sessionStateConfidence(session);
  const freshness = sessionStateObserved(session) ? "observed" : "unobserved";
  const cause = String(evidence.cause || "unknown");
  return `${confidence} ${freshness} ${cause}`;
}

function surfaceSession(session, options = {}) {
  const operatorPressure = operatorPressureSnapshot(session.session_id);
  const surface = {
    sessionId: session.session_id,
    name: session.tmux_name || session.session_id,
    state: String(session.state || "unknown"),
    displayState: sessionStateDisplay(session),
    stateTrustLabel: sessionStateTrustLabel(session),
    stateConfidence: sessionStateConfidence(session),
    stateObserved: sessionStateObserved(session),
    restLabel: String(session.rest_state || "unknown"),
    transportLabel: String(session.transport_health || "unknown"),
    toolLabel: session.tool || "shell",
    cwdLabel: relativeCwd(session.cwd),
    fullCwd: session.cwd || "",
    thoughtLabel: options.detail ? session.thought || "No thought snapshot yet." : summarizeThought(session),
    clawgText: session.thought || "",
    thoughtUpdatedAt: session.thought_updated_at || "",
    objectiveChangedAt: session.objective_changed_at || "",
    contextLabel: `${session.token_count ?? 0} / ${session.context_limit ?? 0}`,
    skillLabel: session.last_skill || "none",
    activityLabel: formatTime(session.last_activity_at),
    commandLabel: session.current_command || "idle",
    attachedLabel: String(session.attached_clients ?? 0),
    commitCandidate: Boolean(session.commit_candidate),
    actionCues: Array.isArray(session.action_cues) ? session.action_cues : [],
    operatorPressure: operatorPressure?.pressure || null,
    batchSendSessionIds: Array.isArray(operatorPressure?.batch_send_session_ids)
      ? operatorPressure.batch_send_session_ids
      : [],
    repoKey: operatorPressure?.repo_key || session.cwd || "",
    repoLabel: operatorPressure?.repo_label || relativeCwd(session.cwd),
    isStale: Boolean(session.is_stale),
  };
  surface.clawgReadIndex = trogdorClawgReadIndex(surface);
  surface.clawgWordCount = trogdorClawgWords(surface).length;
  surface.trogdorAwaitingUser = trogdorSessionAwaitingUser(surface);
  surface.trogdorBurnt = trogdorSessionBurnt(surface);
  surface.trogdorDismissed = trogdorClawgDismissed(surface);
  surface.trogdorSwordsmanVisible = trogdorSwordsmanVisible(surface);
  return surface;
}

function operatorPressureSnapshot(sessionId) {
  return state.operatorPressureBySession.get(String(sessionId || "")) || null;
}

function applyOperatorPressure(payload) {
  const map = new Map();
  const sessions = Array.isArray(payload?.sessions) ? payload.sessions : [];
  for (const session of sessions) {
    if (session?.session_id) {
      map.set(String(session.session_id), session);
    }
  }
  state.operatorPressureBySession = map;
}

function syncTrogdorCueTransitions() {
  const awaiting = new Set();
  for (const session of state.sessions) {
    if (rawSessionAwaitingUser(session)) {
      awaiting.add(String(session.session_id));
    }
  }

  const burnt = [];
  for (const sessionId of state.trogdorAwaitingSessionIds) {
    if (!awaiting.has(sessionId)) {
      burnt.push(sessionId);
    }
  }
  state.trogdorAwaitingSessionIds = awaiting;
  if (burnt.length) {
    markTrogdorSessionsBurnt(burnt, { render: false });
  }

  const hovered = normalizeSessionId(state.hoveredTrogdorSessionId);
  if (hovered) {
    const raw = state.sessions.find((session) => session.session_id === hovered);
    if (
      !raw ||
      (!rawSessionAwaitingUser(raw) &&
        !rawSessionIsSleepingOrDeepSleep(raw) &&
        !trogdorSessionBurnt(hovered))
    ) {
      state.hoveredTrogdorSessionId = null;
      state.trogdorReaderStartedAt = 0;
      state.trogdorReaderStartIndex = 0;
      state.trogdorReaderClawgKey = "";
      syncTrogdorReaderTimer();
    }
  }
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
  const backendMetadata = selectedThoughtBackendMetadata();
  el.thoughtConfigSummary.textContent = backendMetadata
    ? `${backendMetadata.label || backendMetadata.key || "auto"} backend selected.`
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

function ensureDirBrowserBatchSelection() {
  if (!(state.dirBrowser.batchSelected instanceof Set)) {
    state.dirBrowser.batchSelected = new Set();
  }
  return state.dirBrowser.batchSelected;
}

function dirEntryResolvedPath(basePath, entry) {
  const explicit = String(entry?.full_path || "").trim();
  if (explicit) {
    return explicit;
  }
  return joinPath(basePath, entry?.name || "");
}

function dirEntryBatchSelectable(entry, resolvedPath) {
  if (!resolvedPath) {
    return false;
  }
  if (entry?.group && !entry?.full_path) {
    return false;
  }
  return true;
}

function dirEntryGroups(entry) {
  const groups = Array.isArray(entry?.groups) ? entry.groups : [];
  const normalized = groups
    .map((group) => String(group || "").trim())
    .filter(Boolean);
  if (entry?.group && !normalized.includes(String(entry.group))) {
    normalized.push(String(entry.group));
  }
  return normalized;
}

function renderDirGroupActions(entry, entryPath, allGroups, activeGroup) {
  const availableGroups = Array.isArray(allGroups) ? allGroups.map((group) => String(group || "").trim()).filter(Boolean) : [];
  if (!entryPath || !availableGroups.length) {
    return null;
  }
  const memberships = new Set(dirEntryGroups(entry));
  const wrap = document.createElement("div");
  wrap.className = "dir-row-group-actions";
  wrap.setAttribute("aria-label", `Group actions for ${entry?.name || entryPath}`);

  for (const groupName of availableGroups) {
    const isMember = memberships.has(groupName);
    const button = document.createElement("button");
    button.type = "button";
    button.className = `ghost-button dir-entry-group-action${isMember ? " is-member" : ""}`;
    button.dataset.path = entryPath;
    button.dataset.group = groupName;
    button.dataset.action = isMember ? "remove" : activeGroup && memberships.has(activeGroup) ? "move" : "add";
    if (button.dataset.action === "move") {
      button.dataset.removeGroup = activeGroup;
    }
    button.disabled = state.readOnly;
    button.textContent = isMember ? `remove ${groupName}` : button.dataset.action === "move" ? `move to ${groupName}` : `add ${groupName}`;
    wrap.appendChild(button);
  }

  return wrap;
}

function normalizedDirSearch() {
  return String(state.dirBrowser.search || "").trim().toLowerCase();
}

function dirEntryMatchesSearch(entry, resolvedPath) {
  const search = normalizedDirSearch();
  if (!search) {
    return true;
  }
  const haystack = [
    entry?.name,
    resolvedPath,
    entry?.group,
    ...dirEntryGroups(entry),
    entry?.has_children ? "directory" : "leaf repo",
    entry?.is_running ? "running" : "",
    entry?.repo_dirty ? "dirty" : "",
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  return haystack.includes(search);
}

function visibleDirEntries(entries, basePath) {
  return entries.filter((entry) => dirEntryMatchesSearch(entry, dirEntryResolvedPath(basePath, entry)));
}

function visibleSelectableDirPaths() {
  return visibleDirEntries(state.dirBrowser.entries, state.dirBrowser.path)
    .map((entry) => [entry, dirEntryResolvedPath(state.dirBrowser.path, entry)])
    .filter(([entry, resolvedPath]) => dirEntryBatchSelectable(entry, resolvedPath))
    .map(([, resolvedPath]) => resolvedPath);
}

function selectedLaunchTarget() {
  const value = String(el.createLaunchTarget?.value || state.dirBrowser.launchTarget || "local").trim();
  return value || "local";
}

function launchTargetPayload() {
  const target = selectedLaunchTarget();
  return target && target !== "local" ? target : null;
}

function renderLaunchTargetOptions(response) {
  if (!el.createLaunchTarget) {
    return;
  }
  const targets = Array.isArray(response?.launch_targets) && response.launch_targets.length
    ? response.launch_targets
    : [{ id: "local", label: "Local machine", kind: "local" }];
  const defaultTarget = String(response?.default_launch_target || state.dirBrowser.launchTarget || "local").trim() || "local";
  const hasDefault = targets.some((target) => String(target?.id || "") === defaultTarget);
  state.dirBrowser.launchTargets = targets;
  state.dirBrowser.launchTarget = hasDefault ? defaultTarget : String(targets[0]?.id || "local");
  el.createLaunchTarget.innerHTML = "";
  for (const target of targets) {
    const option = document.createElement("option");
    option.value = String(target?.id || "local");
    option.textContent = String(target?.label || target?.id || "Local machine");
    el.createLaunchTarget.appendChild(option);
  }
  el.createLaunchTarget.value = state.dirBrowser.launchTarget;
}

function createRequestPreviewText() {
  const compact = String(el.createRequest?.value || "").replace(/\s+/g, " ").trim();
  if (!compact) {
    return "(none)";
  }
  if (compact.length > 72) {
    return `${compact.slice(0, 69)}...`;
  }
  return compact;
}

function renderCreateBatchBar() {
  const selected = ensureDirBrowserBatchSelection();
  const count = selected.size;
  if (el.createBatchBar) {
    el.createBatchBar.classList.toggle("hidden", count < 1);
  }
  if (el.createBatchCount) {
    el.createBatchCount.textContent = `${count} selected`;
  }
  if (el.createBatchTool) {
    el.createBatchTool.textContent = `tool: ${String(el.createTool?.value || "grok").toLowerCase()} -> ${selectedLaunchTarget()}`;
  }
  if (el.createBatchPreview) {
    el.createBatchPreview.textContent = `request: ${createRequestPreviewText()}`;
  }
}

function clearCreateBatchSelection() {
  const selected = ensureDirBrowserBatchSelection();
  selected.clear();
  renderCreateBatchBar();
  syncSheetActionAvailability();
}

function renderDirEntries(response) {
  const rawEntries = Array.isArray(response?.entries) ? response.entries : [];
  const groups = Array.isArray(response?.groups) ? response.groups : [];
  const activeGroup = String(state.dirBrowser.group || "").trim();
  const selected = ensureDirBrowserBatchSelection();
  const selectablePaths = new Set();

  state.dirBrowser.entries = rawEntries;
  state.dirBrowser.groups = groups;
  state.dirBrowser.overlayLabel = String(response?.overlay_label || "");
  const path = String(response?.path || el.createCwd.value || "").trim();
  state.dirBrowser.path = path;
  localStorage.setItem(DIR_BROWSER_PATH_KEY, path);
  localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(Boolean(el.dirsManagedOnly.checked)));
  el.dirsPath.value = path;
  if (!el.createCwd.value.trim() || !selected.size) {
    el.createCwd.value = path;
  }
  renderLaunchTargetOptions(response);
  el.dirsList.innerHTML = "";

  if (groups.length) {
    const groupSection = document.createElement("section");
    groupSection.className = "dir-section";
    groupSection.innerHTML = `<div class="dir-section-header"><span>Groups</span></div>`;
    const chips = document.createElement("div");
    chips.className = "dir-group-chips";
    const managed = Boolean(el.dirsManagedOnly.checked);
    const overlayLabel = String(response?.overlay_label || "managed").trim().toLowerCase();

    const managedButton = document.createElement("button");
    managedButton.type = "button";
    managedButton.className = "ghost-button dir-group-chip";
    managedButton.dataset.filter = "managed";
    managedButton.dataset.group = "";
    managedButton.textContent = overlayLabel || "managed";
    managedButton.classList.toggle("is-active", managed && !activeGroup);
    chips.appendChild(managedButton);

    const allButton = document.createElement("button");
    allButton.type = "button";
    allButton.className = "ghost-button dir-group-chip";
    allButton.dataset.filter = "all";
    allButton.dataset.group = "";
    allButton.textContent = "all folders";
    allButton.classList.toggle("is-active", !managed && !activeGroup);
    chips.appendChild(allButton);

    for (const groupName of groups) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "ghost-button dir-group-chip";
      chip.dataset.filter = "group";
      chip.dataset.group = String(groupName || "");
      chip.textContent = String(groupName || "");
      chip.classList.toggle("is-active", chip.dataset.group === activeGroup);
      chips.appendChild(chip);
    }
    groupSection.appendChild(chips);
    el.dirsList.appendChild(groupSection);
  }

  const entrySection = document.createElement("section");
  entrySection.className = "dir-section";
  const sectionLabel = activeGroup ? `Entries • ${activeGroup}` : "Entries";
  entrySection.innerHTML = `<div class="dir-section-header"><span>${escapeHtml(sectionLabel)}</span></div>`;
  const entries = visibleDirEntries(rawEntries, path);

  if (!entries.length) {
    const empty = document.createElement("div");
    empty.className = "browser-empty";
    empty.textContent = normalizedDirSearch() ? "No directory matches." : "No child directories found.";
    entrySection.appendChild(empty);
  } else {
    for (const entry of entries) {
      const entryPath = dirEntryResolvedPath(path, entry);
      const selectable = dirEntryBatchSelectable(entry, entryPath);
      const row = document.createElement("div");
      row.className = "dir-row";
      row.dataset.path = entryPath;
      row.dataset.hasChildren = String(Boolean(entry.has_children));
      row.dataset.disabled = String(!selectable);
      if (entry?.group) {
        row.dataset.group = String(entry.group);
      }
      if (selectable) {
        selectablePaths.add(entryPath);
      }

      const selectCell = document.createElement("div");
      selectCell.className = "dir-select-cell";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.className = "dir-row-check";
      checkbox.dataset.path = entryPath;
      checkbox.disabled = state.readOnly || !selectable;
      checkbox.checked = selectable && selected.has(entryPath);
      checkbox.setAttribute("aria-label", `Include ${entry.name} in batch send`);
      selectCell.appendChild(checkbox);
      row.appendChild(selectCell);

      const main = document.createElement("button");
      main.type = "button";
      main.className = "dir-row-main";
      main.dataset.path = entryPath;
      main.dataset.hasChildren = String(Boolean(entry.has_children));
      if (entry?.group) {
        main.dataset.group = String(entry.group);
      }
      main.title = entryPath;
      main.tabIndex = -1;
      if (!entryPath) {
        main.disabled = true;
      }
      const running = Boolean(entry?.is_running);
      const dirty = Boolean(entry?.repo_dirty);
      const memberships = dirEntryGroups(entry);
      const managed = memberships.length > 0 || Boolean(entry?.group);
      const managedLabel = memberships.length ? memberships.join(", ") : entry?.group ? String(entry.group) : "managed";
      main.innerHTML = `
        <span class="dir-row-eyebrow">${entry.has_children ? "directory" : "leaf repo"}</span>
        <span class="dir-row-name">${escapeHtml(entry.name || "(unnamed)")}</span>
        <span class="dir-row-path">${escapeHtml(entryPath || "(no path)")}</span>
      `;
      row.appendChild(main);

      const meta = document.createElement("div");
      meta.className = "dir-row-meta";
      const badges = document.createElement("div");
      badges.className = "dir-row-badges";
      const managedBadge = document.createElement("span");
      managedBadge.className = `dir-badge ${managed ? "is-managed" : "is-unmanaged"}`;
      managedBadge.textContent = managed ? `managed:${managedLabel}` : "unmanaged";
      badges.appendChild(managedBadge);
      if (running) {
        const runningBadge = document.createElement("span");
        runningBadge.className = "dir-badge is-running";
        runningBadge.textContent = "running";
        badges.appendChild(runningBadge);
      }
      if (dirty) {
        const dirtyBadge = document.createElement("span");
        dirtyBadge.className = "dir-badge is-dirty";
        dirtyBadge.textContent = "repo dirty";
        badges.appendChild(dirtyBadge);
      }
      meta.appendChild(badges);
      if (entry?.open_url) {
        const openLink = document.createElement("a");
        openLink.className = "dir-open-url";
        openLink.href = String(entry.open_url);
        openLink.target = "_blank";
        openLink.rel = "noopener noreferrer";
        openLink.textContent = "open url";
        meta.appendChild(openLink);
      }
      const groupActions = renderDirGroupActions(entry, entryPath, groups, activeGroup);
      if (groupActions) {
        meta.appendChild(groupActions);
      }
      row.appendChild(meta);
      entrySection.appendChild(row);
    }
  }

  for (const selectedPath of Array.from(selected)) {
    if (!selectablePaths.has(selectedPath)) {
      selected.delete(selectedPath);
    }
  }

  el.dirsList.appendChild(entrySection);

  const managed = Boolean(el.dirsManagedOnly.checked);
  const shownCount = entries.length;
  const totalCount = rawEntries.length;
  const searchSuffix = normalizedDirSearch() ? ` · ${shownCount}/${totalCount} search matches` : "";
  const targetSuffix = selectedLaunchTarget() !== "local" ? ` · target ${selectedLaunchTarget()}` : "";
  const summary = response?.path
    ? `${shownCount} entries at ${response.path}${managed ? " (managed only)" : ""}${activeGroup ? ` · group ${activeGroup}` : ""}${searchSuffix}${targetSuffix}`
    : "Select a directory to continue.";
  setDirStatus(summary);
  renderCreateBatchBar();
  syncSheetActionAvailability();
}

function renderMermaidArtifact(payload) {
  state.mermaidArtifact.artifact = payload;
  const available = Boolean(payload?.available);
  const path = payload?.path || "(unknown path)";
  const updatedAt = payload?.updated_at ? formatTime(payload.updated_at) : "unknown";
  const sourceResult = boundedArtifactText(
    payload?.source || "",
    MERMAID_SOURCE_DISPLAY_MAX_CHARS,
    `Mermaid source truncated after ${MERMAID_SOURCE_DISPLAY_MAX_CHARS / 1024} KiB for browser display.`,
  );
  const source = sourceResult.text;
  const planFileResult = sanitizeMermaidPlanFiles(payload?.plan_files);
  const planFiles = planFileResult.files;
  state.mermaidArtifact.source = source;
  state.mermaidArtifact.planFiles = planFiles;
  state.mermaidArtifact.activePlanFile = "";
  state.mermaidArtifact.planContent = "";
  el.mermaidSource.textContent = source || "Mermaid source unavailable.";
  el.mermaidPreview.innerHTML = "";
  el.mermaidPlanContent.textContent = "";
  el.mermaidPlanContent.classList.add("hidden");
  el.mermaidPlanContent.classList.remove("error");

  if (available && state.mermaidArtifact.svg) {
    el.mermaidPreview.innerHTML = state.mermaidArtifact.svg;
  }

  renderMermaidPlanTabs();
  const lines = [
    `available: ${available}`,
    `path: ${path}`,
    `updated: ${updatedAt}`,
    planFiles.length ? `plan files: ${planFiles.join(", ")}` : null,
    sourceResult.truncated ? `source: truncated to ${MERMAID_SOURCE_DISPLAY_MAX_CHARS / 1024} KiB for browser display` : null,
    planFileResult.cappedCount ? `plan files: showing first ${MERMAID_PLAN_FILES_MAX}; ${planFileResult.cappedCount} hidden` : null,
    planFileResult.hiddenCount ? `plan files: ${planFileResult.hiddenCount} unsafe name${planFileResult.hiddenCount === 1 ? "" : "s"} hidden` : null,
    payload?.error ? `error: ${payload.error}` : null,
  ].filter(Boolean);
  setMermaidStatus(lines.join("\n"));
  syncSheetActionAvailability();
}

function boundedArtifactText(value, maxChars, marker) {
  const text = String(value || "");
  if (text.length <= maxChars) {
    return { text, truncated: false };
  }
  return {
    text: `${text.slice(0, maxChars)}\n\n[${marker}]`,
    truncated: true,
  };
}

function isSafeMermaidPlanFileName(name) {
  const value = String(name || "").trim();
  return Boolean(
    value
      && value.length <= 96
      && value !== "."
      && value !== ".."
      && !value.includes("..")
      && /^[A-Za-z0-9._-]+$/.test(value),
  );
}

function sanitizeMermaidPlanFiles(value) {
  const input = Array.isArray(value) ? value : [];
  const safe = [];
  let hiddenCount = 0;
  for (const rawName of input) {
    const name = String(rawName || "").trim();
    if (!isSafeMermaidPlanFileName(name)) {
      hiddenCount += 1;
      continue;
    }
    if (!safe.includes(name)) {
      safe.push(name);
    }
  }
  const files = safe.slice(0, MERMAID_PLAN_FILES_MAX);
  return {
    files,
    hiddenCount,
    cappedCount: safe.length - files.length,
  };
}

function planFileLabel(name) {
  const stem = String(name || "").replace(/\.[^.]+$/, "");
  return stem.replace(/[-_]+/g, " ") || name;
}

function renderMermaidPlanTabs() {
  const files = state.mermaidArtifact.planFiles;
  el.mermaidPlanTabs.innerHTML = "";
  el.mermaidPlanTabs.classList.toggle("hidden", !files.length);
  if (!files.length) {
    return;
  }

  for (const name of files) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ghost-button";
    button.dataset.planFile = name;
    button.textContent = planFileLabel(name);
    button.classList.toggle("active", name === state.mermaidArtifact.activePlanFile);
    el.mermaidPlanTabs.appendChild(button);
  }
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

const TROGDOR_REPO_POSITIONS = [
  { x: 18, y: 40, size: "small", variant: "hut" },
  { x: 42, y: 32, size: "large", variant: "tower" },
  { x: 78, y: 38, size: "small", variant: "hut" },
  { x: 22, y: 78, size: "wide", variant: "burning_shack" },
  { x: 88, y: 76, size: "small", variant: "hut" },
  { x: 50, y: 84, size: "small", variant: "ruin" },
  { x: 64, y: 22, size: "small", variant: "hut" },
  { x: 12, y: 60, size: "small", variant: "hut" },
];

const TROGDOR_AGENT_OFFSETS = [
  { x: -98, y: 30 },
  { x: 96, y: 26 },
  { x: -64, y: 92 },
  { x: 64, y: 92 },
  { x: 0, y: 106 },
  { x: -110, y: 78 },
  { x: 108, y: 78 },
];

function escapeAttr(text) {
  return escapeHtml(text);
}

function renderTrogdorSurface() {
  if (!el.trogdorSurface) {
    return;
  }

  const visible = Boolean(state.trogdorAtlasOpen);
  applyTrogdorAtlasVisibility();
  if (!visible) {
    return;
  }

  const sessions = state.sessions.map((session) => surfaceSession(session));
  const groups = buildTrogdorDomGroups(sessions);
  const hoveredCandidate = sessions.find((session) => session.sessionId === state.hoveredTrogdorSessionId) || null;
  const hovered = hoveredCandidate && trogdorSessionCanRead(hoveredCandidate) ? hoveredCandidate : null;
  const summary = summarizeTrogdorDom(groups, sessions);
  const dragonPose = trogdorDragonPose(groups, summary);
  const signature = trogdorSurfaceSignature(sessions, summary);
  if (signature !== state.trogdorSurfaceSignature) {
    state.trogdorSurfaceSignature = signature;
    const wpm = clampInt(state.trogdorWpm, 200, 50, 800);
    el.trogdorSurface.innerHTML = `
      ${renderTrogdorPrintFilter()}
      <div class="trogdor-frame">
        <div class="trogdor-topbar">
          <div class="trogdor-score"><span>score:</span><strong>${summary.score}</strong></div>
          ${renderTrogdorReader(hovered)}
          <div class="trogdor-level"><span>mans: ${sessions.length}</span><span>level: ${summary.level}</span></div>
        </div>
        <div class="trogdor-world" aria-label="Repository structures and agent swordsmen">
          <div class="trogdor-sun-band" aria-hidden="true"></div>
          <div class="trogdor-mountains" aria-hidden="true"></div>
          <div class="trogdor-clouds" aria-hidden="true">
            <span class="trogdor-cloud trogdor-cloud-a"></span>
            <span class="trogdor-cloud trogdor-cloud-b"></span>
          </div>
          <div class="trogdor-props" aria-hidden="true">${renderTrogdorProps()}</div>
          ${renderTrogdorDragon(dragonPose)}
          ${groups.length
            ? groups.map((group, index) => renderTrogdorStructure(group, index, dragonPose)).join("")
            : renderTrogdorEmptyField()}
        </div>
        <div class="trogdor-bottombar">
          <div class="trogdor-wpm">
            <button type="button" data-action="trogdor_read_toggle">${trogdorReadButtonLabel(hovered)}</button>
            <button type="button" data-action="trogdor_wpm_down">-25</button>
            <span class="trogdor-wpm-value" data-trogdor-wpm-value="true">${wpm} wpm</span>
            <button type="button" data-action="trogdor_wpm_up">+25</button>
          </div>
          <div class="trogdor-actions">
            <button type="button" data-action="focus_terminal">terminal</button>
            <button type="button" data-action="open_create"${state.readOnly ? " disabled" : ""}>new agent</button>
            <button type="button" data-action="open_config">config</button>
            <button type="button" data-action="open_native">native</button>
            <button type="button" data-action="open_auth">auth</button>
            <button type="button" data-action="refresh">refresh</button>
          </div>
        </div>
      </div>
    `;
  }
  renderTrogdorReader(hovered);
}

function trogdorSurfaceSignature(sessions, summary) {
  const sessionSignature = sessions.map((session) => {
    return [
      session.sessionId,
      session.name,
      session.repoKey,
      session.repoLabel,
      session.state,
      session.restLabel,
      session.thoughtLabel,
      session.thoughtUpdatedAt,
      session.trogdorAwaitingUser ? "awaiting" : "",
      session.trogdorBurnt ? "burnt" : "",
      session.trogdorDismissed ? "dismissed" : "",
      trogdorDomPressure(session),
      trogdorDomReason(session),
      trogdorAgentGlyph(session),
      (session.batchSendSessionIds || []).join(","),
      session.commitCandidate ? "commit" : "",
    ].join(":");
  });
  return JSON.stringify({
    sessions: sessionSignature,
    readOnly: state.readOnly,
    score: summary.score,
    level: summary.level,
  });
}

function renderTrogdorReader(hoveredSession) {
  const wpm = clampInt(state.trogdorWpm, 200, 50, 800);
  const hovered = hoveredSession || null;
  const bannerText = hovered ? trogdorSpeedReadWord(hovered, wpm) : "burninate!";
  const readerMarkup = `<div class="trogdor-banner" data-trogdor-reader="true">${escapeHtml(bannerText)}</div>`;
  if (!el.trogdorSurface) {
    return readerMarkup;
  }
  const banner = el.trogdorSurface.querySelector("[data-trogdor-reader]");
  if (banner) {
    banner.textContent = bannerText;
  }
  const readToggle = el.trogdorSurface.querySelector('button[data-action="trogdor_read_toggle"]');
  if (readToggle) {
    readToggle.textContent = trogdorReadButtonLabel(hovered);
  }
  const wpmValue = el.trogdorSurface.querySelector("[data-trogdor-wpm-value]");
  if (wpmValue) {
    wpmValue.textContent = `${wpm} wpm`;
  }
  return readerMarkup;
}

function trogdorReadButtonLabel(session) {
  if (session && trogdorClawgReadComplete(session) && state.trogdorReading === false) {
    return "read again";
  }
  return state.trogdorReading === false ? "read" : "pause";
}

function buildTrogdorDomGroups(sessions) {
  const groups = new Map();
  for (const session of sessions) {
    const key = session.repoKey || session.fullCwd || session.cwdLabel || session.name;
    const existing = groups.get(key) || {
      key,
      label: session.repoLabel || relativeCwd(key),
      sessions: [],
      pressure: 0,
      reason: "quiet",
    };
    existing.sessions.push(session);
    const pressure = trogdorDomPressure(session);
    if (pressure >= existing.pressure) {
      existing.pressure = pressure;
      existing.reason = trogdorDomReason(session);
    }
    groups.set(key, existing);
  }
  return Array.from(groups.values()).sort((left, right) => {
    return right.pressure - left.pressure || left.label.localeCompare(right.label);
  });
}

function summarizeTrogdorDom(groups, sessions) {
  const maxPressure = groups.reduce((max, group) => Math.max(max, group.pressure), 0);
  const actionCues = sessions.reduce((count, session) => count + trogdorDomActionCueKinds(session).length, 0);
  return {
    score: String(maxPressure * 100 + actionCues * 37).padStart(4, "0"),
    level: maxPressure || 0,
    actionCues,
  };
}

function renderTrogdorPrintFilter() {
  // Two filters: `trogdor-print` is a light displacement that nudges house/agent
  // outlines so they read as hand-stamped instead of vector. `trogdor-stamp` adds
  // heavier warp + fine speckle dropout for the dragon, flame, and ruin walls so
  // those hero figures look like a worn woodcut block on rough paper.
  return `
    <svg class="trogdor-svg-defs" aria-hidden="true" focusable="false" width="0" height="0">
      <defs>
        <filter id="trogdor-print" x="-12%" y="-12%" width="124%" height="124%" color-interpolation-filters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="1.05" numOctaves="3" seed="7" result="warp" />
          <feDisplacementMap in="SourceGraphic" in2="warp" scale="2.6" xChannelSelector="R" yChannelSelector="G" />
        </filter>
        <filter id="trogdor-stamp" x="-18%" y="-18%" width="136%" height="136%" color-interpolation-filters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="3" seed="13" result="warp" />
          <feDisplacementMap in="SourceGraphic" in2="warp" scale="3.4" xChannelSelector="R" yChannelSelector="G" result="warped" />
          <feTurbulence type="fractalNoise" baseFrequency="2.6" numOctaves="2" seed="5" result="grain" />
          <feColorMatrix in="grain" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 7 -5.6" result="grainAlpha" />
          <feComposite in="warped" in2="grainAlpha" operator="out" />
        </filter>
      </defs>
    </svg>
  `;
}

function renderTrogdorProps() {
  // Sparse decorative props scattered on the playfield: a bone, a torch, a bottle.
  // These do not affect any session — they sell the parchment-print aesthetic.
  return `
    <svg class="trogdor-prop trogdor-prop-bone" viewBox="0 0 60 24" aria-hidden="true" filter="url(#trogdor-print)">
      <path d="M8 12 H52" />
      <circle cx="8" cy="8" r="5" />
      <circle cx="8" cy="16" r="5" />
      <circle cx="52" cy="8" r="5" />
      <circle cx="52" cy="16" r="5" />
    </svg>
    <svg class="trogdor-prop trogdor-prop-torch" viewBox="0 0 32 60" aria-hidden="true" filter="url(#trogdor-print)">
      <path class="prop-torch-stem" d="M12 24 H20 V58 H12 Z" />
      <path class="prop-torch-flame" d="M16 24 C8 14 14 12 12 2 C18 8 20 8 16 -2 C24 6 24 18 16 24 Z" />
    </svg>
    <svg class="trogdor-prop trogdor-prop-bottle" viewBox="0 0 24 40" aria-hidden="true" filter="url(#trogdor-print)">
      <path d="M9 4 H15 V12 C19 14 19 20 19 38 H5 C5 20 5 14 9 12 Z" />
      <path d="M9 22 H19" />
    </svg>
  `;
}

// 8-way body sprite from a (dx, dy) vector pointing dragon → target. Returns
// the asset filename stem ("right", "back-left", "front", ...). Mirrors the
// prototype's `dirIndexFromVec`: bin atan2 into pi/4 sectors, then map.
function trogdorDragonFrameForVector(dx, dy, fallback = "right") {
  if (!dx && !dy) return fallback;
  const sector = Math.round(Math.atan2(dy, dx) / (Math.PI / 4));
  return TROGDOR_DRAGON_FRAME_BY_SECTOR[String(sector)] ?? fallback;
}

function trogdorDragonPose(groups, summary) {
  let focusIndex = -1;
  let focusGroup = null;
  let flamingResponse = false;
  for (let index = 0; index < groups.length; index += 1) {
    if (groups[index].sessions.some((session) => session.trogdorBurnt)) {
      focusIndex = index;
      focusGroup = groups[index];
      flamingResponse = true;
      break;
    }
  }
  if (!focusGroup && groups.length) {
    focusIndex = 0;
    focusGroup = groups[0];
  }

  const target = focusGroup ? TROGDOR_REPO_POSITIONS[focusIndex % TROGDOR_REPO_POSITIONS.length] : null;
  let x = TROGDOR_DRAGON_TARGET.x;
  let y = TROGDOR_DRAGON_TARGET.y;
  let direction = "right";       // flame direction (L/R) — flame plumes stay 2-way.
  let bodyFrame = "right";       // 8-way body sprite filename (sans `.png`).
  let walkX = "3.2vw";
  let walkY = "-1.2vh";

  if (target) {
    const approachX = target.x < 50 ? 20 : -18;
    x = clampInt(target.x + approachX, TROGDOR_DRAGON_TARGET.x, 18, 82);
    y = clampInt(target.y + (target.y < 54 ? 18 : -10), TROGDOR_DRAGON_TARGET.y, 30, 80);
    direction = target.x < x ? "left" : "right";
    walkX = direction === "left" ? "-3.2vw" : "3.2vw";
    walkY = target.y < y ? "-1.2vh" : "1.2vh";
    bodyFrame = trogdorDragonFrameForVector(target.x - x, target.y - y, direction);
  }

  return {
    x,
    y,
    direction,
    bodyFrame,
    walkX,
    walkY,
    heated: clampInt(summary?.level, 0, 0, 99) >= 70,
    firing: flamingResponse,
  };
}

function trogdorDragonAsset(pose, bodyFrame) {
  const frame = TROGDOR_DRAGON_BODY_FRAMES.includes(bodyFrame) ? bodyFrame : "right";
  return `${TROGDOR_DRAGON_ASSET_BASE}/${pose}/${frame}.png`;
}

function renderTrogdorDragon(pose) {
  const direction = pose?.direction === "left" ? "left" : "right";
  const bodyFrame = TROGDOR_DRAGON_BODY_FRAMES.includes(pose?.bodyFrame)
    ? pose.bodyFrame
    : direction;
  const classes = [
    "trogdor-dragon",
    `is-${direction}`,
    `is-frame-${bodyFrame}`,
    pose?.heated ? "is-heated" : "",
    pose?.firing ? "is-firing" : "",
  ].filter(Boolean).join(" ");
  const style = [
    `--dragon-x:${clampInt(pose?.x, TROGDOR_DRAGON_TARGET.x, 12, 88)}%`,
    `--dragon-y:${clampInt(pose?.y, TROGDOR_DRAGON_TARGET.y, 26, 84)}%`,
    `--dragon-walk-x:${pose?.walkX || "3.2vw"}`,
    `--dragon-walk-y:${pose?.walkY || "-1.2vh"}`,
  ].join("; ");
  const idleSrc = escapeAttr(trogdorDragonAsset("mouth-closed", bodyFrame));
  const openSrc = escapeAttr(trogdorDragonAsset("mouth-open", bodyFrame));
  // Prototype fire sequence: mouth-open → short → mid → full, each looping in
  // the order it was drawn. Body frame stays 8-way; flame direction is L/R.
  const fireFrames = TROGDOR_DRAGON_FIRE_STAGES.map((stage) => {
    const src = escapeAttr(trogdorDragonAsset(`fire-${direction}-${stage}`, bodyFrame));
    return `
      <img
        class="trogdor-dragon-sprite trogdor-dragon-fire is-${stage}"
        src="${src}"
        alt=""
        width="155"
        height="147"
        decoding="async"
        draggable="false"
      />
    `;
  }).join("");

  return `
    <div class="${classes}" style="${style}" aria-hidden="true" data-dragon-direction="${direction}" data-dragon-frame="${bodyFrame}">
      <span class="trogdor-dragon-sprite-stack">
        <img
          class="trogdor-dragon-sprite trogdor-dragon-idle"
          src="${idleSrc}"
          alt=""
          width="155"
          height="147"
          decoding="async"
          draggable="false"
        />
        <img
          class="trogdor-dragon-sprite trogdor-dragon-open"
          src="${openSrc}"
          alt=""
          width="155"
          height="147"
          decoding="async"
          draggable="false"
        />
        ${fireFrames}
      </span>
    </div>
  `;
}

function renderTrogdorStructure(group, index, dragonPose = null) {
  const pos = TROGDOR_REPO_POSITIONS[index % TROGDOR_REPO_POSITIONS.length];
  const pressure = clampInt(group.pressure, 0, 0, 99);
  const pressureBurning = pressure >= 70;
  const baseVariant = pos.variant || "hut";
  // Slot variants `ruin` and `burning_shack` carry their own destruction story
  // even when the repo is calm — the field should always show some destroyed
  // structures. High pressure additionally collapses huts/towers into ruins.
  const variant = pressureBurning && baseVariant !== "burning_shack" ? "ruin" : baseVariant;
  // Flame overlay fires when pressure is high OR the slot is a burning shack.
  const burning = pressureBurning || baseVariant === "burning_shack";
  const warning = pressure >= 35 && !pressureBurning;
  const classes = [
    "trogdor-structure",
    `is-${pos.size}`,
    `is-variant-${variant}`,
    burning ? "is-burning" : "",
    warning ? "is-warning" : "",
  ].filter(Boolean).join(" ");
  const label = escapeHtml(group.label);
  const reason = escapeHtml(group.reason);
  const style = `--x:${pos.x}%; --y:${pos.y}%; --delay:${index * 130}ms;`;
  const swordsmen = group.sessions.filter(trogdorSwordsmanVisible);

  return `
    <article class="${classes}" style="${style}" aria-label="${escapeAttr(group.label)} repository pressure ${pressure}">
      ${renderStructureSvg(variant, burning)}
      <div class="trogdor-repo-label">
        <strong>${label}</strong>
        <span>${pressure} / ${reason}</span>
      </div>
      <div class="trogdor-agent-pack">
        ${swordsmen.map((session, agentIndex) => renderTrogdorAgent(session, agentIndex, pos, dragonPose)).join("")}
      </div>
    </article>
  `;
}

function renderStructureSvg(variant, burning) {
  // Each structure is a chunky outlined silhouette: thatched red roof + cream
  // brick body + black arch door + crooked brick mortar lines. The roof and
  // wall paths intentionally use slightly irregular control points so the
  // silhouettes don't look like stamped-out vector copies — they should read
  // as hand-cut woodblock prints with broken edges.
  const filterAttr = ' filter="url(#trogdor-stamp)"';
  let body = "";
  switch (variant) {
    case "tower":
      // Square body, peaked triangular roof — taller proportions like a watchtower.
      body = `
        <path class="structure-roof" d="M14 70 L42 38 L78 12 L116 38 L146 70 L132 78 L116 56 L80 28 L46 58 L28 78 Z" />
        <path class="structure-body" d="M26 68 L36 70 L70 68 L100 70 L132 68 L130 102 L132 130 L130 146 L94 144 L60 146 L28 144 L30 110 Z" />
        <path class="structure-arch" d="M62 146 V112 C62 96 98 96 98 112 V146 Z" />
        <g class="structure-bricks">
          <path d="M28 88 L60 90 L100 88 L132 90" />
          <path d="M28 110 L62 108 L100 110 L132 108" />
          <path d="M28 130 L66 132 L102 130 L132 132" />
          <path d="M48 70 L50 88 M80 68 L80 88 M112 70 L110 88" />
          <path d="M40 90 L42 108 M70 88 L70 110 M100 90 L100 108 M124 90 L122 110" />
          <path d="M56 110 L56 130 M104 108 L106 130" />
        </g>
        <rect class="structure-window" x="60" y="80" width="14" height="12" rx="1" />
        <rect class="structure-window" x="86" y="80" width="14" height="12" rx="1" />
      `;
      break;
    case "longhouse":
      // Wider building with shallow gabled roof — village hall vibe.
      body = `
        <path class="structure-roof" d="M8 72 L24 50 L42 30 L78 28 L122 30 L138 50 L154 72 L138 76 L120 50 L78 42 L42 50 L22 76 Z" />
        <path class="structure-body" d="M18 70 L52 68 L98 70 L142 68 L140 102 L142 130 L138 146 L100 144 L52 146 L18 144 L20 110 Z" />
        <path class="structure-arch" d="M68 146 V112 C68 96 96 96 96 112 V146 Z" />
        <g class="structure-bricks">
          <path d="M18 88 L52 90 L100 88 L142 90" />
          <path d="M18 108 L54 106 L100 108 L142 106" />
          <path d="M18 128 L60 130 L100 128 L142 130" />
          <path d="M40 70 L40 88 M70 68 L70 88 M100 70 L100 88 M126 70 L126 88" />
          <path d="M30 88 L30 108 M58 90 L58 108 M84 88 L84 108 M114 88 L114 108 M134 90 L134 108" />
          <path d="M44 108 L44 128 M118 108 L118 128" />
        </g>
        <rect class="structure-window" x="34" y="78" width="14" height="12" rx="1" />
        <rect class="structure-window" x="112" y="78" width="14" height="12" rx="1" />
      `;
      break;
    case "ruin":
      // Half-collapsed wall: roof gone, jagged broken top, scattered rubble at base.
      body = `
        <path class="structure-body" d="M16 86 L24 76 L36 80 L46 70 L62 78 L72 70 L84 80 L98 72 L112 84 L126 78 L138 86 L146 76 L146 144 L120 146 L82 144 L42 146 L16 144 Z" />
        <g class="structure-bricks">
          <path d="M16 100 L40 102 L80 100 L120 102 L146 100" />
          <path d="M16 118 L36 120 L80 118 L120 120 L146 118" />
          <path d="M16 134 L42 132 L80 134 L120 132 L146 134" />
          <path d="M30 86 L30 100 M52 78 L52 100 M78 80 L78 100 M104 84 L104 100 M128 86 L128 100" />
          <path d="M22 100 L22 118 M44 100 L44 118 M68 100 L68 118 M92 100 L92 118 M118 100 L118 118 M138 100 L138 118" />
          <path d="M36 118 L36 134 M76 118 L76 134 M114 118 L114 134" />
        </g>
        <path class="structure-debris" d="M-2 144 q5 -4 12 0 t12 0 M48 146 q4 -4 10 0 M82 144 q5 -4 12 0 M120 146 q5 -4 12 0 M150 144 q5 -4 12 0" />
        <path class="structure-debris" d="M28 150 q3 -3 7 0 M104 150 q3 -3 7 0 M70 150 q3 -3 7 0" />
      `;
      break;
    case "burning_shack":
      // Low brick shed, roofless with a flame-licked broken upper edge.
      // Always renders flames from the top regardless of pressure.
      body = `
        <path class="structure-body" d="M18 78 L26 68 L40 72 L52 64 L66 70 L82 62 L100 72 L120 66 L138 76 L146 68 L146 144 L118 146 L78 144 L40 146 L18 144 Z" />
        <g class="structure-bricks">
          <path d="M18 92 L40 94 L80 92 L118 94 L146 92" />
          <path d="M18 110 L42 108 L80 110 L118 108 L146 110" />
          <path d="M18 128 L46 130 L82 128 L120 130 L146 128" />
          <path d="M36 78 L36 92 M58 70 L58 92 M82 64 L82 92 M104 74 L104 92 M126 72 L126 92" />
          <path d="M28 92 L28 110 M50 92 L50 110 M70 92 L70 110 M94 92 L94 110 M114 92 L114 110 M134 92 L134 110" />
          <path d="M40 110 L40 128 M100 110 L100 128 M124 110 L124 128" />
        </g>
        <path class="structure-arch" d="M64 146 V120 C64 108 96 108 96 120 V146 Z" />
      `;
      break;
    case "hut":
    default:
      // Round hut with conical thatched roof, single arch door, two small windows.
      body = `
        <path class="structure-roof" d="M14 72 L34 48 L60 26 L80 14 L100 26 L126 48 L146 72 Q146 80 116 80 Q80 78 44 80 Q14 80 14 72 Z" />
        <path class="structure-roof-thatch" d="M30 60 L78 22 M50 50 L78 28 M70 42 L80 30 M104 48 L82 24 M126 60 L82 22 M40 70 L60 40 M120 70 L100 40" />
        <path class="structure-body" d="M22 78 Q22 66 50 66 Q80 64 110 66 Q138 68 138 78 L136 102 L138 130 L134 146 L98 144 L62 146 L26 144 L24 110 Z" />
        <path class="structure-arch" d="M62 146 V112 C62 94 98 94 98 112 V146 Z" />
        <g class="structure-bricks">
          <path d="M22 92 L52 94 L100 92 L138 94" />
          <path d="M22 110 L48 108 L100 110 L138 108" />
          <path d="M22 128 L56 130 L100 128 L138 130" />
          <path d="M44 76 L44 92 M76 76 L76 92 M108 76 L108 92" />
          <path d="M34 92 L34 110 M62 92 L62 110 M98 92 L98 110 M126 92 L126 110" />
          <path d="M50 110 L50 128 M110 110 L110 128" />
        </g>
        <rect class="structure-window" x="34" y="82" width="12" height="10" rx="1" />
        <rect class="structure-window" x="114" y="82" width="12" height="10" rx="1" />
      `;
      break;
  }

  // Jagged flame tongues with cream negative-space cuts. Used by both the
  // pressure-driven `is-burning` overlay and the always-burning shack variant.
  const flames = burning
    ? `
        <path class="structure-flame" d="M22 64
                                          L34 32 L40 56 L52 18 L58 56
                                          L72 8 L76 54 L92 22 L96 56
                                          L110 16 L116 56 L128 30 L132 60
                                          L140 38 L142 62 Z" />
        <path class="structure-flame hot" d="M48 60
                                              L56 32 L62 56 L70 22 L76 56
                                              L88 28 L92 56 L104 34 L110 58
                                              L120 40 L124 60 Z" />
        <path class="structure-flame-cut" d="M40 50 L48 32 M70 48 L80 28 M100 48 L110 28 M122 50 L132 36" />
        <path class="structure-smoke" d="M38 -8 q10 -8 6 -18 M118 -12 q10 -8 6 -20 M78 -10 q8 -10 4 -20" />
      `
    : "";

  return `
    <svg class="structure-svg" viewBox="0 0 160 150" aria-hidden="true"${filterAttr}>
      ${body}
      ${flames}
    </svg>
  `;
}

function renderTrogdorAgent(session, index, structurePos = null, dragonPose = null) {
  const offset = TROGDOR_AGENT_OFFSETS[index % TROGDOR_AGENT_OFFSETS.length];
  const hovered = session.sessionId === state.hoveredTrogdorSessionId;
  const glyph = escapeHtml(trogdorAgentGlyph(session));
  const label = escapeAttr(`${session.name} ${trogdorDomReason(session)}`);
  const tone = trogdorAgentTone(session);
  const attacking = trogdorSessionAwaitingUser(session) && !trogdorClawgDismissed(session) && !session.trogdorBurnt;
  const burnt = Boolean(session.trogdorBurnt);
  const dragonTarget = dragonPose || TROGDOR_DRAGON_TARGET;
  const chargeX = structurePos ? (dragonTarget.x - structurePos.x) * 0.82 : 0;
  const chargeY = structurePos ? (dragonTarget.y - structurePos.y) * 0.62 : 0;
  const style = [
    `--ax:${offset.x}px`,
    `--ay:${offset.y}px`,
    `--walk:${900 + index * 110}ms`,
    `--charge:${22000 + index * 1200}ms`,
    `--charge-x:${chargeX.toFixed(2)}vw`,
    `--charge-y:${chargeY.toFixed(2)}vh`,
  ].join("; ");
  const classes = [
    "trogdor-agent",
    `is-${tone}`,
    hovered ? "is-hovered" : "",
    attacking ? "is-attacking" : "",
    burnt ? "is-burnt" : "",
  ].filter(Boolean).join(" ");
  // Swordsman: chunky blue body with cream outline. Helmet has a tall curved plume,
  // a thin visor slit, and the figure carries a tall sword on the right and a kite
  // shield on the left. The action-cue glyph is overlaid on the shield.
  return `
    <button
      type="button"
      class="${classes}"
      style="${style}"
      data-trogdor-agent="true"
      data-session-id="${escapeAttr(session.sessionId)}"
      aria-label="${label}"
    >
      <svg viewBox="0 0 90 130" aria-hidden="true" filter="url(#trogdor-print)">
        <!-- Plume on top of helmet, curved like a feather -->
        <path class="agent-plume" d="M52 18
                                      C56 4 70 0 78 8
                                      C72 10 70 16 70 22
                                      C66 18 60 18 56 22 Z" />
        <!-- Sword: long blade pointing up with a small cross-guard and round pommel -->
        <path class="agent-sword-blade" d="M76 30 L82 30 L80 84 L78 84 Z" />
        <path class="agent-sword-guard" d="M70 84 H88" />
        <circle class="agent-sword-pommel" cx="79" cy="92" r="3.6" />
        <!-- Helmet: rounded top with a forward visor slit -->
        <path class="agent-helm" d="M28 36
                                     C28 18 64 18 64 36
                                     L64 50
                                     L28 50 Z" />
        <path class="agent-helm-slit" d="M34 42 H58" />
        <!-- Body: blocky tunic that flares slightly at the bottom -->
        <path class="agent-body" d="M26 50
                                     L66 50
                                     L70 96
                                     L62 96
                                     L60 108
                                     L32 108
                                     L30 96
                                     L22 96 Z" />
        <!-- Belt -->
        <path class="agent-belt" d="M26 88 H66" />
        <!-- Shield: kite/hex shape held on the left -->
        <path class="agent-shield" d="M2 56
                                       L24 50
                                       L26 78
                                       L18 96
                                       L10 96
                                       L4 80 Z" />
        <path class="agent-shield-rivet" d="M14 60 v22 M8 70 H22" />
        <!-- Two stubby legs -->
        <path class="agent-leg" d="M34 108 V124 H44 V112" />
        <path class="agent-leg" d="M50 108 V124 H60 V112" />
        <!-- Action-cue glyph centered on the shield -->
        <text class="agent-glyph" x="14" y="76" text-anchor="middle">${glyph}</text>
      </svg>
      ${burnt
        ? `
          <span class="agent-burn-flame" aria-hidden="true"></span>
          <span class="agent-burn-smoke" aria-hidden="true"></span>
        `
        : ""}
    </button>
  `;
}

function renderTrogdorEmptyField() {
  return `
    <div class="trogdor-empty-field">
      <svg viewBox="0 0 240 180" aria-hidden="true">
        <path class="empty-bone" d="M30 126h78M46 113c-13-13-29 6-15 16-15 11 7 31 18 15M90 113c13-13 29 6 15 16 15 11-7 31-18 15" />
        <path class="empty-house" d="M142 84l40-30 42 30M152 84h62v52h-62zM174 136v-24c0-15 20-15 20 0v24" />
      </svg>
      <p>no repos</p>
      <button type="button" data-action="open_create"${state.readOnly ? " disabled" : ""}>launch agent</button>
    </div>
  `;
}

function trogdorDomPressure(session) {
  const pressure = session?.operatorPressure || {};
  if (Number.isFinite(pressure.score)) {
    return clampInt(pressure.score, 1, 0, 99);
  }
  let score = 0;
  const stateLabel = String(session?.state || "").toLowerCase();
  const rest = String(session?.restLabel || "").toLowerCase();
  if (trogdorHasActionCue(session, "awaiting_user")) score += 55;
  if (trogdorHasActionCue(session, "commit_ready")) score += 45;
  if (trogdorHasActionCue(session, "validation_missing_after_edit")) score += 40;
  if (stateLabel === "attention") score += 45;
  if (stateLabel === "busy") score += 12;
  if (stateLabel === "error") score += 55;
  if (rest === "sleeping") score += 35;
  if (rest === "deep_sleep") score += 20;
  if (session?.commitCandidate) score += 25;
  return clampInt(score, 0, 0, 99);
}

function trogdorDomReason(session) {
  const pressure = session?.operatorPressure || {};
  if (pressure.reason) return String(pressure.reason);
  const cue = trogdorPrimaryActionCue(session);
  if (cue) return cue.replaceAll("_", " ");
  if (session?.commitCandidate) return "commit ready";
  const rest = String(session?.restLabel || "").toLowerCase();
  if (rest === "deep_sleep") return "deep sleep";
  if (rest === "sleeping") return "sleeping";
  return String(session?.state || "idle");
}

function trogdorAgentGlyph(session) {
  const pressure = session?.operatorPressure || {};
  if (pressure.glyph) return String(pressure.glyph).slice(0, 1);
  if (trogdorHasActionCue(session, "awaiting_user")) return "!";
  if (trogdorHasActionCue(session, "commit_ready") || session?.commitCandidate) return "$";
  if (trogdorHasActionCue(session, "validation_missing_after_edit")) return "v";
  if (String(session?.state || "").toLowerCase() === "error") return "x";
  if (trogdorSessionIsSleepingOrDeepSleep(session)) return "z";
  return "a";
}

function trogdorAgentTone(session) {
  const tone = String(session?.operatorPressure?.tone || "").toLowerCase();
  if (tone === "danger" || tone === "warning" || tone === "working" || tone === "quiet") {
    return tone;
  }
  const pressure = trogdorDomPressure(session);
  if (pressure >= 70) return "danger";
  if (pressure >= 35) return "warning";
  if (String(session?.state || "").toLowerCase() === "busy") return "working";
  return "quiet";
}

function trogdorSpeedReadWord(session, wpm) {
  const words = trogdorClawgWords(session);
  if (!words.length) return "waiting";
  const index = trogdorReaderWordIndex(session, wpm);
  if (index >= words.length) return "caught up";
  return words[Math.max(0, index)].slice(0, 22);
}

function trogdorDomActionCueKinds(session) {
  return (Array.isArray(session?.actionCues) ? session.actionCues : [])
    .map((cue) => String(cue?.kind || "").toLowerCase())
    .filter(Boolean);
}

function trogdorHasActionCue(session, kind) {
  return trogdorDomActionCueKinds(session).includes(kind);
}

function trogdorPrimaryActionCue(session) {
  const kinds = trogdorDomActionCueKinds(session);
  for (const kind of ["awaiting_user", "commit_ready", "validation_missing_after_edit", "dirty_check_missing"]) {
    if (kinds.includes(kind)) return kind;
  }
  return "";
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

async function loadDirListing(
  path = el.dirsPath.value,
  managedOnly = el.dirsManagedOnly.checked,
  group = state.dirBrowser.group,
  options = {},
) {
  const targetPath = String(path || "").trim();
  const managed = Boolean(managedOnly);
  const groupName = String(group || "").trim();

  state.dirBrowser.loading = true;
  state.dirBrowser.managedOnly = managed;
  state.dirBrowser.group = groupName;
  el.dirsManagedOnly.checked = managed;
  localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(managed));
  setDirStatus("Loading directories...");
  try {
    const url = new URL("/v1/dirs", window.location.origin);
    if (targetPath) {
      url.searchParams.set("path", targetPath);
    }
    url.searchParams.set("managed_only", String(managed));
    if (groupName) {
      url.searchParams.set("group", groupName);
    }
    const response = await apiFetch(url.pathname + url.search);
    const payload = await response.json();
    renderDirEntries(payload);
  } catch (error) {
    if (shouldRetryDirListingFromBase(error, targetPath, groupName, options)) {
      localStorage.removeItem(DIR_BROWSER_PATH_KEY);
      state.dirBrowser.path = "";
      state.dirBrowser.group = "";
      el.dirsPath.value = "";
      el.createCwd.value = "";
      setDirStatus("Saved directory was outside the repository root. Loading the default directory...");
      return loadDirListing("", managed, "", { retriedFromBase: true });
    }
    setDirStatus(`Failed to load directories: ${error.message}`, true);
  } finally {
    state.dirBrowser.loading = false;
    syncSheetActionAvailability();
  }
}

async function updateDirEntryGroupMembership(path, action, groupName, removeGroup = "") {
  const targetPath = String(path || "").trim();
  const targetGroup = String(groupName || "").trim();
  const sourceGroup = String(removeGroup || "").trim();
  if (!targetPath || !targetGroup || state.readOnly) {
    return;
  }

  const add = [];
  const remove = [];
  if (action === "remove") {
    remove.push(targetGroup);
  } else {
    add.push(targetGroup);
    if (action === "move" && sourceGroup && sourceGroup !== targetGroup) {
      remove.push(sourceGroup);
    }
  }

  setDirStatus("Updating directory group...");
  try {
    await apiFetch("/v1/dirs/group-memberships", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: targetPath, add, remove }),
    });
    await loadDirListing(
      state.dirBrowser.path || el.dirsPath.value,
      state.dirBrowser.managedOnly,
      state.dirBrowser.group,
    );
  } catch (error) {
    setDirStatus(`Failed to update group: ${error.message}`, true);
  }
}

function shouldRetryDirListingFromBase(error, targetPath, groupName, options = {}) {
  if (options.retriedFromBase || !targetPath || groupName) {
    return false;
  }
  if (error?.status !== 403) {
    return false;
  }
  return String(error?.message || "").toLowerCase().includes("outside the allowed base directory");
}

async function warmDirBrowserOnStartup() {
  if (state.dirBrowser.loading || state.dirBrowser.entries.length > 0) {
    return;
  }
  await loadDirListing(state.dirBrowser.path || "", state.dirBrowser.managedOnly, state.dirBrowser.group);
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

async function loadMermaidPlanFile(name) {
  const session = currentSession();
  const fileName = String(name || "").trim();
  if (!session || !fileName) {
    return;
  }
  if (!isSafeMermaidPlanFileName(fileName) || !state.mermaidArtifact.planFiles.includes(fileName)) {
    const message = `Plan file name not allowed: ${fileName}`;
    state.mermaidArtifact.planContent = "";
    el.mermaidPlanContent.classList.remove("hidden");
    el.mermaidPlanContent.textContent = message;
    el.mermaidPlanContent.classList.add("error");
    setMermaidStatus(message, true);
    syncSheetActionAvailability();
    return;
  }

  state.mermaidArtifact.activePlanFile = fileName;
  state.mermaidArtifact.planContent = "";
  renderMermaidPlanTabs();
  el.mermaidPlanContent.classList.remove("hidden");
  el.mermaidPlanContent.textContent = "Loading plan file...";
  try {
    const url = new URL(`/v1/sessions/${encodeURIComponent(session.session_id)}/plan-file`, window.location.origin);
    url.searchParams.set("name", fileName);
    const response = await apiMaybeFetch(url.pathname + url.search);
    const payload = await responseJsonOrNull(response);
    const contentResult = boundedArtifactText(
      payload?.content || "",
      MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS,
      `Plan file truncated after ${MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS / 1024} KiB for browser display.`,
    );
    state.mermaidArtifact.planContent = contentResult.text;
    el.mermaidPlanContent.textContent =
      contentResult.text || payload?.error || `${fileName} is unavailable.`;
    el.mermaidPlanContent.classList.toggle("error", Boolean(payload?.error));
    setMermaidStatus(
      payload?.error
        ? `Plan file ${fileName}: ${payload.error}`
        : contentResult.truncated
          ? `Plan file loaded: ${fileName} (truncated to ${MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS / 1024} KiB for browser display)`
          : `Plan file loaded: ${fileName}`,
    );
  } catch (error) {
    el.mermaidPlanContent.textContent = `Failed to load ${fileName}: ${error.message}`;
    el.mermaidPlanContent.classList.add("error");
    setMermaidStatus(`Failed to load plan file: ${error.message}`, true);
  } finally {
    syncSheetActionAvailability();
  }
}

async function launchCommitGrok() {
  const session = currentSession();
  if (!session) {
    return;
  }

  setUtilityStatus(`Launching commit Grok for ${session.session_id}...`, false, 1800);
  try {
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/commit-grok`, {
      method: "POST",
    });
    const payload = await response.json();
    setUtilityStatus(
      `Commit Grok launched: ${payload.session_name} / ${payload.watch_command}`,
      false,
      3800,
    );
  } catch (error) {
    setUtilityStatus(`Failed to launch commit Grok: ${error.message}`, true, 3800);
  }
}

async function refreshSessions() {
  try {
    const publishedRequest = state.followPublishedSelection ? apiFetch("/v1/selection") : Promise.resolve(null);
    const [response, pressureResponse, healthResponse, publishedResponse] = await Promise.all([
      apiFetch("/v1/sessions"),
      apiMaybeFetch("/v1/operator-pressure"),
      apiMaybeFetch("/health"),
      publishedRequest,
    ]);
    const payload = await response.json();
    const pressurePayload = await responseJsonOrNull(pressureResponse);
    const healthPayload = await responseJsonOrNull(healthResponse);
    state.sessions = Array.isArray(payload.sessions) ? payload.sessions : [];
    applyOperatorPressure(pressurePayload);
    applyBackendHealth(healthPayload);
    syncTrogdorCueTransitions();

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
        const fallbackSessionId = state.trogdorAtlasOpen ? null : state.sessions[0]?.session_id ?? null;
        persistSelectedSession(fallbackSessionId);
      }
    }

    await setupHudSurface();
    renderHudSurface();
    syncTerminalTools();
    await connectSelectedSession();
    void refreshAgentContextForSelectedSession({ throttle: true, silent: true });
    void refreshWorkbenchWidgetsForSelectedSession({ throttle: true, silent: true });
    if (state.followPublishedSelection && !state.selectedSessionId) {
      setConnectionStatus("waiting", true);
    } else {
      setConnectionStatus(state.selectedSessionId ? "live" : "idle");
    }
    setModeStatus(state.readOnly ? "observer" : "operator", !state.token);
  } catch (error) {
    state.sessions = [];
    state.operatorPressureBySession = new Map();
    state.backendHealth = null;
    state.publishedSelection = null;
    persistSelectedSession(null);
    resetAgentContextForSession(null);
    resetWorkbenchWidgetsForSession(null);
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
    clearTimeout(state.refreshTimer);
  }
  state.refreshTimer = window.setTimeout(async () => {
    state.refreshTimer = null;
    await refreshSessions();
    scheduleSessionRefresh();
  }, sessionRefreshDelayMs());
}

function sessionRefreshDelayMs() {
  if (state.followPublishedSelection || !sessionEventStreamOpen()) {
    return SESSION_REFRESH_MS;
  }
  return SESSION_REFRESH_STREAMING_MS;
}

function sessionEventStreamOpen() {
  const session = currentSession();
  return Boolean(
    session &&
      state.ws &&
      state.ws.readyState === WebSocket.OPEN &&
      state.ws.sessionId === session.session_id,
  );
}

async function loadFrankenTermFont() {
  if (!boot.franken_term_font_url || !document.fonts?.load) {
    return null;
  }
  if (!state.frankenFontInit) {
    state.frankenFontInit = document.fonts
      .load('12px "Pragmasevka NF"')
      .catch((error) => {
        state.frankenLoadError = `font load failed: ${error?.message || String(error)}`;
        state.frankenFontInit = null;
        return null;
      });
  }
  return state.frankenFontInit;
}

async function ensureFrankenTerm() {
  if (!boot.franken_term_available) {
    return null;
  }

  if (!state.frankenInit) {
    state.frankenInit = (async () => {
      await loadFrankenTermFont();
      const mod = assertFrankenTermModule(await import(boot.franken_term_js_url));
      const wasmUrl = boot.franken_term_wasm_url
        ? new URL(boot.franken_term_wasm_url, window.location.href)
        : undefined;
      if (wasmUrl) {
        await mod.default(wasmUrl);
      } else {
        await mod.default();
      }
      state.frankenModule = mod;
      state.frankenLoadError = "";
      state.frankenAssetSummary = frankenTermAssetSummary();
      return mod;
    })().catch((error) => {
      state.frankenInit = null;
      state.frankenModule = null;
      state.frankenLoadError = error?.message || String(error || "FrankenTerm load failed");
      throw error;
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
  state.hud = validateFrankenTermSurface(
    new mod.FrankenTermWeb(),
    FRANKENTERM_HUD_METHODS,
    "HUD renderer",
  );
  await state.hud.init(el.hudCanvas, undefined);
  if (surfaceSupports(state.hud, "setAccessibility")) {
    state.hud.setAccessibility({
      reducedMotion: window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false,
    });
  }
  applyZoomToSurface(state.hud);
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
  clearTerminalPaintProbe();
  clearPendingTerminalBytes();
  if (state.terminal) {
    state.terminal.destroy();
    state.terminal = null;
  }
  state.terminalAcceptsBytes = false;
  state.terminalSessionId = null;
  state.terminalFallbackAutoFollow = true;
  state.terminalMirrorText = "";
  state.terminalPaintVerified = false;
  state.terminalFrameBytesSeen = 0;
  if (el.terminalA11yMirror) {
    el.terminalA11yMirror.value = "";
  }
  el.terminalCanvas.classList.add("hidden");
}

function clearPendingTerminalBytes() {
  state.pendingTerminalByteChunks = [];
  state.pendingTerminalByteLength = 0;
}

function bufferTerminalBytes(bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength === 0) {
    return false;
  }
  const copy = new Uint8Array(bytes);
  state.pendingTerminalByteChunks.push(copy);
  state.pendingTerminalByteLength += copy.byteLength;
  while (
    state.pendingTerminalByteLength > MAX_PENDING_TERMINAL_BYTES &&
    state.pendingTerminalByteChunks.length > 1
  ) {
    const dropped = state.pendingTerminalByteChunks.shift();
    state.pendingTerminalByteLength -= dropped?.byteLength || 0;
  }
  setConnectionStatus("buffering terminal; renderer attaching");
  return true;
}

function flushPendingTerminalBytes() {
  if (!state.terminal || !state.pendingTerminalByteChunks.length) {
    return false;
  }
  const chunks = state.pendingTerminalByteChunks;
  clearPendingTerminalBytes();
  for (const chunk of chunks) {
    feedTerminalBytes(chunk);
  }
  return true;
}

function clearTerminalPaintProbe() {
  if (state.terminalPaintProbeTimer) {
    window.clearTimeout(state.terminalPaintProbeTimer);
    state.terminalPaintProbeTimer = null;
  }
}

function setTerminalTextFallbackActive(active, options = {}) {
  const wasActive = state.terminalFallbackActive;
  state.terminalFallbackActive = Boolean(active && currentSession());
  el.terminalFallback.classList.toggle("hidden", !state.terminalFallbackActive);
  el.terminalFallback.setAttribute("aria-hidden", state.terminalFallbackActive ? "false" : "true");
  if (state.terminalFallbackActive) {
    state.terminalFallbackAutoFollow = wasActive ? terminalFallbackIsNearBottom() : true;
    startSnapshotPolling();
    if (!wasActive) {
      focusTerminalInputSurface({ onlyIfSurfaceFocused: true, preventScroll: true });
    }
    syncTerminalStatusStrip();
    return;
  }

  if (options.clearText !== false) {
    el.terminalFallback.textContent = "";
  }
  if (state.terminal || !currentSession()) {
    stopSnapshotPolling();
  }
  syncTerminalStatusStrip();
}

function terminalFallbackIsNearBottom() {
  const maxScrollTop = Math.max(0, el.terminalFallback.scrollHeight - el.terminalFallback.clientHeight);
  return maxScrollTop - el.terminalFallback.scrollTop < 48;
}

function updateTerminalFallbackText(text) {
  const previousScrollTop = el.terminalFallback.scrollTop;
  const shouldFollow = state.terminalFallbackAutoFollow || terminalFallbackIsNearBottom();
  el.terminalFallback.textContent = text || "";
  if (shouldFollow) {
    el.terminalFallback.scrollTop = el.terminalFallback.scrollHeight;
  } else {
    el.terminalFallback.scrollTop = Math.min(previousScrollTop, Math.max(0, el.terminalFallback.scrollHeight - el.terminalFallback.clientHeight));
  }
  syncTerminalAccessibilityMirror(text || "");
}

function syncTerminalAccessibilityMirror(fallbackText = null) {
  let mirrorText = "";
  if (typeof fallbackText === "string") {
    mirrorText = fallbackText;
  } else if (terminalSupports("screenReaderMirrorText")) {
    mirrorText = state.terminal.screenReaderMirrorText() || "";
  } else if (terminalSupports("accessibilityDomSnapshot")) {
    mirrorText = state.terminal.accessibilityDomSnapshot()?.value || "";
  }
  state.terminalMirrorText = mirrorText;
  if (el.terminalA11yMirror) {
    el.terminalA11yMirror.value = mirrorText;
  }
  if (terminalSupports("drainAccessibilityAnnouncements") && el.terminalAnnouncer) {
    const announcements = state.terminal.drainAccessibilityAnnouncements();
    if (Array.isArray(announcements) && announcements.length) {
      el.terminalAnnouncer.textContent = announcements.join("\n");
    }
  }
}

function terminalMirrorTextFromRenderer() {
  if (terminalSupports("screenReaderMirrorText")) {
    return state.terminal.screenReaderMirrorText() || "";
  }
  if (terminalSupports("accessibilityDomSnapshot")) {
    return state.terminal.accessibilityDomSnapshot()?.value || "";
  }
  return "";
}

function terminalTextHasContent(text) {
  return /\S/.test(String(text || ""));
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
    setTerminalTextFallbackActive(true, { clearText: false });
    await refreshSnapshotFallback();
    return;
  }

  if (state.terminal && state.terminalSessionId === session.session_id) {
    el.terminalCanvas.classList.remove("hidden");
    el.terminalFallback.classList.toggle("hidden", !state.terminalFallbackActive);
    refreshTerminalSearch();
    syncTerminalAccessibilityMirror();
    syncTerminalTools();
    setLoadingState(false);
    return;
  }

  destroyTerminalInstance();
  setLoadingState(true, "Initializing terminal...");
  try {
    state.terminal = validateFrankenTermSurface(
      new mod.FrankenTermWeb(),
      FRANKENTERM_TERMINAL_METHODS,
      "terminal renderer",
    );
    state.terminalAcceptsBytes = false;
    await state.terminal.init(el.terminalCanvas, undefined);
    state.terminalAcceptsBytes = true;
  } catch (error) {
    destroyTerminalInstance();
    setTerminalTextFallbackActive(true, { clearText: false });
    await refreshSnapshotFallback();
    setLoadingState(false);
    setUtilityStatus(`Live terminal renderer unavailable: ${error.message}`, true, 3600);
    return;
  }
  state.terminalSessionId = session.session_id;
  state.terminalPaintVerified = false;
  state.terminalFrameBytesSeen = 0;
  setTerminalTextFallbackActive(false);
  if (terminalSupports("setLinkOpenPolicy")) {
    state.terminal.setLinkOpenPolicy(frankenTermLinkPolicy());
  }
  if (terminalSupports("setAccessibility")) {
    state.terminal.setAccessibility({
      reducedMotion: window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false,
      screenReader: true,
    });
  }
  applyZoomToSurface(state.terminal);
  el.terminalCanvas.classList.remove("hidden");
  clearTerminalSelection();
  refreshTerminalSearch();
  syncTerminalAccessibilityMirror();
  syncTerminalTools();
  measureAndResizeSurface(true, true);
  flushPendingTerminalBytes();
  setLoadingState(false);
}

function teardownTerminal() {
  disconnectSocket();
  stopSnapshotPolling();
  destroyTerminalInstance();
  setTerminalTextFallbackActive(false);
  syncTerminalTools();
  renderHudSurface();
}

function disconnectSocket() {
  state.connectionGeneration += 1;
  clearReconnectTimer();
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
  const dimensionsChanged = cols !== state.currentCols || rows !== state.currentRows;

  if (!force && !dimensionsChanged) {
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

  if (pushResize && (force || dimensionsChanged)) {
    sendResize();
  }
  if (state.terminal && (force || dimensionsChanged)) {
    captureTerminalRendererDiagnostic("resize");
  }
}

function captureTerminalRendererDiagnostic(reason = "frame") {
  if (!terminalSupports("snapshotResizeStormFrameJsonl")) {
    return null;
  }
  const frameIndex = state.rendererDiagnosticSequence;
  state.rendererDiagnosticSequence += 1;
  const timestamp = new Date().toISOString();
  try {
    const line = state.terminal.snapshotResizeStormFrameJsonl("swimmers-web", 0, timestamp, frameIndex);
    const parsed = JSON.parse(String(line || "{}"));
    state.lastRendererDiagnostic = { reason, line, parsed };
    state.lastRendererDiagnosticError = "";
    return line;
  } catch (error) {
    state.lastRendererDiagnosticError = error?.message || String(error);
    return null;
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
    hoveredTrogdorSessionId: state.hoveredTrogdorSessionId,
    trogdorAtlasOpen: state.trogdorAtlasOpen,
    trogdorWpm: state.trogdorWpm,
    trogdorReading: state.trogdorReading,
    trogdorReaderStartIndex: state.trogdorReaderStartIndex,
    trogdorReaderElapsedMs: state.hoveredTrogdorSessionId
      ? Math.max(0, performance.now() - state.trogdorReaderStartedAt)
      : 0,
    sessions: surfaceSessions,
    selectedSessionId: state.selectedSessionId,
    publishedSessionId: normalizeSessionId(state.publishedSelection?.session_id),
    publishedAtLabel: formatTime(state.publishedSelection?.published_at),
    currentSession: selectedSession ? surfaceSession(selectedSession, { detail: true }) : null,
  };
}

function renderHudSurface() {
  advanceTrogdorReaderProgressForCurrentHover();
  renderTrogdorSurface();
  syncTerminalPresentation();
  if (!state.hud) {
    return;
  }
  const frame = buildSurfaceFrame(buildSurfaceModel());
  state.surfaceZones = frame.zones ?? [];
  state.surfaceMasks = frame.masks ?? [];
  state.hud.applyPatchBatchFlat(frame.spans, frame.cells);
  scheduleRender();
}

function syncTerminalPresentation() {
  const terminalFocusMode = Boolean(currentSession() && !state.trogdorAtlasOpen);
  document.body.classList.toggle("terminal-focus-mode", terminalFocusMode);
  el.terminalStage.classList.toggle("terminal-view-active", terminalFocusMode);
  syncTerminalInputDock();
  syncTrogdorBackButton();
  syncTerminalWorkbench();
  if (state.hud) {
    el.hudCanvas.classList.toggle("hidden", terminalFocusMode);
    el.hudCanvas.style.display = terminalFocusMode ? "none" : "";
    el.hudCanvas.style.visibility = terminalFocusMode ? "hidden" : "";
  }
  if (state.terminal) {
    el.terminalCanvas.classList.toggle("hidden", false);
    el.terminalCanvas.style.display = "";
    el.terminalCanvas.style.visibility = "";
  }
  el.terminalFallback.classList.toggle("hidden", !(terminalFocusMode && state.terminalFallbackActive));
}

async function connectSelectedSession() {
  await setupHudSurface();

  const session = currentSession();
  if (!session) {
    teardownTerminal();
    return;
  }

  await setupTerminalSurface();
  if (!state.terminal && !state.terminalFallbackActive) {
    return;
  }

  if (state.ws && state.ws.readyState <= WebSocket.OPEN && state.ws.sessionId === session.session_id) {
    return;
  }

  disconnectSocket();
  const generation = state.connectionGeneration;
  const url = sessionSocketUrl(session);
  const resumeFromSeq = url.searchParams.get("resume_from_seq") || "";
  const framedOutput = url.searchParams.get("framed") === "1";

  const ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  ws.sessionId = session.session_id;
  ws.framedOutput = framedOutput;
  state.ws = ws;
  setConnectionStatus(
    resumeFromSeq ? `connecting; resuming from seq ${resumeFromSeq}` : "connecting; input disabled",
  );

  ws.onopen = () => {
    if (generation !== state.connectionGeneration || state.ws !== ws) {
      ws.close();
      return;
    }
    measureAndResizeSurface(true, true);
    state.reconnectAttempt = 0;
    setConnectionStatus("attached");
    scheduleSessionRefresh();
  };

  ws.onmessage = (event) => {
    if (generation !== state.connectionGeneration || state.ws !== ws) {
      return;
    }

    if (typeof event.data === "string") {
      handleSocketText(event.data);
      return;
    }

    const terminalBytes = terminalPayloadFromSocketBytes(new Uint8Array(event.data), ws);
    feedTerminalBytes(terminalBytes);
  };

  ws.onclose = () => {
    if (generation !== state.connectionGeneration) {
      return;
    }
    const delay = reconnectDelayMs();
    state.reconnectAttempt += 1;
    setConnectionStatus(`disconnected; input disabled; retrying in ${Math.ceil(delay / 1000)}s`, true);
    scheduleSessionRefresh();
    state.reconnectTimer = window.setTimeout(() => {
      state.reconnectTimer = null;
      if (generation !== state.connectionGeneration || !currentSession()) {
        return;
      }
      connectSelectedSession();
    }, delay);
  };

  ws.onerror = () => {
    setConnectionStatus("attach failed; input disabled", true);
  };
}

function sessionSocketUrl(session) {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = new URL(`${protocol}//${window.location.host}/ws/sessions/${encodeURIComponent(session.session_id)}`);
  if (state.token) {
    url.searchParams.set("token", state.token);
  }
  url.searchParams.set("framed", "1");
  const resumeFromSeq = state.lastTerminalSeqBySession.get(session.session_id);
  if (resumeFromSeq && /^\d+$/.test(String(resumeFromSeq)) && String(resumeFromSeq) !== "0") {
    url.searchParams.set("resume_from_seq", String(resumeFromSeq));
  }
  return url;
}

function terminalPayloadFromSocketBytes(bytes, ws = state.ws) {
  if (!(bytes instanceof Uint8Array) || !ws?.framedOutput) {
    return bytes;
  }
  const frame = decodeTerminalOutputFrame(bytes);
  if (!frame) {
    return bytes;
  }
  if (ws.sessionId) {
    state.lastTerminalSeqBySession.set(ws.sessionId, frame.seq);
  }
  return frame.payload;
}

function decodeTerminalOutputFrame(bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength < 9 || bytes[0] !== TERMINAL_OUTPUT_OPCODE) {
    return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const high = view.getUint32(1);
  const low = view.getUint32(5);
  const seq = readUint64Decimal(high, low);
  return {
    seq,
    payload: bytes.slice(9),
  };
}

function readUint64Decimal(high, low) {
  if (typeof BigInt === "function") {
    return ((BigInt(high) << 32n) | BigInt(low)).toString();
  }
  const numeric = high * 4294967296 + low;
  return Number.isSafeInteger(numeric) ? String(numeric) : "";
}

function feedTerminalBytes(bytes) {
  if (!(bytes instanceof Uint8Array)) {
    return false;
  }
  if (!state.terminal || !state.terminalAcceptsBytes) {
    return bufferTerminalBytes(bytes);
  }

  state.terminal.feed(bytes);
  state.terminalFrameBytesSeen += bytes.byteLength;
  flushEncodedInputBytes();
  if (state.searchQuery) {
    refreshTerminalSearch();
  }
  drainTerminalLinkClicks();
  syncTerminalAccessibilityMirror();
  syncTerminalFallbackFromLiveFrame();
  scheduleRender();
  scheduleTerminalPaintProbe();
  return true;
}

function syncTerminalFallbackFromLiveFrame() {
  if (!state.terminalFallbackActive || !state.terminal) {
    return false;
  }
  const text = terminalMirrorTextFromRenderer();
  if (!terminalTextHasContent(text) && terminalTextHasContent(el.terminalFallback.textContent)) {
    return false;
  }
  if (!terminalTextHasContent(text)) {
    return false;
  }
  updateTerminalFallbackText(text);
  return true;
}

function scheduleTerminalPaintProbe() {
  if (
    state.terminalPaintVerified ||
    state.terminalFallbackActive ||
    state.terminalPaintProbeTimer ||
    !state.terminal ||
    !currentSession() ||
    state.terminalFrameBytesSeen === 0
  ) {
    return;
  }

  state.terminalPaintProbeTimer = window.setTimeout(() => {
    state.terminalPaintProbeTimer = null;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        void verifyTerminalPaintOrFallback();
      });
    });
  }, 180);
}

async function verifyTerminalPaintOrFallback() {
  if (!state.terminal || state.terminalPaintVerified || state.terminalFallbackActive || !currentSession()) {
    return;
  }

  if (terminalCanvasHasVisiblePixels()) {
    state.terminalPaintVerified = true;
    captureTerminalRendererDiagnostic("painted");
    setTerminalTextFallbackActive(false);
    return;
  }

  const hasSnapshotText = await refreshSnapshotFallback();
  if (!state.terminal || state.terminalPaintVerified || state.terminalFallbackActive || !currentSession()) {
    return;
  }
  if (terminalCanvasHasVisiblePixels()) {
    state.terminalPaintVerified = true;
    captureTerminalRendererDiagnostic("painted");
    setTerminalTextFallbackActive(false);
    return;
  }
  if (hasSnapshotText) {
    setTerminalTextFallbackActive(true, { clearText: false });
    syncTerminalPresentation();
  }
}

function terminalCanvasHasVisiblePixels() {
  const canvas = el.terminalCanvas;
  if (!canvas || !canvas.width || !canvas.height) {
    return false;
  }

  const sample = document.createElement("canvas");
  sample.width = Math.min(180, canvas.width);
  sample.height = Math.min(120, canvas.height);
  if (!sample.width || !sample.height) {
    return false;
  }

  const context = sample.getContext("2d", { willReadFrequently: true });
  if (!context) {
    return false;
  }

  try {
    context.drawImage(canvas, 0, 0, sample.width, sample.height);
    const pixels = context.getImageData(0, 0, sample.width, sample.height).data;
    for (let index = 0; index < pixels.length; index += 4) {
      const alpha = pixels[index + 3];
      const red = pixels[index];
      const green = pixels[index + 1];
      const blue = pixels[index + 2];
      if (alpha > 0 && (red > 32 || green > 32 || blue > 32)) {
        return true;
      }
    }
  } catch (_) {
    return false;
  }

  return false;
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
        scheduleSessionRefresh();
        break;
      case "replay_truncated":
        setConnectionStatus("partial replay", true);
        break;
      case "error":
        setConnectionStatus(message.code || "error", true);
        break;
      case "overloaded":
        setConnectionStatus(`server overloaded; input disabled; retrying in ${Math.ceil((message.retryAfterMs || 4000) / 1000)}s`, true);
        break;
      case "input_ack":
        handleInputAck(message);
        break;
      case "control_event":
        applyControlEvent(message);
        break;
      case "lifecycle_event":
        applyLifecycleEvent(message);
        break;
      case "event_stream_lagged":
        setConnectionStatus("event stream lagged", true);
        void refreshSessions();
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

function applyControlEvent(message) {
  const sessionId = normalizeSessionId(message.sessionId || message.session_id);
  if (!sessionId) {
    return;
  }
  const index = state.sessions.findIndex((session) => session.session_id === sessionId);
  if (index < 0) {
    return;
  }

  const payload = message.payload && typeof message.payload === "object" ? message.payload : {};
  const event = String(message.event || "");
  const session = { ...state.sessions[index], last_control_event: event };

  if (event === "session_state") {
    if (payload.state) session.state = payload.state;
    if ("previous_state" in payload) session.previous_state = payload.previous_state;
    if ("current_command" in payload) session.current_command = payload.current_command;
    if (payload.state_evidence && typeof payload.state_evidence === "object") {
      session.state_evidence = payload.state_evidence;
    }
    if (payload.transport_health) session.transport_health = payload.transport_health;
    if (payload.exit_reason) session.exit_reason = payload.exit_reason;
    if (payload.at) session.last_activity_at = payload.at;
  } else if (event === "session_title") {
    const title = String(payload.title || "").trim();
    if (title) {
      session.terminal_title = title;
      if (title.startsWith("/")) {
        session.cwd = title;
      }
    }
  } else if (event === "session_skill") {
    if ("last_skill" in payload) {
      session.last_skill = payload.last_skill;
    }
  } else if (event === "thought_update") {
    if ("thought" in payload) session.thought = payload.thought;
    if ("token_count" in payload) session.token_count = payload.token_count;
    if ("context_limit" in payload) session.context_limit = payload.context_limit;
    if ("thought_state" in payload) session.thought_state = payload.thought_state;
    if ("thought_source" in payload) session.thought_source = payload.thought_source;
    if ("rest_state" in payload) session.rest_state = payload.rest_state;
    if ("commit_candidate" in payload) session.commit_candidate = Boolean(payload.commit_candidate);
    if (Array.isArray(payload.action_cues)) session.action_cues = payload.action_cues;
    if (payload.at) session.thought_updated_at = payload.at;
    if (payload.objective_changed && payload.at) session.objective_changed_at = payload.at;
  }

  state.sessions[index] = session;
  syncTrogdorCueTransitions();
  syncTerminalStatusStrip();
  renderHudSurface();
  refreshSelectedSessionSidecarsFromEvent(sessionId, event);
}

function applyLifecycleEvent(message) {
  const sessionId = normalizeSessionId(message.sessionId || message.session_id);
  if (!sessionId) {
    return;
  }

  if (message.event === "session_created" && message.summary) {
    mergeSummary(message.summary);
    return;
  }

  if (message.event !== "session_deleted") {
    return;
  }

  const index = state.sessions.findIndex((session) => session.session_id === sessionId);
  if (index < 0) {
    return;
  }

  state.sessions[index] = {
    ...state.sessions[index],
    state: "exited",
    is_stale: true,
    transport_health: "disconnected",
    delete_reason: message.reason || "",
    delete_mode: message.deleteMode || message.delete_mode || "",
    tmux_session_alive: Boolean(message.tmuxSessionAlive ?? message.tmux_session_alive),
  };
  if (state.selectedSessionId === sessionId) {
    setConnectionStatus("session ended", true);
  } else {
    syncTerminalStatusStrip();
    renderHudSurface();
  }
}

function refreshSelectedSessionSidecarsFromEvent(sessionId, event) {
  if (sessionId !== state.selectedSessionId) {
    return;
  }
  if (!["session_state", "session_skill", "thought_update"].includes(event)) {
    return;
  }
  void refreshAgentContextForSelectedSession({ throttle: true, silent: true });
  void refreshWorkbenchWidgetsForSelectedSession({ throttle: true, silent: true });
}

function handleInputAck(message) {
  const id = message.clientMessageId || message.client_message_id || "";
  if (!id) {
    return;
  }
  if (message.delivered) {
    updateInputDeliveryStatus(id, "sent", message.method || "");
    scheduleInputAckCleanup(id, "sent", 2500);
  } else {
    updateInputDeliveryStatus(id, "failed", message.message || "input delivery failed");
    // Failed acks also need eviction, otherwise pendingInputMessages grows
    // without bound over a long session. Keep the failure visible a bit longer
    // than a success before clearing it.
    scheduleInputAckCleanup(id, "failed", 8000);
  }
}

function scheduleInputAckCleanup(id, expectedStatus, delayMs) {
  const timer = window.setTimeout(() => {
    const current = state.pendingInputMessages.get(id);
    if (current?.status === expectedStatus) {
      state.pendingInputMessages.delete(id);
    }
  }, delayMs);
  if (timer && typeof timer.unref === "function") {
    timer.unref();
  }
}

function mergeSummary(summary) {
  const index = state.sessions.findIndex((session) => session.session_id === summary.session_id);
  if (index >= 0) {
    state.sessions[index] = summary;
  } else if (summary?.session_id) {
    state.sessions.push(summary);
  }
  syncTerminalStatusStrip();
  renderHudSurface();
}

function syncWriteAccess() {
  el.sendInput.disabled = state.readOnly;
  el.sendSubmitButton.disabled = !sendTargetReady();
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

function sendTerminalInputText(text) {
  if (!text || !state.ws || state.ws.readyState !== WebSocket.OPEN || state.readOnly) {
    return false;
  }
  if (rejectOversizeTerminalText(text, "Input")) {
    return false;
  }
  const clientMessageId = nextInputMessageId();
  state.pendingInputMessages.set(clientMessageId, { text, status: "pending", detail: "" });
  updateInputDeliveryStatus(clientMessageId, "pending");
  state.ws.send(JSON.stringify({ type: "input_text", data: text, clientMessageId }));
  return true;
}

function fallbackTextForKeyEvent(event) {
  if (!event || event.kind !== "key" || event.phase !== "down") {
    return "";
  }

  const key = typeof event.key === "string" ? event.key : "";
  const mods = Number(event.mods) || 0;
  const shift = (mods & 1) !== 0;
  const alt = (mods & 2) !== 0;
  const ctrl = (mods & 4) !== 0;
  const prefix = alt ? "\x1b" : "";

  if (ctrl && key.length === 1) {
    const upper = key.toUpperCase();
    const code = upper.charCodeAt(0);
    if (code >= 64 && code <= 95) {
      return prefix + String.fromCharCode(code - 64);
    }
  }

  if (!ctrl && key.length === 1) {
    return prefix + key;
  }

  switch (key) {
    case "Enter":
      return "\r";
    case "Backspace":
      return "\x7f";
    case "Delete":
      return "\x1b[3~";
    case "Tab":
      return shift ? "\x1b[Z" : "\t";
    case "Escape":
      return "\x1b";
    case "ArrowUp":
      return "\x1b[A";
    case "ArrowDown":
      return "\x1b[B";
    case "ArrowRight":
      return "\x1b[C";
    case "ArrowLeft":
      return "\x1b[D";
    case "Home":
      return "\x1b[H";
    case "End":
      return "\x1b[F";
    case "PageUp":
      return "\x1b[5~";
    case "PageDown":
      return "\x1b[6~";
    default:
      return "";
  }
}

function terminalControlKeyEvent(actionId) {
  switch (String(actionId || "")) {
    case "ctrl-c":
      return { key: "c", code: "KeyC", mods: 4, label: "Ctrl-C" };
    case "escape":
      return { key: "Escape", code: "Escape", mods: 0, label: "Esc" };
    case "tab":
      return { key: "Tab", code: "Tab", mods: 0, label: "Tab" };
    case "arrow-up":
      return { key: "ArrowUp", code: "ArrowUp", mods: 0, label: "Up" };
    case "arrow-down":
      return { key: "ArrowDown", code: "ArrowDown", mods: 0, label: "Down" };
    case "arrow-left":
      return { key: "ArrowLeft", code: "ArrowLeft", mods: 0, label: "Left" };
    case "arrow-right":
      return { key: "ArrowRight", code: "ArrowRight", mods: 0, label: "Right" };
    case "home":
      return { key: "Home", code: "Home", mods: 0, label: "Home" };
    case "end":
      return { key: "End", code: "End", mods: 0, label: "End" };
    case "page-up":
      return { key: "PageUp", code: "PageUp", mods: 0, label: "PgUp" };
    case "page-down":
      return { key: "PageDown", code: "PageDown", mods: 0, label: "PgDn" };
    default:
      return null;
  }
}

function sendTerminalControlKey(actionId) {
  if (state.readOnly || !currentSession()) {
    return false;
  }
  const spec = terminalControlKeyEvent(actionId);
  if (!spec) {
    return false;
  }
  const event = {
    kind: "key",
    phase: "down",
    key: spec.key,
    code: spec.code,
    mods: spec.mods,
    repeat: false,
  };

  if ((state.terminalFallbackActive || !state.terminal) && sendFallbackTerminalEvent(event)) {
    setTerminalInputEcho(`sent: ${spec.label}`);
    return true;
  }
  if (state.terminalFallbackActive || !state.terminal) {
    setTerminalInputEcho(`failed: ${spec.label}`);
    return false;
  }

  forwardTerminalEvent(event);
  setTerminalInputEcho(`sent: ${spec.label}`);
  return true;
}

function terminalKeyActionForDomEvent(event) {
  if (!event || event.metaKey || event.altKey) {
    return "";
  }
  if (event.ctrlKey && String(event.key || "").toLowerCase() === "c") {
    if (terminalInlineInputHasSelection()) {
      return "";
    }
    return "ctrl-c";
  }
  if (String(el.terminalInlineInput?.value || "").length > 0) {
    return "";
  }
  switch (event.key) {
    case "Escape":
      return "escape";
    case "Tab":
      return "tab";
    case "ArrowUp":
      return "arrow-up";
    case "ArrowDown":
      return "arrow-down";
    case "ArrowLeft":
      return "arrow-left";
    case "ArrowRight":
      return "arrow-right";
    case "Home":
      return "home";
    case "End":
      return "end";
    case "PageUp":
      return "page-up";
    case "PageDown":
      return "page-down";
    default:
      return "";
  }
}

function terminalInlineInputHasSelection() {
  const start = Number(el.terminalInlineInput?.selectionStart);
  const end = Number(el.terminalInlineInput?.selectionEnd);
  return Number.isFinite(start) && Number.isFinite(end) && start !== end;
}

function sendFallbackTerminalEvent(event) {
  const text = fallbackTextForKeyEvent(event);
  if (!text) {
    return false;
  }
  return sendTerminalInputText(text);
}

function forwardTerminalEvent(event) {
  if (state.terminalFallbackActive && sendFallbackTerminalEvent(event)) {
    return;
  }
  if (!state.terminal || state.readOnly) {
    return;
  }
  state.terminal.input(event);
  flushEncodedInputBytes();
  drainTerminalLinkClicks();
}

function forwardTerminalKeyDown(event) {
  forwardTerminalEvent({
    kind: "key",
    phase: "down",
    key: typeof event.key === "string" ? event.key : "",
    code: typeof event.code === "string" ? event.code : "",
    mods: keyModifiers(event),
    repeat: Boolean(event.repeat),
  });
}

function forwardTerminalMouse(phase, button, hit, event) {
  forwardTerminalEvent({
    kind: "mouse",
    phase,
    button,
    x: hit.cell.x,
    y: hit.cell.y,
    mods: keyModifiers(event),
  });
}

function sendTerminalText(text) {
  if (!text || state.readOnly || !currentSession()) {
    return false;
  }
  if (rejectOversizeTerminalText(text, "Paste")) {
    return false;
  }
  markTrogdorSessionsResponded([state.selectedSessionId]);
  if (state.terminalFallbackActive && sendTerminalInputText(text)) {
    return true;
  }
  if (terminalSupports("pasteText")) {
    state.terminal.pasteText(text);
    flushEncodedInputBytes();
    return true;
  }
  if (state.ws && state.ws.readyState === WebSocket.OPEN) {
    sendTerminalInputText(text);
    return true;
  }
  forwardTerminalEvent({ kind: "paste", data: text });
  return true;
}

function isCoarsePointer() {
  return window.matchMedia?.("(pointer: coarse)")?.matches ?? false;
}

function syncMobileKeyboardState() {
  document.body.classList.toggle("mobile-keyboard-active", state.mobileKeyboardActive);
  if (el.terminalMobileKeyboard) {
    el.terminalMobileKeyboard.setAttribute("aria-pressed", state.mobileKeyboardActive ? "true" : "false");
  }
}

function focusMobileKeyboard() {
  if (state.readOnly || !currentSession()) {
    return false;
  }
  state.mobileKeyboardActive = true;
  syncMobileKeyboardState();
  el.mobileKeyboardProxy.value = "";
  el.mobileKeyboardProxy.focus({ preventScroll: true });
  forwardTerminalEvent({ kind: "focus", focused: true });
  return true;
}

function terminalInputSurfaceHasFocus() {
  const active = document.activeElement;
  return !active || active === document.body || active === el.terminalStage || active === el.terminalFallback;
}

function focusTerminalInputSurface(options = {}) {
  if (state.activeSheet && !options.force) {
    return false;
  }
  if (options.onlyIfSurfaceFocused && !terminalInputSurfaceHasFocus()) {
    return false;
  }
  const target = state.terminalFallbackActive ? el.terminalFallback : el.terminalStage;
  if (!target || typeof target.focus !== "function") {
    return false;
  }
  target.focus({ preventScroll: Boolean(options.preventScroll) });
  return document.activeElement === target;
}

function closeMobileKeyboard() {
  state.mobileKeyboardActive = false;
  syncMobileKeyboardState();
  if (document.activeElement === el.mobileKeyboardProxy) {
    el.mobileKeyboardProxy.blur();
  }
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

function handleTerminalFallbackKeyEvent(event) {
  if (!state.terminalFallbackActive) {
    return false;
  }
  if (handleGlobalShortcut(event)) {
    event.preventDefault();
    event.stopPropagation?.();
    return true;
  }
  if (!shouldCaptureKey(event)) {
    return false;
  }
  event.preventDefault();
  event.stopPropagation?.();
  if (keyBeginsTrogdorResponse(event)) {
    markTrogdorSessionsResponded([state.selectedSessionId]);
  }
  forwardTerminalKeyDown(event);
  return true;
}

function handleTerminalFallbackPasteEvent(event) {
  if (!state.terminalFallbackActive || state.readOnly || !currentSession()) {
    return false;
  }
  const text = event.clipboardData?.getData("text") ?? "";
  if (!text) {
    return false;
  }
  event.preventDefault();
  event.stopPropagation?.();
  sendTerminalText(text);
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
    if (url.protocol === "http:" && !frankenTermLinkPolicy().allowHttp) {
      setUtilityStatus(`Blocked non-local HTTP link: ${shortenUrl(url.toString())}`, true, 2600);
      return;
    }
    window.open(url.toString(), "_blank", "noopener,noreferrer");
  } catch (error) {
    setUtilityStatus(`Invalid link: ${error.message}`, true, 2600);
  }
}

function syncLinkTools() {
  if (!el.terminalLinkTools) {
    return;
  }
  const visible = Boolean(state.hoveredLinkUrl && currentSession() && !state.activeSheet && !state.selectMode);
  el.terminalLinkTools.classList.toggle("hidden", !visible);
  if (el.terminalLinkText) {
    el.terminalLinkText.textContent = visible ? shortenUrl(state.hoveredLinkUrl) : "";
  }
}

async function copyHoveredLink() {
  if (!state.hoveredLinkUrl) {
    setUtilityStatus("No terminal link is currently hovered.", true, 2200);
    return false;
  }
  if (!navigator.clipboard?.writeText) {
    setUtilityStatus("Clipboard write is unavailable in this browser context.", true, 3000);
    return false;
  }
  try {
    await navigator.clipboard.writeText(state.hoveredLinkUrl);
    setUtilityStatus(`Copied ${shortenUrl(state.hoveredLinkUrl)}.`, false, 2200);
    return true;
  } catch (error) {
    setUtilityStatus(`Clipboard write failed: ${error.message}`, true, 3000);
    return false;
  }
}

function drainTerminalLinkClicks() {
  if (!terminalSupports("drainLinkClicks")) {
    return;
  }
  const clicks = state.terminal.drainLinkClicks();
  if (!Array.isArray(clicks) || !clicks.length) {
    return;
  }
  for (const click of clicks) {
    const url = click?.url || click?.href || "";
    if (!url) {
      continue;
    }
    if (click.openAllowed === false) {
      setUtilityStatus(click.openReason || `Blocked ${shortenUrl(url)}.`, true, 2600);
      continue;
    }
    safeOpenUrl(url);
  }
}

async function copyTerminalFrameText() {
  const text =
    state.terminalMirrorText ||
    (terminalSupports("screenReaderMirrorText") && state.terminal.screenReaderMirrorText()) ||
    (terminalSupports("accessibilityDomSnapshot") && state.terminal.accessibilityDomSnapshot()?.value) ||
    el.terminalFallback.textContent ||
    "";
  if (!text.trim()) {
    setUtilityStatus("No terminal text is available to copy.", true, 2400);
    return false;
  }
  if (!navigator.clipboard?.writeText) {
    setUtilityStatus("Clipboard write is unavailable in this browser context.", true, 3000);
    return false;
  }
  try {
    await navigator.clipboard.writeText(text);
    setUtilityStatus(`Copied ${text.length} visible terminal characters.`, false, 2200);
    return true;
  } catch (error) {
    setUtilityStatus(`Clipboard write failed: ${error.message}`, true, 3000);
    return false;
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
  syncLinkTools();
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
  syncLinkTools();
  syncTerminalTools();
}

async function sendLine(text) {
  return sendLineToSession(state.selectedSessionId, text);
}

function loadSendHistory() {
  try {
    const parsed = JSON.parse(localStorage.getItem(SEND_HISTORY_KEY) || "[]");
    state.sendHistory = Array.isArray(parsed)
      ? parsed.map((item) => String(item || "")).filter(Boolean).slice(0, SEND_HISTORY_LIMIT)
      : [];
  } catch (_error) {
    state.sendHistory = [];
  }
}

function saveSendHistory() {
  localStorage.setItem(SEND_HISTORY_KEY, JSON.stringify(state.sendHistory.slice(0, SEND_HISTORY_LIMIT)));
}

function rememberSendHistory(text) {
  const normalized = String(text || "").trim();
  if (!normalized) {
    return;
  }
  state.sendHistory = [
    normalized,
    ...state.sendHistory.filter((item) => item !== normalized),
  ].slice(0, SEND_HISTORY_LIMIT);
  saveSendHistory();
  renderSendHistory();
}

function renderSendHistory() {
  if (!el.sendHistory) {
    return;
  }
  const items = state.sendHistory.slice(0, 6);
  el.sendHistory.innerHTML = items
    .map((item, index) => {
      const label = item.replace(/\s+/g, " ").trim();
      return `<button class="ghost-button" type="button" data-send-history-index="${index}" title="${escapeHtml(label)}">${escapeHtml(label.length > 42 ? `${label.slice(0, 39)}...` : label)}</button>`;
    })
    .join("");
}

async function sendLineToSession(sessionId, text) {
  const targetSessionId = normalizeSessionId(sessionId);
  if (!text || !targetSessionId) {
    return;
  }

  if (
    state.ws &&
    state.ws.readyState === WebSocket.OPEN &&
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
    state.ws.readyState === WebSocket.OPEN &&
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

function deliveredGroupInputSessionIds(body) {
  if (!Array.isArray(body?.results)) {
    return [];
  }
  return body.results
    .filter((result) => result?.ok)
    .map((result) => normalizeSessionId(result?.session_id))
    .filter(Boolean);
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
  const deliveredSessionIds = deliveredGroupInputSessionIds(body);
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

function sendModeValue() {
  return String(el.sendMode?.value || "line") === "paste" ? "paste" : "line";
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

function sendTargetReady() {
  if (state.readOnly) {
    return false;
  }
  if (!state.sendTarget) {
    return Boolean(currentSession());
  }
  if (state.sendTarget.type === "group") {
    return Array.isArray(state.sendTarget.sessionIds) && state.sendTarget.sessionIds.length >= 2;
  }
  return Boolean(normalizeSessionId(state.sendTarget.sessionId));
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

function openCreateSheetForCwd(cwd) {
  const path = String(cwd || "").trim();
  if (path) {
    el.createCwd.value = path;
    el.dirsPath.value = path;
    state.dirBrowser.path = path;
  }
  state.dirBrowser.group = "";
  clearCreateBatchSelection();
  openSheet("create");
}

function selectedBatchDirs() {
  return Array.from(ensureDirBrowserBatchSelection())
    .map((dir) => String(dir || "").trim())
    .filter(Boolean);
}

function batchFailureLines(results) {
  return results
    .filter((result) => !result?.ok)
    .map((result) => {
      const cwd = String(result?.cwd || "(unknown)");
      const message = result?.error?.message || result?.error?.code || "unknown error";
      return `${cwd} (${message})`;
    });
}

async function createBatchSessionsFromSheet(dirs, spawnTool, initialRequest) {
  const response = await apiFetch("/v1/sessions/batch", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      dirs,
      spawn_tool: spawnTool || "grok",
      launch_target: launchTargetPayload(),
      initial_request: initialRequest || "",
    }),
  });
  const payload = await response.json();
  const results = Array.isArray(payload?.results) ? payload.results : [];
  const total = dirs.length;
  const successResults = results.filter((result) => result?.ok);
  const successCount = successResults.length;
  const failures = batchFailureLines(results);
  const failCount = failures.length;

  if (successCount > 0) {
    closeSheets();
    clearCreateBatchSelection();
    await refreshSessions();
    const firstSessionId = successResults.find((result) => result?.session?.session_id)?.session?.session_id;
    if (firstSessionId) {
      await selectSession(firstSessionId);
    }
  }

  if (failCount > 0) {
    const preview = failures.slice(0, 3).join("; ");
    const overflow = failCount > 3 ? ` (+${failCount - 3} more)` : "";
    const prefix = response.status === 207 ? "Batch send partial" : "Batch send failed";
    setUtilityStatus(`${prefix}: ${successCount}/${total} created. Failed: ${preview}${overflow}`, true, 6200);
    if (successCount === 0) {
      setDirStatus(`Batch send failed for all ${total}: ${preview}${overflow}`, true);
    }
    return;
  }

  setUtilityStatus(`Batch send created ${successCount}/${total} sessions.`, false, 3600);
}

async function createSessionFromSheet() {
  if (state.readOnly) {
    return;
  }

  const batchDirs = selectedBatchDirs();
  const cwd = el.createCwd.value.trim();
  const initialRequest = el.createRequest.value.trim();
  const spawnTool = el.createTool.value;

  if (batchDirs.length > 0) {
    await createBatchSessionsFromSheet(batchDirs, spawnTool, initialRequest);
    return;
  }

  const response = await apiFetch("/v1/sessions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      cwd: cwd || null,
      spawn_tool: spawnTool,
      launch_target: launchTargetPayload(),
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
    return false;
  }

  try {
    const response = await apiFetch(`/v1/sessions/${encodeURIComponent(session.session_id)}/snapshot`);
    const payload = await response.json();
    updateTerminalFallbackText(payload.screen_text || "");
    syncTerminalTools();
    return Boolean(payload.screen_text);
  } catch (error) {
    updateTerminalFallbackText(`Snapshot unavailable: ${error.message}`);
    return false;
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
  const initialPath = preferredPath || state.dirBrowser.path || "";
  ensureDirBrowserBatchSelection().clear();
  state.dirBrowser.group = "";
  if (initialPath) {
    el.createCwd.value = initialPath;
    el.dirsPath.value = initialPath;
  }
  if (typeof state.dirBrowser.managedOnly !== "boolean") {
    state.dirBrowser.managedOnly = false;
  }
  el.dirsManagedOnly.checked = state.dirBrowser.managedOnly;
  renderCreateBatchBar();
  if (!state.dirBrowser.entries.length || (initialPath && initialPath !== state.dirBrowser.path)) {
    await loadDirListing(initialPath, state.dirBrowser.managedOnly, "");
  } else {
    renderDirEntries({
      path: state.dirBrowser.path,
      entries: state.dirBrowser.entries,
      groups: state.dirBrowser.groups,
      overlay_label: state.dirBrowser.overlayLabel || undefined,
      launch_targets: state.dirBrowser.launchTargets,
      default_launch_target: state.dirBrowser.launchTarget,
    });
  }
  focusActiveSheet();
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

function commandPaletteItems() {
  const selected = currentSession();
  const baseItems = [
    { label: "Focus terminal", meta: "terminal", actionId: "focus_terminal", disabled: !selected },
    { label: "Search terminal", meta: "Ctrl+Shift+F", actionId: "open_search", disabled: !selected },
    { label: "Send to terminal", meta: "Ctrl+Shift+S", actionId: "open_send", disabled: state.readOnly || !selected },
    { label: "Copy selection", meta: "Ctrl+Shift+C", actionId: "copy_selection", disabled: !selected },
    { label: "Copy visible text", meta: "frame", action: copyTerminalFrameText, disabled: !selected },
    { label: "Toggle select mode", meta: "Ctrl+Shift+V", actionId: "toggle_select", disabled: !selected },
    { label: "Open native terminal", meta: "desktop", actionId: "open_native", disabled: !selected },
    { label: "Open Mermaid artifacts", meta: "artifacts", actionId: "open_mermaid", disabled: !selected },
    { label: "Create session", meta: "spawn", actionId: "open_create", disabled: state.readOnly },
    { label: "Refresh sessions", meta: "sync", actionId: "refresh" },
    { label: "Toggle follow published", meta: "selection", actionId: "toggle_follow" },
    { label: "Thought config", meta: "policy", actionId: "open_config" },
    { label: "Auth token", meta: "connection", actionId: "open_auth" },
    { label: "Toggle Trogdor atlas", meta: "overview", actionId: "toggle_trogdor_atlas" },
  ];
  const sessionItems = state.sessions.map((session) => ({
    label: `Switch to ${sessionDisplayName(session)}`,
    meta: `${session.session_id}  ${session.state || ""}`,
    sessionId: session.session_id,
  }));
  return [...baseItems, ...sessionItems];
}

function commandPaletteScore(item, query) {
  const haystack = `${item.label} ${item.meta || ""}`.toLowerCase();
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

function filteredCommandPaletteItems() {
  const query = String(el.paletteSearch?.value || "").trim().toLowerCase();
  return commandPaletteItems()
    .map((item) => ({ ...item, score: commandPaletteScore(item, query) }))
    .filter((item) => !query || item.score > 0)
    .sort((a, b) => b.score - a.score || a.label.localeCompare(b.label))
    .slice(0, 18);
}

function renderCommandPalette() {
  if (!el.paletteResults) {
    return;
  }
  state.paletteItems = filteredCommandPaletteItems();
  state.paletteIndex = clampInt(state.paletteIndex, 0, 0, Math.max(0, state.paletteItems.length - 1));
  if (!state.paletteItems.length) {
    el.paletteResults.innerHTML = `<div class="sheet-copy">No matching commands.</div>`;
    return;
  }
  el.paletteResults.innerHTML = state.paletteItems
    .map((item, index) => `
      <button
        class="palette-item${index === state.paletteIndex ? " is-active" : ""}"
        type="button"
        role="option"
        aria-selected="${index === state.paletteIndex ? "true" : "false"}"
        data-palette-index="${index}"
        ${item.disabled ? "disabled" : ""}
      >
        <span class="palette-item-title">${escapeHtml(item.label)}</span>
        <span class="palette-item-meta">${escapeHtml(item.disabled ? "unavailable" : item.meta || "")}</span>
      </button>
    `)
    .join("");
}

async function runCommandPaletteItem(item = state.paletteItems[state.paletteIndex]) {
  if (!item || item.disabled) {
    return false;
  }
  closeSheets();
  if (item.sessionId) {
    await selectSession(item.sessionId);
    return true;
  }
  if (typeof item.action === "function") {
    await item.action();
    return true;
  }
  if (item.actionId) {
    await handleSurfaceAction({ type: "action", actionId: item.actionId });
    return true;
  }
  return false;
}

function openCommandPalette() {
  setActiveSheet("palette");
  if (el.paletteSearch) {
    el.paletteSearch.value = "";
  }
  state.paletteIndex = 0;
  renderCommandPalette();
  focusActiveSheet();
}

function setActiveSheet(sheetId) {
  state.activeSheet = sheetId;
  document.body.classList.toggle("sheet-open", Boolean(sheetId));
  el.modalRoot.classList.toggle("visible", Boolean(sheetId));
  el.modalRoot.setAttribute("aria-hidden", sheetId ? "false" : "true");
  el.paletteSheet.classList.toggle("hidden", sheetId !== "palette");
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
      case "palette":
        el.paletteSearch.focus();
        el.paletteSearch.select();
        break;
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
        {
          const firstCheckbox = el.dirsList.querySelector(".dir-row-check:not(:disabled)");
          if (firstCheckbox) {
            firstCheckbox.focus();
          } else {
            el.createCwd.focus();
          }
        }
        break;
      case "mermaid":
        el.mermaidRefreshButton.focus();
        break;
      default:
        focusTerminalInputSurface({ preventScroll: true });
        break;
    }
  });
}

function openSheet(sheetId) {
  setActiveSheet(sheetId);
  if (sheetId === "palette") {
    renderCommandPalette();
  }
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
  if (state.activeSheet === "send") {
    state.sendTarget = null;
  }
  if (state.activeSheet === "create") {
    clearCreateBatchSelection();
    state.dirBrowser.group = "";
  }
  setActiveSheet(null);
  focusTerminalInputSurface({ preventScroll: true });
}

function closeTrogdorAtlasForTerminal() {
  state.trogdorAtlasOpen = false;
  state.hoveredTrogdorSessionId = null;
  state.trogdorReaderStartedAt = 0;
  state.trogdorReaderStartIndex = 0;
  state.trogdorReaderClawgKey = "";
  state.trogdorSurfaceSignature = "";
  syncTrogdorReaderTimer();
  applyTrogdorAtlasVisibility();
  syncTerminalPresentation();
}

function openTrogdorAtlas() {
  state.trogdorAtlasOpen = true;
  state.trogdorSurfaceSignature = "";
  closeMobileKeyboard();
  renderHudSurface();
  setUtilityStatus("Back to Trogdor atlas.", false, 1600);
}

function applyTrogdorAtlasVisibility() {
  const visible = Boolean(state.trogdorAtlasOpen);
  if (el.trogdorSurface) {
    el.trogdorSurface.classList.toggle("hidden", !visible);
    el.trogdorSurface.setAttribute("aria-hidden", visible ? "false" : "true");
    el.trogdorSurface.style.display = visible ? "" : "none";
  }
  el.trogdorLauncher?.classList.toggle("hidden", visible || Boolean(state.activeSheet));
  document.body.classList.toggle("trogdor-mode", visible);
}

async function selectSession(sessionId) {
  const normalized = normalizeSessionId(sessionId);
  if (!normalized) {
    return;
  }
  closeTrogdorAtlasForTerminal();
  if (state.followPublishedSelection) {
    setFollowPublishedSelection(false);
  }
  persistSelectedSession(normalized);
  renderHudSurface();
  await connectSelectedSession();
  void refreshAgentContextForSelectedSession({ force: true });
  void refreshWorkbenchWidgetsForSelectedSession({ force: true });
  if (state.activeSheet === "mermaid") {
    await refreshMermaidArtifact();
  }
}

async function openTrogdorAgentTerminal(sessionId) {
  const normalized = normalizeSessionId(sessionId);
  if (!normalized) {
    return;
  }

  await selectSession(normalized);
  focusTerminalInputSurface({ preventScroll: true });
  const session = currentSession();
  setUtilityStatus(
    session
      ? `Opened terminal for ${session.tmux_name || session.session_id}.`
      : "Opened terminal for agent.",
    false,
    2200,
  );
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

  if (zone.type === "trogdor_agent") {
    await openTrogdorAgentTerminal(zone.sessionId);
    return;
  }

  if (zone.type === "trogdor_reader") {
    return;
  }

  switch (zone.actionId) {
    case "trogdor_read_toggle":
    {
      const wasReading = state.trogdorReading !== false;
      advanceTrogdorReaderProgressForCurrentHover();
      if (wasReading) {
        state.trogdorReading = false;
      } else {
        const session = currentTrogdorSurfaceSession();
        if (session) {
          startTrogdorReaderForSession(session, { readAgain: trogdorClawgReadComplete(session) });
        } else {
          state.trogdorReading = true;
        }
        state.trogdorReaderStartedAt = performance.now();
      }
      renderHudSurface();
      syncTrogdorReaderTimer();
      break;
    }
    case "trogdor_wpm_down":
    {
      advanceTrogdorReaderProgressForCurrentHover();
      state.trogdorWpm = clampInt(state.trogdorWpm - 25, 200, 50, 800);
      const session = currentTrogdorSurfaceSession();
      state.trogdorReaderStartIndex = session
        ? trogdorClawgReadIndex(session)
        : state.trogdorReaderStartIndex;
      state.trogdorReaderStartedAt = performance.now();
      renderHudSurface();
      break;
    }
    case "trogdor_wpm_up":
    {
      advanceTrogdorReaderProgressForCurrentHover();
      state.trogdorWpm = clampInt(state.trogdorWpm + 25, 200, 50, 800);
      const session = currentTrogdorSurfaceSession();
      state.trogdorReaderStartIndex = session
        ? trogdorClawgReadIndex(session)
        : state.trogdorReaderStartIndex;
      state.trogdorReaderStartedAt = performance.now();
      renderHudSurface();
      break;
    }
    case "toggle_trogdor_atlas":
      state.trogdorAtlasOpen = !state.trogdorAtlasOpen;
      renderHudSurface();
      break;
    case "trogdor_send":
      openSendSheet({
        type: "session",
        sessionId: zone.sessionId,
        label: zone.label || zone.sessionId,
      });
      break;
    case "trogdor_group_send":
      openSendSheet({
        type: "group",
        sessionIds: Array.isArray(zone.sessionIds) ? zone.sessionIds : [],
        label: zone.label || "batch agents",
      });
      break;
    case "trogdor_launch":
      openCreateSheetForCwd(zone.cwd);
      break;
    case "trogdor_mermaid":
      await selectSession(zone.sessionId);
      openMermaidSheet();
      break;
    case "trogdor_commit":
      await selectSession(zone.sessionId);
      await launchCommitGrok();
      break;
    case "open_search":
      openSheet("search");
      break;
    case "open_send":
      if (!state.readOnly && currentSession()) {
        openSendSheet({
          type: "session",
          sessionId: currentSession().session_id,
          label: currentSession().tmux_name || currentSession().session_id,
        });
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
      await launchCommitGrok();
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
      state.trogdorAtlasOpen = false;
      renderHudSurface();
      focusTerminalInputSurface({ preventScroll: true });
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

function terminalFallbackOwnsPointer(event) {
  return Boolean(
    state.terminalFallbackActive &&
      event.target instanceof Element &&
      event.target.closest("#terminal-fallback"),
  );
}

function captureSurfaceAction(event, phase) {
  if (state.activeSheet) {
    return false;
  }
  if (terminalFallbackOwnsPointer(event)) {
    return false;
  }
  if (event.target instanceof Element && event.target.closest("#trogdor-surface, #trogdor-launcher")) {
    return false;
  }
  const hit = surfaceHit(event);
  if (!hit.action && !hit.consume) {
    return false;
  }

  if (hit.action) {
    if (phase === "wheel") {
      event.preventDefault();
      stopSurfaceEvent(event);
      return true;
    }
    if (phase === "click" && shouldIgnoreSyntheticClick(performance.now(), state.surfaceClickSuppressUntil)) {
      event.preventDefault();
      stopSurfaceEvent(event);
      return true;
    }
    if (phase === "down" || phase === "touch" || phase === "click") {
      if (phase === "down" || phase === "touch") {
        state.surfaceClickSuppressUntil = performance.now() + SURFACE_CLICK_SUPPRESS_MS;
      }
      event.preventDefault();
      stopSurfaceEvent(event);
      void handleSurfaceAction(hit.action);
      return true;
    }
  }

  if (hit.consume) {
    event.preventDefault();
    stopSurfaceEvent(event);
    return true;
  }

  return false;
}

function stopSurfaceEvent(event) {
  if (typeof event.stopImmediatePropagation === "function") {
    event.stopImmediatePropagation();
    return;
  }
  event.stopPropagation();
}

function updateHoveredTrogdorSurface(zone) {
  const previousSessionId = state.hoveredTrogdorSessionId;
  const nextSessionId =
    zone?.type === "trogdor_agent" || zone?.type === "trogdor_reader"
      ? zone.sessionId
      : String(zone?.actionId || "").startsWith("trogdor_")
        ? state.hoveredTrogdorSessionId
      : null;
  if (nextSessionId === previousSessionId) {
    return;
  }
  state.hoveredTrogdorSessionId = nextSessionId;
  state.trogdorReaderStartedAt = 0;
  state.trogdorReaderStartIndex = 0;
  state.trogdorReaderClawgKey = "";
  if (el.trogdorSurface) {
    const agents = el.trogdorSurface.querySelectorAll("[data-trogdor-agent]");
    for (const agent of agents) {
      agent.classList.toggle("is-hovered", Boolean(nextSessionId) && agent.dataset.sessionId === nextSessionId);
    }
  }
  if (nextSessionId) {
    const session = state.sessions.find((item) => item.session_id === nextSessionId);
    if (session) {
      startTrogdorReaderForSession(surfaceSession(session));
    }
    setUtilityStatus(
      session
        ? `Speed reading ${session.tmux_name || session.session_id} at ${state.trogdorWpm} wpm.`
        : `Speed reading agent at ${state.trogdorWpm} wpm.`,
      false,
      1200,
    );
  }
  renderHudSurface();
  syncTrogdorReaderTimer();
}

function syncTrogdorReaderTimer() {
  const session = currentTrogdorSurfaceSession();
  const shouldRun = Boolean(
    session && trogdorSessionCanRead(session) && state.trogdorReading && !trogdorClawgReadComplete(session),
  );
  if (shouldRun && !state.trogdorReaderTimer) {
    state.trogdorReaderTimer = window.setInterval(() => {
      renderHudSurface();
    }, 120);
    return;
  }
  if (!shouldRun && state.trogdorReaderTimer) {
    window.clearInterval(state.trogdorReaderTimer);
    state.trogdorReaderTimer = null;
  }
}

async function handleTrogdorDomAction(button) {
  if (!button || button.disabled) {
    return;
  }
  const actionId = String(button.dataset.action || "");
  const zone = {
    type: "action",
    actionId,
  };
  if (button.dataset.sessionId) {
    zone.sessionId = button.dataset.sessionId;
  }
  if (button.dataset.label) {
    zone.label = button.dataset.label;
  }
  if (button.dataset.cwd) {
    zone.cwd = button.dataset.cwd;
  }
  if (button.dataset.sessionIds) {
    try {
      zone.sessionIds = JSON.parse(button.dataset.sessionIds);
    } catch (_error) {
      zone.sessionIds = [];
    }
  }
  await handleSurfaceAction(zone);
}

function bindTrogdorEvents() {
  if (el.trogdorLauncher) {
    el.trogdorLauncher.addEventListener("click", (event) => {
      event.preventDefault();
      openTrogdorAtlas();
    });
  }

  if (!el.trogdorSurface) {
    return;
  }

  el.trogdorSurface.addEventListener("pointerdown", (event) => {
    const agent = event.target instanceof Element ? event.target.closest("[data-trogdor-agent]") : null;
    const sessionId = agent?.dataset?.sessionId || "";
    if (!sessionId) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    void openTrogdorAgentTerminal(sessionId);
  });

  for (const eventName of ["mousedown", "mouseup", "mousemove", "touchend", "wheel"]) {
    el.trogdorSurface.addEventListener(
      eventName,
      (event) => {
        event.stopPropagation();
      },
      eventName === "wheel" || eventName === "touchend" ? { passive: false } : undefined,
    );
  }

  el.trogdorSurface.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    const button = event.target instanceof Element ? event.target.closest("button[data-action]") : null;
    if (button) {
      void handleTrogdorDomAction(button);
      return;
    }
    const agent = event.target instanceof Element ? event.target.closest("[data-trogdor-agent]") : null;
    const sessionId = agent?.dataset?.sessionId || "";
    if (sessionId) {
      void handleSurfaceAction({ type: "trogdor_agent", sessionId });
    }
  });

  el.trogdorSurface.addEventListener("mouseover", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const agent = target?.closest("[data-trogdor-agent]");
    if (agent?.dataset?.sessionId) {
      updateHoveredTrogdorSurface({ type: "trogdor_agent", sessionId: agent.dataset.sessionId });
      return;
    }
    const action = target?.closest("button[data-action]");
    if (action?.dataset?.action?.startsWith("trogdor_")) {
      updateHoveredTrogdorSurface({ type: "action", actionId: action.dataset.action });
    }
  });

  el.trogdorSurface.addEventListener("mouseleave", () => {
    updateHoveredTrogdorSurface(null);
  });

  el.trogdorSurface.addEventListener("focusin", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const agent = target?.closest("[data-trogdor-agent]");
    if (agent?.dataset?.sessionId) {
      updateHoveredTrogdorSurface({ type: "trogdor_agent", sessionId: agent.dataset.sessionId });
    }
  });

  el.trogdorSurface.addEventListener("focusout", (event) => {
    const next = event.relatedTarget instanceof Element ? event.relatedTarget : null;
    if (!next || !el.trogdorSurface.contains(next)) {
      updateHoveredTrogdorSurface(null);
    }
  });
}

function handleGlobalShortcut(event) {
  if ((event.ctrlKey || event.metaKey) && !event.altKey) {
    switch (event.code) {
      case "KeyK":
        openCommandPalette();
        return true;
      case "Equal":
      case "NumpadAdd":
        setTerminalZoom(state.terminalZoom + TERMINAL_ZOOM_STEP, { announce: true });
        return true;
      case "Minus":
      case "NumpadSubtract":
        setTerminalZoom(state.terminalZoom - TERMINAL_ZOOM_STEP, { announce: true });
        return true;
      case "Digit0":
      case "Numpad0":
        setTerminalZoom(1, { announce: true });
        return true;
      default:
        break;
    }
  }

  if (event.key === "Escape") {
    if (state.activeSheet) {
      closeSheets();
      return true;
    }
    if (state.trogdorAtlasOpen) {
      state.trogdorAtlasOpen = false;
      renderHudSurface();
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
    case "KeyL":
      if (state.hoveredLinkUrl) {
        void copyHoveredLink();
      }
      return true;
    case "KeyR":
      void refreshSessions();
      return true;
    default:
      return false;
  }
}

function bindEvents() {
  bindTrogdorEvents();
  document.addEventListener?.("keydown", (event) => {
    if ((event.ctrlKey || event.metaKey) && !event.altKey && event.code === "KeyK") {
      event.preventDefault();
      openCommandPalette();
    }
  });
  el.terminalPalette.addEventListener("click", () => {
    openCommandPalette();
  });
  el.terminalCopyFrame.addEventListener("click", () => {
    void copyTerminalFrameText();
  });
  el.terminalLinkOpen.addEventListener("click", () => {
    if (state.hoveredLinkUrl) {
      safeOpenUrl(state.hoveredLinkUrl);
    }
  });
  el.terminalLinkCopy.addEventListener("click", () => {
    void copyHoveredLink();
  });
  el.terminalZoomOut.addEventListener("click", () => {
    setTerminalZoom(state.terminalZoom - TERMINAL_ZOOM_STEP, { announce: true });
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalZoomReset.addEventListener("click", () => {
    setTerminalZoom(1, { announce: true });
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalZoomIn.addEventListener("click", () => {
    setTerminalZoom(state.terminalZoom + TERMINAL_ZOOM_STEP, { announce: true });
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalMobileKeyboard.addEventListener("click", () => {
    if (state.mobileKeyboardActive) {
      closeMobileKeyboard();
      focusTerminalInputSurface({ preventScroll: true });
      return;
    }
    focusMobileKeyboard();
  });
  el.terminalTrogdorBack.addEventListener("click", (event) => {
    event.preventDefault();
    openTrogdorAtlas();
  });
  el.terminalWorkbenchToggle.addEventListener("click", () => {
    setTerminalWorkbenchOpen(!state.terminalWorkbenchOpen);
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalWorkbenchRefresh.addEventListener("click", () => {
    void refreshAgentContextForSelectedSession({ force: true });
    void refreshWorkbenchWidgetsForSelectedSession({ force: true });
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalWorkbenchWidgets.addEventListener("click", (event) => {
    const turnButton = event.target?.closest?.("[data-workbench-turn-id]");
    if (turnButton) {
      event.preventDefault();
      state.workbenchSelectedTurnId = String(turnButton.dataset.workbenchTurnId || "");
      state.workbenchWidgets.transcript = null;
      state.workbenchWidgets.transcriptTurnId = "";
      state.workbenchWidgets.transcriptNextCursor = 0;
      renderWorkbenchWidgets();
      void refreshWorkbenchWidgetsForSelectedSession({ force: true, silent: true });
      focusTerminalInputSurface({ preventScroll: true });
      return;
    }

    const logModeButton = event.target?.closest?.("[data-workbench-log-mode]");
    if (logModeButton) {
      event.preventDefault();
      state.workbenchLogMode = logModeButton.dataset.workbenchLogMode === "raw" ? "raw" : "lens";
      renderWorkbenchWidgets();
      focusTerminalInputSurface({ preventScroll: true });
      return;
    }

    const mermaidButton = event.target?.closest?.("[data-workbench-open-mermaid]");
    if (!mermaidButton) {
      return;
    }
    event.preventDefault();
    openSheet("mermaid");
  });
  el.terminalWorkbenchWidgets.addEventListener("input", (event) => {
    const target = event.target;
    if (!target?.matches?.("[data-workbench-log-search]")) {
      return;
    }
    state.workbenchLogSearch = target.value || "";
    renderWorkbenchWidgets();
  });
  el.terminalWorkbenchWidgets.addEventListener("change", (event) => {
    const target = event.target;
    if (!target?.matches?.("[data-workbench-log-filter]")) {
      return;
    }
    state.workbenchLogFilter = WORKBENCH_LOG_FILTERS.includes(target.value) ? target.value : "all";
    renderWorkbenchWidgets();
  });
  el.terminalInputDock.addEventListener("submit", (event) => {
    event.preventDefault();
    void submitTerminalInputDock();
  });
  el.terminalInlineInput.addEventListener("input", () => {
    resizeTerminalInlineInput();
    syncTerminalInputDock();
  });
  el.terminalInlineInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submitTerminalInputDock();
    } else {
      const actionId = terminalKeyActionForDomEvent(event);
      if (actionId) {
        event.preventDefault();
        sendTerminalControlKey(actionId);
      }
    }
    event.stopPropagation();
  });
  el.terminalKeyStrip.addEventListener("click", (event) => {
    const button = event.target instanceof Element ? event.target.closest("button[data-terminal-key]") : null;
    if (!button || button.disabled) {
      return;
    }
    event.preventDefault();
    sendTerminalControlKey(button.dataset.terminalKey);
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalInlineInput.addEventListener("focus", () => {
    if (!state.activeSheet) {
      forwardTerminalEvent({ kind: "focus", focused: true });
    }
  });
  el.terminalFallback.addEventListener("mousedown", () => {
    if (state.terminalFallbackActive && !state.activeSheet) {
      requestAnimationFrame(() => {
        focusTerminalInputSurface({ preventScroll: true });
      });
    }
  });
  el.terminalFallback.addEventListener("click", () => {
    if (state.terminalFallbackActive && !state.activeSheet) {
      focusTerminalInputSurface({ preventScroll: true });
    }
  });
  el.terminalFallback.addEventListener("keydown", handleTerminalFallbackKeyEvent);
  el.terminalFallback.addEventListener("paste", handleTerminalFallbackPasteEvent);
  el.terminalFallback.addEventListener("focus", () => {
    if (state.terminalFallbackActive && !state.activeSheet) {
      forwardTerminalEvent({ kind: "focus", focused: true });
    }
  });
  el.terminalFallback.addEventListener("blur", () => {
    if (document.activeElement === el.mobileKeyboardProxy) {
      return;
    }
    if (state.terminalFallbackActive) {
      forwardTerminalEvent({ kind: "focus", focused: false });
    }
  });
  el.terminalFallback.addEventListener("scroll", () => {
    if (state.terminalFallbackActive) {
      state.terminalFallbackAutoFollow = terminalFallbackIsNearBottom();
    }
  });
  el.mobileKeyboardProxy.addEventListener("focus", () => {
    state.mobileKeyboardActive = true;
    syncMobileKeyboardState();
    forwardTerminalEvent({ kind: "focus", focused: true });
  });
  el.mobileKeyboardProxy.addEventListener("blur", () => {
    state.mobileKeyboardActive = false;
    syncMobileKeyboardState();
    forwardTerminalEvent({ kind: "focus", focused: false });
  });
  el.mobileKeyboardProxy.addEventListener("keydown", (event) => {
    if (handleGlobalShortcut(event)) {
      event.preventDefault();
      return;
    }
    if (state.readOnly || !currentSession()) {
      return;
    }
    const specialKeys = new Set([
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
    if (!specialKeys.has(event.key)) {
      return;
    }
    event.preventDefault();
    if (event.key === "Escape") {
      closeMobileKeyboard();
      focusTerminalInputSurface({ preventScroll: true });
      return;
    }
    if (keyBeginsTrogdorResponse(event)) {
      markTrogdorSessionsResponded([state.selectedSessionId]);
    }
    forwardTerminalKeyDown(event);
  });
  el.mobileKeyboardProxy.addEventListener("input", (event) => {
    if (state.readOnly || !currentSession()) {
      el.mobileKeyboardProxy.value = "";
      return;
    }
    const inputType = String(event.inputType || "");
    const inserted = typeof event.data === "string" ? event.data : el.mobileKeyboardProxy.value;
    el.mobileKeyboardProxy.value = "";
    if (inputType === "deleteContentBackward") {
      forwardTerminalEvent({
        kind: "key",
        phase: "down",
        key: "Backspace",
        code: "Backspace",
        mods: 0,
        repeat: false,
      });
      return;
    }
    if (inputType === "insertLineBreak") {
      forwardTerminalEvent({
        kind: "key",
        phase: "down",
        key: "Enter",
        code: "Enter",
        mods: 0,
        repeat: false,
      });
      return;
    }
    sendTerminalText(inserted);
  });
  el.modalBackdrop.addEventListener("click", closeSheets);
  el.modalRoot.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      closeSheets();
    }
  });
  el.paletteSearch.addEventListener("input", () => {
    state.paletteIndex = 0;
    renderCommandPalette();
  });
  el.paletteSearch.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      state.paletteIndex = Math.min(state.paletteItems.length - 1, state.paletteIndex + 1);
      renderCommandPalette();
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      state.paletteIndex = Math.max(0, state.paletteIndex - 1);
      renderCommandPalette();
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      void runCommandPaletteItem();
    }
  });
  el.paletteResults.addEventListener("mousemove", (event) => {
    const item = event.target instanceof Element ? event.target.closest("[data-palette-index]") : null;
    if (!item) {
      return;
    }
    state.paletteIndex = clampInt(Number(item.dataset.paletteIndex), 0, 0, Math.max(0, state.paletteItems.length - 1));
    renderCommandPalette();
  });
  el.paletteResults.addEventListener("click", (event) => {
    const item = event.target instanceof Element ? event.target.closest("[data-palette-index]") : null;
    if (!item) {
      return;
    }
    state.paletteIndex = clampInt(Number(item.dataset.paletteIndex), 0, 0, Math.max(0, state.paletteItems.length - 1));
    void runCommandPaletteItem();
  });
  el.paletteCloseButton.addEventListener("click", closeSheets);

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
  el.sendMode.addEventListener("change", updateSendHint);

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
    const text = el.sendInput.value;
    if (!text.trim()) {
      return;
    }
    try {
      rememberSendHistory(text);
      if (state.sendTarget?.type === "group") {
        const result = await sendGroupLine(state.sendTarget.sessionIds, text);
        const total = result?.total || state.sendTarget.sessionIds.length;
        const skipped = result?.skipped || 0;
        const delivered = result?.delivered || 0;
        setUtilityStatus(
          skipped > 0
            ? `Sent batch line to ${delivered} of ${total} agents.`
            : `Sent batch line to ${delivered} agents.`,
          delivered === 0,
          skipped > 0 ? 3200 : 2400,
        );
      } else {
        const targetSessionId = state.sendTarget?.sessionId || state.selectedSessionId;
        if (sendModeValue() === "paste") {
          await sendRawTextToSession(targetSessionId, text);
          setUtilityStatus(`Pasted text to ${state.sendTarget?.label || targetSessionId}.`, false, 2200);
        } else {
          await sendLineToSession(targetSessionId, text);
          setUtilityStatus(`Sent line to ${state.sendTarget?.label || targetSessionId}.`, false, 2200);
        }
      }
      el.sendInput.value = "";
      state.sendTarget = null;
      closeSheets();
      await refreshSessions();
    } catch (error) {
      setUtilityStatus(`Send failed: ${error.message}`, true, 3200);
      syncSheetActionAvailability();
    }
  });
  el.sendCloseButton.addEventListener("click", () => {
    state.sendTarget = null;
    closeSheets();
  });
  el.sendHistory.addEventListener("click", (event) => {
    const button = event.target instanceof Element ? event.target.closest("[data-send-history-index]") : null;
    if (!button) {
      return;
    }
    const index = Number(button.dataset.sendHistoryIndex);
    const text = state.sendHistory[index] || "";
    if (text) {
      el.sendInput.value = text;
      el.sendInput.focus();
    }
  });

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
  el.createTool.addEventListener("change", () => {
    syncSheetActionAvailability();
  });
  el.createLaunchTarget.addEventListener("change", () => {
    state.dirBrowser.launchTarget = selectedLaunchTarget();
    renderCreateBatchBar();
    syncSheetActionAvailability();
  });
  el.createRequest.addEventListener("input", () => {
    syncSheetActionAvailability();
  });
  el.dirsSearch.addEventListener("input", () => {
    state.dirBrowser.search = String(el.dirsSearch.value || "");
    renderDirEntries({
      path: state.dirBrowser.path,
      entries: state.dirBrowser.entries,
      groups: state.dirBrowser.groups,
      overlay_label: state.dirBrowser.overlayLabel || undefined,
      launch_targets: state.dirBrowser.launchTargets,
      default_launch_target: state.dirBrowser.launchTarget,
    });
  });
  el.createBatchVisible.addEventListener("click", () => {
    const paths = visibleSelectableDirPaths();
    const selected = ensureDirBrowserBatchSelection();
    selected.clear();
    for (const path of paths) {
      selected.add(path);
    }
    const firstPath = paths[0] || state.dirBrowser.path || el.dirsPath.value;
    if (firstPath) {
      el.createCwd.value = firstPath;
    }
    renderDirEntries({
      path: state.dirBrowser.path,
      entries: state.dirBrowser.entries,
      groups: state.dirBrowser.groups,
      overlay_label: state.dirBrowser.overlayLabel || undefined,
      launch_targets: state.dirBrowser.launchTargets,
      default_launch_target: state.dirBrowser.launchTarget,
    });
    setDirStatus(paths.length ? `Batching ${paths.length} visible directories.` : "No visible directories to batch.", paths.length < 1);
  });
  if (el.createBatchClear) {
    el.createBatchClear.addEventListener("click", () => {
      clearCreateBatchSelection();
      setDirStatus("Batch selection cleared.");
    });
  }
  el.createCwd.addEventListener("input", () => {
    el.dirsPath.value = el.createCwd.value;
    syncSheetActionAvailability();
  });
  el.dirsManagedOnly.addEventListener("change", () => {
    state.dirBrowser.managedOnly = Boolean(el.dirsManagedOnly.checked);
    localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(state.dirBrowser.managedOnly));
    syncSheetActionAvailability();
    void loadDirListing(el.dirsPath.value, state.dirBrowser.managedOnly);
  });
  el.dirsPath.addEventListener("input", () => {
    syncSheetActionAvailability();
  });
  el.dirsPath.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      state.dirBrowser.group = "";
      clearCreateBatchSelection();
      void loadDirListing(el.dirsPath.value, el.dirsManagedOnly.checked, "");
    }
  });
  el.dirsLoadButton.addEventListener("click", async () => {
    state.dirBrowser.group = "";
    clearCreateBatchSelection();
    await loadDirListing(el.dirsPath.value, el.dirsManagedOnly.checked, "");
  });
  el.dirsSpawnHere.addEventListener("click", async () => {
    if (state.readOnly) {
      return;
    }
    const path = String(state.dirBrowser.path || el.dirsPath.value || el.createCwd.value || "").trim();
    if (!path) {
      return;
    }
    clearCreateBatchSelection();
    el.createCwd.value = path;
    el.dirsPath.value = path;
    try {
      await createSessionFromSheet();
    } catch (error) {
      setDirStatus(`Failed to spawn here: ${error.message}`, true);
      syncSheetActionAvailability();
    }
  });
  el.dirsUpButton.addEventListener("click", async () => {
    const parent = parentDir(el.dirsPath.value);
    if (parent) {
      state.dirBrowser.group = "";
      clearCreateBatchSelection();
      el.dirsPath.value = parent;
      el.createCwd.value = parent;
      await loadDirListing(parent, el.dirsManagedOnly.checked, "");
    }
  });
  el.dirsList.addEventListener("change", (event) => {
    const checkbox = event.target instanceof Element ? event.target.closest(".dir-row-check") : null;
    if (!checkbox) {
      return;
    }
    const path = String(checkbox.dataset.path || "").trim();
    if (!path) {
      checkbox.checked = false;
      return;
    }
    const selected = ensureDirBrowserBatchSelection();
    if (checkbox.checked) {
      selected.add(path);
      el.createCwd.value = path;
    } else {
      selected.delete(path);
    }
    syncSheetActionAvailability();
  });
  el.dirsList.addEventListener("click", async (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target) {
      return;
    }
    if (target.closest(".dir-open-url")) {
      return;
    }

    const groupButton = target.closest(".dir-group-chip");
    if (groupButton) {
      const filter = String(groupButton.dataset.filter || "group");
      const groupName = String(groupButton.dataset.group || "").trim();
      const managedOnly = filter === "managed" ? true : filter === "all" ? false : el.dirsManagedOnly.checked;
      state.dirBrowser.group = filter === "group" ? groupName : "";
      state.dirBrowser.managedOnly = managedOnly;
      el.dirsManagedOnly.checked = managedOnly;
      localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(managedOnly));
      clearCreateBatchSelection();
      await loadDirListing(
        state.dirBrowser.path || el.dirsPath.value,
        managedOnly,
        state.dirBrowser.group,
      );
      return;
    }

    const groupActionButton = target.closest(".dir-entry-group-action");
    if (groupActionButton) {
      await updateDirEntryGroupMembership(
        groupActionButton.dataset.path,
        groupActionButton.dataset.action,
        groupActionButton.dataset.group,
        groupActionButton.dataset.removeGroup,
      );
      return;
    }

    const rowButton = target.closest(".dir-row-main");
    if (!rowButton) {
      return;
    }
    const path = String(rowButton.dataset.path || "").trim();
    if (!path) {
      return;
    }
    el.dirsPath.value = path;
    el.createCwd.value = path;
    if (rowButton.dataset.hasChildren === "true") {
      state.dirBrowser.group = "";
      clearCreateBatchSelection();
      await loadDirListing(path, el.dirsManagedOnly.checked, "");
      return;
    }
    setDirStatus(`Selected ${path}`);
    syncSheetActionAvailability();
  });

  el.mermaidRefreshButton.addEventListener("click", async () => {
    await refreshMermaidArtifact();
  });
  el.mermaidOpenButton.addEventListener("click", async () => {
    await openMermaidArtifactHost();
  });
  el.mermaidPlanTabs.addEventListener("click", async (event) => {
    const button = event.target instanceof Element ? event.target.closest("button[data-plan-file]") : null;
    if (!button) {
      return;
    }
    await loadMermaidPlanFile(button.dataset.planFile);
  });
  el.mermaidCloseButton.addEventListener("click", closeSheets);

  el.terminalStage.addEventListener(
    "mousedown",
    (event) => {
      captureSurfaceAction(event, "down");
    },
    { capture: true },
  );
  el.terminalStage.addEventListener(
    "click",
    (event) => {
      captureSurfaceAction(event, "click");
    },
    { capture: true },
  );
  el.terminalStage.addEventListener(
    "touchend",
    (event) => {
      captureSurfaceAction(event, "touch");
    },
    { capture: true, passive: false },
  );
  el.terminalStage.addEventListener(
    "wheel",
    (event) => {
      captureSurfaceAction(event, "wheel");
    },
    { capture: true, passive: false },
  );

  el.terminalStage.addEventListener("click", (event) => {
    if (terminalFallbackOwnsPointer(event)) {
      if (!state.activeSheet) {
        focusTerminalInputSurface({ preventScroll: true });
      }
      return;
    }
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
      focusTerminalInputSurface({ preventScroll: true });
    }
  });

  el.terminalStage.addEventListener(
    "touchend",
    (event) => {
      if (terminalFallbackOwnsPointer(event)) {
        return;
      }
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
        if (!isCoarsePointer() || !focusMobileKeyboard()) {
          focusTerminalInputSurface({ preventScroll: true });
        }
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
    if (keyBeginsTrogdorResponse(event)) {
      markTrogdorSessionsResponded([state.selectedSessionId]);
    }
    forwardTerminalKeyDown(event);
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
    sendTerminalText(text);
  });

  el.terminalStage.addEventListener("focus", () => {
    if (!state.activeSheet) {
      forwardTerminalEvent({ kind: "focus", focused: true });
    }
  });

  el.terminalStage.addEventListener("blur", () => {
    if (document.activeElement === el.mobileKeyboardProxy) {
      return;
    }
    forwardTerminalEvent({ kind: "focus", focused: false });
  });

  el.terminalStage.addEventListener("mousedown", (event) => {
    if (terminalFallbackOwnsPointer(event)) {
      return;
    }
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

    forwardTerminalMouse("down", clampInt(event.button, 0, 0, 2), hit, event);
  });

  el.terminalStage.addEventListener("mouseup", (event) => {
    if (terminalFallbackOwnsPointer(event)) {
      return;
    }
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

    forwardTerminalMouse("up", clampInt(event.button, 0, 0, 2), hit, event);
  });

  el.terminalStage.addEventListener("mousemove", (event) => {
    if (terminalFallbackOwnsPointer(event)) {
      return;
    }
    const hit = surfaceHit(event);
    updateHoveredTrogdorSurface(hit.action);
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

    forwardTerminalMouse("move", 0, hit, event);
  });

  el.terminalStage.addEventListener(
    "wheel",
    (event) => {
      if (terminalFallbackOwnsPointer(event)) {
        return;
      }
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
    updateHoveredTrogdorSurface(null);
  });

  const resizeObserver = new ResizeObserver(() => {
    measureAndResizeSurface(true, false);
  });
  resizeObserver.observe(el.terminalStage);
}

async function init() {
  loadInitialState();
  bindEvents();
  setUtilityStatus(defaultUtilityLabel(), true);
  syncWriteAccess();
  setLoadingState(boot.franken_term_available, boot.franken_term_available ? "Loading rendered control surface..." : "Snapshot fallback mode");
  void warmDirBrowserOnStartup();
  await setupHudSurface();
  renderHudSurface();
  await refreshSessions();
  scheduleSessionRefresh();
  if (boot.franken_term_available) {
    setLoadingState(false);
  }
}

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

export const __swimmersWebTest = {
  state,
  el,
  closeTrogdorAtlasForTerminal,
  openTrogdorAtlas,
  persistSelectedSession,
  renderHudSurface,
  syncTerminalPresentation,
  sessionSocketUrl,
  terminalPayloadFromSocketBytes,
  decodeTerminalOutputFrame,
  sessionRefreshDelayMs,
  sessionEventStreamOpen,
  feedTerminalBytes,
  flushPendingTerminalBytes,
  setTerminalTextFallbackActive,
  updateTerminalFallbackText,
  terminalFallbackOwnsPointer,
  sendTerminalText,
  sendTerminalInputText,
  utf8ByteLength,
  terminalTextWithinPasteBudget,
  frankenTermLinkPolicy,
  isLoopbackHostname,
  validateFrankenTermSurface,
  captureTerminalRendererDiagnostic,
  sendRawTextToSession,
  sendGroupLine,
  markTrogdorSessionsResponded,
  handleTerminalFallbackKeyEvent,
  handleTerminalFallbackPasteEvent,
  sendTerminalControlKey,
  terminalKeyActionForDomEvent,
  focusTerminalInputSurface,
  syncTerminalInputDock,
  submitTerminalInputDock,
  handleSocketText,
  syncTerminalWorkbench,
  renderTerminalWorkbench,
  renderWorkbenchWidgets,
  writeWorkbenchWidgetsHtml,
  renderTranscriptBlocks,
  renderMermaidArtifact,
  loadMermaidPlanFile,
  isSafeMermaidPlanFileName,
  sanitizeMermaidPlanFiles,
  refreshAgentContextForSelectedSession,
  refreshWorkbenchWidgetsForSelectedSession,
  setTerminalWorkbenchOpen,
  openCommandPalette,
  renderCommandPalette,
  runCommandPaletteItem,
  rememberSendHistory,
  renderSendHistory,
  syncTerminalAccessibilityMirror,
  syncTerminalStatusStrip,
  backendHealthWarningText,
  applyBackendHealth,
  copyTerminalFrameText,
  syncLinkTools,
};

if (!window.__SWIMMERS_DISABLE_AUTO_INIT__) {
  init().catch((error) => {
    console.error("[swimmers-web] failed to initialize", error);
    setConnectionStatus("init failed", true);
    setLoadingState(false);
  });
}
