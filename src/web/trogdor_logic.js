export const TROGDOR_DRAGON_TARGET = { x: 56, y: 64 };
export const TROGDOR_READ_PROGRESS_STORAGE_KEY = "swimmers.web.trogdor.readProgress";

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

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

function relativeCwd(cwd) {
  if (!cwd) return "unknown cwd";
  const parts = cwd.split("/").filter(Boolean);
  if (!parts.length) return cwd;
  return parts.slice(-2).join("/");
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
    return progress;
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

export function trogdorSessionIsSleepingOrDeepSleep(session) {
  const rest = String(session?.restLabel || "").toLowerCase();
  return rest === "sleeping" || rest === "deep_sleep";
}

export function buildTrogdorDomGroups(sessions) {
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

export function summarizeTrogdorDom(groups, sessions) {
  const maxPressure = groups.reduce((max, group) => Math.max(max, group.pressure), 0);
  const actionCues = sessions.reduce((count, session) => count + trogdorDomActionCueKinds(session).length, 0);
  return {
    score: String(maxPressure * 100 + actionCues * 37).padStart(4, "0"),
    level: maxPressure || 0,
    actionCues,
  };
}

export function trogdorDragonFrameForVector(dx, dy, fallback = "right") {
  if (!dx && !dy) return fallback;
  const sector = Math.round(Math.atan2(dy, dx) / (Math.PI / 4));
  return TROGDOR_DRAGON_FRAME_BY_SECTOR[String(sector)] ?? fallback;
}

export function trogdorDragonPose(groups, summary, repoPositions) {
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

  const target = focusGroup ? repoPositions[focusIndex % repoPositions.length] : null;
  let x = TROGDOR_DRAGON_TARGET.x;
  let y = TROGDOR_DRAGON_TARGET.y;
  let direction = "right";
  let bodyFrame = "right";
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

export function trogdorDomPressure(session) {
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

export function trogdorDomReason(session) {
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

export function trogdorAgentGlyph(session) {
  const pressure = session?.operatorPressure || {};
  if (pressure.glyph) return String(pressure.glyph).slice(0, 1);
  if (trogdorHasActionCue(session, "awaiting_user")) return "!";
  if (trogdorHasActionCue(session, "commit_ready") || session?.commitCandidate) return "$";
  if (trogdorHasActionCue(session, "validation_missing_after_edit")) return "v";
  if (String(session?.state || "").toLowerCase() === "error") return "x";
  if (trogdorSessionIsSleepingOrDeepSleep(session)) return "z";
  return "a";
}

export function trogdorAgentTone(session) {
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

export function trogdorDomActionCueKinds(session) {
  return (Array.isArray(session?.actionCues) ? session.actionCues : [])
    .map((cue) => String(cue?.kind || "").toLowerCase())
    .filter(Boolean);
}

export function trogdorHasActionCue(session, kind) {
  return trogdorDomActionCueKinds(session).includes(kind);
}

export function trogdorPrimaryActionCue(session) {
  const kinds = trogdorDomActionCueKinds(session);
  for (const kind of ["awaiting_user", "commit_ready", "validation_missing_after_edit", "dirty_check_missing"]) {
    if (kinds.includes(kind)) return kind;
  }
  return "";
}
