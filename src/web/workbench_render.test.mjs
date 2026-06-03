import test from "node:test";
import assert from "node:assert/strict";

import {
  agentActionLabel,
  buildWorkbenchWidgetsHtml,
  operatorPressureSummary,
  renderTerminalWorkbenchActions,
  renderTranscriptBlocks,
  renderWorkbenchLogLens,
  truncateWorkbenchText,
} from "./workbench_render.js";

test("workbench text and action summaries stay compact", () => {
  assert.equal(truncateWorkbenchText("  alpha\n beta  ", 40), "alpha beta");
  assert.equal(truncateWorkbenchText("abcdefgh", 6), "abc...");
  assert.equal(agentActionLabel({ tool: "shell", detail: "cargo test" }), "shell: cargo test");

  const pressure = operatorPressureSummary(
    {
      action_cues: [{ kind: "awaiting_user" }],
      state: "attention",
      attached_clients: 2,
      token_count: 50,
      context_limit: 100,
    },
    null,
  );
  assert.match(pressure, /awaiting user/);
  assert.match(pressure, /50% context/);

  const html = renderTerminalWorkbenchActions([{ tool: "exec", detail: "node --test" }], true);
  assert.match(html, /workbench-action-tool/);
  assert.match(html, /node --test/);
});

test("workbench log lens classifies, filters, searches, and renders raw mode", () => {
  const text = [
    "You ran cargo test",
    "cargo test --lib",
    "error: failed",
    "diff --git a/src/lib.rs b/src/lib.rs",
    "+added",
  ].join("\n");
  const blocks = renderTranscriptBlocks(text);
  assert.deepEqual(blocks.map((block) => block.kind), ["operator", "command", "status", "diff"]);

  const commandHtml = renderWorkbenchLogLens(text, {
    logState: { mode: "lens", filter: "command", query: "" },
  });
  assert.match(commandHtml, /workbench-log-block-command/);
  assert.doesNotMatch(commandHtml, /workbench-log-block-status/);

  const searchHtml = renderWorkbenchLogLens(text, {
    logState: { mode: "lens", filter: "all", query: "cargo" },
  });
  assert.match(searchHtml, /workbench-log-mark/);

  const rawHtml = renderWorkbenchLogLens(text, {
    logState: { mode: "raw", filter: "all", query: "" },
  });
  assert.match(rawHtml, /workbench-log-raw/);
  assert.match(rawHtml, /cargo test --lib/);
});

test("workbench widget renderer composes turns, JSONL, diff, artifacts, and skills", () => {
  const record = {
    id: "r1",
    kind: "function_call",
    summary: "exec: cargo test",
    raw: JSON.stringify({
      type: "function_call",
      name: "exec_command",
      arguments: { cmd: "cargo test", workdir: "/repo" },
    }),
    byte_start: 12,
  };
  const html = buildWorkbenchWidgetsHtml({
    widgets: {
      timeline: { events: [{ kind: "tool_call", title: "exec", summary: "cargo test" }] },
      transcript: {
        available: true,
        selected_turn: { id: "turn-1", text: "run tests" },
        turns: [{ id: "turn-1", order: 1, source: "Codex", text: "run tests" }],
        records: [record],
      },
      artifact: { available: true, path: "plan.mmdx", plan_files: ["PLAN.md"] },
      gitDiff: {
        available: true,
        status_short: " M src/lib.rs",
        unstaged_diff: "diff --git a/src/lib.rs b/src/lib.rs\n+added",
        files: [{ path: "src/lib.rs", added_lines: 1, removed_lines: 0, change: "modified" }],
      },
      skills: { available: true, skills: [{ name: "describe", description: "spec tests" }], issues: [] },
    },
    contextPayload: { turns: [] },
    selectedTurnId: "turn-1",
    logState: { mode: "lens", filter: "all", query: "" },
  });

  assert.match(html, /workbench-turn is-selected/);
  assert.match(html, /Post-turn JSONL/);
  assert.match(html, /data-workbench-open-mermaid="true"/);
  assert.match(html, /workbench-diff/);
  assert.match(html, /describe/);
});
