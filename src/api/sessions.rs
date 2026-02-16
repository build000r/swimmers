use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::session::actor::SessionCommand;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, ErrorResponse, SessionListResponse,
    SessionPaneTailResponse, TerminalSnapshot,
};

// ---------------------------------------------------------------------------
// GET /v1/sessions
// ---------------------------------------------------------------------------

async fn list_sessions(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<SessionListResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let sessions = state.supervisor.list_sessions().await;
    // The version counter is not tracked by the supervisor itself; we use 0
    // as a placeholder. A proper monotonic version can be added to the
    // supervisor later if clients need ETag-style cache validation.
    Ok(Json(SessionListResponse {
        sessions,
        version: 0,
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
    match state.supervisor.create_session(body.name).await {
        Ok(session) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(CreateSessionResponse { session }).unwrap()),
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            // The supervisor returns anyhow errors. We detect specific failure
            // modes by inspecting the error message.
            if msg.contains("already exists") || msg.contains("duplicate session") {
                (
                    StatusCode::CONFLICT,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "SESSION_ALREADY_EXISTS".to_string(),
                            message: Some(msg),
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            } else {
                tracing::error!("create_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "INTERNAL_ERROR".to_string(),
                            message: Some(msg),
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DELETE /v1/sessions/{session_id}
// ---------------------------------------------------------------------------

async fn delete_session(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    match state.supervisor.delete_session(&session_id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "SESSION_NOT_FOUND".to_string(),
                            message: None,
                        })
                        .unwrap(),
                    ),
                )
                    .into_response()
            } else {
                tracing::error!("delete_session failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            code: "INTERNAL_ERROR".to_string(),
                            message: Some(msg),
                        })
                        .unwrap(),
                    ),
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
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    if let Err(e) = handle.send(SessionCommand::DismissAttention).await {
        tracing::error!("[session {session_id}] dismiss_attention send failed: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some(e.to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
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
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<TerminalSnapshot>();
    if handle.send(SessionCommand::GetSnapshot(tx)).await.is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("session actor unavailable".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(snapshot)) => (
            StatusCode::OK,
            Json(serde_json::to_value(snapshot).unwrap()),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("actor dropped snapshot reply".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("snapshot request timed out".to_string()),
                })
                .unwrap(),
            ),
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
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }
    let handle = match state.supervisor.get_session(&session_id).await {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "SESSION_NOT_FOUND".to_string(),
                        message: None,
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    let (tx, rx) = oneshot::channel::<String>();
    if handle
        .send(SessionCommand::GetPaneTail {
            lines: 300,
            reply: tx,
        })
        .await
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("session actor unavailable".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
        Ok(Ok(text)) => (
            StatusCode::OK,
            Json(serde_json::to_value(SessionPaneTailResponse { session_id, text }).unwrap()),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("actor dropped pane tail reply".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INTERNAL_ERROR".to_string(),
                    message: Some("pane tail request timed out".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/sessions", get(list_sessions).post(create_session))
        .route("/v1/sessions/{session_id}", delete(delete_session))
        .route(
            "/v1/sessions/{session_id}/attention/dismiss",
            post(dismiss_attention),
        )
        .route("/v1/sessions/{session_id}/snapshot", get(get_snapshot))
        .route("/v1/sessions/{session_id}/pane-tail", get(get_pane_tail))
}
