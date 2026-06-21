import {
  trogdorDomActionCueKinds,
  trogdorHasActionCue,
  trogdorSessionIsSleepingOrDeepSleep,
} from "./trogdor_dom_logic.js";

export {
  TROGDOR_DRAGON_TARGET,
  buildTrogdorDomGroups,
  summarizeTrogdorDom,
  trogdorAgentGlyph,
  trogdorAgentTone,
  trogdorDomActionCueKinds,
  trogdorDomPressure,
  trogdorDomReason,
  trogdorDragonFrameForVector,
  trogdorDragonPose,
  trogdorHasActionCue,
  trogdorPrimaryActionCue,
  trogdorSessionIsSleepingOrDeepSleep,
} from "./trogdor_dom_logic.js";

export const TROGDOR_READ_PROGRESS_STORAGE_KEY = "swimmers.web.trogdor.readProgress";

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

export function stableTextHash(text) {
  let hash = 5381;
  for (let index = 0; index < text.length; index += 1) {
    hash = ((hash << 5) + hash + text.charCodeAt(index)) >>> 0;
  }
  return hash.toString(36);
}

export function rawActionCueKinds(session) {
  return (Array.isArray(session?.action_cues) ? session.action_cues : [])
    .map((cue) => String(cue?.kind || "").toLowerCase())
    .filter(Boolean);
}

export function rawHasActionCue(session, kind) {
  return rawActionCueKinds(session).includes(kind);
}

export function rawTrogdorSessionAwaitingUser(session, operatorPressure = null) {
  const pressure = operatorPressure?.pressure || operatorPressure || {};
  const reasonKind = String(pressure.reason_kind || "").toLowerCase();
  const stateLabel = String(session?.state || "").toLowerCase();
  return rawHasActionCue(session, "awaiting_user") || reasonKind === "awaiting_user" || stateLabel === "attention";
}

export function rawSessionIsSleepingOrDeepSleep(session) {
  const rest = String(session?.rest_state || "").toLowerCase();
  return rest === "sleeping" || rest === "deep_sleep";
}

export function trogdorClawgText(session) {
  return String(session?.clawgText || session?.thoughtLabel || session?.commandLabel || session?.name || "waiting");
}

export function trogdorClawgWords(session) {
  return trogdorClawgText(session)
    .split(/\s+/)
    .map((word) => word.trim())
    .filter(Boolean);
}

export function trogdorClawgKey(session) {
  const sessionId = String(session?.sessionId || "");
  if (!sessionId) {
    return "";
  }
  const updated = String(session?.thoughtUpdatedAt || session?.objectiveChangedAt || "");
  const text = trogdorClawgText(session);
  return `${sessionId}:${updated}:${stableTextHash(text)}`;
}

// Each thought update mints a fresh read-progress key (sessionId:updated:hash),
// so without a bound the persisted map grows forever until the localStorage
// quota throws and progress silently stops saving. Cap to the most recently
// written entries (object key order is insertion order, oldest first).
export const MAX_TROGDOR_READ_PROGRESS_ENTRIES = 500;

export function parseTrogdorReadProgress(raw) {
  try {
    const parsed = typeof raw === "string" ? JSON.parse(raw || "{}") : raw;
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
    const keys = Object.keys(progress);
    if (keys.length <= MAX_TROGDOR_READ_PROGRESS_ENTRIES) {
      return progress;
    }
    const bounded = {};
    for (const key of keys.slice(keys.length - MAX_TROGDOR_READ_PROGRESS_ENTRIES)) {
      bounded[key] = progress[key];
    }
    return bounded;
  } catch (_error) {
    return {};
  }
}

export function serializeTrogdorReadProgress(progress) {
  return JSON.stringify(parseTrogdorReadProgress(progress || {}));
}

export function loadTrogdorReadProgress(
  storage = globalThis.localStorage,
  key = TROGDOR_READ_PROGRESS_STORAGE_KEY,
) {
  if (!storage) {
    return {};
  }
  try {
    return parseTrogdorReadProgress(storage.getItem(key));
  } catch (_error) {
    return {};
  }
}

export function saveTrogdorReadProgress(
  progress,
  storage = globalThis.localStorage,
  key = TROGDOR_READ_PROGRESS_STORAGE_KEY,
) {
  if (!storage) {
    return false;
  }
  try {
    storage.setItem(key, serializeTrogdorReadProgress(progress || {}));
    return true;
  } catch (_error) {
    return false;
  }
}

export function trogdorClawgReadIndexForProgress(session, progress = {}) {
  const words = trogdorClawgWords(session);
  const key = trogdorClawgKey(session);
  if (!key) {
    return 0;
  }
  return clampInt(progress?.[key], 0, 0, words.length);
}

export function setTrogdorClawgReadIndexForProgress(progress = {}, session, index) {
  const key = trogdorClawgKey(session);
  if (!key) {
    return { progress: progress || {}, changed: false, index: 0 };
  }
  const words = trogdorClawgWords(session);
  const nextIndex = clampInt(index, 0, 0, words.length);
  if (progress?.[key] === nextIndex) {
    return { progress, changed: false, index: nextIndex };
  }
  return {
    progress: {
      ...(progress || {}),
      [key]: nextIndex,
    },
    changed: true,
    index: nextIndex,
  };
}

export function trogdorClawgReadCompleteForProgress(session, progress = {}) {
  const words = trogdorClawgWords(session);
  return words.length > 0 && trogdorClawgReadIndexForProgress(session, progress) >= words.length;
}

export function normalizeTrogdorSessionId(sessionId) {
  const trimmed = typeof sessionId === "string" ? sessionId.trim() : "";
  return trimmed || null;
}

export function normalizeTrogdorSessionIds(sessionIds) {
  return Array.isArray(sessionIds) ? sessionIds.map(normalizeTrogdorSessionId).filter(Boolean) : [];
}

export function trogdorReaderBaseIndexForProgress(
  session,
  {
    readerClawgKey = "",
    readerStartIndex = 0,
    progress = {},
  } = {},
) {
  const words = trogdorClawgWords(session);
  const key = trogdorClawgKey(session);
  if (key && key === readerClawgKey) {
    return clampInt(readerStartIndex, 0, 0, words.length);
  }
  return trogdorClawgReadIndexForProgress(session, progress);
}

export function trogdorReaderWordIndexForProgress(
  session,
  {
    wpm = 200,
    readerClawgKey = "",
    readerStartIndex = 0,
    progress = {},
    reading = true,
    hoveredSessionId = "",
    readerStartedAt = 0,
    now = 0,
  } = {},
) {
  const words = trogdorClawgWords(session);
  if (!words.length) {
    return -1;
  }
  const baseIndex = trogdorReaderBaseIndexForProgress(session, {
    readerClawgKey,
    readerStartIndex,
    progress,
  });
  if (baseIndex >= words.length) {
    return words.length;
  }
  if (reading === false) {
    return baseIndex;
  }
  const elapsed = hoveredSessionId ? Math.max(0, now - readerStartedAt) : 0;
  const msPerWord = Math.max(60, 60000 / Math.max(1, wpm));
  return Math.min(words.length, baseIndex + Math.floor(elapsed / msPerWord));
}

export function trogdorReaderDisplayState(
  session,
  {
    wordIndex = -1,
    progress = {},
    emptyText = "burninate!",
    waitingText = "waiting",
    caughtUpText = "caught up",
    maxWordChars = 22,
  } = {},
) {
  if (!session) {
    return { bannerText: emptyText, readComplete: false };
  }
  const words = trogdorClawgWords(session);
  if (!words.length) {
    return { bannerText: waitingText, readComplete: false };
  }
  const index = clampInt(wordIndex, 0, 0, words.length);
  return {
    bannerText: index >= words.length
      ? caughtUpText
      : words[index].slice(0, clampInt(maxWordChars, 22, 1, 200)),
    readComplete: trogdorClawgReadCompleteForProgress(session, progress),
  };
}

export function trogdorReaderTimerAction(
  session,
  sessionCanRead = () => false,
  readComplete = () => false,
  reading = true,
  timerActive = false,
) {
  const shouldRun = Boolean(
    session && sessionCanRead(session) && reading && !readComplete(session),
  );
  if (shouldRun && !timerActive) {
    return "start";
  }
  if (!shouldRun && timerActive) {
    return "stop";
  }
  return "keep";
}

export function trogdorReaderToggleAction(
  reading = true,
  session = null,
  readComplete = () => false,
) {
  if (reading !== false) {
    return { session: null, reading: false, readAgain: false, restartClock: false };
  }
  if (!session) {
    return { session: null, reading: true, readAgain: false, restartClock: true };
  }
  return {
    session,
    reading: null,
    readAgain: Boolean(readComplete(session)),
    restartClock: true,
  };
}

export function trogdorReaderWpmForAction(
  actionId,
  wpm,
  {
    step = 25,
    fallback = 200,
    min = 50,
    max = 800,
  } = {},
) {
  return clampInt(
    actionId === "trogdor_wpm_down" ? wpm - step : wpm + step,
    fallback,
    min,
    max,
  );
}

export function trogdorReaderProgressAdvanceForSession(
  session,
  {
    wordIndex = -1,
    reading = true,
  } = {},
) {
  const words = trogdorClawgWords(session);
  if (reading === false || !words.length || wordIndex < 0) {
    return {
      shouldAdvance: false,
      nextReadIndex: 0,
      reading,
      complete: false,
    };
  }
  const nextReadIndex = Math.min(words.length, wordIndex + 1);
  const complete = nextReadIndex >= words.length;
  return {
    shouldAdvance: true,
    nextReadIndex,
    reading: complete ? false : reading,
    complete,
  };
}

export function trogdorReaderStateForWpmChange(
  session,
  {
    currentStartIndex = 0,
    progress = {},
    now = 0,
  } = {},
) {
  return {
    trogdorReaderStartIndex: session
      ? trogdorClawgReadIndexForProgress(session, progress)
      : currentStartIndex,
    trogdorReaderStartedAt: now,
  };
}

export function startTrogdorReaderStateForSession(
  session,
  {
    readAgain = false,
    dismissedClawgs = {},
    progress = {},
    now = 0,
  } = {},
) {
  const words = trogdorClawgWords(session);
  const key = trogdorClawgKey(session);
  let nextDismissedClawgs = dismissedClawgs || {};
  let nextProgress = progress || {};
  let progressChanged = false;

  if (readAgain && key) {
    nextDismissedClawgs = clearTrogdorDismissedClawgInMap(nextDismissedClawgs, session).dismissedClawgs;
    const nextReadIndex = setTrogdorClawgReadIndexForProgress(nextProgress, session, 0);
    nextProgress = nextReadIndex.progress;
    progressChanged = nextReadIndex.changed;
  }

  const startIndex = readAgain ? 0 : trogdorClawgReadIndexForProgress(session, nextProgress);
  const readerStartIndex = clampInt(startIndex, 0, 0, words.length);
  return {
    readerClawgKey: key,
    readerStartIndex,
    readerStartedAt: now,
    reading: readerStartIndex < words.length,
    dismissedClawgs: nextDismissedClawgs,
    progress: nextProgress,
    progressChanged,
  };
}

export function trogdorClawgDismissedForMap(session, dismissedClawgs = {}) {
  const key = trogdorClawgKey(session);
  return Boolean(key && dismissedClawgs?.[key]);
}

export function dismissTrogdorClawgInMap(dismissedClawgs = {}, session) {
  const key = trogdorClawgKey(session);
  if (!key) {
    return { dismissedClawgs: dismissedClawgs || {}, changed: false, key: "" };
  }
  return {
    dismissedClawgs: {
      ...(dismissedClawgs || {}),
      [key]: true,
    },
    changed: !dismissedClawgs?.[key],
    key,
  };
}

export function clearTrogdorDismissedClawgInMap(dismissedClawgs = {}, session) {
  const key = trogdorClawgKey(session);
  if (!key) {
    return { dismissedClawgs: dismissedClawgs || {}, changed: false, key: "" };
  }
  const { [key]: _dismissed, ...remainingDismissed } = dismissedClawgs || {};
  return {
    dismissedClawgs: remainingDismissed,
    changed: Boolean(dismissedClawgs?.[key]),
    key,
  };
}

export function markTrogdorSessionsRespondedState({
  sessionIds = [],
  sessions = [],
  toSurfaceSession = (session) => session,
  dismissedClawgs = {},
  progress = {},
  hoveredSessionId = "",
} = {}) {
  const ids = normalizeTrogdorSessionIds(sessionIds);
  let nextDismissedClawgs = dismissedClawgs || {};
  let nextProgress = progress || {};
  let progressChanged = false;
  const burntIds = [];

  for (const sessionId of ids) {
    const raw = sessions.find((item) => item?.session_id === sessionId);
    if (!raw) {
      continue;
    }
    const session = toSurfaceSession(raw);
    if (!trogdorSessionAwaitingUser(session)) {
      continue;
    }

    nextDismissedClawgs = dismissTrogdorClawgInMap(nextDismissedClawgs, session).dismissedClawgs;
    const completed = setTrogdorClawgReadIndexForProgress(
      nextProgress,
      session,
      trogdorClawgWords(session).length,
    );
    nextProgress = completed.progress;
    progressChanged = progressChanged || completed.changed;
    burntIds.push(sessionId);
  }

  return {
    burntIds,
    dismissedClawgs: nextDismissedClawgs,
    progress: nextProgress,
    progressChanged,
    resetReader: burntIds.includes(normalizeTrogdorSessionId(hoveredSessionId)),
  };
}

export function trogdorSessionBurntInMap(burntSessions = new Map(), sessionOrId, now = 0) {
  const sessionId = normalizeTrogdorSessionId(
    typeof sessionOrId === "string" ? sessionOrId : sessionOrId?.sessionId,
  );
  const until = sessionId ? burntSessions.get(sessionId) : null;
  if (!until) {
    return { burnt: false, burntSessions, changed: false };
  }
  if (until <= now) {
    const nextBurntSessions = new Map(burntSessions);
    nextBurntSessions.delete(sessionId);
    return { burnt: false, burntSessions: nextBurntSessions, changed: true };
  }
  return { burnt: true, burntSessions, changed: false };
}

export function pruneTrogdorBurntSessionMap(burntSessions = new Map(), now = 0) {
  let nextBurntSessions = burntSessions;
  let changed = false;
  for (const [sessionId, until] of burntSessions.entries()) {
    if (until <= now) {
      if (!changed) {
        nextBurntSessions = new Map(burntSessions);
      }
      nextBurntSessions.delete(sessionId);
      changed = true;
    }
  }
  return { burntSessions: nextBurntSessions, changed };
}

export function markTrogdorBurntSessionsInMap(
  burntSessions = new Map(),
  sessionIds = [],
  now = 0,
  burnMs = 0,
) {
  const ids = normalizeTrogdorSessionIds(sessionIds);
  if (!ids.length) {
    return { ids, burntSessions, changed: false, until: now + burnMs };
  }
  const nextBurntSessions = new Map(burntSessions);
  const until = now + burnMs;
  for (const sessionId of ids) {
    nextBurntSessions.set(sessionId, until);
  }
  return { ids, burntSessions: nextBurntSessions, changed: true, until };
}

export function trogdorCueTransitionState({
  sessions = [],
  previousAwaitingSessionIds = new Set(),
  hoveredSessionId = "",
  rawAwaitingUser = rawTrogdorSessionAwaitingUser,
  rawSleepingOrDeepSleep = rawSessionIsSleepingOrDeepSleep,
  sessionBurnt = () => false,
} = {}) {
  const awaitingSessionIds = new Set();
  for (const session of sessions) {
    if (rawAwaitingUser(session)) {
      awaitingSessionIds.add(String(session.session_id));
    }
  }

  const burntIds = [];
  for (const sessionId of previousAwaitingSessionIds || []) {
    if (!awaitingSessionIds.has(sessionId)) {
      burntIds.push(sessionId);
    }
  }

  const hovered = normalizeTrogdorSessionId(hoveredSessionId);
  let resetReader = false;
  if (hovered) {
    const raw = sessions.find((session) => session.session_id === hovered);
    resetReader =
      !raw ||
      (!rawAwaitingUser(raw) && !rawSleepingOrDeepSleep(raw) && !sessionBurnt(hovered));
  }

  return {
    awaitingSessionIds,
    burntIds,
    resetReader,
  };
}

export function trogdorSessionAwaitingUser(session) {
  const reasonKind = String(session?.operatorPressure?.reason_kind || "").toLowerCase();
  const stateLabel = String(session?.state || "").toLowerCase();
  return trogdorHasActionCue(session, "awaiting_user") || reasonKind === "awaiting_user" || stateLabel === "attention";
}

export function trogdorSessionHasReadyClawg(session) {
  const reasonKind = String(session?.operatorPressure?.reason_kind || "").toLowerCase();
  return (
    trogdorDomActionCueKinds(session).length > 0 ||
    ["awaiting_user", "commit_ready", "validation_missing_after_edit", "dirty_check_missing"].includes(reasonKind) ||
    String(session?.state || "").toLowerCase() === "attention"
  );
}

export function trogdorSwordsmanVisibleForState(
  session,
  {
    burnt = false,
    dismissed = false,
  } = {},
) {
  if (burnt) {
    return true;
  }
  return (
    (trogdorSessionHasReadyClawg(session) && !dismissed) ||
    trogdorSessionIsSleepingOrDeepSleep(session)
  );
}

export function trogdorSessionCanReadForState(
  session,
  {
    burnt = false,
    dismissed = false,
  } = {},
) {
  return !burnt && trogdorSwordsmanVisibleForState(session, { burnt, dismissed });
}

export function trogdorHoverReaderResetState(hoveredSessionId = null) {
  return {
    hoveredTrogdorSessionId: hoveredSessionId,
    trogdorReaderStartedAt: 0,
    trogdorReaderStartIndex: 0,
    trogdorReaderClawgKey: "",
  };
}

export function trogdorAtlasTransitionState(action, atlasOpen = false) {
  switch (action) {
    case "open":
      return { trogdorAtlasOpen: true, trogdorSurfaceSignature: "" };
    case "close_terminal":
      return {
        trogdorAtlasOpen: false,
        ...trogdorHoverReaderResetState(),
        trogdorSurfaceSignature: "",
      };
    case "toggle":
      return atlasOpen
        ? { trogdorAtlasOpen: false, ...trogdorHoverReaderResetState() }
        : { trogdorAtlasOpen: true };
    case "close":
      return { trogdorAtlasOpen: false, ...trogdorHoverReaderResetState() };
    default:
      return { trogdorAtlasOpen: Boolean(atlasOpen) };
  }
}

export function trogdorActionPayloadForZone(zone = {}) {
  switch (zone?.actionId) {
    case "trogdor_send":
      return {
        type: "session",
        sessionId: zone.sessionId,
        label: zone.label || zone.sessionId,
      };
    case "trogdor_group_send":
      return {
        type: "group",
        sessionIds: Array.isArray(zone.sessionIds) ? zone.sessionIds : [],
        label: zone.label || "batch agents",
      };
    case "trogdor_launch":
      return { cwd: zone.cwd, launchTarget: zone.launchTarget || "" };
    case "trogdor_mermaid":
    case "trogdor_commit":
      return { sessionId: zone.sessionId };
    default:
      return null;
  }
}

export function trogdorTerminalFocusStatus(selectedSession) {
  return {
    message: selectedSession
      ? "Terminal focused. Type directly or use the terminal actions below."
      : "Select a session row to attach its terminal first.",
    error: !selectedSession,
    timeoutMs: 2200,
  };
}

export function trogdorDomActionZoneForDataset(dataset = {}) {
  const zone = {
    type: "action",
    actionId: String(dataset?.action || ""),
  };
  if (dataset?.sessionId) {
    zone.sessionId = dataset.sessionId;
  }
  if (dataset?.label) {
    zone.label = dataset.label;
  }
  if (dataset?.cwd) {
    zone.cwd = dataset.cwd;
  }
  if (dataset?.sessionIds) {
    try {
      zone.sessionIds = JSON.parse(dataset.sessionIds);
    } catch (_error) {
      zone.sessionIds = [];
    }
  }
  return zone;
}

function closestFromTarget(target, selector) {
  return target && typeof target.closest === "function" ? target.closest(selector) : null;
}

export function trogdorSurfacePassthroughBindings() {
  return ["mousedown", "mouseup", "mousemove", "touchend", "wheel"].map((eventName) => ({
    eventName,
    options: eventName === "wheel" || eventName === "touchend" ? { passive: false } : undefined,
  }));
}

export function trogdorSurfacePointerDownPlan(target) {
  const agent = closestFromTarget(target, "[data-trogdor-agent]");
  const sessionId = agent?.dataset?.sessionId || "";
  if (!sessionId) {
    return { type: "ignore" };
  }
  return { type: "open_agent_terminal", sessionId, preventDefault: true, stopPropagation: true };
}

export function trogdorSurfaceClickPlan(target) {
  const button = closestFromTarget(target, "button[data-action]");
  if (button) {
    return { type: "dom_action", button, preventDefault: true, stopPropagation: true };
  }
  const agent = closestFromTarget(target, "[data-trogdor-agent]");
  const sessionId = agent?.dataset?.sessionId || "";
  if (sessionId) {
    return {
      type: "surface_action",
      zone: { type: "trogdor_agent", sessionId },
      preventDefault: true,
      stopPropagation: true,
    };
  }
  return { type: "ignore", preventDefault: true, stopPropagation: true };
}

export function trogdorSurfaceMouseoverPlan(target) {
  const agent = closestFromTarget(target, "[data-trogdor-agent]");
  if (agent?.dataset?.sessionId) {
    return { type: "hover", hover: { type: "trogdor_agent", sessionId: agent.dataset.sessionId } };
  }
  const action = closestFromTarget(target, "button[data-action]");
  const actionId = action?.dataset?.action || "";
  if (actionId.startsWith("trogdor_")) {
    return { type: "hover", hover: { type: "action", actionId } };
  }
  return { type: "ignore" };
}

export function trogdorSurfaceMouseleavePlan() {
  return { type: "clear_hover", hover: null };
}

export function trogdorSurfaceFocusInPlan(target) {
  const agent = closestFromTarget(target, "[data-trogdor-agent]");
  if (agent?.dataset?.sessionId) {
    return { type: "hover", hover: { type: "trogdor_agent", sessionId: agent.dataset.sessionId } };
  }
  return { type: "ignore" };
}

export function trogdorSurfaceFocusOutPlan({ relatedTargetInsideSurface = false } = {}) {
  return relatedTargetInsideSurface ? { type: "ignore" } : { type: "clear_hover", hover: null };
}

export function trogdorHoverSessionIdForZone(zone, previousSessionId = null) {
  if (zone?.type === "trogdor_agent" || zone?.type === "trogdor_reader") {
    return zone.sessionId;
  }
  if (String(zone?.actionId || "").startsWith("trogdor_")) {
    return previousSessionId;
  }
  return null;
}

export function trogdorRawSessionForHover(
  sessions,
  hoveredSessionId,
  {
    normalize = true,
  } = {},
) {
  const sessionId = normalize ? normalizeTrogdorSessionId(hoveredSessionId) : hoveredSessionId;
  if (!sessionId) {
    return null;
  }
  return (Array.isArray(sessions) ? sessions : []).find((session) => session?.session_id === sessionId) || null;
}

export function trogdorCurrentSurfaceSessionForHover({
  sessions = [],
  hoveredSessionId = null,
  toSurfaceSession = (session) => session,
} = {}) {
  const raw = trogdorRawSessionForHover(sessions, hoveredSessionId);
  return raw ? toSurfaceSession(raw) : null;
}

export function trogdorReadableHoveredSurfaceSession(
  sessions,
  hoveredSessionId,
  {
    sessionCanRead = () => true,
  } = {},
) {
  const hovered = (Array.isArray(sessions) ? sessions : [])
    .find((session) => session?.sessionId === hoveredSessionId) || null;
  return hovered && sessionCanRead(hovered) ? hovered : null;
}

export function trogdorSurfaceSessionTrogdorState(
  session,
  {
    burnt = false,
    dismissedClawgs = {},
    readProgress = {},
  } = {},
) {
  const dismissed = trogdorClawgDismissedForMap(session, dismissedClawgs);
  return {
    clawgReadIndex: trogdorClawgReadIndexForProgress(session, readProgress),
    clawgWordCount: trogdorClawgWords(session).length,
    trogdorAwaitingUser: trogdorSessionAwaitingUser(session),
    trogdorBurnt: Boolean(burnt),
    trogdorDismissed: dismissed,
    trogdorSwordsmanVisible: trogdorSwordsmanVisibleForState(session, { burnt, dismissed }),
  };
}
