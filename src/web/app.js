import { buildSurfaceFrame } from "./rendered_surface.js";
import {
  authTokenButtonPlan, controlEventSessionPatchPlan, eventCell, initialStateBootPlan, lifecycleDeletedSessionPatchPlan,
  sheetActionAvailabilityPlan,
  surfaceActionDispatchContextPlan, surfaceActionDispatchPlan, surfaceActionExecutionContextPlan, surfaceActionExecutionPlan, surfaceActionFocusTerminalExecutionPlan, surfaceActionTrogdorReaderExecutionPlan,
  terminalDestroyStatePatch,
  terminalPaintProbeSchedulePlan, terminalPaintVerificationPlan, terminalPresentationPlan, terminalToolsAvailabilityPlan,
} from "./input_support.js";
import { createAppEventHandlers } from "./app_event_handlers.js";
import { createTerminalStageController } from "./terminal_stage_controller.js";
import { createTerminalFocusController } from "./terminal_focus.js";
import { createSendController } from "./send_controller.js";
import { createTerminalInputController } from "./terminal_input.js";
import { createTerminalZoomInputController } from "./terminal_zoom_input.js";
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
  createTerminalSurfaceController,
} from "./terminal_surface_controller.js";
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
} from "./terminal_protocol.js";
import {
  createDirBrowserController,
} from "./dir_browser_controller.js";
import {
  createCommandPaletteController,
} from "./command_palette_controller.js";
import {
  TROGDOR_DRAGON_TARGET,
  loadTrogdorReadProgress,
  normalizeTrogdorSessionId,
  trogdorDomActionCueKinds,
  trogdorHasActionCue,
  trogdorPrimaryActionCue,
  trogdorActionPayloadForZone,
  trogdorAtlasTransitionState,
  trogdorReaderToggleAction,
  trogdorReaderWpmForAction,
  trogdorSessionAwaitingUser,
  trogdorTerminalFocusStatus,
} from "./trogdor_logic.js";
import {
  createTrogdorStateHelpers,
} from "./trogdor_state.js";
import {
  createTrogdorSurfaceController,
} from "./trogdor_surface_controller.js";
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

let trogdorSurfaceController;
function renderTrogdorSurface() { return trogdorSurfaceController.renderTrogdorSurface(); }
function applyTrogdorAtlasVisibility() { return trogdorSurfaceController.applyTrogdorAtlasVisibility(); }
function updateHoveredTrogdorSurface(zone) { return trogdorSurfaceController.updateHoveredTrogdorSurface(zone); }
function syncTrogdorReaderTimer() { return trogdorSurfaceController.syncTrogdorReaderTimer(); }

const {
  advanceTrogdorReaderProgressForCurrentHover,
  currentTrogdorSurfaceSession,
  markTrogdorSessionsResponded,
  rawSessionAwaitingUser,
  resetTrogdorReaderAfterWpmChange,
  startTrogdorReaderForSession,
  syncTrogdorCueTransitions,
  trogdorClawgReadComplete,
  trogdorReaderWordIndex,
  trogdorSessionBurnt,
  trogdorSessionCanRead,
} = createTrogdorStateHelpers({
  state,
  operatorPressureSnapshot,
  surfaceSession,
  renderHudSurface: (...args) => renderHudSurface(...args),
  syncTrogdorReaderTimer,
  performanceRef: performance,
  windowRef: window,
  burnMs: TROGDOR_BURN_MS,
});

const defaultDocumentTitle = document.title || "swimmers";
let terminalZoomInputController;
let loadFrankenTermFont;
let ensureFrankenTerm;
let setupHudSurface;
let destroyTerminalInstance;
let clearTerminalPaintProbe;
let teardownTerminal;
let disconnectSocket;
let surfaceBusy;
let withSurfaceOperation;
let queueRenderRetry;
let queueHudRender;
let queueMeasureAndResizeSurface;
let scheduleRender;
let sendResize;
let measureAndResizeSurface;
let captureTerminalRendererDiagnostic;
let buildSurfaceModel;
let renderHudSurface;
let syncTerminalPresentation;
let feedTerminalBytes;
let scheduleTerminalPaintProbe;
let terminalPaintVerificationContext;
let applyTerminalPaintVerificationPlan;
let verifyTerminalPaintOrFallback;
let terminalCanvasHasVisiblePixels;

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
  renderHudSurface: (...args) => renderHudSurface(...args),
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
  renderHudSurface: (...args) => renderHudSurface(...args),
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
  scheduleRender: (...args) => scheduleRender(...args),
  renderHudSurface: (...args) => renderHudSurface(...args),
  setSearchStatus,
  setUtilityStatus,
  defaultUtilityLabel,
  shortenUrl,
  currentSession,
  frankenTermLinkPolicy,
  surfaceBusy: (...args) => surfaceBusy(...args),
  withSurfaceOperation: (...args) => withSurfaceOperation(...args),
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

const terminalInputController = createTerminalInputController({
  state,
  el,
  WebSocketClass: WebSocket,
  windowRef: window,
  maxTerminalPasteBytes: MAX_TERMINAL_PASTE_BYTES,
  currentSession,
  nextInputMessageId,
  updateInputDeliveryStatus,
  rejectOversizeTerminalText,
  setUtilityStatus,
  setTerminalInputEcho,
  markTrogdorSessionsResponded,
  terminalSupports,
  drainTerminalLinkClicks,
});

const {
  flushEncodedInputBytes,
  forwardTerminalEvent,
  forwardTerminalKeyDown,
  forwardTerminalMouse,
  handleInputAck,
  sendTerminalControlKey,
  sendTerminalInputText,
  sendTerminalText,
  terminalKeyActionForDomEvent,
} = terminalInputController;

const terminalFocusController = createTerminalFocusController({
  state,
  el,
  documentRef: document,
  windowRef: window,
  requestAnimationFrameRef: requestAnimationFrame,
  currentSession,
  forwardTerminalEvent,
  forwardTerminalKeyDown,
  handleGlobalShortcut: (event) => handleGlobalShortcut(event),
  keyBeginsTrogdorResponse,
  markTrogdorSessionsResponded,
  sendTerminalText,
  terminalFallbackIsNearBottom: () => terminalFallbackIsNearBottom(),
});

const {
  closeMobileKeyboard,
  focusMobileKeyboard,
  focusTerminalInputSurface,
  handleMobileKeyboardProxyFocusEvent,
  handleTerminalFallbackBlur,
  handleTerminalFallbackClick,
  handleTerminalFallbackFocus,
  handleTerminalFallbackKeyEvent,
  handleTerminalFallbackMousedown,
  handleTerminalFallbackPasteEvent,
  handleTerminalFallbackScroll,
  handleTerminalInlineInputFocus,
  handleTerminalStageFocusEvent,
  isCoarsePointer,
  shouldCaptureKey,
} = terminalFocusController;

const terminalSurfaceRuntime = {
  state,
  el,
  requiredTerminalMethods: FRANKENTERM_TERMINAL_METHODS,
  maxPendingTerminalBytes: MAX_PENDING_TERMINAL_BYTES,
  validateFrankenTermSurface,
  teardownTerminal: (...args) => teardownTerminal(...args),
  destroyTerminalInstance: (...args) => destroyTerminalInstance(...args),
  currentSession,
  ensureFrankenTerm: (...args) => ensureFrankenTerm(...args),
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
  measureAndResizeSurface: (...args) => measureAndResizeSurface(...args),
  feedTerminalBytes: (...args) => feedTerminalBytes(...args),
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
  setupHudSurface: (...args) => setupHudSurface(...args),
  setupTerminalSurface,
  teardownTerminal: (...args) => teardownTerminal(...args),
  disconnectSocket: (...args) => disconnectSocket(...args),
  measureAndResizeSurface: (...args) => measureAndResizeSurface(...args),
  scheduleSessionRefresh,
  reconnectDelayMs,
  setConnectionStatus,
  setModeStatus,
  syncWriteAccess,
  syncTerminalTools,
  feedTerminalBytes: (...args) => feedTerminalBytes(...args),
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
  surfaceBusy: (...args) => surfaceBusy(...args),
  queueMeasureAndResizeSurface: (...args) => queueMeasureAndResizeSurface(...args),
  withSurfaceOperation: (...args) => withSurfaceOperation(...args),
  renderHudSurface: (...args) => renderHudSurface(...args),
  scheduleRender: (...args) => scheduleRender(...args),
  sendResize: (...args) => sendResize(...args),
  captureTerminalRendererDiagnostic: (...args) => captureTerminalRendererDiagnostic(...args),
  devicePixelRatio: () => window.devicePixelRatio || 1,
};

({
  loadFrankenTermFont,
  ensureFrankenTerm,
  setupHudSurface,
  destroyTerminalInstance,
  clearTerminalPaintProbe,
  teardownTerminal,
  disconnectSocket,
  surfaceBusy,
  withSurfaceOperation,
  queueRenderRetry,
  queueHudRender,
  queueMeasureAndResizeSurface,
  scheduleRender,
  sendResize,
  measureAndResizeSurface,
  captureTerminalRendererDiagnostic,
  buildSurfaceModel,
  renderHudSurface,
  syncTerminalPresentation,
  feedTerminalBytes,
  scheduleTerminalPaintProbe,
  terminalPaintVerificationContext,
  applyTerminalPaintVerificationPlan,
  verifyTerminalPaintOrFallback,
  terminalCanvasHasVisiblePixels,
} = createTerminalSurfaceController({
  state,
  el,
  boot,
  hudMethods: FRANKENTERM_HUD_METHODS,
  assertFrankenTermModule,
  canvasHasVisiblePixels,
  formatFrankenTermAssetSummary,
  runtimeSurfaceBusy,
  surfaceSupports,
  validateFrankenTermSurface,
  runSurfaceOperation,
  runTerminalSurfaceResize,
  terminalResizeRuntime,
  terminalDestroyStatePatch,
  terminalPaintProbeSchedulePlan,
  terminalPaintVerificationPlan,
  terminalPresentationPlan,
  buildSurfaceFrame,
  buildSurfaceModelFromState,
  currentSession,
  operatorPressureSnapshot,
  sessionBurnt: trogdorSessionBurnt,
  normalizeSessionId,
  terminalSupports,
  clearReconnectTimer,
  clearHoveredLink,
  clearPendingTerminalBytes,
  bufferTerminalBytes,
  flushEncodedInputBytes,
  setTerminalTextFallbackActive,
  syncTerminalTools,
  stopSnapshotPolling,
  applyZoomToSurface,
  setLoadingState,
  renderTrogdorSurface,
  advanceTrogdorReaderProgressForCurrentHover,
  syncTerminalInputDock,
  syncTrogdorBackButton,
  syncTerminalWorkbench,
  refreshTerminalSearch,
  drainTerminalLinkClicks,
  syncTerminalAccessibilityMirror,
  syncTerminalFallbackFromLiveFrame,
  refreshSnapshotFallback,
  windowRef: window,
  documentRef: document,
  URLImpl: URL,
  WebSocketClass: WebSocket,
  Uint8ArrayClass: Uint8Array,
  importModule: (url) => import(url),
  requestAnimationFrameRef: (callback) => requestAnimationFrame(callback),
  setTimeoutRef: (callback, delay) => window.setTimeout(callback, delay),
  clearTimeoutRef: (timer) => window.clearTimeout(timer),
  prefersReducedMotion: () => window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false,
  isoTimestamp: () => new Date().toISOString(),
  now: () => performance.now(),
}));

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
  return terminalZoomInputController.terminalZoomSupported();
}

function normalizeTerminalZoom(value) {
  return terminalZoomInputController.normalizeTerminalZoom(value);
}

function loadTerminalZoom(url) {
  return terminalZoomInputController.loadTerminalZoom(url);
}

function syncTerminalZoomControls() {
  return terminalZoomInputController.syncTerminalZoomControls();
}

function syncTerminalInputDock() {
  return terminalZoomInputController.syncTerminalInputDock();
}

function resizeTerminalInlineInput() {
  return terminalZoomInputController.resizeTerminalInlineInput();
}

function setTerminalInputEcho(text) {
  return terminalZoomInputController.setTerminalInputEcho(text);
}

function projectTerminalInputIntoFallback(text) {
  return terminalZoomInputController.projectTerminalInputIntoFallback(text);
}

async function submitTerminalInputDock() {
  return terminalZoomInputController.submitTerminalInputDock();
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
  return terminalZoomInputController.applyZoomToSurface(surface);
}

function persistTerminalZoomToUrl(plan) {
  return terminalZoomInputController.persistTerminalZoomToUrl(plan);
}

function applyTerminalZoom(options = {}) {
  return terminalZoomInputController.applyTerminalZoom(options);
}

function setTerminalZoom(nextZoom, options = {}) {
  return terminalZoomInputController.setTerminalZoom(nextZoom, options);
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

terminalZoomInputController = createTerminalZoomInputController({
  state,
  el,
  storage: localStorage,
  windowRef: window,
  documentRef: document,
  URLImpl: URL,
  surfaceSupports,
  terminalSupports,
  currentSession,
  updateTerminalFallbackText,
  sendLineToSession,
  rememberSendHistory,
  refreshSessions,
  setConnectionStatus,
  setUtilityStatus,
  measureAndResizeSurface,
  focusTerminalInputSurface,
  terminalZoomStorageKey: TERMINAL_ZOOM_STORAGE_KEY,
  minZoom: TERMINAL_ZOOM_MIN,
  maxZoom: TERMINAL_ZOOM_MAX,
  step: TERMINAL_ZOOM_STEP,
});

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

trogdorSurfaceController = createTrogdorSurfaceController({
  state,
  el,
  documentRef: document,
  windowRef: window,
  surfaceSession,
  currentTrogdorSurfaceSession,
  trogdorSessionCanRead,
  trogdorClawgReadComplete,
  trogdorReaderWordIndex,
  startTrogdorReaderForSession,
  renderHudSurface: (...args) => renderHudSurface(...args),
  setUtilityStatus,
  clampInt,
});

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

const terminalStageController = createTerminalStageController({
  state,
  el,
  ElementClass: Element,
  performanceRef: performance,
  surfaceClickSuppressMs: SURFACE_CLICK_SUPPRESS_MS,
  mouseCell,
  cellOffset,
  clampInt,
  handleSurfaceAction,
  handleGlobalShortcut: (...args) => handleGlobalShortcut(...args),
  shouldCaptureKey,
  keyBeginsTrogdorResponse,
  markTrogdorSessionsResponded,
  forwardTerminalKeyDown,
  sendTerminalText,
  updateHoveredLink,
  clearHoveredLink,
  safeOpenUrl,
  setTerminalSelectionRange,
  forwardTerminalMouse,
  forwardTerminalEvent,
  updateHoveredTrogdorSurface,
  focusMobileKeyboard,
  focusTerminalInputSurface,
  isCoarsePointer,
});

const {
  captureSurfaceAction,
  handleTerminalStageClick,
  handleTerminalStageKeydown,
  handleTerminalStageMouseDown,
  handleTerminalStageMouseMove,
  handleTerminalStageMouseUp,
  handleTerminalStagePaste,
  handleTerminalStageTouchEnd,
  handleTerminalStageWheel,
  terminalFallbackOwnsPointer,
} = terminalStageController;

const {
  bindEvents,
  handleCommandPaletteEvent,
  handleGlobalShortcut,
  handleMobileKeyboardProxyInput,
  handleMobileKeyboardProxyKeydown,
  handleTerminalInlineInputKeydown,
  handleTerminalWorkbenchWidgetsClick,
} = createAppEventHandlers({
  documentRef: document,
  elements: el,
  state,
  ElementClass: Element,
  ResizeObserverCtor: globalThis.ResizeObserver,
  applySearchQuery,
  captureSurfaceAction,
  clearHoveredLink,
  closeMobileKeyboard,
  closeSheets,
  copyHoveredLink,
  copyTerminalFrameText,
  currentSession,
  cycleSearchMatch,
  focusMobileKeyboard,
  focusTerminalInputSurface,
  forwardTerminalEvent,
  forwardTerminalKeyDown,
  globalShortcutRuntime,
  handleAuthTokenButtonAction,
  handleCreateBatchClearClick,
  handleCreateBatchVisibleAction,
  handleCreateCwdInput,
  handleCreateFormSubmit,
  handleCreateLaunchTargetChange,
  handleDirCheckboxChange,
  handleDirsListClick,
  handleDirsLoadButtonClick,
  handleDirsManagedOnlyChange,
  handleDirsPathInput,
  handleDirsPathKeydown,
  handleDirsSearchInput,
  handleDirsSpawnHereClick,
  handleDirsUpButtonClick,
  handleMobileKeyboardProxyFocusEvent,
  handleSendFormSubmit,
  handleSendHistoryClick,
  handleSurfaceAction,
  handleTerminalFallbackBlur,
  handleTerminalFallbackClick,
  handleTerminalFallbackFocus,
  handleTerminalFallbackKeyEvent,
  handleTerminalFallbackMousedown,
  handleTerminalFallbackPasteEvent,
  handleTerminalFallbackScroll,
  handleTerminalInlineInputFocus,
  handleTerminalStageClick,
  handleTerminalStageFocusEvent,
  handleTerminalStageKeydown,
  handleTerminalStageMouseDown,
  handleTerminalStageMouseMove,
  handleTerminalStageMouseUp,
  handleTerminalStagePaste,
  handleTerminalStageTouchEnd,
  handleTerminalStageWheel,
  keyBeginsTrogdorResponse,
  markTrogdorSessionsResponded,
  mermaidArtifactController,
  nativeDesktopSheet,
  openCommandPalette,
  openMermaidArtifactHost,
  openTrogdorAgentTerminal,
  openTrogdorAtlas,
  queueMeasureAndResizeSurface,
  refreshAgentContextForSelectedSession,
  refreshMermaidArtifact,
  refreshWorkbenchWidgetsForSelectedSession,
  renderCommandPalette,
  runCommandPaletteItem,
  safeOpenUrl,
  sendTerminalControlKey,
  sendTerminalText,
  setTerminalWorkbenchOpen,
  submitTerminalInputDock,
  syncSheetActionAvailability,
  terminalKeyActionForDomEvent,
  terminalWorkbenchController,
  terminalZoomInputController,
  thoughtConfigSheet,
  updateHoveredTrogdorSurface,
  updateSendHint,
});

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
