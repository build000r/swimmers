import test from "node:test";
import assert from "node:assert/strict";

import { createTerminalWorkbenchController } from "./terminal_workbench_controller.js";

function classList() {
  const values = new Set();
  return {
    contains(name) {
      return values.has(name);
    },
    toggle(name, force) {
      if (force) {
        values.add(name);
      } else {
        values.delete(name);
      }
    },
  };
}

function buildController(overrides = {}) {
  const renderViews = [];
  const state = {
    selectedSessionId: "sess_0",
    trogdorAtlasOpen: false,
    terminalWorkbenchOpen: true,
    agentContextSessionId: "sess_0",
    agentContextLoading: false,
    agentContextPayload: {
      available: true,
      tool: "codex",
      cwd: "/repo",
      current_tool: { tool: "shell", detail: "npm test" },
      recent_actions: [],
    },
    agentContextError: "",
    workbenchSelectedTurnId: "turn-1",
    workbenchLogMode: "lens",
    workbenchLogFilter: "all",
    workbenchLogSearch: "",
    workbenchWidgets: {
      sessionId: "sess_0",
      loading: false,
      timeline: { events: [{ kind: "tool_call", title: "exec", summary: "npm test" }] },
      transcript: {
        available: true,
        selected_turn: { id: "turn-1", text: "run tests" },
        turns: [{ id: "turn-1", order: 1, text: "run tests" }],
        records: [{ id: "record-1", kind: "message", raw: "npm test", byte_start: 1 }],
      },
      artifact: { available: true, path: "docs/plan.mmdx", plan_files: [] },
      gitDiff: { available: true, unstaged_diff: "", staged_diff: "", repo_root: "/repo" },
      skills: { available: true, skills: [], issues: [] },
      lastHtml: "",
      requestSeq: 0,
    },
    ...overrides.state,
  };
  const container = {
    innerHTML: "",
    querySelectorAll() {
      return [];
    },
  };
  const el = {
    terminalWorkbench: { classList: classList(), setAttribute() {}, scrollTop: 44 },
    terminalWorkbenchActions: { innerHTML: "" },
    terminalWorkbenchCurrent: { textContent: "" },
    terminalWorkbenchMeta: { textContent: "" },
    terminalWorkbenchPressure: { textContent: "" },
    terminalWorkbenchRefresh: { disabled: false },
    terminalWorkbenchStatus: { textContent: "" },
    terminalWorkbenchTask: { textContent: "" },
    terminalWorkbenchTitle: { textContent: "" },
    terminalWorkbenchToggle: { disabled: false, setAttribute() {} },
    terminalWorkbenchWidgets: container,
    ...overrides.el,
  };
  const controller = createTerminalWorkbenchController({
    state,
    el,
    currentSession: () => ({ session_id: "sess_0", tmux_name: "swordsman", tool: "codex", cwd: "/repo" }),
    apiFetch: async () => ({}),
    apiMaybeFetch: async () => null,
    responseJson: async () => ({}),
    responseJsonOrNull: async () => null,
    documentRef: { body: { classList: classList() } },
    requestAnimationFrameRef: null,
    renderWorkbenchWidgetsView(view) {
      renderViews.push(view);
      return overrides.renderReturn ?? true;
    },
    ...overrides.runtime,
  });
  return { controller, container, el, renderViews, state };
}

test("terminal workbench controller delegates widget view rendering without changing state ownership", () => {
  const { controller, container, renderViews, state } = buildController();

  controller.renderWorkbenchWidgets();

  assert.equal(renderViews.length, 1);
  assert.deepEqual(renderViews[0].model.items.map((item) => item.title), [
    "Terminal",
    "Turns",
    "Logs",
    "Activity",
    "Diffs",
    "Artifacts",
    "Skills",
  ]);
  assert.match(renderViews[0].html, /workbench-widget-title/);
  assert.match(renderViews[0].model.items[0].bodyHtml, /data-workbench-open-terminal="true"/);
  assert.match(renderViews[0].model.items[1].bodyHtml, /data-workbench-turn-id="turn-1"/);
  assert.match(renderViews[0].model.items[2].bodyHtml, /workbench-log-lens/);
  assert.match(renderViews[0].model.items[5].bodyHtml, /data-workbench-open-mermaid="true"/);
  assert.equal(container.innerHTML, "", "React-rendered path does not use legacy innerHTML");
  assert.ok(state.workbenchWidgets.lastHtml.includes("workbench-widget-title"));
});

test("terminal workbench controller preserves legacy widget fallback when island declines", () => {
  const { controller, container, renderViews, state } = buildController({ renderReturn: false });

  controller.renderWorkbenchWidgets();

  assert.equal(renderViews.length, 1);
  assert.ok(container.innerHTML.includes("workbench-widget-title"));
  assert.ok(container.innerHTML.includes("data-workbench-open-mermaid=\"true\""));
  assert.equal(state.workbenchWidgets.lastHtml, container.innerHTML);
});
