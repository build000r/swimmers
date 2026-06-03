use crate::thought::context::{
    context_reader_for, AgentAction, AgentTranscriptRecord as ContextTranscriptRecord,
    AgentUserTurn as ContextUserTurn, ContextSnapshot,
};
use crate::types::{
    AgentContextActionSummary, SessionAgentContextResponse, SessionAgentTurn, SessionSummary,
    SessionTimelinePinned, SessionTranscriptRecord, SessionTranscriptResponse,
};

enum ContextReadResult {
    Unsupported,
    Missing,
    Snapshot(StructuredContextSnapshot),
}

struct StructuredContextSnapshot {
    user_task: Option<String>,
    turns: Vec<SessionAgentTurn>,
    transcript_records: Vec<SessionTranscriptRecord>,
    source_size: u64,
    current_tool: Option<AgentContextActionSummary>,
    recent_actions: Vec<AgentContextActionSummary>,
    token_count: u64,
    context_limit: u64,
}

pub(super) async fn read_agent_context_for_summary(
    summary: SessionSummary,
) -> anyhow::Result<SessionAgentContextResponse> {
    let session_id = summary.session_id.clone();
    let tool = summary.tool.clone();
    let cwd = summary.cwd.clone();
    let baseline_token_count = summary.token_count;
    let baseline_context_limit = context_limit_for_agent_context(&tool, summary.context_limit);

    let Some(tool_name) = tool.clone() else {
        return Ok(agent_context_unavailable(
            session_id,
            tool,
            cwd,
            baseline_token_count,
            baseline_context_limit,
            "session tool is unknown",
        ));
    };

    let reader_tool = tool_name.clone();
    let reader_cwd = cwd.clone();
    let read_result =
        tokio::task::spawn_blocking(move || read_context_snapshot(&reader_tool, &reader_cwd))
            .await?;

    Ok(match read_result {
        ContextReadResult::Unsupported => agent_context_unavailable(
            session_id,
            tool,
            cwd,
            baseline_token_count,
            baseline_context_limit,
            format!("structured context is not supported for {tool_name}"),
        ),
        ContextReadResult::Missing => agent_context_unavailable(
            session_id,
            tool,
            cwd,
            baseline_token_count,
            baseline_context_limit,
            "no matching structured JSONL context was found",
        ),
        ContextReadResult::Snapshot(StructuredContextSnapshot {
            user_task,
            turns,
            current_tool,
            recent_actions,
            token_count,
            context_limit,
            ..
        }) => SessionAgentContextResponse {
            session_id,
            available: true,
            tool,
            cwd,
            user_task,
            turns,
            current_tool,
            recent_actions,
            token_count,
            context_limit: context_limit_for_agent_context(&Some(tool_name), context_limit),
            message: None,
        },
    })
}

pub(super) async fn read_transcript_for_summary(
    summary: SessionSummary,
    query: super::TranscriptQuery,
) -> anyhow::Result<SessionTranscriptResponse> {
    let session_id = summary.session_id.clone();
    let tool = summary.tool.clone();
    let cwd = summary.cwd.clone();
    let Some(tool_name) = tool.clone() else {
        return Ok(transcript_unavailable(
            session_id,
            tool,
            cwd,
            "session tool is unknown",
        ));
    };

    let reader_tool = tool_name.clone();
    let reader_cwd = cwd.clone();
    let read_result =
        tokio::task::spawn_blocking(move || read_context_snapshot(&reader_tool, &reader_cwd))
            .await?;

    Ok(match read_result {
        ContextReadResult::Unsupported => transcript_unavailable(
            session_id,
            tool,
            cwd,
            format!("structured transcript is not supported for {tool_name}"),
        ),
        ContextReadResult::Missing => transcript_unavailable(
            session_id,
            tool,
            cwd,
            "no matching structured JSONL transcript was found",
        ),
        ContextReadResult::Snapshot(StructuredContextSnapshot {
            turns,
            transcript_records,
            source_size,
            ..
        }) => build_transcript_response(
            session_id,
            tool,
            cwd,
            turns,
            transcript_records,
            source_size,
            query,
        ),
    })
}

pub(super) fn agent_context_unavailable(
    session_id: String,
    tool: Option<String>,
    cwd: String,
    token_count: u64,
    context_limit: u64,
    message: impl Into<String>,
) -> SessionAgentContextResponse {
    SessionAgentContextResponse {
        session_id,
        available: false,
        tool,
        cwd,
        user_task: None,
        turns: Vec::new(),
        current_tool: None,
        recent_actions: Vec::new(),
        token_count,
        context_limit,
        message: Some(message.into()),
    }
}

pub(super) fn context_limit_for_agent_context(tool: &Option<String>, context_limit: u64) -> u64 {
    if context_limit > 0 {
        context_limit
    } else {
        crate::types::context_limit_for_tool(tool.as_deref())
    }
}

pub(super) fn append_context_events(
    builder: &mut super::TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    context: &SessionAgentContextResponse,
) {
    if let Some(task) = context
        .user_task
        .as_deref()
        .filter(|task| !task.trim().is_empty())
    {
        let summary = super::timeline_excerpt(task, 180);
        let event_id = builder.push(
            "task",
            "task",
            "agent-context",
            "Task",
            summary.clone(),
            Some(task.to_string()),
        );
        pinned.task = Some(super::pinned_item(
            "Task",
            summary,
            "agent-context",
            event_id,
        ));
    }

    if let Some(action) = context.current_tool.as_ref() {
        let summary = action
            .detail
            .as_deref()
            .filter(|detail| !detail.trim().is_empty())
            .unwrap_or(&action.tool);
        let summary = super::timeline_excerpt(summary, 180);
        let event_id = builder.push(
            "current-action",
            "tool_call",
            "agent-context",
            action.tool.clone(),
            summary.clone(),
            action.detail.clone(),
        );
        pinned.current_action = Some(super::pinned_item(
            action.tool.clone(),
            summary,
            "agent-context",
            event_id,
        ));
    }

    for (index, action) in context.recent_actions.iter().take(8).enumerate() {
        let summary = action
            .detail
            .as_deref()
            .filter(|detail| !detail.trim().is_empty())
            .unwrap_or(&action.tool);
        builder.push(
            format!("recent-action-{}", index + 1),
            "tool_call",
            "agent-context",
            action.tool.clone(),
            super::timeline_excerpt(summary, 180),
            action.detail.clone(),
        );
    }

    if !context.available {
        let message = context
            .message
            .as_deref()
            .unwrap_or("structured context unavailable");
        builder.push(
            "context-unavailable",
            "context",
            "agent-context",
            "Context unavailable",
            super::timeline_excerpt(message, 180),
            None,
        );
    }
}

fn read_context_snapshot(tool: &str, cwd: &str) -> ContextReadResult {
    let Some(mut reader) = context_reader_for(tool, cwd, &[]) else {
        return ContextReadResult::Unsupported;
    };

    let Some(snapshot) = reader.read() else {
        return ContextReadResult::Missing;
    };

    ContextReadResult::Snapshot(structured_context_snapshot(snapshot))
}

fn structured_context_snapshot(snapshot: ContextSnapshot) -> StructuredContextSnapshot {
    StructuredContextSnapshot {
        user_task: snapshot.user_task,
        turns: snapshot
            .user_turns
            .into_iter()
            .map(agent_turn_summary)
            .collect(),
        transcript_records: snapshot
            .transcript_records
            .into_iter()
            .map(transcript_record_summary)
            .collect(),
        source_size: snapshot.source_size,
        current_tool: snapshot.current_tool.map(agent_action_summary),
        recent_actions: snapshot
            .recent_actions
            .into_iter()
            .map(agent_action_summary)
            .collect(),
        token_count: snapshot.token_count,
        context_limit: snapshot.context_limit,
    }
}

fn build_transcript_response(
    session_id: String,
    tool: Option<String>,
    cwd: String,
    turns: Vec<SessionAgentTurn>,
    transcript_records: Vec<SessionTranscriptRecord>,
    source_size: u64,
    query: super::TranscriptQuery,
) -> SessionTranscriptResponse {
    let selected_turn = query
        .turn_id
        .as_deref()
        .and_then(|turn_id| turns.iter().find(|turn| turn.id == turn_id).cloned())
        .or_else(|| turns.last().cloned());
    let turn_cursor = selected_turn
        .as_ref()
        .map(|turn| turn.byte_end)
        .unwrap_or(0);
    let cursor = query.after.unwrap_or(turn_cursor).max(turn_cursor);
    let limit = query.limit.unwrap_or(80).clamp(1, 240);
    let records = transcript_records
        .into_iter()
        .filter(|record| record.byte_start >= cursor)
        .take(limit)
        .collect::<Vec<_>>();
    let next_cursor = records
        .iter()
        .map(|record| record.byte_end)
        .max()
        .unwrap_or_else(|| source_size.max(cursor));

    SessionTranscriptResponse {
        session_id,
        available: true,
        tool,
        cwd,
        selected_turn_id: selected_turn.as_ref().map(|turn| turn.id.clone()),
        selected_turn,
        next_cursor,
        records,
        turns,
        message: None,
    }
}

fn transcript_unavailable(
    session_id: String,
    tool: Option<String>,
    cwd: String,
    message: impl Into<String>,
) -> SessionTranscriptResponse {
    SessionTranscriptResponse {
        session_id,
        available: false,
        tool,
        cwd,
        selected_turn_id: None,
        selected_turn: None,
        next_cursor: 0,
        records: Vec::new(),
        turns: Vec::new(),
        message: Some(message.into()),
    }
}

fn agent_action_summary(action: AgentAction) -> AgentContextActionSummary {
    AgentContextActionSummary {
        tool: action.tool,
        detail: action.detail,
    }
}

fn agent_turn_summary(turn: ContextUserTurn) -> SessionAgentTurn {
    SessionAgentTurn {
        id: turn.id,
        source: turn.source,
        text: turn.text,
        byte_start: turn.byte_start,
        byte_end: turn.byte_end,
        order: turn.order,
        timestamp: turn.timestamp,
    }
}

fn transcript_record_summary(record: ContextTranscriptRecord) -> SessionTranscriptRecord {
    SessionTranscriptRecord {
        id: record.id,
        source: record.source,
        kind: record.kind,
        role: record.role,
        summary: record.summary,
        raw: record.raw,
        byte_start: record.byte_start,
        byte_end: record.byte_end,
        timestamp: record.timestamp,
        truncated: record.truncated,
    }
}
