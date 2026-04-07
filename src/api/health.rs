use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::api::AppState;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    uptime_secs: u64,
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

async fn health() -> Json<HealthResponse> {
    let uptime_secs = process_start().elapsed().as_secs();
    Json(HealthResponse {
        status: "ok",
        uptime_secs,
    })
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
        .route("/version", get(version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use serde_json::Value;

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn health_returns_ok_status_and_uptime() {
        let response = health().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["status"], "ok");
        assert!(
            json["uptime_secs"].is_u64(),
            "uptime_secs should be a u64, got {:?}",
            json["uptime_secs"]
        );
    }

    #[tokio::test]
    async fn version_returns_name_and_cargo_pkg_version() {
        let response = version().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["name"], "swimmers");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    }
}
