mod api;
mod auth;
mod config;
mod metrics;
mod native;
mod persistence;
mod realtime;
mod repo_theme;
mod scroll;
mod session;
mod sprites;
mod state;
mod thought;
mod types;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use api::AppState;
use config::{Config, ThoughtBackend};
use persistence::file_store::FileStore;
use session::supervisor::{SessionSupervisor, SupervisorProvider};
use thought::bridge_runner::BridgeRunner;
use thought::emitter_client::{fetch_daemon_defaults, EmitterClient};
use thought::loop_runner::ThoughtLoopRunner;
use thought::runtime_config::ThoughtConfig;

const STARTUP_PHASE_WARN_THRESHOLD: Duration = Duration::from_secs(2);

fn log_startup_phase_complete(phase: &'static str, started: Instant) {
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;
    if elapsed >= STARTUP_PHASE_WARN_THRESHOLD {
        tracing::warn!(phase, elapsed_ms, "startup phase completed slowly");
    } else {
        tracing::info!(phase, elapsed_ms, "startup phase completed");
    }
}

#[tokio::main]
async fn main() {
    let startup_started = Instant::now();

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

    // Query clawgs for resolved daemon defaults (model, prompts).
    tracing::info!(phase = "clawgs_defaults", "startup phase begin");
    let daemon_defaults_started = Instant::now();
    let daemon_defaults = fetch_daemon_defaults().await;
    log_startup_phase_complete("clawgs_defaults", daemon_defaults_started);
    if daemon_defaults.is_some() {
        tracing::info!("loaded daemon defaults from clawgs");
    } else {
        tracing::info!("continuing without daemon defaults from clawgs");
    }

    // Create session supervisor (new() returns Arc<Self>)
    let supervisor = SessionSupervisor::new(config.clone());
    let thought_config = Arc::new(RwLock::new(ThoughtConfig::default()));
    let mut persistence_store: Option<Arc<FileStore>> = None;

    // Initialize persistence store.
    tracing::info!(phase = "persistence_init", "startup phase begin");
    let persistence_started = Instant::now();
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
    log_startup_phase_complete("persistence_init", persistence_started);

    // Auto-discover existing tmux sessions (upgrades stale sessions) before
    // serving requests, so bootstrap doesn't race against startup discovery.
    tracing::info!(phase = "tmux_startup_discovery", "startup phase begin");
    let discovery_started = Instant::now();
    match supervisor.discover_tmux_sessions().await {
        Ok(()) => tracing::info!("tmux session discovery complete"),
        Err(e) => tracing::error!("tmux discovery failed: {e}"),
    }
    log_startup_phase_complete("tmux_startup_discovery", discovery_started);

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
        daemon_defaults,
        file_store: persistence_store,
        published_selection: Arc::new(RwLock::new(api::PublishedSelectionState::default())),
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

    tracing::info!(
        elapsed_ms = startup_started.elapsed().as_millis() as u64,
        "startup complete; listener ready"
    );
    tracing::info!("Throngterm running on http://0.0.0.0:{port}");

    axum::serve(listener, app).await.expect("server error");
}
