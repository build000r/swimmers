import test from "node:test";
import assert from "node:assert/strict";

import {
  buildCommandPaletteItems,
  commandPaletteExecutionPlan,
  commandPaletteResultEventPlan,
  commandPaletteScore,
  commandPaletteSearchKeyPlan,
  commandPaletteSessionDisplayName,
  filterCommandPaletteItems,
  filteredCommandPaletteItemsForState,
  renderCommandPaletteResultsHtml,
} from "./command_palette.js";

function session(overrides = {}) {
  return {
    session_id: "sess_0",
    tmux_name: "alpha",
    name: "fallback-name",
    state: "attention",
    ...overrides,
  };
}

test("command palette item helpers preserve disabled states and session labels", () => {
  const copyFrameAction = () => "copied";
  const items = buildCommandPaletteItems({
    selectedSession: session(),
    readOnly: false,
    sessions: [
      session(),
      session({ session_id: "sess_1", tmux_name: "", name: "beta", state: "idle" }),
      session({ session_id: "sess_2", tmux_name: "", name: "", state: "" }),
    ],
    copyFrameAction,
  });

  assert.equal(commandPaletteSessionDisplayName(session({ tmux_name: "", name: "", session_id: "" })), "session");
  assert.equal(items.find((item) => item.actionId === "focus_terminal").disabled, false);
  assert.equal(items.find((item) => item.actionId === "open_create").disabled, false);
  assert.equal(items.find((item) => item.label === "Copy visible text").action, copyFrameAction);
  assert.deepEqual(items.slice(-3), [
    { label: "Switch to alpha", meta: "sess_0  attention", sessionId: "sess_0" },
    { label: "Switch to beta", meta: "sess_1  idle", sessionId: "sess_1" },
    { label: "Switch to sess_2", meta: "sess_2  ", sessionId: "sess_2" },
  ]);

  const disabled = buildCommandPaletteItems({
    selectedSession: null,
    readOnly: true,
    sessions: [],
    copyFrameAction,
  });
  assert.equal(disabled.find((item) => item.actionId === "focus_terminal").disabled, true);
  assert.equal(disabled.find((item) => item.actionId === "open_send").disabled, true);
  assert.equal(disabled.find((item) => item.actionId === "open_create").disabled, true);
  assert.equal(disabled.find((item) => item.actionId === "refresh").disabled, undefined);
});

test("command palette scoring and filtering preserve search ordering and limits", () => {
  assert.equal(commandPaletteScore({ label: "Focus terminal", meta: "terminal" }, ""), 1);
  assert.equal(commandPaletteScore({ label: "Focus terminal", meta: "terminal" }, "terminal"), 994);
  assert.equal(commandPaletteScore({ label: "Focus terminal", meta: "terminal" }, "ft"), 75);
  assert.equal(commandPaletteScore({ label: "Focus terminal", meta: "terminal" }, "zz"), 0);

  const items = [
    { label: "Bravo", meta: "second" },
    { label: "Alpha", meta: "first" },
    { label: "Beta", meta: "agent" },
    ...Array.from({ length: 24 }, (_, index) => ({ label: `Item ${index}`, meta: "bulk" })),
  ];
  assert.deepEqual(filterCommandPaletteItems(items, "  BETA  ").map((item) => item.label), ["Beta"]);

  const unqueried = filterCommandPaletteItems(items, "");
  assert.equal(unqueried.length, 18);
  assert.equal(unqueried[0].label, "Alpha");
  assert.equal(unqueried[1].label, "Beta");
});

test("command palette filtering matches full-sort ranking for bounded result sets", () => {
  const items = Array.from({ length: 80 }, (_, index) => ({
    label: index % 2 === 0 ? `Switch to Agent ${80 - index}` : `Open Agent ${index}`,
    meta: index % 5 === 0 ? "agent running" : `agent workspace ${index % 7}`,
    actionId: `action_${index}`,
    disabled: index % 11 === 0,
  }));
  const oracle = (query, limit) => {
    const normalizedQuery = String(query || "").trim().toLowerCase();
    return items
      .map((item) => ({ ...item, score: commandPaletteScore(item, normalizedQuery) }))
      .filter((item) => !normalizedQuery || item.score > 0)
      .sort((a, b) => b.score - a.score || a.label.localeCompare(b.label))
      .slice(0, limit);
  };

  assert.deepEqual(filterCommandPaletteItems(items, "agent", 7), oracle("agent", 7));
  assert.deepEqual(filterCommandPaletteItems(items, "", 5), oracle("", 5));
  assert.deepEqual(filterCommandPaletteItems(items, "workspace 3", 9), oracle("workspace 3", 9));
});

test("command palette state helper combines built-in commands, sessions, and scores", () => {
  const items = filteredCommandPaletteItemsForState({
    selectedSession: session(),
    readOnly: false,
    sessions: [session(), session({ session_id: "sess_1", tmux_name: "beta", state: "idle" })],
    query: "beta",
  });

  assert.equal(items[0].label, "Switch to beta");
  assert.equal(items[0].sessionId, "sess_1");
  assert.ok(items[0].score > 0);
});

test("command palette result markup helper preserves escaping, active state, and disabled copy", () => {
  assert.equal(
    renderCommandPaletteResultsHtml([], 0),
    `<div class="sheet-copy">No matching commands.</div>`,
  );

  const html = renderCommandPaletteResultsHtml([
    { label: "Alpha <one>", meta: "ready & waiting" },
    { label: "Send \"quote\"", meta: "unsafe", disabled: true },
  ], 1);

  assert.match(html, /class="palette-item"/);
  assert.match(html, /class="palette-item is-active"/);
  assert.match(html, /aria-selected="false"/);
  assert.match(html, /aria-selected="true"/);
  assert.match(html, /data-palette-index="1"/);
  assert.match(html, /disabled/);
  assert.match(html, /Alpha &lt;one&gt;/);
  assert.match(html, /ready &amp; waiting/);
  assert.match(html, /Send &quot;quote&quot;/);
  assert.match(html, /palette-item-meta">unavailable</);
});

test("command palette execution plan helper preserves no-ops and dispatch ordering", () => {
  const action = () => "copied";

  assert.deepEqual(commandPaletteExecutionPlan(null), { type: "none" });
  assert.deepEqual(commandPaletteExecutionPlan({ disabled: true, action }), { type: "none" });
  assert.deepEqual(commandPaletteExecutionPlan({ label: "Inert" }), { type: "none" });
  assert.deepEqual(
    commandPaletteExecutionPlan({ sessionId: "sess_1", action, actionId: "refresh" }),
    { type: "selectSession", sessionId: "sess_1" },
  );
  assert.deepEqual(commandPaletteExecutionPlan({ actionId: "refresh" }), {
    type: "dispatchAction",
    actionId: "refresh",
  });

  const plan = commandPaletteExecutionPlan({ action });
  assert.equal(plan.type, "invokeAction");
  assert.equal(plan.action, action);
});

test("command palette event helpers preserve keyboard and result index decisions", () => {
  assert.deepEqual(commandPaletteSearchKeyPlan({ key: "ArrowDown" }, 0, 3), {
    type: "set_index",
    index: 1,
    preventDefault: true,
  });
  assert.deepEqual(commandPaletteSearchKeyPlan({ key: "ArrowDown" }, 0, 0), {
    type: "set_index",
    index: -1,
    preventDefault: true,
  });
  assert.deepEqual(commandPaletteSearchKeyPlan({ key: "ArrowUp" }, 0, 3), {
    type: "set_index",
    index: 0,
    preventDefault: true,
  });
  assert.deepEqual(commandPaletteSearchKeyPlan({ key: "Enter" }, 1, 3), {
    type: "run_item",
    preventDefault: true,
  });
  assert.deepEqual(commandPaletteSearchKeyPlan({ key: "Escape" }, 1, 3), { type: "ignore" });

  const target = (rawIndex) => ({
    closest(selector) {
      return selector === "[data-palette-index]" ? { dataset: { paletteIndex: rawIndex } } : null;
    },
  });
  assert.deepEqual(commandPaletteResultEventPlan("mousemove", target("2"), 4), {
    type: "set_index",
    index: 2,
  });
  assert.deepEqual(commandPaletteResultEventPlan("click", target("9"), 4), {
    type: "run_item",
    index: 3,
  });
  assert.deepEqual(commandPaletteResultEventPlan("click", target("bad"), 4), {
    type: "run_item",
    index: 0,
  });
  assert.deepEqual(commandPaletteResultEventPlan("click", null, 4), { type: "ignore" });
});
