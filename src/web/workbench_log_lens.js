import {
  recordMatchesSearch,
  transcriptRecordDisplay,
} from "./workbench_records.js";

function escapeHtml(text) {
  return String(text || "").replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      case "'":
        return "&#39;";
      default:
        return char;
    }
  });
}

function truncateWorkbenchText(value, max = 180) {
  const normalized = String(value || "").replace(/\s+/g, " ").trim();
  if (normalized.length <= max) {
    return normalized;
  }
  return `${normalized.slice(0, Math.max(0, max - 3))}...`;
}

function widgetTextExcerpt(text, max = 4200) {
  const normalized = String(text || "").replace(/\r/g, "");
  if (normalized.length <= max) {
    return normalized;
  }
  return `... truncated ...\n${normalized.slice(-max)}`;
}

const WORKBENCH_LOG_KIND_LABELS = {
  all: "All",
  operator: "Chat",
  command: "Command",
  status: "Status",
  diff: "Diff",
  output: "Output",
  truncation: "Trimmed",
};

export const WORKBENCH_LOG_FILTERS = ["all", "operator", "command", "status", "diff", "output", "truncation"];

const WORKBENCH_LOG_COMMAND_RE =
  /^(?:cargo|make|git|node|bun|npm|pnpm|yarn|python3?|pytest|uv|xcodebuild|swift|curl|tmux|cat|sed|rg|grep|ls|cd|cp|mv|mkdir|touch|chmod|ssh|docker|kubectl)\b/;

function normalizeWorkbenchLogState(logState = {}) {
  const mode = logState.mode === "raw" ? "raw" : "lens";
  const filter = WORKBENCH_LOG_FILTERS.includes(logState.filter) ? logState.filter : "all";
  const query = String(logState.query || "");
  return { mode, filter, query };
}

function transcriptLineKind(line) {
  const trimmed = String(line || "").trim();
  if (!trimmed) {
    return "output";
  }
  if (/^\.\.\. truncated \.\.\.$/i.test(trimmed) || /^truncated[:\s]/i.test(trimmed)) {
    return "truncation";
  }
  if (/^(?:[•*]\s+|[-]\s+You\b|You ran\b|Using [a-z][\w-]*\b)/i.test(trimmed)) {
    return "operator";
  }
  if (
    /^(?:diff --git|index [0-9a-f]+\.\.|@@\s|---\s|\+\+\+\s)/.test(trimmed) ||
    /^[+][^+]/.test(trimmed) ||
    /^-[^\-\s]/.test(trimmed)
  ) {
    return "diff";
  }
  if (
    /(?:\berror\b|\bfailed\b|\bfatal\b|\bpanic\b|\bwarning\b|\bdenied\b|\brefused\b|\btimed out\b|\bunavailable\b|\bblocked\b)/i.test(trimmed) ||
    /^(?:Finished|Running|Compiling|Waiting|Worked for|Validation|Evidence|PASS|FAIL)\b/i.test(trimmed) ||
    /^[-]\s+(?:Worked for|Evidence)\b/i.test(trimmed)
  ) {
    return "status";
  }
  if (/^(?:[$#❯>]\s+|[A-Za-z0-9_.~/-]+[$#]\s+)/.test(trimmed) || WORKBENCH_LOG_COMMAND_RE.test(trimmed)) {
    return "command";
  }
  return "output";
}

export function renderTranscriptBlocks(text) {
  const lines = String(text || "").replace(/\r/g, "").split("\n");
  const blocks = [];
  let current = null;

  lines.forEach((line, index) => {
    if (!line.trim() && !current) {
      return;
    }
    const kind = transcriptLineKind(line);
    if (current && current.kind === kind) {
      current.lines.push(line);
      current.endLine = index + 1;
      return;
    }
    current = {
      kind,
      label: WORKBENCH_LOG_KIND_LABELS[kind] || "Output",
      lines: [line],
      startLine: index + 1,
      endLine: index + 1,
    };
    blocks.push(current);
  });

  return blocks.filter((block) => block.lines.some((line) => line.trim()));
}

function blockMatchesSearch(block, query) {
  const needle = String(query || "").trim().toLowerCase();
  if (!needle) {
    return true;
  }
  return block.lines.join("\n").toLowerCase().includes(needle);
}

function renderHighlightedLogLine(line, query) {
  const text = String(line || "");
  const needle = String(query || "").trim();
  if (!needle) {
    return escapeHtml(text || " ");
  }

  const lower = text.toLowerCase();
  const lowerNeedle = needle.toLowerCase();
  let cursor = 0;
  let html = "";
  while (cursor < text.length) {
    const index = lower.indexOf(lowerNeedle, cursor);
    if (index < 0) {
      html += escapeHtml(text.slice(cursor));
      break;
    }
    html += escapeHtml(text.slice(cursor, index));
    html += `<mark class="workbench-log-mark">${escapeHtml(text.slice(index, index + needle.length))}</mark>`;
    cursor = index + needle.length;
  }
  return html || escapeHtml(text || " ");
}

function workbenchLogCounts(blocks) {
  return blocks.reduce((counts, block) => {
    counts[block.kind] = (counts[block.kind] || 0) + 1;
    return counts;
  }, {});
}

const WORKBENCH_LOG_PATH_RE = /(?:^|[\s"'`([])((?:~\/|\.{1,2}\/|\/)?[A-Za-z0-9_@%+=:.-][A-Za-z0-9_@%+=:./-]*\.(?:c|cc|cpp|css|h|html|js|jsx|json|jsonl|lock|log|md|mjs|mmd|mmdx|py|rs|sh|toml|ts|tsx|txt|wasm|yaml|yml))(?:$|[\s"'`),\]])/g;

function normalizeWorkbenchBriefText(text, limit = 260) {
  return truncateWorkbenchText(String(text || "").replace(/\r/g, "").replace(/\s+/g, " ").trim(), limit);
}

function uniqueNonEmpty(values) {
  const seen = new Set();
  const result = [];
  for (const value of values) {
    const text = String(value || "").trim();
    if (!text || seen.has(text)) {
      continue;
    }
    seen.add(text);
    result.push(text);
  }
  return result;
}

function extractWorkbenchPaths(text) {
  const paths = [];
  const source = String(text || "");
  for (const match of source.matchAll(WORKBENCH_LOG_PATH_RE)) {
    const path = String(match[1] || "").replace(/[;:.,]+$/, "");
    if (path && !path.startsWith("http")) {
      paths.push(path);
    }
  }
  return paths;
}

function workbenchPathScore(path) {
  const text = String(path || "").toLowerCase();
  let score = 0;
  if (text.includes("result")) {
    score += 80;
  }
  if (text.endsWith(".md") || text.endsWith(".mmd") || text.endsWith(".mmdx")) {
    score += 30;
  }
  if (text.startsWith("target/") || text.includes("/target/")) {
    score += 20;
  }
  if (text.startsWith("/")) {
    score += 10;
  }
  return score;
}

function workbenchRecordBody(record) {
  const body = String(record?.body || "").trim();
  if (!body || body === "Message" || body === "Tool output") {
    return "";
  }
  return body;
}

function workbenchRecordRole(record) {
  const roleField = record?.fields?.find?.(([key]) => key === "role");
  return String(roleField?.[1] || "").trim();
}

function workbenchBriefItems(records, options = {}) {
  const items = [];
  const selectedTurnText = normalizeWorkbenchBriefText(options.selectedTurn?.text || "", 220);
  const userRecord = [...records].reverse().find((record) => workbenchRecordRole(record) === "user");
  const userText = selectedTurnText || normalizeWorkbenchBriefText(workbenchRecordBody(userRecord), 220);
  const outcomeRecord = [...records].reverse().find((record) => {
    const body = workbenchRecordBody(record);
    return body && /baked|blocked|complete|done|error|fail|pass|result|summary|written/i.test(body);
  });
  const assistantRecord = [...records].reverse().find((record) => {
    const role = workbenchRecordRole(record);
    const body = workbenchRecordBody(record);
    return body && (role === "assistant" || /assistant|agent/.test(record?.title || ""));
  });
  const fallbackRecord = [...records].reverse().find((record) => workbenchRecordBody(record));
  const outcomeText = normalizeWorkbenchBriefText(
    workbenchRecordBody(outcomeRecord) || workbenchRecordBody(assistantRecord) || workbenchRecordBody(fallbackRecord),
    280,
  );
  const commands = uniqueNonEmpty(
    records
      .filter((record) => record.kind === "command")
      .map((record) => normalizeWorkbenchBriefText(workbenchRecordBody(record), 120)),
  ).slice(0, 3);
  const paths = uniqueNonEmpty(
    records.flatMap((record) => [
      ...extractWorkbenchPaths(record.body),
      ...extractWorkbenchPaths(record.raw),
      ...(record.fields || []).flatMap(([, value]) => extractWorkbenchPaths(value)),
    ]),
  )
    .sort((left, right) => workbenchPathScore(right) - workbenchPathScore(left))
    .slice(0, 4);

  if (userText) {
    items.push(["User turn", userText]);
  }
  if (outcomeText) {
    items.push(["Outcome", outcomeText]);
  }
  if (commands.length) {
    items.push(["Tool actions", commands.join("\n")]);
  }
  if (paths.length) {
    items.push(["Where to read", paths.join("\n")]);
  }
  return items;
}

function renderWorkbenchLogBrief(records, options = {}) {
  const items = workbenchBriefItems(records, options);
  if (!items.length) {
    return "";
  }
  return `
    <section class="workbench-log-brief" aria-label="Log summary">
      <div class="workbench-log-brief-title">Start here</div>
      <div class="workbench-log-brief-items">
        ${items
          .map(
            ([label, value]) => `
              <div class="workbench-log-brief-item">
                <div class="workbench-log-brief-label">${escapeHtml(label)}</div>
                <div class="workbench-log-brief-value">${String(value)
                  .split("\n")
                  .map((line) => `<span>${escapeHtml(line)}</span>`)
                  .join("")}</div>
              </div>
            `,
          )
          .join("")}
      </div>
    </section>
  `;
}

function renderWorkbenchRecordLens(records, options = {}) {
  const parsedRecords = Array.isArray(records) ? records.map(transcriptRecordDisplay) : [];
  const rawText = options.rawText ?? transcriptRecordsToRawText(records);
  const rawExcerpt = widgetTextExcerpt(rawText);
  const rawHasText = Boolean(rawExcerpt.trim());
  const title = options.title || "Post-turn JSONL";
  const emptyText = options.emptyText || "No JSONL records after this turn yet.";
  const counts = workbenchLogCounts(parsedRecords);
  const { mode, filter, query } = normalizeWorkbenchLogState(options.logState);
  const filteredRecords = parsedRecords.filter((record) => {
    const kindMatches = filter === "all" || record.kind === filter;
    return kindMatches && recordMatchesSearch(record, query);
  });

  const controls = renderWorkbenchLogControls(filter, query, mode);
  if (mode === "raw") {
    return `
      <div class="workbench-action-detail">${escapeHtml(title)}</div>
      ${controls}
      ${rawHasText ? `<pre class="workbench-log-raw">${escapeHtml(rawExcerpt)}</pre>` : `<div>${escapeHtml(emptyText)}</div>`}
    `;
  }

  const countChips = renderWorkbenchLogCountChips(counts);
  const briefRecords = filter === "all" && !query.trim() ? parsedRecords : filteredRecords;
  const briefHtml = renderWorkbenchLogBrief(briefRecords, options);
  const recordsHtml = !parsedRecords.length
    ? `<div class="workbench-log-empty">${escapeHtml(emptyText)}</div>`
    : filteredRecords.length
      ? filteredRecords.map((record) => renderWorkbenchLogRecord(record, query)).join("")
      : `<div class="workbench-log-empty">No JSONL records match.</div>`;
  const evidenceOpen = query.trim() || filter !== "all" ? "open" : "";
  const evidenceMeta = parsedRecords.length
    ? `${filteredRecords.length}/${parsedRecords.length} shown`
    : "empty";

  return `
    <div class="workbench-action-detail">${escapeHtml(title)}</div>
    <div class="workbench-log-lens">
      ${briefHtml}
      ${controls}
      ${countChips ? `<div class="workbench-log-chips">${countChips}</div>` : ""}
      <details class="workbench-log-evidence" ${evidenceOpen}>
        <summary>
          <span>Event stream</span>
          <span>${escapeHtml(evidenceMeta)}</span>
        </summary>
        <div class="workbench-log-records">${recordsHtml}</div>
      </details>
    </div>
  `;
}

function renderWorkbenchLogRecord(record, query) {
  const fields = record.fields
    .filter(([, value]) => String(value || "").trim())
    .map(
      ([key, value]) => `
        <span class="workbench-log-field">
          <span class="workbench-log-field-key">${escapeHtml(key)}</span>
          <span class="workbench-log-field-value">${escapeHtml(String(value))}</span>
        </span>
      `,
    )
    .join("");
  const bodyLines = String(record.body || "")
    .split("\n")
    .slice(0, 24)
    .map((line) => `<div class="workbench-log-line">${renderHighlightedLogLine(line, query)}</div>`)
    .join("");
  return `
    <article class="workbench-log-record workbench-log-block workbench-log-block-${record.kind}" data-log-kind="${escapeHtml(record.kind)}">
      <div class="workbench-log-block-header">
        <span>${escapeHtml(record.label)} · ${escapeHtml(record.title)}</span>
        <span>${escapeHtml(record.meta)}</span>
      </div>
      ${fields ? `<div class="workbench-log-fields">${fields}</div>` : ""}
      <div class="workbench-log-block-body">${bodyLines || `<div class="workbench-log-line">${escapeHtml(record.title)}</div>`}</div>
      ${record.raw ? `
        <details class="workbench-log-json">
          <summary>JSON</summary>
          <pre>${escapeHtml(widgetTextExcerpt(record.raw, 2200))}</pre>
        </details>
      ` : ""}
    </article>
  `;
}

export function transcriptRecordsToLensText(records) {
  if (!Array.isArray(records) || !records.length) {
    return "";
  }
  return records
    .map((record) => {
      const kind = String(record?.kind || "record").replace(/_/g, " ");
      const summary = String(record?.summary || "").trim();
      if (/function call|tool call/i.test(kind) && /^exec:\s+/i.test(summary)) {
        return summary.replace(/^exec:\s+/i, "");
      }
      return `${kind}: ${summary || "(empty record)"}`;
    })
    .join("\n");
}

export function transcriptRecordsToRawText(records) {
  if (!Array.isArray(records) || !records.length) {
    return "";
  }
  return records
    .map((record) => String(record?.raw || "").trim())
    .filter(Boolean)
    .join("\n");
}

function renderWorkbenchLogCountChips(counts) {
  return WORKBENCH_LOG_FILTERS.filter((kind) => kind !== "all" && counts[kind])
    .map(
      (kind) => `
        <span class="workbench-log-chip workbench-log-chip-${kind}">
          <span>${escapeHtml(WORKBENCH_LOG_KIND_LABELS[kind])}</span>
          <span class="workbench-log-chip-count">${counts[kind]}</span>
        </span>
      `,
    )
    .join("");
}

function renderWorkbenchLogControls(filter, query, mode) {
  const filterOptions = WORKBENCH_LOG_FILTERS.map(
    (kind) => `<option value="${escapeHtml(kind)}" ${filter === kind ? "selected" : ""}>${escapeHtml(WORKBENCH_LOG_KIND_LABELS[kind])}</option>`,
  ).join("");

  return `
    <div class="workbench-log-toolbar">
      <div class="workbench-log-view-toggle" role="group" aria-label="Log view">
        <button type="button" class="workbench-log-view-button" data-workbench-log-mode="lens" aria-pressed="${mode === "lens" ? "true" : "false"}">Lens</button>
        <button type="button" class="workbench-log-view-button" data-workbench-log-mode="raw" aria-pressed="${mode === "raw" ? "true" : "false"}">Raw</button>
      </div>
      <select class="workbench-log-filter" name="workbench-log-filter" aria-label="Filter log blocks" data-workbench-log-filter>
        ${filterOptions}
      </select>
      <input class="workbench-log-search" type="search" name="workbench-log-search" aria-label="Search logs" placeholder="Search logs" value="${escapeHtml(query)}" data-workbench-log-search />
    </div>
  `;
}

export function renderWorkbenchLogLens(tailText, options = {}) {
  if (Array.isArray(options.records)) {
    return renderWorkbenchRecordLens(options.records, options);
  }

  const excerpt = widgetTextExcerpt(tailText);
  const rawExcerpt = widgetTextExcerpt(options.rawText ?? tailText);
  const hasText = Boolean(excerpt.trim());
  const rawHasText = Boolean(rawExcerpt.trim());
  const title = options.title || "Recent output";
  const emptyText = options.emptyText || "No recent pane output.";
  const blocks = hasText ? renderTranscriptBlocks(excerpt) : [];
  const counts = workbenchLogCounts(blocks);
  const { mode, filter, query } = normalizeWorkbenchLogState(options.logState);
  const filteredBlocks = blocks.filter((block) => {
    const kindMatches = filter === "all" || block.kind === filter;
    return kindMatches && blockMatchesSearch(block, query);
  });
  const countChips = renderWorkbenchLogCountChips(counts);
  const controls = renderWorkbenchLogControls(filter, query, mode);

  if (mode === "raw") {
    return `
      <div class="workbench-action-detail">${escapeHtml(title)}</div>
      ${controls}
      ${rawHasText ? `<pre class="workbench-log-raw">${escapeHtml(rawExcerpt)}</pre>` : `<div>${escapeHtml(emptyText)}</div>`}
    `;
  }

  const blocksHtml = !hasText
    ? `<div class="workbench-log-empty">${escapeHtml(emptyText)}</div>`
    : filteredBlocks.length
    ? filteredBlocks
        .map((block) => {
          const lineRange = block.startLine === block.endLine ? `L${block.startLine}` : `L${block.startLine}-${block.endLine}`;
          const lines = block.lines
            .map((line) => `<div class="workbench-log-line">${renderHighlightedLogLine(line, query)}</div>`)
            .join("");
          return `
            <article class="workbench-log-block workbench-log-block-${block.kind}" data-log-kind="${escapeHtml(block.kind)}">
              <div class="workbench-log-block-header">
                <span>${escapeHtml(block.label)}</span>
                <span>${escapeHtml(lineRange)}</span>
              </div>
              <div class="workbench-log-block-body">${lines}</div>
            </article>
          `;
        })
        .join("")
    : `<div class="workbench-log-empty">No log blocks match.</div>`;

  return `
    <div class="workbench-action-detail">${escapeHtml(title)}</div>
    <div class="workbench-log-lens">
      ${controls}
      ${countChips ? `<div class="workbench-log-chips">${countChips}</div>` : ""}
      <div class="workbench-log-blocks">${blocksHtml}</div>
    </div>
  `;
}
