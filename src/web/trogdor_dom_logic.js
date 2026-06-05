export const TROGDOR_DRAGON_TARGET = { x: 56, y: 64 };

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

const TROGDOR_ACTION_CUE_AWAITING_USER = 1 << 0;
const TROGDOR_ACTION_CUE_COMMIT_READY = 1 << 1;
const TROGDOR_ACTION_CUE_VALIDATION_MISSING = 1 << 2;
const TROGDOR_ACTION_CUE_DIRTY_CHECK_MISSING = 1 << 3;
const TROGDOR_PRIMARY_ACTION_CUE_MASK =
  TROGDOR_ACTION_CUE_AWAITING_USER
  | TROGDOR_ACTION_CUE_COMMIT_READY
  | TROGDOR_ACTION_CUE_VALIDATION_MISSING
  | TROGDOR_ACTION_CUE_DIRTY_CHECK_MISSING;

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

function actionCuesForSession(session) {
  return Array.isArray(session?.actionCues) ? session.actionCues : [];
}

function normalizedActionCueKind(cue) {
  return String(cue?.kind || "").toLowerCase();
}

function actionCueBit(kind) {
  switch (kind) {
    case "awaiting_user":
      return TROGDOR_ACTION_CUE_AWAITING_USER;
    case "commit_ready":
      return TROGDOR_ACTION_CUE_COMMIT_READY;
    case "validation_missing_after_edit":
      return TROGDOR_ACTION_CUE_VALIDATION_MISSING;
    case "dirty_check_missing":
      return TROGDOR_ACTION_CUE_DIRTY_CHECK_MISSING;
    default:
      return 0;
  }
}

function primaryActionCueMask(session) {
  let mask = 0;
  for (const cue of actionCuesForSession(session)) {
    mask |= actionCueBit(normalizedActionCueKind(cue));
    if (mask === TROGDOR_PRIMARY_ACTION_CUE_MASK) break;
  }
  return mask;
}

function primaryActionCueFromMask(mask) {
  if (mask & TROGDOR_ACTION_CUE_AWAITING_USER) return "awaiting_user";
  if (mask & TROGDOR_ACTION_CUE_COMMIT_READY) return "commit_ready";
  if (mask & TROGDOR_ACTION_CUE_VALIDATION_MISSING) return "validation_missing_after_edit";
  if (mask & TROGDOR_ACTION_CUE_DIRTY_CHECK_MISSING) return "dirty_check_missing";
  return "";
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
  const cueMask = primaryActionCueMask(session);
  const stateLabel = String(session?.state || "").toLowerCase();
  const rest = String(session?.restLabel || "").toLowerCase();
  if (cueMask & TROGDOR_ACTION_CUE_AWAITING_USER) score += 55;
  if (cueMask & TROGDOR_ACTION_CUE_COMMIT_READY) score += 45;
  if (cueMask & TROGDOR_ACTION_CUE_VALIDATION_MISSING) score += 40;
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
  const cueMask = primaryActionCueMask(session);
  if (cueMask & TROGDOR_ACTION_CUE_AWAITING_USER) return "!";
  if ((cueMask & TROGDOR_ACTION_CUE_COMMIT_READY) || session?.commitCandidate) return "$";
  if (cueMask & TROGDOR_ACTION_CUE_VALIDATION_MISSING) return "v";
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
  const kinds = [];
  for (const cue of actionCuesForSession(session)) {
    const kind = normalizedActionCueKind(cue);
    if (kind) kinds.push(kind);
  }
  return kinds;
}

export function trogdorHasActionCue(session, kind) {
  const knownBit = actionCueBit(kind);
  if (knownBit) {
    return Boolean(primaryActionCueMask(session) & knownBit);
  }
  for (const cue of actionCuesForSession(session)) {
    if (normalizedActionCueKind(cue) === kind) return true;
  }
  return false;
}

export function trogdorPrimaryActionCue(session) {
  return primaryActionCueFromMask(primaryActionCueMask(session));
}
