import test from "node:test";
import assert from "node:assert/strict";

import {
  createElement,
  fakeDocumentForIds,
} from "./island_test_helpers.mjs";
import { buildWorkbenchWidgetsViewModel } from "./workbench_render.js";
import {
  WORKBENCH_WIDGETS_ISLAND_IDS,
  WorkbenchWidgets,
  assertStableWorkbenchWidgetsIslandContainers,
  createWorkbenchWidgetsElements,
  mountWorkbenchWidgetsIsland,
  resolveWorkbenchWidgetsIslandContainers,
} from "./workbench_widgets_island.js";

function fakeDocument() {
  return fakeDocumentForIds(WORKBENCH_WIDGETS_ISLAND_IDS);
}

function titlesFor(nodes) {
  return nodes
    .filter((node) => node?.type === "details")
    .map((node) => node.props.children[0].props.children[0].props.children[0]);
}

function bodyHtmlFor(node) {
  return node.props.children[1].props.dangerouslySetInnerHTML.__html;
}

function richWorkbenchModel() {
  return buildWorkbenchWidgetsViewModel({
    widgets: {
      loading: false,
      timeline: { events: [{ kind: "tool_call", title: "exec", summary: "cargo test" }] },
      transcript: {
        available: true,
        selected_turn: { id: "turn-1", text: "run tests" },
        turns: [{ id: "turn-1", order: 1, source: "Codex", text: "run tests" }],
        records: [
          {
            id: "record-1",
            kind: "function_call",
            summary: "exec: cargo test",
            raw: JSON.stringify({ type: "function_call", name: "exec_command" }),
            byte_start: 10,
          },
        ],
      },
      artifact: { available: true, path: "docs/plan.mmdx", plan_files: ["WORKGRAPH.md"] },
      gitDiff: {
        available: true,
        status_short: " M src/web/app.js",
        unstaged_diff: "diff --git a/src/web/app.js b/src/web/app.js\n+added",
        files: [{ path: "src/web/app.js", source: "unstaged", change: "modified", added_lines: 1, removed_lines: 0 }],
      },
      skills: { available: true, skills: [{ name: "ui", description: "front-end skill" }], issues: [] },
    },
    contextPayload: {
      current_tool: { tool: "shell", detail: "npm test" },
      recent_actions: [],
    },
    selectedTurnId: "turn-1",
    logState: { mode: "lens", filter: "all", query: "" },
  });
}

test("workbench widgets island preserves widget shell and body DOM contract", () => {
  const nodes = createWorkbenchWidgetsElements(createElement, richWorkbenchModel());
  const details = nodes.filter((node) => node.type === "details");
  const [terminal, turns, logs, activity, diffs, artifacts, skills] = details;

  assert.deepEqual(titlesFor(nodes), ["Terminal", "Turns", "Logs", "Activity", "Diffs", "Artifacts", "Skills"]);
  assert.deepEqual(details.map((node) => node.props.className), Array(7).fill("workbench-widget"));
  assert.deepEqual(details.map((node) => node.props.key), ["terminal", "turns", "logs", "activity", "diffs", "artifacts", "skills"]);
  assert.deepEqual(details.map((node) => node.props.open), [true, true, true, true, true, false, false]);
  assert.match(bodyHtmlFor(terminal), /data-workbench-open-terminal="true"/);
  assert.equal(turns.props.children[0].type, "summary");
  assert.equal(turns.props.children[0].props.children[1].props.className, "workbench-widget-meta");
  assert.equal(turns.props.children[0].props.children[1].props.children[0], "1 user");

  assert.match(bodyHtmlFor(turns), /data-workbench-turn-id="turn-1"/);
  assert.match(bodyHtmlFor(turns), /workbench-turn is-selected/);
  assert.match(bodyHtmlFor(logs), /workbench-log-lens/);
  assert.match(bodyHtmlFor(logs), /data-workbench-log-mode="lens"/);
  assert.match(bodyHtmlFor(logs), /data-workbench-log-filter/);
  assert.match(bodyHtmlFor(logs), /data-workbench-log-search/);
  assert.match(bodyHtmlFor(activity), /Tool calls/);
  assert.match(bodyHtmlFor(diffs), /workbench-diff/);
  assert.match(bodyHtmlFor(diffs), /diff-line-add/);
  assert.match(bodyHtmlFor(artifacts), /data-workbench-open-mermaid="true"/);
  assert.match(bodyHtmlFor(skills), /ui/);
  assert.throws(() => createWorkbenchWidgetsElements(null, {}), /createElement function/);
});

test("workbench widgets island preserves loading, error, and empty states", () => {
  const loadingNodes = createWorkbenchWidgetsElements(createElement, {
    statusText: "Loading pinned widgets...",
    items: [],
  });
  assert.equal(loadingNodes[0].type, "div");
  assert.equal(loadingNodes[0].props.className, "workbench-action-detail");
  assert.equal(loadingNodes[0].props.children[0], "Loading pinned widgets...");

  const empty = buildWorkbenchWidgetsViewModel({ widgets: {}, contextPayload: null });
  const emptyNodes = createWorkbenchWidgetsElements(createElement, empty);
  assert.equal(emptyNodes.length, 7);
  assert.match(bodyHtmlFor(emptyNodes[0]), /Live terminal detached/);
  assert.match(bodyHtmlFor(emptyNodes[1]), /No user-submitted turns found/);
  assert.match(bodyHtmlFor(emptyNodes[2]), /No recent pane output/);
  assert.match(bodyHtmlFor(emptyNodes[5]), /No Mermaid or plan artifact found/);
  assert.match(bodyHtmlFor(emptyNodes[6]), /Skillbox skills have not loaded/);

  const errorNodes = createWorkbenchWidgetsElements(createElement, {
    statusText: "timeline: unavailable",
    items: empty.items,
  });
  assert.equal(errorNodes[0].props.children[0], "timeline: unavailable");
});

test("workbench widgets island mounts, rerenders, and guards stable host identity", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountWorkbenchWidgetsIsland({
    documentRef,
    createRootImpl(root) {
      calls.push(["createRoot", root.id]);
      return {
        render(element) {
          calls.push(["render", root.id, element.type]);
        },
        unmount() {
          calls.push(["unmount", root.id]);
        },
      };
    },
    flushSyncImpl(callback) {
      calls.push(["flush:start"]);
      callback();
      calls.push(["flush:end"]);
    },
  });
  const before = { ...handle.containers };

  assert.deepEqual(calls[0], ["createRoot", WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets]);
  assert.deepEqual(resolveWorkbenchWidgetsIslandContainers({ documentRef }), before);
  assert.equal(handle.render(richWorkbenchModel()), true);
  assert.deepEqual(calls.slice(1), [
    ["flush:start"],
    ["render", WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets, WorkbenchWidgets],
    ["flush:end"],
  ]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableWorkbenchWidgetsIslandContainers(before, {
      terminalWorkbenchWidgets: replace(WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets),
    }),
    /replaced stable container terminalWorkbenchWidgets/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount", WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets]);
});

test("workbench widgets island detects missing and synchronously replaced host", () => {
  const missing = fakeDocument();
  missing.remove(WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets);
  assert.throws(
    () => resolveWorkbenchWidgetsIslandContainers({ documentRef: missing.documentRef }),
    /missing stable container terminalWorkbenchWidgets/,
  );

  const { documentRef, replace } = fakeDocument();
  const handle = mountWorkbenchWidgetsIsland({
    documentRef,
    createRootImpl() {
      return {
        render() {
          replace(WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets);
        },
        unmount() {},
      };
    },
    flushSyncImpl(callback) {
      callback();
    },
  });

  assert.throws(
    () => handle.render({ items: [] }),
    /replaced stable container terminalWorkbenchWidgets/,
  );
});
