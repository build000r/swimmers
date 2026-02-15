mod api;
mod config;
mod realtime;
mod scroll;
mod session;
mod state;
mod thought;
mod types;

use std::sync::Arc;

use axum::Router;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use api::AppState;
use config::Config;
use session::supervisor::SessionSupervisor;

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env();
    let port = config.port;
    let config = Arc::new(config);

    // Create session supervisor (new() returns Arc<Self>)
    let supervisor = SessionSupervisor::new(config.clone());

    // Auto-discover existing tmux sessions
    {
        let supervisor = supervisor.clone();
        tokio::spawn(async move {
            match supervisor.discover_tmux_sessions().await {
                Ok(()) => tracing::info!("tmux session discovery complete"),
                Err(e) => tracing::error!("tmux discovery failed: {e}"),
            }
        });
    }

    // Start thought generation loop.
    // TODO: Wire up SessionProvider impl for SessionSupervisor and call
    // ThoughtLoopRunner::new(config.thought_tick_ms, event_tx).spawn(provider)
    // once the broadcast channel integration is finalized.

    // Build app state
    let state = Arc::new(AppState {
        supervisor,
        config: config.clone(),
    });

    // Build router
    let app = Router::new()
        .merge(api::api_router())
        .nest("/v1/realtime", realtime::handler::ws_router())
        .fallback_service(ServeDir::new("dist").append_index_html_on_directories(true))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("failed to bind");

    tracing::info!("Throngterm running on http://0.0.0.0:{port}");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
