pub mod dirs;
pub mod envelope;
pub mod health;
pub mod native;
pub mod operator_pressure;
pub mod remote_sessions;
pub mod selection;
pub mod service;
pub(crate) mod session_git_diff;
pub mod sessions;
pub mod skills;
pub mod thought_config;
pub mod web_actions;

use axum::body::to_bytes;
use axum::http::{header, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::Router;
use chrono::{DateTime, Utc};
use std::sync::{Arc, OnceLock};
use tokio::sync::{oneshot, RwLock};

use crate::auth;
use crate::config::Config;
use crate::host_actions::RepoActionTracker;
use crate::persistence::file_store::FileStore;
use crate::session::actor::SessionCommand;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::health::BridgeHealthState;
use crate::thought::protocol::SyncRequestSequence;
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};
use crate::types::{GhosttyOpenMode, NativeDesktopApp, SessionSummary};

#[derive(Debug, Clone, Default)]
pub struct PublishedSelectionState {
    pub session_id: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

pub struct AppState {
    pub supervisor: Arc<SessionSupervisor>,
    pub config: Arc<Config>,
    pub thought_config: Arc<RwLock<ThoughtConfig>>,
    pub native_desktop_app: Arc<RwLock<NativeDesktopApp>>,
    pub ghostty_open_mode: Arc<RwLock<GhosttyOpenMode>>,
    pub sync_request_sequence: Arc<SyncRequestSequence>,
    pub daemon_defaults: OnceLock<DaemonDefaults>,
    pub file_store: OnceLock<Arc<FileStore>>,
    pub bridge_health: Arc<BridgeHealthState>,
    pub published_selection: Arc<RwLock<PublishedSelectionState>>,
    pub repo_actions: RepoActionTracker,
}

impl AppState {
    pub fn current_daemon_defaults(&self) -> Option<DaemonDefaults> {
        self.daemon_defaults.get().cloned()
    }

    pub fn current_file_store(&self) -> Option<Arc<FileStore>> {
        self.file_store.get().cloned()
    }

    pub fn set_daemon_defaults(&self, defaults: DaemonDefaults) -> bool {
        self.daemon_defaults.set(defaults).is_ok()
    }

    pub fn set_file_store(&self, store: Arc<FileStore>) -> bool {
        self.file_store.set(store).is_ok()
    }
}

pub fn once_lock_with<T>(value: Option<T>) -> OnceLock<T> {
    let lock = OnceLock::new();
    if let Some(value) = value {
        let _ = lock.set(value);
    }
    lock
}

pub(crate) async fn fetch_live_summary(
    state: &Arc<AppState>,
    session_id: &str,
) -> anyhow::Result<Option<SessionSummary>> {
    let handle = match state.supervisor.get_session(session_id).await {
        Some(handle) => handle,
        None => return Ok(None),
    };

    let (tx, rx) = oneshot::channel();
    handle
        .send(SessionCommand::GetSummary(tx))
        .await
        .map_err(|err| anyhow::anyhow!("failed to request session summary: {err}"))?;

    let summary = tokio::time::timeout(std::time::Duration::from_secs(2), rx)
        .await
        .map_err(|_| anyhow::anyhow!("session summary request timed out"))?
        .map_err(|_| anyhow::anyhow!("session summary actor dropped reply"))?;

    Ok(Some(summary))
}

pub fn api_router(config: Arc<Config>) -> Router<Arc<AppState>> {
    let personal_workflows_enabled = config.personal_workflows_enabled;
    let config_for_middleware = config;

    let router = Router::new()
        .merge(native::routes())
        .merge(operator_pressure::routes())
        .merge(selection::routes())
        .merge(sessions::routes())
        .merge(thought_config::routes())
        .merge(web_actions::routes(personal_workflows_enabled));

    let router = if personal_workflows_enabled {
        router.merge(dirs::routes()).merge(skills::routes())
    } else {
        router
    };

    router
        .layer(middleware::map_response(api_error_envelope_middleware))
        .layer(middleware::from_fn(move |request, next| {
            auth::auth_middleware(config_for_middleware.clone(), request, next)
        }))
        .merge(health::health_router())
}

async fn api_error_envelope_middleware(response: Response) -> Response {
    let status = response.status();
    if !should_envelope_api_error(&response) {
        return response;
    }

    let message = response_body_text(response, status).await;
    let code = inferred_extractor_error_code(status, &message);
    (
        status,
        axum::Json(crate::api::envelope::error_body(code, Some(message))),
    )
        .into_response()
}

fn should_envelope_api_error(response: &Response) -> bool {
    response.status().is_client_error()
        && !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"))
}

async fn response_body_text(response: Response, status: StatusCode) -> String {
    match to_bytes(response.into_body(), 64 * 1024).await {
        Ok(bytes) if !bytes.is_empty() => String::from_utf8_lossy(&bytes).trim().to_string(),
        _ => status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_string(),
    }
}

fn inferred_extractor_error_code(status: StatusCode, message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("json") || message.contains("content-type") {
        "INVALID_JSON"
    } else if message.contains("query") {
        "INVALID_QUERY"
    } else if message.contains("path") || message.contains("url") {
        "INVALID_PATH"
    } else if status == StatusCode::PAYLOAD_TOO_LARGE {
        "INPUT_TOO_LARGE"
    } else {
        "VALIDATION_FAILED"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::health::BridgeHealthState;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{GhosttyOpenMode, NativeDesktopApp};
    use serde_json::Value;
    use std::time::Duration;

    fn test_state(config: Arc<Config>) -> Arc<AppState> {
        Arc::new(AppState {
            supervisor: SessionSupervisor::new(config.clone()),
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: once_lock_with(None),
            file_store: once_lock_with(None),
            bridge_health: Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15))),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    async fn spawn_api_test_server() -> (String, tokio::task::JoinHandle<()>) {
        let config = Arc::new(Config::default());
        let app = api_router(config.clone()).with_state(test_state(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind api test server");
        let addr = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve api");
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn JsonRejection_malformed_json_returns_api_error_envelope() {
        let (base_url, handle) = spawn_api_test_server().await;

        let response = reqwest::Client::new()
            .post(format!("{base_url}/v1/sessions/group-input"))
            .header(header::CONTENT_TYPE, "application/json")
            .body("{")
            .send()
            .await
            .expect("api response");

        assert_eq!(response.status().as_u16(), StatusCode::BAD_REQUEST.as_u16());
        let json: Value = response.json().await.expect("json body");
        assert_eq!(json["code"], "INVALID_JSON");
        assert!(json["message"]
            .as_str()
            .expect("message")
            .to_ascii_lowercase()
            .contains("json"));
        handle.abort();
    }

    #[tokio::test]
    async fn query_rejection_returns_api_error_envelope() {
        let (base_url, handle) = spawn_api_test_server().await;

        let response = reqwest::Client::new()
            .get(format!("{base_url}/v1/sessions/sess-plan/plan-file"))
            .send()
            .await
            .expect("api response");

        assert_eq!(response.status().as_u16(), StatusCode::BAD_REQUEST.as_u16());
        let json: Value = response.json().await.expect("json body");
        assert_eq!(json["code"], "INVALID_QUERY");
        assert!(json["message"]
            .as_str()
            .expect("message")
            .to_ascii_lowercase()
            .contains("query"));
        handle.abort();
    }

    #[test]
    fn path_rejection_message_infers_invalid_path_code() {
        assert_eq!(
            inferred_extractor_error_code(StatusCode::BAD_REQUEST, "Invalid URL path segment"),
            "INVALID_PATH"
        );
    }
}
