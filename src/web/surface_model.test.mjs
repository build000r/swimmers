import test from "node:test";
import assert from "node:assert/strict";

import {
  buildSurfaceModel,
  surfaceSession,
} from "./surface_model.js";

function rawSession(overrides = {}) {
  return {
    session_id: "sess-1",
    tmux_name: "agent-one",
    state: "attention",
    state_evidence: {
      confidence: "high",
      observed_at: "2026-06-03T00:00:00Z",
      cause: "liveness_has_children",
    },
    rest_state: "active",
    transport_health: "healthy",
    tool: "codex",
    cwd: "/srv/skillbox/repos/swimmers",
    thought: "approve migration before commit",
    thought_updated_at: "2026-06-03T00:00:00Z",
    objective_changed_at: "2026-06-02T00:00:00Z",
    token_count: 10,
    context_limit: 100,
    last_skill: "describe",
    last_activity_at: "raw-time",
    current_command: "cargo test",
    attached_clients: 2,
    commit_candidate: true,
    action_cues: [{ kind: "awaiting_user" }],
    is_stale: true,
    ...overrides,
  };
}

function baseState(overrides = {}) {
  return {
    sessions: [rawSession()],
    currentCols: 120,
    currentRows: 36,
    followPublishedSelection: true,
    connectionLabel: "attached",
    connectionMuted: false,
    modeLabel: "operator",
    modeMuted: false,
    searchLabel: "Search idle",
    searchMuted: true,
    utilityLabel: "ready",
    utilityMuted: true,
    searchQuery: "needle",
    selectMode: true,
    readOnly: false,
    activeSheet: "send",
    hoveredLinkUrl: "https://example.test/repo",
    hoveredTrogdorSessionId: "sess-1",
    trogdorAtlasOpen: true,
    trogdorWpm: 225,
    trogdorReading: true,
    trogdorReaderStartIndex: 3,
    trogdorReaderStartedAt: 1000,
    trogdorDismissedClawgs: {},
    trogdorReadProgress: {},
    selectedSessionId: "sess-1",
    publishedSelection: {
      session_id: "sess-2",
      published_at: "published-raw",
    },
    terminal: {},
    ws: { readyState: 7 },
    ...overrides,
  };
}

test("surfaceSession preserves labels, pressure fields, and Trogdor enrichment", () => {
  const pressure = {
    pressure: { reason_kind: "awaiting_user", score: 91 },
    batch_send_session_ids: ["sess-1", "sess-2"],
    repo_key: "repo-key",
    repo_label: "swimmers",
  };

  const session = surfaceSession(rawSession({ action_cues: [] }), {
    operatorPressure: pressure,
    sessionBurnt: () => true,
  });

  assert.equal(session.sessionId, "sess-1");
  assert.equal(session.name, "agent-one");
  assert.equal(session.displayState, "attention");
  assert.equal(session.stateTrustLabel, "high observed liveness_has_children");
  assert.equal(session.cwdLabel, "repos/swimmers");
  assert.equal(session.fullCwd, "/srv/skillbox/repos/swimmers");
  assert.equal(session.thoughtLabel, "approve migration before commit");
  assert.equal(session.activityLabel, "raw-time");
  assert.equal(session.contextLabel, "10 / 100");
  assert.equal(session.skillLabel, "describe");
  assert.equal(session.attachedLabel, "2");
  assert.equal(session.commitCandidate, true);
  assert.deepEqual(session.operatorPressure, { reason_kind: "awaiting_user", score: 91 });
  assert.deepEqual(session.batchSendSessionIds, ["sess-1", "sess-2"]);
  assert.equal(session.repoKey, "repo-key");
  assert.equal(session.repoLabel, "swimmers");
  assert.equal(session.isStale, true);
  assert.equal(session.trogdorAwaitingUser, true);
  assert.equal(session.trogdorBurnt, true);
  assert.equal(session.trogdorSwordsmanVisible, true);
  assert.equal(session.clawgWordCount, 4);
});

test("surfaceSession marks low or unobserved state as uncertain and keeps detail thought full", () => {
  const longThought = `${"x".repeat(120)} final`;
  const raw = rawSession({
    state: "busy",
    state_evidence: { confidence: "medium", observed_at: "", cause: "summary_cache" },
    thought: longThought,
  });

  const summary = surfaceSession(raw);
  const detail = surfaceSession(raw, { detail: true });

  assert.equal(summary.displayState, "busy?");
  assert.equal(summary.stateTrustLabel, "medium unobserved summary_cache");
  assert.equal(summary.stateConfidence, "medium");
  assert.equal(summary.stateObserved, false);
  assert.equal(summary.thoughtLabel.length, 110);
  assert.match(summary.thoughtLabel, /\.\.\.$/);
  assert.equal(detail.thoughtLabel, longThought);
});

test("buildSurfaceModel preserves selected/current session, terminal, reader, and HUD fields", () => {
  const selected = rawSession({ thought: `${"selected ".repeat(16)}done` });
  const state = baseState({
    sessions: [
      selected,
      rawSession({ session_id: "sess-2", tmux_name: "agent-two", cwd: "/tmp/other" }),
    ],
  });
  const pressureBySession = new Map([
    ["sess-1", {
      pressure: { reason_kind: "commit_ready", score: 70 },
      repo_label: "swimmers",
    }],
  ]);

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: true, franken_term_available: true },
    currentSession: () => selected,
    operatorPressureSnapshot: (sessionId) => pressureBySession.get(sessionId) || null,
    sessionBurnt: (session) => session.sessionId === "sess-2",
    normalizeSessionId: (sessionId) => String(sessionId || "").trim() || null,
    now: () => 1450,
    websocketOpen: 7,
  });

  assert.equal(model.cols, 120);
  assert.equal(model.rows, 36);
  assert.equal(model.focusLayout, true);
  assert.equal(model.followPublishedSelection, true);
  assert.equal(model.terminalReady, true);
  assert.equal(model.snapshotFallback, false);
  assert.equal(model.activeSheet, "send");
  assert.equal(model.hoveredLinkUrl, "https://example.test/repo");
  assert.equal(model.hoveredTrogdorSessionId, "sess-1");
  assert.equal(model.trogdorAtlasOpen, true);
  assert.equal(model.trogdorWpm, 225);
  assert.equal(model.trogdorReading, true);
  assert.equal(model.trogdorReaderStartIndex, 3);
  assert.equal(model.trogdorReaderElapsedMs, 450);
  assert.equal(model.selectedSessionId, "sess-1");
  assert.equal(model.publishedSessionId, "sess-2");
  assert.equal(model.publishedAtLabel, "published-raw");
  assert.equal(model.sessions.length, 2);
  assert.equal(model.sessions[0].operatorPressure.reason_kind, "commit_ready");
  assert.equal(model.sessions[1].trogdorBurnt, true);
  assert.equal(model.currentSession.sessionId, "sess-1");
  assert.equal(model.currentSession.thoughtLabel, selected.thought);
  assert.notEqual(model.sessions[0].thoughtLabel, selected.thought);
});

test("buildSurfaceModel reports terminal not ready without the expected websocket state", () => {
  const state = baseState({
    hoveredTrogdorSessionId: null,
    ws: { readyState: 2 },
  });

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: true, franken_term_available: false },
    currentSession: () => null,
    now: () => 2000,
    websocketOpen: 7,
  });

  assert.equal(model.terminalReady, false);
  assert.equal(model.snapshotFallback, true);
  assert.equal(model.currentSession, null);
  assert.equal(model.trogdorReaderElapsedMs, 0);
});
