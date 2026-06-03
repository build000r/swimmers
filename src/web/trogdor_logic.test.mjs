import test from "node:test";
import assert from "node:assert/strict";

import {
  buildTrogdorDomGroups,
  rawActionCueKinds,
  rawSessionIsSleepingOrDeepSleep,
  stableTextHash,
  summarizeTrogdorDom,
  trogdorAgentGlyph,
  trogdorAgentTone,
  trogdorClawgKey,
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
