import test from "node:test";
import assert from "node:assert/strict";

import { buildSurfaceFrame, computeSurfaceDirtySpans, surfaceActionAt } from "./rendered_surface.js";

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

test("surface frame reuses caller buffers without changing rendered output", () => {
  const seed = buildSurfaceFrame(baseModel());
  const reuse = { cells: seed.cells, spans: seed.spans };
  const fresh = buildSurfaceFrame(baseModel());
  const reused = buildSurfaceFrame(baseModel(), reuse);

  // Same buffer instances are reused (no per-render reallocation)...
  assert.equal(reused.cells, reuse.cells);
  assert.equal(reused.spans, reuse.spans);
  // ...and the rendered frame is byte-identical to a fresh allocation.
  assert.equal(framePatchHash(reused), framePatchHash(fresh));

  // A larger grid cannot reuse the smaller buffer; it must allocate fresh.
  const resized = buildSurfaceFrame(baseModel({ cols: 200, rows: 80 }), reuse);
  assert.notEqual(resized.cells, reuse.cells);
});

test("computeSurfaceDirtySpans reports only the changed cell runs", () => {
  const cols = 4;
  const rows = 2; // 8 cells, 32 uint32
  const prev = new Uint32Array(cols * rows * 4);
  const cur = new Uint32Array(cols * rows * 4);

  // No comparable baseline -> upload the whole grid.
  assert.deepEqual(Array.from(computeSurfaceDirtySpans(cur, null, cols, rows)), [0, 8]);
  // Identical frames -> nothing to upload.
  assert.deepEqual(Array.from(computeSurfaceDirtySpans(cur, prev, cols, rows)), []);

  // Change cell 2 and the run of cells 5-6 -> two disjoint spans.
  cur[2 * 4 + 1] = 99;
  cur[5 * 4] = 7;
  cur[6 * 4 + 3] = 7;
  assert.deepEqual(Array.from(computeSurfaceDirtySpans(cur, prev, cols, rows)), [2, 3, 5, 7]);

  // Size mismatch (resize) -> full grid.
  assert.deepEqual(Array.from(computeSurfaceDirtySpans(new Uint32Array(4), prev, cols, rows)), [0, 8]);

  // A change in the final cell extends the run to the cell count.
  const lastChanged = new Uint32Array(cols * rows * 4);
  lastChanged[7 * 4] = 1;
  assert.deepEqual(Array.from(computeSurfaceDirtySpans(lastChanged, prev, cols, rows)), [7, 8]);
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
    repoKey: "/Users/tester/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    cwdLabel: "opensource/swimmers",
  });
  const remote = session({
    sessionId: "remote",
    name: "remote-agent",
    targetLabel: "Skillbox devbox",
    repoKey: "/Users/tester/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    cwdLabel: "opensource/swimmers @ Skillbox devbox",
  });
  const frame = buildSurfaceFrame(baseModel({
    currentSession: remote,
    selectedSessionId: "remote",
    sessionGroupMode: "project",
    attentionInboxCount: 2,
    sessions: [local, remote],
    sessionRailRows: [
      {
        type: "session",
        session: local,
        group: {
          key: "/Users/tester/repos/opensource/swimmers",
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
          key: "/Users/tester/repos/opensource/swimmers",
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
  assert.match(text, /sessions grouped \/ inbox 2/);
  assert.match(text, /2x swimmers L\+Skillbox/);
  assert.ok(actionIds.includes("toggle_session_grouping"));
});

test("surface renders clickable environment matrix rows including ssh-only handoff targets", () => {
  const remote = session({
    sessionId: "remote",
    name: "remote-agent",
    state: "attention",
    displayState: "attention",
    targetLabel: "Skillbox API",
    repoKey: "/Users/tester/repos/opensource/swimmers",
    repoLabel: "opensource/swimmers",
    cwdLabel: "opensource/swimmers @ Skillbox API",
  });
  const frame = buildSurfaceFrame(baseModel({
    currentSession: null,
    selectedSessionId: null,
    sessions: [remote],
    environmentMatrix: [
      {
        id: "skillbox-api",
        displayHost: "Skillbox API",
        label: "Skillbox API",
        readinessKey: "needs_attention",
        readinessLabel: "needs attention",
        sessionCount: 1,
        degradedCount: 0,
        pathMappingCount: 2,
        capabilityLabels: ["observe", "launch", "dirs"],
      },
      {
        id: "skillbox-devbox",
        displayHost: "Skillbox devbox",
        label: "Skillbox devbox",
        readinessKey: "handoff",
        readinessLabel: "handoff",
        sessionCount: 0,
        degradedCount: 0,
        pathMappingCount: 0,
        capabilityLabels: ["ssh", "bootstrap", "external"],
        handoffOnly: true,
        attachHint: "ssh skillbox-devbox",
        bootstrapHint: "ssh skillbox-devbox 'swimmers serve'",
      },
    ],
  }));
  const text = frameText(frame);
  const handoffZone = frame.zones.find((zone) => (
    zone.actionId === "fleet_filter" && zone.kind === "target" && zone.key === "skillbox-devbox"
  ));

  assert.match(text, /envs 2 \/ handoff 1 \/ degraded 0/);
  assert.match(text, /Skillbox API 1 needs attention observe\/launch\/dirs maps 2/);
  assert.match(text, /Skillbox devbox 0 handoff ssh\/bootstrap\/external/);
  assert.match(text, /attach ssh skillbox-devbox/);
  assert.match(text, /bootstrap ssh skillbox-devbox 'swimmers serve'/);
  assert.deepEqual(
    {
      actionId: handoffZone?.actionId,
      kind: handoffZone?.kind,
      key: handoffZone?.key,
      type: handoffZone?.type,
    },
    {
      actionId: "fleet_filter",
      kind: "target",
      key: "skillbox-devbox",
      type: "environment",
    },
  );
  assert.deepEqual(
    frame.zones
      .filter((zone) => zone.actionId === "copy_environment_hint")
      .map((zone) => [zone.kind, zone.key, zone.copyText]),
    [
      ["attach", "skillbox-devbox", "ssh skillbox-devbox"],
      ["bootstrap", "skillbox-devbox", "ssh skillbox-devbox 'swimmers serve'"],
    ],
  );
});

test("surface renders down API health error with configured bootstrap hint and no fake sessions", () => {
  const frame = buildSurfaceFrame(baseModel({
    currentSession: null,
    selectedSessionId: null,
    sessions: [],
    trogdorAtlasOpen: true,
    environmentMatrix: [
      {
        id: "skillbox-api",
        displayHost: "Skillbox API",
        label: "Skillbox API",
        readinessKey: "degraded",
        readinessLabel: "degraded",
        sessionCount: 0,
        degradedCount: 0,
        pathMappingCount: 2,
        capabilityLabels: ["bootstrap", "external"],
        status: "Unavailable",
        lastError: "base_url_unavailable",
        bootstrapHint: "ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'",
      },
    ],
  }));
  const text = frameText(frame);

  assert.match(text, /Skillbox API 0 degraded bootstrap\/external maps 2/);
  assert.match(text, /health base_url_unavailable/);
  assert.match(text, /bootstrap ssh skillbox-devbox 'AUTH_TOKEN=\$AUTH_TOKEN/);
  assert.doesNotMatch(text, /Skillbox API 1/);
  assert.deepEqual(
    frame.zones
      .filter((zone) => zone.actionId === "copy_environment_hint")
      .map((zone) => [zone.kind, zone.copyText]),
    [["bootstrap", "ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'"]],
  );
});

test("surface tolerates partial fleet chip labels", () => {
  const frame = buildSurfaceFrame(baseModel({
    cols: 240,
    fleetChips: [{ kind: "target", key: "skillbox", active: true }],
  }));
  const text = frameText(frame);
  const fleetZone = frame.zones.find((zone) => zone.actionId === "fleet_filter");

  assert.match(text, /filter/);
  assert.deepEqual(
    {
      actionId: fleetZone?.actionId,
      kind: fleetZone?.kind,
      key: fleetZone?.key,
    },
    {
      actionId: "fleet_filter",
      kind: "target",
      key: "skillbox",
    },
  );
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

test("surface renders passive advisory metadata in selected session details", () => {
  const advisorySession = session({
    advisoryLabel: "c0 (external stale)",
  });
  const frame = buildSurfaceFrame(baseModel({ currentSession: advisorySession }));
  const text = frameText(frame);

  assert.match(text, /advisory/);
  assert.match(text, /c0 \(external stale\)/);
  assert.match(text, /external stale/);
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
    launchCwd: "/workspace/swimmers",
    launchTarget: "devbox",
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
  const launchZone = zones.find((zone) => zone.actionId === "trogdor_launch");

  assert.match(text, /pressure 88/);
  assert.match(text, /repo swimmers \/ commit ready/);
  assert.ok(actionIds.includes("trogdor_group_send"));
  assert.ok(actionIds.includes("trogdor_commit"));
  assert.deepEqual(batchZone.sessionIds, ["agent-1", "agent-2"]);
  assert.equal(launchZone.cwd, "/workspace/swimmers");
  assert.equal(launchZone.launchTarget, "devbox");
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

// Concatenate the glyphs of one frame row into a string, for substring checks.
function rowText(frame, row) {
  let line = "";
  for (let col = 0; col < frame.cols; col += 1) {
    const index = (row * frame.cols + col) * 4;
    line += String.fromCodePoint(frame.cells[index + 2] || 32);
  }
  return line;
}

// Concatenate the column-band glyphs (cols >= startCol) of one frame row.
function rowTextFrom(frame, row, startCol) {
  let line = "";
  for (let col = Math.max(0, startCol); col < frame.cols; col += 1) {
    const index = (row * frame.cols + col) * 4;
    line += String.fromCodePoint(frame.cells[index + 2] || 32);
  }
  return line;
}

// (tm98 item 1) — clamp divergence on extreme dims. The model dims are clamped
// once in buildSurfaceFrame (cols → [32,240], rows → [16,120]) and both the
// layout and every draw call derive from that single clamped source, so a
// pathological dim cannot overrun the frame buffer.
test("surface clamps extreme model dims to a single consistent geometry source", () => {
  const frame = buildSurfaceFrame(baseModel({ cols: 10000, rows: 9999 }));

  // Frame grid is the clamped max, not the raw model dims.
  assert.equal(frame.cols, 240);
  assert.equal(frame.rows, 120);
  assert.equal(frame.cells.length, 240 * 120 * 4);

  // Layout is computed from the clamped dims, so it stays inside the grid.
  assert.equal(frame.layout.header.x + frame.layout.header.w <= frame.cols, true);
  assert.equal(frame.layout.footer.y + frame.layout.footer.h <= frame.rows, true);
  assert.equal(frame.layout.center.x + frame.layout.center.w <= frame.cols, true);
  assert.equal(frame.layout.center.y + frame.layout.center.h <= frame.rows, true);

  // Every published hit zone resolves inside the clamped grid (no out-of-bounds
  // frame writes / divergent geometry from a raw 10000-col model).
  for (const zone of frame.zones) {
    assert.equal(zone.rect.x >= 0, true);
    assert.equal(zone.rect.y >= 0, true);
    assert.equal(zone.rect.x + zone.rect.w <= frame.cols, true);
    assert.equal(zone.rect.y + zone.rect.h <= frame.rows, true);
  }

  // Below the floor clamps identically: tiny/negative dims snap to the minimum.
  const tiny = buildSurfaceFrame(baseModel({ cols: 1, rows: -5 }));
  assert.equal(tiny.cols, 32);
  assert.equal(tiny.rows, 16);
  assert.equal(tiny.cells.length, 32 * 16 * 4);
});

// (tm98 item 2) — center-overlay overrun. On a clamped-small terminal the
// trogdor overview overlay must stay inside layout.center instead of bleeding
// its box/text into the footer or right edge.
test("surface clamps the trogdor overview overlay to the center rect on tiny terminals", () => {
  // rows=17 gives center.h=6 with the center bottom (row 12) at the footer top,
  // so a too-tall overlay (the old Math.max(8, center.h-2)=8) would bleed its
  // unique banner/title text down into the footer rows. cols=140 keeps the
  // center wide so this isolates the vertical clamp.
  const overlaySession = session({
    sessionId: "agent-1",
    name: "agent-1",
    state: "attention",
    displayState: "attention",
  });
  const frame = buildSurfaceFrame(
    baseModel({
      cols: 140,
      rows: 17,
      currentSession: null,
      selectedSessionId: null,
      sessions: [overlaySession],
      trogdorAtlasOpen: true,
    }),
  );
  const center = frame.layout.center;

  // The clamped layout really is small enough to exercise the overrun guard.
  assert.equal(center.h, 6);

  // The overlay still renders its unique banner/panel inside the center. (At
  // this cramped height the atlas/title lines collapse, but the overlay box and
  // banner remain — the point is they stay put, not that every line survives.)
  assert.match(frameText(frame), /burninate/i);
  assert.match(frameText(frame), /overview/i);

  // None of the overlay-unique text bleeds into the rows below the center (where
  // a too-tall overlay / fixed-offset atlas used to overrun the footer). "cues"
  // is the atlas header, which previously leaked below the overlay.
  const centerBottom = center.y + center.h;
  for (let row = centerBottom; row < frame.rows; row += 1) {
    const line = rowText(frame, row);
    assert.doesNotMatch(line, /burninate/i, `overlay banner bled into row ${row}`);
    assert.doesNotMatch(line, /trogdor pressure/i, `overlay title bled into row ${row}`);
    assert.doesNotMatch(line, /overview/i, `overlay panel bled into row ${row}`);
    assert.doesNotMatch(line, /cues \d/i, `atlas header bled into row ${row}`);
  }

  // Nothing bleeds into the columns to the right of the center either.
  const centerRight = center.x + center.w;
  for (let row = 0; row < frame.rows; row += 1) {
    const band = rowTextFrom(frame, row, centerRight);
    assert.doesNotMatch(band, /burninate/i, `overlay banner bled right of center on row ${row}`);
    assert.doesNotMatch(band, /overview/i, `overlay panel bled right of center on row ${row}`);
    assert.doesNotMatch(band, /cues \d/i, `atlas header bled right of center on row ${row}`);
  }
});

// (tm98 item 4) — selectionFocus scroll edge cases. The visible-window start is
// clamped to [0, max(0, rows.length - visibleCount)] so first/last/overflow
// selection never produces a negative or out-of-range window.
function railSessions(count) {
  return Array.from({ length: count }, (_value, index) =>
    session({ sessionId: `s-${index}`, name: `s-${index}` }),
  );
}

function visibleRailSessionIds(frame) {
  return frame.zones
    .filter((zone) => zone.type === "session")
    .map((zone) => zone.sessionId);
}

test("session rail selection-focus window stays in range for first/last/overflow", () => {
  const sessions = railSessions(20);

  const renderRail = (selectedSessionId) =>
    buildSurfaceFrame(
      baseModel({
        cols: 120,
        rows: 40,
        sessions,
        currentSession: sessions.find((item) => item.sessionId === selectedSessionId) || sessions[0],
        selectedSessionId,
      }),
    );

  // First row selected: window starts at the top, includes the first session.
  const firstFrame = renderRail("s-0");
  const firstVisible = visibleRailSessionIds(firstFrame);
  assert.ok(firstVisible.length >= 1);
  assert.equal(firstVisible[0], "s-0");

  // Last row selected: window is clamped so the final session is visible.
  const lastFrame = renderRail("s-19");
  const lastVisible = visibleRailSessionIds(lastFrame);
  assert.ok(lastVisible.includes("s-19"));

  // Unknown selection (findIndex -> -1) falls back to the top without going
  // negative.
  const unknownFrame = renderRail("does-not-exist");
  const unknownVisible = visibleRailSessionIds(unknownFrame);
  assert.equal(unknownVisible[0], "s-0");

  // Overflow: visibleCount >= rows.length. A tall rail with few sessions shows
  // all of them starting at index 0 (no negative start, no out-of-range end).
  const fewSessions = railSessions(2);
  const overflowFrame = buildSurfaceFrame(
    baseModel({
      cols: 120,
      rows: 40,
      sessions: fewSessions,
      currentSession: fewSessions[1],
      selectedSessionId: "s-1",
    }),
  );
  const overflowVisible = visibleRailSessionIds(overflowFrame);
  assert.deepEqual(overflowVisible, ["s-0", "s-1"]);
});

// (tm98 item 3) — zoom-step rounding is OUT OF SCOPE for this file. The
// rendered surface carries no zoom/scale geometry; the zoom step lives in
// terminal_zoom_input.js / global_shortcut_dispatch.js, which are off-limits
// under this bead's edit constraint. Recorded here so the gap is explicit.
