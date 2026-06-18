import test from "node:test";
import assert from "node:assert/strict";

import { buildSurfaceFrame, surfaceActionAt } from "./rendered_surface.js";

function session(overrides = {}) {
  return {
    sessionId: "sess-1",
    name: "sess-1",
    state: "busy",
    displayState: "busy",
    stateTrustLabel: "high observed liveness_has_children",
    stateConfidence: "high",
    stateObserved: true,
    restLabel: "active",
    transportLabel: "healthy",
    toolLabel: "Codex",
    cwdLabel: "repo/swimmers",
    fullCwd: "/tmp/repo/swimmers",
    thoughtLabel: "reviewing parity work",
    contextLabel: "10 / 100",
    skillLabel: "commit",
    activityLabel: "Apr 1, 8:00 PM",
    commandLabel: "cargo test",
    attachedLabel: "1",
    commitCandidate: true,
    actionCues: [],
    operatorPressure: null,
    batchSendSessionIds: [],
    repoKey: "/tmp/repo/swimmers",
    repoLabel: "swimmers",
    isStale: false,
    ...overrides,
  };
}

function baseModel(overrides = {}) {
  const currentSession = overrides.currentSession ?? session();
  return {
    cols: 140,
    rows: 40,
    focusLayout: false,
    followPublishedSelection: false,
    connectionLabel: "attached",
    connectionMuted: false,
    modeLabel: "operator",
    modeMuted: false,
    searchLabel: "idle",
    searchMuted: true,
    utilityLabel: "ready",
    utilityMuted: true,
    searchQuery: "",
    selectMode: false,
    readOnly: false,
    frankenTermAvailable: true,
    terminalReady: true,
    snapshotFallback: false,
    activeSheet: null,
    hoveredLinkUrl: "",
    sessions: overrides.sessions ?? (currentSession ? [currentSession] : [session()]),
    selectedSessionId: overrides.selectedSessionId ?? currentSession?.sessionId ?? null,
    publishedSessionId: null,
    publishedAtLabel: "",
    currentSession,
    ...overrides,
  };
}

function frameText(frame) {
  const lines = [];
  for (let row = 0; row < frame.rows; row += 1) {
    let line = "";
    for (let col = 0; col < frame.cols; col += 1) {
      const index = (row * frame.cols + col) * 4;
      line += String.fromCodePoint(frame.cells[index + 2] || 32);
    }
    lines.push(line);
  }
  return lines.join("\n");
}

function mixUint32(hash, value) {
  let next = Number(value) >>> 0;
  for (let byte = 0; byte < 4; byte += 1) {
    hash ^= next & 0xff;
    hash = Math.imul(hash, 0x01000193) >>> 0;
    next >>>= 8;
  }
  return hash >>> 0;
}

function framePatchHash(frame) {
  let hash = 0x811c9dc5;
  for (const value of frame.spans) {
    hash = mixUint32(hash, value);
  }
  for (const value of frame.cells) {
    hash = mixUint32(hash, value);
  }
  return hash.toString(16).padStart(8, "0");
}

test("surface frame emits flat patch payload invariants", () => {
  const frame = buildSurfaceFrame(baseModel());
  const cellCount = frame.cols * frame.rows;

  assert.ok(frame.spans instanceof Uint32Array);
  assert.ok(frame.cells instanceof Uint32Array);
  assert.deepEqual(Array.from(frame.spans), [0, cellCount]);
  assert.equal(frame.cells.length, cellCount * 4);
  assert.equal(frame.spans.length % 2, 0);
  for (let index = 0; index < frame.spans.length; index += 2) {
    const offset = frame.spans[index];
    const len = frame.spans[index + 1];
    assert.equal(Number.isInteger(offset), true);
    assert.equal(Number.isInteger(len), true);
    assert.equal(offset >= 0, true);
    assert.equal(len >= 0, true);
    assert.equal(offset + len <= cellCount, true);
  }
});

test("surface frame patch hash is deterministic and model-sensitive", () => {
  const frameA = buildSurfaceFrame(baseModel());
  const frameB = buildSurfaceFrame(baseModel());
  const changed = buildSurfaceFrame(
    baseModel({
      utilityLabel: "different utility status",
    }),
  );

  assert.equal(framePatchHash(frameA), framePatchHash(frameB));
  assert.notEqual(framePatchHash(frameA), framePatchHash(changed));
});

test("surface exposes parity actions for the selected session", () => {
  const frame = buildSurfaceFrame(baseModel());
  const actionIds = frame.zones
    .map((zone) => zone.actionId)
    .filter(Boolean);

  assert.ok(actionIds.includes("focus_terminal"));
  assert.ok(actionIds.includes("open_mermaid"));
  assert.ok(actionIds.includes("launch_commit"));
  assert.ok(actionIds.includes("open_config"));
  assert.ok(actionIds.includes("open_native"));
  assert.match(frameText(frame), /live terminal/);
});

test("surface renders grouped session rail with host-disambiguated project labels", () => {
  const local = session({
    sessionId: "local",
    name: "local-agent",
    targetLabel: "local",
    repoKey: "/Users/b/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    cwdLabel: "opensource/swimmers",
  });
  const remote = session({
    sessionId: "remote",
    name: "remote-agent",
    targetLabel: "Skillbox devbox",
    repoKey: "/Users/b/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    cwdLabel: "opensource/swimmers @ Skillbox devbox",
  });
  const frame = buildSurfaceFrame(baseModel({
    currentSession: remote,
    selectedSessionId: "remote",
    sessionGroupMode: "project",
    sessions: [local, remote],
    sessionRailRows: [
      {
        type: "session",
        session: local,
        group: {
          key: "/Users/b/repos/opensource/swimmers",
          label: "opensource/swimmers",
          count: 2,
          hostSummary: "local + Skillbox devbox",
          first: true,
        },
      },
      {
        type: "session",
        session: remote,
        group: {
          key: "/Users/b/repos/opensource/swimmers",
          label: "opensource/swimmers",
          count: 2,
          hostSummary: "local + Skillbox devbox",
          first: false,
        },
      },
    ],
  }));
  const text = frameText(frame);
  const actionIds = frame.zones.map((zone) => zone.actionId).filter(Boolean);

  assert.match(text, /view grouped/);
  assert.match(text, /2x swimmers L\+Skillbox/);
  assert.ok(actionIds.includes("toggle_session_grouping"));
});

test("surface shows an overview prompt when no session is selected", () => {
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      terminalReady: false,
    }),
  );

  const text = frameText(frame);
  assert.match(text, /select a session to attach its terminal/i);
  assert.match(text, /Select a session from the/);
  assert.match(text, /rendered rail to attach/);
});

test("surface calls out snapshot fallback mode", () => {
  const frame = buildSurfaceFrame(
    baseModel({
      frankenTermAvailable: false,
      snapshotFallback: true,
      terminalReady: false,
    }),
  );

  assert.match(frameText(frame), /snapshot fallback/i);
});

test("surface annotates untrusted session state", () => {
  const lowEvidenceSession = session({
    displayState: "busy?",
    stateTrustLabel: "low unobserved summary_cache_degraded",
    stateConfidence: "low",
    stateObserved: false,
  });
  const frame = buildSurfaceFrame(baseModel({ currentSession: lowEvidenceSession }));
  const text = frameText(frame);

  assert.match(text, /busy\?/);
  assert.match(text, /low unobserved/);
});

test("surface annotates locally inferred medium-confidence state", () => {
  const mediumEvidenceSession = session({
    displayState: undefined,
    stateTrustLabel: "medium observed local_input",
    stateConfidence: "medium",
    stateObserved: true,
  });
  const frame = buildSurfaceFrame(baseModel({ currentSession: mediumEvidenceSession }));
  const text = frameText(frame);

  assert.match(text, /busy\?/);
  assert.match(text, /medium observed lo/);
});

test("surface renders trogdor pressure atlas with hover speed reader", () => {
  const hovered = session({
    sessionId: "agent-1",
    name: "agent-1",
    state: "attention",
    displayState: "attention",
    restLabel: "sleeping",
    cwdLabel: "repos/swimmers",
    fullCwd: "/tmp/repos/swimmers",
    thoughtLabel: "approve migration before commit",
    actionCues: [
      {
        kind: "awaiting_user",
        status: "active",
        source: "transcript",
        confidence: "deterministic",
        evidence: ["awaiting_user_input"],
      },
    ],
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [
        hovered,
        session({
          sessionId: "agent-2",
          name: "agent-2",
          cwdLabel: "repos/swimmers",
          fullCwd: "/tmp/repos/swimmers",
          thoughtLabel: "running tests",
        }),
      ],
      hoveredTrogdorSessionId: "agent-1",
      trogdorWpm: 200,
      trogdorReaderElapsedMs: 0,
    }),
  );
  const text = frameText(frame);
  const zoneTypes = frame.zones.map((zone) => zone.type);
  const actionIds = frame.zones.map((zone) => zone.actionId).filter(Boolean);

  assert.match(text, /trogdor pressure/i);
  assert.match(text, /burninate/i);
  assert.match(text, /speed read agent/i);
  assert.match(text, /200 wpm/);
  assert.match(text, /approve/);
  assert.match(text, /awaiting user/);
  assert.ok(zoneTypes.includes("trogdor_agent"));
  assert.ok(actionIds.includes("trogdor_wpm_down"));
  assert.ok(actionIds.includes("trogdor_wpm_up"));
  assert.ok(actionIds.includes("trogdor_send"));
  assert.ok(actionIds.includes("trogdor_launch"));
  assert.ok(actionIds.includes("trogdor_mermaid"));
});

test("surface uses shared operator pressure and exposes batch/commit actions", () => {
  const hovered = session({
    sessionId: "agent-1",
    name: "agent-1",
    state: "idle",
    displayState: "idle",
    restLabel: "active",
    commitCandidate: false,
    operatorPressure: {
      score: 88,
      reason: "commit ready",
      reason_kind: "commit_ready",
      glyph: "$",
      tone: "danger",
      needs_input: false,
      launch_ready: false,
      commit_ready: true,
      action_cue_count: 1,
    },
    batchSendSessionIds: ["agent-1", "agent-2"],
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [hovered],
      hoveredTrogdorSessionId: "agent-1",
      trogdorWpm: 200,
      trogdorReaderElapsedMs: 0,
    }),
  );
  const text = frameText(frame);
  const zones = frame.zones.filter((zone) => zone.actionId);
  const actionIds = zones.map((zone) => zone.actionId);
  const batchZone = zones.find((zone) => zone.actionId === "trogdor_group_send");

  assert.match(text, /pressure 88/);
  assert.match(text, /repo swimmers \/ commit ready/);
  assert.ok(actionIds.includes("trogdor_group_send"));
  assert.ok(actionIds.includes("trogdor_commit"));
  assert.deepEqual(batchZone.sessionIds, ["agent-1", "agent-2"]);
});

test("surface advances hovered speed-reader word by wpm timing", () => {
  const hovered = session({
    sessionId: "agent-1",
    name: "agent-1",
    state: "attention",
    displayState: "attention",
    restLabel: "sleeping",
    cwdLabel: "repos/swimmers",
    fullCwd: "/tmp/repos/swimmers",
    thoughtLabel: "alpha beta gamma",
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [hovered],
      hoveredTrogdorSessionId: "agent-1",
      trogdorWpm: 200,
      trogdorReaderElapsedMs: 650,
    }),
  );

  assert.match(frameText(frame), /gamma/);
});

test("surface starts the speed reader at the unread clawg cursor", () => {
  const hovered = session({
    sessionId: "agent-1",
    name: "agent-1",
    state: "attention",
    displayState: "attention",
    restLabel: "sleeping",
    thoughtLabel: "alpha beta gamma",
    clawgReadIndex: 1,
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [hovered],
      hoveredTrogdorSessionId: "agent-1",
      trogdorWpm: 200,
      trogdorReaderElapsedMs: 0,
      trogdorReaderStartIndex: 1,
    }),
  );

  const text = frameText(frame);
  assert.match(text, /speed read agent/i);
  assert.match(text, /beta/);
});

test("surface shows read-again instead of replaying a fully read clawg", () => {
  const hovered = session({
    sessionId: "agent-1",
    name: "agent-1",
    state: "attention",
    displayState: "attention",
    restLabel: "sleeping",
    thoughtLabel: "alpha beta gamma",
    clawgReadIndex: 3,
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [hovered],
      hoveredTrogdorSessionId: "agent-1",
      trogdorWpm: 200,
      trogdorReading: false,
      trogdorReaderElapsedMs: 0,
      trogdorReaderStartIndex: 3,
    }),
  );

  const text = frameText(frame);
  assert.match(text, /caught up/i);
  assert.match(text, /Read again/);
});

test("surface does not render quiet sessions as swordsmen", () => {
  const quiet = session({
    sessionId: "agent-quiet",
    name: "agent-quiet",
    state: "busy",
    displayState: "busy",
    restLabel: "active",
    actionCues: [],
    operatorPressure: null,
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [quiet],
      hoveredTrogdorSessionId: "agent-quiet",
      trogdorAtlasOpen: true,
    }),
  );

  assert.equal(frame.zones.some((zone) => zone.type === "trogdor_agent"), false);
  assert.doesNotMatch(frameText(frame), /speed read agent/i);
});

test("surface renders deep sleep sessions as hoverable swordsmen", () => {
  const deepSleep = session({
    sessionId: "agent-deep",
    name: "agent-deep",
    state: "idle",
    displayState: "idle",
    restLabel: "deep_sleep",
    thoughtLabel: "parked until the operator launches it",
    actionCues: [],
    operatorPressure: null,
    commitCandidate: false,
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [deepSleep],
      hoveredTrogdorSessionId: "agent-deep",
      trogdorAtlasOpen: true,
    }),
  );
  const text = frameText(frame);
  const actionIds = frame.zones.map((zone) => zone.actionId).filter(Boolean);

  assert.equal(
    frame.zones.some((zone) => zone.type === "trogdor_agent" && zone.sessionId === "agent-deep"),
    true,
  );
  assert.match(text, /speed read agent/i);
  assert.match(text, /deep_sleep/);
  assert.ok(actionIds.includes("trogdor_launch"));
});

test("atlas toggle keeps trogdor available after a terminal is selected", () => {
  const selected = session({
    sessionId: "agent-1",
    name: "agent-1",
    operatorPressure: {
      score: 77,
      reason: "awaiting user",
      reason_kind: "awaiting_user",
      glyph: "!",
      tone: "danger",
      needs_input: true,
      launch_ready: true,
      commit_ready: false,
      action_cue_count: 1,
    },
  });
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: selected,
      selectedSessionId: "agent-1",
      sessions: [selected],
      terminalReady: true,
      trogdorAtlasOpen: true,
    }),
  );
  const text = frameText(frame);
  const actionIds = frame.zones.map((zone) => zone.actionId).filter(Boolean);

  assert.match(text, /trogdor pressure/i);
  assert.match(text, /trogdor on/);
  assert.ok(actionIds.includes("toggle_trogdor_atlas"));
});

test("trogdor opens as an empty atlas before any sessions exist", () => {
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [],
      trogdorAtlasOpen: true,
    }),
  );
  const text = frameText(frame);
  const trogdorZone = frame.zones.find((zone) => zone.actionId === "toggle_trogdor_atlas");
  const newZone = frame.zones.find((zone) => zone.actionId === "open_create");

  assert.match(text, /trogdor pressure/i);
  assert.match(text, /no repos/i);
  assert.equal(trogdorZone?.disabled, false);
  assert.equal(newZone?.disabled, false);
});

test("top-level action chips have forgiving vertical hitboxes", () => {
  const frame = buildSurfaceFrame(
    baseModel({
      currentSession: null,
      selectedSessionId: null,
      sessions: [],
      trogdorAtlasOpen: true,
    }),
  );
  const zone = frame.zones.find((item) => item.actionId === "toggle_trogdor_atlas");

  assert.ok(zone);
  assert.equal(zone.rect.h >= 2, true);
  assert.equal(surfaceActionAt(frame.zones, { x: zone.rect.x, y: zone.rect.y + zone.rect.h - 1 })?.actionId, "toggle_trogdor_atlas");
});
