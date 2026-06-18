use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::api::envelope::error_body;
use crate::api::service::{create_local_sessions_batch, list_sessions_for_client};
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::config::SessionDeleteMode;
use crate::fleet_lens::build_fleet_lens_summary;
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};
use crate::session::supervisor::TmuxAdoptError;
use crate::types::{
    AdoptSessionRequest, AdoptSessionResponse, CreateSessionRequest, CreateSessionResponse,
    CreateSessionsBatchRequest, CreateSessionsBatchResponse, SessionInputRequest,
    SessionInputResponse, SessionListResponse, SessionState, TerminalSnapshot,
    MAX_SESSION_INPUT_BYTES,
};

const SNAPSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const INPUT_DELIVERY_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

// ---------------------------------------------------------------------------
// GET /v1/sessions
// ---------------------------------------------------------------------------

pub(super) async fn list_sessions(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let sessions = list_sessions_for_client(&state, true).await;
    // The version counter is not tracked by the supervisor itself; we use 0
    // as a placeholder. A proper monotonic version can be added to the
    // supervisor later if clients need ETag-style cache validation.
    Ok(Json(SessionListResponse {
        fleet_lens: build_fleet_lens_summary(&sessions),
        sessions,
        version: 0,
        repo_themes: Default::default(),
        environments: remote_sessions::environment_summaries(true),
    }))
}

// ---------------------------------------------------------------------------
// POST /v1/sessions
// ---------------------------------------------------------------------------

pub(super) async fn create_session(
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
        error_response(StatusCode::CONFLICT, "SESSION_ALREADY_EXISTS", Some(msg))
    } else if msg.contains("cwd does not exist") {
        validation_error(msg)
    } else {
        tracing::error!("create_session failed: {error}");
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some(msg),
        )
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/adopt
// ---------------------------------------------------------------------------

pub(super) async fn adopt_session(
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

    error_response(status, code, Some(error.to_string()))
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/batch
// ---------------------------------------------------------------------------

pub(super) async fn create_sessions_batch(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionsBatchRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if remote_sessions::is_remote_launch_target(body.launch_target.as_deref()) {
        return create_remote_sessions_batch_response(body).await;
    }

    create_local_sessions_batch_response(state, body).await
}

async fn create_remote_sessions_batch_response(body: CreateSessionsBatchRequest) -> Response {
    remote_sessions_batch_result_response(remote_sessions::create_remote_sessions_batch(body).await)
}

fn remote_sessions_batch_result_response(
    result: Result<CreateSessionsBatchResponse, remote_sessions::RemoteSessionError>,
) -> Response {
    match result {
        Ok(response) => create_sessions_batch_response(response),
        Err(err) => err.into_response(),
    }
}

async fn create_local_sessions_batch_response(
    state: Arc<AppState>,
    body: CreateSessionsBatchRequest,
) -> Response {
    match create_local_sessions_batch(state, body.dirs, body.spawn_tool, body.initial_request).await
    {
        Ok(response) => create_sessions_batch_response(response),
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

fn create_sessions_batch_response(response: CreateSessionsBatchResponse) -> Response {
    (create_sessions_batch_status(&response), Json(response)).into_response()
}

fn create_sessions_batch_status(response: &CreateSessionsBatchResponse) -> StatusCode {
    if response.results.iter().all(|result| result.ok) {
        StatusCode::CREATED
    } else {
        StatusCode::MULTI_STATUS
    }
}

// ---------------------------------------------------------------------------
// DELETE /v1/sessions/{session_id}
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct DeleteSessionQuery {
    pub(super) mode: Option<String>,
}

pub(super) async fn delete_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<DeleteSessionQuery>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let delete_mode = match parse_delete_session_mode(query.mode.as_deref()) {
        Ok(delete_mode) => delete_mode,
        Err(response) => return response,
    };

    delete_session_response(&state, &session_id, delete_mode).await
}

pub(super) fn parse_delete_session_mode(mode: Option<&str>) -> Result<SessionDeleteMode, Response> {
    match mode {
        None | Some("detach_bridge") => Ok(SessionDeleteMode::DetachBridge),
        Some("kill_tmux") => Ok(SessionDeleteMode::KillTmux),
        Some(other) => Err(validation_error(format!("invalid delete mode: {other}"))),
    }
}

async fn delete_session_response(
    state: &Arc<AppState>,
    session_id: &str,
    delete_mode: SessionDeleteMode,
) -> Response {
    match state
        .supervisor
        .delete_session(session_id, delete_mode)
        .await
    {
        Ok(()) => delete_session_success_response(),
        Err(error) => delete_session_error_response(error),
    }
}

fn delete_session_success_response() -> Response {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

pub(super) fn delete_session_error_response(error: anyhow::Error) -> Response {
    let msg = error.to_string();
    if msg.contains("not found") {
        error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None)
    } else {
        tracing::error!("delete_session failed: {error}");
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some(msg),
        )
    }
}

// ---------------------------------------------------------------------------
// POST /v1/sessions/{session_id}/attention/dismiss
// ---------------------------------------------------------------------------

pub(super) async fn dismiss_attention(
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
            return error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::error!("[session {session_id}] dismiss_attention send failed: {e}");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some(e.to_string()),
        );
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

fn input_too_large_error() -> Response {
    error_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        "INPUT_TOO_LARGE",
        Some(format!(
            "terminal input exceeds {MAX_SESSION_INPUT_BYTES} byte limit"
        )),
    )
}

pub(super) fn error_response(
    status: StatusCode,
    code: impl Into<String>,
    message: Option<String>,
) -> Response {
    (status, Json(error_body(code, message))).into_response()
}

pub(super) async fn send_input(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(body): Json<SessionInputRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    send_input_response(&state, session_id, body).await
}

async fn send_input_response(
    state: &Arc<AppState>,
    session_id: String,
    body: SessionInputRequest,
) -> Response {
    if body.text.is_empty() {
        return validation_error("text must not be empty");
    }
    if body.text.len() > MAX_SESSION_INPUT_BYTES {
        return input_too_large_error();
    }
    if body.submit && body.text.trim().is_empty() {
        return validation_error("submitted text must not be blank");
    }

    match remote_sessions::denamespace_for_target(&session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::send_remote_input(&target, remote_session_id, body).await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    match deliver_session_input(state, &session_id, body).await {
        Ok(delivery) => session_input_delivery_response(session_id, delivery),
        Err(response) => response,
    }
}

async fn deliver_session_input(
    state: &Arc<AppState>,
    session_id: &str,
    body: SessionInputRequest,
) -> Result<InputDeliveryResult, Response> {
    let handle = match writable_session_handle(state, session_id).await {
        Ok(handle) => handle,
        Err(response) => return Err(response),
    };

    let (ack_tx, ack_rx) = oneshot::channel();
    let command = session_input_command(body, ack_tx);
    send_session_input_command(session_id, &handle, command).await?;
    wait_for_input_delivery(ack_rx).await
}

async fn send_session_input_command(
    session_id: &str,
    handle: &ActorHandle,
    command: SessionCommand,
) -> Result<(), Response> {
    if let Err(err) = handle.send(command).await {
        tracing::error!("[session {session_id}] send_input failed: {err}");
        return Err(error_response(
            StatusCode::NOT_FOUND,
            "SESSION_NOT_FOUND",
            Some(err.to_string()),
        ));
    }

    Ok(())
}

pub(super) fn session_input_delivery_response(
    session_id: String,
    delivery: InputDeliveryResult,
) -> Response {
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
            ));
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
    match tokio::time::timeout(INPUT_DELIVERY_ACK_TIMEOUT, ack_rx).await {
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
// GET /v1/sessions/{session_id}/snapshot
// ---------------------------------------------------------------------------

pub(super) async fn get_snapshot(
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
            return error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
    };

    snapshot_request_response(request_terminal_snapshot(&handle).await)
}

pub(super) async fn request_terminal_snapshot(
    handle: &ActorHandle,
) -> Result<TerminalSnapshot, SnapshotRequestError> {
    let (tx, rx) = oneshot::channel::<TerminalSnapshot>();
    if handle.send(SessionCommand::GetSnapshot(tx)).await.is_err() {
        return Err(SnapshotRequestError::ActorUnavailable);
    }

    match tokio::time::timeout(SNAPSHOT_TIMEOUT, rx).await {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(_)) => Err(SnapshotRequestError::ReplyDropped),
        Err(_) => Err(SnapshotRequestError::Timeout),
    }
}

fn snapshot_request_response(result: Result<TerminalSnapshot, SnapshotRequestError>) -> Response {
    match result {
        Ok(snapshot) => (StatusCode::OK, Json(snapshot)).into_response(),
        Err(error) => snapshot_error_response(error),
    }
}

pub(super) fn snapshot_error_response(error: SnapshotRequestError) -> Response {
    let detail = match error {
        SnapshotRequestError::ActorUnavailable => "session actor unavailable",
        SnapshotRequestError::ReplyDropped => "actor dropped snapshot reply",
        SnapshotRequestError::Timeout => "snapshot request timed out",
    };
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
        Some(detail.to_string()),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SnapshotRequestError {
    ActorUnavailable,
    ReplyDropped,
    Timeout,
}
