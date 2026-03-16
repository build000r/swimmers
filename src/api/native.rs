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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::runtime_config::ThoughtConfig;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use serde_json::Value;
    use tokio::sync::RwLock;

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            daemon_defaults: None,
            file_store: None,
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
        })
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn native_open_rejects_unsupported_hosts() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "example.com".parse().expect("host header"));

        let response = native_open(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            headers,
            Json(NativeDesktopOpenRequest {
                session_id: "sess-1".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "NATIVE_DESKTOP_UNAVAILABLE");
    }

    #[tokio::test]
    async fn native_open_returns_not_found_for_missing_session() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost:3210".parse().expect("host header"));

        let response = native_open(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            headers,
            Json(NativeDesktopOpenRequest {
                session_id: "missing".to_string(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }
}
