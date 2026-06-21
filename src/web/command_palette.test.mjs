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
  recordCommandPaletteUse,
  renderCommandPaletteResultsHtml,
} from "./command_palette.js";
import { globalShortcutPlan } from "./input_support.js";

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
  assert.deepEqual(filterCommandPaletteItems(items, "", 0), []);
  assert.deepEqual(filterCommandPaletteItems(items, "", -1), []);
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
      .map((item) => ({ ...item, score: commandPaletteScore(item, normalizedQuery), recency: 0 }))
      .filter((item) => !normalizedQuery || item.score > 0)
      .sort(
        (a, b) =>
          b.score - a.score || (b.recency || 0) - (a.recency || 0) || a.label.localeCompare(b.label),
      )
      .slice(0, limit);
  };

  assert.deepEqual(filterCommandPaletteItems(items, "agent", 7), oracle("agent", 7));
  assert.deepEqual(filterCommandPaletteItems(items, "", 5), oracle("", 5));
  assert.deepEqual(filterCommandPaletteItems(items, "workspace 3", 9), oracle("workspace 3", 9));
});

test("every command palette accelerator hint maps to a wired global shortcut", () => {
  const items = buildCommandPaletteItems({ selectedSession: session(), sessions: [] });
  const chordItems = items.filter((item) => /^Ctrl\+Shift\+[A-Z]$/.test(item.meta || ""));
  // The palette is the in-app keyboard reference; most actions advertise their chord.
  assert.ok(chordItems.length >= 10, `expected many advertised chords, got ${chordItems.length}`);
  for (const item of chordItems) {
    const key = item.meta.slice("Ctrl+Shift+".length);
    const plan = globalShortcutPlan(
      { ctrlKey: true, shiftKey: true, code: `Key${key}` },
      { hasCurrentSession: true },
    );
    // A hint that doesn't actually trigger a shortcut is misleading documentation.
    assert.notEqual(
      plan.type,
      "unhandled",
      `${item.meta} advertised by "${item.label}" must be a wired Ctrl+Shift shortcut`,
    );
  }
});

test("command palette frecency lifts recently-used items as a tie-breaker", () => {
  const items = [
    { label: "Focus terminal", actionId: "focus_terminal" },
    { label: "Open auth", actionId: "open_auth" },
    { label: "Send to terminal", actionId: "open_send" },
  ];

  // With no query, all items score equally; recency orders the list so the most
  // recently used action is first (open the palette -> your last action is right there).
  let recency = {};
  recency = recordCommandPaletteUse(recency, items[1]); // open_auth
  recency = recordCommandPaletteUse(recency, items[2]); // open_send (more recent)
  const ranked = filterCommandPaletteItems(items, "", 18, recency);
  assert.equal(ranked[0].actionId, "open_send");
  assert.equal(ranked[1].actionId, "open_auth");
  assert.equal(ranked[2].actionId, "focus_terminal");

  // A strong query match still dominates recency (recency never overrides relevance).
  const queried = filterCommandPaletteItems(items, "focus", 18, recency);
  assert.equal(queried[0].actionId, "focus_terminal");

  // The recency map is bounded.
  let capped = {};
  for (let i = 0; i < 10; i += 1) {
    capped = recordCommandPaletteUse(capped, { actionId: `a_${i}` }, 3);
  }
  assert.equal(Object.keys(capped).length, 3);
  assert.ok(capped.a_9 && capped.a_8 && capped.a_7, "keeps the 3 most-recent keys");
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
    index: 0,
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

  // Arrow nav skips disabled items so keyboard users never stall on an
  // unavailable command.
  const withDisabled = [{}, { disabled: true }, {}];
  assert.equal(commandPaletteSearchKeyPlan({ key: "ArrowDown" }, 0, withDisabled).index, 2);
  assert.equal(commandPaletteSearchKeyPlan({ key: "ArrowUp" }, 2, withDisabled).index, 0);
  // When every item ahead is disabled, hold position instead of landing on one.
  const trailingDisabled = [{}, { disabled: true }, { disabled: true }];
  assert.equal(commandPaletteSearchKeyPlan({ key: "ArrowDown" }, 0, trailingDisabled).index, 0);

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
