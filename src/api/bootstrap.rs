use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use std::sync::Arc;

use crate::api::AppState;
use crate::config::AuthMode;
use crate::types::BootstrapResponse;

fn auth_mode_to_wire(mode: &AuthMode) -> String {
    match mode {
        AuthMode::LocalTrust => "local-trust".to_string(),
        AuthMode::Token => "token".to_string(),
    }
}

fn delete_mode_to_wire(mode: &crate::config::SessionDeleteMode) -> String {
    match mode {
        crate::config::SessionDeleteMode::DetachBridge => "detach_bridge".to_string(),
        crate::config::SessionDeleteMode::KillTmux => "kill_tmux".to_string(),
    }
}

async fn bootstrap(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Json<BootstrapResponse> {
    let sessions = state.supervisor.bootstrap().await;

    // Derive the realtime WebSocket URL from the request Host header.
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let realtime_url = format!("wss://{host}/v1/realtime");

    let config = &state.config;

    Json(BootstrapResponse {
        server_time: chrono::Utc::now(),
        auth_mode: auth_mode_to_wire(&config.auth_mode),
        realtime_url,
        poll_fallback_ms: config.poll_fallback_ms,
        thought_tick_ms: config.thought_tick_ms,
        thoughts_enabled_default: config.thoughts_enabled_default,
        terminal_cache_ttl_ms: config.terminal_cache_ttl_ms,
        session_delete_mode: delete_mode_to_wire(&config.session_delete_mode),
        legacy_parity_locked: true,
        sessions,
    })
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/bootstrap", get(bootstrap))
}
