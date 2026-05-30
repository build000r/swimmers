use crate::api::envelope::{
    api_error, api_error_msg, success_json, NATIVE_DESKTOP_UNAVAILABLE, NATIVE_OPEN_FAILED,
    SESSION_EXITED, SESSION_NOT_FOUND,
};
use crate::api::service::{
    native_status_for_host as native_status_for_host_service, open_native_attention_group_for_host,
    open_native_session_for_host, NativeOpenServiceError,
};
use crate::api::{remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    NativeAttentionGroupOpenRequest, NativeDesktopConfigRequest, NativeDesktopModeRequest,
    NativeDesktopOpenRequest, NativeDesktopStatusResponse,
};
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use std::net::SocketAddr;
use std::sync::Arc;

fn request_peer(ConnectInfo(addr): &ConnectInfo<SocketAddr>) -> String {
    addr.to_string()
}

fn reject_non_loopback_native_preference(
    ConnectInfo(addr): &ConnectInfo<SocketAddr>,
) -> Option<axum::response::Response> {
    if addr.ip().is_loopback() {
        return None;
    }

    Some(api_error_msg(
        &NATIVE_DESKTOP_UNAVAILABLE,
        "native terminal preferences can only be changed from localhost",
    ))
}

async fn native_status_for_peer(
    state: &Arc<AppState>,
    connect_info: &ConnectInfo<SocketAddr>,
) -> NativeDesktopStatusResponse {
    native_status_for_host_service(state, &request_peer(connect_info)).await
}

async fn native_status(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
) -> Result<Json<NativeDesktopStatusResponse>, axum::response::Response> {
    auth.require_scope(AuthScope::SessionsRead)?;
    let status = native_status_for_peer(&state, &connect_info).await;
    Ok(Json(status))
}

async fn set_native_app(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(body): Json<NativeDesktopConfigRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    if let Some(resp) = reject_non_loopback_native_preference(&connect_info) {
        return resp;
    }

    {
        let mut native_app = state.native_desktop_app.write().await;
        *native_app = body.app;
    }

    let status = native_status_for_peer(&state, &connect_info).await;
    success_json(StatusCode::OK, &status)
}

async fn set_native_mode(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(body): Json<NativeDesktopModeRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }
    if let Some(resp) = reject_non_loopback_native_preference(&connect_info) {
        return resp;
    }

    {
        let mut ghostty_mode = state.ghostty_open_mode.write().await;
        *ghostty_mode = body.mode;
    }

    let status = native_status_for_peer(&state, &connect_info).await;
    success_json(StatusCode::OK, &status)
}

async fn native_open(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(body): Json<NativeDesktopOpenRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    if remote_sessions::split_remote_session_id(&body.session_id).is_some() {
        return api_error_msg(
            &NATIVE_DESKTOP_UNAVAILABLE,
            "remote sessions are visible locally, but native terminal handoff must be opened on the target host",
        );
    }

    let peer = request_peer(&connect_info);
    match open_native_session_for_host(&state, &peer, &body.session_id).await {
        Ok(result) => success_json(StatusCode::OK, &result),
        Err(NativeOpenServiceError::Unsupported { reason }) => {
            let msg = reason.unwrap_or_else(|| NATIVE_DESKTOP_UNAVAILABLE.default_message.into());
            api_error_msg(&NATIVE_DESKTOP_UNAVAILABLE, msg)
        }
        Err(NativeOpenServiceError::NoAttentionSessions) => api_error_msg(
            &NATIVE_OPEN_FAILED,
            "no sessions are waiting for operator input",
        ),
        Err(NativeOpenServiceError::SessionNotFound) => api_error(&SESSION_NOT_FOUND),
        Err(NativeOpenServiceError::SessionExited) => api_error(&SESSION_EXITED),
        Err(NativeOpenServiceError::Internal(err)) => api_error_msg(&NATIVE_OPEN_FAILED, err),
    }
}

async fn native_open_attention_group(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(body): Json<NativeAttentionGroupOpenRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let peer = request_peer(&connect_info);
    match open_native_attention_group_for_host(&state, &peer, body).await {
        Ok(result) => success_json(StatusCode::OK, &result),
        Err(NativeOpenServiceError::Unsupported { reason }) => {
            let msg = reason.unwrap_or_else(|| NATIVE_DESKTOP_UNAVAILABLE.default_message.into());
            api_error_msg(&NATIVE_DESKTOP_UNAVAILABLE, msg)
        }
        Err(NativeOpenServiceError::NoAttentionSessions) => api_error_msg(
            &NATIVE_OPEN_FAILED,
            "no sessions are waiting for operator input",
        ),
        Err(NativeOpenServiceError::SessionNotFound) => api_error(&SESSION_NOT_FOUND),
        Err(NativeOpenServiceError::SessionExited) => api_error(&SESSION_EXITED),
        Err(NativeOpenServiceError::Internal(err)) => api_error_msg(&NATIVE_OPEN_FAILED, err),
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/native/status", get(native_status))
        .route("/v1/native/app", put(set_native_app))
        .route("/v1/native/mode", put(set_native_mode))
        .route("/v1/native/open", post(native_open))
        .route(
            "/v1/native/attention-group/open",
            post(native_open_attention_group),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{GhosttyOpenMode, NativeDesktopApp};
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
            native_desktop_app: Arc::new(RwLock::new(NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(GhosttyOpenMode::Swap)),
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

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    fn loopback_peer() -> ConnectInfo<SocketAddr> {
        ConnectInfo("127.0.0.1:3210".parse().expect("loopback peer"))
    }

    fn remote_peer() -> ConnectInfo<SocketAddr> {
        ConnectInfo("100.101.1.2:3210".parse().expect("remote peer"))
    }

    #[tokio::test]
    async fn native_open_rejects_non_loopback_peer() {
        let response = native_open(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            remote_peer(),
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
        let response = native_open(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            loopback_peer(),
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

    #[tokio::test]
    async fn set_native_app_switches_status_to_requested_app() {
        let state = test_state();

        let response = set_native_app(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            loopback_peer(),
            Json(NativeDesktopConfigRequest {
                app: NativeDesktopApp::Ghostty,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["app_id"], "ghostty");
        assert_eq!(json["app"], "Ghostty");

        let status = native_status(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            loopback_peer(),
        )
        .await
        .expect("native status response");
        let json = serde_json::to_value(status.0).expect("status json");
        assert_eq!(json["app_id"], "ghostty");
        assert_eq!(json["app"], "Ghostty");
        assert_eq!(json["ghostty_mode"], "swap");
    }

    #[tokio::test]
    async fn set_native_app_rejects_non_loopback_peer_without_mutating() {
        let state = test_state();

        let response = set_native_app(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            remote_peer(),
            Json(NativeDesktopConfigRequest {
                app: NativeDesktopApp::Ghostty,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "NATIVE_DESKTOP_UNAVAILABLE");

        let app = *state.native_desktop_app.read().await;
        assert_eq!(app, NativeDesktopApp::Iterm);
    }

    #[tokio::test]
    async fn set_native_mode_updates_status_for_ghostty() {
        let state = test_state();
        {
            let mut app = state.native_desktop_app.write().await;
            *app = NativeDesktopApp::Ghostty;
        }

        let response = set_native_mode(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            loopback_peer(),
            Json(NativeDesktopModeRequest {
                mode: GhosttyOpenMode::Add,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["app_id"], "ghostty");
        assert_eq!(json["ghostty_mode"], "add");

        let status = native_status(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            loopback_peer(),
        )
        .await
        .expect("native status response");
        let json = serde_json::to_value(status.0).expect("status json");
        assert_eq!(json["ghostty_mode"], "add");
    }

    #[tokio::test]
    async fn set_native_mode_rejects_non_loopback_peer_without_mutating() {
        let state = test_state();

        let response = set_native_mode(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state.clone()),
            remote_peer(),
            Json(NativeDesktopModeRequest {
                mode: GhosttyOpenMode::Add,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "NATIVE_DESKTOP_UNAVAILABLE");

        let mode = *state.ghostty_open_mode.read().await;
        assert_eq!(mode, GhosttyOpenMode::Swap);
    }
}
