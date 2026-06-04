import { buildSurfaceFrame, surfaceActionAt, surfaceConsumesPointer } from "./rendered_surface.js";
import {
  appEventListenerBindingPlan, authTokenButtonPlan, controlEventSessionPatchPlan, eventCell, globalShortcutPlan, initialStateBootPlan, inputAckActionPlan, lifecycleDeletedSessionPatchPlan, mobileKeyboardInputExecutorPlan, mobileKeyboardInputPlan,
  sheetActionAvailabilityPlan,
  mobileKeyboardKeydownPlan, mobileKeyboardKeyPlan, shouldIgnoreSyntheticClick, surfaceActionDispatchContextPlan, surfaceActionDispatchPlan, surfaceActionExecutionContextPlan, surfaceActionExecutionPlan, surfaceActionFocusTerminalExecutionPlan, surfaceActionTrogdorReaderExecutionPlan,
  terminalComposerControlAction, terminalDestroyStatePatch, terminalFallbackFocusPlan, terminalFallbackKeydownPlan, terminalFallbackPastePlan, terminalFallbackPointerFocusPlan, terminalInlineInputKeydownPlan, terminalKeyStripClickExecutorPlan, terminalKeyStripClickPlan, terminalStageCaptureBindings, terminalStageClickPlan, terminalStageFocusExecutorPlan, terminalStageFocusPlan,
  normalizeTerminalZoomValue, terminalAuxiliaryControlsPlan, terminalFallbackScrollPlan, terminalInputDockPlan, terminalPaintProbeSchedulePlan, terminalPaintVerificationPlan, terminalPresentationPlan, terminalStageKeydownPlan, terminalStageMouseDownPlan, terminalStageMouseMovePlan, terminalStageMouseUpPlan, terminalStagePasteExecutorPlan, terminalStagePastePlan, terminalStageTouchEndPlan, terminalStageWheelPlan, terminalToolsAvailabilityPlan, terminalZoomControlsPlan, terminalZoomLoadValue, terminalZoomPercentLabel, terminalZoomPersistencePlan,
} from "./input_support.js";
import { bindAppEvents } from "./app_event_bindings.js";
import { createTrogdorEventBindings } from "./trogdor_event_bindings.js";
import { createSendController } from "./send_controller.js";
import {
  createThoughtConfigSheetController,
} from "./thought_config_sheet.js";
import {
  createNativeDesktopSheetController,
} from "./native_desktop_sheet.js";
import {
  createTerminalSurfaceRuntimeHelpers,
} from "./terminal_surface_setup.js";
import {
  assertFrankenTermModule,
  canvasHasVisiblePixels,
  frankenTermAssetSummary as formatFrankenTermAssetSummary,
  surfaceBusy as runtimeSurfaceBusy,
  surfaceSupports,
  validateFrankenTermSurface,
  withSurfaceOperation as runSurfaceOperation,
} from "./terminal_runtime.js";
import {
  createSessionSocketController,
} from "./session_socket_controller.js";
import { runTerminalSurfaceResize } from "./terminal_resize.js";
import { runGlobalShortcutAction } from "./global_shortcut_dispatch.js";
import {
  backendHealthWarningText,
  runSessionRefresh,
  sessionDisplayName,
} from "./session_refresh.js";
import { createTerminalWorkbenchController } from "./terminal_workbench_controller.js";
import {
  isSafeMermaidPlanFileName,
  sanitizeMermaidPlanFiles,
} from "./mermaid_artifact.js";
import {
  createMermaidArtifactController,
} from "./mermaid_artifact_controller.js";
import {
  MAX_TERMINAL_PASTE_BYTES,
  frankenTermLinkPolicy,
  isLoopbackHostname,
  safeAnchorHref,
  terminalTextWithinPasteBudget,
  utf8ByteLength,
} from "./terminal_safety.js";
import {
  createTerminalSearchLinksController,
} from "./terminal_search_links.js";
import {
  createTerminalStatusController,
} from "./terminal_status.js";
import {
  decodeTerminalOutputFrame,
  fallbackTextForKeyEvent,
  keyModifiers,
  terminalControlKeyEvent,
} from "./terminal_protocol.js";
import {
  createDirBrowserController,
} from "./dir_browser_controller.js";
import {
  commandPaletteResultEventPlan,
  commandPaletteSearchKeyPlan,
} from "./command_palette.js";
import {
  createCommandPaletteController,
} from "./command_palette_controller.js";
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
  emptyWorkbenchWidgets,
  renderTranscriptBlocks,
} from "./workbench_render.js";
import {
  buildSurfaceModel as buildSurfaceModelFromState,
  formatTime,
  summarizeThought,
  surfaceSession as buildSurfaceSession,
} from "./surface_model.js";
import { createApiClient } from "./api_client.js";
import { createSessionPersistenceController } from "./session_persistence.js";

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

const {
  apiFetch,
  apiMaybeFetch,
  responseJsonOrNull,
} = createApiClient({
  getToken: () => state.token,
  fetchImpl: (...args) => fetch(...args),
});

const {
  normalizeSessionId,
  persistSelectedSession,
  setFollowPublishedSelection,
  syncUrlState,
} = createSessionPersistenceController({
  state,
  windowRef: window,
  documentRef: document,
  storage: localStorage,
  sessionStorageKey: SESSION_STORAGE_KEY,
  normalizeSessionId: normalizeTrogdorSessionId,
  resetAgentContextForSession,
  resetWorkbenchWidgetsForSession,
  closeTrogdorAtlasForTerminal,
  renderHudSurface,
});

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

const terminalStatusController = createTerminalStatusController({
  state,
  el,
  boot,
  defaultDocumentTitle,
  currentSession,
  sessionDisplayName,
  sessionNeedsAttention,
  backendHealthWarningText,
  shortenUrl,
  renderHudSurface,
  documentRef: document,
  setTimeoutRef: (callback, delay) => window.setTimeout(callback, delay),
  clearTimeoutRef: (timer) => clearTimeout(timer),
  webSocketOpenReadyState: () => WebSocket.OPEN,
});

const {
  applyBackendHealth,
  defaultUtilityLabel,
  setConnectionStatus,
  setModeStatus,
  setSearchStatus,
  setUtilityStatus,
  syncTerminalStatusStrip,
} = terminalStatusController;

const thoughtConfigSheet = createThoughtConfigSheetController({
  state,
  el,
  apiFetch,
  refreshSessions,
  syncSheetActionAvailability,
});

const nativeDesktopSheet = createNativeDesktopSheetController({
  state,
  el,
  apiFetch,
  currentSession,
  refreshSessions,
  syncSheetActionAvailability,
});

const terminalSearchLinks = createTerminalSearchLinksController({
  state,
  el,
  terminalSupports,
  hasLiveTerminal,
  scheduleRender,
  renderHudSurface,
  setSearchStatus,
  setUtilityStatus,
  defaultUtilityLabel,
  shortenUrl,
  currentSession,
  frankenTermLinkPolicy,
  surfaceBusy,
  withSurfaceOperation,
  mouseCell,
  syncTerminalTools,
  navigatorRef: globalThis.navigator,
  windowRef: globalThis.window,
  URLImpl: globalThis.URL,
});

const {
  setTerminalSelectionRange,
  clearTerminalSelection,
  setSelectMode,
  refreshTerminalSearch,
  applySearchQuery,
  cycleSearchMatch,
  copyTerminalSelection,
  safeOpenUrl,
  syncLinkTools,
  copyHoveredLink,
  drainTerminalLinkClicks,
  copyTerminalFrameText,
  clearHoveredLink,
  updateHoveredLink,
} = terminalSearchLinks;

const terminalSurfaceRuntime = {
  state,
  el,
  requiredTerminalMethods: FRANKENTERM_TERMINAL_METHODS,
  maxPendingTerminalBytes: MAX_PENDING_TERMINAL_BYTES,
  validateFrankenTermSurface,
  teardownTerminal,
  destroyTerminalInstance,
  currentSession,
  ensureFrankenTerm,
  stopSnapshotPolling,
  startSnapshotPolling,
  focusTerminalInputSurface,
  refreshSnapshotFallback,
  setLoadingState,
  setUtilityStatus,
  setConnectionStatus,
  terminalSupports,
  frankenTermLinkPolicy,
  applyZoomToSurface,
  clearTerminalSelection,
  refreshTerminalSearch,
  syncTerminalTools,
  syncTerminalStatusStrip,
  measureAndResizeSurface,
  feedTerminalBytes,
  prefersReducedMotion: () => window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false,
};

const {
  clearPendingTerminalBytes,
  bufferTerminalBytes,
  flushPendingTerminalBytes,
  setTerminalTextFallbackActive,
  terminalFallbackIsNearBottom,
  updateTerminalFallbackText,
  syncTerminalAccessibilityMirror,
  syncTerminalFallbackFromLiveFrame,
  setupTerminalSurface,
} = createTerminalSurfaceRuntimeHelpers(terminalSurfaceRuntime);

Object.assign(terminalSurfaceRuntime, {
  flushPendingTerminalBytes,
  setTerminalTextFallbackActive,
  syncTerminalAccessibilityMirror,
});

const sessionSocketRuntime = {
  state,
  window,
  WebSocketClass: window.WebSocket,
  currentSession,
  setupHudSurface,
  setupTerminalSurface,
  teardownTerminal,
  disconnectSocket,
  measureAndResizeSurface,
  scheduleSessionRefresh,
  reconnectDelayMs,
  setConnectionStatus,
  setModeStatus,
  syncWriteAccess,
  syncTerminalTools,
  feedTerminalBytes,
  mergeSummary,
  handleInputAck,
  applyControlEvent,
  applyLifecycleEvent,
  refreshSessions,
};

const {
  connectSelectedSession,
  sessionSocketUrl,
  sessionSocketAuthMessage,
  terminalPayloadFromSocketBytes,
  handleSocketText,
} = createSessionSocketController(sessionSocketRuntime);

const terminalResizeRuntime = {
  state,
  el,
  surfaceBusy,
  queueMeasureAndResizeSurface,
  withSurfaceOperation,
  renderHudSurface,
  scheduleRender,
  sendResize,
  captureTerminalRendererDiagnostic,
  devicePixelRatio: () => window.devicePixelRatio || 1,
};

const globalShortcutRuntime = {
  state,
  terminalZoomStep: TERMINAL_ZOOM_STEP,
  openCommandPalette,
  setTerminalZoom,
  closeSheets,
  trogdorAtlasTransitionState,
  renderHudSurface,
  setSelectMode,
  openSheet,
  openThoughtConfigSheet,
  openNativeSheet,
  openMermaidSheet,
  toggleFollowPublished,
  copyTerminalSelection,
  copyHoveredLink,
  refreshSessions,
};

const terminalWorkbenchController = createTerminalWorkbenchController({
  state,
  el,
  refreshMs: AGENT_CONTEXT_REFRESH_MS,
  currentSession,
  normalizeSessionId,
  sessionDisplayName,
  summarizeThought,
  apiFetch,
  apiMaybeFetch,
  responseJsonOrNull,
  openSheet,
  focusTerminalInputSurface,
  documentRef: document,
  requestAnimationFrameRef: typeof requestAnimationFrame === "function" ? requestAnimationFrame : null,
});

const sessionRefreshRuntime = {
  state,
  apiFetch,
  apiMaybeFetch,
  responseJsonOrNull,
  applyOperatorPressure,
  applyBackendHealth,
  syncTrogdorCueTransitions,
  normalizeSessionId,
  sessionExists,
  persistSelectedSession,
  setupHudSurface,
  renderHudSurface,
  syncTerminalTools,
  connectSelectedSession,
  refreshAgentContextForSelectedSession,
  refreshWorkbenchWidgetsForSelectedSession,
  setConnectionStatus,
  setModeStatus,
  resetAgentContextForSession,
  resetWorkbenchWidgetsForSession,
};

const mermaidArtifactController = createMermaidArtifactController({
  state,
  el,
  currentSession,
  apiFetch,
  apiMaybeFetch,
  responseJsonOrNull,
  syncSheetActionAvailability,
  formatTime,
  documentRef: document,
  ElementClass: Element,
  URLImpl: globalThis.URL,
  locationOrigin: () => window.location.origin,
});

function currentSession() {
  return state.sessions.find((session) => session.session_id === state.selectedSessionId) ?? null;
}

function sessionNeedsAttention(session) {
  if (!session) {
    return false;
  }
  const stateLabel = String(session.state || "").toLowerCase();
  return stateLabel === "attention" || rawSessionAwaitingUser(session);
}

function terminalSupports(methodName) {
  return surfaceSupports(state.terminal, methodName);
}

function hasLiveTerminal() {
  return Boolean(state.terminal);
}

function rejectOversizeTerminalText(text, label = "Paste") {
  const bytes = utf8ByteLength(text);
  if (bytes <= MAX_TERMINAL_PASTE_BYTES) {
    return false;
  }
  setUtilityStatus(`${label} blocked: ${bytes} bytes exceeds ${MAX_TERMINAL_PASTE_BYTES}.`, true, 3200);
  return true;
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
  return terminalWorkbenchController.resetAgentContextForSession(sessionId);
}

function resetWorkbenchWidgetsForSession(sessionId) {
  return terminalWorkbenchController.resetWorkbenchWidgetsForSession(sessionId);
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
  return terminalWorkbenchController.syncTerminalWorkbench();
}

function setTerminalWorkbenchOpen(open) {
  return terminalWorkbenchController.setTerminalWorkbenchOpen(open);
}

function renderTerminalWorkbench() {
  return terminalWorkbenchController.renderTerminalWorkbench();
}

async function refreshAgentContextForSelectedSession(options = {}) {
  return terminalWorkbenchController.refreshAgentContextForSelectedSession(options);
}

function writeWorkbenchWidgetsHtml(nextHtml) {
  return terminalWorkbenchController.writeWorkbenchWidgetsHtml(nextHtml);
}

function renderWorkbenchWidgets() {
  return terminalWorkbenchController.renderWorkbenchWidgets();
}

async function refreshWorkbenchWidgetsForSelectedSession(options = {}) {
  return terminalWorkbenchController.refreshWorkbenchWidgetsForSelectedSession(options);
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

function shortenUrl(raw) {
  if (!raw) return "";
  return raw.length > 72 ? `${raw.slice(0, 69)}...` : raw;
}

function surfaceSession(session, options = {}) {
  return buildSurfaceSession(session, {
    ...options,
    operatorPressure: operatorPressureSnapshot(session.session_id),
    sessionBurnt: trogdorSessionBurnt,
    dismissedClawgs: state.trogdorDismissedClawgs,
    readProgress: state.trogdorReadProgress,
  });
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

function setDirStatus(message, isError = false) {
  state.dirBrowser.status = message;
  state.dirBrowser.error = isError ? message : "";
  if (el.dirsSummary) {
    el.dirsSummary.textContent = message || "";
    el.dirsSummary.classList.toggle("error", Boolean(isError));
  }
}

const dirBrowserController = createDirBrowserController({
  state,
  el,
  apiFetch,
  setDirStatus,
  syncSheetActionAvailability,
  currentSession,
  closeSheets,
  refreshSessions,
  selectSession,
  setUtilityStatus,
  openSheet,
  focusActiveSheet,
  parentDir,
  storage: localStorage,
  location: window.location,
  ElementClass: Element,
  pathStorageKey: DIR_BROWSER_PATH_KEY,
  managedOnlyStorageKey: DIR_BROWSER_MANAGED_ONLY_KEY,
});

const {
  clearCreateBatchSelection,
  handleCreateBatchClearClick,
  handleCreateBatchVisibleAction,
  handleCreateCwdInput,
  handleCreateFormSubmit,
  handleCreateLaunchTargetChange,
  handleDirCheckboxChange,
  handleDirGroupChipClick,
  handleDirsListClick,
  handleDirsLoadButtonClick,
  handleDirsManagedOnlyChange,
  handleDirsPathInput,
  handleDirsPathKeydown,
  handleDirsSearchInput,
  handleDirsSpawnHereClick,
  handleDirsUpButtonClick,
  openCreateSheet,
  openCreateSheetForCwd,
  renderCreateBatchBar,
  visibleSelectableDirPaths,
  warmDirBrowserOnStartup,
} = dirBrowserController;

function renderMermaidArtifact(payload) { mermaidArtifactController.renderArtifact(payload); }

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

async function refreshThoughtConfig() { await thoughtConfigSheet.refresh(); }

async function refreshNativeStatus() { await nativeDesktopSheet.refreshNativeStatus(); }

async function refreshMermaidArtifact() { await mermaidArtifactController.refresh(); }

async function openMermaidArtifactHost() { await mermaidArtifactController.openHost(); }

async function loadMermaidPlanFile(name) { await mermaidArtifactController.loadPlanFile(name); }

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
  await runSessionRefresh(sessionRefreshRuntime);
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
      state.frankenAssetSummary = formatFrankenTermAssetSummary(boot.franken_term_asset_info);
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

function clearTerminalPaintProbe() {
  if (state.terminalPaintProbeTimer) {
    window.clearTimeout(state.terminalPaintProbeTimer);
    state.terminalPaintProbeTimer = null;
  }
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
  return runtimeSurfaceBusy(state);
}

function withSurfaceOperation(label, callback) {
  return runSurfaceOperation(state, label, callback);
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
  runTerminalSurfaceResize({ pushResize, force }, terminalResizeRuntime);
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
  return buildSurfaceModelFromState({
    state,
    boot,
    currentSession,
    operatorPressureSnapshot,
    sessionBurnt: trogdorSessionBurnt,
    normalizeSessionId,
    now: () => performance.now(),
    websocketOpen: WebSocket.OPEN,
  });
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
  return canvasHasVisiblePixels(el.terminalCanvas, document);
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
function handleTerminalInlineInputFocus() { runTerminalFocusAction(terminalStageFocusPlan("focus", { activeSheet: state.activeSheet })); }
function handleTerminalFallbackPointerFocus(eventType) { runTerminalFallbackPointerFocusAction(terminalFallbackPointerFocusPlan(eventType, { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet })); }
function handleTerminalFallbackFocusEvent(eventType) { runTerminalFocusAction(terminalFallbackFocusPlan(eventType, eventType === "focus" ? { terminalFallbackActive: state.terminalFallbackActive, activeSheet: state.activeSheet } : { terminalFallbackActive: state.terminalFallbackActive, mobileKeyboardOwnsFocus: document.activeElement === el.mobileKeyboardProxy })); }
function handleTerminalFallbackScroll() { const plan = terminalFallbackScrollPlan("scroll", { terminalFallbackActive: state.terminalFallbackActive, nearBottom: state.terminalFallbackActive ? terminalFallbackIsNearBottom() : false }); if (plan.updateAutoFollow) state.terminalFallbackAutoFollow = plan.autoFollow; }
function handleMobileKeyboardProxyFocusEvent(focused) { state.mobileKeyboardActive = focused; syncMobileKeyboardState(); forwardTerminalEvent({ kind: "focus", focused }); }
function handleTerminalFallbackMousedown() { handleTerminalFallbackPointerFocus("mousedown"); }
function handleTerminalFallbackClick() { handleTerminalFallbackPointerFocus("click"); }
function handleTerminalFallbackFocus() { handleTerminalFallbackFocusEvent("focus"); }
function handleTerminalFallbackBlur() { handleTerminalFallbackFocusEvent("blur"); }
function handleMobileKeyboardProxyFocus() { handleMobileKeyboardProxyFocusEvent(true); }
function handleMobileKeyboardProxyBlur() { handleMobileKeyboardProxyFocusEvent(false); }
function mouseCell(event) {
  const rect = el.terminalStage.getBoundingClientRect();
  return eventCell(event, rect, state.currentCols, state.currentRows);
}

function cellOffset(cell) {
  return cell.y * Math.max(1, state.currentCols) + cell.x;
}

const sendController = createSendController({
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
  storage: localStorage,
  WebSocketClass: WebSocket,
  sendHistoryKey: SEND_HISTORY_KEY,
  sendHistoryLimit: SEND_HISTORY_LIMIT,
  ElementClass: Element,
});

const {
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
} = sendController;

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

const commandPaletteController = createCommandPaletteController({
  state,
  el,
  documentRef: document,
  requestAnimationFrameRef: requestAnimationFrame,
  currentSession,
  copyTerminalFrameText,
  clampInt,
  selectSession,
  handleSurfaceAction,
  syncSheetActionAvailability,
  renderHudSurface,
  focusTerminalInputSurface,
  clearCreateBatchSelection,
  openCreateSheet,
  refreshThoughtConfig,
  refreshNativeStatus,
  refreshMermaidArtifact,
});

function renderCommandPalette() { return commandPaletteController.renderCommandPalette(); }

async function runCommandPaletteItem(item = state.paletteItems[state.paletteIndex]) { return commandPaletteController.runCommandPaletteItem(item); }

function openCommandPalette() { return commandPaletteController.openCommandPalette(); }

function setActiveSheet(sheetId) { return commandPaletteController.setActiveSheet(sheetId); }

function focusActiveSheet() { return commandPaletteController.focusActiveSheet(); }

function openSheet(sheetId) { return commandPaletteController.openSheet(sheetId); }

function closeSheets() { return commandPaletteController.closeSheets(); }

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
  const plan = surfaceActionPlan(zone);
  if (plan.type === "select_session") return selectSession(plan.sessionId);
  if (plan.type === "open_trogdor_agent_terminal") return openTrogdorAgentTerminal(plan.sessionId);
  if (plan.type === "trogdor_read_toggle" || plan.type === "trogdor_wpm") return runTrogdorReaderSurfaceAction(plan);
  if (plan.type === "toggle_trogdor_atlas") return toggleTrogdorAtlasSurfaceAction();
  if (plan.type === "focus_terminal") return focusTerminalSurfaceAction();
  return runSurfaceActionExecution(surfaceActionExecutionForZone(plan, zone));
}

function surfaceActionPlan(zone) {
  const contextPlan = surfaceActionDispatchContextPlan(zone);
  const planContext = {};
  if (contextPlan.includeReadOnly) planContext.readOnly = state.readOnly;
  if (contextPlan.includeCurrentSession) planContext.currentSession = currentSession();
  return surfaceActionDispatchPlan(zone, planContext);
}

function runTrogdorReaderSurfaceAction(plan) {
  advanceTrogdorReaderProgressForCurrentHover();
  const readerPlan = surfaceActionTrogdorReaderExecutionPlan(plan, plan.type === "trogdor_read_toggle"
    ? { toggle: trogdorReaderToggleAction(state.trogdorReading, currentTrogdorSurfaceSession(), trogdorClawgReadComplete) }
    : { nextWpm: trogdorReaderWpmForAction(plan.actionId, state.trogdorWpm) });
  if (readerPlan.session) startTrogdorReaderForSession(readerPlan.session, { readAgain: readerPlan.readAgain });
  Object.assign(state, readerPlan.statePatch);
  if (readerPlan.restartClock) state.trogdorReaderStartedAt = performance.now();
  if (readerPlan.resetAfterWpmChange) resetTrogdorReaderAfterWpmChange();
  renderHudSurface();
  if (readerPlan.syncReaderTimer) syncTrogdorReaderTimer();
}

function toggleTrogdorAtlasSurfaceAction() {
  Object.assign(state, trogdorAtlasTransitionState("toggle", state.trogdorAtlasOpen));
  renderHudSurface();
}

function surfaceActionExecutionForZone(plan, zone) {
  const executionContext = surfaceActionExecutionContextPlan(plan);
  return surfaceActionExecutionPlan(plan, executionContext.includeZonePayload ? { zonePayload: trogdorActionPayloadForZone(zone) } : {});
}

const surfaceActionExecutors = {
  open_send_sheet: (action) => openSendSheet(action.payload),
  open_create_sheet_for_cwd: (action) => openCreateSheetForCwd(action.cwd),
  select_then_open_mermaid: async (action) => { await selectSession(action.sessionId); openMermaidSheet(); },
  select_then_launch_commit: async (action) => { await selectSession(action.sessionId); await launchCommitGrok(); },
  open_sheet: (action) => openSheet(action.sheetId),
  open_thought_config: () => openThoughtConfigSheet(),
  open_native: () => openNativeSheet(),
  open_mermaid: () => openMermaidSheet(),
  launch_commit: () => launchCommitGrok(),
  toggle_follow: () => toggleFollowPublished(),
  toggle_select: () => setSelectMode(!state.selectMode),
  copy_selection: () => copyTerminalSelection(),
  refresh: () => refreshSessions(),
};

function runSurfaceActionExecution(action) {
  return surfaceActionExecutors[action.type]?.(action);
}

function focusTerminalSurfaceAction() {
  const focusPlan = surfaceActionFocusTerminalExecutionPlan(trogdorTerminalFocusStatus(currentSession()));
  Object.assign(state, trogdorAtlasTransitionState(focusPlan.atlasTransitionAction));
  renderHudSurface();
  focusTerminalInputSurface(focusPlan.focusOptions);
  setUtilityStatus(focusPlan.statusMessage, focusPlan.statusError, focusPlan.statusTimeoutMs);
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

function handleTerminalStageKeydown(event) {
  const globalShortcutHandled = handleGlobalShortcut(event);
  const shouldCaptureTerminalKey = !globalShortcutHandled && shouldCaptureKey(event);
  const plan = terminalStageKeydownPlan({ globalShortcutHandled, shouldCaptureKey: shouldCaptureTerminalKey, beginsResponse: shouldCaptureTerminalKey && keyBeginsTrogdorResponse(event) });
  if (plan.preventDefault) event.preventDefault();
  if (plan.markResponse) markTrogdorSessionsResponded([state.selectedSessionId]);
  if (plan.forwardKey) forwardTerminalKeyDown(event);
}

function handleTerminalStagePaste(event) {
  const action = terminalStagePasteExecutorPlan(terminalStagePastePlan(state.readOnly, event.clipboardData?.getData("text") ?? ""));
  if (action.preventDefault) event.preventDefault();
  if (action.sendText) sendTerminalText(action.text);
}

function handleTerminalStageFocusEvent(eventType) { runTerminalFocusAction(terminalStageFocusPlan(eventType, eventType === "focus" ? { activeSheet: state.activeSheet } : { mobileKeyboardOwnsFocus: document.activeElement === el.mobileKeyboardProxy })); }

function handleTerminalStageMouseDown(event) {
  const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
  const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
  if (!fallbackOwnsPointer && !hit.action && !hit.consume && state.terminal) updateHoveredLink(event);
  const plan = terminalStageMouseDownPlan({
    fallbackOwnsPointer,
    hit,
    hasTerminal: Boolean(state.terminal),
    modifierKey: event.metaKey || event.ctrlKey,
    hoveredLinkUrl: state.hoveredLinkUrl,
    selectMode: state.selectMode,
    button: event.button,
    readOnly: state.readOnly,
  });
  applyTerminalStageMousePlan(event, plan, hit);
}

function handleTerminalStageMouseUp(event) {
  const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
  const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
  if (!fallbackOwnsPointer && !hit.action && !hit.consume && state.terminal) updateHoveredLink(event);
  const plan = terminalStageMouseUpPlan({
    fallbackOwnsPointer,
    hit,
    hasTerminal: Boolean(state.terminal),
    modifierKey: event.metaKey || event.ctrlKey,
    hoveredLinkUrl: state.hoveredLinkUrl,
    selectMode: state.selectMode,
    selectionAnchor: state.selectionAnchor,
    button: event.button,
    readOnly: state.readOnly,
  });
  applyTerminalStageMousePlan(event, plan, hit);
}

function applyTerminalStageMousePlan(event, plan, hit) {
  if (plan.suppressClick) state.surfaceClickSuppressUntil = performance.now() + SURFACE_CLICK_SUPPRESS_MS;
  if (plan.preventDefault) event.preventDefault();
  if (plan.handleAction) {
    void handleSurfaceAction(plan.action);
  } else if (plan.openHoveredLink) {
    safeOpenUrl(state.hoveredLinkUrl);
  } else if (plan.startSelection) {
    const anchor = cellOffset(hit.cell);
    state.selectionAnchor = anchor;
    setTerminalSelectionRange(anchor, anchor);
  } else if (plan.completeSelection) {
    setTerminalSelectionRange(state.selectionAnchor, cellOffset(hit.cell));
    state.selectionAnchor = null;
  } else if (plan.forwardMouse) {
    forwardTerminalMouse(plan.mouseKind, clampInt(event.button, 0, 0, 2), hit, event);
  }
}

function handleTerminalStageMouseMove(event) {
  const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
  const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
  const plan = terminalStageMouseMovePlan({
    fallbackOwnsPointer, hit, hasTerminal: Boolean(state.terminal), selectMode: state.selectMode,
    selectionAnchor: state.selectionAnchor, buttons: event.buttons, readOnly: state.readOnly,
  });
  applyTerminalStageMouseMovePlan(event, plan, hit);
}

function applyTerminalStageMouseMovePlan(event, plan, hit) {
  if (plan.updateTrogdorSurface) updateHoveredTrogdorSurface(plan.trogdorZone);
  if (plan.clearHoveredLink) clearHoveredLink(true);
  if (plan.preventDefault) event.preventDefault();
  if (plan.updateSelectionRange) {
    setTerminalSelectionRange(state.selectionAnchor, cellOffset(hit.cell));
  }
  if (plan.updateHoveredLink) updateHoveredLink(event);
  if (plan.forwardMouse) forwardTerminalMouse("move", 0, hit, event);
}

function handleTerminalStageWheel(event) {
  const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
  const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
  const plan = terminalStageWheelPlan({
    fallbackOwnsPointer, hit, hasTerminal: Boolean(state.terminal),
    readOnly: state.readOnly, selectMode: state.selectMode,
  });
  applyTerminalStageWheelPlan(event, plan, hit);
}

function applyTerminalStageWheelPlan(event, plan, hit) {
  if (plan.preventDefault) event.preventDefault();
  if (plan.forwardWheel) {
    forwardTerminalEvent({
      kind: "wheel",
      x: hit.cell.x,
      y: hit.cell.y,
      dx: Math.round(event.deltaX),
      dy: Math.round(event.deltaY),
      mods: keyModifiers(event),
    });
  }
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

let trogdorEventBindings = null;

function bindTrogdorEvents() {
  if (!trogdorEventBindings) {
    trogdorEventBindings = createTrogdorEventBindings({
      elements: el,
      ElementClass: Element,
      handleSurfaceAction,
      openTrogdorAgentTerminal,
      openTrogdorAtlas,
      updateHoveredTrogdorSurface,
    });
  }
  trogdorEventBindings.bindTrogdorEvents();
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
  runGlobalShortcutAction(plan, globalShortcutRuntime);
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
  return terminalWorkbenchController.handleTerminalWorkbenchWidgetsClick(event);
}

function handleTerminalWorkbenchWidgetsLogEvent(event) {
  return terminalWorkbenchController.handleTerminalWorkbenchWidgetsLogEvent(event);
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

function handleDocumentCommandPaletteShortcut(event) { if ((event.ctrlKey || event.metaKey) && !event.altKey && event.code === "KeyK") { event.preventDefault(); openCommandPalette(); } }

function handleTerminalPaletteClick() { openCommandPalette(); }

function handleTerminalCopyFrameClick() { void copyTerminalFrameText(); }

function handleTerminalLinkOpenClick() { if (state.hoveredLinkUrl) safeOpenUrl(state.hoveredLinkUrl); }

function handleTerminalLinkCopyClick() { void copyHoveredLink(); }

function setTerminalZoomAndRefocus(nextZoom) { setTerminalZoom(nextZoom, { announce: true }); focusTerminalInputSurface({ preventScroll: true }); }

function handleTerminalZoomOutClick() { setTerminalZoomAndRefocus(state.terminalZoom - TERMINAL_ZOOM_STEP); }

function handleTerminalZoomResetClick() { setTerminalZoomAndRefocus(1); }

function handleTerminalZoomInClick() { setTerminalZoomAndRefocus(state.terminalZoom + TERMINAL_ZOOM_STEP); }

function handleTerminalMobileKeyboardClick() { if (state.mobileKeyboardActive) { closeMobileKeyboard(); focusTerminalInputSurface({ preventScroll: true }); return; } focusMobileKeyboard(); }

function handleTerminalTrogdorBackClick(event) { event.preventDefault(); openTrogdorAtlas(); }

function handleTerminalWorkbenchToggleClick() { setTerminalWorkbenchOpen(!state.terminalWorkbenchOpen); focusTerminalInputSurface({ preventScroll: true }); }

function handleTerminalWorkbenchRefreshClick() { void refreshAgentContextForSelectedSession({ force: true }); void refreshWorkbenchWidgetsForSelectedSession({ force: true }); focusTerminalInputSurface({ preventScroll: true }); }

function handleTerminalInputDockSubmit(event) { event.preventDefault(); void submitTerminalInputDock(); }

function handleTerminalInlineInputInput() { resizeTerminalInlineInput(); syncTerminalInputDock(); }

function handleTerminalKeyStripClick(event) { const action = terminalKeyStripClickExecutorPlan(terminalKeyStripClickPlan(event.type, event.target)); if (!action.sendKey) return; if (action.preventDefault) event.preventDefault(); sendTerminalControlKey(action.actionId); focusTerminalInputSurface({ preventScroll: true }); }

function handleModalRootKeydown(event) { if (event.key === "Escape") { event.preventDefault(); closeSheets(); } }

function handlePaletteSearchInput() { state.paletteIndex = 0; renderCommandPalette(); }

function handleSearchFormSubmit(event) { event.preventDefault(); closeSheets(); }

function handleTerminalSearchInput(event) { applySearchQuery(event.target.value); }

function handleSearchPrevButtonClick() { cycleSearchMatch(-1); }

function handleSearchNextButtonClick() { cycleSearchMatch(1); }

function handleSearchClearButtonClick() { el.terminalSearch.value = ""; applySearchQuery(""); }

function handleSendModeChange() { updateSendHint(); }

async function handleThoughtConfigFormSubmit(event) { await thoughtConfigSheet.handleFormSubmit(event); }

function handleThoughtConfigBackendChange() { thoughtConfigSheet.handleBackendChange(); }

function handleThoughtConfigOptionChange() { thoughtConfigSheet.handleOptionChange(); }

async function handleThoughtConfigTestButtonClick() { await thoughtConfigSheet.handleTestButtonClick(); }

async function handleNativeFormSubmit(event) { await nativeDesktopSheet.handleNativeFormSubmit(event); }

async function handleNativeRefreshButtonClick() { await nativeDesktopSheet.handleNativeRefreshButtonClick(); }

async function handleNativeOpenButtonClick() { await nativeDesktopSheet.handleNativeOpenButtonClick(); }

function handleNativeAppChange() { nativeDesktopSheet.handleNativeAppChange(); }

function handleNativeModeChange() { nativeDesktopSheet.handleNativeModeChange(); }

function handleSendCloseButtonClick() { state.sendTarget = null; closeSheets(); }

function handleSaveTokenButtonClick() { return handleAuthTokenButtonAction("save"); }

function handleClearTokenButtonClick() { return handleAuthTokenButtonAction("clear"); }

function handleCreateToolChange() { syncSheetActionAvailability(); }

function handleCreateRequestInput() { syncSheetActionAvailability(); }

async function handleMermaidRefreshButtonClick() { await refreshMermaidArtifact(); }

async function handleMermaidOpenButtonClick() { await openMermaidArtifactHost(); }

async function handleMermaidPlanTabsClick(event) { await mermaidArtifactController.handlePlanTabsClick(event); }

function handleTerminalStageFocus() { handleTerminalStageFocusEvent("focus"); }

function handleTerminalStageBlur() { handleTerminalStageFocusEvent("blur"); }

function handleTerminalStageMouseleave() { clearHoveredLink(true); updateHoveredTrogdorSurface(null); }

const eventListenerHandlers = {
  closeSheets, handleClearTokenButtonClick, handleCommandPaletteEvent, handleCreateBatchClearClick, handleCreateBatchVisibleAction, handleCreateCwdInput, handleCreateFormSubmit, handleCreateLaunchTargetChange, handleCreateRequestInput, handleCreateToolChange,
  handleDirCheckboxChange, handleDirsListClick, handleDirsLoadButtonClick, handleDirsManagedOnlyChange, handleDirsPathInput, handleDirsPathKeydown, handleDirsSearchInput, handleDirsSpawnHereClick, handleDirsUpButtonClick, handleDocumentCommandPaletteShortcut,
  handleMermaidOpenButtonClick, handleMermaidPlanTabsClick, handleMermaidRefreshButtonClick, handleMobileKeyboardProxyBlur, handleMobileKeyboardProxyFocus, handleMobileKeyboardProxyInput, handleMobileKeyboardProxyKeydown, handleModalRootKeydown,
  handleNativeAppChange, handleNativeFormSubmit, handleNativeModeChange, handleNativeOpenButtonClick, handleNativeRefreshButtonClick, handlePaletteSearchInput, handleSaveTokenButtonClick, handleSearchClearButtonClick, handleSearchFormSubmit, handleSearchNextButtonClick, handleSearchPrevButtonClick,
  handleSendCloseButtonClick, handleSendFormSubmit, handleSendHistoryClick, handleSendModeChange, handleTerminalCopyFrameClick, handleTerminalFallbackBlur, handleTerminalFallbackClick, handleTerminalFallbackFocus, handleTerminalFallbackKeyEvent, handleTerminalFallbackMousedown,
  handleTerminalFallbackPasteEvent, handleTerminalFallbackScroll, handleTerminalInlineInputFocus, handleTerminalInlineInputInput, handleTerminalInlineInputKeydown, handleTerminalInputDockSubmit, handleTerminalKeyStripClick, handleTerminalLinkCopyClick, handleTerminalLinkOpenClick,
  handleTerminalMobileKeyboardClick, handleTerminalPaletteClick, handleTerminalSearchInput, handleTerminalStageBlur, handleTerminalStageClick, handleTerminalStageFocus, handleTerminalStageKeydown, handleTerminalStageMouseDown, handleTerminalStageMouseMove, handleTerminalStageMouseUp,
  handleTerminalStageMouseleave, handleTerminalStagePaste, handleTerminalStageTouchEnd, handleTerminalStageWheel, handleTerminalTrogdorBackClick, handleTerminalWorkbenchRefreshClick, handleTerminalWorkbenchToggleClick, handleTerminalWorkbenchWidgetsClick,
  handleTerminalWorkbenchWidgetsLogEvent, handleTerminalZoomInClick, handleTerminalZoomOutClick, handleTerminalZoomResetClick, handleThoughtConfigBackendChange, handleThoughtConfigFormSubmit, handleThoughtConfigOptionChange, handleThoughtConfigTestButtonClick,
};

function bindEvents() {
  bindAppEvents({
    document,
    elements: el,
    handlers: eventListenerHandlers,
    bindTrogdorEvents,
    appEventListenerBindingPlan,
    terminalStageCaptureBindings,
    captureSurfaceAction,
    ResizeObserver,
    queueMeasureAndResizeSurface,
  });
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
