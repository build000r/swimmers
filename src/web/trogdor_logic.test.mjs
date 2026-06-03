import test from "node:test";
import assert from "node:assert/strict";

import {
  buildTrogdorDomGroups,
  clearTrogdorDismissedClawgInMap,
  dismissTrogdorClawgInMap,
  loadTrogdorReadProgress,
  parseTrogdorReadProgress,
  rawActionCueKinds,
  rawSessionIsSleepingOrDeepSleep,
  saveTrogdorReadProgress,
  serializeTrogdorReadProgress,
  setTrogdorClawgReadIndexForProgress,
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
  trogdorSessionAwaitingUser,
  trogdorSessionHasReadyClawg,
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
