import test from "node:test";
import assert from "node:assert/strict";

import { buildSurfaceFrame } from "./rendered_surface.js";

function session(overrides = {}) {
  return {
    sessionId: "sess-1",
    name: "sess-1",
    state: "busy",
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
