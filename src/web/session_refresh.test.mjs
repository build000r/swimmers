import test from "node:test";
import assert from "node:assert/strict";

import {
  runSessionRefresh,
  sessionRefreshErrorPlan,
  sessionRefreshRequestPlan,
  sessionRefreshSelectionPlan,
  sessionRefreshSuccessStatusPlan,
} from "./session_refresh.js";

test("sessionRefreshRequestPlan preserves refresh endpoint ordering data", () => {
  assert.deepEqual(sessionRefreshRequestPlan(false), {
    sessionsPath: "/v1/sessions",
    operatorPressurePath: "/v1/operator-pressure",
    healthPath: "/health",
    selectionPath: null,
  });
  assert.deepEqual(sessionRefreshRequestPlan(true).selectionPath, "/v1/selection");
});

test("sessionRefreshSelectionPlan preserves published and local selection decisions", () => {
  assert.deepEqual(sessionRefreshSelectionPlan({
    hasPublishedResponse: true,
    publishedSelection: { session_id: "agent-1" },
    publishedSessionId: "agent-1",
    publishedSessionExists: true,
  }), {
    publishedSelection: { session_id: "agent-1" },
    persistSelectedSession: "agent-1",
  });
  assert.deepEqual(sessionRefreshSelectionPlan({
    hasPublishedResponse: true,
    publishedSelection: { session_id: "missing" },
    publishedSessionId: "missing",
    publishedSessionExists: false,
  }), {
    publishedSelection: { session_id: "missing" },
    persistSelectedSession: null,
  });
  assert.deepEqual(sessionRefreshSelectionPlan({
    selectedSessionId: "agent-2",
    selectedSessionExists: true,
    fallbackSessionId: "agent-1",
  }), {
    publishedSelection: null,
  });
  assert.deepEqual(sessionRefreshSelectionPlan({
    selectedSessionId: "missing",
    selectedSessionExists: false,
    trogdorAtlasOpen: false,
    fallbackSessionId: "agent-1",
  }), {
    publishedSelection: null,
    persistSelectedSession: "agent-1",
  });
  assert.deepEqual(sessionRefreshSelectionPlan({
    selectedSessionId: "",
    selectedSessionExists: false,
    trogdorAtlasOpen: true,
    fallbackSessionId: "agent-1",
  }), {
    publishedSelection: null,
    persistSelectedSession: null,
  });
});

test("sessionRefreshSuccessStatusPlan and sessionRefreshErrorPlan preserve status labels", () => {
  assert.deepEqual(sessionRefreshSuccessStatusPlan({
    followPublishedSelection: true,
    selectedSessionId: "",
    readOnly: true,
    token: "",
  }), {
    connection: { label: "waiting", muted: true },
    mode: { label: "observer", muted: true },
  });
  assert.deepEqual(sessionRefreshSuccessStatusPlan({
    selectedSessionId: "agent-1",
    readOnly: false,
    token: "secret",
  }), {
    connection: { label: "live", muted: false },
    mode: { label: "operator", muted: false },
  });
  assert.deepEqual(sessionRefreshErrorPlan({ status: 403 }), {
    connection: { label: "auth required", muted: true },
    mode: { label: "token needed", muted: false },
  });
  assert.deepEqual(sessionRefreshErrorPlan({ status: 500 }), {
    connection: { label: "backend unavailable", muted: true },
    mode: { label: "offline", muted: true },
  });
});

test("runSessionRefresh preserves successful refresh ordering and status side effects", async () => {
  const calls = [];
  const runtime = {
    state: {
      followPublishedSelection: false,
      sessions: [],
      selectedSessionId: null,
      trogdorAtlasOpen: false,
      readOnly: false,
      token: "token",
    },
    apiFetch: async (path) => {
      calls.push(["apiFetch", path]);
      return {
        path,
        json: async () => (
          path === "/v1/selection"
            ? { session_id: "agent-1" }
            : {
                sessions: [{ session_id: "agent-1" }],
                environments: [{ id: "skillbox", backend_mode: "remote_swimmers_api" }],
              }
        ),
      };
    },
    apiMaybeFetch: async (path) => {
      calls.push(["apiMaybeFetch", path]);
      return {
        path,
        json: async () => (
          path === "/v1/operator-pressure"
            ? {
                sessions: [{
                  session_id: "agent-1",
                  repo_key: 7,
                  repo_label: null,
                  pressure: { score: "8", reason_kind: "awaiting_user" },
                  batch_send_session_ids: ["agent-1", null, "agent-2"],
                }],
              }
            : { persistence: { available: true, ok: true } }
        ),
      };
    },
    responseJson: async (response, normalizer) => {
      calls.push(["responseJson", response.path]);
      return normalizer(await response.json());
    },
    responseJsonOrNull: async (response, normalizer = (value) => value) => {
      calls.push(["responseJsonOrNull", response.path]);
      return normalizer(await response.json());
    },
    applyOperatorPressure: (payload) => calls.push(["applyOperatorPressure", payload]),
    applyBackendHealth: (payload) => calls.push(["applyBackendHealth", payload]),
    syncTrogdorCueTransitions: () => calls.push(["syncTrogdorCueTransitions"]),
    normalizeSessionId: (sessionId) => sessionId || null,
    sessionExists: (sessionId) => runtime.state.sessions.some((session) => session.session_id === sessionId),
    persistSelectedSession: (sessionId) => {
      calls.push(["persistSelectedSession", sessionId]);
      runtime.state.selectedSessionId = sessionId;
    },
    setupHudSurface: async () => calls.push(["setupHudSurface"]),
    renderHudSurface: () => calls.push(["renderHudSurface"]),
    syncTerminalTools: () => calls.push(["syncTerminalTools"]),
    connectSelectedSession: async () => calls.push(["connectSelectedSession"]),
    refreshAgentContextForSelectedSession: (options) => calls.push(["refreshAgentContextForSelectedSession", options]),
    refreshWorkbenchWidgetsForSelectedSession: (options) => calls.push(["refreshWorkbenchWidgetsForSelectedSession", options]),
    setConnectionStatus: (label, muted) => calls.push(["setConnectionStatus", label, muted]),
    setModeStatus: (label, muted) => calls.push(["setModeStatus", label, muted]),
  };

  await runSessionRefresh(runtime);

  assert.equal(runtime.state.environments[0].id, "skillbox");
  assert.equal(runtime.state.environments[0].backend_mode, "remote_swimmers_api");
  assert.deepEqual(calls, [
    ["apiFetch", "/v1/sessions"],
    ["apiMaybeFetch", "/v1/operator-pressure"],
    ["apiMaybeFetch", "/health"],
    ["responseJson", "/v1/sessions"],
    ["responseJsonOrNull", "/v1/operator-pressure"],
    ["responseJsonOrNull", "/health"],
    ["applyOperatorPressure", {
      sessions: [{
        session_id: "agent-1",
        repo_key: "7",
        repo_label: "",
        pressure: {
          score: 8,
          reason: "",
          reason_kind: "awaiting_user",
          glyph: "a",
          tone: "quiet",
          needs_input: false,
          launch_ready: false,
          commit_ready: false,
          action_cue_count: 0,
        },
        batch_send_session_ids: ["agent-1", "agent-2"],
      }],
      repos: [],
      inbox: [],
      summary: {
        max_score: 0,
        action_cues: 0,
        batch_send_groups: 0,
      },
    }],
    ["applyBackendHealth", { persistence: { available: true, ok: true } }],
    ["syncTrogdorCueTransitions"],
    ["persistSelectedSession", "agent-1"],
    ["setupHudSurface"],
    ["renderHudSurface"],
    ["syncTerminalTools"],
    ["connectSelectedSession"],
    ["refreshAgentContextForSelectedSession", { throttle: true, silent: true }],
    ["refreshWorkbenchWidgetsForSelectedSession", { throttle: true, silent: true }],
    ["setConnectionStatus", "live", false],
    ["setModeStatus", "operator", false],
  ]);
});

test("runSessionRefresh preserves refresh error reset behavior", async () => {
  const calls = [];
  const runtime = {
    state: {
      sessions: [{ session_id: "agent-1" }],
      environments: [{ id: "skillbox" }],
      operatorPressureBySession: new Map([["agent-1", {}]]),
      backendHealth: {},
      publishedSelection: { session_id: "agent-1" },
      followPublishedSelection: false,
    },
    apiFetch: async () => {
      throw { status: 401 };
    },
    apiMaybeFetch: async () => null,
    persistSelectedSession: (sessionId) => calls.push(["persistSelectedSession", sessionId]),
    resetAgentContextForSession: (sessionId) => calls.push(["resetAgentContextForSession", sessionId]),
    resetWorkbenchWidgetsForSession: (sessionId) => calls.push(["resetWorkbenchWidgetsForSession", sessionId]),
    renderHudSurface: () => calls.push(["renderHudSurface"]),
    setConnectionStatus: (label, muted) => calls.push(["setConnectionStatus", label, muted]),
    setModeStatus: (label, muted) => calls.push(["setModeStatus", label, muted]),
  };

  await runSessionRefresh(runtime);

  assert.deepEqual(runtime.state.sessions, []);
  assert.deepEqual(runtime.state.environments, []);
  assert.equal(runtime.state.operatorPressureBySession.size, 0);
  assert.equal(runtime.state.backendHealth, null);
  assert.equal(runtime.state.publishedSelection, null);
  assert.deepEqual(calls, [
    ["persistSelectedSession", null],
    ["resetAgentContextForSession", null],
    ["resetWorkbenchWidgetsForSession", null],
    ["renderHudSurface"],
    ["setConnectionStatus", "auth required", true],
    ["setModeStatus", "token needed", false],
  ]);
});
