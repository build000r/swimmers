import test from "node:test";
import assert from "node:assert/strict";

import {
  WORKBENCH_LOG_FILTERS,
  renderTranscriptBlocks as publicRenderTranscriptBlocks,
  renderWorkbenchLogLens as publicRenderWorkbenchLogLens,
  truncateWorkbenchText as publicTruncateWorkbenchText,
} from "./workbench_render.js";
import {
  transcriptRecordDisplay,
  recordMatchesSearch,
} from "./workbench_records.js";
import {
  renderTranscriptBlocks,
  renderWorkbenchLogLens,
  transcriptRecordsToLensText,
  transcriptRecordsToRawText,
} from "./workbench_log_lens.js";

test("workbench log lens public exports remain available from workbench_render", () => {
  assert.equal(publicRenderTranscriptBlocks, renderTranscriptBlocks);
  assert.equal(publicRenderWorkbenchLogLens, renderWorkbenchLogLens);
  assert.equal(publicTruncateWorkbenchText("  abc\n def  ", 20), "abc def");
  assert.equal(publicTruncateWorkbenchText("abcdefgh", 6), "abc...");
  assert.deepEqual(WORKBENCH_LOG_FILTERS, ["all", "operator", "command", "status", "diff", "output", "thinking", "truncation"]);
});

test("workbench log lens classifies transcript blocks with stable labels and line ranges", () => {
  const blocks = renderTranscriptBlocks([
    "",
    "You ran tests",
    "cargo test --lib",
    "warning: denied",
    "diff --git a/src/lib.rs b/src/lib.rs",
    "+added",
    "... truncated ...",
    "plain output",
  ].join("\r\n"));

  assert.deepEqual(blocks.map(({ kind, label, startLine, endLine, lines }) => ({
    kind,
    label,
    startLine,
    endLine,
    lines,
  })), [
    { kind: "operator", label: "Chat", startLine: 2, endLine: 2, lines: ["You ran tests"] },
    { kind: "command", label: "Command", startLine: 3, endLine: 3, lines: ["cargo test --lib"] },
    { kind: "status", label: "Status", startLine: 4, endLine: 4, lines: ["warning: denied"] },
    {
      kind: "diff",
      label: "Diff",
      startLine: 5,
      endLine: 6,
      lines: ["diff --git a/src/lib.rs b/src/lib.rs", "+added"],
    },
    { kind: "truncation", label: "Trimmed", startLine: 7, endLine: 7, lines: ["... truncated ..."] },
    { kind: "output", label: "Output", startLine: 8, endLine: 8, lines: ["plain output"] },
  ]);
});

test("workbench log lens preserves lens controls, filters, data attrs, highlighting, and escaping", () => {
  const html = renderWorkbenchLogLens("cargo test <unit> & check\nerror: a & b", {
    logState: { mode: "lens", filter: "all", query: "<unit>" },
  });

  assert.equal(html.includes('class="workbench-log-view-button" data-workbench-log-mode="lens" aria-pressed="true"'), true);
  assert.equal(html.includes('class="workbench-log-view-button" data-workbench-log-mode="raw" aria-pressed="false"'), true);
  assert.equal(html.includes('class="workbench-log-filter" name="workbench-log-filter" aria-label="Filter log blocks" data-workbench-log-filter'), true);
  assert.equal(html.includes('class="workbench-log-search" type="search" name="workbench-log-search" aria-label="Search logs" placeholder="Search logs" value="&lt;unit&gt;" data-workbench-log-search'), true);
  assert.equal(html.includes('class="workbench-log-block workbench-log-block-command" data-log-kind="command"'), true);
  assert.equal(html.includes('<mark class="workbench-log-mark">&lt;unit&gt;</mark>'), true);
  assert.equal(html.includes("cargo test <unit>"), false);
  assert.equal(html.includes("&amp; check"), true);
  assert.equal(html.includes("& check"), false);
});

test("workbench log lens preserves raw and empty output states", () => {
  const rawHtml = renderWorkbenchLogLens("cargo test <unit>", {
    emptyText: "No <output>.",
    logState: { mode: "raw", filter: "missing", query: "" },
  });
  assert.equal(rawHtml.includes('<pre class="workbench-log-raw">cargo test &lt;unit&gt;</pre>'), true);
  assert.equal(rawHtml.includes('value="missing"'), false);

  const emptyHtml = renderWorkbenchLogLens("", {
    emptyText: "No <output>.",
    logState: { mode: "lens", filter: "all", query: "" },
  });
  assert.equal(emptyHtml.includes('<div class="workbench-log-empty">No &lt;output&gt;.</div>'), true);
});

test("workbench JSONL lens preserves brief, record HTML, raw JSON, and record text helpers", () => {
  const records = [
    {
      id: "u1",
      kind: "message",
      role: "user",
      summary: "ask",
      raw: JSON.stringify({ type: "message", role: "user", content: "inspect src/web/workbench_render.js" }),
      byte_start: 1,
    },
    {
      id: "c1",
      kind: "function_call",
      summary: "exec: cargo test",
      raw: JSON.stringify({
        type: "function_call",
        name: "exec_command",
        arguments: { cmd: "cargo test <unit>", workdir: "/repo" },
      }),
      byte_start: 2,
    },
    {
      id: "a1",
      kind: "message",
      role: "assistant",
      summary: "done",
      raw: JSON.stringify({ type: "message", role: "assistant", content: "complete; see result.mmdx" }),
      byte_start: 3,
    },
  ];

  assert.equal(transcriptRecordsToLensText(records), "message: ask\ncargo test\nmessage: done");
  assert.equal(transcriptRecordsToRawText(records), records.map((record) => record.raw).join("\n"));

  const html = renderWorkbenchLogLens("", {
    records,
    rawText: transcriptRecordsToRawText(records),
    selectedTurn: { text: "run <tests>" },
    logState: { mode: "lens", filter: "all", query: "" },
  });

  assert.equal(html.includes('<div class="workbench-action-detail">Post-turn JSONL</div>'), true);
  assert.equal(html.includes('<section class="workbench-log-brief" aria-label="Log summary">'), true);
  assert.equal(html.includes('<div class="workbench-log-brief-label">User turn</div>'), true);
  assert.equal(html.includes("<span>run &lt;tests&gt;</span>"), true);
  assert.equal(html.includes('<div class="workbench-log-brief-label">Tool actions</div>'), true);
  assert.equal(html.includes("<span>cargo test &lt;unit&gt;</span>"), true);
  assert.equal(html.includes('<div class="workbench-log-brief-label">Where to read</div>'), true);
  assert.ok(
    html.indexOf("<span>result.mmdx</span>") <
      html.indexOf("<span>src/web/workbench_render.js</span>"),
  );
  assert.equal(html.includes('<details class="workbench-log-evidence" >'), true);
  assert.equal(html.includes("<span>Event stream</span>"), true);
  assert.equal(html.includes("<span>3/3 shown</span>"), true);
  assert.equal(html.includes('class="workbench-log-record workbench-log-block workbench-log-block-command" data-log-kind="command"'), true);
  assert.equal(html.includes('<span class="workbench-log-field-key">cwd</span>'), true);
  assert.equal(html.includes('<span class="workbench-log-field-value">/repo</span>'), true);
  assert.equal(html.includes('<details class="workbench-log-json">'), true);

  const rawHtml = renderWorkbenchLogLens("", {
    records,
    rawText: '{"content":"<raw>"}',
    logState: { mode: "raw", filter: "all", query: "" },
  });
  assert.equal(rawHtml.includes('<pre class="workbench-log-raw">{&quot;content&quot;:&quot;&lt;raw&gt;&quot;}</pre>'), true);
});

test("transcriptRecordDisplay classifies thinking records with thinking kind and readable body", () => {
  const thinkingRecord = {
    id: "t1",
    kind: "thinking",
    role: "assistant",
    summary: "Analyzing the error trace to find root cause",
    raw: JSON.stringify({
      type: "assistant",
      message: {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "The error is in the database connection pool" },
          { type: "text", text: "Let me check the connection settings." },
        ],
      },
    }),
    byte_start: 100,
  };

  const display = transcriptRecordDisplay(thinkingRecord);
  assert.equal(display.kind, "thinking");
  assert.equal(display.label, "Thinking");
  assert.ok(display.body.includes("database connection pool"), `thinking body should contain thinking text, got: ${display.body}`);
});

test("transcriptRecordDisplay keeps mixed thinking and tool_use records as thinking", () => {
  const thinkingRecord = {
    id: "mixed-thinking",
    kind: "thinking",
    role: "assistant",
    summary: "Thinking before tool call",
    raw: JSON.stringify({
      type: "assistant",
      message: {
        role: "assistant",
        content: [
          { type: "thinking", thinking: "Need to inspect the failing test first" },
          { type: "tool_use", id: "tool-1", name: "bash", input: { cmd: "cargo test" } },
        ],
      },
    }),
    byte_start: 140,
  };

  const display = transcriptRecordDisplay(thinkingRecord);

  assert.equal(display.kind, "thinking");
  assert.equal(display.label, "Thinking");
  assert.ok(display.body.includes("inspect the failing test"), `thinking body should contain thinking text, got: ${display.body}`);
  assert.equal(display.body.includes("cargo test"), false, "tool command should not replace thinking body");
});

test("transcriptRecordDisplay classifies reasoning records as thinking", () => {
  const reasoningRecord = {
    id: "r1",
    kind: "reasoning",
    summary: "Reasoning about approach",
    raw: JSON.stringify({
      type: "response_item",
      payload: {
        type: "reasoning",
        summary: [{ type: "summary_text", text: "Considering the trade-offs" }],
      },
    }),
    byte_start: 200,
  };

  const display = transcriptRecordDisplay(reasoningRecord);
  assert.equal(display.kind, "thinking");
  assert.equal(display.label, "Thinking");
});

test("workbench JSONL lens renders thinking records with filter chip and proper CSS class", () => {
  const records = [
    {
      id: "t1",
      kind: "thinking",
      role: "assistant",
      summary: "Analyzing error",
      raw: JSON.stringify({
        type: "assistant",
        message: {
          role: "assistant",
          content: [{ type: "thinking", thinking: "The bug is in the retry logic" }],
        },
      }),
      byte_start: 100,
    },
    {
      id: "c1",
      kind: "function_call",
      summary: "exec: grep -r retry",
      raw: JSON.stringify({ type: "function_call", name: "exec", arguments: { cmd: "grep -r retry" } }),
      byte_start: 200,
    },
  ];

  const html = renderWorkbenchLogLens("", {
    records,
    logState: { mode: "lens", filter: "all", query: "" },
  });

  assert.ok(html.includes('workbench-log-chip-thinking'), "should have thinking chip");
  assert.ok(html.includes('workbench-log-block-thinking'), "should have thinking block CSS class");
  assert.ok(html.includes("2/2 shown"), "should show both records");
});

test("workbench JSONL lens filters to thinking records only", () => {
  const records = [
    {
      id: "t1",
      kind: "thinking",
      role: "assistant",
      summary: "Deep analysis",
      raw: JSON.stringify({ type: "assistant", message: { role: "assistant", content: [{ type: "thinking", thinking: "root cause" }] } }),
      byte_start: 100,
    },
    {
      id: "m1",
      kind: "message",
      role: "user",
      summary: "user prompt",
      raw: JSON.stringify({ type: "message", role: "user", content: "fix the bug" }),
      byte_start: 200,
    },
  ];

  const html = renderWorkbenchLogLens("", {
    records,
    logState: { mode: "lens", filter: "thinking", query: "" },
  });

  assert.ok(html.includes("1/2 shown"), "only thinking record should be shown");
  assert.ok(html.includes('workbench-log-block-thinking'), "thinking record should appear");
});

test("recordMatchesSearch finds text in thinking record body", () => {
  const record = transcriptRecordDisplay({
    id: "t1",
    kind: "thinking",
    summary: "analyzing",
    raw: JSON.stringify({
      type: "assistant",
      message: { role: "assistant", content: [{ type: "thinking", thinking: "database pool exhaustion" }] },
    }),
    byte_start: 0,
  });

  assert.ok(recordMatchesSearch(record, "pool"), "should find 'pool' in thinking body");
  assert.ok(!recordMatchesSearch(record, "nonexistent"), "should not match absent text");
});
