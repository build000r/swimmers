mod api;
mod auth;
mod cli;
mod config;
mod env_bootstrap;
mod host_actions;
mod metrics;
mod native;
mod openrouter_models;
mod persistence;
mod repo_theme;
mod scroll;
mod session;
mod state;
#[cfg(test)]
mod test_support;
mod thought;
mod thought_ui;
mod tmux_target;
mod types;
mod web;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use clap::Parser;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use api::AppState;
use cli::{ConfigAction, ServerCli, ServerCommand};
use config::{Config, ThoughtBackend};
use persistence::file_store::FileStore;
use session::supervisor::{SessionSupervisor, SupervisorProvider};
use thought::bridge_runner::BridgeRunner;
use thought::emitter_client::{fetch_daemon_defaults, EmitterClient};
use thought::loop_runner::ThoughtLoopRunner;
use thought::protocol::SyncRequestSequence;
use thought::runtime_config::ThoughtConfig;

const STARTUP_PHASE_WARN_THRESHOLD: Duration = Duration::from_secs(2);

fn resolve_data_dir() -> std::path::PathBuf {
    if let Ok(val) = std::env::var("SWIMMERS_DATA_DIR") {
        if !val.is_empty() {
            return std::path::PathBuf::from(val);
        }
    }
    match dirs::data_dir() {
        Some(base) => base.join("swimmers"),
        None => {
            tracing::warn!(
                "dirs::data_dir() returned None (HOME may be unset); \
                 falling back to ./data/swimmers/"
            );
            std::path::PathBuf::from("./data/swimmers/")
        }
    }
}

fn log_startup_phase_complete(phase: &'static str, started: Instant) {
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;
    if elapsed >= STARTUP_PHASE_WARN_THRESHOLD {
        tracing::warn!(phase, elapsed_ms, "startup phase completed slowly");
    } else {
        tracing::info!(phase, elapsed_ms, "startup phase completed");
    }
}

async fn init_persistence_store(
    supervisor: &Arc<SessionSupervisor>,
    thought_config: &Arc<RwLock<ThoughtConfig>>,
) -> Option<Arc<FileStore>> {
    tracing::info!(phase = "persistence_init", "startup phase begin");
    let persistence_started = Instant::now();
    let data_dir = resolve_data_dir();
    tracing::info!(data_dir = %data_dir.display(), "using data dir");
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        tracing::error!(error = %err, dir = %data_dir.display(), "failed to create data dir");
    }
    let store = match FileStore::new(&data_dir).await {
        Ok(store) => {
            supervisor.init_persistence(store.clone()).await;
            let loaded_config = store.load_thought_config().await;
            {
                let mut runtime_config = thought_config.write().await;
                *runtime_config = loaded_config;
            }
            tracing::info!("persistence store initialized");
            Some(store)
        }
        Err(e) => {
            tracing::error!("failed to initialize persistence store: {e}");
            None
        }
    };
    log_startup_phase_complete("persistence_init", persistence_started);
    store
}

async fn run_startup_tmux_discovery(supervisor: &Arc<SessionSupervisor>) {
    tracing::info!(phase = "tmux_startup_discovery", "startup phase begin");
    let discovery_started = Instant::now();
    match supervisor.discover_tmux_sessions().await {
        Ok(()) => tracing::info!("tmux session discovery complete"),
        Err(e) => tracing::error!("tmux discovery failed: {e}"),
    }
    log_startup_phase_complete("tmux_startup_discovery", discovery_started);
}

fn start_thought_backend(
    config: &Arc<Config>,
    supervisor: &Arc<SessionSupervisor>,
    thought_config: Arc<RwLock<ThoughtConfig>>,
    sync_request_sequence: Arc<SyncRequestSequence>,
) {
    let thought_tx = supervisor.thought_event_sender();
    let provider = Arc::new(SupervisorProvider::new(supervisor.clone()));
    match config.thought_backend {
        ThoughtBackend::Inproc => {
            tracing::warn!("thought backend=inproc is deprecated; using daemon compatibility shim");
            let runner = ThoughtLoopRunner::with_runtime_config(
                config.thought_tick_ms,
                thought_tx,
                thought_config,
                sync_request_sequence,
            );
            runner.spawn(provider);
        }
        ThoughtBackend::Daemon => {
            tracing::info!("thought backend=daemon: starting thought bridge runner");
            let bridge_runner = BridgeRunner::with_tick(
                thought_tx,
                Duration::from_millis(config.thought_tick_ms),
                thought_config,
            );
            bridge_runner.spawn(
                provider,
                EmitterClient::with_request_sequence(sync_request_sequence),
            );
        }
    }
}

fn build_app_router(
    config: Arc<Config>,
    state: Arc<AppState>,
    prom_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> Router {
    Router::new()
        .merge(web::routes())
        .merge(api::api_router(config))
        .merge(metrics::endpoint::metrics_router(prom_handle))
        .with_state(state)
}

async fn bind_listener(addr: &str, port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(format!("{addr}:{port}"))
        .await
        .map_err(|err| anyhow::anyhow!("failed to bind listener: {err}"))
}

async fn run() -> anyhow::Result<()> {
    let startup_started = Instant::now();

    // Load .env before anything reads env vars.
    let _ = dotenvy::dotenv();
    env_bootstrap::bootstrap_provider_env_from_shell();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Initialize Prometheus metrics exporter
    let prom_handle = metrics::init_metrics();

    let config = Config::from_env();

    // Refuse to start if LocalTrust auth is paired with a non-loopback bind.
    // The pre-clap version only emitted a stderr warning here, which the
    // README's own external-access example silently relied on; that left
    // the API exposed to the network with no auth. Now we exit with
    // sysexits EX_CONFIG instead.
    if let Err(msg) = cli::enforce_localtrust_loopback(&config) {
        eprintln!("swimmers: {msg}");
        std::process::exit(cli::EXIT_CONFIG);
    }

    let port = config.port;
    let bind = config.bind.clone();
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
    let sync_request_sequence = Arc::new(SyncRequestSequence::new());
    let persistence_store = init_persistence_store(&supervisor, &thought_config).await;
    run_startup_tmux_discovery(&supervisor).await;

    // Start periodic persistence checkpoint (every 30s).
    supervisor.spawn_persistence_checkpoint();
    supervisor.spawn_process_exit_reaper();

    // Start thought engine.
    start_thought_backend(
        &config,
        &supervisor,
        thought_config.clone(),
        sync_request_sequence.clone(),
    );

    // Build app state
    let state = Arc::new(AppState {
        supervisor,
        config: config.clone(),
        thought_config,
        native_desktop_app: Arc::new(RwLock::new(native::default_native_app())),
        ghostty_open_mode: Arc::new(RwLock::new(native::default_ghostty_open_mode())),
        sync_request_sequence,
        daemon_defaults,
        file_store: persistence_store,
        published_selection: Arc::new(RwLock::new(api::PublishedSelectionState::default())),
    });

    let app = build_app_router(config.clone(), state, prom_handle);
    let listener = bind_listener(&bind, port).await?;

    tracing::info!(
        elapsed_ms = startup_started.elapsed().as_millis() as u64,
        "startup complete; listener ready"
    );
    tracing::info!("Swimmers running on http://{bind}:{port}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|err| anyhow::anyhow!("server error: {err}"))?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl-C handler: {err}");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => {
                tracing::error!("failed to install SIGTERM handler: {err}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("received shutdown signal; draining");
}

fn run_config_subcommand(action: Option<ConfigAction>) -> i32 {
    // Load .env so subcommands see the same environment the server would.
    let _ = dotenvy::dotenv();

    match action {
        None => {
            cli::print_config_table();
            0
        }
        Some(ConfigAction::Doctor) => {
            let config = Config::from_env();
            let tmux_present = cli::tmux_on_path();
            let data_dir = resolve_data_dir();
            let data_dir_writable = cli::check_data_dir_writable(&data_dir);
            let findings = cli::run_doctor_checks(&config, tmux_present, data_dir_writable);
            cli::print_doctor_findings(&findings)
        }
    }
}

fn main() {
    let cli_args = ServerCli::parse();
    match cli_args.command {
        None => {
            // Default behavior: run the API server.
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    eprintln!("swimmers: failed to build tokio runtime: {err}");
                    std::process::exit(1);
                }
            };
            if let Err(err) = runtime.block_on(run()) {
                tracing::error!("{err}");
                std::process::exit(1);
            }
        }
        Some(ServerCommand::Config { action }) => {
            std::process::exit(run_config_subcommand(action));
        }
    }
}
