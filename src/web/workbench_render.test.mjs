import test from "node:test";
import assert from "node:assert/strict";

import {
  agentActionLabel,
  applyWorkbenchWidgetResults,
  buildWorkbenchWidgetRequestPlan,
  buildWorkbenchWidgetsHtml,
  emptyWorkbenchWidgets,
  mergeWorkbenchTranscriptPage,
  resetWorkbenchWidgetsState,
  operatorPressureSummary,
  renderTerminalWorkbenchActions,
  renderTranscriptBlocks,
  renderWorkbenchLogLens,
  selectedWorkbenchWidgetsSnapshot,
  shouldThrottleWorkbenchWidgets,
  truncateWorkbenchText,
  workbenchWidgetClickPlan,
  workbenchWidgetLogPlan,
  workbenchWidgetsHaveCurrentPayload,
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

test("operatorPressureSummary falls back to session counts when the payload count is zero", () => {
  const pressure = operatorPressureSummary(
    { action_cues: [], state: "idle", token_count: 0, context_limit: 0 },
    { token_count: 5000, context_limit: 100000 },
  );
  assert.match(pressure, /5% context/);
});

test("workbench Diffs panel reports untracked-only trees as dirty, not clean", () => {
  const html = buildWorkbenchWidgetsHtml({
    widgets: {
      timeline: { events: [] },
      transcript: { available: false, turns: [], records: [] },
      artifact: { available: false },
      gitDiff: {
        available: true,
        status_short: "?? new_file.rs",
        unstaged_diff: "",
        staged_diff: "",
        files: [],
        repo_root: "/repo",
      },
      skills: { available: false },
    },
    contextPayload: { turns: [] },
    selectedTurnId: "",
    logState: { mode: "lens", filter: "all", query: "" },
  });
  assert.doesNotMatch(html, /is clean\./);
  assert.match(html, /uncommitted changes/);
  assert.match(html, /new_file\.rs/);
});

test("workbench diff truncation keeps the head so hunks line up with file summaries", () => {
  const diff = "diff --git a/HEADMARKER b/HEADMARKER\n" + "+x\n".repeat(3000) + "TAILMARKERLINE\n";
  const html = buildWorkbenchWidgetsHtml({
    widgets: {
      timeline: { events: [] },
      transcript: { available: false, turns: [], records: [] },
      artifact: { available: false },
      gitDiff: {
        available: true,
        status_short: " M x",
        unstaged_diff: diff,
        files: [],
        truncated: true,
      },
      skills: { available: false },
    },
    contextPayload: { turns: [] },
    selectedTurnId: "",
    logState: { mode: "lens", filter: "all", query: "" },
  });
  assert.match(html, /HEADMARKER/);
  assert.doesNotMatch(html, /TAILMARKERLINE/);
  assert.match(html, /\.\.\. truncated \.\.\./);
});

test("workbench widget event planners classify click, input, and change targets", () => {
  const clickTarget = (matches) => ({
    closest(selector) {
      return matches[selector] ?? null;
    },
  });
  const matchingTarget = (selector, value) => ({
    value,
    matches(query) {
      return query === selector;
    },
  });

  assert.deepEqual(workbenchWidgetClickPlan(null), { type: "ignore" });
  assert.deepEqual(workbenchWidgetClickPlan(clickTarget({
    "[data-workbench-turn-id]": { dataset: { workbenchTurnId: "turn-7" } },
    "[data-workbench-log-mode]": { dataset: { workbenchLogMode: "raw" } },
  })), { type: "select_turn", turnId: "turn-7" });
  assert.deepEqual(workbenchWidgetClickPlan(clickTarget({
    "[data-workbench-log-mode]": { dataset: { workbenchLogMode: "raw" } },
  })), { type: "set_log_mode", mode: "raw" });
  assert.deepEqual(workbenchWidgetClickPlan(clickTarget({
    "[data-workbench-log-mode]": { dataset: { workbenchLogMode: "unknown" } },
  })), { type: "set_log_mode", mode: "lens" });
  assert.deepEqual(workbenchWidgetClickPlan(clickTarget({
    "[data-workbench-open-mermaid]": {},
  })), { type: "open_mermaid" });

  assert.deepEqual(workbenchWidgetLogPlan("input", matchingTarget("[data-workbench-log-search]", "cargo")), {
    type: "set_log_search",
    query: "cargo",
  });
  assert.deepEqual(workbenchWidgetLogPlan("input", matchingTarget("[data-other]", "cargo")), { type: "ignore" });

  assert.deepEqual(workbenchWidgetLogPlan("change", matchingTarget("[data-workbench-log-filter]", "diff")), {
    type: "set_log_filter",
    filter: "diff",
  });
  assert.deepEqual(workbenchWidgetLogPlan("change", matchingTarget("[data-workbench-log-filter]", "invalid")), {
    type: "set_log_filter",
    filter: "all",
  });
  assert.deepEqual(workbenchWidgetLogPlan("change", matchingTarget("[data-other]", "diff")), { type: "ignore" });
  assert.deepEqual(workbenchWidgetLogPlan("click", matchingTarget("[data-workbench-log-filter]", "diff")), { type: "ignore" });
});

test("workbench refresh helpers plan delta fetches and merge transcript pages", () => {
  const widgets = emptyWorkbenchWidgets({
    sessionId: "sess_0",
    timeline: { events: [] },
    transcript: {
      selected_turn_id: "turn-1",
      records: [{ id: "old", byte_start: 10, text: "old" }],
    },
    transcriptTurnId: "turn-1",
    transcriptNextCursor: 42,
    lastLoadedAt: 1000,
  });

  assert.equal(workbenchWidgetsHaveCurrentPayload(widgets, "sess_0"), true);
  assert.equal(
    shouldThrottleWorkbenchWidgets({
      options: { throttle: true },
      widgets,
      sessionId: "sess_0",
      now: 1200,
      throttleMs: 5000,
    }),
    true,
  );
  assert.equal(selectedWorkbenchWidgetsSnapshot(widgets, "other").sessionId, null);

  const plan = buildWorkbenchWidgetRequestPlan({
    sessionId: "sess_0",
    selectedTurnId: "turn-1",
    widgets,
  });
  assert.equal(plan.canDeltaTranscript, true);
  assert.match(plan.paths.skills, /source=sbp$/);
  assert.match(plan.paths.transcript, /turn_id=turn-1/);
  assert.match(plan.paths.transcript, /after=42/);
  assert.match(plan.paths.transcript, /limit=80/);

  const applied = applyWorkbenchWidgetResults(
    widgets,
    {
      timelineResult: { status: "fulfilled", value: { events: [{ kind: "diff" }] } },
      skillsResult: { status: "rejected", reason: new Error("skills unavailable") },
      tailResult: { status: "fulfilled", value: { text: "tail" } },
      transcriptResult: {
        status: "fulfilled",
        value: {
          selected_turn_id: "turn-1",
          next_cursor: 64,
          records: [
            { id: "new", byte_start: 20, text: "new" },
            { id: "old", byte_start: 10, text: "old updated" },
          ],
        },
      },
      artifactResult: { status: "fulfilled", value: { available: true } },
      diffResult: { status: "rejected", reason: new Error("diff unavailable") },
    },
    {
      canDeltaTranscript: plan.canDeltaTranscript,
      requestedTurnId: plan.requestedTurnId,
      selectedTurnId: "turn-1",
    },
  );

  assert.equal(applied.selectedTurnId, "turn-1");
  assert.equal(widgets.loading, false);
  assert.equal(widgets.skills, null);
  assert.deepEqual(
    widgets.transcript.records.map((record) => record.id),
    ["old", "new"],
  );
  assert.equal(widgets.transcript.records[0].text, "old updated");
  assert.equal(widgets.transcriptNextCursor, 64);
  assert.match(widgets.error, /skills: skills unavailable/);
  assert.match(widgets.error, /diffs: diff unavailable/);
});

test("mergeWorkbenchTranscriptPage orders equal-byte_start records deterministically", () => {
  const result = mergeWorkbenchTranscriptPage({
    previous: { selected_turn_id: "t", records: [{ id: "b", byte_start: 5 }] },
    nextTranscript: {
      selected_turn_id: "t",
      records: [
        { id: "c", byte_start: 5 },
        { id: "a", byte_start: 5 },
      ],
    },
    canDeltaTranscript: true,
    selectedTurnId: "t",
  });
  // All records share byte_start 5, so the id tie-break gives a stable a,b,c
  // order regardless of merge insertion order.
  assert.deepEqual(
    result.transcript.records.map((record) => record.id),
    ["a", "b", "c"],
  );
});

test("workbench widget reset preserves request sequence and clears payload", () => {
  const widgets = emptyWorkbenchWidgets({
    sessionId: "sess_old",
    loading: true,
    timeline: { events: [{ kind: "tool_call" }] },
    requestSeq: 7,
    lastHtml: "<details>old</details>",
  });

  resetWorkbenchWidgetsState(widgets, "sess_new");

  assert.equal(widgets.sessionId, "sess_new");
  assert.equal(widgets.requestSeq, 7);
  assert.equal(widgets.loading, false);
  assert.equal(widgets.timeline, null);
  assert.equal(widgets.lastHtml, "");
});
