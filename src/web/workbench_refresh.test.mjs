import test from "node:test";
import assert from "node:assert/strict";

import {
  runWorkbenchWidgetRefresh,
  workbenchRefreshStalePlan,
  workbenchRefreshStartPlan,
} from "./workbench_refresh.js";

function buildRuntime(overrides = {}) {
  const calls = [];
  const state = {
    selectedSessionId: "sess_0",
    trogdorAtlasOpen: false,
    workbenchSelectedTurnId: "",
    workbenchWidgets: {
      sessionId: null,
      loading: false,
      requestSeq: 0,
      lastLoadedAt: 0,
      transcript: null,
      transcriptTurnId: "",
      transcriptNextCursor: 0,
    },
    ...overrides.state,
  };
  const runtime = {
    state,
    throttleMs: 5000,
    now: () => 4242,
    currentSession: () => Object.prototype.hasOwnProperty.call(overrides, "session")
      ? overrides.session
      : { session_id: "sess_0" },
    renderWorkbenchWidgets: () => calls.push(["renderWorkbenchWidgets"]),
    apiMaybeFetch: async (path) => {
      calls.push(["apiMaybeFetch", path]);
      return { path };
    },
    responseJsonOrNull: async (response) => {
      calls.push(["responseJsonOrNull", response.path]);
      if (response.path.includes("/transcript?")) {
        return { selected_turn_id: "", next_cursor: 12, records: [] };
      }
      return { path: response.path };
    },
  };
  return { calls, runtime };
}

test("workbenchRefreshStartPlan preserves no-session, throttle, loading, and start gates", () => {
  assert.deepEqual(workbenchRefreshStartPlan({ hasSession: false }), { type: "reset_and_render" });
  assert.deepEqual(workbenchRefreshStartPlan({ hasSession: true, trogdorAtlasOpen: true }), {
    type: "reset_and_render",
  });
  assert.deepEqual(workbenchRefreshStartPlan({ hasSession: true, throttled: true }), { type: "ignore" });
  assert.deepEqual(workbenchRefreshStartPlan({ hasSession: true, loadingBlocked: true }), { type: "ignore" });
  // A collapsed panel skips background refreshes, but a forced refresh (just
  // opened) still loads.
  assert.deepEqual(workbenchRefreshStartPlan({ hasSession: true, workbenchClosed: true }), {
    type: "ignore",
  });
  assert.deepEqual(
    workbenchRefreshStartPlan({ hasSession: true, workbenchClosed: true, force: true, sessionId: "sess_0" }),
    { type: "start_refresh", sessionId: "sess_0", requestSeq: 1, loading: true },
  );
  assert.deepEqual(workbenchRefreshStartPlan({
    hasSession: true,
    sessionId: "sess_0",
    requestSeq: 4,
    silent: true,
  }), {
    type: "start_refresh",
    sessionId: "sess_0",
    requestSeq: 5,
    loading: false,
  });
});

test("workbenchRefreshStalePlan preserves request and selected-session stale guards", () => {
  assert.deepEqual(workbenchRefreshStalePlan({
    requestSeq: 2,
    currentRequestSeq: 2,
    selectedSessionId: "sess_0",
    sessionId: "sess_0",
  }), { stale: false, clearLoading: true });
  assert.deepEqual(workbenchRefreshStalePlan({
    requestSeq: 2,
    currentRequestSeq: 3,
    selectedSessionId: "sess_0",
    sessionId: "sess_0",
  }), { stale: true, clearLoading: false });
  assert.deepEqual(workbenchRefreshStalePlan({
    requestSeq: 2,
    currentRequestSeq: 2,
    selectedSessionId: "other",
    sessionId: "sess_0",
  }), { stale: true, clearLoading: true });
});

test("runWorkbenchWidgetRefresh preserves no-session reset behavior", async () => {
  const { calls, runtime } = buildRuntime({
    session: null,
    state: { selectedSessionId: null, workbenchWidgets: { loading: true, requestSeq: 7 } },
  });

  await runWorkbenchWidgetRefresh({}, runtime);

  assert.equal(runtime.state.workbenchWidgets.loading, false);
  assert.deepEqual(calls, [["renderWorkbenchWidgets"]]);
});

test("runWorkbenchWidgetRefresh preserves request ordering and result application", async () => {
  const { calls, runtime } = buildRuntime();

  await runWorkbenchWidgetRefresh({}, runtime);

  assert.equal(runtime.state.workbenchWidgets.requestSeq, 1);
  assert.equal(runtime.state.workbenchWidgets.sessionId, "sess_0");
  assert.equal(runtime.state.workbenchWidgets.loading, false);
  assert.equal(runtime.state.workbenchWidgets.lastLoadedAt, 4242);
  assert.deepEqual(calls, [
    ["renderWorkbenchWidgets"],
    ["apiMaybeFetch", "/v1/sessions/sess_0/timeline"],
    ["apiMaybeFetch", "/v1/sessions/sess_0/skills?source=sbp"],
    ["apiMaybeFetch", "/v1/sessions/sess_0/pane-tail"],
    ["apiMaybeFetch", "/v1/sessions/sess_0/transcript?limit=160"],
    ["apiMaybeFetch", "/v1/sessions/sess_0/mermaid-artifact"],
    ["apiMaybeFetch", "/v1/sessions/sess_0/git-diff"],
    ["responseJsonOrNull", "/v1/sessions/sess_0/timeline"],
    ["responseJsonOrNull", "/v1/sessions/sess_0/skills?source=sbp"],
    ["responseJsonOrNull", "/v1/sessions/sess_0/pane-tail"],
    ["responseJsonOrNull", "/v1/sessions/sess_0/transcript?limit=160"],
    ["responseJsonOrNull", "/v1/sessions/sess_0/mermaid-artifact"],
    ["responseJsonOrNull", "/v1/sessions/sess_0/git-diff"],
    ["renderWorkbenchWidgets"],
  ]);
});

test("runWorkbenchWidgetRefresh preserves stale response guard", async () => {
  const { calls, runtime } = buildRuntime();
  const originalResponseJsonOrNull = runtime.responseJsonOrNull;
  let staleOnce = true;
  runtime.responseJsonOrNull = async (response) => {
    if (staleOnce && response.path.endsWith("/timeline")) {
      staleOnce = false;
      runtime.state.selectedSessionId = "other";
    }
    return originalResponseJsonOrNull(response);
  };

  await runWorkbenchWidgetRefresh({}, runtime);

  assert.equal(runtime.state.workbenchWidgets.lastLoadedAt, 0);
  assert.equal(runtime.state.workbenchWidgets.loading, false);
  assert.equal(calls.filter(([name]) => name === "renderWorkbenchWidgets").length, 2);

  runtime.state.selectedSessionId = "sess_0";
  await runWorkbenchWidgetRefresh({ throttle: true }, runtime);

  assert.equal(runtime.state.workbenchWidgets.loading, false);
  assert.equal(runtime.state.workbenchWidgets.lastLoadedAt, 4242);
  assert.equal(calls.filter(([name]) => name === "apiMaybeFetch").length, 12);
  assert.equal(calls.filter(([name]) => name === "renderWorkbenchWidgets").length, 4);
});

test("runWorkbenchWidgetRefresh does not let old responses clear newer loading state", async () => {
  const { calls, runtime } = buildRuntime();
  const originalResponseJsonOrNull = runtime.responseJsonOrNull;
  let supersedeOnce = true;
  runtime.responseJsonOrNull = async (response) => {
    if (supersedeOnce && response.path.endsWith("/timeline")) {
      supersedeOnce = false;
      runtime.state.workbenchWidgets.requestSeq = 2;
      runtime.state.workbenchWidgets.loading = true;
    }
    return originalResponseJsonOrNull(response);
  };

  await runWorkbenchWidgetRefresh({}, runtime);

  assert.equal(runtime.state.workbenchWidgets.requestSeq, 2);
  assert.equal(runtime.state.workbenchWidgets.loading, true);
  assert.equal(runtime.state.workbenchWidgets.lastLoadedAt, 0);
  assert.equal(calls.filter(([name]) => name === "renderWorkbenchWidgets").length, 1);
});
