use std::future::IntoFuture;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use axum::Router;
use clap::Parser;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing_subscriber::EnvFilter;

use swimmers::api::AppState;
use swimmers::cli::{self, ConfigAction, ServerCli, ServerCommand};
use swimmers::config::Config;
use swimmers::thought::health::BridgeHealthState;
use swimmers::{env_bootstrap, metrics, startup};

// 10s gives in-flight requests time to finish while preventing indefinite hangs.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
// 5s aligns with the existing persistence drain window in startup shutdown flow.
const SHUTDOWN_PERSIST_TIMEOUT: Duration = Duration::from_secs(5);

enum ShutdownTrigger {
    Signal(&'static str),
    Bridge(String),
}

fn build_app_router(
    config: Arc<Config>,
    state: Arc<AppState>,
    prom_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> Router {
    Router::new()
        .merge(swimmers::web::routes())
        .merge(swimmers::api::api_router(config))
        .merge(swimmers::metrics::endpoint::metrics_router(prom_handle))
        .with_state(state)
}

#[cfg(unix)]
async fn wait_for_shutdown_trigger(bridge_health: Arc<BridgeHealthState>) -> ShutdownTrigger {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(sig) => sig,
        Err(err) => {
            tracing::error!("failed to install SIGINT handler: {err}");
            let reason = bridge_health.wait_for_shutdown_request().await;
            return ShutdownTrigger::Bridge(reason);
        }
    };
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(sig) => sig,
        Err(err) => {
            tracing::error!("failed to install SIGTERM handler: {err}");
            let reason = bridge_health.wait_for_shutdown_request().await;
            return ShutdownTrigger::Bridge(reason);
        }
    };

    tokio::select! {
        _ = sigint.recv() => ShutdownTrigger::Signal("SIGINT"),
        _ = sigterm.recv() => ShutdownTrigger::Signal("SIGTERM"),
        reason = bridge_health.wait_for_shutdown_request() => ShutdownTrigger::Bridge(reason),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_trigger(bridge_health: Arc<BridgeHealthState>) -> ShutdownTrigger {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => ShutdownTrigger::Signal("SIGINT"),
        reason = bridge_health.wait_for_shutdown_request() => ShutdownTrigger::Bridge(reason),
    }
}

async fn finalize_persistence_shutdown(
    state: &Arc<AppState>,
    thought_backend: JoinHandle<()>,
) -> anyhow::Result<()> {
    thought_backend.abort();
    let _ = thought_backend.await;
    state
        .supervisor
        .wait_for_pending_thought_persists(SHUTDOWN_PERSIST_TIMEOUT)
        .await;
    state.supervisor.persist_registry().await;
    if let Some(store) = state.current_file_store() {
        store.flush_barrier().await?;
    }
    Ok(())
}

fn map_server_join_result(
    result: Result<Result<(), std::io::Error>, tokio::task::JoinError>,
) -> anyhow::Result<()> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(anyhow!("server error: {err}")),
        Err(err) => Err(anyhow!("server join error: {err}")),
    }
}

fn log_shutdown_trigger(trigger: ShutdownTrigger) {
    match trigger {
        ShutdownTrigger::Signal(signal) => {
            tracing::info!(signal, "received shutdown signal");
        }
        ShutdownTrigger::Bridge(reason) => {
            tracing::error!(reason, "thought bridge requested process shutdown");
        }
    }
}

async fn drain_server_task(
    shutdown_tx: oneshot::Sender<()>,
    server_task: &mut JoinHandle<Result<(), std::io::Error>>,
) -> anyhow::Result<()> {
    let _ = shutdown_tx.send(());

    match timeout(SHUTDOWN_DRAIN_TIMEOUT, &mut *server_task).await {
        Ok(result) => map_server_join_result(result),
        Err(_) => {
            tracing::warn!("graceful shutdown timed out; forcing server task abort");
            server_task.abort();
            let _ = server_task.await;
            Err(anyhow!(
                "graceful shutdown exceeded {}s drain timeout",
                SHUTDOWN_DRAIN_TIMEOUT.as_secs()
            ))
        }
    }
}

async fn run_server_with_bounded_shutdown(
    config: Arc<Config>,
    prom_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> anyhow::Result<()> {
    let startup_started = Instant::now();
    let bind = config.bind.clone();
    let port = config.port;

    let (state, thought_backend, bridge_health) = startup::init_app_state(config.clone()).await;
    let app = build_app_router(config, state.clone(), prom_handle);
    let listener = tokio::net::TcpListener::bind(startup::listener_addr(&bind, port))
        .await
        .map_err(|err| anyhow!("failed to bind listener: {err}"))?;
    startup::signal_readiness();

    tracing::info!(
        elapsed_ms = startup_started.elapsed().as_millis() as u64,
        "startup complete; listener ready"
    );
    tracing::info!(
        "Swimmers running on http://{}",
        startup::listener_addr(&bind, port)
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let mut server_task = tokio::spawn(
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
            tracing::info!("received shutdown signal; draining");
        })
        .into_future(),
    );

    let server_result = tokio::select! {
        result = &mut server_task => {
            map_server_join_result(result)
        }
        trigger = wait_for_shutdown_trigger(bridge_health.clone()) => {
            log_shutdown_trigger(trigger);
            drain_server_task(shutdown_tx, &mut server_task).await
        }
    };

    finalize_persistence_shutdown(&state, thought_backend).await?;
    server_result?;

    if let Some(reason) = bridge_health.shutdown_reason() {
        return Err(anyhow!("thought bridge requested shutdown: {reason}"));
    }

    Ok(())
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
            let clawgs_defaults = cli::check_clawgs_defaults();
            let data_dir = startup::resolve_data_dir();
            let data_dir_writable = cli::check_data_dir_writable(&data_dir);
            let findings =
                cli::run_doctor_checks(&config, tmux_present, clawgs_defaults, data_dir_writable);
            cli::print_doctor_findings(&findings)
        }
    }
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

fn prepare_server_startup() -> (Arc<Config>, metrics_exporter_prometheus::PrometheusHandle) {
    // Load .env before anything reads env vars.
    let _ = dotenvy::dotenv();
    env_bootstrap::bootstrap_provider_env_from_shell();

    init_tracing();

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

    (Arc::new(config), prom_handle)
}

fn build_runtime_or_exit() -> tokio::runtime::Runtime {
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("swimmers: failed to build tokio runtime: {err}");
            std::process::exit(1);
        }
    }
}

fn run_server_command() {
    let (config, prom_handle) = prepare_server_startup();
    let runtime = build_runtime_or_exit();

    if let Err(err) = runtime.block_on(run_server_with_bounded_shutdown(config, prom_handle)) {
        tracing::error!("{err}");
        std::process::exit(1);
    }
}

fn main() {
    let cli_args = ServerCli::parse();
    match cli_args.command {
        None | Some(ServerCommand::Serve) => {
            run_server_command();
        }
        Some(ServerCommand::Config { action }) => {
            std::process::exit(run_config_subcommand(action));
        }
    }
}
