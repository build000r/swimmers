import { buildSurfaceFrame, surfaceActionAt, surfaceConsumesPointer } from "./rendered_surface.js";
import {
  authTokenButtonPlan, controlEventSessionPatchPlan, eventCell, globalShortcutPlan, initialStateBootPlan, inputAckActionPlan, lifecycleDeletedSessionPatchPlan, mobileKeyboardInputExecutorPlan, mobileKeyboardInputPlan,
  sheetActionAvailabilityPlan,
  mobileKeyboardKeydownPlan, mobileKeyboardKeyPlan, shouldIgnoreSyntheticClick,
  terminalComposerControlAction, terminalDestroyStatePatch, terminalFallbackActivationPlan, terminalFallbackFocusPlan, terminalFallbackKeydownPlan, terminalFallbackPastePlan, terminalFallbackPointerFocusPlan, terminalInlineInputKeydownPlan, terminalKeyStripClickExecutorPlan, terminalKeyStripClickPlan, terminalStageCaptureBindings, terminalStageClickPlan, terminalStageFocusExecutorPlan, terminalStageFocusPlan,
  normalizeTerminalZoomValue, terminalAuxiliaryControlsPlan, terminalFallbackScrollPlan, terminalFallbackTextScrollPlan, terminalInputDockPlan, terminalLiveFrameFallbackPlan, terminalPaintProbeSchedulePlan, terminalPaintVerificationPlan, terminalPendingByteBufferPlan, terminalPresentationPlan, terminalResizeGeometryPlan, terminalStageKeydownPlan, terminalStagePasteExecutorPlan, terminalStagePastePlan, terminalStageTouchEndPlan, terminalToolsAvailabilityPlan, terminalZoomControlsPlan, terminalZoomLoadValue, terminalZoomPercentLabel, terminalZoomPersistencePlan,
} from "./input_support.js";
import { sendHistoryClickPlan, sendSheetFailureStatus, sendSheetSubmitPlan, sendSheetSuccessStatus } from "./send_sheet.js";
import {
  MERMAID_PLAN_CONTENT_DISPLAY_MAX_CHARS,
  buildMermaidArtifactView,
  boundedArtifactText,
  isSafeMermaidPlanFileName,
  mermaidPlanTabClickPlan,
  planFileLabel,
  sanitizeMermaidPlanFiles,
} from "./mermaid_artifact.js";
import {
  MAX_TERMINAL_PASTE_BYTES,
  frankenTermLinkPolicy,
  isLoopbackHostname,
  safeAnchorHref,
  terminalTextWithinPasteBudget,
  utf8ByteLength,
} from "./terminal_safety.js";
import {
  buildSessionSocketUrl,
  decodeTerminalOutputFrame,
  fallbackTextForKeyEvent,
  keyModifiers,
  sessionSocketAuthMessageForToken,
  terminalControlKeyEvent,
} from "./terminal_protocol.js";
import {
  clearCreateBatchSelection as clearDirBrowserBatchSelection,
  dirCheckboxChangePlan as dirBrowserCheckboxChangePlan,
  dirGroupChipClickPlan as dirBrowserGroupChipClickPlan,
  dirGroupMembershipClickPlan as dirBrowserGroupMembershipClickPlan,
  dirRowClickPlan as dirBrowserRowClickPlan,
  launchTargetPayload as dirBrowserLaunchTargetPayload,
  renderCreateBatchBar as renderDirBrowserCreateBatchBar,
  renderDirEntries as renderDirBrowserEntries,
  selectedLaunchTarget as dirBrowserSelectedLaunchTarget,
  visibleDirBatchPlan as dirBrowserVisibleDirBatchPlan,
  visibleSelectableDirPaths as dirBrowserVisibleSelectableDirPaths,
} from "./dir_browser.js";
import {
  commandPaletteExecutionPlan,
  commandPaletteResultEventPlan,
  commandPaletteSearchKeyPlan,
  filteredCommandPaletteItemsForState,
  renderCommandPaletteResultsHtml,
} from "./command_palette.js";
import {
  TROGDOR_DRAGON_TARGET,
  buildTrogdorDomGroups,
  loadTrogdorReadProgress,
  markTrogdorBurntSessionsInMap,
  markTrogdorSessionsRespondedState,
  normalizeTrogdorSessionId,
  pruneTrogdorBurntSessionMap,
  rawTrogdorSessionAwaitingUser,
  saveTrogdorReadProgress,
  setTrogdorClawgReadIndexForProgress,
  startTrogdorReaderStateForSession,
  summarizeTrogdorDom,
  trogdorClawgKey,
  trogdorClawgDismissedForMap,
  trogdorClawgReadCompleteForProgress,
  trogdorDomActionCueKinds,
  trogdorDragonPose as buildTrogdorDragonPose,
  trogdorHasActionCue,
  trogdorPrimaryActionCue,
  trogdorActionPayloadForZone,
  trogdorAtlasTransitionState,
  trogdorCueTransitionState,
  trogdorCurrentSurfaceSessionForHover,
  trogdorDomActionZoneForDataset,
  trogdorHoverReaderResetState,
  trogdorHoverSessionIdForZone,
  trogdorReadableHoveredSurfaceSession,
  trogdorReaderDisplayState,
  trogdorReaderProgressAdvanceForSession,
  trogdorReaderStateForWpmChange,
  trogdorReaderTimerAction,
  trogdorReaderToggleAction,
  trogdorReaderWpmForAction,
  trogdorReaderWordIndexForProgress,
  trogdorRawSessionForHover,
  trogdorSessionCanReadForState,
  trogdorSessionBurntInMap,
  trogdorSessionAwaitingUser,
  trogdorSurfaceSessionTrogdorState,
  trogdorSwordsmanVisibleForState,
  trogdorTerminalFocusStatus,
} from "./trogdor_logic.js";
import {
  TROGDOR_REPO_POSITIONS,
  trogdorReadButtonLabel,
  renderTrogdorSurfaceFrame,
  trogdorSurfaceSignature,
} from "./trogdor_render.js";
import {
  agentActionLabel,
  applyWorkbenchWidgetResults,
  buildWorkbenchWidgetRequestPlan,
  buildWorkbenchWidgetsHtml,
  emptyWorkbenchWidgets,
  operatorPressureSummary,
  renderTerminalWorkbenchActions,
  renderTranscriptBlocks,
  resetWorkbenchWidgetsState,
  selectedWorkbenchWidgetsSnapshot,
  shouldThrottleWorkbenchWidgets,
  truncateWorkbenchText,
  workbenchWidgetClickPlan,
  workbenchWidgetLogPlan,
} from "./workbench_render.js";

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
const TROGDOR_BURN_MS = 1100;
const TERMINAL_ZOOM_MIN = 0.65;
const TERMINAL_ZOOM_MAX = 2.4;
const TERMINAL_ZOOM_STEP = 0.1;
const SEND_HISTORY_LIMIT = 8;
const MAX_PENDING_TERMINAL_BYTES = 524288;
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
  workbenchWidgets: emptyWorkbenchWidgets(),
  workbenchLogMode: "lens",
  workbenchLogFilter: "all",
  workbenchLogSearch: "",
  workbenchSelectedTurnId: "",
  refreshTimer: null,
  snapshotTimer: null,
  terminalPaintProbeTimer: null,
  renderQueued: false,
  renderRetryQueued: false,
  surfaceInitInProgress: 0,
  surfaceOperationDepth: 0,
  hudRenderQueued: false,
  resizeQueued: false,
  resizePushResize: false,
  resizeForce: false,
  resizeRetryTimer: null,
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
    svgUrl: "",
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
  dirsGroups: document.getElementById("dirs-groups"),
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
  return normalizeTrogdorSessionId(sessionId);
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
  return normalizeTerminalZoomValue(value, { minZoom: TERMINAL_ZOOM_MIN, maxZoom: TERMINAL_ZOOM_MAX, step: TERMINAL_ZOOM_STEP });
}

function loadTerminalZoom(url) {
  return terminalZoomLoadValue({ urlZoom: url.searchParams.get("zoom"), storedZoom: localStorage.getItem(TERMINAL_ZOOM_STORAGE_KEY) }, { minZoom: TERMINAL_ZOOM_MIN, maxZoom: TERMINAL_ZOOM_MAX, step: TERMINAL_ZOOM_STEP });
}

function syncTerminalZoomControls() {
  if (!el.terminalControlStrip) {
    return;
  }
  const plan = terminalZoomControlsPlan({ zoomSupported: terminalZoomSupported(), hasTerminal: Boolean(state.terminal), zoom: state.terminalZoom, minZoom: TERMINAL_ZOOM_MIN, maxZoom: TERMINAL_ZOOM_MAX });
  el.terminalZoomOut.disabled = plan.zoomOutDisabled;
  el.terminalZoomIn.disabled = plan.zoomInDisabled;
  el.terminalZoomReset.disabled = plan.zoomResetDisabled;
  el.terminalZoomReset.textContent = plan.zoomResetLabel;
  const auxiliaryPlan = terminalAuxiliaryControlsPlan({ hasCurrentSession: Boolean(currentSession()), readOnly: state.readOnly, mobileKeyboardActive: state.mobileKeyboardActive, hasCopyFrame: Boolean(el.terminalCopyFrame) });
  el.terminalMobileKeyboard.disabled = auxiliaryPlan.mobileKeyboardDisabled;
  el.terminalMobileKeyboard.setAttribute("aria-pressed", auxiliaryPlan.mobileKeyboardAriaPressed);
  syncTerminalInputDock();
  if (auxiliaryPlan.copyFrameAvailable) el.terminalCopyFrame.disabled = auxiliaryPlan.copyFrameDisabled;
}

function syncTerminalInputDock() {
  if (!el.terminalInputDock) {
    return;
  }
  const plan = terminalInputDockPlan({ hasCurrentSession: Boolean(currentSession()), trogdorAtlasOpen: state.trogdorAtlasOpen, readOnly: state.readOnly, inputValue: el.terminalInlineInput.value });
  document.body.classList.toggle("terminal-input-dock-visible", plan.visible);
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

function resetAgentContextForSession(sessionId) {
  state.agentContextSessionId = normalizeSessionId(sessionId);
  state.agentContextLoading = false;
  state.agentContextPayload = null;
  state.agentContextError = "";
  state.agentContextLastLoadedAt = 0;
  renderTerminalWorkbench();
}

function resetWorkbenchWidgetsForSession(sessionId) {
  resetWorkbenchWidgetsState(state.workbenchWidgets, normalizeSessionId(sessionId));
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

  el.terminalWorkbenchActions.innerHTML = renderTerminalWorkbenchActions(actions, Boolean(payload?.available));
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
  return selectedWorkbenchWidgetsSnapshot(state.workbenchWidgets, state.selectedSessionId);
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

  const contextPayload = selectedAgentContextPayload();
  writeWorkbenchWidgetsHtml(buildWorkbenchWidgetsHtml({
    widgets,
    contextPayload,
    selectedTurnId: state.workbenchSelectedTurnId,
    logState: {
      mode: state.workbenchLogMode,
      filter: state.workbenchLogFilter,
      query: state.workbenchLogSearch,
    },
  }));
}

async function refreshWorkbenchWidgetsForSelectedSession(options = {}) {
  const session = currentSession();
  if (!session || state.trogdorAtlasOpen) {
    state.workbenchWidgets.loading = false;
    renderWorkbenchWidgets();
    return;
  }

  const sessionId = session.session_id;
  if (shouldThrottleWorkbenchWidgets({
    options,
    widgets: state.workbenchWidgets,
    sessionId,
    throttleMs: AGENT_CONTEXT_REFRESH_MS,
  })) {
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

  const requestPlan = buildWorkbenchWidgetRequestPlan({
    sessionId,
    selectedTurnId: state.workbenchSelectedTurnId,
    widgets: state.workbenchWidgets,
    force: Boolean(options.force),
  });
  const { paths } = requestPlan;
  const [timelineResult, skillsResult, tailResult, transcriptResult, artifactResult, diffResult] = await Promise.allSettled([
    apiMaybeFetch(paths.timeline).then(responseJsonOrNull),
    apiMaybeFetch(paths.skills).then(responseJsonOrNull),
    apiMaybeFetch(paths.paneTail).then(responseJsonOrNull),
    apiMaybeFetch(paths.transcript).then(responseJsonOrNull),
    apiMaybeFetch(paths.artifact).then(responseJsonOrNull),
    apiMaybeFetch(paths.gitDiff).then(responseJsonOrNull),
  ]);

  if (requestSeq !== state.workbenchWidgets.requestSeq || state.selectedSessionId !== sessionId) {
    return;
  }

  const applied = applyWorkbenchWidgetResults(
    state.workbenchWidgets,
    { timelineResult, skillsResult, tailResult, transcriptResult, artifactResult, diffResult },
    {
      canDeltaTranscript: requestPlan.canDeltaTranscript,
      requestedTurnId: requestPlan.requestedTurnId,
      selectedTurnId: state.workbenchSelectedTurnId,
    },
  );
  state.workbenchSelectedTurnId = applied.selectedTurnId;
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

function persistTerminalZoomToUrl(plan) {
  const url = new URL(window.location.href);
  if (plan.urlParamAction === "delete") url.searchParams.delete("zoom");
  else url.searchParams.set("zoom", plan.urlParamValue);
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
    const persistencePlan = terminalZoomPersistencePlan(state.terminalZoom);
    localStorage.setItem(TERMINAL_ZOOM_STORAGE_KEY, persistencePlan.storageValue);
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

function syncTerminalTools() {
  const searchReady = terminalSupports("setSearchQuery");
  const selectionReady = terminalSupports("copySelection") || terminalSupports("extractSelectionText");
  const liveTerminal = hasLiveTerminal();
  const plan = terminalToolsAvailabilityPlan({ searchReady, liveTerminal, frankenTermAvailable: boot.franken_term_available, searchQuery: state.searchQuery, readOnly: state.readOnly, sendTargetType: state.sendTarget?.type, hasCurrentSession: Boolean(currentSession()) });

  el.terminalSearch.disabled = plan.searchDisabled;
  el.searchPrevButton.disabled = plan.searchDisabled;
  el.searchNextButton.disabled = plan.searchDisabled;
  el.searchClearButton.disabled = plan.searchDisabled;
  el.sendInput.disabled = plan.sendInputDisabled;
  if (el.sendMode) {
    el.sendMode.disabled = plan.sendModeDisabled;
  }
  el.sendSubmitButton.disabled = plan.sendSubmitDisabled;
  Array.from(el.createForm.elements).forEach((element) => {
    element.disabled = plan.createFormElementsDisabled;
  });

  el.terminalStage.classList.toggle("select-mode", state.selectMode);
  el.terminalStage.classList.toggle("link-hot", Boolean(state.hoveredLinkUrl) && !state.selectMode);
  syncTerminalZoomControls();
  if ((state.readOnly || !currentSession()) && state.mobileKeyboardActive) {
    closeMobileKeyboard();
  }
  syncLinkTools();
  syncTerminalStatusStrip();

  if (plan.searchStatus) setSearchStatus(plan.searchStatus.label, plan.searchStatus.muted);
}

function syncSheetActionAvailability() {
  const writeDisabled = Boolean(state.readOnly);
  const batchCount = state.dirBrowser.batchSelected instanceof Set ? state.dirBrowser.batchSelected.size : 0;
  const batchReady = batchCount > 0;
  const dirsPath = el.dirsPath.value.trim();
  const plan = sheetActionAvailabilityPlan({
    writeDisabled, hasSession: Boolean(currentSession()), batchReady,
    hasSinglePath: Boolean(el.createCwd.value.trim()), visibleSelectableCount: visibleSelectableDirPaths().length,
    hasBrowserPath: Boolean((state.dirBrowser.path || dirsPath || "").trim()),
    hasThoughtConfig: Boolean(state.thoughtConfig.config), hasNativeStatus: Boolean(state.nativeDesktop.status),
    nativeSupported: Boolean(state.nativeDesktop.status?.supported), hasMermaidPath: Boolean(state.mermaidArtifact.artifact?.path),
    hasDirsPath: Boolean(dirsPath), hasParentDir: Boolean(parentDir(dirsPath)),
    sendTargetType: state.sendTarget?.type, sendTargetReady: sendTargetReady(),
  });
  const setDisabled = (control, disabled) => { control.disabled = disabled; };
  const setOptionalDisabled = (control, disabled) => { if (control) control.disabled = disabled; };

  setDisabled(el.createButton, plan.createButtonDisabled);
  setOptionalDisabled(el.createBatchSubmit, plan.createBatchSubmitDisabled);
  setOptionalDisabled(el.createBatchVisible, plan.createBatchVisibleDisabled);
  setOptionalDisabled(el.dirsSpawnHere, plan.dirsSpawnHereDisabled);
  setDisabled(el.thoughtConfigTestButton, plan.thoughtConfigTestDisabled);
  setDisabled(el.thoughtConfigSaveButton, plan.thoughtConfigSaveDisabled);
  setDisabled(el.nativeSaveButton, plan.nativeSaveDisabled);
  setDisabled(el.nativeOpenButton, plan.nativeOpenDisabled);
  setDisabled(el.nativeRefreshButton, plan.nativeRefreshDisabled);
  setDisabled(el.mermaidOpenButton, plan.mermaidOpenDisabled);
  setDisabled(el.mermaidRefreshButton, plan.mermaidRefreshDisabled);
  setDisabled(el.dirsLoadButton, plan.dirsLoadDisabled);
  setDisabled(el.dirsUpButton, plan.dirsUpDisabled);
  setOptionalDisabled(el.sendMode, plan.sendModeDisabled);
  setDisabled(el.sendSubmitButton, plan.sendSubmitDisabled);
  updateSendHint();
  renderCreateBatchBar();
}

function loadInitialState() {
  const url = new URL(window.location.href);
  const plan = initialStateBootPlan({
    searchParams: url.searchParams, storedToken: localStorage.getItem(TOKEN_STORAGE_KEY) ?? "",
    selectedFromStorage: localStorage.getItem(SESSION_STORAGE_KEY),
    rawStoredDirPath: localStorage.getItem(DIR_BROWSER_PATH_KEY) ?? "",
    rawStoredManagedOnly: localStorage.getItem(DIR_BROWSER_MANAGED_ONLY_KEY),
    bootFollowPublishedSelection: boot.follow_published_selection,
    terminalWorkbenchMobile: window.matchMedia?.("(max-width: 700px)")?.matches ?? false,
  });
  if (plan.clearStoredDirPath) {
    localStorage.removeItem(DIR_BROWSER_PATH_KEY);
  }

  state.terminalZoom = loadTerminalZoom(url);
  state.terminalWorkbenchOpen = plan.terminalWorkbenchOpen;
  state.trogdorReadProgress = loadTrogdorReadProgress();
  loadSendHistory();
  persistToken(plan.tokenToPersist);
  setFollowPublishedSelection(plan.followPublishedSelection, { skipUrlSync: true });
  state.dirBrowser.path = plan.storedDirPath;
  state.dirBrowser.managedOnly = plan.storedManagedOnly;
  el.dirsPath.value = plan.storedDirPath;
  el.dirsManagedOnly.checked = plan.storedManagedOnly;
  el.createCwd.value = plan.storedDirPath;
  persistSelectedSession(plan.selectedSessionId, { syncUrl: false });
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

function rawSessionAwaitingUser(session) {
  return rawTrogdorSessionAwaitingUser(session, operatorPressureSnapshot(session?.session_id));
}

function setTrogdorClawgReadIndex(session, index) {
  const next = setTrogdorClawgReadIndexForProgress(
    state.trogdorReadProgress || {},
    session,
    index,
  );
  if (!next.changed) {
    return false;
  }
  state.trogdorReadProgress = next.progress;
  saveTrogdorReadProgress(state.trogdorReadProgress);
  return true;
}

function trogdorClawgReadComplete(session) {
  return trogdorClawgReadCompleteForProgress(session, state.trogdorReadProgress);
}

function trogdorClawgDismissed(session) {
  return trogdorClawgDismissedForMap(session, state.trogdorDismissedClawgs);
}

function trogdorSessionBurnt(sessionOrId) {
  const next = trogdorSessionBurntInMap(
    state.trogdorBurntSessions,
    sessionOrId,
    performance.now(),
  );
  state.trogdorBurntSessions = next.burntSessions;
  return next.burnt;
}

function pruneTrogdorBurntSessions() {
  const next = pruneTrogdorBurntSessionMap(state.trogdorBurntSessions, performance.now());
  state.trogdorBurntSessions = next.burntSessions;
  return next.changed;
}

function markTrogdorSessionsBurnt(sessionIds, options = {}) {
  const next = markTrogdorBurntSessionsInMap(
    state.trogdorBurntSessions,
    sessionIds,
    performance.now(),
    TROGDOR_BURN_MS,
  );
  if (!next.ids.length) {
    return;
  }
  state.trogdorBurntSessions = next.burntSessions;
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
  return trogdorCurrentSurfaceSessionForHover({
    sessions: state.sessions,
    hoveredSessionId: state.hoveredTrogdorSessionId,
    toSurfaceSession: surfaceSession,
  });
}

function trogdorSwordsmanVisible(session) {
  const burnt = typeof session?.trogdorBurnt === "boolean" ? session.trogdorBurnt : trogdorSessionBurnt(session);
  const dismissed = typeof session?.trogdorDismissed === "boolean" ? session.trogdorDismissed : trogdorClawgDismissed(session);
  return trogdorSwordsmanVisibleForState(session, { burnt, dismissed });
}

function trogdorSessionCanRead(session) {
  const burnt = typeof session?.trogdorBurnt === "boolean" ? session.trogdorBurnt : trogdorSessionBurnt(session);
  const dismissed = typeof session?.trogdorDismissed === "boolean" ? session.trogdorDismissed : trogdorClawgDismissed(session);
  return trogdorSessionCanReadForState(session, { burnt, dismissed });
}

function trogdorReaderWordIndex(session, wpm) {
  return trogdorReaderWordIndexForProgress(session, {
    wpm,
    readerClawgKey: state.trogdorReaderClawgKey,
    readerStartIndex: state.trogdorReaderStartIndex,
    progress: state.trogdorReadProgress,
    reading: state.trogdorReading,
    hoveredSessionId: state.hoveredTrogdorSessionId,
    readerStartedAt: state.trogdorReaderStartedAt,
    now: performance.now(),
  });
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
  const next = trogdorReaderProgressAdvanceForSession(session, {
    wordIndex: trogdorReaderWordIndex(session, state.trogdorWpm),
    reading: state.trogdorReading,
  });
  if (!next.shouldAdvance) {
    return;
  }
  setTrogdorClawgReadIndex(session, next.nextReadIndex);
  state.trogdorReading = next.reading;
}

function resetTrogdorReaderAfterWpmChange() {
  Object.assign(state, trogdorReaderStateForWpmChange(currentTrogdorSurfaceSession(), {
    currentStartIndex: state.trogdorReaderStartIndex,
    progress: state.trogdorReadProgress,
    now: performance.now(),
  }));
}

function startTrogdorReaderForSession(session, options = {}) {
  const next = startTrogdorReaderStateForSession(session, {
    readAgain: Boolean(options.readAgain),
    dismissedClawgs: state.trogdorDismissedClawgs || {},
    progress: state.trogdorReadProgress || {},
    now: performance.now(),
  });
  state.trogdorDismissedClawgs = next.dismissedClawgs;
  if (next.progressChanged) {
    state.trogdorReadProgress = next.progress;
    saveTrogdorReadProgress(state.trogdorReadProgress);
  }
  state.trogdorReaderClawgKey = next.readerClawgKey;
  state.trogdorReaderStartIndex = next.readerStartIndex;
  state.trogdorReaderStartedAt = next.readerStartedAt;
  state.trogdorReading = next.reading;
}

function markTrogdorSessionsResponded(sessionIds) {
  const next = markTrogdorSessionsRespondedState({
    sessionIds,
    sessions: state.sessions,
    toSurfaceSession: surfaceSession,
    dismissedClawgs: state.trogdorDismissedClawgs || {},
    progress: state.trogdorReadProgress || {},
    hoveredSessionId: state.hoveredTrogdorSessionId,
  });
  state.trogdorDismissedClawgs = next.dismissedClawgs;
  if (next.progressChanged) {
    state.trogdorReadProgress = next.progress;
    saveTrogdorReadProgress(state.trogdorReadProgress);
  }
  if (next.burntIds.length) {
    if (next.resetReader) {
      Object.assign(state, trogdorHoverReaderResetState());
      syncTrogdorReaderTimer();
    }
    markTrogdorSessionsBurnt(next.burntIds);
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
  Object.assign(surface, trogdorSurfaceSessionTrogdorState(surface, {
    burnt: trogdorSessionBurnt(surface),
    dismissedClawgs: state.trogdorDismissedClawgs,
    readProgress: state.trogdorReadProgress,
  }));
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
  const next = trogdorCueTransitionState({
    sessions: state.sessions,
    previousAwaitingSessionIds: state.trogdorAwaitingSessionIds,
    hoveredSessionId: state.hoveredTrogdorSessionId,
    rawAwaitingUser: rawSessionAwaitingUser,
    sessionBurnt: trogdorSessionBurnt,
  });
  state.trogdorAwaitingSessionIds = next.awaitingSessionIds;
  if (next.burntIds.length) {
    markTrogdorSessionsBurnt(next.burntIds, { render: false });
  }

  if (next.resetReader) {
    Object.assign(state, trogdorHoverReaderResetState());
    syncTrogdorReaderTimer();
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
  return state.dirBrowser.batchSelected instanceof Set
    ? state.dirBrowser.batchSelected
    : (state.dirBrowser.batchSelected = new Set());
}

function visibleSelectableDirPaths() {
  return dirBrowserVisibleSelectableDirPaths(state.dirBrowser);
}

function selectedLaunchTarget() {
  return dirBrowserSelectedLaunchTarget(el, state.dirBrowser);
}

function launchTargetPayload() {
  return dirBrowserLaunchTargetPayload(el, state.dirBrowser);
}

function renderCreateBatchBar() {
  renderDirBrowserCreateBatchBar({ el, dirBrowser: state.dirBrowser });
}

function currentDirListingPayload() {
  return {
    path: state.dirBrowser.path,
    entries: state.dirBrowser.entries,
    groups: state.dirBrowser.groups,
    overlay_label: state.dirBrowser.overlayLabel || undefined,
    launch_targets: state.dirBrowser.launchTargets,
    default_launch_target: state.dirBrowser.launchTarget,
  };
}

function clearCreateBatchSelection() {
  clearDirBrowserBatchSelection({
    el,
    dirBrowser: state.dirBrowser,
    syncSheetActionAvailability,
  });
}

function handleCreateBatchVisibleAction() {
  const plan = dirBrowserVisibleDirBatchPlan(visibleSelectableDirPaths(), state.dirBrowser.path, el.dirsPath.value);
  const selected = ensureDirBrowserBatchSelection();
  selected.clear();
  for (const path of plan.paths) selected.add(path);
  if (plan.firstPath) el.createCwd.value = plan.firstPath;
  renderDirEntries(currentDirListingPayload());
  setDirStatus(plan.statusLabel, plan.statusMuted);
}

function handleDirCheckboxChange(event) {
  const target = event.target instanceof Element ? event.target : null;
  const plan = dirBrowserCheckboxChangePlan(event.type, target);
  if (plan.type === "ignore") return false;
  if (plan.type === "reset_checkbox") {
    plan.checkbox.checked = false;
    return true;
  }
  const selected = ensureDirBrowserBatchSelection();
  (plan.type === "add" ? selected.add : selected.delete).call(selected, plan.path);
  if (plan.type === "add") el.createCwd.value = plan.path;
  syncSheetActionAvailability();
  return true;
}

async function handleDirGroupChipClick(event, target = event.target instanceof Element ? event.target : null) {
  const plan = dirBrowserGroupChipClickPlan(event.type, target, el.dirsManagedOnly.checked, state.dirBrowser.path, el.dirsPath.value);
  if (plan.type !== "filter") return false;
  state.dirBrowser.group = plan.group;
  state.dirBrowser.managedOnly = plan.managedOnly;
  el.dirsManagedOnly.checked = plan.managedOnly;
  localStorage.setItem(DIR_BROWSER_MANAGED_ONLY_KEY, String(plan.managedOnly));
  clearCreateBatchSelection();
  await loadDirListing(plan.path, plan.managedOnly, plan.group);
  return true;
}

function renderDirEntries(response) {
  renderDirBrowserEntries(response, {
    el,
    dirBrowser: state.dirBrowser,
    readOnly: state.readOnly,
    pathStorageKey: DIR_BROWSER_PATH_KEY,
    managedOnlyStorageKey: DIR_BROWSER_MANAGED_ONLY_KEY,
    setDirStatus,
    syncSheetActionAvailability,
  });
}

function renderMermaidArtifact(payload) {
  state.mermaidArtifact.artifact = payload;
  const view = buildMermaidArtifactView(payload, { formatTime });
  state.mermaidArtifact.source = view.source;
  const planFiles = view.planFiles;
  state.mermaidArtifact.planFiles = planFiles;
  state.mermaidArtifact.activePlanFile = "";
  state.mermaidArtifact.planContent = "";
  el.mermaidSource.textContent = view.source || "Mermaid source unavailable.";
  el.mermaidPreview.innerHTML = "";
  el.mermaidPlanContent.textContent = "";
  el.mermaidPlanContent.classList.add("hidden");
  el.mermaidPlanContent.classList.remove("error");

  if (view.available && state.mermaidArtifact.svgUrl) {
    const img = document.createElement("img");
    img.src = state.mermaidArtifact.svgUrl;
    img.alt = "Mermaid artifact preview";
    img.className = "mermaid-preview-image";
    el.mermaidPreview.appendChild(img);
  }

  renderMermaidPlanTabs();
  setMermaidStatus(view.status);
  syncSheetActionAvailability();
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
  const hovered = trogdorReadableHoveredSurfaceSession(sessions, state.hoveredTrogdorSessionId, {
    sessionCanRead: trogdorSessionCanRead,
  });
  const summary = summarizeTrogdorDom(groups, sessions);
  const dragonPose = buildTrogdorDragonPose(groups, summary, TROGDOR_REPO_POSITIONS);
  const signature = trogdorSurfaceSignature(sessions, summary, state.readOnly);
  if (signature !== state.trogdorSurfaceSignature) {
    state.trogdorSurfaceSignature = signature;
    const wpm = clampInt(state.trogdorWpm, 200, 50, 800);
    el.trogdorSurface.innerHTML = renderTrogdorSurfaceFrame({
      groups,
      sessions,
      summary,
      dragonPose,
      readerMarkup: renderTrogdorReader(hovered),
      readButtonLabel: trogdorReadButtonLabel(state.trogdorReading, Boolean(hovered && trogdorClawgReadComplete(hovered))),
      wpm,
      readOnly: state.readOnly,
      hoveredSessionId: state.hoveredTrogdorSessionId,
    });
  }
  renderTrogdorReader(hovered);
}

function renderTrogdorReader(hoveredSession) {
  const wpm = clampInt(state.trogdorWpm, 200, 50, 800);
  const hovered = hoveredSession || null;
  const readerState = trogdorReaderDisplayState(hovered, {
    wordIndex: hovered ? trogdorReaderWordIndex(hovered, wpm) : -1,
    progress: state.trogdorReadProgress,
  });
  const bannerText = readerState.bannerText;
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
    readToggle.textContent = trogdorReadButtonLabel(state.trogdorReading, readerState.readComplete);
  }
  const wpmValue = el.trogdorSurface.querySelector("[data-trogdor-wpm-value]");
  if (wpmValue) {
    wpmValue.textContent = `${wpm} wpm`;
  }
  return readerMarkup;
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
  clearMermaidSvgUrl();
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
        setMermaidSvgUrl(URL.createObjectURL(await svgResponse.blob()));
      } else {
        clearMermaidSvgUrl();
      }
    } else {
      clearMermaidSvgUrl();
    }
    renderMermaidArtifact(artifact);
  } catch (error) {
    setMermaidStatus(`Failed to load Mermaid artifact: ${error.message}`, true);
  } finally {
    state.mermaidArtifact.loading = false;
    syncSheetActionAvailability();
  }
}

function clearMermaidSvgUrl() {
  if (state.mermaidArtifact.svgUrl && typeof URL !== "undefined" && typeof URL.revokeObjectURL === "function") {
    URL.revokeObjectURL(state.mermaidArtifact.svgUrl);
  }
  state.mermaidArtifact.svgUrl = "";
}

function setMermaidSvgUrl(url) {
  clearMermaidSvgUrl();
  state.mermaidArtifact.svgUrl = url || "";
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
  state.surfaceInitInProgress += 1;
  try {
    await state.hud.init(el.hudCanvas, undefined);
  } finally {
    state.surfaceInitInProgress -= 1;
  }
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
  const destroyPatch = terminalDestroyStatePatch();
  state.selectionAnchor = destroyPatch.selectionAnchor;
  state.selectionFocus = destroyPatch.selectionFocus;
  clearHoveredLink(false);
  clearTerminalPaintProbe();
  clearPendingTerminalBytes();
  if (state.terminal) {
    state.terminal.destroy();
  }
  Object.assign(state, destroyPatch);
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
  const isUint8Array = bytes instanceof Uint8Array;
  const plan = terminalPendingByteBufferPlan({ isUint8Array, byteLength: isUint8Array ? bytes.byteLength : 0, pendingByteLength: state.pendingTerminalByteLength, pendingChunkByteLengths: state.pendingTerminalByteChunks.map((chunk) => chunk?.byteLength || 0), maxPendingBytes: MAX_PENDING_TERMINAL_BYTES });
  if (!plan.accept) return false;
  const copy = new Uint8Array(bytes);
  state.pendingTerminalByteChunks.push(copy);
  state.pendingTerminalByteLength += copy.byteLength;
  for (let index = 0; index < plan.dropCount; index += 1) {
    const dropped = state.pendingTerminalByteChunks.shift();
    state.pendingTerminalByteLength -= dropped?.byteLength || 0;
  }
  setConnectionStatus(plan.status);
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
  const hasCurrentSession = Boolean(currentSession());
  const wasActive = state.terminalFallbackActive;
  const nextActive = Boolean(active && hasCurrentSession);
  const plan = terminalFallbackActivationPlan({ active, hasCurrentSession, wasActive, hasTerminal: Boolean(state.terminal), clearText: options.clearText !== false, nearBottom: nextActive && wasActive ? terminalFallbackIsNearBottom() : false });
  state.terminalFallbackActive = plan.terminalFallbackActive;
  el.terminalFallback.classList.toggle("hidden", plan.hidden);
  el.terminalFallback.setAttribute("aria-hidden", plan.ariaHidden);
  if (plan.updateAutoFollow) state.terminalFallbackAutoFollow = plan.autoFollow;
  if (plan.clearText) el.terminalFallback.textContent = "";
  if (plan.startSnapshotPolling) startSnapshotPolling();
  if (plan.focusTerminal) focusTerminalInputSurface({ onlyIfSurfaceFocused: true, preventScroll: true });
  if (plan.stopSnapshotPolling) stopSnapshotPolling();
  syncTerminalStatusStrip();
}

function terminalFallbackIsNearBottom() {
  const maxScrollTop = Math.max(0, el.terminalFallback.scrollHeight - el.terminalFallback.clientHeight);
  return maxScrollTop - el.terminalFallback.scrollTop < 48;
}

function updateTerminalFallbackText(text) {
  const previousScrollTop = el.terminalFallback.scrollTop;
  const nearBottom = state.terminalFallbackAutoFollow ? false : terminalFallbackIsNearBottom();
  const fallbackText = text || "";
  el.terminalFallback.textContent = fallbackText;
  const scrollPlan = terminalFallbackTextScrollPlan({ terminalFallbackAutoFollow: state.terminalFallbackAutoFollow, nearBottom, previousScrollTop, scrollHeight: el.terminalFallback.scrollHeight, clientHeight: el.terminalFallback.clientHeight });
  el.terminalFallback.scrollTop = scrollPlan.scrollTop;
  syncTerminalAccessibilityMirror(fallbackText);
}

function syncTerminalAccessibilityMirror(fallbackText = null) {
  const mirrorText = typeof fallbackText === "string" ? fallbackText : terminalMirrorTextFromRenderer();
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
    state.surfaceInitInProgress += 1;
    try {
      await state.terminal.init(el.terminalCanvas, undefined);
    } finally {
      state.surfaceInitInProgress -= 1;
    }
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

function surfaceBusy() {
  return state.surfaceInitInProgress > 0 || state.surfaceOperationDepth > 0;
}

function frankenTermErrorMessage(error) {
  return error?.message || String(error || "");
}

function isFrankenTermReentryError(error) {
  return /recursive use of an object/i.test(frankenTermErrorMessage(error));
}

function withSurfaceOperation(label, callback) {
  if (surfaceBusy()) {
    return { deferred: true };
  }
  state.surfaceOperationDepth += 1;
  try {
    return { deferred: false, value: callback() };
  } catch (error) {
    if (isFrankenTermReentryError(error)) {
      state.lastRendererDiagnosticError = `${label}: ${frankenTermErrorMessage(error)}`;
      return { deferred: true, error };
    }
    throw error;
  } finally {
    state.surfaceOperationDepth -= 1;
  }
}

function queueRenderRetry() {
  if (state.renderRetryQueued) {
    return;
  }
  state.renderRetryQueued = true;
  window.setTimeout(() => {
    state.renderRetryQueued = false;
    if (!surfaceBusy()) {
      scheduleRender();
    }
  }, 0);
}

function queueHudRender() {
  if (state.hudRenderQueued) {
    return;
  }
  state.hudRenderQueued = true;
  window.setTimeout(() => {
    state.hudRenderQueued = false;
    if (!surfaceBusy()) {
      renderHudSurface();
    }
  }, 0);
}

function queueMeasureAndResizeSurface(pushResize = false, force = false) {
  state.resizeQueued = true;
  state.resizePushResize = state.resizePushResize || Boolean(pushResize);
  state.resizeForce = state.resizeForce || Boolean(force);
  if (state.resizeRetryTimer) {
    return;
  }
  state.resizeRetryTimer = window.setTimeout(() => {
    state.resizeRetryTimer = null;
    if (!state.resizeQueued || surfaceBusy()) {
      return;
    }
    const queuedPushResize = state.resizePushResize;
    const queuedForce = state.resizeForce;
    state.resizeQueued = false;
    state.resizePushResize = false;
    state.resizeForce = false;
    measureAndResizeSurface(queuedPushResize, queuedForce);
  }, 0);
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
    // A surface `init()` holds the wasm instance borrowed across its internal
    // `await`; calling `render()` during that window re-enters the same borrow
    // and trips the wasm-bindgen "recursive use of an object" panic. Re-queue
    // until init settles.
    if (surfaceBusy()) {
      queueRenderRetry();
      return;
    }
    const rendered = withSurfaceOperation("render", () => {
      if (state.terminal) {
        state.terminal.render();
      }
      if (state.hud) {
        state.hud.render();
      }
    });
    if (rendered.deferred) {
      queueRenderRetry();
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

  // `fitToContainer()`/`resize()` take `&mut self` in wasm; invoking them while a
  // surface `init()` is still awaiting (e.g. from the ResizeObserver that fires
  // when init sets the canvas CSS size) re-enters the held borrow and panics.
  // Defer the measurement until init settles instead.
  if (surfaceBusy()) {
    queueMeasureAndResizeSurface(pushResize, force);
    return;
  }
  state.resizeQueued = false;
  state.resizePushResize = false;
  state.resizeForce = false;

  const rect = el.terminalStage.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const fit = withSurfaceOperation("fitToContainer", () =>
    referenceSurface.fitToContainer(rect.width, rect.height, dpr),
  );
  if (fit.deferred) {
    queueMeasureAndResizeSurface(pushResize, force);
    return;
  }
  const resizePlan = terminalResizeGeometryPlan({ cols: fit.value?.cols, rows: fit.value?.rows, currentCols: state.currentCols, currentRows: state.currentRows, force, pushResize, hasTerminal: Boolean(state.terminal) });
  if (!resizePlan.shouldResize) {
    return;
  }

  const resized = withSurfaceOperation("resize", () => {
    if (state.hud) {
      state.hud.resize(resizePlan.cols, resizePlan.rows);
    }
    if (state.terminal) {
      state.terminal.resize(resizePlan.cols, resizePlan.rows);
    }
  });
  if (resized.deferred) {
    queueMeasureAndResizeSurface(pushResize, force);
    return;
  }

  state.currentCols = resizePlan.cols;
  state.currentRows = resizePlan.rows;
  renderHudSurface();
  scheduleRender();

  if (resizePlan.sendResize) {
    sendResize();
  }
  if (resizePlan.captureDiagnostic) {
    captureTerminalRendererDiagnostic(resizePlan.diagnosticReason);
  }
}

function captureTerminalRendererDiagnostic(reason = "frame") {
  if (!terminalSupports("snapshotResizeStormFrameJsonl")) {
    return null;
  }
  if (surfaceBusy()) {
    return null;
  }
  const frameIndex = state.rendererDiagnosticSequence;
  state.rendererDiagnosticSequence += 1;
  const timestamp = new Date().toISOString();
  const diagnostic = withSurfaceOperation("snapshotResizeStormFrameJsonl", () => {
    const line = state.terminal.snapshotResizeStormFrameJsonl("swimmers-web", 0, timestamp, frameIndex);
    const parsed = JSON.parse(String(line || "{}"));
    return { line, parsed };
  });
  if (diagnostic.deferred) {
    return null;
  }
  try {
    const { line, parsed } = diagnostic.value;
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
  // `applyPatchBatchFlat()` takes `&mut self`; while a surface `init()` is still
  // awaiting it holds that borrow, so re-entering here would panic. Defer the
  // HUD patch until init settles, then re-run.
  if (surfaceBusy()) {
    queueHudRender();
    return;
  }
  const frame = buildSurfaceFrame(buildSurfaceModel());
  state.surfaceZones = frame.zones ?? [];
  state.surfaceMasks = frame.masks ?? [];
  const patched = withSurfaceOperation("applyPatchBatchFlat", () => {
    state.hud.applyPatchBatchFlat(frame.spans, frame.cells);
  });
  if (patched.deferred) {
    queueHudRender();
    return;
  }
  scheduleRender();
}

function syncTerminalPresentation() {
  const plan = terminalPresentationPlan({ hasCurrentSession: Boolean(currentSession()), trogdorAtlasOpen: state.trogdorAtlasOpen, hasTerminal: Boolean(state.terminal), terminalFallbackActive: state.terminalFallbackActive });
  document.body.classList.toggle("terminal-focus-mode", plan.terminalFocusMode);
  el.terminalStage.classList.toggle("terminal-view-active", plan.terminalStageActive);
  syncTerminalInputDock();
  syncTrogdorBackButton();
  syncTerminalWorkbench();
  if (state.hud) {
    el.hudCanvas.classList.toggle("hidden", plan.hudHidden);
    [el.hudCanvas.style.display, el.hudCanvas.style.visibility] = [plan.hudDisplay, plan.hudVisibility];
  }
  if (plan.showTerminalCanvas) {
    el.terminalCanvas.classList.toggle("hidden", plan.terminalCanvasHidden);
    [el.terminalCanvas.style.display, el.terminalCanvas.style.visibility] = [plan.terminalCanvasDisplay, plan.terminalCanvasVisibility];
  }
  el.terminalFallback.classList.toggle("hidden", plan.terminalFallbackHidden);
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
  state.readOnly = true;
  syncWriteAccess();
  setConnectionStatus(
    resumeFromSeq ? `connecting; resuming from seq ${resumeFromSeq}` : "connecting; input disabled",
  );

  ws.onopen = () => {
    if (generation !== state.connectionGeneration || state.ws !== ws) {
      ws.close();
      return;
    }
    const sentAuth = sendSessionSocketAuth(ws);
    measureAndResizeSurface(true, true);
    state.reconnectAttempt = 0;
    setConnectionStatus(sentAuth ? "authenticating; input disabled" : "attached");
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
  return buildSessionSocketUrl(session, window.location, state.lastTerminalSeqBySession.get(session.session_id));
}

function sessionSocketAuthMessage() {
  return sessionSocketAuthMessageForToken(state.token);
}

function sendSessionSocketAuth(ws) {
  const message = sessionSocketAuthMessage();
  if (!message || !ws || ws.readyState !== WebSocket.OPEN) {
    return false;
  }
  ws.send(message);
  return true;
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
  const canReadLiveText = state.terminalFallbackActive && state.terminal;
  const plan = terminalLiveFrameFallbackPlan({ terminalFallbackActive: state.terminalFallbackActive, hasTerminal: Boolean(state.terminal), liveText: canReadLiveText ? terminalMirrorTextFromRenderer() : "", existingFallbackText: el.terminalFallback.textContent });
  if (!plan.update) {
    return false;
  }
  updateTerminalFallbackText(plan.text);
  return true;
}

function scheduleTerminalPaintProbe() {
  const plan = terminalPaintProbeSchedulePlan({ terminalPaintVerified: state.terminalPaintVerified, terminalFallbackActive: state.terminalFallbackActive, hasProbeTimer: Boolean(state.terminalPaintProbeTimer), hasTerminal: Boolean(state.terminal), hasCurrentSession: Boolean(currentSession()), terminalFrameBytesSeen: state.terminalFrameBytesSeen });
  if (!plan.scheduleProbe) {
    return;
  }

  state.terminalPaintProbeTimer = window.setTimeout(() => {
    state.terminalPaintProbeTimer = null;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        void verifyTerminalPaintOrFallback();
      });
    });
  }, plan.delayMs);
}

function terminalPaintVerificationContext(extra = {}) {
  return { hasTerminal: Boolean(state.terminal), terminalPaintVerified: state.terminalPaintVerified, terminalFallbackActive: state.terminalFallbackActive, hasCurrentSession: Boolean(currentSession()), ...extra };
}

function applyTerminalPaintVerificationPlan(plan) {
  if (plan.type === "painted") {
    state.terminalPaintVerified = true;
    captureTerminalRendererDiagnostic(plan.diagnosticReason);
    setTerminalTextFallbackActive(plan.fallbackActive);
    return true;
  }
  if (plan.type === "activate_fallback") {
    setTerminalTextFallbackActive(plan.fallbackActive, { clearText: plan.clearText });
    syncTerminalPresentation();
    return true;
  }
  return plan.done;
}

async function verifyTerminalPaintOrFallback() {
  let plan = terminalPaintVerificationPlan(terminalPaintVerificationContext());
  if (applyTerminalPaintVerificationPlan(plan)) return;
  plan = terminalPaintVerificationPlan(terminalPaintVerificationContext({ canvasHasVisiblePixels: terminalCanvasHasVisiblePixels() }));
  if (applyTerminalPaintVerificationPlan(plan)) return;
  const hasSnapshotText = await refreshSnapshotFallback();
  plan = terminalPaintVerificationPlan(terminalPaintVerificationContext({ afterSnapshotRefresh: true }));
  if (applyTerminalPaintVerificationPlan(plan)) return;
  applyTerminalPaintVerificationPlan(terminalPaintVerificationPlan(terminalPaintVerificationContext({ afterSnapshotRefresh: true, canvasHasVisiblePixels: terminalCanvasHasVisiblePixels(), hasSnapshotText })));
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
        setConnectionStatus("attached");
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

  const plan = controlEventSessionPatchPlan(state.sessions[index], message);
  state.sessions[index] = plan.session;
  syncTrogdorCueTransitions();
  syncTerminalStatusStrip();
  renderHudSurface();
  refreshSelectedSessionSidecarsFromEvent(sessionId, plan.event);
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

  state.sessions[index] = lifecycleDeletedSessionPatchPlan(state.sessions[index], message);
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
  const plan = inputAckActionPlan(message);
  if (plan.action === "ignore") {
    return;
  }
  updateInputDeliveryStatus(plan.id, plan.status, plan.detail);
  scheduleInputAckCleanup(plan.id, plan.expectedStatus, plan.delayMs);
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
      if (bytes.byteLength > MAX_TERMINAL_PASTE_BYTES) {
        setUtilityStatus(
          `Input blocked: ${bytes.byteLength} bytes exceeds ${MAX_TERMINAL_PASTE_BYTES}.`,
          true,
          3200,
        );
        continue;
      }
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
  return terminalComposerControlAction(event, {
    hasSelection: terminalInlineInputHasSelection(),
    inputValue: el.terminalInlineInput?.value,
  });
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
  const fallbackActive = state.terminalFallbackActive;
  const globalShortcutHandled = fallbackActive && handleGlobalShortcut(event);
  const shouldCaptureTerminalKey = fallbackActive && !globalShortcutHandled && shouldCaptureKey(event);
  const plan = terminalFallbackKeydownPlan({
    terminalFallbackActive: fallbackActive,
    globalShortcutHandled,
    shouldCaptureKey: shouldCaptureTerminalKey,
    beginsResponse: shouldCaptureTerminalKey && keyBeginsTrogdorResponse(event),
  });
  if (plan.preventDefault) event.preventDefault();
  if (plan.stopPropagation) event.stopPropagation?.();
  if (plan.markResponse) markTrogdorSessionsResponded([state.selectedSessionId]);
  if (plan.forwardKey) forwardTerminalKeyDown(event);
  return plan.handled;
}

function handleTerminalFallbackPasteEvent(event) {
  const plan = terminalFallbackPastePlan({
    terminalFallbackActive: state.terminalFallbackActive, readOnly: state.readOnly,
    hasCurrentSession: Boolean(currentSession()), text: event.clipboardData?.getData("text") ?? "",
  });
  if (plan.preventDefault) event.preventDefault();
  if (plan.stopPropagation) event.stopPropagation?.();
  if (plan.sendText) sendTerminalText(plan.text);
  return plan.handled;
}

function runTerminalFocusAction(plan) {
  const action = terminalStageFocusExecutorPlan(plan);
  if (action.forwardEvent) forwardTerminalEvent(action.event);
}

function runTerminalFallbackPointerFocusAction(plan) {
  if (!plan.focusTerminal) return;
  const focus = () => focusTerminalInputSurface({ preventScroll: true });
  if (plan.scheduleFrame) requestAnimationFrame(focus);
  else focus();
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
  // `setHoveredLinkId()` takes `&mut self`; don't touch the wasm instance while
  // another FrankenTerm operation is active.
  if (terminalSupports("setHoveredLinkId") && !surfaceBusy()) {
    const cleared = withSurfaceOperation("setHoveredLinkId", () => state.terminal.setHoveredLinkId(0));
    if (!cleared.deferred) {
      scheduleRender();
    }
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

  // Hover probing touches the wasm instance; skip it while another FrankenTerm
  // operation is active. A later mousemove re-runs this once the operation
  // settles.
  if (surfaceBusy()) {
    return;
  }

  const cell = mouseCell(event);
  const hover = withSurfaceOperation("link hover", () => {
    const url = state.terminal.linkUrlAt(cell.x, cell.y) ?? "";
    const linkId =
      terminalSupports("linkAt") && terminalSupports("setHoveredLinkId")
        ? state.terminal.linkAt(cell.x, cell.y)
        : null;
    if (linkId !== null) {
      state.terminal.setHoveredLinkId(linkId);
    }
    return { url, highlighted: linkId !== null };
  });
  if (hover.deferred) {
    return;
  }
  const { url, highlighted } = hover.value;
  state.hoveredLinkUrl = url;
  if (highlighted) {
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

async function handleSendFormSubmit(event) {
  event.preventDefault();
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
  }
}

async function handleAuthTokenButtonAction(action) {
  const plan = authTokenButtonPlan(action, el.tokenInput.value);
  if (plan.type === "ignore") return false;
  persistToken(plan.token);
  if (plan.resetReadOnly) {
    state.readOnly = false;
    syncWriteAccess();
  }
  closeSheets();
  return refreshSessions().then(() => true);
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

function filteredCommandPaletteItems() {
  return filteredCommandPaletteItemsForState({
    selectedSession: currentSession(),
    readOnly: state.readOnly,
    sessions: state.sessions,
    copyFrameAction: copyTerminalFrameText,
    query: el.paletteSearch?.value,
  });
}

function renderCommandPalette() {
  if (!el.paletteResults) {
    return;
  }
  state.paletteItems = filteredCommandPaletteItems();
  state.paletteIndex = clampInt(state.paletteIndex, 0, 0, Math.max(0, state.paletteItems.length - 1));
  el.paletteResults.innerHTML = renderCommandPaletteResultsHtml(state.paletteItems, state.paletteIndex);
}

async function runCommandPaletteItem(item = state.paletteItems[state.paletteIndex]) {
  const plan = commandPaletteExecutionPlan(item);
  if (plan.type === "none") {
    return false;
  }
  closeSheets();
  if (plan.type === "selectSession") {
    await selectSession(plan.sessionId);
  } else if (plan.type === "invokeAction") {
    await plan.action();
  } else if (plan.type === "dispatchAction") {
    await handleSurfaceAction({ type: "action", actionId: plan.actionId });
  } else {
    return false;
  }
  return true;
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
  Object.assign(state, trogdorAtlasTransitionState("close_terminal"));
  syncTrogdorReaderTimer();
  applyTrogdorAtlasVisibility();
  syncTerminalPresentation();
}

function openTrogdorAtlas() {
  Object.assign(state, trogdorAtlasTransitionState("open"));
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
      advanceTrogdorReaderProgressForCurrentHover();
      const toggle = trogdorReaderToggleAction(state.trogdorReading, currentTrogdorSurfaceSession(), trogdorClawgReadComplete);
      if (toggle.session) startTrogdorReaderForSession(toggle.session, { readAgain: toggle.readAgain });
      if (toggle.reading !== null) state.trogdorReading = toggle.reading;
      if (toggle.restartClock) state.trogdorReaderStartedAt = performance.now();
      renderHudSurface();
      syncTrogdorReaderTimer();
      break;
    }
    case "trogdor_wpm_down":
    case "trogdor_wpm_up":
    {
      advanceTrogdorReaderProgressForCurrentHover();
      state.trogdorWpm = trogdorReaderWpmForAction(zone.actionId, state.trogdorWpm);
      resetTrogdorReaderAfterWpmChange();
      renderHudSurface();
      break;
    }
    case "toggle_trogdor_atlas":
      Object.assign(state, trogdorAtlasTransitionState("toggle", state.trogdorAtlasOpen));
      renderHudSurface();
      break;
    case "trogdor_send":
      openSendSheet(trogdorActionPayloadForZone(zone));
      break;
    case "trogdor_group_send":
      openSendSheet(trogdorActionPayloadForZone(zone));
      break;
    case "trogdor_launch":
      openCreateSheetForCwd(trogdorActionPayloadForZone(zone).cwd);
      break;
    case "trogdor_mermaid":
      await selectSession(trogdorActionPayloadForZone(zone).sessionId);
      openMermaidSheet();
      break;
    case "trogdor_commit":
      await selectSession(trogdorActionPayloadForZone(zone).sessionId);
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
    {
      const focusStatus = trogdorTerminalFocusStatus(currentSession());
      Object.assign(state, trogdorAtlasTransitionState("close"));
      renderHudSurface();
      focusTerminalInputSurface({ preventScroll: true });
      setUtilityStatus(focusStatus.message, focusStatus.error, focusStatus.timeoutMs);
      break;
    }
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

function applyTerminalStagePointerPlan(event, plan) {
  if (plan.suppressClick) state.surfaceClickSuppressUntil = performance.now() + SURFACE_CLICK_SUPPRESS_MS;
  if (plan.preventDefault) event.preventDefault();
  if (plan.handleAction) {
    void handleSurfaceAction(plan.action);
    return;
  }
  if (plan.focusMobileThenTerminal) {
    if (!isCoarsePointer() || !focusMobileKeyboard()) {
      focusTerminalInputSurface({ preventScroll: true });
    }
    return;
  }
  if (plan.focusTerminal) focusTerminalInputSurface({ preventScroll: true });
}

function handleTerminalStageClick(event) {
  const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
  const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
  const plan = terminalStageClickPlan({
    fallbackOwnsPointer,
    hit,
    activeSheet: state.activeSheet,
    ignoreSyntheticClick: hit.action ? shouldIgnoreSyntheticClick(performance.now(), state.surfaceClickSuppressUntil) : false,
  });
  applyTerminalStagePointerPlan(event, plan);
}

function handleTerminalStageTouchEnd(event) {
  const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
  const plan = terminalStageTouchEndPlan({
    fallbackOwnsPointer,
    hit: fallbackOwnsPointer ? {} : surfaceHit(event),
    activeSheet: state.activeSheet,
  });
  applyTerminalStagePointerPlan(event, plan);
}

function updateHoveredTrogdorSurface(zone) {
  const previousSessionId = state.hoveredTrogdorSessionId;
  const nextSessionId = trogdorHoverSessionIdForZone(zone, previousSessionId);
  if (nextSessionId === previousSessionId) {
    return;
  }
  Object.assign(state, trogdorHoverReaderResetState(nextSessionId));
  if (el.trogdorSurface) {
    const agents = el.trogdorSurface.querySelectorAll("[data-trogdor-agent]");
    for (const agent of agents) {
      agent.classList.toggle("is-hovered", Boolean(nextSessionId) && agent.dataset.sessionId === nextSessionId);
    }
  }
  if (nextSessionId) {
    const session = trogdorRawSessionForHover(state.sessions, nextSessionId, { normalize: false });
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
  const timerAction = trogdorReaderTimerAction(
    currentTrogdorSurfaceSession(), trogdorSessionCanRead, trogdorClawgReadComplete,
    state.trogdorReading, state.trogdorReaderTimer,
  );
  if (timerAction === "start") {
    state.trogdorReaderTimer = window.setInterval(() => renderHudSurface(), 120);
    return;
  }
  if (timerAction === "stop") {
    window.clearInterval(state.trogdorReaderTimer);
    state.trogdorReaderTimer = null;
  }
}

async function handleTrogdorDomAction(button) {
  if (!button || button.disabled) {
    return;
  }
  await handleSurfaceAction(trogdorDomActionZoneForDataset(button.dataset));
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
  const plan = globalShortcutPlan(event, {
    activeSheet: state.activeSheet,
    trogdorAtlasOpen: state.trogdorAtlasOpen,
    selectMode: state.selectMode,
    readOnly: state.readOnly,
    hasCurrentSession: Boolean(currentSession()),
    hoveredLinkUrl: state.hoveredLinkUrl,
  });
  if (plan.type === "unhandled") {
    return false;
  }
  switch (plan.type) {
    case "open_palette":
      openCommandPalette();
      break;
    case "zoom_in":
      setTerminalZoom(state.terminalZoom + TERMINAL_ZOOM_STEP, { announce: true });
      break;
    case "zoom_out":
      setTerminalZoom(state.terminalZoom - TERMINAL_ZOOM_STEP, { announce: true });
      break;
    case "zoom_reset":
      setTerminalZoom(1, { announce: true });
      break;
    case "close_sheets":
      closeSheets();
      break;
    case "close_trogdor_atlas":
      Object.assign(state, trogdorAtlasTransitionState("close"));
      renderHudSurface();
      break;
    case "exit_select_mode":
      setSelectMode(false);
      break;
    case "open_sheet":
      openSheet(plan.sheetId);
      break;
    case "open_thought_config":
      openThoughtConfigSheet();
      break;
    case "open_native":
      openNativeSheet();
      break;
    case "open_mermaid":
      openMermaidSheet();
      break;
    case "toggle_follow":
      void toggleFollowPublished();
      break;
    case "toggle_select":
      setSelectMode(!state.selectMode);
      break;
    case "copy_selection":
      void copyTerminalSelection();
      break;
    case "copy_hovered_link":
      void copyHoveredLink();
      break;
    case "refresh_sessions":
      void refreshSessions();
      break;
    default:
      break;
  }
  return true;
}

function handleMobileKeyboardProxyKeydown(event) {
  const globalShortcutHandled = handleGlobalShortcut(event);
  const keyPlan = globalShortcutHandled ? { type: "ignore" } : mobileKeyboardKeyPlan(event, {
    readOnly: state.readOnly,
    hasCurrentSession: Boolean(currentSession()),
  });
  const shouldForwardKey = !globalShortcutHandled && keyPlan.type === "forward_key";
  const plan = mobileKeyboardKeydownPlan({
    globalShortcutHandled,
    keyPlan,
    beginsResponse: shouldForwardKey && keyBeginsTrogdorResponse(event),
  });
  if (plan.preventDefault) event.preventDefault();
  if (plan.closeKeyboard) closeMobileKeyboard();
  if (plan.focusTerminal) focusTerminalInputSurface({ preventScroll: true });
  if (plan.markResponse) markTrogdorSessionsResponded([state.selectedSessionId]);
  if (plan.forwardKey) forwardTerminalKeyDown(event);
  return plan.handled;
}

function handleMobileKeyboardProxyInput(event) {
  const plan = mobileKeyboardInputPlan(event, {
    readOnly: state.readOnly,
    hasCurrentSession: Boolean(currentSession()),
    proxyValue: el.mobileKeyboardProxy.value,
  });
  el.mobileKeyboardProxy.value = "";
  const action = mobileKeyboardInputExecutorPlan(plan);
  if (action.forwardEvent) forwardTerminalEvent(action.forwardEvent);
  if (action.sendText) sendTerminalText(action.text);
  return action.handled;
}

function handleTerminalInlineInputKeydown(event) {
  const actionId = event.key === "Enter" && !event.shiftKey ? "" : terminalKeyActionForDomEvent(event);
  const plan = terminalInlineInputKeydownPlan(event, actionId);
  if (plan.preventDefault) event.preventDefault();
  if (plan.submit) void submitTerminalInputDock();
  if (plan.sendKey) sendTerminalControlKey(plan.actionId);
  if (plan.stopPropagation) event.stopPropagation();
  return plan.handled;
}

function handleTerminalWorkbenchWidgetsClick(event) {
  const plan = workbenchWidgetClickPlan(event.target);
  if (plan.type === "ignore") {
    return false;
  }
  event.preventDefault();
  if (plan.type === "open_mermaid") {
    openSheet("mermaid");
    return;
  }
  const refreshWidgets = plan.type === "select_turn";
  if (refreshWidgets) {
    state.workbenchSelectedTurnId = plan.turnId;
    state.workbenchWidgets.transcript = null;
    state.workbenchWidgets.transcriptTurnId = "";
    state.workbenchWidgets.transcriptNextCursor = 0;
  } else {
    state.workbenchLogMode = plan.mode;
  }
  renderWorkbenchWidgets();
  if (refreshWidgets) {
    void refreshWorkbenchWidgetsForSelectedSession({ force: true, silent: true });
  }
  focusTerminalInputSurface({ preventScroll: true });
}

function handleTerminalWorkbenchWidgetsLogEvent(event) {
  const plan = workbenchWidgetLogPlan(event.type, event.target);
  if (plan.type === "set_log_search") {
    state.workbenchLogSearch = plan.query;
  } else if (plan.type === "set_log_filter") {
    state.workbenchLogFilter = plan.filter;
  } else {
    return;
  }
  renderWorkbenchWidgets();
}

function handleCommandPaletteEvent(event) {
  const target = event.target instanceof Element ? event.target : null;
  const plan = event.type === "keydown"
    ? commandPaletteSearchKeyPlan(event, state.paletteIndex, state.paletteItems.length)
    : commandPaletteResultEventPlan(event.type, target, state.paletteItems.length);
  if (plan.type === "ignore") {
    return false;
  }
  if (plan.preventDefault) {
    event.preventDefault();
  }
  if (Number.isFinite(plan.index)) {
    state.paletteIndex = plan.index;
  }
  if (plan.type === "run_item") {
    void runCommandPaletteItem();
    return true;
  }
  renderCommandPalette();
  return true;
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
  el.terminalWorkbenchWidgets.addEventListener("click", handleTerminalWorkbenchWidgetsClick);
  el.terminalWorkbenchWidgets.addEventListener("input", handleTerminalWorkbenchWidgetsLogEvent);
  el.terminalWorkbenchWidgets.addEventListener("change", handleTerminalWorkbenchWidgetsLogEvent);
  el.terminalInputDock.addEventListener("submit", (event) => {
    event.preventDefault();
    void submitTerminalInputDock();
  });
  el.terminalInlineInput.addEventListener("input", () => {
    resizeTerminalInlineInput();
    syncTerminalInputDock();
  });
  el.terminalInlineInput.addEventListener("keydown", handleTerminalInlineInputKeydown);
  el.terminalKeyStrip.addEventListener("click", (event) => {
    const action = terminalKeyStripClickExecutorPlan(terminalKeyStripClickPlan(event.type, event.target));
    if (!action.sendKey) return;
    if (action.preventDefault) event.preventDefault();
    sendTerminalControlKey(action.actionId);
    focusTerminalInputSurface({ preventScroll: true });
  });
  el.terminalInlineInput.addEventListener("focus", () => runTerminalFocusAction(terminalStageFocusPlan("focus", { activeSheet: state.activeSheet })));
  el.terminalFallback.addEventListener("mousedown", () => runTerminalFallbackPointerFocusAction(terminalFallbackPointerFocusPlan("mousedown", { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet })));
  el.terminalFallback.addEventListener("click", () => runTerminalFallbackPointerFocusAction(terminalFallbackPointerFocusPlan("click", { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet })));
  el.terminalFallback.addEventListener("keydown", handleTerminalFallbackKeyEvent);
  el.terminalFallback.addEventListener("paste", handleTerminalFallbackPasteEvent);
  el.terminalFallback.addEventListener("focus", () => runTerminalFocusAction(terminalFallbackFocusPlan("focus", { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet })));
  el.terminalFallback.addEventListener("blur", () => runTerminalFocusAction(terminalFallbackFocusPlan("blur", { terminalFallbackActive: state.terminalFallbackActive, mobileKeyboardOwnsFocus: document.activeElement === el.mobileKeyboardProxy })));
  el.terminalFallback.addEventListener("scroll", () => {
    const plan = terminalFallbackScrollPlan("scroll", { terminalFallbackActive: state.terminalFallbackActive, nearBottom: state.terminalFallbackActive ? terminalFallbackIsNearBottom() : false });
    if (plan.updateAutoFollow) state.terminalFallbackAutoFollow = plan.autoFollow;
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
  el.mobileKeyboardProxy.addEventListener("keydown", handleMobileKeyboardProxyKeydown);
  el.mobileKeyboardProxy.addEventListener("input", handleMobileKeyboardProxyInput);
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
  el.paletteSearch.addEventListener("keydown", handleCommandPaletteEvent);
  el.paletteResults.addEventListener("mousemove", handleCommandPaletteEvent);
  el.paletteResults.addEventListener("click", handleCommandPaletteEvent);
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

  el.sendForm.addEventListener("submit", handleSendFormSubmit);
  el.sendCloseButton.addEventListener("click", () => {
    state.sendTarget = null;
    closeSheets();
  });
  el.sendHistory.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const plan = sendHistoryClickPlan(event.type, target, state.sendHistory);
    if (plan.type === "use_history") {
      el.sendInput.value = plan.text;
      el.sendInput.focus();
    }
  });

  el.saveTokenButton.addEventListener("click", () => handleAuthTokenButtonAction("save"));
  el.clearTokenButton.addEventListener("click", () => handleAuthTokenButtonAction("clear"));
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
    renderDirEntries(currentDirListingPayload());
  });
  el.createBatchVisible.addEventListener("click", handleCreateBatchVisibleAction);
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
  el.dirsList.addEventListener("change", handleDirCheckboxChange);
  el.dirsList.addEventListener("click", async (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target) {
      return;
    }
    if (target.closest(".dir-open-url")) {
      return;
    }

    if (await handleDirGroupChipClick(event, target)) {
      return;
    }

    const groupActionPlan = dirBrowserGroupMembershipClickPlan(event.type, target);
    if (groupActionPlan.type === "membership") {
      await updateDirEntryGroupMembership(groupActionPlan.path, groupActionPlan.action, groupActionPlan.group, groupActionPlan.removeGroup);
      return;
    }

    const rowPlan = dirBrowserRowClickPlan(event.type, target);
    if (rowPlan.type !== "row") {
      return;
    }
    const path = rowPlan.path;
    el.dirsPath.value = path;
    el.createCwd.value = path;
    if (rowPlan.hasChildren) {
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
    const plan = mermaidPlanTabClickPlan(event.type, event.target instanceof Element ? event.target : null);
    if (plan.type === "load_plan_file") await loadMermaidPlanFile(plan.planFile);
  });
  el.mermaidCloseButton.addEventListener("click", closeSheets);

  for (const binding of terminalStageCaptureBindings()) {
    el.terminalStage.addEventListener(
      binding.eventType,
      (event) => captureSurfaceAction(event, binding.action),
      binding.options,
    );
  }

  el.terminalStage.addEventListener("click", handleTerminalStageClick);

  el.terminalStage.addEventListener("touchend", handleTerminalStageTouchEnd, { passive: false });

  el.terminalStage.addEventListener("keydown", (event) => {
    const globalShortcutHandled = handleGlobalShortcut(event);
    const shouldCaptureTerminalKey = !globalShortcutHandled && shouldCaptureKey(event);
    const plan = terminalStageKeydownPlan({
      globalShortcutHandled,
      shouldCaptureKey: shouldCaptureTerminalKey,
      beginsResponse: shouldCaptureTerminalKey && keyBeginsTrogdorResponse(event),
    });
    if (plan.preventDefault) event.preventDefault();
    if (plan.markResponse) markTrogdorSessionsResponded([state.selectedSessionId]);
    if (plan.forwardKey) forwardTerminalKeyDown(event);
  });

  el.terminalStage.addEventListener("paste", (event) => {
    const action = terminalStagePasteExecutorPlan(terminalStagePastePlan(state.readOnly, event.clipboardData?.getData("text") ?? ""));
    if (action.preventDefault) event.preventDefault();
    if (action.sendText) sendTerminalText(action.text);
  });

  el.terminalStage.addEventListener("focus", () => runTerminalFocusAction(terminalStageFocusPlan("focus", { activeSheet: state.activeSheet })));
  el.terminalStage.addEventListener("blur", () => runTerminalFocusAction(terminalStageFocusPlan("blur", { mobileKeyboardOwnsFocus: document.activeElement === el.mobileKeyboardProxy })));

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
    queueMeasureAndResizeSurface(true, false);
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
  scheduleRender,
  measureAndResizeSurface,
  queueMeasureAndResizeSurface,
  syncTerminalPresentation,
  sessionSocketUrl,
  sessionSocketAuthMessage,
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
  safeAnchorHref,
  isLoopbackHostname,
  validateFrankenTermSurface,
  captureTerminalRendererDiagnostic,
  sendRawTextToSession,
  sendGroupLine,
  markTrogdorSessionsResponded,
  handleTerminalFallbackKeyEvent,
  handleTerminalFallbackPasteEvent,
  handleGlobalShortcut,
  handleMobileKeyboardProxyKeydown,
  handleMobileKeyboardProxyInput,
  sendTerminalControlKey,
  terminalKeyActionForDomEvent,
  handleTerminalInlineInputKeydown,
  handleSendFormSubmit, handleAuthTokenButtonAction, handleCreateBatchVisibleAction, handleDirCheckboxChange, handleDirGroupChipClick,
  focusTerminalInputSurface,
  syncTerminalInputDock,
  submitTerminalInputDock,
  handleSocketText,
  syncTerminalWorkbench,
  renderTerminalWorkbench,
  renderWorkbenchWidgets,
  handleTerminalWorkbenchWidgetsClick,
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
  handleCommandPaletteEvent,
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
