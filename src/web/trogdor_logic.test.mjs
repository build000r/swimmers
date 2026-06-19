import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  buildTrogdorDomGroups,
  clearTrogdorDismissedClawgInMap,
  dismissTrogdorClawgInMap,
  loadTrogdorReadProgress,
  markTrogdorBurntSessionsInMap,
  markTrogdorSessionsRespondedState,
  normalizeTrogdorSessionIds,
  parseTrogdorReadProgress,
  pruneTrogdorBurntSessionMap,
  rawActionCueKinds,
  rawSessionIsSleepingOrDeepSleep,
  rawTrogdorSessionAwaitingUser,
  saveTrogdorReadProgress,
  serializeTrogdorReadProgress,
  setTrogdorClawgReadIndexForProgress,
  startTrogdorReaderStateForSession,
  stableTextHash,
  summarizeTrogdorDom,
  trogdorAgentGlyph,
  trogdorAgentTone,
  trogdorClawgDismissedForMap,
  trogdorClawgKey,
  trogdorClawgReadCompleteForProgress,
  trogdorClawgReadIndexForProgress,
  trogdorClawgWords,
  trogdorDomPressure,
  trogdorDomReason,
  trogdorDragonFrameForVector,
  trogdorDragonPose,
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
  trogdorReaderBaseIndexForProgress,
  trogdorReaderStateForWpmChange,
  trogdorReaderTimerAction,
  trogdorReaderToggleAction,
  trogdorReaderWpmForAction,
  trogdorReaderWordIndexForProgress,
  trogdorRawSessionForHover,
  trogdorSessionCanReadForState,
  trogdorSessionBurntInMap,
  trogdorSessionAwaitingUser,
  trogdorSessionHasReadyClawg,
  trogdorSurfaceClickPlan,
  trogdorSurfaceFocusInPlan,
  trogdorSurfaceFocusOutPlan,
  trogdorSurfaceMouseleavePlan,
  trogdorSurfaceMouseoverPlan,
  trogdorSurfacePassthroughBindings,
  trogdorSurfacePointerDownPlan,
  trogdorSurfaceSessionTrogdorState,
  trogdorSwordsmanVisibleForState,
  trogdorTerminalFocusStatus,
} from "./trogdor_logic.js";

function session(overrides = {}) {
  return {
    sessionId: "agent-1",
    name: "agent-1",
    repoKey: "/tmp/repos/swimmers",
    repoLabel: "swimmers",
    state: "attention",
    restLabel: "active",
    thoughtLabel: "approve migration before commit",
    thoughtUpdatedAt: "2026-06-03T00:00:00Z",
    actionCues: [{ kind: "awaiting_user" }],
    operatorPressure: null,
    commitCandidate: false,
    ...overrides,
  };
}

function closestTarget(matches = {}) {
  return {
    closest(selector) {
      return matches[selector] ?? null;
    },
  };
}

test("Trogdor web docs keep backend vocabulary and action contract explicit", () => {
  const doc = readFileSync(new URL("../../docs/TROGDOR_WEB.md", import.meta.url), "utf8");

  for (const term of ["SessionSummary", "RestState", "StateEvidence", "action_cues", "operator_pressure"]) {
    assert.ok(doc.includes(term), `${term} should stay named as a source fact`);
  }
  for (const action of ["send", "batch", "launch", "mmd", "commit"]) {
    assert.ok(doc.includes(`- \`${action}\``), `${action} hover action should stay documented`);
  }
  assert.match(doc, /Do not add separate Trogdor backend facts/);
  assert.match(doc, /presentation only/);
});

test("Trogdor action cues, clawg keys, and raw rest detection are stable", () => {
  const raw = {
    rest_state: "deep_sleep",
    action_cues: [{ kind: "AWAITING_USER" }, { kind: "" }],
  };
  const item = session();

  assert.deepEqual(rawActionCueKinds(raw), ["awaiting_user"]);
  assert.equal(rawSessionIsSleepingOrDeepSleep(raw), true);
  assert.deepEqual(trogdorClawgWords(item), ["approve", "migration", "before", "commit"]);
  assert.equal(trogdorClawgKey(item), `agent-1:2026-06-03T00:00:00Z:${stableTextHash(item.thoughtLabel)}`);
  assert.equal(rawTrogdorSessionAwaitingUser({ state: "idle", action_cues: [] }), false);
  assert.equal(rawTrogdorSessionAwaitingUser({ state: "idle", action_cues: [] }, { pressure: { reason_kind: "awaiting_user" } }), true);
  assert.equal(trogdorPrimaryActionCue(item), "awaiting_user");
  assert.equal(trogdorSessionAwaitingUser(item), true);
  assert.equal(trogdorSessionHasReadyClawg(item), true);
});

test("Trogdor read progress helpers sanitize, clamp, complete, and serialize", () => {
  const item = session();
  const key = trogdorClawgKey(item);
  const parsed = parseTrogdorReadProgress(JSON.stringify({
    [key]: 2.8,
    negative: -1,
    bad: "nope",
    zero: 0,
  }));

  assert.deepEqual(parsed, { [key]: 2, zero: 0 });
  assert.deepEqual(parseTrogdorReadProgress("[1,2]"), {});
  assert.deepEqual(parseTrogdorReadProgress("{"), {});
  assert.equal(trogdorClawgReadIndexForProgress(item, parsed), 2);

  const advanced = setTrogdorClawgReadIndexForProgress(parsed, item, 99);
  assert.equal(advanced.changed, true);
  assert.equal(advanced.progress[key], trogdorClawgWords(item).length);
  assert.equal(trogdorClawgReadCompleteForProgress(item, advanced.progress), true);

  const unchanged = setTrogdorClawgReadIndexForProgress(advanced.progress, item, 99);
  assert.equal(unchanged.changed, false);
  assert.equal(serializeTrogdorReadProgress({ [key]: 3.4, ignored: -2 }), `{"${key}":3}`);
});

test("Trogdor read progress storage helpers are best-effort and injectable", () => {
  const writes = new Map();
  const storage = {
    getItem(key) {
      return key === "progress" ? "{\"a\":2,\"bad\":-1}" : "{";
    },
    setItem(key, value) {
      writes.set(key, value);
    },
  };

  assert.deepEqual(loadTrogdorReadProgress(storage, "progress"), { a: 2 });
  assert.deepEqual(loadTrogdorReadProgress(storage, "broken"), {});
  assert.equal(saveTrogdorReadProgress({ a: 2.9, bad: -1 }, storage, "progress"), true);
  assert.equal(writes.get("progress"), "{\"a\":2}");
  assert.equal(saveTrogdorReadProgress({ a: 1 }, { setItem() { throw new Error("full"); } }, "progress"), false);
});

test("Trogdor dismissed clawg helpers set and clear by current clawg key", () => {
  const item = session();
  const key = trogdorClawgKey(item);

  const dismissed = dismissTrogdorClawgInMap({}, item);
  assert.equal(dismissed.changed, true);
  assert.equal(dismissed.dismissedClawgs[key], true);
  assert.equal(trogdorClawgDismissedForMap(item, dismissed.dismissedClawgs), true);

  const repeated = dismissTrogdorClawgInMap(dismissed.dismissedClawgs, item);
  assert.equal(repeated.changed, false);

  const cleared = clearTrogdorDismissedClawgInMap(dismissed.dismissedClawgs, item);
  assert.equal(cleared.changed, true);
  assert.equal(trogdorClawgDismissedForMap(item, cleared.dismissedClawgs), false);
});

test("Trogdor response helpers normalize ids and update only awaiting sessions", () => {
  const ready = session({ sessionId: "ready" });
  const idle = session({
    sessionId: "idle",
    state: "idle",
    actionCues: [],
    operatorPressure: null,
  });
  const readyKey = trogdorClawgKey(ready);
  const idleKey = trogdorClawgKey(idle);

  const next = markTrogdorSessionsRespondedState({
    sessionIds: [" ready ", "missing", "idle", "", null],
    sessions: [
      { session_id: "ready", surface: ready },
      { session_id: "idle", surface: idle },
    ],
    toSurfaceSession: (raw) => raw.surface,
    dismissedClawgs: {},
    progress: {},
    hoveredSessionId: "ready",
  });

  assert.deepEqual(normalizeTrogdorSessionIds([" a ", "", null, "b"]), ["a", "b"]);
  assert.deepEqual(next.burntIds, ["ready"]);
  assert.equal(next.dismissedClawgs[readyKey], true);
  assert.equal(next.dismissedClawgs[idleKey], undefined);
  assert.equal(next.progress[readyKey], trogdorClawgWords(ready).length);
  assert.equal(next.progress[idleKey], undefined);
  assert.equal(next.progressChanged, true);
  assert.equal(next.resetReader, true);
});

test("Trogdor burn helpers mark, check, and prune expired sessions immutably", () => {
  const marked = markTrogdorBurntSessionsInMap(new Map([["old", 50]]), [" new ", "", "other"], 100, 900);
  assert.deepEqual(marked.ids, ["new", "other"]);
  assert.equal(marked.burntSessions.get("old"), 50);
  assert.equal(marked.burntSessions.get("new"), 1000);
  assert.equal(marked.burntSessions.get("other"), 1000);
  assert.equal(marked.changed, true);

  const active = trogdorSessionBurntInMap(marked.burntSessions, { sessionId: "new" }, 999);
  assert.equal(active.burnt, true);
  assert.equal(active.changed, false);

  const expired = trogdorSessionBurntInMap(marked.burntSessions, "old", 100);
  assert.equal(expired.burnt, false);
  assert.equal(expired.changed, true);
  assert.equal(expired.burntSessions.has("old"), false);
  assert.equal(marked.burntSessions.has("old"), true);

  const pruned = pruneTrogdorBurntSessionMap(marked.burntSessions, 1000);
  assert.equal(pruned.changed, true);
  assert.deepEqual(Array.from(pruned.burntSessions.keys()), []);
});

test("Trogdor cue transition helpers derive awaiting, burns, and hover reset", () => {
  const sessions = [
    { session_id: "still", state: "attention", rest_state: "active", action_cues: [] },
    { session_id: "sleepy", state: "idle", rest_state: "deep_sleep", action_cues: [] },
    { session_id: "quiet", state: "idle", rest_state: "active", action_cues: [] },
    { session_id: "burnt", state: "idle", rest_state: "active", action_cues: [] },
  ];
  const transition = trogdorCueTransitionState({
    sessions,
    previousAwaitingSessionIds: new Set(["still", "gone"]),
    hoveredSessionId: "quiet",
    sessionBurnt: (sessionId) => sessionId === "burnt",
  });

  assert.deepEqual(Array.from(transition.awaitingSessionIds), ["still"]);
  assert.deepEqual(transition.burntIds, ["gone"]);
  assert.equal(transition.resetReader, true);

  assert.equal(trogdorCueTransitionState({
    sessions,
    hoveredSessionId: "sleepy",
  }).resetReader, false);
  assert.equal(trogdorCueTransitionState({
    sessions,
    hoveredSessionId: "burnt",
    sessionBurnt: (sessionId) => sessionId === "burnt",
  }).resetReader, false);
  assert.equal(trogdorCueTransitionState({
    sessions,
    hoveredSessionId: "missing",
  }).resetReader, true);
});

test("Trogdor visibility helpers preserve burnt, dismissed, and rest-state rules", () => {
  const ready = session();
  const key = trogdorClawgKey(ready);
  const dismissedClawgs = { [key]: true };
  const sleeping = session({
    sessionId: "sleepy",
    state: "idle",
    restLabel: "deep_sleep",
    actionCues: [],
    operatorPressure: null,
  });
  const quiet = session({
    sessionId: "quiet",
    state: "idle",
    actionCues: [],
    operatorPressure: null,
  });

  assert.equal(trogdorSwordsmanVisibleForState(ready), true);
  assert.equal(trogdorSessionCanReadForState(ready), true);
  assert.equal(trogdorSwordsmanVisibleForState(ready, { dismissed: true }), false);
  assert.equal(trogdorSessionCanReadForState(ready, { dismissed: true }), false);
  assert.equal(trogdorSwordsmanVisibleForState(sleeping, { dismissed: true }), true);
  assert.equal(trogdorSessionCanReadForState(sleeping, { dismissed: true }), true);
  assert.equal(trogdorSwordsmanVisibleForState(quiet, { burnt: true }), true);
  assert.equal(trogdorSessionCanReadForState(quiet, { burnt: true }), false);

  const state = trogdorSurfaceSessionTrogdorState(ready, {
    readProgress: { [key]: 2 },
    dismissedClawgs,
  });
  assert.deepEqual(state, {
    clawgReadIndex: 2,
    clawgWordCount: 4,
    trogdorAwaitingUser: true,
    trogdorBurnt: false,
    trogdorDismissed: true,
    trogdorSwordsmanVisible: false,
  });
});

test("Trogdor hover helpers preserve current session and reader reset decisions", () => {
  const rawSessions = [
    { session_id: "agent-1", tmux_name: "one" },
    { session_id: "agent-2", tmux_name: "two" },
  ];
  const surfaceSessions = [
    session({ sessionId: "agent-1" }),
    session({ sessionId: "agent-2", actionCues: [], operatorPressure: null }),
  ];

  assert.deepEqual(trogdorHoverReaderResetState("agent-1"), {
    hoveredTrogdorSessionId: "agent-1",
    trogdorReaderStartedAt: 0,
    trogdorReaderStartIndex: 0,
    trogdorReaderClawgKey: "",
  });
  assert.equal(trogdorHoverSessionIdForZone({ type: "trogdor_agent", sessionId: "agent-1" }, "old"), "agent-1");
  assert.equal(trogdorHoverSessionIdForZone({ type: "trogdor_reader", sessionId: "agent-2" }, "old"), "agent-2");
  assert.equal(trogdorHoverSessionIdForZone({ type: "action", actionId: "trogdor_wpm_up" }, "old"), "old");
  assert.equal(trogdorHoverSessionIdForZone({ type: "action", actionId: "refresh" }, "old"), null);
  assert.equal(trogdorHoverSessionIdForZone(null, "old"), null);

  assert.equal(trogdorRawSessionForHover(rawSessions, " agent-1 "), rawSessions[0]);
  assert.equal(trogdorRawSessionForHover(rawSessions, " agent-1 ", { normalize: false }), null);
  assert.equal(trogdorRawSessionForHover(rawSessions, "missing"), null);
  assert.deepEqual(trogdorCurrentSurfaceSessionForHover({
    sessions: rawSessions,
    hoveredSessionId: "agent-2",
    toSurfaceSession: (raw) => ({ sessionId: raw.session_id, label: raw.tmux_name }),
  }), { sessionId: "agent-2", label: "two" });
  assert.equal(trogdorCurrentSurfaceSessionForHover({
    sessions: rawSessions,
    hoveredSessionId: "",
  }), null);

  assert.equal(trogdorReadableHoveredSurfaceSession(surfaceSessions, "agent-1", {
    sessionCanRead: (item) => item.sessionId === "agent-1",
  }), surfaceSessions[0]);
  assert.equal(trogdorReadableHoveredSurfaceSession(surfaceSessions, "agent-2", {
    sessionCanRead: (item) => item.sessionId === "agent-1",
  }), null);
  assert.equal(trogdorReadableHoveredSurfaceSession(surfaceSessions, " agent-1 ", {
    sessionCanRead: () => true,
  }), null);
});

test("Trogdor atlas transition helpers preserve path-specific state patches", () => {
  assert.deepEqual(trogdorAtlasTransitionState("open"), {
    trogdorAtlasOpen: true,
    trogdorSurfaceSignature: "",
  });
  assert.deepEqual(trogdorAtlasTransitionState("close_terminal"), {
    trogdorAtlasOpen: false,
    hoveredTrogdorSessionId: null,
    trogdorReaderStartedAt: 0,
    trogdorReaderStartIndex: 0,
    trogdorReaderClawgKey: "",
    trogdorSurfaceSignature: "",
  });
  assert.deepEqual(trogdorAtlasTransitionState("toggle", true), { trogdorAtlasOpen: false });
  assert.deepEqual(trogdorAtlasTransitionState("toggle", false), { trogdorAtlasOpen: true });
  assert.deepEqual(trogdorAtlasTransitionState("close"), { trogdorAtlasOpen: false });
  assert.deepEqual(trogdorAtlasTransitionState("unknown", "yes"), { trogdorAtlasOpen: true });
});

test("Trogdor action payload helpers preserve zone defaults and terminal copy", () => {
  const groupIds = ["agent-1", "agent-2"];

  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_send",
    sessionId: "agent-1",
    label: "Agent One",
  }), {
    type: "session",
    sessionId: "agent-1",
    label: "Agent One",
  });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_send",
    sessionId: "agent-1",
    label: "",
  }), {
    type: "session",
    sessionId: "agent-1",
    label: "agent-1",
  });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_group_send",
    sessionIds: groupIds,
    label: "",
  }), {
    type: "group",
    sessionIds: groupIds,
    label: "batch agents",
  });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_group_send",
    sessionIds: "agent-1",
    label: "not array",
  }), {
    type: "group",
    sessionIds: [],
    label: "not array",
  });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_launch",
    cwd: "/tmp/repos/swimmers",
  }), { cwd: "/tmp/repos/swimmers", launchTarget: "" });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_launch",
    cwd: "/workspace/swimmers",
    launchTarget: "devbox",
  }), { cwd: "/workspace/swimmers", launchTarget: "devbox" });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_mermaid",
    sessionId: "agent-1",
  }), { sessionId: "agent-1" });
  assert.deepEqual(trogdorActionPayloadForZone({
    actionId: "trogdor_commit",
    sessionId: "agent-2",
  }), { sessionId: "agent-2" });
  assert.equal(trogdorActionPayloadForZone({ actionId: "refresh" }), null);

  assert.deepEqual(trogdorTerminalFocusStatus({ session_id: "agent-1" }), {
    message: "Terminal focused. Type directly or use the terminal actions below.",
    error: false,
    timeoutMs: 2200,
  });
  assert.deepEqual(trogdorTerminalFocusStatus(null), {
    message: "Select a session row to attach its terminal first.",
    error: true,
    timeoutMs: 2200,
  });
});

test("Trogdor DOM action zone helper preserves dataset parsing semantics", () => {
  assert.deepEqual(trogdorDomActionZoneForDataset(), {
    type: "action",
    actionId: "",
  });
  assert.deepEqual(trogdorDomActionZoneForDataset({
    action: "trogdor_group_send",
    sessionId: "agent-1",
    label: "Batch",
    cwd: "/tmp/repos/swimmers",
    sessionIds: "[\"agent-1\",\"agent-2\"]",
  }), {
    type: "action",
    actionId: "trogdor_group_send",
    sessionId: "agent-1",
    label: "Batch",
    cwd: "/tmp/repos/swimmers",
    sessionIds: ["agent-1", "agent-2"],
  });
  assert.deepEqual(trogdorDomActionZoneForDataset({
    action: "trogdor_group_send",
    sessionIds: "not-json",
  }), {
    type: "action",
    actionId: "trogdor_group_send",
    sessionIds: [],
  });
  assert.deepEqual(trogdorDomActionZoneForDataset({
    action: "trogdor_group_send",
    sessionIds: "\"agent-1\"",
  }), {
    type: "action",
    actionId: "trogdor_group_send",
    sessionIds: "agent-1",
  });
  assert.deepEqual(trogdorDomActionZoneForDataset({
    action: "trogdor_send",
    sessionId: "",
    label: "",
    cwd: "",
    sessionIds: "",
  }), {
    type: "action",
    actionId: "trogdor_send",
  });
});

test("Trogdor DOM event planners preserve pointer, click, and passthrough semantics", () => {
  const agent = { dataset: { sessionId: "agent-1" } };
  const blankAgent = { dataset: { sessionId: "" } };
  const button = { dataset: { action: "trogdor_send" } };
  const agentTarget = closestTarget({ "[data-trogdor-agent]": agent });
  const buttonTarget = closestTarget({ "button[data-action]": button, "[data-trogdor-agent]": agent });

  assert.deepEqual(trogdorSurfacePointerDownPlan(null), { type: "ignore" });
  assert.deepEqual(trogdorSurfacePointerDownPlan(closestTarget({ "[data-trogdor-agent]": blankAgent })), { type: "ignore" });
  assert.deepEqual(trogdorSurfacePointerDownPlan(agentTarget), {
    type: "open_agent_terminal",
    sessionId: "agent-1",
    preventDefault: true,
    stopPropagation: true,
  });

  assert.deepEqual(trogdorSurfacePassthroughBindings(), [
    { eventName: "mousedown", options: undefined },
    { eventName: "mouseup", options: undefined },
    { eventName: "mousemove", options: undefined },
    { eventName: "touchend", options: { passive: false } },
    { eventName: "wheel", options: { passive: false } },
  ]);

  assert.deepEqual(trogdorSurfaceClickPlan(buttonTarget), {
    type: "dom_action",
    button,
    preventDefault: true,
    stopPropagation: true,
  });
  assert.deepEqual(trogdorSurfaceClickPlan(agentTarget), {
    type: "surface_action",
    zone: { type: "trogdor_agent", sessionId: "agent-1" },
    preventDefault: true,
    stopPropagation: true,
  });
  assert.deepEqual(trogdorSurfaceClickPlan(null), {
    type: "ignore",
    preventDefault: true,
    stopPropagation: true,
  });
});

test("Trogdor DOM event planners preserve hover and focus semantics", () => {
  const agent = { dataset: { sessionId: "agent-1" } };
  const action = { dataset: { action: "trogdor_commit" } };
  const ignoredAction = { dataset: { action: "refresh" } };

  assert.deepEqual(trogdorSurfaceMouseoverPlan(closestTarget({ "[data-trogdor-agent]": agent })), {
    type: "hover",
    hover: { type: "trogdor_agent", sessionId: "agent-1" },
  });
  assert.deepEqual(trogdorSurfaceMouseoverPlan(closestTarget({ "button[data-action]": action })), {
    type: "hover",
    hover: { type: "action", actionId: "trogdor_commit" },
  });
  assert.deepEqual(trogdorSurfaceMouseoverPlan(closestTarget({ "button[data-action]": ignoredAction })), {
    type: "ignore",
  });
  assert.deepEqual(trogdorSurfaceMouseleavePlan(), { type: "clear_hover", hover: null });
  assert.deepEqual(trogdorSurfaceFocusInPlan(closestTarget({ "[data-trogdor-agent]": agent })), {
    type: "hover",
    hover: { type: "trogdor_agent", sessionId: "agent-1" },
  });
  assert.deepEqual(trogdorSurfaceFocusInPlan(closestTarget({ "button[data-action]": action })), { type: "ignore" });
  assert.deepEqual(trogdorSurfaceFocusOutPlan({ relatedTargetInsideSurface: true }), { type: "ignore" });
  assert.deepEqual(trogdorSurfaceFocusOutPlan({ relatedTargetInsideSurface: false }), { type: "clear_hover", hover: null });
  assert.deepEqual(trogdorSurfaceFocusOutPlan(), { type: "clear_hover", hover: null });
});

test("Trogdor reader base index prefers active reader key over persisted progress", () => {
  const item = session();
  const key = trogdorClawgKey(item);
  const progress = { [key]: 3 };

  assert.equal(trogdorReaderBaseIndexForProgress(item, {
    readerClawgKey: key,
    readerStartIndex: 1,
    progress,
  }), 1);
  assert.equal(trogdorReaderBaseIndexForProgress(item, {
    readerClawgKey: "other",
    readerStartIndex: 1,
    progress,
  }), 3);
  assert.equal(trogdorReaderBaseIndexForProgress(item, {
    readerClawgKey: key,
    readerStartIndex: 99,
    progress,
  }), 4);
});

test("Trogdor reader word index advances by elapsed WPM and respects pause/complete states", () => {
  const item = session();
  const key = trogdorClawgKey(item);
  const base = {
    readerClawgKey: key,
    readerStartIndex: 1,
    progress: {},
    hoveredSessionId: item.sessionId,
    readerStartedAt: 1_000,
  };

  assert.equal(trogdorReaderWordIndexForProgress(item, {
    ...base,
    wpm: 120,
    now: 2_100,
  }), 3);
  assert.equal(trogdorReaderWordIndexForProgress(item, {
    ...base,
    wpm: 120,
    reading: false,
    now: 3_000,
  }), 1);
  assert.equal(trogdorReaderWordIndexForProgress(item, {
    ...base,
    wpm: 120,
    hoveredSessionId: "",
    now: 3_000,
  }), 1);
  assert.equal(trogdorReaderWordIndexForProgress(session({ clawgText: "   " }), { now: 3_000 }), -1);
  assert.equal(trogdorReaderWordIndexForProgress(item, {
    readerClawgKey: key,
    readerStartIndex: 4,
    wpm: 120,
    hoveredSessionId: item.sessionId,
    now: 3_000,
  }), 4);
});

test("Trogdor reader display state formats banners and read-complete state", () => {
  const longWord = "supercalifragilisticexpialidocious";
  const item = session({ clawgText: `first ${longWord} final` });
  const key = trogdorClawgKey(item);

  assert.deepEqual(trogdorReaderDisplayState(null), {
    bannerText: "burninate!",
    readComplete: false,
  });
  assert.deepEqual(trogdorReaderDisplayState(session({ clawgText: "   " })), {
    bannerText: "waiting",
    readComplete: false,
  });
  assert.deepEqual(trogdorReaderDisplayState(item, { wordIndex: -1 }), {
    bannerText: "first",
    readComplete: false,
  });
  assert.deepEqual(trogdorReaderDisplayState(item, {
    wordIndex: 1,
    progress: { [key]: 3 },
  }), {
    bannerText: longWord.slice(0, 22),
    readComplete: true,
  });
  assert.deepEqual(trogdorReaderDisplayState(item, {
    wordIndex: 99,
    progress: { [key]: 2 },
  }), {
    bannerText: "caught up",
    readComplete: false,
  });
  assert.equal(
    trogdorReaderDisplayState(item, { wordIndex: 1, maxWordChars: 8 }).bannerText,
    longWord.slice(0, 8),
  );
});

test("Trogdor reader timer action preserves run-state decisions and short-circuits", () => {
  const item = session();
  const calls = [];
  const canRead = (value) => {
    calls.push(["canRead", value.sessionId]);
    return true;
  };
  const complete = (value) => {
    calls.push(["complete", value.sessionId]);
    return false;
  };

  assert.equal(trogdorReaderTimerAction(item, canRead, complete, true, false), "start");
  assert.deepEqual(calls, [["canRead", "agent-1"], ["complete", "agent-1"]]);
  calls.length = 0;

  assert.equal(trogdorReaderTimerAction(item, canRead, complete, true, 1), "keep");
  assert.equal(trogdorReaderTimerAction(item, canRead, () => true, true, 1), "stop");
  assert.equal(trogdorReaderTimerAction(item, () => false, complete, true, 1), "stop");
  assert.equal(trogdorReaderTimerAction(item, canRead, complete, false, 1), "stop");
  assert.equal(trogdorReaderTimerAction(null, canRead, complete, true, 1), "stop");

  assert.deepEqual(calls, [
    ["canRead", "agent-1"],
    ["complete", "agent-1"],
    ["canRead", "agent-1"],
    ["canRead", "agent-1"],
  ]);
});

test("Trogdor reader toggle action preserves pause, resume, and read-again decisions", () => {
  const item = session();
  const calls = [];
  const readComplete = (value) => {
    calls.push(value.sessionId);
    return value.sessionId === "done";
  };

  assert.deepEqual(trogdorReaderToggleAction(true, item, readComplete), {
    session: null,
    reading: false,
    readAgain: false,
    restartClock: false,
  });
  assert.deepEqual(trogdorReaderToggleAction(undefined, item, readComplete), {
    session: null,
    reading: false,
    readAgain: false,
    restartClock: false,
  });
  assert.deepEqual(trogdorReaderToggleAction(false, null, readComplete), {
    session: null,
    reading: true,
    readAgain: false,
    restartClock: true,
  });
  assert.deepEqual(trogdorReaderToggleAction(false, item, readComplete), {
    session: item,
    reading: null,
    readAgain: false,
    restartClock: true,
  });
  assert.deepEqual(trogdorReaderToggleAction(false, session({ sessionId: "done" }), readComplete), {
    session: session({ sessionId: "done" }),
    reading: null,
    readAgain: true,
    restartClock: true,
  });
  assert.deepEqual(calls, ["agent-1", "done"]);
});

test("Trogdor reader WPM action preserves step, bounds, and JS arithmetic semantics", () => {
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_down", 200), 175);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_up", 200), 225);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_down", 50), 50);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_up", 800), 800);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_down", undefined), 200);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_up", undefined), 200);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_down", "200"), 175);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_up", "200"), 200);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_down", 105, {
    step: 10,
    min: 100,
    max: 210,
  }), 100);
  assert.equal(trogdorReaderWpmForAction("trogdor_wpm_up", 205, {
    step: 10,
    min: 100,
    max: 210,
  }), 210);
});

test("Trogdor reader advancement helpers clamp progress and preserve WPM restart state", () => {
  const item = session();
  const key = trogdorClawgKey(item);
  const progress = { [key]: 2 };

  assert.deepEqual(trogdorReaderProgressAdvanceForSession(item, {
    wordIndex: 1,
    reading: true,
  }), {
    shouldAdvance: true,
    nextReadIndex: 2,
    reading: true,
    complete: false,
  });
  assert.deepEqual(trogdorReaderProgressAdvanceForSession(item, {
    wordIndex: 99,
    reading: true,
  }), {
    shouldAdvance: true,
    nextReadIndex: 4,
    reading: false,
    complete: true,
  });
  assert.deepEqual(trogdorReaderProgressAdvanceForSession(item, {
    wordIndex: 1,
    reading: false,
  }), {
    shouldAdvance: false,
    nextReadIndex: 0,
    reading: false,
    complete: false,
  });
  assert.deepEqual(trogdorReaderProgressAdvanceForSession(session({ clawgText: " " }), {
    wordIndex: 1,
    reading: true,
  }), {
    shouldAdvance: false,
    nextReadIndex: 0,
    reading: true,
    complete: false,
  });
  assert.deepEqual(trogdorReaderProgressAdvanceForSession(item, {
    wordIndex: -1,
    reading: true,
  }), {
    shouldAdvance: false,
    nextReadIndex: 0,
    reading: true,
    complete: false,
  });

  assert.deepEqual(trogdorReaderStateForWpmChange(item, {
    currentStartIndex: 1,
    progress,
    now: 12_345,
  }), {
    trogdorReaderStartIndex: 2,
    trogdorReaderStartedAt: 12_345,
  });
  assert.deepEqual(trogdorReaderStateForWpmChange(null, {
    currentStartIndex: 3,
    progress,
    now: 12_345,
  }), {
    trogdorReaderStartIndex: 3,
    trogdorReaderStartedAt: 12_345,
  });
});

test("Trogdor reader start state resumes progress and read-again resets progress", () => {
  const item = session();
  const key = trogdorClawgKey(item);
  const dismissed = { [key]: true };
  const progress = { [key]: 3 };

  const resumed = startTrogdorReaderStateForSession(item, {
    dismissedClawgs: dismissed,
    progress,
    now: 10_000,
  });
  assert.deepEqual(resumed, {
    readerClawgKey: key,
    readerStartIndex: 3,
    readerStartedAt: 10_000,
    reading: true,
    dismissedClawgs: dismissed,
    progress,
    progressChanged: false,
  });

  const reset = startTrogdorReaderStateForSession(item, {
    readAgain: true,
    dismissedClawgs: dismissed,
    progress,
    now: 12_000,
  });
  assert.equal(reset.readerClawgKey, key);
  assert.equal(reset.readerStartIndex, 0);
  assert.equal(reset.readerStartedAt, 12_000);
  assert.equal(reset.reading, true);
  assert.deepEqual(reset.dismissedClawgs, {});
  assert.deepEqual(reset.progress, { [key]: 0 });
  assert.equal(reset.progressChanged, true);
});

test("Trogdor pressure, reason, glyph, and tone prefer operator pressure when present", () => {
  const pressured = session({
    operatorPressure: {
      score: 88,
      reason: "needs review",
      glyph: "?",
      tone: "danger",
    },
    actionCues: [],
    state: "busy",
  });

  assert.equal(trogdorDomPressure(pressured), 88);
  assert.equal(trogdorDomReason(pressured), "needs review");
  assert.equal(trogdorAgentGlyph(pressured), "?");
  assert.equal(trogdorAgentTone(pressured), "danger");

  const dirtyCheck = session({
    operatorPressure: {
      score: 40,
      reason: "dirty check",
      reason_kind: "dirty_check_missing",
      glyph: "d",
      tone: "warning",
    },
    actionCues: [{ kind: "dirty_check_missing" }],
    state: "idle",
  });
  assert.equal(trogdorDomReason(dirtyCheck), "dirty check");
  assert.equal(trogdorAgentGlyph(dirtyCheck), "d");
  assert.equal(trogdorAgentTone(dirtyCheck), "warning");

  const inferredDirtyCheck = session({
    actionCues: [{ kind: "dirty_check_missing" }],
    state: "idle",
  });
  assert.equal(trogdorDomPressure(inferredDirtyCheck), 40);
  assert.equal(trogdorDomReason(inferredDirtyCheck), "dirty check missing");
  assert.equal(trogdorAgentGlyph(inferredDirtyCheck), "d");
  assert.equal(trogdorAgentTone(inferredDirtyCheck), "warning");

  const inferred = session({ actionCues: [{ kind: "commit_ready" }], state: "busy", commitCandidate: true });
  assert.equal(trogdorDomPressure(inferred), 82);
  assert.equal(trogdorDomReason(inferred), "commit ready");
  assert.equal(trogdorAgentGlyph(inferred), "$");
  assert.equal(trogdorAgentTone(inferred), "danger");
});

test("Trogdor groups sort by pressure and summarize score/action cues", () => {
  const calm = session({
    sessionId: "calm",
    repoKey: "/tmp/repos/aaa",
    repoLabel: "aaa",
    state: "idle",
    actionCues: [],
  });
  const urgent = session({
    sessionId: "urgent",
    repoKey: "/tmp/repos/zzz",
    repoLabel: "zzz",
    operatorPressure: { score: 90, reason: "blocked" },
    actionCues: [{ kind: "awaiting_user" }, { kind: "commit_ready" }],
  });

  const groups = buildTrogdorDomGroups([calm, urgent]);
  const summary = summarizeTrogdorDom(groups, [calm, urgent]);

  assert.equal(groups[0].key, "/tmp/repos/zzz");
  assert.equal(groups[0].reason, "blocked");
  assert.deepEqual(summary, { score: "9074", level: 90, actionCues: 2 });
});

test("Trogdor groups preserve cross-host repo grouping metadata", () => {
  const local = session({
    sessionId: "local",
    repoKey: "/Users/b/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    targetLabel: "local",
    actionCues: [],
  });
  const remote = session({
    sessionId: "remote",
    repoKey: "/Users/b/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    targetLabel: "Skillbox devbox",
    actionCues: [],
    operatorPressure: { score: 60, reason: "remote waiting" },
  });

  const groups = buildTrogdorDomGroups([local, remote]);

  assert.equal(groups.length, 1);
  assert.equal(groups[0].label, "opensource/swimmers");
  assert.equal(groups[0].sessions.length, 2);
  assert.equal(groups[0].hostSummary, "local + Skillbox devbox");
  assert.equal(groups[0].reason, "remote waiting");
});

test("Trogdor dragon pose focuses burnt sessions and keeps 8-way body frames", () => {
  const positions = [{ x: 18, y: 40 }, { x: 70, y: 60 }];
  const groups = [
    { sessions: [session({ trogdorBurnt: false })], pressure: 10 },
    { sessions: [session({ sessionId: "burnt", trogdorBurnt: true })], pressure: 80 },
  ];
  const pose = trogdorDragonPose(groups, { level: 80 }, positions);

  assert.equal(pose.firing, true);
  assert.equal(pose.heated, true);
  assert.equal(pose.direction, "right");
  assert.equal(pose.bodyFrame, "3q-right");
  assert.equal(trogdorDragonFrameForVector(-1, -1), "back-left");
});
