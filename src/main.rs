mod api;
mod auth;
mod config;
mod metrics;
mod persistence;
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
use persistence::file_store::FileStore;
use session::supervisor::{SessionSupervisor, SupervisorProvider};
use thought::loop_runner::ThoughtLoopRunner;

#[tokio::main]
async fn main() {
    // Load .env before anything reads env vars.
    let _ = dotenvy::dotenv();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Initialize Prometheus metrics exporter
    let prom_handle = metrics::init_metrics();

    let config = Config::from_env();
    let port = config.port;
    let config = Arc::new(config);

    // Create session supervisor (new() returns Arc<Self>)
    let supervisor = SessionSupervisor::new(config.clone());

    // Initialize persistence store.
    match FileStore::new("./data/throngterm/").await {
        Ok(store) => {
            supervisor.init_persistence(store).await;
            tracing::info!("persistence store initialized");
        }
        Err(e) => {
            tracing::error!("failed to initialize persistence store: {e}");
            // Continue without persistence -- the server still works.
        }
    }

    // Auto-discover existing tmux sessions (upgrades stale sessions).
    {
        let supervisor = supervisor.clone();
        tokio::spawn(async move {
            match supervisor.discover_tmux_sessions().await {
                Ok(()) => tracing::info!("tmux session discovery complete"),
                Err(e) => tracing::error!("tmux discovery failed: {e}"),
            }
        });
    }

    // Start periodic persistence checkpoint (every 30s).
    supervisor.spawn_persistence_checkpoint();

    // Start thought generation loop.
    {
        let thought_tx = supervisor.thought_event_sender();
        let provider = Arc::new(SupervisorProvider::new(supervisor.clone()));
        let runner = ThoughtLoopRunner::new(config.thought_tick_ms, thought_tx);
        runner.spawn(provider);
    }

    // Build app state
    let state = Arc::new(AppState {
        supervisor,
        config: config.clone(),
    });

    // Build router
    let app = Router::new()
        .merge(api::api_router(config.clone()))
        .fallback_service(ServeDir::new("dist").append_index_html_on_directories(true))
        .with_state(state)
        .merge(metrics::endpoint::metrics_router(prom_handle));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("failed to bind");

    tracing::info!("Throngterm running on http://0.0.0.0:{port}");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
