use crate::api::{fetch_live_summary, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{ErrorResponse, NativeDesktopOpenRequest, NativeDesktopStatusResponse};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use std::sync::Arc;

fn request_host(headers: &HeaderMap) -> &str {
    headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost")
}

fn unsupported_native_response(status: &NativeDesktopStatusResponse) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(
            serde_json::to_value(ErrorResponse {
                code: "NATIVE_DESKTOP_UNAVAILABLE".to_string(),
                message: status.reason.clone(),
            })
            .unwrap(),
        ),
    )
        .into_response()
}

async fn native_status(
    Extension(auth): Extension<AuthInfo>,
    headers: HeaderMap,
) -> Result<Json<NativeDesktopStatusResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let status = crate::native::support_for_host(request_host(&headers));
    Ok(Json(status))
}

async fn native_open(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<NativeDesktopOpenRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let status = crate::native::support_for_host(request_host(&headers));
    if !status.supported {
        return unsupported_native_response(&status);
    }

    let summary = match fetch_live_summary(&state, &body.session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
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
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::to_value(ErrorResponse {
                        code: "INTERNAL_ERROR".to_string(),
                        message: Some(err.to_string()),
                    })
                    .unwrap(),
                ),
            )
                .into_response();
        }
    };

    if summary.state == crate::types::SessionState::Exited {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "SESSION_EXITED".to_string(),
                    message: Some("session has already exited".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    match crate::native::open_or_focus_iterm_session(
        &summary.session_id,
        &summary.tmux_name,
        &summary.cwd,
    )
    .await
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::to_value(result).unwrap())).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "NATIVE_DESKTOP_OPEN_FAILED".to_string(),
                    message: Some(err.to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response(),
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/native/status", get(native_status))
        .route("/v1/native/open", post(native_open))
}
