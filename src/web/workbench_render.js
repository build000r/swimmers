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

export function truncateWorkbenchText(value, max = 180) {
  const normalized = String(value || "").replace(/\s+/g, " ").trim();
  if (normalized.length <= max) {
    return normalized;
  }
  return `${normalized.slice(0, Math.max(0, max - 3))}...`;
}

export function agentActionLabel(action) {
  if (!action) {
    return "";
  }
  const tool = String(action.tool || "action").trim() || "action";
  const detail = String(action.detail || "").trim();
  return detail ? `${tool}: ${detail}` : tool;
}

export function operatorPressureSummary(session, payload) {
  if (!session) {
    return "No pressure cues.";
  }
  const cues = [];
  const actionCues = Array.isArray(session.action_cues) ? session.action_cues : [];
  for (const cue of actionCues.slice(0, 3)) {
    const kind = String(cue?.kind || "").replace(/_/g, " ").trim();
    if (kind) {
      cues.push(kind);
    }
  }
  if (session.state && session.state !== "idle") {
    cues.push(String(session.state).replace(/_/g, " "));
  }
  if (session.transport_health && session.transport_health !== "healthy") {
    cues.push(`transport ${String(session.transport_health).replace(/_/g, " ")}`);
  }
  if (session.is_stale) {
    cues.push("stale registry");
  }
  const attached = Number(session.attached_clients || 0);
  const staleAttached = Number(session.stale_attached_clients || 0);
  if (attached || staleAttached) {
    cues.push(`${attached} attached${staleAttached ? `, ${staleAttached} stale` : ""}`);
  }
  const tokens = Number(payload?.token_count ?? session.token_count ?? 0);
  const limit = Number(payload?.context_limit ?? session.context_limit ?? 0);
  if (tokens > 0 && limit > 0) {
    const pct = Math.min(999, Math.round((tokens / limit) * 100));
    cues.push(`${pct}% context`);
  }
  return cues.length ? cues.slice(0, 5).join(" · ") : "No pressure cues.";
}

export function renderTerminalWorkbenchActions(actions, payloadAvailable = false) {
  if (!Array.isArray(actions) || !actions.length) {
    return `<li class="workbench-action"><span class="workbench-action-detail">${escapeHtml(payloadAvailable ? "No recent actions." : "No structured actions.")}</span></li>`;
  }
  return actions
    .slice(0, 6)
    .map((action) => {
      const toolLabel = truncateWorkbenchText(action?.tool || "action", 44);
      const detail = truncateWorkbenchText(action?.detail || "", 160);
      return `
        <li class="workbench-action">
          <span class="workbench-action-tool">${escapeHtml(toolLabel)}</span>
          <span class="workbench-action-detail">${escapeHtml(detail || "No detail.")}</span>
        </li>
      `;
    })
    .join("");
}

function tailLineCount(text) {
  const trimmed = String(text || "").trimEnd();
  return trimmed ? trimmed.split(/\n/).length : 0;
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

function transcriptRecordsToLensText(records) {
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

function transcriptRecordsToRawText(records) {
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

function renderDiffHtml(diffText) {
  const text = widgetTextExcerpt(diffText, 6400);
  if (!text.trim()) {
    return "";
  }
  return text
    .split("\n")
    .map((line) => {
      let klass = "diff-line";
      if (line.startsWith("+") && !line.startsWith("+++")) {
        klass += " diff-line-add";
      } else if (line.startsWith("-") && !line.startsWith("---")) {
        klass += " diff-line-del";
      } else if (line.startsWith("@@")) {
        klass += " diff-line-hunk";
      }
      return `<span class="${klass}">${escapeHtml(line || " ")}</span>`;
    })
    .join("\n");
}

function timelineEventsByKind(timeline, kinds) {
  const wanted = new Set(Array.isArray(kinds) ? kinds : [kinds]);
  return Array.isArray(timeline?.events)
    ? timeline.events.filter((event) => wanted.has(event?.kind))
    : [];
}

function renderTimelineEvents(events, emptyText = "No timeline events.") {
  if (!events.length) {
    return `<div>${escapeHtml(emptyText)}</div>`;
  }
  return `
    <ul class="workbench-actions">
      ${events
        .slice(0, 8)
        .map(
          (event) => `
            <li class="workbench-action">
              <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(event?.title || event?.kind || "event", 44))}</span>
              <span class="workbench-action-detail">${escapeHtml(truncateWorkbenchText(event?.summary || "No summary.", 220))}</span>
            </li>
          `,
        )
        .join("")}
    </ul>
  `;
}

function renderTurnsPanel(turns, selectedTurnId) {
  if (!Array.isArray(turns) || !turns.length) {
    return `<div>No user-submitted turns found.</div>`;
  }
  return `
    <div class="workbench-turn-list" role="list">
      ${turns
        .slice(-20)
        .map((turn) => {
          const id = String(turn?.id || "");
          const selected = id && id === selectedTurnId;
          const label = `Turn ${turn?.order || "?"}`;
          const text = truncateWorkbenchText(turn?.text || "", 180);
          const meta = [turn?.source, turn?.timestamp].filter(Boolean).join(" · ");
          return `
            <button class="workbench-turn ${selected ? "is-selected" : ""}" type="button" data-workbench-turn-id="${escapeHtml(id)}" aria-pressed="${selected ? "true" : "false"}">
              <span class="workbench-turn-label">${escapeHtml(label)}</span>
              <span class="workbench-turn-text">${escapeHtml(text || "Empty turn")}</span>
              ${meta ? `<span class="workbench-turn-meta">${escapeHtml(meta)}</span>` : ""}
            </button>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderDiffFileSummaries(files) {
  if (!Array.isArray(files) || !files.length) {
    return "";
  }
  return `
    <ul class="workbench-actions workbench-diff-files">
      ${files
        .slice(0, 8)
        .map((file) => {
          const hunks = Array.isArray(file?.hunks) ? file.hunks.length : 0;
          const meta = `${file?.source || "diff"} ${file?.change || "modified"} +${file?.added_lines || 0}/-${file?.removed_lines || 0}${hunks ? `, ${hunks} hunks` : ""}`;
          return `
            <li class="workbench-action">
              <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(file?.path || "unknown file", 72))}</span>
              <span class="workbench-action-detail">${escapeHtml(meta)}</span>
            </li>
          `;
        })
        .join("")}
    </ul>
  `;
}

function renderSkillsPanel(skillsPayload) {
  if (!skillsPayload) {
    return `<div>Skillbox skills have not loaded.</div>`;
  }
  if (!skillsPayload.available) {
    return `<div>${escapeHtml(skillsPayload.message || "Skillbox skills unavailable.")}</div>`;
  }
  const skills = Array.isArray(skillsPayload.skills) ? skillsPayload.skills : [];
  const issues = Array.isArray(skillsPayload.issues) ? skillsPayload.issues : [];
  const skillsHtml = skills.length
    ? `
      <ul class="workbench-actions">
        ${skills
          .slice(0, 8)
          .map(
            (skill) => `
              <li class="workbench-action">
                <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(skill?.name || "skill", 44))}</span>
                <span class="workbench-action-detail">${escapeHtml(truncateWorkbenchText(skill?.description || skill?.source_bucket || skill?.state || "available", 180))}</span>
              </li>
            `,
          )
          .join("")}
      </ul>
    `
    : `<div>No matching skills.</div>`;
  const issueHtml = issues.length
    ? `<div class="workbench-action-detail">${escapeHtml(`${issues.length} policy issue${issues.length === 1 ? "" : "s"} reported`)}</div>`
    : `<div class="workbench-action-detail">No policy issues reported.</div>`;
  return `${skillsHtml}${issueHtml}`;
}

export function buildWorkbenchWidgetsHtml({
  widgets = {},
  contextPayload = null,
  selectedTurnId = "",
  logState = {},
} = {}) {
  const timeline = widgets.timeline;
  const timelineEvents = Array.isArray(timeline?.events) ? timeline.events : [];
  const paneEvent = timelineEventsByKind(timeline, "pane_tail")[0];
  const artifactEvent = timelineEventsByKind(timeline, "artifact")[0];
  const diffEvent = timelineEventsByKind(timeline, "diff")[0];
  const tailText = widgets.paneTail?.text || paneEvent?.detail || "";
  const transcript = widgets.transcript;
  const transcriptRecords = Array.isArray(transcript?.records) ? transcript.records : [];
  const turns = Array.isArray(transcript?.turns) && transcript.turns.length
    ? transcript.turns
    : Array.isArray(contextPayload?.turns)
      ? contextPayload.turns
      : [];
  const activeTurnId =
    selectedTurnId ||
    transcript?.selected_turn_id ||
    turns.at(-1)?.id ||
    "";
  const transcriptAvailable = Boolean(transcript?.available);
  const transcriptText = transcriptRecordsToLensText(transcriptRecords);
  const transcriptRawText = transcriptRecordsToRawText(transcriptRecords);
  const useTranscriptLogs = transcriptAvailable && (Boolean(transcript?.selected_turn) || transcriptRecords.length > 0);
  const lines = useTranscriptLogs ? transcriptRecords.length : tailLineCount(tailText);
  const artifact = widgets.artifact;
  const gitDiff = widgets.gitDiff;
  const toolActions = [
    contextPayload?.current_tool,
    ...(Array.isArray(contextPayload?.recent_actions) ? contextPayload.recent_actions : []),
  ].filter(Boolean);
  const activityEvents = timelineEventsByKind(timeline, ["task", "tool_call", "context"]);
  const planFiles = Array.isArray(artifact?.plan_files) ? artifact.plan_files : [];
  const artifactAvailable = Boolean(artifact?.available);
  const artifactMeta = artifactAvailable
    ? `${planFiles.length} plan file${planFiles.length === 1 ? "" : "s"}`
    : "unavailable";
  const diffAvailable = Boolean(gitDiff?.available);
  const unstagedDiff = gitDiff?.unstaged_diff || "";
  const stagedDiff = gitDiff?.staged_diff || "";
  const diffText = [stagedDiff, unstagedDiff].filter((part) => String(part || "").trim()).join("\n");
  const diffMeta = diffAvailable
    ? diffText.trim()
      ? gitDiff?.truncated
        ? "truncated"
        : "dirty"
      : "clean"
    : diffEvent?.summary || "unavailable";
  const status = widgets.loading
    ? `<div class="workbench-action-detail">Loading pinned widgets...</div>`
    : widgets.error
      ? `<div class="workbench-action-detail">${escapeHtml(widgets.error)}</div>`
      : "";
  const outputBody = useTranscriptLogs
    ? renderWorkbenchLogLens(transcriptText, {
        title: "Post-turn JSONL",
        rawText: transcriptRawText,
        records: transcriptRecords,
        selectedTurn: transcript?.selected_turn,
        emptyText: "No JSONL records after this turn yet.",
        logState,
      })
    : renderWorkbenchLogLens(tailText, { logState });
  const activityBody = activityEvents.length
    ? `${activityEvents.some((event) => event?.kind === "tool_call") ? `<div class="workbench-action-detail">Tool calls</div>` : ""}${renderTimelineEvents(activityEvents, "No structured activity.")}`
    : toolActions.length
      ? `
        <div class="workbench-action-detail">Tool calls</div>
        <ul class="workbench-actions">
          ${toolActions
            .slice(0, 8)
            .map(
              (action) => `
                <li class="workbench-action">
                  <span class="workbench-action-tool">${escapeHtml(truncateWorkbenchText(action?.tool || "action", 44))}</span>
                  <span class="workbench-action-detail">${escapeHtml(truncateWorkbenchText(action?.detail || "No detail.", 180))}</span>
                </li>
              `,
            )
            .join("")}
        </ul>
      `
      : `<div>No structured activity or Tool calls.</div>`;
  const artifactBody = artifactAvailable
    ? `
      <div>${escapeHtml(artifact.path || "Artifact path unavailable.")}</div>
      ${planFiles.length ? `<div>${escapeHtml(planFiles.join(", "))}</div>` : `<div>No plan files advertised.</div>`}
      <button class="workbench-widget-action" type="button" data-workbench-open-mermaid="true">Open viewer</button>
    `
    : `<div>${escapeHtml(artifact?.error || artifactEvent?.summary || "No Mermaid or plan artifact found.")}</div>`;
  const diffBody = diffAvailable
    ? diffText.trim()
      ? `
        <div>${escapeHtml(gitDiff.status_short || "dirty tree")}</div>
        ${renderDiffFileSummaries(gitDiff.files)}
        <pre class="workbench-diff">${renderDiffHtml(diffText)}</pre>
      `
      : `<div>${escapeHtml(gitDiff.repo_root || gitDiff.cwd || "Repository")} is clean.</div>`
    : `<div>${escapeHtml(gitDiff?.message || diffEvent?.summary || "No git diff available.")}</div>`;
  const skills = widgets.skills;
  const skillsMeta = skills?.available
    ? `${Array.isArray(skills.skills) ? skills.skills.length : 0} skills`
    : "unavailable";

  return `
    ${status}
    <details class="workbench-widget" open>
      <summary>
        <span class="workbench-widget-title">Turns</span>
        <span class="workbench-widget-meta">${turns.length ? `${turns.length} user` : "empty"}</span>
      </summary>
      <div class="workbench-widget-body">${renderTurnsPanel(turns, activeTurnId)}</div>
    </details>
    <details class="workbench-widget" open>
      <summary>
        <span class="workbench-widget-title">Logs</span>
        <span class="workbench-widget-meta">${useTranscriptLogs ? `${lines} records` : lines ? `${lines} lines` : "empty"}</span>
      </summary>
      <div class="workbench-widget-body">${outputBody}</div>
    </details>
    <details class="workbench-widget" ${activityEvents.length || toolActions.length ? "open" : ""}>
      <summary>
        <span class="workbench-widget-title">Activity</span>
        <span class="workbench-widget-meta">${timelineEvents.length ? `${timelineEvents.length} events` : "snapshot"}</span>
      </summary>
      <div class="workbench-widget-body">${activityBody}</div>
    </details>
    <details class="workbench-widget" ${diffAvailable && diffText.trim() ? "open" : ""}>
      <summary>
        <span class="workbench-widget-title">Diffs</span>
        <span class="workbench-widget-meta">${escapeHtml(diffMeta)}</span>
      </summary>
      <div class="workbench-widget-body">${diffBody}</div>
    </details>
    <details class="workbench-widget">
      <summary>
        <span class="workbench-widget-title">Artifacts</span>
        <span class="workbench-widget-meta">${escapeHtml(artifactMeta)}</span>
      </summary>
      <div class="workbench-widget-body">${artifactBody}</div>
    </details>
    <details class="workbench-widget">
      <summary>
        <span class="workbench-widget-title">Skills</span>
        <span class="workbench-widget-meta">${escapeHtml(skillsMeta)}</span>
      </summary>
      <div class="workbench-widget-body">${renderSkillsPanel(skills)}</div>
    </details>
  `;
}
