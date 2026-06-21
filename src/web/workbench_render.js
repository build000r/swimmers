import {
  WORKBENCH_LOG_FILTERS,
  renderWorkbenchLogLens,
  renderTranscriptBlocks,
  transcriptRecordsToRawText,
} from "./workbench_log_lens.js";

export {
  WORKBENCH_LOG_FILTERS,
  renderWorkbenchLogLens,
  renderTranscriptBlocks,
} from "./workbench_log_lens.js";

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
  // The agent-context normalizer coerces absent numerics to 0, and `??` does
  // not fall through 0 — so prefer the payload value only when it is positive,
  // else fall back to the session's count.
  const tokens = Number(payload?.token_count) > 0 ? Number(payload.token_count) : Number(session.token_count || 0);
  const limit = Number(payload?.context_limit) > 0 ? Number(payload.context_limit) : Number(session.context_limit || 0);
  // Context fill % predicts an imminent compaction/reset, so it is the most
  // decision-relevant cue for an agent cockpit — keep it first so the 5-cue cap
  // can never silently drop it behind action cues and transport noise.
  const contextCue =
    tokens > 0 && limit > 0 ? `${Math.min(999, Math.round((tokens / limit) * 100))}% context` : null;
  const ordered = contextCue ? [contextCue, ...cues] : cues;
  return ordered.length ? ordered.slice(0, 5).join(" · ") : "No pressure cues.";
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

export function workbenchWidgetClickPlan(target) {
  const turnButton = target?.closest?.("[data-workbench-turn-id]");
  if (turnButton) {
    return {
      type: "select_turn",
      turnId: String(turnButton.dataset?.workbenchTurnId || ""),
    };
  }

  const logModeButton = target?.closest?.("[data-workbench-log-mode]");
  if (logModeButton) {
    return {
      type: "set_log_mode",
      mode: logModeButton.dataset?.workbenchLogMode === "raw" ? "raw" : "lens",
    };
  }

  return target?.closest?.("[data-workbench-open-mermaid]")
    ? { type: "open_mermaid" }
    : { type: "ignore" };
}

export function workbenchWidgetLogPlan(eventType, target, filters = WORKBENCH_LOG_FILTERS) {
  if (eventType === "input" && target?.matches?.("[data-workbench-log-search]")) {
    return { type: "set_log_search", query: target.value || "" };
  }
  if (eventType === "change" && target?.matches?.("[data-workbench-log-filter]")) {
    return {
      type: "set_log_filter",
      filter: filters.includes(target.value) ? target.value : "all",
    };
  }
  return { type: "ignore" };
}

export function emptyWorkbenchWidgets(overrides = {}) {
  return {
    sessionId: null,
    loading: false,
    timeline: null,
    skills: null,
    paneTail: null,
    transcript: null,
    transcriptTurnId: "",
    transcriptNextCursor: 0,
    artifact: null,
    gitDiff: null,
    error: "",
    requestSeq: 0,
    lastLoadedAt: 0,
    lastHtml: "",
    ...overrides,
  };
}

export function resetWorkbenchWidgetsState(widgets, sessionId) {
  Object.assign(
    widgets,
    emptyWorkbenchWidgets({
      sessionId,
      requestSeq: Number(widgets?.requestSeq || 0),
    }),
  );
  return widgets;
}

export function selectedWorkbenchWidgetsSnapshot(widgets, selectedSessionId) {
  if (widgets?.sessionId === selectedSessionId) {
    return widgets;
  }
  return emptyWorkbenchWidgets({
    requestSeq: Number(widgets?.requestSeq || 0),
    lastLoadedAt: Number(widgets?.lastLoadedAt || 0),
  });
}

export function workbenchWidgetsHaveCurrentPayload(widgets, sessionId) {
  return Boolean(
    widgets?.sessionId === sessionId &&
      (widgets.timeline ||
        widgets.skills ||
        widgets.paneTail ||
        widgets.transcript ||
        widgets.artifact),
  );
}

export function shouldThrottleWorkbenchWidgets({
  options = {},
  widgets,
  sessionId,
  now = Date.now(),
  throttleMs,
} = {}) {
  return Boolean(
    options.throttle &&
      workbenchWidgetsHaveCurrentPayload(widgets, sessionId) &&
      now - Number(widgets?.lastLoadedAt || 0) < throttleMs,
  );
}

export function buildWorkbenchWidgetRequestPlan({
  sessionId,
  selectedTurnId = "",
  widgets,
  force = false,
} = {}) {
  const encodedSessionId = encodeURIComponent(sessionId || "");
  const requestedTurnId = String(selectedTurnId || "");
  const canDeltaTranscript = Boolean(
    !force &&
      widgets?.sessionId === sessionId &&
      widgets?.transcript &&
      widgets?.transcriptTurnId === requestedTurnId &&
      Number(widgets?.transcriptNextCursor || 0) > 0,
  );
  const transcriptParams = new URLSearchParams();
  if (requestedTurnId) {
    transcriptParams.set("turn_id", requestedTurnId);
  }
  if (canDeltaTranscript) {
    transcriptParams.set("after", String(widgets.transcriptNextCursor));
  }
  transcriptParams.set("limit", canDeltaTranscript ? "80" : "160");

  return {
    requestedTurnId,
    canDeltaTranscript,
    paths: {
      timeline: `/v1/sessions/${encodedSessionId}/timeline`,
      skills: `/v1/sessions/${encodedSessionId}/skills?source=sbp`,
      paneTail: `/v1/sessions/${encodedSessionId}/pane-tail`,
      transcript: `/v1/sessions/${encodedSessionId}/transcript?${transcriptParams.toString()}`,
      artifact: `/v1/sessions/${encodedSessionId}/mermaid-artifact`,
      gitDiff: `/v1/sessions/${encodedSessionId}/git-diff`,
    },
  };
}

export function mergeWorkbenchTranscriptPage({
  previous,
  nextTranscript,
  canDeltaTranscript = false,
  requestedTurnId = "",
  selectedTurnId = "",
} = {}) {
  if (!nextTranscript) {
    return {
      transcript: null,
      transcriptTurnId: "",
      transcriptNextCursor: 0,
      selectedTurnId,
    };
  }

  const transcript = { ...nextTranscript };
  const previousRecords = Array.isArray(previous?.records) ? previous.records : [];
  const nextRecords = Array.isArray(transcript.records) ? transcript.records : [];
  const mergeDelta =
    canDeltaTranscript &&
    previous &&
    (transcript?.selected_turn_id || "") === (previous?.selected_turn_id || "");
  if (mergeDelta) {
    const byId = new Map();
    for (const record of previousRecords.concat(nextRecords)) {
      if (record?.id) {
        byId.set(record.id, record);
      }
    }
    transcript.records = Array.from(byId.values())
      // Total, deterministic order so records sharing a byte_start (or a missing
      // byte_start defaulted to 0) do not reorder between delta and full fetches.
      .sort(
        (left, right) =>
          Number(left?.byte_start || 0) - Number(right?.byte_start || 0) ||
          Number(left?.byte_end || 0) - Number(right?.byte_end || 0) ||
          String(left?.id || "").localeCompare(String(right?.id || "")),
      )
      .slice(-240);
  }

  return {
    transcript,
    transcriptTurnId: requestedTurnId || transcript?.selected_turn_id || "",
    transcriptNextCursor: Number(transcript?.next_cursor || 0),
    selectedTurnId: selectedTurnId || transcript?.selected_turn_id || "",
  };
}

function applySettledWidgetResult(widgets, field, result, errorLabel, errors) {
  if (result?.status === "fulfilled") {
    widgets[field] = result.value;
  } else {
    widgets[field] = null;
    errors.push(`${errorLabel}: ${result?.reason?.message || "unavailable"}`);
  }
}

export function applyWorkbenchWidgetResults(widgets, results = {}, options = {}) {
  const errors = [];
  let selectedTurnId = String(options.selectedTurnId || "");
  applySettledWidgetResult(widgets, "timeline", results.timelineResult, "timeline", errors);
  applySettledWidgetResult(widgets, "skills", results.skillsResult, "skills", errors);
  applySettledWidgetResult(widgets, "paneTail", results.tailResult, "output", errors);

  if (results.transcriptResult?.status === "fulfilled") {
    const merged = mergeWorkbenchTranscriptPage({
      previous: widgets.transcript,
      nextTranscript: results.transcriptResult.value,
      canDeltaTranscript: options.canDeltaTranscript,
      requestedTurnId: options.requestedTurnId,
      selectedTurnId,
    });
    widgets.transcript = merged.transcript;
    widgets.transcriptTurnId = merged.transcriptTurnId;
    widgets.transcriptNextCursor = merged.transcriptNextCursor;
    selectedTurnId = merged.selectedTurnId;
  } else {
    widgets.transcript = null;
    widgets.transcriptTurnId = "";
    widgets.transcriptNextCursor = 0;
    errors.push(`transcript: ${results.transcriptResult?.reason?.message || "unavailable"}`);
  }

  applySettledWidgetResult(widgets, "artifact", results.artifactResult, "artifacts", errors);
  applySettledWidgetResult(widgets, "gitDiff", results.diffResult, "diffs", errors);
  widgets.error = errors.join("; ");
  widgets.loading = false;
  return { selectedTurnId, error: widgets.error };
}

function renderDiffHtml(diffText) {
  const normalized = String(diffText || "").replace(/\r/g, "");
  // Keep the HEAD when truncating so the rendered hunks line up with the file
  // summaries shown above them, which the backend parses from the head of the
  // diff. (widgetTextExcerpt keeps the tail, which is right for logs but
  // desyncs a diff from its summary.)
  const text =
    normalized.length <= 6400 ? normalized : `${normalized.slice(0, 6400)}\n... truncated ...`;
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

export function buildWorkbenchWidgetsViewModel({
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
  const useTranscriptLogs = transcriptAvailable && (Boolean(transcript?.selected_turn) || transcriptRecords.length > 0);
  const transcriptRawText = useTranscriptLogs ? transcriptRecordsToRawText(transcriptRecords) : "";
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
  // Untracked files never appear in `git diff`/`git diff --cached`, so an empty
  // diffText with a non-empty `git status --short` is dirty, not clean.
  const statusShort = String(gitDiff?.status_short || "").trim();
  const diffMeta = diffAvailable
    ? diffText.trim()
      ? gitDiff?.truncated
        ? "truncated"
        : "dirty"
      : statusShort
        ? "untracked"
        : "clean"
    : diffEvent?.summary || "unavailable";
  const outputBody = useTranscriptLogs
    ? renderWorkbenchLogLens("", {
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
      : statusShort
        ? `
        <div>${escapeHtml(gitDiff.repo_root || gitDiff.cwd || "Repository")} has uncommitted changes (untracked or unstaged):</div>
        <pre class="workbench-diff">${escapeHtml(statusShort)}</pre>
      `
        : `<div>${escapeHtml(gitDiff.repo_root || gitDiff.cwd || "Repository")} is clean.</div>`
    : `<div>${escapeHtml(gitDiff?.message || diffEvent?.summary || "No git diff available.")}</div>`;
  const skills = widgets.skills;
  const skillsMeta = skills?.available
    ? `${Array.isArray(skills.skills) ? skills.skills.length : 0} skills`
    : "unavailable";

  return {
    statusText: widgets.loading
      ? "Loading pinned widgets..."
      : widgets.error
        ? String(widgets.error)
        : "",
    items: [
      {
        key: "turns",
        title: "Turns",
        meta: turns.length ? `${turns.length} user` : "empty",
        bodyHtml: renderTurnsPanel(turns, activeTurnId),
        open: true,
      },
      {
        key: "logs",
        title: "Logs",
        meta: useTranscriptLogs ? `${lines} records` : lines ? `${lines} lines` : "empty",
        bodyHtml: outputBody,
        open: true,
      },
      {
        key: "activity",
        title: "Activity",
        meta: timelineEvents.length ? `${timelineEvents.length} events` : "snapshot",
        bodyHtml: activityBody,
        open: Boolean(activityEvents.length || toolActions.length),
      },
      {
        key: "diffs",
        title: "Diffs",
        meta: diffMeta,
        bodyHtml: diffBody,
        open: Boolean(diffAvailable && diffText.trim()),
      },
      {
        key: "artifacts",
        title: "Artifacts",
        meta: artifactMeta,
        bodyHtml: artifactBody,
        open: false,
      },
      {
        key: "skills",
        title: "Skills",
        meta: skillsMeta,
        bodyHtml: renderSkillsPanel(skills),
        open: false,
      },
    ],
  };
}

export function renderWorkbenchWidgetsViewModelHtml(model = {}) {
  const items = Array.isArray(model?.items) ? model.items : [];
  const status = model?.statusText
    ? `<div class="workbench-action-detail" role="status" aria-live="polite">${escapeHtml(model.statusText)}</div>`
    : "";
  return `
    ${status}
    ${items
      .map((item) => `
        <details class="workbench-widget" ${item?.open ? "open" : ""}>
          <summary>
            <span class="workbench-widget-title">${escapeHtml(item?.title || "")}</span>
            <span class="workbench-widget-meta">${escapeHtml(item?.meta || "")}</span>
          </summary>
          <div class="workbench-widget-body">${String(item?.bodyHtml || "")}</div>
        </details>
      `)
      .join("")}
  `;
}

export function buildWorkbenchWidgetsHtml(options = {}) {
  return renderWorkbenchWidgetsViewModelHtml(buildWorkbenchWidgetsViewModel(options));
}
