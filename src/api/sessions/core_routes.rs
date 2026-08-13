use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::api::envelope::error_body;
use crate::api::service::{
    cleanup_exact_local_session, create_local_session, create_local_sessions_batch,
    list_sessions_for_client,
};
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::config::SessionDeleteMode;
use crate::fleet_lens::{build_fleet_lens_presets, build_fleet_lens_summary};
use crate::session::actor::{ActorHandle, InputDeliveryResult, SessionCommand};
use crate::session::supervisor::TmuxAdoptError;
use crate::types::{
    AdoptSessionRequest, AdoptSessionResponse, AuthorizedProviderResumeLaunchReceipt,
    AuthorizedProviderResumeReceipt, CreateSessionRequest, CreateSessionResponse,
    CreateSessionsBatchRequest, CreateSessionsBatchResponse, EnvironmentListResponse,
    LaunchTargetSummary, ProviderResumeCaptureSource, ProviderResumeProvider, SessionInputRequest,
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
    let environment_metadata = environment_list_response();
    Ok(Json(SessionListResponse {
        fleet_lens: build_fleet_lens_summary(&sessions),
        fleet_presets: environment_metadata.fleet_presets,
        sessions,
        version: 0,
        repo_themes: Default::default(),
        environments: environment_metadata.environments,
    }))
}

pub(super) async fn list_environments(
    Extension(auth): Extension<AuthInfo>,
) -> Result<Json<EnvironmentListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    Ok(Json(environment_list_response()))
}

fn environment_list_response() -> EnvironmentListResponse {
    EnvironmentListResponse {
        environments: remote_sessions::environment_summaries(true),
        fleet_presets: build_fleet_lens_presets(
            crate::session::overlay::default_overlay()
                .map(|overlay| overlay.all_fleet_presets())
                .unwrap_or_default(),
        ),
    }
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

    create_local_session_response(&auth, &state, body).await
}

async fn create_remote_session_response(body: CreateSessionRequest) -> axum::response::Response {
    match remote_sessions::create_remote_session(body).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn create_local_session_response(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    body: CreateSessionRequest,
) -> axum::response::Response {
    let explicit_local_override = explicit_local_launch_override(body.launch_target.as_deref());
    match create_local_session(
        state,
        body.name,
        body.cwd,
        body.spawn_tool,
        body.tmux_target,
        body.initial_request,
    )
    .await
    {
        Ok(mut response) => {
            if explicit_local_override {
                mark_create_session_local_override(&mut response);
            }
            provider_create_session_response(auth, state, response).await
        }
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

async fn provider_create_session_response(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    response: CreateSessionResponse,
) -> Response {
    let (provider_receipt, session_generation) = match response.session.as_ref() {
        Some(session) => {
            let provider_receipt = state
                .supervisor
                .provider_launch_receipt(&session.session_id)
                .await;
            let session_generation = state
                .supervisor
                .exact_cleanup_generation(&session.session_id)
                .await;
            (provider_receipt, session_generation)
        }
        None => (None, None),
    };
    provider_create_session_response_with_receipt(
        auth,
        response,
        provider_receipt,
        session_generation.as_deref(),
    )
}

fn provider_create_session_response_with_receipt(
    auth: &AuthInfo,
    response: CreateSessionResponse,
    stored_receipt: Option<AuthorizedProviderResumeLaunchReceipt>,
    session_generation: Option<&str>,
) -> Response {
    let Some(launch_receipt) = response.launch_receipt.clone() else {
        return (StatusCode::CREATED, Json(response)).into_response();
    };
    let provider_resume = stored_receipt
        .as_ref()
        .map(|receipt| receipt.provider_resume().clone())
        .unwrap_or_else(|| {
            AuthorizedProviderResumeReceipt::unknown(
                ProviderResumeProvider::Unknown,
                ProviderResumeCaptureSource::Unknown,
            )
        });
    let receipt = AuthorizedProviderResumeLaunchReceipt::new(launch_receipt, provider_resume);
    let mut value = match serde_json::to_value(&response) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "failed to serialize create-session response");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some("failed to serialize create-session response".to_string()),
            );
        }
    };
    let Some(object) = value.as_object_mut() else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            Some("invalid create-session response shape".to_string()),
        );
    };

    if auth.has_scope(AuthScope::StreamWrite) {
        let Some(session_generation) = session_generation else {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some("exact cleanup generation unavailable".to_string()),
            );
        };
        let mut authorized_receipt =
            serde_json::to_value(receipt).expect("provider receipt serialization cannot fail");
        authorized_receipt
            .as_object_mut()
            .expect("provider receipt must serialize as an object")
            .insert(
                "session_generation".to_string(),
                serde_json::Value::String(session_generation.to_string()),
            );
        object.insert("launch_receipt".to_string(), authorized_receipt);
    } else {
        object.remove("launch_receipt");
        object.insert(
            "provider_resume".to_string(),
            serde_json::to_value(receipt.public_projection())
                .expect("public provider projection serialization cannot fail"),
        );
    }
    (StatusCode::CREATED, Json(value)).into_response()
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
        .adopt_tmux_session(body.tmux_name, body.tmux_target, body.session_id)
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
        TmuxAdoptError::InvalidTarget { .. } => (StatusCode::BAD_REQUEST, "INVALID_TMUX_TARGET"),
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
    let explicit_local_override = explicit_local_launch_override(body.launch_target.as_deref());
    match create_local_sessions_batch(
        state,
        body.dirs,
        body.spawn_tool,
        body.tmux_target,
        body.initial_request,
    )
    .await
    {
        Ok(mut response) => {
            if explicit_local_override {
                mark_batch_local_override(&mut response);
            }
            create_sessions_batch_response(response)
        }
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

fn explicit_local_launch_override(target: Option<&str>) -> bool {
    target
        .map(str::trim)
        .is_some_and(|target| target.eq_ignore_ascii_case("local"))
}

fn mark_create_session_local_override(response: &mut crate::types::CreateSessionResponse) {
    if let Some(receipt) = response.launch_receipt.as_mut() {
        receipt.mark_local_override();
    }
}

fn mark_batch_local_override(response: &mut CreateSessionsBatchResponse) {
    for result in &mut response.results {
        if let Some(receipt) = result.launch_receipt.as_mut() {
            receipt.mark_local_override();
        }
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
    #[serde(alias = "generation")]
    pub(super) session_generation: Option<String>,
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

    delete_session_response(
        &auth,
        &state,
        &session_id,
        query.session_generation.as_deref(),
        delete_mode,
    )
    .await
}

#[allow(clippy::result_large_err)]
pub(super) fn parse_delete_session_mode(mode: Option<&str>) -> Result<SessionDeleteMode, Response> {
    match mode {
        None | Some("detach_bridge") => Ok(SessionDeleteMode::DetachBridge),
        Some("kill_tmux") => Ok(SessionDeleteMode::KillTmux),
        Some(other) => Err(validation_error(format!("invalid delete mode: {other}"))),
    }
}

async fn delete_session_response(
    auth: &AuthInfo,
    state: &Arc<AppState>,
    session_id: &str,
    session_generation: Option<&str>,
    delete_mode: SessionDeleteMode,
) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::delete_remote_session(
                &target,
                remote_session_id,
                &delete_mode,
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

    let Some(session_generation) = session_generation else {
        return match state
            .supervisor
            .delete_session(session_id, delete_mode)
            .await
        {
            Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
            Err(error) => delete_session_error_response(error),
        };
    };
    if let Err(response) = auth.require_scope(AuthScope::StreamWrite) {
        return response;
    }

    match cleanup_exact_local_session(state, session_id, session_generation, delete_mode).await {
        Ok(receipt) => (StatusCode::OK, Json(receipt)).into_response(),
        Err(error) => error_response(
            error.status(),
            error.code(),
            Some(error.message().to_string()),
        ),
    }
}

pub(super) fn delete_session_error_response(error: anyhow::Error) -> Response {
    let msg = error.to_string();
    // "not found" identifies a genuinely untracked session, but a failed tmux
    // kill on a *live* session can surface "not found" in tmux's stderr too —
    // that must stay a 500, not a misleading 404 that claims the session is gone
    // while it is still tracked and running.
    let session_missing = msg.contains("not found") && !msg.starts_with("tmux kill-session failed");
    if session_missing {
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
    match remote_sessions::denamespace_for_target(&session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::dismiss_remote_attention(&target, remote_session_id).await
            {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
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
            // ok/delivered stay true (some-vs-none contract), but a partial
            // submit is flagged so a caller needing an all-or-nothing delivery
            // can retry without the response's ok flipping (swimmers-bjsu).
            partial: delivery.is_partial(),
            delivery_method: Some(delivery.method.to_string()),
            // Carries the partial-delivery warning ("...may not have been fully
            // submitted") so a 200 isn't silently mistaken for a complete submit.
            message: delivery.message,
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

    match snapshot_route(&session_id) {
        Ok(SnapshotRoute::Remote {
            target,
            remote_session_id,
        }) => remote_snapshot_response(&target, remote_session_id).await,
        Ok(SnapshotRoute::Local) => local_snapshot_response(&state, &session_id).await,
        Err(err) => err.into_response(),
    }
}

enum SnapshotRoute<'a> {
    Remote {
        target: Box<LaunchTargetSummary>,
        remote_session_id: &'a str,
    },
    Local,
}

fn snapshot_route(
    session_id: &str,
) -> Result<SnapshotRoute<'_>, remote_sessions::RemoteSessionError> {
    Ok(match remote_sessions::denamespace_for_target(session_id)? {
        Some((target, remote_session_id)) => SnapshotRoute::Remote {
            target: Box::new(target),
            remote_session_id,
        },
        None => SnapshotRoute::Local,
    })
}

pub(super) async fn remote_snapshot_response(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Response {
    match remote_sessions::fetch_remote_snapshot(target, remote_session_id).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn local_snapshot_response(state: &Arc<AppState>, session_id: &str) -> Response {
    let handle = match state.supervisor.get_session(session_id).await {
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

#[cfg(test)]
mod provider_resume_integration_tests {
    use super::*;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::{Config, SessionDeleteMode};
    use crate::persistence::file_store::FileStore;
    use crate::session::supervisor::{ExactSessionCleanupOutcome, SessionSupervisor};
    use crate::types::{
        LaunchReceipt, ProviderResumeCapability, SpawnTool, PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION,
    };
    use axum::body::to_bytes;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    use tempfile::tempdir;

    struct EnvRestore {
        key: &'static str,
        value: Option<OsString>,
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.value.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    struct TmuxCleanup {
        name: String,
    }

    impl Drop for TmuxCleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("tmux")
                .args(["kill-session", "-t", &format!("={}", self.name)])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    fn response_fixture() -> CreateSessionResponse {
        CreateSessionResponse {
            session: None,
            repo_theme: None,
            launch_receipt: Some(LaunchReceipt::local(
                "/workspace/swimmers",
                "swimmers-session-22",
                false,
            )),
        }
    }

    fn receipt_fixture() -> AuthorizedProviderResumeLaunchReceipt {
        AuthorizedProviderResumeLaunchReceipt::new(
            response_fixture().launch_receipt.expect("launch receipt"),
            AuthorizedProviderResumeReceipt::resumable(
                ProviderResumeProvider::Codex,
                "thread-authoritative-22",
                vec![
                    "codex".to_string(),
                    "resume".to_string(),
                    "thread-authoritative-22".to_string(),
                ],
                "codex resume thread-authoritative-22",
                ProviderResumeCaptureSource::ProviderResponse,
            )
            .expect("valid Codex receipt"),
        )
    }

    async fn response_value(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("response JSON")
    }

    #[tokio::test]
    async fn provider_resume_integration_authorized_response_is_versioned_and_legacy_readable() {
        let response = provider_create_session_response_with_receipt(
            &AuthInfo::new(OPERATOR_SCOPES.to_vec()),
            response_fixture(),
            Some(receipt_fixture()),
            Some("generation-authorized"),
        );
        assert_eq!(response.status(), StatusCode::CREATED);
        let value = response_value(response).await;
        assert_eq!(
            value["launch_receipt"]["version"],
            PROVIDER_RESUME_LAUNCH_RECEIPT_VERSION
        );
        assert_eq!(
            value["launch_receipt"]["provider_resume"]["conversation_id"],
            "thread-authoritative-22"
        );
        assert_eq!(
            value["launch_receipt"]["session_generation"],
            "generation-authorized"
        );
        let legacy: CreateSessionResponse =
            serde_json::from_value(value).expect("legacy CreateSessionResponse reader");
        assert_eq!(
            legacy
                .launch_receipt
                .and_then(|receipt| receipt.session_id)
                .as_deref(),
            Some("swimmers-session-22")
        );
    }

    #[tokio::test]
    async fn provider_resume_integration_public_projection_omits_private_identity() {
        let response = provider_create_session_response_with_receipt(
            &AuthInfo::new(vec![AuthScope::SessionsWrite]),
            response_fixture(),
            Some(receipt_fixture()),
            Some("generation-must-stay-private"),
        );
        let value = response_value(response).await;
        assert!(value.get("launch_receipt").is_none());
        assert_eq!(
            value["provider_resume"]["capability"],
            serde_json::to_value(ProviderResumeCapability::Unknown).expect("capability")
        );
        let encoded = serde_json::to_string(&value).expect("encode response");
        for private in [
            "conversation_id",
            "resume_argv",
            "resume_command",
            "session_generation",
        ] {
            assert!(
                !encoded.contains(private),
                "public response leaked {private}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_resume_integration_grok_adapter_tmux_persistence_api_path() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let fixture = dir.path().join("grok-fixture");
        std::fs::write(&fixture, "#!/bin/sh\nsleep 5\n").expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-provider-integration-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let config = Arc::new(Config::default());
        let supervisor =
            SessionSupervisor::new_with_provider_receipt_data_dir(config, dir.path().join("data"));
        let (session, _) = supervisor
            .create_session(
                Some(tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("provider launch");
        let receipt = supervisor
            .provider_launch_receipt(&session.session_id)
            .await
            .expect("provider receipt");
        assert!(receipt.provider_resume().is_resumable());
        assert_eq!(
            receipt.provider_resume().provider(),
            ProviderResumeProvider::Grok
        );
        assert_eq!(
            receipt.provider_resume().capture_source(),
            ProviderResumeCaptureSource::Preassigned
        );

        let durable_files =
            std::fs::read_dir(dir.path().join("data").join("provider_resume_receipts"))
                .expect("durable receipt directory")
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
                .count();
        assert_eq!(durable_files, 1);

        let response = provider_create_session_response_with_receipt(
            &AuthInfo::new(OPERATOR_SCOPES.to_vec()),
            CreateSessionResponse {
                launch_receipt: Some(LaunchReceipt::local(
                    session.cwd.clone(),
                    session.session_id.clone(),
                    false,
                )),
                session: Some(session.clone()),
                repo_theme: None,
            },
            Some(receipt.clone()),
            supervisor
                .exact_cleanup_generation(&session.session_id)
                .await
                .as_deref(),
        );
        let value = response_value(response).await;
        assert_eq!(
            value["launch_receipt"]["provider_resume"]["conversation_id"],
            receipt
                .provider_resume()
                .conversation_id()
                .expect("conversation id")
        );

        supervisor
            .delete_session(&session.session_id, SessionDeleteMode::KillTmux)
            .await
            .expect("delete exact fixture tmux");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exact_session_cleanup_provider_history_survives_tmux_kill_and_resume_is_available() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let history_dir = dir.path().join("provider-history");
        std::fs::create_dir(&history_dir).expect("history directory");
        let fixture = dir.path().join("grok-history-fixture");
        let history_arg = crate::launcher::shell_single_quote(&history_dir.to_string_lossy());
        std::fs::write(
            &fixture,
            format!(
                r#"#!/bin/sh
history_dir={history_arg}
if [ "$1" = "--resume" ]; then
  test -f "$history_dir/$2"
  exit $?
fi
conversation_id=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --session-id)
      conversation_id="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
test -n "$conversation_id" || exit 31
: > "$history_dir/$conversation_id"
sleep 5
"#
            ),
        )
        .expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-exact-cleanup-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let data_dir = dir.path().join("data");
        let supervisor = SessionSupervisor::new_with_provider_receipt_data_dir(
            Arc::new(Config::default()),
            data_dir.clone(),
        );
        let (session, _) = supervisor
            .create_session(
                Some(tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("provider launch");
        let generation = supervisor
            .exact_cleanup_generation(&session.session_id)
            .await
            .expect("cleanup generation");
        let before = supervisor
            .provider_launch_receipt(&session.session_id)
            .await
            .expect("provider receipt before cleanup");

        let cleanup = supervisor
            .delete_exact_session(
                &session.session_id,
                &generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect("exact cleanup");
        assert_eq!(cleanup.outcome, ExactSessionCleanupOutcome::Deleted);
        assert!(!cleanup.tmux_session_alive);
        assert!(supervisor.get_session(&session.session_id).await.is_none());

        let after = supervisor
            .provider_launch_receipt(&session.session_id)
            .await
            .expect("provider receipt survives cleanup");
        assert_eq!(after, before);
        let durable_files =
            std::fs::read_dir(dir.path().join("data").join("provider_resume_receipts"))
                .expect("durable receipt directory")
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
                .count();
        assert_eq!(durable_files, 1);

        let resume_argv = after
            .provider_resume()
            .resume_argv()
            .expect("exact resume argv survives");
        let resume = std::process::Command::new(&resume_argv[0])
            .args(&resume_argv[1..])
            .current_dir(dir.path())
            .status()
            .expect("execute exact provider resume");
        assert!(resume.success(), "provider history missing after tmux kill");

        let restarted = SessionSupervisor::new_with_provider_receipt_data_dir(
            Arc::new(Config::default()),
            data_dir,
        );
        let registry_store = FileStore::new(dir.path().join("registry"))
            .await
            .expect("restart registry store");
        restarted.init_persistence(registry_store).await;
        let replacement_tmux_name = format!("swimmers-post-restart-{}", uuid::Uuid::new_v4());
        let _replacement_tmux_cleanup = TmuxCleanup {
            name: replacement_tmux_name.clone(),
        };
        let (replacement, _) = restarted
            .create_session(
                Some(replacement_tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("post-restart provider launch");
        assert_ne!(
            replacement.session_id, session.session_id,
            "durable cleanup tombstone must reserve the old session id"
        );
        assert_eq!(
            restarted.provider_launch_receipt(&session.session_id).await,
            Some(after.clone()),
            "new launch must not overwrite old provider resume receipt"
        );
        let repeat = restarted
            .delete_exact_session(
                &session.session_id,
                &generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect("idempotent cleanup repeat");
        assert_eq!(repeat.outcome, ExactSessionCleanupOutcome::AlreadyGone);
        assert_eq!(
            restarted.provider_launch_receipt(&session.session_id).await,
            Some(after)
        );
        let replacement_generation = restarted
            .exact_cleanup_generation(&replacement.session_id)
            .await
            .expect("replacement cleanup generation");
        restarted
            .delete_exact_session(
                &replacement.session_id,
                &replacement_generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect("replacement exact cleanup");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exact_session_cleanup_stale_generation_cannot_kill_same_name_replacement() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let fixture = dir.path().join("grok-reuse-fixture");
        std::fs::write(&fixture, "#!/bin/sh\nsleep 5\n").expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-exact-reuse-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let supervisor = SessionSupervisor::new_with_provider_receipt_data_dir(
            Arc::new(Config::default()),
            dir.path().join("data"),
        );
        let (session, _) = supervisor
            .create_session(
                Some(tmux_name.clone()),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect("original provider launch");
        let generation = supervisor
            .exact_cleanup_generation(&session.session_id)
            .await
            .expect("original generation");
        supervisor
            .delete_session(&session.session_id, SessionDeleteMode::DetachBridge)
            .await
            .expect("detach original actor");

        let killed = std::process::Command::new("tmux")
            .args(["kill-session", "-t", &format!("={tmux_name}")])
            .status()
            .expect("kill original tmux");
        assert!(killed.success());
        let replacement = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &tmux_name, "sleep 5"])
            .status()
            .expect("spawn same-name replacement");
        assert!(replacement.success());

        let error = supervisor
            .delete_exact_session(
                &session.session_id,
                &generation,
                SessionDeleteMode::KillTmux,
            )
            .await
            .expect_err("stale authority must reject replacement incarnation");
        assert_eq!(
            error,
            crate::session::supervisor::ExactSessionCleanupError::TmuxIncarnationMismatch
        );
        let replacement_alive = std::process::Command::new("tmux")
            .args(["has-session", "-t", &format!("={tmux_name}")])
            .status()
            .expect("probe replacement tmux");
        assert!(replacement_alive.success());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_resume_integration_rejected_grok_never_persists_or_returns_success() {
        let _env_lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(crate::launcher::SWIMMERS_GROK_BIN_ENV);
        let _restore = EnvRestore {
            key: crate::launcher::SWIMMERS_GROK_BIN_ENV,
            value: original,
        };
        let dir = tempdir().expect("tempdir");
        let fixture = dir.path().join("grok-reject-fixture");
        std::fs::write(&fixture, "#!/bin/sh\nexit 29\n").expect("write fixture");
        let mut permissions = std::fs::metadata(&fixture)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&fixture, permissions).expect("fixture executable");
        std::env::set_var(crate::launcher::SWIMMERS_GROK_BIN_ENV, &fixture);

        let tmux_name = format!("swimmers-provider-reject-{}", uuid::Uuid::new_v4());
        let _tmux_cleanup = TmuxCleanup {
            name: tmux_name.clone(),
        };
        let config = Arc::new(Config::default());
        let supervisor =
            SessionSupervisor::new_with_provider_receipt_data_dir(config, dir.path().join("data"));
        let error = supervisor
            .create_session(
                Some(tmux_name),
                Some(dir.path().to_string_lossy().into_owned()),
                Some(SpawnTool::Grok),
                None,
            )
            .await
            .expect_err("rejected Grok launch must fail");
        assert!(
            error
                .to_string()
                .contains("did not remain running long enough"),
            "{error:#}"
        );
        let receipt_dir = dir.path().join("data").join("provider_resume_receipts");
        let durable_files = std::fs::read_dir(receipt_dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(durable_files, 0);
    }
}
