import test from "node:test";
import assert from "node:assert/strict";

import {
  agentContextRefreshStalePlan,
  agentContextRefreshStartPlan,
  agentContextRequestPath,
  runAgentContextRefresh,
} from "./agent_context_refresh.js";

function buildRuntime(overrides = {}) {
  const calls = [];
  const state = {
    selectedSessionId: "sess_0",
    trogdorAtlasOpen: false,
    agentContextSessionId: null,
    agentContextLoading: false,
    agentContextPayload: null,
    agentContextError: "",
    agentContextRequestSeq: 0,
    agentContextLastLoadedAt: 0,
    ...overrides.state,
  };
  const runtime = {
    state,
    throttleMs: 5000,
    now: overrides.now || (() => 4242),
    currentSession: () => Object.prototype.hasOwnProperty.call(overrides, "session")
      ? overrides.session
      : { session_id: "sess_0" },
    renderTerminalWorkbench: () => {
      calls.push(["renderTerminalWorkbench"]);
      overrides.onRender?.(state);
    },
    apiFetch: async (path) => {
      calls.push(["apiFetch", path]);
      if (overrides.fetchError) {
        throw overrides.fetchError;
      }
      return {
        json: async () => {
          calls.push(["json"]);
          if (overrides.onJson) {
            overrides.onJson(runtime);
          }
          return overrides.payload ?? { available: true, session_id: "sess_0" };
        },
      };
    },
    responseJson: async (response, normalizer) => {
      calls.push(["responseJson"]);
      return normalizer(await response.json());
    },
  };
  return { calls, runtime };
}

test("agent context plans preserve request path, start gates, and stale guards", () => {
  assert.equal(agentContextRequestPath("sess 0/a"), "/v1/sessions/sess%200%2Fa/agent-context");
  assert.deepEqual(agentContextRefreshStartPlan({ hasSession: false }), { type: "reset_and_render" });
  assert.deepEqual(agentContextRefreshStartPlan({ hasSession: true, trogdorAtlasOpen: true }), {
    type: "reset_and_render",
  });
  assert.deepEqual(agentContextRefreshStartPlan({ hasSession: true, throttled: true }), { type: "ignore" });
  assert.deepEqual(agentContextRefreshStartPlan({ hasSession: true, loadingBlocked: true }), { type: "ignore" });
  assert.deepEqual(agentContextRefreshStartPlan({
    hasSession: true,
    sessionId: "sess_0",
    requestSeq: 4,
    silent: true,
    hasCurrentPayload: true,
  }), {
    type: "start_refresh",
    sessionId: "sess_0",
    requestSeq: 5,
    loading: false,
  });
  assert.deepEqual(agentContextRefreshStalePlan({
    requestSeq: 2,
    currentRequestSeq: 2,
    selectedSessionId: "sess_0",
    sessionId: "sess_0",
  }), { stale: false });
  assert.deepEqual(agentContextRefreshStalePlan({
    requestSeq: 2,
    currentRequestSeq: 3,
    selectedSessionId: "sess_0",
    sessionId: "sess_0",
  }), { stale: true });
  assert.deepEqual(agentContextRefreshStalePlan({
    requestSeq: 2,
    currentRequestSeq: 2,
    selectedSessionId: "other",
    sessionId: "sess_0",
  }), { stale: true });
});

test("runAgentContextRefresh preserves no-session reset behavior", async () => {
  let nowCalls = 0;
  const { calls, runtime } = buildRuntime({
    session: null,
    now: () => {
      nowCalls += 1;
      return 4242;
    },
    state: {
      selectedSessionId: null,
      agentContextSessionId: "old",
      agentContextLoading: true,
      agentContextPayload: { available: true },
      agentContextError: "old error",
      agentContextLastLoadedAt: 123,
    },
  });

  await runAgentContextRefresh({ force: true }, runtime);

  assert.equal(runtime.state.agentContextLoading, false);
  assert.equal(runtime.state.agentContextSessionId, "old");
  assert.deepEqual(runtime.state.agentContextPayload, { available: true });
  assert.equal(runtime.state.agentContextError, "old error");
  assert.equal(runtime.state.agentContextLastLoadedAt, 123);
  assert.equal(nowCalls, 0);
  assert.deepEqual(calls, [["renderTerminalWorkbench"]]);
});

test("runAgentContextRefresh preserves Atlas unsupported reset behavior", async () => {
  let nowCalls = 0;
  const { calls, runtime } = buildRuntime({
    now: () => {
      nowCalls += 1;
      return 4242;
    },
    state: {
      trogdorAtlasOpen: true,
      agentContextLoading: true,
      agentContextPayload: { available: true },
    },
  });

  await runAgentContextRefresh({ force: true }, runtime);

  assert.equal(runtime.state.agentContextLoading, false);
  assert.deepEqual(runtime.state.agentContextPayload, { available: true });
  assert.equal(nowCalls, 0);
  assert.deepEqual(calls, [["renderTerminalWorkbench"]]);
});

test("runAgentContextRefresh preserves successful payload and loading transitions", async () => {
  const payload = { available: true, session_id: "sess_0", user_task: "build" };
  const expectedPayload = {
    available: true,
    session_id: "sess_0",
    tool: null,
    cwd: "",
    user_task: "build",
    turns: [],
    current_tool: null,
    recent_actions: [],
    token_count: 0,
    context_limit: 0,
    message: null,
  };
  const renderStates = [];
  const { calls, runtime } = buildRuntime({
    payload,
    onRender: (state) => renderStates.push({
      loading: state.agentContextLoading,
      payload: state.agentContextPayload,
      error: state.agentContextError,
    }),
  });

  await runAgentContextRefresh({ force: true }, runtime);

  assert.equal(runtime.state.agentContextRequestSeq, 1);
  assert.equal(runtime.state.agentContextSessionId, "sess_0");
  assert.equal(runtime.state.agentContextLoading, false);
  assert.deepEqual(runtime.state.agentContextPayload, expectedPayload);
  assert.equal(runtime.state.agentContextError, "");
  assert.equal(runtime.state.agentContextLastLoadedAt, 4242);
  assert.deepEqual(renderStates, [
    { loading: true, payload: null, error: "" },
    { loading: false, payload: expectedPayload, error: "" },
  ]);
  assert.deepEqual(calls, [
    ["renderTerminalWorkbench"],
    ["apiFetch", "/v1/sessions/sess_0/agent-context"],
    ["responseJson"],
    ["json"],
    ["renderTerminalWorkbench"],
  ]);
});

test("runAgentContextRefresh clears previous-session payload before loading render", async () => {
  const renderStates = [];
  const { runtime } = buildRuntime({
    session: { session_id: "sess_1" },
    payload: { available: true, session_id: "sess_1", user_task: "new task" },
    state: {
      selectedSessionId: "sess_1",
      agentContextSessionId: "sess_0",
      agentContextPayload: { available: true, session_id: "sess_0", user_task: "old task" },
      agentContextLastLoadedAt: 111,
    },
    onRender: (state) => renderStates.push({
      sessionId: state.agentContextSessionId,
      loading: state.agentContextLoading,
      payload: state.agentContextPayload
        ? {
            session_id: state.agentContextPayload.session_id,
            user_task: state.agentContextPayload.user_task,
          }
        : null,
    }),
  });

  await runAgentContextRefresh({ force: true }, runtime);

  assert.deepEqual(renderStates[0], {
    sessionId: "sess_1",
    loading: true,
    payload: null,
  });
  assert.deepEqual(renderStates[1], {
    sessionId: "sess_1",
    loading: false,
    payload: {
      session_id: "sess_1",
      user_task: "new task",
    },
  });
});

test("runAgentContextRefresh preserves throttle no-op behavior", async () => {
  const payload = { available: true, session_id: "sess_0" };
  const { calls, runtime } = buildRuntime({
    now: () => 6000,
    state: {
      agentContextSessionId: "sess_0",
      agentContextPayload: payload,
      agentContextLastLoadedAt: 2000,
    },
  });

  await runAgentContextRefresh({ throttle: true, silent: true }, runtime);

  assert.equal(runtime.state.agentContextRequestSeq, 0);
  assert.equal(runtime.state.agentContextLoading, false);
  assert.deepEqual(runtime.state.agentContextPayload, payload);
  assert.deepEqual(calls, []);
});

test("runAgentContextRefresh preserves stale selected-session guard", async () => {
  const { calls, runtime } = buildRuntime({
    state: {
      agentContextSessionId: "sess_0",
      agentContextPayload: { available: true, old: true },
      agentContextLastLoadedAt: 111,
    },
    payload: { available: true, new: true },
    onJson: (targetRuntime) => {
      targetRuntime.state.selectedSessionId = "other";
    },
  });

  await runAgentContextRefresh({ force: true }, runtime);

  assert.deepEqual(runtime.state.agentContextPayload, { available: true, old: true });
  assert.equal(runtime.state.agentContextError, "");
  assert.equal(runtime.state.agentContextLastLoadedAt, 111);
  assert.equal(runtime.state.agentContextLoading, false);
  assert.deepEqual(calls, [
    ["renderTerminalWorkbench"],
    ["apiFetch", "/v1/sessions/sess_0/agent-context"],
    ["responseJson"],
    ["json"],
    ["renderTerminalWorkbench"],
  ]);
});

test("runAgentContextRefresh preserves fetch error handling", async () => {
  const { calls, runtime } = buildRuntime({
    state: { agentContextPayload: { available: true } },
    fetchError: new Error("backend offline"),
  });

  await runAgentContextRefresh({ force: true }, runtime);

  assert.equal(runtime.state.agentContextLoading, false);
  assert.equal(runtime.state.agentContextPayload, null);
  assert.equal(runtime.state.agentContextError, "backend offline");
  assert.deepEqual(calls, [
    ["renderTerminalWorkbench"],
    ["apiFetch", "/v1/sessions/sess_0/agent-context"],
    ["renderTerminalWorkbench"],
  ]);
});
