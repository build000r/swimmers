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
use std::time::Duration;

use axum::Router;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use api::AppState;
use config::{Config, ThoughtBackend};
use persistence::file_store::FileStore;
use session::supervisor::{SessionSupervisor, SupervisorProvider};
use thought::bridge_runner::BridgeRunner;
use thought::emitter_client::EmitterClient;
use thought::loop_runner::ThoughtLoopRunner;
use thought::runtime_config::ThoughtConfig;

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
    let thought_config = Arc::new(RwLock::new(ThoughtConfig::default()));
    let mut persistence_store: Option<Arc<FileStore>> = None;

    // Initialize persistence store.
    match FileStore::new("./data/throngterm/").await {
        Ok(store) => {
            supervisor.init_persistence(store.clone()).await;
            let loaded_config = store.load_thought_config().await;
            {
                let mut runtime_config = thought_config.write().await;
                *runtime_config = loaded_config;
            }
            persistence_store = Some(store);
            tracing::info!("persistence store initialized");
        }
        Err(e) => {
            tracing::error!("failed to initialize persistence store: {e}");
            // Continue without persistence -- the server still works.
        }
    }

    // Auto-discover existing tmux sessions (upgrades stale sessions) before
    // serving requests, so bootstrap doesn't race against startup discovery.
    match supervisor.discover_tmux_sessions().await {
        Ok(()) => tracing::info!("tmux session discovery complete"),
        Err(e) => tracing::error!("tmux discovery failed: {e}"),
    }

    // Start periodic persistence checkpoint (every 30s).
    supervisor.spawn_persistence_checkpoint();
    supervisor.spawn_process_exit_reaper();

    // Start thought engine.
    {
        let thought_tx = supervisor.thought_event_sender();
        let provider = Arc::new(SupervisorProvider::new(supervisor.clone()));
        match config.thought_backend {
            ThoughtBackend::Inproc => {
                tracing::warn!(
                    "thought backend=inproc is deprecated; using daemon compatibility shim"
                );
                let runner = ThoughtLoopRunner::with_runtime_config(
                    config.thought_tick_ms,
                    thought_tx,
                    thought_config.clone(),
                );
                runner.spawn(provider);
            }
            ThoughtBackend::Daemon => {
                tracing::info!("thought backend=daemon: starting thought bridge runner");
                let bridge_runner = BridgeRunner::with_tick(
                    thought_tx,
                    Duration::from_millis(config.thought_tick_ms),
                    thought_config.clone(),
                );
                bridge_runner.spawn(provider, EmitterClient::new());
            }
        }
    }

    // Build app state
    let state = Arc::new(AppState {
        supervisor,
        config: config.clone(),
        thought_config,
        file_store: persistence_store,
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

    axum::serve(listener, app).await.expect("server error");
}
