use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::api::service::{
    create_local_sessions_batch, list_sessions_for_client, request_plan_file, PlanFileServiceError,
};
pub use crate::api::service::{
    create_sessions_batch_result, new_batch_context, session_batch_membership,
    BATCH_CREATE_CONCURRENCY, BATCH_CREATE_MAX_DIRS,
};
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::operator_pressure::session_ready_for_operator_group_input;
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};
use crate::session::supervisor::TmuxAdoptError;
use crate::thought::context::{
    context_reader_for, AgentTranscriptRecord as ContextTranscriptRecord,
    AgentUserTurn as ContextUserTurn,
};
use crate::types::{
    AdoptSessionRequest, AdoptSessionResponse, AgentContextActionSummary, CreateSessionRequest,
    CreateSessionResponse, CreateSessionsBatchRequest, ErrorResponse, MermaidArtifactResponse,
    SessionAgentContextResponse, SessionAgentTurn, SessionGitDiffFileSummary,
    SessionGitDiffHunkSummary, SessionGitDiffResponse, SessionGroupInputRequest,
    SessionGroupInputResponse, SessionGroupInputResult, SessionInputRequest, SessionInputResponse,
    SessionListResponse, SessionPaneTailResponse, SessionState, SessionSummary,
    SessionTimelineEvent, SessionTimelinePinned, SessionTimelinePinnedItem,
    SessionTimelineResponse, SessionTranscriptRecord, SessionTranscriptResponse, TerminalSnapshot,
};

const PANE_TAIL_LINES: usize = 300;
const PANE_TAIL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const GIT_DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const GIT_DIFF_MAX_BYTES: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// GET /v1/sessions
// ---------------------------------------------------------------------------

async fn list_sessions(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let sessions = list_sessions_for_client(&state, true).await;
    // The version counter is not tracked by the supervisor itself; we use 0
    // as a placeholder. A proper monotonic version can be added to the
    // supervisor later if clients need ETag-style cache validation.
    Ok(Json(SessionListResponse {
        sessions,
        version: 0,
        repo_themes: Default::default(),
    }))
}

// ---------------------------------------------------------------------------
// POST /v1/sessions
// ---------------------------------------------------------------------------

async fn create_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return create_remote_session_response(body).await;
    }

    create_local_session_response(&state, body).await
}

async fn create_remote_session_response(body: CreateSessionRequest) -> axum::response::Response {
    match remote_sessions::create_remote_session(body).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn create_local_session_response(
    state: &Arc<AppState>,
    body: CreateSessionRequest,
) -> axum::response::Response {
    match state
        .supervisor
        .create_session(body.name, body.cwd, body.spawn_tool, body.initial_request)
        .await
    {
        Ok((session, repo_theme)) => (
            StatusCode::CREATED,
            Json(CreateSessionResponse {
                session,
                repo_theme,
            }),
        )
            .into_response(),
        Err(err) => create_local_session_error_response(err),
    }
}

fn create_local_session_error_response(error: anyhow::Error) -> axum::response::Response {
    let msg = error.to_string();
    // The supervisor returns anyhow errors. We detect specific failure modes by
    // inspecting the error message.
    if msg.contains("already exists") || msg.contains("duplicate session") {
        (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                code: "SESSION_ALREADY_EXISTS".to_string(),
                message: Some(msg),
            }),
        )
            .into_response()
    } else if msg.contains("cwd does not exist") {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                code: "VALIDATION_FAILED".to_string(),
                message: Some(msg),
            }),
        )
            .into_response()
    } else {
        tracing::error!("create_session failed: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some(msg),
            }),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/adopt
// ---------------------------------------------------------------------------

async fn adopt_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<AdoptSessionRequest>,
) -> axum::response::Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match state
        .supervisor
        .adopt_tmux_session(body.tmux_name, body.session_id)
        .await
    {
        Ok(adopted) => (
            StatusCode::CREATED,
            Json(AdoptSessionResponse {
                session: adopted.session,
                repo_theme: adopted.repo_theme,
                reused_session_id: adopted.reused_session_id,
            }),
        )
            .into_response(),
        Err(error) => adopt_session_error_response(error),
    }
}

fn adopt_session_error_response(error: TmuxAdoptError) -> axum::response::Response {
    let (status, code) = match &error {
        TmuxAdoptError::EmptyTmuxName => (StatusCode::BAD_REQUEST, "TMUX_NAME_REQUIRED"),
        TmuxAdoptError::DiscoveryUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "TMUX_DISCOVERY_UNAVAILABLE",
        ),
        TmuxAdoptError::TargetNotFound { .. } => (StatusCode::NOT_FOUND, "TMUX_SESSION_NOT_FOUND"),
        TmuxAdoptError::AmbiguousTarget { .. } => (StatusCode::CONFLICT, "TMUX_SESSION_AMBIGUOUS"),
        TmuxAdoptError::AlreadyTracked { .. } => {
            (StatusCode::CONFLICT, "TMUX_SESSION_ALREADY_TRACKED")
        }
        TmuxAdoptError::StaleSessionNotFound { .. } => {
            (StatusCode::NOT_FOUND, "STALE_SESSION_NOT_FOUND")
        }
        TmuxAdoptError::StaleSessionConflict { .. } => {
            (StatusCode::CONFLICT, "STALE_SESSION_CONFLICT")
        }
        TmuxAdoptError::SpawnFailed { .. } => {
            tracing::error!("adopt_session failed: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, "TMUX_ADOPT_FAILED")
        }
    };

    (
        status,
        Json(ErrorResponse {
            code: code.to_string(),
            message: Some(error.to_string()),
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/batch
// ---------------------------------------------------------------------------

async fn create_sessions_batch(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionsBatchRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return match remote_sessions::create_remote_sessions_batch(body).await {
            Ok(response) => {
                let status = if response.results.iter().all(|result| result.ok) {
                    StatusCode::CREATED
                } else {
                    StatusCode::MULTI_STATUS
                };
                (status, Json(response)).into_response()
            }
            Err(err) => err.into_response(),
        };
    }

    match create_local_sessions_batch(state, body.dirs, body.spawn_tool, body.initial_request).await
    {
        Ok(response) => {
            let status = if response.results.iter().all(|result| result.ok) {
                StatusCode::CREATED
            } else {
                StatusCode::MULTI_STATUS
            };
            (status, Json(response)).into_response()
        }
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

// ---------------------------------------------------------------------------
// DELETE /v1/sessions/{session_id}
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DeleteSessionQuery {
    mode: Option<String>,
}

async fn delete_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<DeleteSessionQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    let delete_mode = match query.mode.as_deref() {
        None | Some("detach_bridge") => crate::config::SessionDeleteMode::DetachBridge,
        Some("kill_tmux") => crate::config::SessionDeleteMode::KillTmux,
        Some(other) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    code: "VALIDATION_FAILED".to_string(),
                    message: Some(format!("invalid delete mode: {}", other)),
                }),
            )
                .into_response();
        }
    };

    match state
        .supervisor
        .delete_session(&session_id, delete_mode)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    }),
                )
                    .into_response()
            } else {
                tracing::error!("delete_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        code: "INTERNAL_ERROR".to_string(),
                        message: Some(msg),
                    }),
                )
                    .into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/attention/dismiss
// ---------------------------------------------------------------------------

async fn dismiss_attention(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::error!("[session {session_id}] dismiss_attention send failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some(e.to_string()),
            }),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/input
// ---------------------------------------------------------------------------

fn validation_error(message: impl Into<String>) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "VALIDATION_FAILED",
        Some(message.into()),
    )
}

fn error_response(
    status: StatusCode,
    code: impl Into<String>,
    message: Option<String>,
) -> Response {
    (
        status,
        Json(ErrorResponse {
            code: code.into(),
            message,
        }),
    )
        .into_response()
}

async fn send_input(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SessionInputRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if body.text.is_empty() {
        return validation_error("text must not be empty");
    }

    let handle = match writable_session_handle(&state, &session_id).await {
        Ok(handle) => handle,
        Err(response) => return response,
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    let command = session_input_command(body, ack_tx);

    if let Err(err) = handle.send(command).await {
        tracing::error!("[session {session_id}] send_input failed: {err}");
        return error_response(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            Some(err.to_string()),
        );
    }

    let delivery = match wait_for_input_delivery(ack_rx).await {
        Ok(delivery) => delivery,
        Err(response) => return response,
    };

    if !delivery.delivered {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "INPUT_DELIVERY_FAILED",
            delivery.message,
        );
    }

    (
        StatusCode::OK,
        Json(SessionInputResponse {
            ok: true,
            session_id,
            delivered: true,
            delivery_method: Some(delivery.method.to_string()),
            message: None,
        }),
    )
        .into_response()
}

async fn writable_session_handle(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<ActorHandle, Response> {
    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                "SESSION_NOT_FOUND",
                None,
            ))
        }
        Err(err) => {
            tracing::error!("send_input summary lookup failed: {err}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            ));
        }
    };

    if summary.state == SessionState::Exited {
        return Err(error_response(
            StatusCode::CONFLICT,
            "SESSION_EXITED",
            Some("session has already exited".to_string()),
        ));
    }

    state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None))
}

fn session_input_command(
    body: SessionInputRequest,
    ack: oneshot::Sender<InputDeliveryResult>,
) -> SessionCommand {
    if body.submit {
        SessionCommand::SubmitLineAck {
            text: body.text,
            ack,
        }
    } else {
        SessionCommand::WriteInputAck {
            data: body.text.into_bytes(),
            ack,
        }
    }
}

async fn wait_for_input_delivery(
    ack_rx: oneshot::Receiver<InputDeliveryResult>,
) -> Result<InputDeliveryResult, Response> {
    match tokio::time::timeout(std::time::Duration::from_secs(2), ack_rx).await {
        Ok(Ok(delivery)) => Ok(delivery),
        Ok(Err(_)) => Err(error_response(
            StatusCode::BAD_GATEWAY,
            "INPUT_DELIVERY_UNKNOWN",
            Some("session actor dropped input delivery ack".to_string()),
        )),
        Err(_) => Err(error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "INPUT_DELIVERY_TIMEOUT",
            Some("timed out waiting for input delivery confirmation".to_string()),
        )),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/agent-context
// ---------------------------------------------------------------------------

async fn get_agent_context(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_agent_context_response(&state, &session_id).await
}

async fn fetch_agent_context_response(state: &Arc<AppState>, session_id: &str) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::fetch_remote_agent_context(&target, remote_session_id)
                .await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("agent context summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response();
        }
    };

    match read_agent_context_for_summary(summary).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => {
            tracing::error!("agent context read failed: {err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response()
        }
    }
}

enum AgentContextReadResult {
    Unsupported,
    Missing,
    Snapshot {
        user_task: Option<String>,
        turns: Vec<SessionAgentTurn>,
        transcript_records: Vec<SessionTranscriptRecord>,
        source_size: u64,
        current_tool: Option<AgentContextActionSummary>,
        recent_actions: Vec<AgentContextActionSummary>,
        token_count: u64,
        context_limit: u64,
    },
}

async fn read_agent_context_for_summary(
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
    let read_result = tokio::task::spawn_blocking(move || {
        let Some(mut reader) = context_reader_for(&reader_tool, &reader_cwd, &[]) else {
            return AgentContextReadResult::Unsupported;
        };

        let Some(snapshot) = reader.read() else {
            return AgentContextReadResult::Missing;
        };

        AgentContextReadResult::Snapshot {
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
    })
    .await?;

    Ok(match read_result {
        AgentContextReadResult::Unsupported => agent_context_unavailable(
            session_id,
            tool,
            cwd,
            baseline_token_count,
            baseline_context_limit,
            format!("structured context is not supported for {tool_name}"),
        ),
        AgentContextReadResult::Missing => agent_context_unavailable(
            session_id,
            tool,
            cwd,
            baseline_token_count,
            baseline_context_limit,
            "no matching structured JSONL context was found",
        ),
        AgentContextReadResult::Snapshot {
            user_task,
            turns,
            transcript_records: _transcript_records,
            source_size: _source_size,
            current_tool,
            recent_actions,
            token_count,
            context_limit,
        } => SessionAgentContextResponse {
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

fn agent_action_summary(action: crate::thought::context::AgentAction) -> AgentContextActionSummary {
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

fn agent_context_unavailable(
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

fn context_limit_for_agent_context(tool: &Option<String>, context_limit: u64) -> u64 {
    if context_limit > 0 {
        context_limit
    } else {
        crate::types::context_limit_for_tool(tool.as_deref())
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/transcript
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TranscriptQuery {
    turn_id: Option<String>,
    after: Option<u64>,
    limit: Option<usize>,
}

async fn get_transcript(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<TranscriptQuery>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_transcript_response(&state, &session_id, query).await
}

async fn fetch_transcript_response(
    state: &Arc<AppState>,
    session_id: &str,
    query: TranscriptQuery,
) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::fetch_remote_transcript(
                &target,
                remote_session_id,
                query.turn_id.as_deref(),
                query.after,
                query.limit,
            )
            .await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("transcript summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response();
        }
    };

    match read_transcript_for_summary(summary, query).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => {
            tracing::error!("transcript read failed: {err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response()
        }
    }
}

async fn read_transcript_for_summary(
    summary: SessionSummary,
    query: TranscriptQuery,
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
    let read_result = tokio::task::spawn_blocking(move || {
        let Some(mut reader) = context_reader_for(&reader_tool, &reader_cwd, &[]) else {
            return AgentContextReadResult::Unsupported;
        };

        let Some(snapshot) = reader.read() else {
            return AgentContextReadResult::Missing;
        };

        AgentContextReadResult::Snapshot {
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
    })
    .await?;

    Ok(match read_result {
        AgentContextReadResult::Unsupported => transcript_unavailable(
            session_id,
            tool,
            cwd,
            format!("structured transcript is not supported for {tool_name}"),
        ),
        AgentContextReadResult::Missing => transcript_unavailable(
            session_id,
            tool,
            cwd,
            "no matching structured JSONL transcript was found",
        ),
        AgentContextReadResult::Snapshot {
            turns,
            transcript_records,
            source_size,
            ..
        } => build_transcript_response(
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

fn build_transcript_response(
    session_id: String,
    tool: Option<String>,
    cwd: String,
    turns: Vec<SessionAgentTurn>,
    transcript_records: Vec<SessionTranscriptRecord>,
    source_size: u64,
    query: TranscriptQuery,
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

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/timeline
// ---------------------------------------------------------------------------

async fn get_timeline(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_timeline_response(&state, &session_id).await
}

async fn fetch_timeline_response(state: &Arc<AppState>, session_id: &str) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::fetch_remote_timeline(&target, remote_session_id).await {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("timeline summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response();
        }
    };

    let response = build_timeline_response(state, summary).await;
    (StatusCode::OK, Json(response)).into_response()
}

async fn build_timeline_response(
    state: &Arc<AppState>,
    summary: SessionSummary,
) -> SessionTimelineResponse {
    let session_id = summary.session_id.clone();
    let cwd = summary.cwd.clone();
    let tool = summary.tool.clone();
    let context = read_agent_context_for_summary(summary.clone())
        .await
        .unwrap_or_else(|err| {
            tracing::warn!("timeline context read failed: {err}");
            agent_context_unavailable(
                session_id.clone(),
                tool.clone(),
                cwd.clone(),
                summary.token_count,
                context_limit_for_agent_context(&tool, summary.context_limit),
                "structured context could not be read",
            )
        });
    let git_diff = read_git_diff_for_summary(summary.clone()).await;
    let pane_tail = request_pane_tail(state, &session_id).await;
    let artifact = fetch_mermaid_artifact_response(state, &session_id)
        .await
        .ok();

    let mut builder = TimelineBuilder::default();
    let mut pinned = SessionTimelinePinned::default();

    append_context_events(&mut builder, &mut pinned, &context);
    append_git_diff_event(&mut builder, &mut pinned, &git_diff);
    append_pane_tail_event(&mut builder, &mut pinned, pane_tail);
    append_artifact_event(&mut builder, &mut pinned, artifact.as_ref());

    SessionTimelineResponse {
        session_id,
        available: true,
        cwd,
        tool,
        events: builder.events,
        pinned,
        message: None,
    }
}

#[derive(Default)]
struct TimelineBuilder {
    next_order: u64,
    events: Vec<SessionTimelineEvent>,
}

impl TimelineBuilder {
    fn push(
        &mut self,
        id: impl Into<String>,
        kind: impl Into<String>,
        source: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: Option<String>,
    ) -> String {
        self.next_order += 1;
        let id = id.into();
        self.events.push(SessionTimelineEvent {
            id: id.clone(),
            kind: kind.into(),
            source: source.into(),
            title: title.into(),
            summary: summary.into(),
            timestamp: None,
            order: Some(self.next_order),
            detail,
        });
        id
    }
}

fn pinned_item(
    title: impl Into<String>,
    summary: impl Into<String>,
    source: impl Into<String>,
    event_id: impl Into<String>,
) -> SessionTimelinePinnedItem {
    SessionTimelinePinnedItem {
        title: title.into(),
        summary: summary.into(),
        source: source.into(),
        event_id: Some(event_id.into()),
    }
}

fn timeline_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.replace('\r', "").trim().to_string();
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut excerpt = normalized.chars().take(max_chars).collect::<String>();
    excerpt.push_str("...");
    excerpt
}

fn append_context_events(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    context: &SessionAgentContextResponse,
) {
    if let Some(task) = context
        .user_task
        .as_deref()
        .filter(|task| !task.trim().is_empty())
    {
        let summary = timeline_excerpt(task, 180);
        let event_id = builder.push(
            "task",
            "task",
            "agent-context",
            "Task",
            summary.clone(),
            Some(task.to_string()),
        );
        pinned.task = Some(pinned_item("Task", summary, "agent-context", event_id));
    }

    if let Some(action) = context.current_tool.as_ref() {
        let summary = action
            .detail
            .as_deref()
            .filter(|detail| !detail.trim().is_empty())
            .unwrap_or(&action.tool);
        let summary = timeline_excerpt(summary, 180);
        let event_id = builder.push(
            "current-action",
            "tool_call",
            "agent-context",
            action.tool.clone(),
            summary.clone(),
            action.detail.clone(),
        );
        pinned.current_action = Some(pinned_item(
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
            timeline_excerpt(summary, 180),
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
            timeline_excerpt(message, 180),
            None,
        );
    }
}

fn append_git_diff_event(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    git_diff: &SessionGitDiffResponse,
) {
    let summary = git_diff_timeline_summary(git_diff);
    let detail = git_diff_timeline_detail(git_diff);
    let event_id = builder.push(
        "git-diff",
        "diff",
        "git-diff",
        "Diffs",
        timeline_excerpt(&summary, 180),
        detail,
    );
    pinned.diff = Some(pinned_item("Diffs", summary, "git-diff", event_id));
}

fn git_diff_timeline_summary(git_diff: &SessionGitDiffResponse) -> String {
    if !git_diff.available {
        return git_diff
            .message
            .clone()
            .unwrap_or_else(|| "git diff unavailable".to_string());
    }

    if git_diff_has_no_changes(git_diff) {
        return "clean".to_string();
    }

    if git_diff.truncated {
        "dirty, truncated".to_string()
    } else {
        "dirty".to_string()
    }
}

fn git_diff_has_no_changes(git_diff: &SessionGitDiffResponse) -> bool {
    git_diff.status_short.trim().is_empty()
        && git_diff.unstaged_diff.trim().is_empty()
        && git_diff.staged_diff.trim().is_empty()
}

fn git_diff_timeline_detail(git_diff: &SessionGitDiffResponse) -> Option<String> {
    let detail = [
        git_diff.status_short.as_str(),
        git_diff.staged_diff.as_str(),
        git_diff.unstaged_diff.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n");

    (!detail.is_empty()).then(|| timeline_excerpt(&detail, 1200))
}

fn append_pane_tail_event(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    pane_tail: Result<String, PaneTailError>,
) {
    let (summary, detail) = match pane_tail {
        Ok(text) => {
            let line_count = text.trim_end().lines().count();
            let summary = if line_count == 0 {
                "empty".to_string()
            } else {
                format!("{line_count} lines")
            };
            (
                summary,
                (!text.trim().is_empty()).then(|| timeline_excerpt(&text, 1200)),
            )
        }
        Err(err) => (err.message().to_string(), None),
    };
    let event_id = builder.push(
        "pane-tail",
        "pane_tail",
        "pane-tail",
        "Recent output",
        summary.clone(),
        detail,
    );
    pinned.pane_tail = Some(pinned_item("Recent output", summary, "pane-tail", event_id));
}

fn append_artifact_event(
    builder: &mut TimelineBuilder,
    pinned: &mut SessionTimelinePinned,
    artifact: Option<&MermaidArtifactResponse>,
) {
    let (summary, detail) = match artifact {
        Some(artifact) if artifact.available => {
            let plan_count = artifact.plan_files.as_ref().map_or(0, Vec::len);
            let summary = if plan_count > 0 {
                format!("{plan_count} plan files")
            } else {
                artifact
                    .path
                    .clone()
                    .unwrap_or_else(|| "artifact available".to_string())
            };
            (summary, artifact.source.clone())
        }
        Some(artifact) => (
            artifact
                .error
                .clone()
                .unwrap_or_else(|| "artifact unavailable".to_string()),
            None,
        ),
        None => ("artifact unavailable".to_string(), None),
    };
    let event_id = builder.push(
        "artifact",
        "artifact",
        "mermaid-artifact",
        "Artifacts",
        timeline_excerpt(&summary, 180),
        detail.map(|detail| timeline_excerpt(&detail, 1200)),
    );
    pinned.artifact = Some(pinned_item(
        "Artifacts",
        summary,
        "mermaid-artifact",
        event_id,
    ));
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/git-diff
// ---------------------------------------------------------------------------

async fn get_git_diff(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_git_diff_response(&state, &session_id).await
}

async fn fetch_git_diff_response(state: &Arc<AppState>, session_id: &str) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::fetch_remote_git_diff(&target, remote_session_id).await {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("git diff summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(err.to_string()),
                }),
            )
                .into_response();
        }
    };

    let response = read_git_diff_for_summary(summary).await;
    (StatusCode::OK, Json(response)).into_response()
}

async fn read_git_diff_for_summary(summary: SessionSummary) -> SessionGitDiffResponse {
    let repo_root = match run_git_capture(&summary.cwd, &["rev-parse", "--show-toplevel"]).await {
        Ok(root) => root.trim().to_string(),
        Err(message) => {
            return git_diff_unavailable(
                summary.session_id,
                summary.cwd,
                format!("git repo root unavailable: {message}"),
            );
        }
    };

    if repo_root.is_empty() {
        return git_diff_unavailable(
            summary.session_id,
            summary.cwd,
            "git repo root unavailable: empty git output",
        );
    }

    let status_short = match run_git_capture(&repo_root, &["status", "--short"]).await {
        Ok(output) => output,
        Err(message) => {
            return git_diff_unavailable(
                summary.session_id,
                summary.cwd,
                format!("git status unavailable: {message}"),
            );
        }
    };
    let unstaged_raw =
        match run_git_capture(&repo_root, &["diff", "--no-ext-diff", "--no-color"]).await {
            Ok(output) => output,
            Err(message) => {
                return git_diff_unavailable(
                    summary.session_id,
                    summary.cwd,
                    format!("git diff unavailable: {message}"),
                );
            }
        };
    let staged_raw = match run_git_capture(
        &repo_root,
        &["diff", "--cached", "--no-ext-diff", "--no-color"],
    )
    .await
    {
        Ok(output) => output,
        Err(message) => {
            return git_diff_unavailable(
                summary.session_id,
                summary.cwd,
                format!("git diff --cached unavailable: {message}"),
            );
        }
    };

    let (unstaged_diff, unstaged_truncated) = truncate_git_output(unstaged_raw);
    let (staged_diff, staged_truncated) = truncate_git_output(staged_raw);
    let files = summarize_git_diff_files(
        &staged_diff,
        staged_truncated,
        &unstaged_diff,
        unstaged_truncated,
    );
    SessionGitDiffResponse {
        session_id: summary.session_id,
        available: true,
        cwd: summary.cwd,
        repo_root: Some(repo_root),
        status_short,
        unstaged_diff,
        staged_diff,
        truncated: unstaged_truncated || staged_truncated,
        message: None,
        files,
    }
}

async fn run_git_capture(cwd: &str, args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        GIT_DIFF_TIMEOUT,
        Command::new("git").arg("-C").arg(cwd).args(args).output(),
    )
    .await
    .map_err(|_| format!("git {} timed out", args.join(" ")))?
    .map_err(|err| err.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(format!(
                "git {} exited with {}",
                args.join(" "),
                output.status
            ));
        }
        return Err(stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn truncate_git_output(output: String) -> (String, bool) {
    if output.len() <= GIT_DIFF_MAX_BYTES {
        return (output, false);
    }

    let mut end = GIT_DIFF_MAX_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    (output[..end].to_string(), true)
}

fn summarize_git_diff_files(
    staged_diff: &str,
    staged_truncated: bool,
    unstaged_diff: &str,
    unstaged_truncated: bool,
) -> Vec<SessionGitDiffFileSummary> {
    // Stamp each source with its own truncation flag: a staged file's summary
    // must not be marked truncated just because the unstaged diff overflowed.
    let mut files = Vec::new();
    files.extend(parse_git_diff_file_summaries(
        "staged",
        staged_diff,
        staged_truncated,
    ));
    files.extend(parse_git_diff_file_summaries(
        "unstaged",
        unstaged_diff,
        unstaged_truncated,
    ));
    files
}

fn parse_git_diff_file_summaries(
    source: &str,
    diff_text: &str,
    truncated: bool,
) -> Vec<SessionGitDiffFileSummary> {
    let mut files = Vec::new();
    let mut current: Option<SessionGitDiffFileSummary> = None;
    let mut current_hunk: Option<SessionGitDiffHunkSummary> = None;

    for line in diff_text.lines() {
        if line.starts_with("diff --git ") {
            push_diff_hunk(&mut current, &mut current_hunk);
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(SessionGitDiffFileSummary {
                path: parse_diff_git_path(line).unwrap_or_else(|| "unknown".to_string()),
                old_path: None,
                source: source.to_string(),
                change: "modified".to_string(),
                added_lines: 0,
                removed_lines: 0,
                truncated,
                hunks: Vec::new(),
            });
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue;
        };

        if line.starts_with("new file mode ") {
            file.change = "added".to_string();
            continue;
        }
        if line.starts_with("deleted file mode ") {
            file.change = "deleted".to_string();
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            file.old_path = Some(path.to_string());
            file.change = "renamed".to_string();
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            file.path = path.to_string();
            file.change = "renamed".to_string();
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            if let Some(path) = normalize_diff_path(path) {
                file.path = path;
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            if let Some(path) = normalize_diff_path(path) {
                file.old_path = Some(path);
            }
            continue;
        }
        if line.starts_with("@@") {
            push_diff_hunk(&mut current, &mut current_hunk);
            current_hunk = Some(SessionGitDiffHunkSummary {
                header: line.to_string(),
                added_lines: 0,
                removed_lines: 0,
            });
            continue;
        }

        if line.starts_with('+') && !line.starts_with("+++") {
            file.added_lines += 1;
            if let Some(hunk) = current_hunk.as_mut() {
                hunk.added_lines += 1;
            }
        } else if line.starts_with('-') && !line.starts_with("---") {
            file.removed_lines += 1;
            if let Some(hunk) = current_hunk.as_mut() {
                hunk.removed_lines += 1;
            }
        }
    }

    push_diff_hunk(&mut current, &mut current_hunk);
    if let Some(file) = current {
        files.push(file);
    }
    files
}

fn push_diff_hunk(
    current: &mut Option<SessionGitDiffFileSummary>,
    current_hunk: &mut Option<SessionGitDiffHunkSummary>,
) {
    if let (Some(file), Some(hunk)) = (current.as_mut(), current_hunk.take()) {
        file.hunks.push(hunk);
    }
}

fn parse_diff_git_path(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let _diff = parts.next()?;
    let _git = parts.next()?;
    let _old = parts.next()?;
    let new = parts.next()?;
    normalize_diff_path(new)
}

fn normalize_diff_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed == "/dev/null" {
        return None;
    }
    Some(
        trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed)
            .to_string(),
    )
}

fn git_diff_unavailable(
    session_id: String,
    cwd: String,
    message: impl Into<String>,
) -> SessionGitDiffResponse {
    SessionGitDiffResponse {
        session_id,
        available: false,
        cwd,
        repo_root: None,
        status_short: String::new(),
        unstaged_diff: String::new(),
        staged_diff: String::new(),
        truncated: false,
        message: Some(message.into()),
        files: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/group-input
// ---------------------------------------------------------------------------

fn group_input_error_result(
    session_id: String,
    code: impl Into<String>,
    message: Option<String>,
) -> SessionGroupInputResult {
    SessionGroupInputResult {
        session_id,
        ok: false,
        error: Some(ErrorResponse {
            code: code.into(),
            message,
        }),
    }
}

fn session_ready_for_group_input(summary: &SessionSummary) -> bool {
    session_ready_for_operator_group_input(summary)
}

fn group_input_batch_scope_error(summaries: &[SessionSummary]) -> Option<(&'static str, String)> {
    if summaries.iter().any(|summary| summary.batch.is_none()) {
        return Some((
            "SESSION_NOT_IN_BATCH",
            "session is not part of a batch".to_string(),
        ));
    }

    let batch_ids = summaries
        .iter()
        .filter_map(|summary| summary.batch.as_ref().map(|batch| batch.id.as_str()))
        .collect::<HashSet<_>>();
    if batch_ids.len() > 1 {
        return Some((
            "SESSION_BATCH_MISMATCH",
            "sessions are not in the same batch".to_string(),
        ));
    }

    None
}

fn group_input_bytes(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.extend_from_slice(b"\r\r");
    bytes
}

async fn send_group_input_to_session(
    state: &Arc<AppState>,
    session_id: String,
    summary: Option<SessionSummary>,
    input: &[u8],
) -> SessionGroupInputResult {
    let Some(summary) = summary else {
        return group_input_error_result(session_id, "SESSION_NOT_FOUND", None);
    };

    if summary.state == SessionState::Exited {
        return group_input_error_result(
            session_id,
            "SESSION_EXITED",
            Some("session has already exited".to_string()),
        );
    }

    if !session_ready_for_group_input(&summary) {
        return group_input_error_result(
            session_id,
            "SESSION_NOT_READY",
            Some("session is not waiting for input".to_string()),
        );
    }

    let handle = match state.supervisor.get_session(&session_id).await {
        Some(handle) => handle,
        None => return group_input_error_result(session_id, "SESSION_NOT_FOUND", None),
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    if let Err(err) = handle
        .send(SessionCommand::WriteInputAck {
            data: input.to_vec(),
            ack: ack_tx,
        })
        .await
    {
        tracing::error!("[session {session_id}] send_group_input failed: {err}");
        return group_input_error_result(session_id, "SESSION_NOT_FOUND", Some(err.to_string()));
    }

    match tokio::time::timeout(std::time::Duration::from_secs(2), ack_rx).await {
        Ok(Ok(delivery)) if delivery.delivered => {}
        Ok(Ok(delivery)) => {
            return group_input_error_result(session_id, "INPUT_DELIVERY_FAILED", delivery.message);
        }
        Ok(Err(_)) => {
            return group_input_error_result(
                session_id,
                "INPUT_DELIVERY_UNKNOWN",
                Some("session actor dropped input delivery ack".to_string()),
            );
        }
        Err(_) => {
            return group_input_error_result(
                session_id,
                "INPUT_DELIVERY_TIMEOUT",
                Some("timed out waiting for input delivery confirmation".to_string()),
            );
        }
    }

    SessionGroupInputResult {
        session_id,
        ok: true,
        error: None,
    }
}

pub async fn send_group_input_service(
    state: Arc<AppState>,
    body: SessionGroupInputRequest,
) -> Result<SessionGroupInputResponse, ErrorResponse> {
    if body.session_ids.is_empty() {
        return Err(ErrorResponse {
            code: "VALIDATION_FAILED".to_string(),
            message: Some("session_ids must not be empty".to_string()),
        });
    }
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return Err(ErrorResponse {
            code: "VALIDATION_FAILED".to_string(),
            message: Some("text must not be empty".to_string()),
        });
    }

    let mut seen = HashSet::new();
    let session_ids = body
        .session_ids
        .into_iter()
        .filter(|session_id| seen.insert(session_id.clone()))
        .collect::<Vec<_>>();
    if session_ids.len() < 2 {
        return Err(ErrorResponse {
            code: "VALIDATION_FAILED".to_string(),
            message: Some("session_ids must include at least two unique sessions".to_string()),
        });
    }
    let summaries = state
        .supervisor
        .list_sessions()
        .await
        .into_iter()
        .map(|summary| (summary.session_id.clone(), summary))
        .collect::<HashMap<_, _>>();
    let found_summaries = session_ids
        .iter()
        .filter_map(|session_id| summaries.get(session_id).cloned())
        .collect::<Vec<_>>();
    if let Some((code, message)) = group_input_batch_scope_error(&found_summaries) {
        let results = session_ids
            .into_iter()
            .map(|session_id| {
                if summaries.contains_key(&session_id) {
                    group_input_error_result(session_id, code, Some(message.clone()))
                } else {
                    group_input_error_result(session_id, "SESSION_NOT_FOUND", None)
                }
            })
            .collect::<Vec<_>>();
        return Ok(SessionGroupInputResponse::from_results(results));
    }
    let input = group_input_bytes(&text);
    let results = futures::future::join_all(session_ids.into_iter().map(|session_id| {
        let summary = summaries.get(&session_id).cloned();
        send_group_input_to_session(&state, session_id, summary, &input)
    }))
    .await;
    Ok(SessionGroupInputResponse::from_results(results))
}

async fn send_group_input(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionGroupInputRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match send_group_input_service(state, body).await {
        Ok(response) => {
            let status = if response.skipped == 0 {
                StatusCode::OK
            } else {
                StatusCode::MULTI_STATUS
            };
            (status, Json(response)).into_response()
        }
        Err(error) => (StatusCode::BAD_REQUEST, Json(error)).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/snapshot
// ---------------------------------------------------------------------------

async fn get_snapshot(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<TerminalSnapshot>();
    if handle.send(SessionCommand::GetSnapshot(tx)).await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("session actor unavailable".to_string()),
            }),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(snapshot)) => (StatusCode::OK, Json(snapshot)).into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("actor dropped snapshot reply".to_string()),
            }),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("snapshot request timed out".to_string()),
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/pane-tail
// ---------------------------------------------------------------------------

async fn get_pane_tail(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match request_pane_tail(&state, &session_id).await {
        Ok(text) => (
            StatusCode::OK,
            Json(SessionPaneTailResponse { session_id, text }),
        )
            .into_response(),
        Err(error) => pane_tail_error_response(error),
    }
}

enum PaneTailError {
    SessionNotFound,
    ActorUnavailable,
    ReplyDropped,
    TimedOut,
}

impl PaneTailError {
    fn message(&self) -> &'static str {
        match self {
            PaneTailError::SessionNotFound => "session not found",
            PaneTailError::ActorUnavailable => "session actor unavailable",
            PaneTailError::ReplyDropped => "actor dropped pane tail reply",
            PaneTailError::TimedOut => "pane tail request timed out",
        }
    }
}

async fn request_pane_tail(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<String, PaneTailError> {
    let handle = state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or(PaneTailError::SessionNotFound)?;
    request_pane_tail_from_actor(&handle).await
}

async fn request_pane_tail_from_actor(handle: &ActorHandle) -> Result<String, PaneTailError> {
    let (tx, rx) = oneshot::channel::<String>();
    if handle
        .send(SessionCommand::GetPaneTail {
            lines: PANE_TAIL_LINES,
            reply: tx,
        })
        .await
        .is_err()
    {
        return Err(PaneTailError::ActorUnavailable);
    }

    match tokio::time::timeout(PANE_TAIL_TIMEOUT, rx).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(_)) => Err(PaneTailError::ReplyDropped),
        Err(_) => Err(PaneTailError::TimedOut),
    }
}

fn pane_tail_error_response(error: PaneTailError) -> Response {
    match error {
        PaneTailError::SessionNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: "SESSION_NOT_FOUND".to_string(),
                message: None,
            }),
        )
            .into_response(),
        PaneTailError::ActorUnavailable => pane_tail_internal_error("session actor unavailable"),
        PaneTailError::ReplyDropped => pane_tail_internal_error("actor dropped pane tail reply"),
        PaneTailError::TimedOut => pane_tail_internal_error("pane tail request timed out"),
    }
}

fn pane_tail_internal_error(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            code: "INTERNAL_ERROR".to_string(),
            message: Some(message.to_string()),
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/mermaid-artifact
// ---------------------------------------------------------------------------

async fn get_mermaid_artifact(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match fetch_mermaid_artifact_response(&state, &session_id).await {
        Ok(artifact) => (StatusCode::OK, Json(artifact)).into_response(),
        Err(resp) => resp,
    }
}

pub(crate) async fn fetch_mermaid_artifact_response(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<MermaidArtifactResponse, axum::response::Response> {
    if let Some((target, remote_session_id)) =
        remote_sessions::denamespace_for_target(session_id).map_err(|err| err.into_response())?
    {
        return remote_sessions::fetch_remote_mermaid_artifact(&target, remote_session_id)
            .await
            .map_err(|err| err.into_response());
    }

    let handle = match state.supervisor.get_session(session_id).await {
        Some(h) => h,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    code: "SESSION_NOT_FOUND".to_string(),
                    message: None,
                }),
            )
                .into_response());
        }
    };

    let (tx, rx) = oneshot::channel::<MermaidArtifactResponse>();
    if handle
        .send(SessionCommand::GetMermaidArtifact(tx))
        .await
        .is_err()
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("session actor unavailable".to_string()),
            }),
        )
            .into_response());
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(artifact)) => Ok(artifact),
        Ok(Err(_)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("actor dropped mermaid artifact reply".to_string()),
            }),
        )
            .into_response()),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                code: "INTERNAL_ERROR".to_string(),
                message: Some("mermaid artifact request timed out".to_string()),
            }),
        )
            .into_response()),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/plan-file?name=plan.md
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PlanFileQuery {
    name: String,
}

async fn get_plan_file(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<PlanFileQuery>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_plan_file_response(&state, &session_id, &query.name).await
}

async fn fetch_plan_file_response(state: &Arc<AppState>, session_id: &str, name: &str) -> Response {
    match request_plan_file(state, session_id, name).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => plan_file_error_response(error),
    }
}

fn plan_file_error_response(error: PlanFileServiceError) -> Response {
    match error {
        PlanFileServiceError::Remote(err) => err.into_response(),
        PlanFileServiceError::SessionNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: "SESSION_NOT_FOUND".to_string(),
                message: None,
            }),
        )
            .into_response(),
        PlanFileServiceError::ActorUnavailable => {
            plan_file_internal_error("session actor unavailable")
        }
        PlanFileServiceError::ReplyDropped => {
            plan_file_internal_error("actor dropped plan file reply")
        }
        PlanFileServiceError::TimedOut => plan_file_internal_error("plan file request timed out"),
    }
}

fn plan_file_internal_error(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            code: "INTERNAL_ERROR".to_string(),
            message: Some(message.to_string()),
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/sessions", get(list_sessions).post(create_session))
        .route("/v1/sessions/adopt", post(adopt_session))
        .route("/v1/sessions/reattach", post(adopt_session))
        .route("/v1/sessions/batch", post(create_sessions_batch))
        .route("/v1/sessions/group-input", post(send_group_input))
        .route("/v1/sessions/{session_id}", delete(delete_session))
        .route(
            "/v1/sessions/{session_id}/attention/dismiss",
            post(dismiss_attention),
        )
        .route("/v1/sessions/{session_id}/input", post(send_input))
        .route(
            "/v1/sessions/{session_id}/agent-context",
            get(get_agent_context),
        )
        .route("/v1/sessions/{session_id}/transcript", get(get_transcript))
        .route("/v1/sessions/{session_id}/timeline", get(get_timeline))
        .route("/v1/sessions/{session_id}/git-diff", get(get_git_diff))
        .route("/v1/sessions/{session_id}/snapshot", get(get_snapshot))
        .route("/v1/sessions/{session_id}/pane-tail", get(get_pane_tail))
        .route(
            "/v1/sessions/{session_id}/mermaid-artifact",
            get(get_mermaid_artifact),
        )
        .route("/v1/sessions/{session_id}/plan-file", get(get_plan_file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::{OBSERVER_SCOPES, OPERATOR_SCOPES};
    use crate::config::Config;
    use crate::session::actor::ActorHandle;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::{SyncRequestSequence, ThoughtDeliveryState};
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{
        PlanFileResponse, RestState, StateEvidence, ThoughtSource, ThoughtState, TransportHealth,
    };
    use axum::body::to_bytes;
    use axum::extract::{Json, Path, Query, State};
    use axum::response::IntoResponse;
    use chrono::Utc;
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path as FsPath;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tempfile::{tempdir, TempDir};
    use tokio::sync::{mpsc, RwLock};

    fn p95_duration(mut samples: Vec<Duration>) -> Duration {
        assert!(!samples.is_empty(), "p95 requires at least one sample");
        samples.sort_unstable();
        let index = samples
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1);
        samples[index]
    }

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
                std::time::Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    fn summary(session_id: &str, state: SessionState) -> crate::types::SessionSummary {
        let state_evidence = match state {
            SessionState::Busy => StateEvidence::new("osc133_command"),
            SessionState::Exited => StateEvidence::new("process_exit"),
            _ => StateEvidence::new("osc133_prompt"),
        };
        crate::types::SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: format!("tmux-{session_id}"),
            state,
            current_command: None,
            state_evidence,
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: crate::types::fallback_rest_state(state, ThoughtState::Holding),
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
        }
    }

    fn with_test_batch(mut summary: SessionSummary, batch_id: &str) -> SessionSummary {
        summary.batch = Some(session_batch_membership(
            batch_id.to_string(),
            "test batch".to_string(),
            0,
            2,
            Utc::now(),
            Some("continue".to_string()),
        ));
        summary
    }

    async fn insert_summary_test_handle(
        state: &Arc<AppState>,
        summary: SessionSummary,
    ) -> mpsc::Receiver<Vec<u8>> {
        let session_id = summary.session_id.clone();
        let tmux_name = summary.tmux_name.clone();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let (write_tx, write_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::WriteInput(bytes) => {
                        let _ = write_tx.send(bytes).await;
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = write_tx.send(data).await;
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                    }
                    _ => {}
                }
            }
        });
        write_rx
    }

    async fn insert_timeline_test_handle(
        state: &Arc<AppState>,
        summary: SessionSummary,
        pane_tail: String,
        artifact: MermaidArtifactResponse,
    ) {
        let session_id = summary.session_id.clone();
        let tmux_name = summary.tmux_name.clone();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::GetPaneTail { lines, reply } => {
                        assert_eq!(lines, PANE_TAIL_LINES);
                        let _ = reply.send(pane_tail.clone());
                    }
                    SessionCommand::GetMermaidArtifact(reply) => {
                        let _ = reply.send(artifact.clone());
                    }
                    _ => {}
                }
            }
        });
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    struct TestPathGuard(Option<OsString>);

    impl Drop for TestPathGuard {
        fn drop(&mut self) {
            if let Some(value) = self.0.take() {
                std::env::set_var("PATH", value);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }

    struct TestEnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl TestEnvVarGuard {
        fn set_path(key: &'static str, value: &FsPath) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for TestEnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_executable(path: &FsPath, contents: &str) {
        std::fs::write(path, contents).expect("write executable");
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    fn prepend_test_path(bin_dir: &FsPath, original_path: Option<&OsStr>) {
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = original_path {
            entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
        }
        std::env::set_var("PATH", std::env::join_paths(entries).expect("path"));
    }

    fn install_fake_tmux(script: &str) -> (TempDir, TestPathGuard) {
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        write_executable(&bin_dir.join("tmux"), script);
        let original_path = std::env::var_os("PATH");
        prepend_test_path(&bin_dir, original_path.as_deref());
        (dir, TestPathGuard(original_path))
    }

    const FAKE_TMUX_FOR_CREATE: &str = r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  new-session|attach-session)
    while IFS= read -r line; do
      printf '%s\r\n' "$line"
    done
    ;;
  send-keys|kill-session)
    exit 0
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_path}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_CWD:-/tmp/project}" ;;
      "#{pane_current_command}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_COMMAND:-zsh}" ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
  capture-pane)
    printf 'captured pane\n'
    ;;
  list-sessions)
    exit 0
    ;;
esac
"##;

    fn generated_dir_name_sets() -> Vec<Vec<String>> {
        let mut runner = TestRunner::deterministic();
        let name = proptest::string::string_regex("[a-z]{1,8}").expect("valid regex");
        let strategy = proptest::collection::btree_set(name, 1..=4);
        (0..4)
            .map(|_| {
                strategy
                    .new_tree(&mut runner)
                    .expect("generate dir names")
                    .current()
                    .into_iter()
                    .collect()
            })
            .collect()
    }

    fn create_case_dirs(root: &FsPath, case_index: usize, names: &[String]) -> Vec<String> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let path = root.join(format!("case-{case_index}-{index}-{name}"));
                std::fs::create_dir_all(&path).expect("create test cwd");
                path.to_string_lossy().into_owned()
            })
            .collect()
    }

    async fn create_batch(state: Arc<AppState>, dirs: Vec<String>) -> axum::response::Response {
        create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(CreateSessionsBatchRequest {
                dirs,
                spawn_tool: None,
                launch_target: None,
                initial_request: None,
            }),
        )
        .await
        .into_response()
    }

    async fn cleanup_created_sessions(state: &Arc<AppState>, json: &Value) {
        let Some(results) = json["results"].as_array() else {
            return;
        };
        for result in results {
            let Some(session_id) = result["session"]["session_id"].as_str() else {
                continue;
            };
            let _ = state
                .supervisor
                .delete_session(session_id, crate::config::SessionDeleteMode::DetachBridge)
                .await;
        }
    }

    fn cwd_result_classes(json: &Value) -> BTreeMap<String, bool> {
        json["results"]
            .as_array()
            .expect("results array")
            .iter()
            .map(|result| {
                (
                    result["cwd"].as_str().expect("cwd").to_string(),
                    result["ok"].as_bool().expect("ok"),
                )
            })
            .collect()
    }

    fn success_count(json: &Value) -> usize {
        json["results"]
            .as_array()
            .expect("results array")
            .iter()
            .filter(|result| result["ok"].as_bool() == Some(true))
            .count()
    }

    async fn spawn_summary_handle(summary: crate::types::SessionSummary) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(
            summary.session_id.clone(),
            summary.tmux_name.clone(),
            cmd_tx,
        );
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    #[tokio::test]
    async fn create_session_requires_write_scope() {
        let response = create_session(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionRequest {
                name: None,
                cwd: None,
                spawn_tool: None,
                launch_target: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn adopt_session_requires_write_scope() {
        let response = adopt_session(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(AdoptSessionRequest {
                tmux_name: "alpha".to_string(),
                session_id: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn adopt_session_rejects_already_tracked_tmux_without_duplication() {
        let state = test_state();
        let active = summary("sess-1", SessionState::Idle);
        let tmux_name = active.tmux_name.clone();
        let _rx = insert_summary_test_handle(&state, active.clone()).await;

        let response = adopt_session(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(AdoptSessionRequest {
                tmux_name,
                session_id: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = response_json(response).await;
        assert_eq!(json["code"], "TMUX_SESSION_ALREADY_TRACKED");
        assert!(json["message"]
            .as_str()
            .expect("message")
            .contains("sess-1"));
    }

    #[tokio::test]
    async fn create_session_rejects_unknown_non_local_launch_target_explicitly() {
        let response = create_session(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionRequest {
                name: None,
                // Remote launch now requires an explicit cwd; supply the current
                // dir (what launch_cwd used to inject implicitly) so this test
                // still reaches the unknown-launch-target check rather than the
                // missing-cwd validation that would otherwise preempt it.
                cwd: Some(
                    std::env::current_dir()
                        .expect("current dir")
                        .to_string_lossy()
                        .into_owned(),
                ),
                spawn_tool: None,
                launch_target: Some("not-configured-target-for-test".to_string()),
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
        assert!(json["message"]
            .as_str()
            .expect("message")
            .contains("launch target 'not-configured-target-for-test' is not configured"));
    }

    #[tokio::test]
    async fn create_session_rejects_missing_cwd_as_validation_error() {
        let missing = tempdir().expect("tempdir").path().join("missing");
        let response = create_session(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionRequest {
                name: None,
                cwd: Some(missing.to_string_lossy().into_owned()),
                spawn_tool: None,
                launch_target: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
        assert!(json["message"]
            .as_str()
            .expect("message")
            .contains("cwd does not exist"));
    }

    #[tokio::test]
    async fn create_sessions_batch_requires_write_scope() {
        let response = create_sessions_batch(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionsBatchRequest {
                dirs: vec!["/tmp/project".to_string()],
                spawn_tool: None,
                launch_target: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn create_sessions_batch_rejects_empty_dirs() {
        let response = create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionsBatchRequest {
                dirs: Vec::new(),
                spawn_tool: None,
                launch_target: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
        assert_eq!(json["message"], "dirs must not be empty");
    }

    #[tokio::test]
    async fn create_sessions_batch_rejects_oversized_batches() {
        let response = create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(CreateSessionsBatchRequest {
                dirs: (0..=BATCH_CREATE_MAX_DIRS)
                    .map(|idx| format!("/tmp/project-{idx}"))
                    .collect(),
                spawn_tool: None,
                launch_target: None,
                initial_request: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
        assert_eq!(
            json["message"],
            format!("dirs must include at most {BATCH_CREATE_MAX_DIRS} entries")
        );
    }

    #[tokio::test]
    async fn create_sessions_batch_assigns_shared_batch_metadata() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");
        let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);

        let response = create_sessions_batch(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Json(CreateSessionsBatchRequest {
                dirs,
                spawn_tool: None,
                launch_target: None,
                initial_request: Some("wire jwt refresh + tests".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::CREATED);
        let json = response_json(response).await;
        let results = json["results"].as_array().expect("results");
        let first_batch = &results[0]["session"]["batch"];
        let second_batch = &results[1]["session"]["batch"];

        assert!(first_batch["id"]
            .as_str()
            .expect("batch id")
            .starts_with("batch-"));
        assert_eq!(second_batch["id"], first_batch["id"]);
        assert_eq!(first_batch["label"], "wire jwt refresh + tests");
        assert_eq!(first_batch["prompt_excerpt"], "wire jwt refresh + tests");
        assert_eq!(first_batch["index"], 0);
        assert_eq!(second_batch["index"], 1);
        assert_eq!(first_batch["total"], 2);
        assert_eq!(second_batch["total"], 2);
        assert!(first_batch["created_at"].is_string());

        cleanup_created_sessions(&state, &json).await;
    }

    #[tokio::test]
    async fn create_sessions_batch_mr_permutation_preserves_cwd_result_classes() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");

        for (case_index, names) in generated_dir_name_sets().into_iter().enumerate() {
            let dirs = create_case_dirs(root.path(), case_index, &names);
            let reversed_dirs = dirs.iter().rev().cloned().collect::<Vec<_>>();

            let response = create_batch(state.clone(), dirs.clone()).await;
            assert_eq!(response.status(), StatusCode::CREATED);
            let forward_json = response_json(response).await;

            let response = create_batch(state.clone(), reversed_dirs).await;
            assert_eq!(response.status(), StatusCode::CREATED);
            let reversed_json = response_json(response).await;

            assert_eq!(
                cwd_result_classes(&forward_json),
                cwd_result_classes(&reversed_json)
            );

            cleanup_created_sessions(&state, &forward_json).await;
            cleanup_created_sessions(&state, &reversed_json).await;
        }
    }

    #[tokio::test]
    async fn create_sessions_batch_mr_additive_valid_dir_increases_success_count() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");
        let base_dirs =
            create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
        let mut extended_dirs = base_dirs.clone();
        extended_dirs.extend(create_case_dirs(root.path(), 1, &["docs".to_string()]));

        let response = create_batch(state.clone(), base_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let base_json = response_json(response).await;

        let response = create_batch(state.clone(), extended_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let extended_json = response_json(response).await;

        assert_eq!(success_count(&extended_json), success_count(&base_json) + 1);
        assert_eq!(
            extended_json["results"].as_array().expect("results").len(),
            base_json["results"].as_array().expect("results").len() + 1
        );

        cleanup_created_sessions(&state, &base_json).await;
        cleanup_created_sessions(&state, &extended_json).await;
    }

    #[tokio::test]
    async fn create_sessions_batch_mr_invalid_dir_injection_is_exclusive() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
        let state = test_state();
        let root = tempdir().expect("tempdir");
        let valid_dirs = create_case_dirs(
            root.path(),
            0,
            &["frontend".to_string(), "backend".to_string()],
        );
        let missing_dir = root.path().join("missing").to_string_lossy().into_owned();
        let dirs = vec![
            valid_dirs[0].clone(),
            missing_dir.clone(),
            valid_dirs[1].clone(),
        ];

        let response = create_batch(state.clone(), dirs).await;
        assert_eq!(response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(response).await;
        let results = json["results"].as_array().expect("results");

        assert_eq!(results.len(), 3);
        assert_eq!(success_count(&json), 2);
        assert_eq!(results[1]["index"], 1);
        assert_eq!(results[1]["cwd"], missing_dir);
        assert_eq!(results[1]["ok"], false);
        assert_eq!(results[1]["error"]["code"], "VALIDATION_FAILED");
        assert!(results[0]["session"]["session_id"].is_string());
        assert!(results[2]["session"]["session_id"].is_string());

        cleanup_created_sessions(&state, &json).await;
    }

    #[tokio::test]
    async fn delete_session_rejects_invalid_mode() {
        let response = delete_session(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-missing".to_string()),
            Query(DeleteSessionQuery {
                mode: Some("invalid".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
    }

    #[tokio::test]
    async fn send_input_rejects_empty_text() {
        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: String::new(),
                submit: false,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
    }

    #[tokio::test]
    async fn send_input_returns_not_found_for_missing_session() {
        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-missing".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
                submit: false,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn send_input_rejects_exited_session() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-exited", "tmux-1", cmd_tx))
            .await;

        let worker = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSummary(reply) = cmd {
                    let _ = reply.send(summary("sess-exited", SessionState::Exited));
                    return;
                }
            }
        });

        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-exited".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
                submit: false,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_EXITED");
        worker.await.expect("worker");
    }

    #[tokio::test]
    async fn send_input_forwards_text_to_session_actor() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
            .await;

        let worker = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary("sess-1", SessionState::Idle));
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                        return data;
                    }
                    _ => {}
                }
            }
            Vec::new()
        });

        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
                submit: false,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(worker.await.expect("worker"), b"status".to_vec());
    }

    #[tokio::test]
    async fn send_input_submit_forwards_submit_line_to_session_actor() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
            .await;

        let worker = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary("sess-1", SessionState::Idle));
                    }
                    SessionCommand::SubmitLineAck { text, ack } => {
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                        return text;
                    }
                    _ => {}
                }
            }
            String::new()
        });

        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
                submit: true,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(worker.await.expect("worker"), "status");
    }

    #[tokio::test]
    async fn send_input_reports_failed_delivery_ack() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
            .await;

        let worker = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary("sess-1", SessionState::Idle));
                    }
                    SessionCommand::WriteInputAck { ack, .. } => {
                        let _ = ack.send(InputDeliveryResult {
                            delivered: false,
                            method: "test",
                            message: Some("pty write failed".to_string()),
                        });
                        return;
                    }
                    _ => {}
                }
            }
        });

        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
                submit: false,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INPUT_DELIVERY_FAILED");
        assert_eq!(json["message"], "pty write failed");
        worker.await.expect("worker");
    }

    #[tokio::test]
    async fn send_input_reports_dropped_delivery_ack() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
            .await;

        let worker = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary("sess-1", SessionState::Idle));
                    }
                    SessionCommand::WriteInputAck { ack, .. } => {
                        drop(ack);
                        return;
                    }
                    _ => {}
                }
            }
        });

        let response = send_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-1".to_string()),
            Json(SessionInputRequest {
                text: "status".to_string(),
                submit: false,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INPUT_DELIVERY_UNKNOWN");
        worker.await.expect("worker");
    }

    #[tokio::test]
    async fn get_agent_context_returns_codex_jsonl_snapshot() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
        let sessions_dir = tmp
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("07");
        std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
        std::fs::write(
            sessions_dir.join("rollout-target.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"build the workbench\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"command\\\":\\\"cargo test agent_context\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":777}},\"model_context_window\":258400}}\n"
            ),
        )
        .expect("target rollout");

        let state = test_state();
        let _write_rx =
            insert_summary_test_handle(&state, summary("sess-context", SessionState::Idle)).await;

        let response = get_agent_context(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-context".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-context");
        assert_eq!(json["available"], true);
        assert_eq!(json["tool"], "Codex");
        assert_eq!(json["cwd"], "/tmp/project");
        assert_eq!(json["user_task"], "build the workbench");
        assert_eq!(json["turns"].as_array().unwrap().len(), 1);
        assert_eq!(json["turns"][0]["text"], "build the workbench");
        assert_eq!(json["current_tool"]["tool"], "exec");
        assert_eq!(json["current_tool"]["detail"], "cargo test agent_context");
        assert_eq!(json["recent_actions"][0]["tool"], "exec");
        assert_eq!(json["token_count"], 777);
        assert_eq!(json["context_limit"], 258400);
    }

    #[tokio::test]
    async fn get_transcript_returns_records_after_selected_user_turn() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
        let sessions_dir = tmp
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("10");
        std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
        std::fs::write(
            sessions_dir.join("rollout-transcript.jsonl"),
            [
                json!({"type": "session_meta", "payload": {"cwd": "/tmp/project"}}).to_string(),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>skip me</environment_context>"}]}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "first turn"}}).to_string(),
                json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec", "arguments": "{\"command\":\"cargo test first\"}"}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "second turn"}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "working after second"}}).to_string(),
            ]
            .join("\n")
                + "\n",
        )
        .expect("target rollout");

        let state = test_state();
        let _write_rx =
            insert_summary_test_handle(&state, summary("sess-transcript", SessionState::Idle))
                .await;

        let context_response = get_agent_context(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Path("sess-transcript".to_string()),
        )
        .await;
        assert_eq!(context_response.status(), StatusCode::OK);
        let context_json = response_json(context_response).await;
        let turns = context_json["turns"].as_array().expect("turns");
        assert_eq!(
            turns
                .iter()
                .map(|turn| turn["text"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["first turn", "second turn"]
        );
        assert!(
            !turns.iter().any(|turn| turn["text"]
                .as_str()
                .unwrap()
                .contains("environment_context")),
            "system/environment records must not appear as user turns"
        );

        let first_turn_id = turns[0]["id"].as_str().unwrap().to_string();
        let response = get_transcript(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-transcript".to_string()),
            Query(TranscriptQuery {
                turn_id: Some(first_turn_id),
                after: None,
                limit: Some(10),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["available"], true);
        assert_eq!(json["selected_turn"]["text"], "first turn");
        let records = json["records"].as_array().expect("records");
        assert_eq!(records[0]["kind"], "function_call");
        assert!(records[0]["summary"]
            .as_str()
            .unwrap()
            .contains("cargo test first"));
        assert!(
            records
                .iter()
                .any(|record| record["summary"].as_str().unwrap().contains("second turn")),
            "stream should include later JSONL records after the selected turn"
        );
        assert!(json["next_cursor"].as_u64().unwrap() > turns[0]["byte_end"].as_u64().unwrap());
    }

    #[tokio::test]
    async fn get_agent_context_returns_unavailable_for_unsupported_tool() {
        let state = test_state();
        let mut unsupported = summary("sess-shell", SessionState::Idle);
        unsupported.tool = Some("shell".to_string());
        unsupported.context_limit = 0;
        let _write_rx = insert_summary_test_handle(&state, unsupported).await;

        let response = get_agent_context(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-shell".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-shell");
        assert_eq!(json["available"], false);
        assert_eq!(json["tool"], "shell");
        assert_eq!(json["recent_actions"].as_array().unwrap().len(), 0);
        assert!(json["message"].as_str().unwrap().contains("not supported"));
    }

    #[tokio::test]
    async fn get_agent_context_returns_not_found_for_missing_session() {
        let response = get_agent_context(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("missing-context".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_timeline_returns_ordered_events_and_pinned_summaries() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
        let repo = tempdir().expect("repo tempdir");
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["init", "-q"])
            .status()
            .expect("git init");
        assert!(init.success(), "git init should succeed");
        std::fs::write(repo.path().join("app.txt"), "before\n").expect("write app");
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["add", "app.txt"])
            .status()
            .expect("git add");
        assert!(add.success(), "git add should succeed");
        std::fs::write(repo.path().join("app.txt"), "before\nafter\n").expect("modify app");

        let cwd = repo.path().to_string_lossy().into_owned();
        let sessions_dir = tmp
            .path()
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("05")
            .join("08");
        std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
        let jsonl = [
            json!({"type": "session_meta", "payload": {"cwd": cwd}}).to_string(),
            json!({"type": "response_item", "payload": {"role": "user", "content": [{"type": "input_text", "text": "build the workbench"}]}}).to_string(),
            json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec", "arguments": "{\"command\":\"cargo test timeline\"}"}}).to_string(),
        ]
        .join("\n");
        std::fs::write(
            sessions_dir.join("rollout-timeline-target.jsonl"),
            format!("{jsonl}\n"),
        )
        .expect("timeline jsonl");

        let state = test_state();
        let mut session = summary("sess-timeline", SessionState::Idle);
        session.cwd = cwd.clone();
        insert_timeline_test_handle(
            &state,
            session,
            "cargo test\nfinished green\n".to_string(),
            MermaidArtifactResponse {
                session_id: "sess-timeline".to_string(),
                available: true,
                path: Some("/tmp/project/docs/plan.mmd".to_string()),
                updated_at: Some(Utc::now()),
                source: Some("flowchart TD; A-->B".to_string()),
                error: None,
                slice_name: None,
                plan_files: Some(vec!["plan.md".to_string(), "WORKGRAPH.md".to_string()]),
            },
        )
        .await;

        let response = get_timeline(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-timeline".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-timeline");
        assert_eq!(json["available"], true);
        assert_eq!(json["cwd"], cwd);
        assert_eq!(json["pinned"]["task"]["summary"], "build the workbench");
        assert_eq!(json["pinned"]["current_action"]["title"], "exec");
        assert_eq!(json["pinned"]["diff"]["summary"], "dirty");
        assert_eq!(json["pinned"]["pane_tail"]["summary"], "2 lines");
        assert_eq!(json["pinned"]["artifact"]["summary"], "2 plan files");
        let events = json["events"].as_array().expect("timeline events");
        assert!(events.iter().any(|event| event["kind"] == "task"));
        assert!(events.iter().any(|event| event["kind"] == "tool_call"));
        assert!(events.iter().any(|event| event["kind"] == "diff"));
        assert!(events.iter().any(|event| event["kind"] == "pane_tail"));
        assert!(events.iter().any(|event| event["kind"] == "artifact"));
        let orders = events
            .iter()
            .map(|event| event["order"].as_u64().expect("event order"))
            .collect::<Vec<_>>();
        let sorted = {
            let mut sorted = orders.clone();
            sorted.sort_unstable();
            sorted
        };
        assert_eq!(orders, sorted);
    }

    #[test]
    fn git_diff_timeline_summary_and_detail_cover_available_states() {
        let response = |available: bool,
                        status_short: &str,
                        staged_diff: &str,
                        unstaged_diff: &str,
                        truncated: bool,
                        message: Option<&str>| {
            SessionGitDiffResponse {
                session_id: "sess-diff".to_string(),
                available,
                cwd: "/tmp/project".to_string(),
                repo_root: Some("/tmp/project".to_string()),
                status_short: status_short.to_string(),
                staged_diff: staged_diff.to_string(),
                unstaged_diff: unstaged_diff.to_string(),
                truncated,
                message: message.map(str::to_string),
                files: Vec::new(),
            }
        };

        let clean = response(true, "", "", "", false, None);
        assert_eq!(git_diff_timeline_summary(&clean), "clean");
        assert_eq!(git_diff_timeline_detail(&clean), None);

        let dirty = response(
            true,
            " M app.txt",
            "",
            "diff --git a/app.txt b/app.txt\n@@ -1 +1 @@\n-old\n+new\n",
            false,
            None,
        );
        assert_eq!(git_diff_timeline_summary(&dirty), "dirty");
        let dirty_detail = git_diff_timeline_detail(&dirty).expect("dirty detail");
        assert!(dirty_detail.contains("M app.txt"));
        assert!(dirty_detail.contains("diff --git"));

        let truncated = response(true, "", "diff --git a/lib.rs b/lib.rs\n", "", true, None);
        assert_eq!(git_diff_timeline_summary(&truncated), "dirty, truncated");

        let unavailable = response(false, "", "", "", false, Some("not a git repo"));
        assert_eq!(git_diff_timeline_summary(&unavailable), "not a git repo");

        let unavailable_default = response(false, "", "", "", false, None);
        assert_eq!(
            git_diff_timeline_summary(&unavailable_default),
            "git diff unavailable"
        );
    }

    #[tokio::test]
    async fn get_timeline_keeps_working_without_structured_context() {
        let state = test_state();
        let tmp = tempdir().expect("tempdir");
        let mut session = summary("sess-shell-timeline", SessionState::Idle);
        session.cwd = tmp.path().to_string_lossy().into_owned();
        session.tool = Some("shell".to_string());
        insert_timeline_test_handle(
            &state,
            session,
            "shell output\n".to_string(),
            MermaidArtifactResponse {
                session_id: "sess-shell-timeline".to_string(),
                available: false,
                path: None,
                updated_at: None,
                source: None,
                error: Some("no artifact".to_string()),
                slice_name: None,
                plan_files: None,
            },
        )
        .await;

        let response = get_timeline(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-shell-timeline".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["available"], true);
        assert_eq!(json["pinned"]["pane_tail"]["summary"], "1 lines");
        let events = json["events"].as_array().expect("timeline events");
        assert!(events
            .iter()
            .any(|event| event["id"] == "context-unavailable"));
        assert!(events.iter().any(|event| event["kind"] == "diff"));
        assert!(events.iter().any(|event| event["kind"] == "artifact"));
    }

    #[tokio::test]
    async fn get_timeline_returns_not_found_for_missing_session() {
        let response = get_timeline(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("missing-timeline".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_git_diff_returns_session_repo_diff() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let repo = tempdir().expect("repo tempdir");
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["init", "-q"])
            .status()
            .expect("git init");
        assert!(init.success(), "git init should succeed");
        std::fs::write(repo.path().join("app.txt"), "before\n").expect("write app");
        let add = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["add", "app.txt"])
            .status()
            .expect("git add");
        assert!(add.success(), "git add should succeed");
        std::fs::write(repo.path().join("app.txt"), "before\nafter\n").expect("modify app");

        let state = test_state();
        let mut session = summary("sess-diff", SessionState::Idle);
        session.cwd = repo.path().to_string_lossy().into_owned();
        let _write_rx = insert_summary_test_handle(&state, session).await;

        let response = get_git_diff(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-diff".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-diff");
        assert_eq!(json["available"], true);
        let expected_root = std::fs::canonicalize(repo.path())
            .unwrap_or_else(|_| repo.path().to_path_buf())
            .to_string_lossy()
            .into_owned();
        assert_eq!(json["repo_root"].as_str().unwrap(), expected_root);
        assert!(json["status_short"].as_str().unwrap().contains("app.txt"));
        assert!(json["staged_diff"].as_str().unwrap().contains("new file"));
        assert!(json["unstaged_diff"].as_str().unwrap().contains("+after"));
        let files = json["files"].as_array().expect("structured diff files");
        assert!(files.iter().any(|file| file["path"] == "app.txt"
            && file["source"] == "staged"
            && file["change"] == "added"
            && file["added_lines"].as_u64().unwrap() >= 1
            && !file["hunks"].as_array().unwrap().is_empty()));
        assert!(files.iter().any(|file| file["path"] == "app.txt"
            && file["source"] == "unstaged"
            && file["change"] == "modified"
            && file["added_lines"] == 1));
    }

    #[test]
    fn summarize_git_diff_files_marks_truncation_per_source() {
        // A staged file's summary must reflect only the staged diff's
        // truncation state, never the unstaged diff's (and vice versa).
        let staged_diff = "diff --git a/staged.txt b/staged.txt\n\
            new file mode 100644\n\
            --- /dev/null\n\
            +++ b/staged.txt\n\
            @@ -0,0 +1 @@\n\
            +hello\n";
        let unstaged_diff = "diff --git a/unstaged.txt b/unstaged.txt\n\
            --- a/unstaged.txt\n\
            +++ b/unstaged.txt\n\
            @@ -1 +1 @@\n\
            -old\n\
            +new\n";

        // Only the unstaged diff overflowed.
        let files = summarize_git_diff_files(staged_diff, false, unstaged_diff, true);

        let staged = files
            .iter()
            .find(|f| f.source == "staged")
            .expect("staged file summary");
        let unstaged = files
            .iter()
            .find(|f| f.source == "unstaged")
            .expect("unstaged file summary");

        assert!(
            !staged.truncated,
            "staged file must not be marked truncated when only the unstaged diff overflowed"
        );
        assert!(
            unstaged.truncated,
            "unstaged file must reflect its own truncation"
        );
    }

    #[tokio::test]
    async fn get_git_diff_returns_empty_structured_files_for_clean_repo() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let repo = tempdir().expect("repo tempdir");
        let init = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["init", "-q"])
            .status()
            .expect("git init");
        assert!(init.success(), "git init should succeed");

        let state = test_state();
        let mut session = summary("sess-clean-diff", SessionState::Idle);
        session.cwd = repo.path().to_string_lossy().into_owned();
        let _write_rx = insert_summary_test_handle(&state, session).await;

        let response = get_git_diff(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-clean-diff".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["available"], true);
        assert_eq!(json["status_short"], "");
        assert_eq!(json["staged_diff"], "");
        assert_eq!(json["unstaged_diff"], "");
        assert!(json["files"].as_array().expect("files").is_empty());
    }

    #[tokio::test]
    async fn get_git_diff_returns_unavailable_for_non_repo() {
        let state = test_state();
        let mut session = summary("sess-no-repo", SessionState::Idle);
        let tmp = tempdir().expect("tempdir");
        session.cwd = tmp.path().to_string_lossy().into_owned();
        let _write_rx = insert_summary_test_handle(&state, session).await;

        let response = get_git_diff(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-no-repo".to_string()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["available"], false);
        assert!(json["message"]
            .as_str()
            .unwrap()
            .contains("repo root unavailable"));
    }

    #[tokio::test]
    async fn send_group_input_rejects_empty_session_ids() {
        let response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(SessionGroupInputRequest {
                session_ids: Vec::new(),
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
        assert_eq!(json["message"], "session_ids must not be empty");
    }

    #[test]
    fn group_input_bytes_appends_double_enter_for_agent_delivery() {
        assert_eq!(group_input_bytes("ship it"), b"ship it\r\r");
    }

    #[tokio::test]
    async fn send_group_input_rejects_fewer_than_two_unique_session_ids() {
        let response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(SessionGroupInputRequest {
                session_ids: vec!["only".to_string(), "only".to_string()],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "VALIDATION_FAILED");
        assert_eq!(
            json["message"],
            "session_ids must include at least two unique sessions"
        );
    }

    #[tokio::test]
    async fn send_group_input_sends_only_ready_sessions() {
        let state = test_state();

        let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
        let mut busy = with_test_batch(summary("busy", SessionState::Busy), "batch-group");
        busy.rest_state = RestState::Active;

        let (ready_cmd_tx, mut ready_cmd_rx) = mpsc::channel(8);
        let (ready_write_tx, mut ready_write_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "ready",
                "tmux-ready",
                ready_cmd_tx,
            ))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = ready_cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(ready.clone());
                    }
                    SessionCommand::WriteInput(bytes) => {
                        let _ = ready_write_tx.send(bytes).await;
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = ready_write_tx.send(data).await;
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                    }
                    _ => {}
                }
            }
        });

        let (busy_cmd_tx, mut busy_cmd_rx) = mpsc::channel(8);
        let (busy_write_tx, mut busy_write_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("busy", "tmux-busy", busy_cmd_tx))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = busy_cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(busy.clone());
                    }
                    SessionCommand::WriteInput(bytes) => {
                        let _ = busy_write_tx.send(bytes).await;
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = busy_write_tx.send(data).await;
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                    }
                    _ => {}
                }
            }
        });

        state
            .supervisor
            .persist_thought(
                "ready",
                Some("waiting for direction"),
                0,
                192_000,
                ThoughtState::Sleeping,
                ThoughtSource::Llm,
                RestState::Sleeping,
                false,
                Vec::new(),
                Utc::now(),
                ThoughtDeliveryState::default(),
                None,
                None,
            )
            .await;

        let response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(SessionGroupInputRequest {
                session_ids: vec![
                    "ready".to_string(),
                    "ready".to_string(),
                    "busy".to_string(),
                    "missing".to_string(),
                    remote_sessions::namespace_session_id("jeremy-skillbox", "remote-ready"),
                ],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(response).await;
        assert_eq!(json["delivered"], 1);
        assert_eq!(json["skipped"], 3);
        let results = json["results"].as_array().expect("results");
        assert_eq!(results.len(), 4, "duplicate session IDs should be deduped");
        assert_eq!(results[3]["session_id"], "jeremy-skillbox::remote-ready");
        assert_eq!(results[3]["ok"], false);
        assert_eq!(results[3]["error"]["code"], "SESSION_NOT_FOUND");
        assert_eq!(
            ready_write_rx.recv().await.expect("ready write"),
            b"continue\r\r".to_vec()
        );
        let duplicate_ready_write =
            tokio::time::timeout(Duration::from_millis(25), ready_write_rx.recv()).await;
        assert!(
            matches!(duplicate_ready_write, Err(_) | Ok(None)),
            "duplicate session IDs must not receive duplicate group input"
        );
        let busy_write =
            tokio::time::timeout(Duration::from_millis(25), busy_write_rx.recv()).await;
        assert!(
            matches!(busy_write, Err(_) | Ok(None)),
            "busy sessions must not receive group input"
        );
    }

    #[tokio::test]
    async fn send_group_input_skips_stale_and_disconnected_sessions() {
        let state = test_state();

        let mut ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
        ready.rest_state = RestState::Sleeping;
        let mut stale = with_test_batch(summary("stale", SessionState::Idle), "batch-group");
        stale.rest_state = RestState::Sleeping;
        stale.is_stale = true;
        let mut disconnected =
            with_test_batch(summary("disconnected", SessionState::Idle), "batch-group");
        disconnected.rest_state = RestState::Sleeping;
        disconnected.transport_health = TransportHealth::Disconnected;

        let mut ready_write_rx = insert_summary_test_handle(&state, ready).await;
        let mut stale_write_rx = insert_summary_test_handle(&state, stale).await;
        let mut disconnected_write_rx = insert_summary_test_handle(&state, disconnected).await;

        let response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(SessionGroupInputRequest {
                session_ids: vec![
                    "ready".to_string(),
                    "stale".to_string(),
                    "disconnected".to_string(),
                ],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(response).await;
        assert_eq!(json["delivered"], 1);
        assert_eq!(json["skipped"], 2);
        assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
        assert_eq!(json["results"][2]["error"]["code"], "SESSION_NOT_READY");
        assert_eq!(
            ready_write_rx.recv().await.expect("ready write"),
            b"continue\r\r".to_vec()
        );
        let stale_write =
            tokio::time::timeout(Duration::from_millis(25), stale_write_rx.recv()).await;
        assert!(
            matches!(stale_write, Err(_) | Ok(None)),
            "stale sessions must not receive group input"
        );
        let disconnected_write =
            tokio::time::timeout(Duration::from_millis(25), disconnected_write_rx.recv()).await;
        assert!(
            matches!(disconnected_write, Err(_) | Ok(None)),
            "disconnected sessions must not receive group input"
        );
    }

    #[tokio::test]
    async fn send_group_input_skips_degraded_overloaded_and_unobserved_sessions() {
        let state = test_state();

        let mut ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
        ready.rest_state = RestState::Sleeping;
        let mut degraded = with_test_batch(summary("degraded", SessionState::Idle), "batch-group");
        degraded.rest_state = RestState::Sleeping;
        degraded.transport_health = TransportHealth::Degraded;
        degraded.state_evidence = StateEvidence::unobserved("summary_cache_degraded");
        let mut overloaded = with_test_batch(
            summary("overloaded", SessionState::Attention),
            "batch-group",
        );
        overloaded.transport_health = TransportHealth::Overloaded;
        overloaded.state_evidence = StateEvidence::unobserved("summary_cache_overloaded");
        let mut unobserved =
            with_test_batch(summary("unobserved", SessionState::Idle), "batch-group");
        unobserved.rest_state = RestState::Sleeping;
        unobserved.state_evidence = StateEvidence::unobserved("initial_state");

        let mut ready_write_rx = insert_summary_test_handle(&state, ready).await;
        let degraded_write_rx = insert_summary_test_handle(&state, degraded).await;
        let overloaded_write_rx = insert_summary_test_handle(&state, overloaded).await;
        let unobserved_write_rx = insert_summary_test_handle(&state, unobserved).await;

        let response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(SessionGroupInputRequest {
                session_ids: vec![
                    "ready".to_string(),
                    "degraded".to_string(),
                    "overloaded".to_string(),
                    "unobserved".to_string(),
                ],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(response).await;
        assert_eq!(json["delivered"], 1);
        assert_eq!(json["skipped"], 3);
        assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
        assert_eq!(json["results"][2]["error"]["code"], "SESSION_NOT_READY");
        assert_eq!(json["results"][3]["error"]["code"], "SESSION_NOT_READY");
        assert_eq!(
            ready_write_rx.recv().await.expect("ready write"),
            b"continue\r\r".to_vec()
        );
        for (mut rx, label) in [
            (degraded_write_rx, "degraded"),
            (overloaded_write_rx, "overloaded"),
            (unobserved_write_rx, "unobserved"),
        ] {
            let write = tokio::time::timeout(Duration::from_millis(25), rx.recv()).await;
            assert!(
                matches!(write, Err(_) | Ok(None)),
                "{label} sessions must not receive group input"
            );
        }
    }

    #[tokio::test]
    async fn send_group_input_rejects_attention_deep_sleep_sessions() {
        let state = test_state();

        let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
        let mut deep_attention = with_test_batch(
            summary("deep-attention", SessionState::Attention),
            "batch-group",
        );
        deep_attention.rest_state = RestState::DeepSleep;

        let (ready_cmd_tx, mut ready_cmd_rx) = mpsc::channel(8);
        let (ready_write_tx, mut ready_write_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "ready",
                "tmux-ready",
                ready_cmd_tx,
            ))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = ready_cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(ready.clone());
                    }
                    SessionCommand::WriteInput(bytes) => {
                        let _ = ready_write_tx.send(bytes).await;
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = ready_write_tx.send(data).await;
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                    }
                    _ => {}
                }
            }
        });

        let (deep_cmd_tx, mut deep_cmd_rx) = mpsc::channel(8);
        let (deep_write_tx, mut deep_write_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "deep-attention",
                "tmux-deep-attention",
                deep_cmd_tx,
            ))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = deep_cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(deep_attention.clone());
                    }
                    SessionCommand::WriteInput(bytes) => {
                        let _ = deep_write_tx.send(bytes).await;
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = deep_write_tx.send(data).await;
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                    }
                    _ => {}
                }
            }
        });

        state
            .supervisor
            .persist_thought(
                "ready",
                Some("waiting for direction"),
                0,
                192_000,
                ThoughtState::Sleeping,
                ThoughtSource::Llm,
                RestState::Sleeping,
                false,
                Vec::new(),
                Utc::now(),
                ThoughtDeliveryState::default(),
                None,
                None,
            )
            .await;

        let response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(SessionGroupInputRequest {
                session_ids: vec!["ready".to_string(), "deep-attention".to_string()],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(response).await;
        assert_eq!(json["delivered"], 1);
        assert_eq!(json["skipped"], 1);
        assert_eq!(json["results"][1]["session_id"], "deep-attention");
        assert_eq!(json["results"][1]["ok"], false);
        assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
        assert_eq!(
            ready_write_rx.recv().await.expect("ready write"),
            b"continue\r\r".to_vec()
        );
        let deep_write =
            tokio::time::timeout(Duration::from_millis(25), deep_write_rx.recv()).await;
        assert!(
            matches!(deep_write, Err(_) | Ok(None)),
            "deep sleep sessions must not receive group input"
        );
    }

    #[tokio::test]
    async fn send_group_input_rejects_unbatched_or_mixed_batch_groups() {
        let state = test_state();

        let unbatched = summary("unbatched", SessionState::Idle);
        let batch_a = with_test_batch(summary("batch-a", SessionState::Idle), "batch-a");
        let batch_b = with_test_batch(summary("batch-b", SessionState::Idle), "batch-b");

        for (session_id, summary) in [
            ("unbatched", unbatched),
            ("batch-a", batch_a),
            ("batch-b", batch_b),
        ] {
            let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
            state
                .supervisor
                .insert_test_handle(ActorHandle::test_handle(
                    session_id,
                    format!("tmux-{session_id}"),
                    cmd_tx,
                ))
                .await;
            tokio::spawn(async move {
                while let Some(cmd) = cmd_rx.recv().await {
                    if let SessionCommand::GetSummary(reply) = cmd {
                        let _ = reply.send(summary.clone());
                    }
                }
            });
        }

        let unbatched_response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            Json(SessionGroupInputRequest {
                session_ids: vec!["unbatched".to_string(), "batch-a".to_string()],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(unbatched_response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(unbatched_response).await;
        assert_eq!(json["delivered"], 0);
        assert_eq!(json["skipped"], 2);
        assert_eq!(json["results"][0]["error"]["code"], "SESSION_NOT_IN_BATCH");
        assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_IN_BATCH");

        let mixed_response = send_group_input(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Json(SessionGroupInputRequest {
                session_ids: vec!["batch-a".to_string(), "batch-b".to_string()],
                text: "continue".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(mixed_response.status(), StatusCode::MULTI_STATUS);
        let json = response_json(mixed_response).await;
        assert_eq!(json["delivered"], 0);
        assert_eq!(json["skipped"], 2);
        assert_eq!(
            json["results"][0]["error"]["code"],
            "SESSION_BATCH_MISMATCH"
        );
        assert_eq!(
            json["results"][1]["error"]["code"],
            "SESSION_BATCH_MISMATCH"
        );
    }

    #[tokio::test]
    async fn get_snapshot_returns_actor_snapshot() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-snap", "tmux-snap", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSnapshot(reply) = cmd {
                    let _ = reply.send(TerminalSnapshot {
                        session_id: "sess-snap".to_string(),
                        latest_seq: 9,
                        truncated: false,
                        screen_text: "hello from tmux".to_string(),
                    });
                    break;
                }
            }
        });

        let response = get_snapshot(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-snap".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-snap");
        assert_eq!(json["screen_text"], "hello from tmux");
    }

    #[tokio::test]
    async fn list_sessions_perf_gate_batches_tmux_lookup_within_budget() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, _path_guard) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 0.20
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
work-3	1	1	0.0:%3
work-4	1	1	0.0:%4
work-5	1	1	0.0:%5
work-6	1	1	0.0:%6
EOF
    ;;
  display-message)
    sleep 0.20
    printf '0.0:%%1\n'
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
        );

        let state = test_state();
        let mut expected_ids = Vec::new();
        for index in 1..=6 {
            let session_id = format!("sess-{index}");
            let mut live_summary = summary(&session_id, SessionState::Idle);
            live_summary.tmux_name = format!("work-{index}");
            state
                .supervisor
                .insert_test_handle(spawn_summary_handle(live_summary).await)
                .await;
            expected_ids.push(session_id);
        }
        expected_ids.sort();

        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            let Json(payload) = list_sessions(
                Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
                State(state.clone()),
            )
            .await
            .expect("session list should succeed");
            let elapsed = started.elapsed();
            samples.push(elapsed);

            let mut actual_ids = payload
                .sessions
                .iter()
                .map(|session| session.session_id.clone())
                .collect::<Vec<_>>();
            actual_ids.sort();
            assert_eq!(actual_ids, expected_ids);
        }

        let p95 = p95_duration(samples);
        eprintln!("/v1/sessions p95: {p95:?} (budget 500ms)");
        assert!(
            p95 < Duration::from_millis(500),
            "expected /v1/sessions p95 under 500ms, got {:?}",
            p95
        );
    }

    #[tokio::test]
    async fn list_sessions_perf_gate_skips_hung_tmux_active_pane_lookup() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, _path_guard) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 2
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
EOF
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
        );

        let state = test_state();
        let mut expected_ids = Vec::new();
        for index in 1..=2 {
            let session_id = format!("sess-{index}");
            let mut live_summary = summary(&session_id, SessionState::Idle);
            live_summary.tmux_name = format!("work-{index}");
            state
                .supervisor
                .insert_test_handle(spawn_summary_handle(live_summary).await)
                .await;
            expected_ids.push(session_id);
        }

        let started = Instant::now();
        let Json(payload) = list_sessions(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state),
        )
        .await
        .expect("session list should succeed when tmux stalls");
        let elapsed = started.elapsed();

        let mut actual_ids = payload
            .sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<Vec<_>>();
        actual_ids.sort();
        expected_ids.sort();

        assert_eq!(actual_ids, expected_ids);
        assert!(
            elapsed < Duration::from_millis(900),
            "expected /v1/sessions to degrade gracefully when tmux list-panes stalls, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn get_pane_tail_returns_actor_text() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetPaneTail { lines, reply } = cmd {
                    assert_eq!(lines, 300);
                    let _ = reply.send("recent pane output".to_string());
                    break;
                }
            }
        });

        let response = get_pane_tail(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-tail".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-tail");
        assert_eq!(json["text"], "recent pane output");
    }

    #[tokio::test]
    async fn get_plan_file_returns_actor_payload() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-plan", "tmux-plan", cmd_tx))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetPlanFile { name, reply } = cmd {
                    assert_eq!(name, "plan.md");
                    let _ = reply.send(PlanFileResponse {
                        session_id: "sess-plan".to_string(),
                        name,
                        content: Some("# Plan\n".to_string()),
                        error: None,
                    });
                    break;
                }
            }
        });

        let response = get_plan_file(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-plan".to_string()),
            Query(PlanFileQuery {
                name: "plan.md".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-plan");
        assert_eq!(json["name"], "plan.md");
        assert_eq!(json["content"], "# Plan\n");
    }

    #[tokio::test]
    async fn get_plan_file_returns_not_found_for_missing_session() {
        let response = get_plan_file(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("missing-plan".to_string()),
            Query(PlanFileQuery {
                name: "plan.md".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_plan_file_returns_internal_error_when_actor_drops_reply() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-dropped-plan",
                "tmux-dropped-plan",
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            if let Some(SessionCommand::GetPlanFile { name, reply }) = cmd_rx.recv().await {
                assert_eq!(name, "plan.md");
                drop(reply);
            }
        });

        let response = get_plan_file(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-dropped-plan".to_string()),
            Query(PlanFileQuery {
                name: "plan.md".to_string(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INTERNAL_ERROR");
        assert_eq!(json["message"], "actor dropped plan file reply");
    }

    #[tokio::test]
    async fn request_plan_file_service_reports_timeout_with_short_budget() {
        let state = test_state();
        let (cmd_tx, _cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-timeout-plan",
                "tmux-timeout-plan",
                cmd_tx,
            ))
            .await;

        let err = crate::api::service::request_plan_file_with_timeout(
            &state,
            "sess-timeout-plan",
            "plan.md",
            Duration::from_millis(1),
        )
        .await
        .expect_err("plan request should time out");

        assert!(matches!(
            err,
            crate::api::service::PlanFileServiceError::TimedOut
        ));
    }

    #[tokio::test]
    async fn get_mermaid_artifact_returns_actor_payload() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                "sess-mermaid",
                "tmux-mermaid",
                cmd_tx,
            ))
            .await;

        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetMermaidArtifact(reply) = cmd {
                    let _ = reply.send(MermaidArtifactResponse {
                        session_id: "sess-mermaid".to_string(),
                        available: true,
                        path: Some("/tmp/project/diagram.mmd".to_string()),
                        updated_at: Some(Utc::now()),
                        source: Some("graph TD\nA-->B\n".to_string()),
                        error: None,
                        slice_name: None,
                        plan_files: None,
                    });
                    break;
                }
            }
        });

        let response = get_mermaid_artifact(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-mermaid".to_string()),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-mermaid");
        assert_eq!(json["available"], true);
        assert_eq!(json["path"], "/tmp/project/diagram.mmd");
        assert_eq!(json["source"], "graph TD\nA-->B\n");
    }

    #[tokio::test]
    async fn dismiss_attention_requires_write_scope() {
        let response = dismiss_attention(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Path("sess-1".to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn dismiss_attention_returns_not_found_for_unknown_session() {
        let response = dismiss_attention(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Path("missing".to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn dismiss_attention_forwards_command_and_returns_ok() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-att", "tmux-att", cmd_tx))
            .await;

        let received = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if matches!(cmd, SessionCommand::DismissAttention) {
                    return true;
                }
            }
            false
        });

        let response = dismiss_attention(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Path("sess-att".to_string()),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["ok"], true);
        assert!(
            received.await.expect("worker"),
            "actor never saw DismissAttention"
        );
    }
}
