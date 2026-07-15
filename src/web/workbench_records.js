const WORKBENCH_LOG_KIND_LABELS = {
  all: "All",
  operator: "Chat",
  command: "Command",
  status: "Status",
  diff: "Diff",
  output: "Output",
  thinking: "Thinking",
  truncation: "Trimmed",
};

const WORKBENCH_LOG_COMMAND_RE =
  /^(?:cargo|make|git|node|bun|npm|pnpm|yarn|python3?|pytest|uv|xcodebuild|swift|curl|tmux|cat|sed|rg|grep|ls|cd|cp|mv|mkdir|touch|chmod|ssh|docker|kubectl)\b/;

function truncateWorkbenchText(value, max = 180) {
  const normalized = String(value || "").replace(/\s+/g, " ").trim();
  if (normalized.length <= max) {
    return normalized;
  }
  return `${normalized.slice(0, Math.max(0, max - 3))}...`;
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

function parseJsonObject(text) {
  try {
    const parsed = JSON.parse(String(text || ""));
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

function parseNestedJsonObject(value) {
  if (value && typeof value === "object") {
    return value;
  }
  if (typeof value !== "string" || !value.trim().startsWith("{")) {
    return null;
  }
  return parseJsonObject(value);
}

function compactJsonValue(value, limit = 360) {
  if (value === undefined || value === null) {
    return "";
  }
  try {
    const text = typeof value === "string" ? value : JSON.stringify(value);
    return truncateWorkbenchText(String(text || "").replace(/\r/g, "").trim(), limit);
  } catch {
    return "";
  }
}

function payloadTextContent(value) {
  if (typeof value === "string") {
    return value;
  }
  if (Array.isArray(value)) {
    return value
      .map((block) => {
        if (typeof block === "string") {
          return block;
        }
        if (block?.type === "tool_use") {
          const input = compactJsonValue(block.input, 300);
          return [block.name || "tool_use", input].filter(Boolean).join(": ");
        }
        if (block?.type === "tool_result") {
          return payloadTextContent(block.content) || compactJsonValue(block, 300);
        }
        if (block?.type === "thinking") {
          return block.thinking || "";
        }
        if (typeof block?.text === "string") {
          return block.text;
        }
        if (typeof block?.content === "string") {
          return block.content;
        }
        return payloadTextContent(block?.content);
      })
      .map((part) => String(part || "").trim())
      .filter(Boolean)
      .join("\n");
  }
  if (value && typeof value === "object") {
    return compactJsonValue(value);
  }
  return "";
}

function transcriptRecordEnvelope(record) {
  const raw = String(record?.raw || "").trim();
  const parsed = parseJsonObject(raw);
  const message = parsed?.message && typeof parsed.message === "object" ? parsed.message : null;
  const payload = parsed?.payload && typeof parsed.payload === "object"
    ? parsed.payload
    : message || (parsed && typeof parsed === "object" ? parsed : {});
  return { raw, parsed, payload, message };
}

function payloadToolUseBlock(payload) {
  const content = Array.isArray(payload?.content) ? payload.content : [];
  return content.find((block) => block && typeof block === "object" && block.type === "tool_use") || null;
}

function payloadToolResultBlock(payload) {
  const content = Array.isArray(payload?.content) ? payload.content : [];
  return content.find((block) => block && typeof block === "object" && block.type === "tool_result") || null;
}

function payloadThinkingText(payload) {
  const content = Array.isArray(payload?.content) ? payload.content : [];
  return content
    .filter((block) => block && typeof block === "object" && /thinking|reasoning/.test(String(block.type || "")))
    .map((block) => block.thinking || block.text || payloadTextContent(block.content))
    .map((part) => String(part || "").trim())
    .filter(Boolean)
    .join("\n");
}

function readableRecordSummary(record, raw) {
  const summary = String(record?.summary || "").trim();
  if (!summary || summary === raw || /^[\[{]/.test(summary)) {
    return "";
  }
  return summary;
}

function compactRecordFields(value) {
  if (!value || typeof value !== "object") {
    return "";
  }
  const skipped = new Set(["payload", "message", "content", "signature", "thinking"]);
  return Object.entries(value)
    .filter(([key, entry]) => !skipped.has(key) && entry !== undefined && entry !== null && entry !== "")
    .slice(0, 5)
    .map(([key, entry]) => `${key}: ${compactJsonValue(entry, 160)}`)
    .join("\n");
}

function payloadMessageText(payload) {
  if (typeof payload?.message === "string") {
    return payload.message;
  }
  if (payload?.message && typeof payload.message === "object") {
    return payloadTextContent(payload.message.content) || compactRecordFields(payload.message);
  }
  return "";
}

function transcriptRecordIsCall(kind) {
  return /^(function_call|custom_tool_call)$/.test(String(kind || ""));
}

function transcriptRecordIsCallOutput(kind) {
  return /^(function_call_output|custom_tool_call_output)$/.test(String(kind || ""));
}

function transcriptRecordDisplayKind(record, payload) {
  const kind = String(record?.kind || payload?.type || "record");
  if (/thinking|reasoning/.test(kind)) {
    return "thinking";
  }
  if (payloadToolResultBlock(payload)) {
    return "output";
  }
  if (transcriptRecordIsCallOutput(kind)) {
    return "output";
  }
  if (payloadToolUseBlock(payload)) {
    return "command";
  }
  if (transcriptRecordIsCall(kind)) {
    return "command";
  }
  if (/agent_message|assistant_message|message|user_message/.test(kind)) {
    return "operator";
  }
  if (/token_count|session_meta|turn_context|compacted|patch_apply/.test(kind)) {
    return "status";
  }
  if (/diff|patch/.test(kind)) {
    return "diff";
  }
  if (record?.truncated) {
    return "truncation";
  }
  return transcriptLineKind(record?.summary || record?.raw || "");
}

function transcriptRecordMeta(record, parsed) {
  const pieces = [];
  const source = String(record?.source || "").trim();
  const rawType = String(parsed?.type || "").trim();
  const cursor = Number(record?.byte_start || 0);
  if (source) {
    pieces.push(source);
  }
  if (rawType) {
    pieces.push(rawType);
  }
  if (cursor > 0) {
    pieces.push(`@${cursor}`);
  }
  if (record?.truncated) {
    pieces.push("trimmed");
  }
  return pieces.join(" · ");
}

export function transcriptRecordDisplay(record) {
  const { raw, parsed, payload } = transcriptRecordEnvelope(record);
  const kind = String(record?.kind || payload?.type || parsed?.type || "record");
  const displayKind = transcriptRecordDisplayKind(record, payload);
  const title = kind.replace(/_/g, " ");
  const summary = readableRecordSummary(record, raw);
  const role = record?.role || payload?.role || "";
  const toolUse = payloadToolUseBlock(payload);
  const toolResult = payloadToolResultBlock(payload);
  const fields = [];
  let body = "";

  if (/thinking|reasoning/.test(kind)) {
    body = payloadThinkingText(payload) || payloadTextContent(payload?.content) || payloadMessageText(payload) || summary || "Thinking";
    if (role) {
      fields.push(["role", role]);
    }
  } else if (toolUse || transcriptRecordIsCall(kind)) {
    const name = payload?.name || toolUse?.name || summary.split(":")[0] || "tool";
    const args = parseNestedJsonObject(payload?.arguments || payload?.input || toolUse?.input);
    const command = args?.cmd || args?.command || "";
    const workdir = args?.workdir || args?.cwd || "";
    body = command || payloadTextContent(payload?.input || toolUse?.input) || summary || name;
    fields.push(["tool", name]);
    if (workdir) {
      fields.push(["cwd", workdir]);
    }
    if (toolUse?.id) {
      fields.push(["call", toolUse.id]);
    }
  } else if (toolResult || transcriptRecordIsCallOutput(kind)) {
    const output = parseNestedJsonObject(payload?.output || toolResult?.content);
    body =
      output?.output ||
      output?.error ||
      payloadTextContent(payload?.output || toolResult?.content) ||
      summary ||
      "Tool output";
    if (payload?.call_id || toolResult?.tool_use_id) {
      fields.push(["call", payload?.call_id || toolResult.tool_use_id]);
    }
  } else if (/agent_message|assistant_message|message|user_message/.test(kind)) {
    body = payloadMessageText(payload) || payloadTextContent(payload?.content) || summary || "Message";
    if (role) {
      fields.push(["role", role]);
    }
  } else if (/token_count/.test(kind)) {
    const usage = payload?.info?.total_token_usage || payload?.usage || {};
    const contextWindow = payload?.model_context_window || payload?.info?.model_context_window;
    if (usage?.input_tokens !== undefined) {
      fields.push(["input", String(usage.input_tokens)]);
    }
    if (usage?.output_tokens !== undefined) {
      fields.push(["output", String(usage.output_tokens)]);
    }
    if (contextWindow !== undefined) {
      fields.push(["window", String(contextWindow)]);
    }
    body = summary || "Token usage update";
  } else if (/patch_apply/.test(kind)) {
    body = payloadMessageText(payload) || summary || kind.replace(/_/g, " ");
  } else {
    body = payloadMessageText(payload) || payloadTextContent(payload?.content) || summary || compactRecordFields(payload) || kind;
  }

  return {
    id: record?.id || `${kind}-${record?.byte_start || 0}`,
    kind: displayKind,
    label: WORKBENCH_LOG_KIND_LABELS[displayKind] || "Output",
    title,
    meta: transcriptRecordMeta(record, parsed),
    fields,
    body: truncateWorkbenchText(String(body || "").replace(/\r/g, ""), 1400),
    raw,
  };
}

export function recordMatchesSearch(record, query) {
  const needle = String(query || "").trim().toLowerCase();
  if (!needle) {
    return true;
  }
  return [record.label, record.title, record.meta, record.body, record.raw]
    .join("\n")
    .toLowerCase()
    .includes(needle);
}
