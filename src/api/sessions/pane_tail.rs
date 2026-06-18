use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

use crate::api::{remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::types::{LaunchTargetSummary, SessionPaneTailResponse};

use super::error_response;

pub(super) const PANE_TAIL_LINES: usize = 300;
const PANE_TAIL_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// GET /v1/sessions/{session_id}/pane-tail
// ---------------------------------------------------------------------------

pub(super) async fn get_pane_tail(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match pane_tail_route(&session_id) {
        Ok(PaneTailRoute::Remote {
            target,
            remote_session_id,
        }) => remote_pane_tail_response(&target, remote_session_id).await,
        Ok(PaneTailRoute::Local) => local_pane_tail_response(&state, &session_id).await,
        Err(err) => err.into_response(),
    }
}

enum PaneTailRoute<'a> {
    Remote {
        target: LaunchTargetSummary,
        remote_session_id: &'a str,
    },
    Local,
}

fn pane_tail_route(
    session_id: &str,
) -> Result<PaneTailRoute<'_>, remote_sessions::RemoteSessionError> {
    Ok(match remote_sessions::denamespace_for_target(session_id)? {
        Some((target, remote_session_id)) => PaneTailRoute::Remote {
            target,
            remote_session_id,
        },
        None => PaneTailRoute::Local,
    })
}

pub(super) async fn remote_pane_tail_response(
    target: &LaunchTargetSummary,
    remote_session_id: &str,
) -> Response {
    match remote_sessions::fetch_remote_pane_tail(target, remote_session_id).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn local_pane_tail_response(state: &Arc<AppState>, session_id: &str) -> Response {
    match request_pane_tail(state, session_id).await {
        Ok(text) => (
            StatusCode::OK,
            Json(SessionPaneTailResponse {
                session_id: session_id.to_string(),
                text,
            }),
        )
            .into_response(),
        Err(error) => pane_tail_error_response(error),
    }
}

#[derive(Debug)]
pub(super) enum PaneTailError {
    SessionNotFound,
    ActorUnavailable,
    ReplyDropped,
    TimedOut,
}

impl PaneTailError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            PaneTailError::SessionNotFound => "session not found",
            PaneTailError::ActorUnavailable => "session actor unavailable",
            PaneTailError::ReplyDropped => "actor dropped pane tail reply",
            PaneTailError::TimedOut => "pane tail request timed out",
        }
    }
}

pub(super) async fn request_pane_tail(
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

pub(super) async fn request_pane_tail_from_actor(
    handle: &ActorHandle,
) -> Result<String, PaneTailError> {
    request_pane_tail_from_actor_with_timeout(handle, PANE_TAIL_TIMEOUT).await
}

pub(super) async fn request_pane_tail_from_actor_with_timeout(
    handle: &ActorHandle,
    timeout: Duration,
) -> Result<String, PaneTailError> {
    let (tx, rx) = oneshot::channel::<String>();
    if handle.send(pane_tail_request(tx)).await.is_err() {
        return Err(PaneTailError::ActorUnavailable);
    }

    classify_pane_tail_reply(tokio::time::timeout(timeout, rx).await)
}

fn pane_tail_request(reply: oneshot::Sender<String>) -> SessionCommand {
    SessionCommand::GetPaneTail {
        lines: PANE_TAIL_LINES,
        reply,
    }
}

fn classify_pane_tail_reply(
    result: Result<Result<String, oneshot::error::RecvError>, tokio::time::error::Elapsed>,
) -> Result<String, PaneTailError> {
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(_)) => Err(PaneTailError::ReplyDropped),
        Err(_) => Err(PaneTailError::TimedOut),
    }
}

fn pane_tail_error_response(error: PaneTailError) -> Response {
    match error {
        PaneTailError::SessionNotFound => {
            error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None)
        }
        PaneTailError::ActorUnavailable => pane_tail_internal_error("session actor unavailable"),
        PaneTailError::ReplyDropped => pane_tail_internal_error("actor dropped pane tail reply"),
        PaneTailError::TimedOut => pane_tail_internal_error("pane tail request timed out"),
    }
}

fn pane_tail_internal_error(message: &str) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
        Some(message.to_string()),
    )
}
