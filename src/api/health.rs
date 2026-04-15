use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::api::AppState;
use crate::thought::health::{BridgeHealthSnapshot, BridgeStatus};

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: BridgeStatus,
    uptime_secs: u64,
    thought_bridge: BridgeHealthSnapshot,
}

#[derive(Debug, Serialize)]
struct VersionResponse {
    name: &'static str,
    version: &'static str,
}

// ---------------------------------------------------------------------------
// Process start time — captured the first time it is read.
// ---------------------------------------------------------------------------

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

fn health_response(state: &Arc<AppState>) -> HealthResponse {
    let uptime_secs = process_start().elapsed().as_secs();
    let thought_bridge = state.bridge_health.snapshot();
    HealthResponse {
        status: thought_bridge.status,
        uptime_secs,
        thought_bridge,
    }
}

async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(health_response(&state))
}

async fn readyz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let response = health_response(&state);
    let status = if response.thought_bridge.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(response))
}

// ---------------------------------------------------------------------------
// GET /version
// ---------------------------------------------------------------------------

async fn version() -> Json<VersionResponse> {
    Json(VersionResponse {
        name: "swimmers",
        version: env!("CARGO_PKG_VERSION"),
    })
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn health_router() -> Router<Arc<AppState>> {
    // Ensure the start time is captured at router construction so /health
    // reports a sensible uptime even on the very first request.
    let _ = process_start();
    Router::new()
        .route("/health", get(health))
        .route("/readyz", get(readyz))
        .route("/version", get(version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;

    use crate::config::Config;
    use crate::host_actions::RepoActionTracker;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::health::BridgeHealthState;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    fn test_state(bridge_health: Arc<BridgeHealthState>) -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: None,
            file_store: None,
            bridge_health,
            published_selection: Arc::new(RwLock::new(crate::api::PublishedSelectionState {
                session_id: None,
                published_at: Some(Utc::now()),
            })),
            repo_actions: RepoActionTracker::default(),
        })
    }

    #[tokio::test]
    async fn health_returns_bridge_snapshot_and_uptime() {
        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        bridge_health.record_success(None);
        let response = health(State(test_state(bridge_health)))
            .await
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["status"], "healthy");
        assert!(
            json["uptime_secs"].is_u64(),
            "uptime_secs should be a u64, got {:?}",
            json["uptime_secs"]
        );
        assert_eq!(json["thought_bridge"]["status"], "healthy");
        assert_eq!(json["thought_bridge"]["tick_ms"], 15_000);
    }

    #[tokio::test]
    async fn version_returns_name_and_cargo_pkg_version() {
        let response = version().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["name"], "swimmers");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn readyz_returns_service_unavailable_when_bridge_is_starting() {
        let bridge_health = Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15)));
        let response = readyz(State(test_state(bridge_health)))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json = response_json(response).await;
        assert_eq!(json["status"], "starting");
        assert_eq!(json["thought_bridge"]["status"], "starting");
    }
}
