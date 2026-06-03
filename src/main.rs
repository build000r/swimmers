use std::future::IntoFuture;
use std::path::Path;
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
use swimmers::cli::{self, ConfigAction, ServerCli, ServerCommand, TmuxAction};
use swimmers::config::Config;
use swimmers::{env_bootstrap, metrics, startup};

// 10s gives in-flight requests time to finish while preventing indefinite hangs.
const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
// 5s aligns with the existing persistence drain window in startup shutdown flow.
const SHUTDOWN_PERSIST_TIMEOUT: Duration = Duration::from_secs(5);

enum ShutdownTrigger {
    Signal(&'static str),
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
async fn wait_for_shutdown_trigger() -> ShutdownTrigger {
    shutdown_signal_from(wait_for_sigint(), wait_for_sigterm()).await
}

#[cfg(unix)]
async fn wait_for_sigint() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!("failed to install SIGINT handler: {err}");
        std::future::pending::<()>().await;
    }
}

#[cfg(unix)]
async fn wait_for_sigterm() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut sigterm) => {
            sigterm.recv().await;
        }
        Err(err) => {
            tracing::error!("failed to install SIGTERM handler: {err}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal_from<S, T>(sigint: S, sigterm: T) -> ShutdownTrigger
where
    S: std::future::Future<Output = ()>,
    T: std::future::Future<Output = ()>,
{
    tokio::pin!(sigint);
    tokio::pin!(sigterm);
    tokio::select! {
        _ = &mut sigint => ShutdownTrigger::Signal("SIGINT"),
        _ = &mut sigterm => ShutdownTrigger::Signal("SIGTERM"),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_trigger() -> ShutdownTrigger {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!("failed to install SIGINT handler: {err}");
        std::future::pending::<()>().await;
    }
    ShutdownTrigger::Signal("SIGINT")
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

    let (state, thought_backend, _bridge_health) = startup::init_app_state(config.clone()).await;
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
        trigger = wait_for_shutdown_trigger() => {
            log_shutdown_trigger(trigger);
            drain_server_task(shutdown_tx, &mut server_task).await
        }
    };

    finalize_persistence_shutdown(&state, thought_backend).await?;
    server_result?;

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::future::pending;
    use std::time::Duration;

    #[tokio::test]
    async fn shutdown_trigger_returns_sigint_future() {
        let trigger = tokio::time::timeout(
            Duration::from_millis(25),
            shutdown_signal_from(async {}, pending::<()>()),
        )
        .await
        .expect("sigint future should complete shutdown wait");

        assert!(matches!(trigger, ShutdownTrigger::Signal("SIGINT")));
    }

    #[tokio::test]
    async fn shutdown_trigger_returns_sigterm_future() {
        let trigger = tokio::time::timeout(
            Duration::from_millis(25),
            shutdown_signal_from(pending::<()>(), async {}),
        )
        .await
        .expect("sigterm future should complete shutdown wait");

        assert!(matches!(trigger, ShutdownTrigger::Signal("SIGTERM")));
    }

    #[tokio::test]
    async fn shutdown_trigger_ignores_other_internal_conditions() {
        let result = tokio::time::timeout(
            Duration::from_millis(25),
            shutdown_signal_from(pending::<()>(), pending::<()>()),
        )
        .await;

        assert!(
            result.is_err(),
            "the standalone HTTP API should only stop for process signals"
        );
    }
}

fn run_config_subcommand(action: Option<ConfigAction>) -> i32 {
    // Load .env so subcommands see the same environment the server would.
    let _ = dotenvy::dotenv();

    match action {
        None => {
            let load = Config::from_env_report();
            cli::print_config_table_for_load(&load);
            cli::print_config_diagnostics(&load.diagnostics);
            if load.has_errors() {
                1
            } else {
                0
            }
        }
        Some(ConfigAction::Doctor) => {
            let load = Config::from_env_report();
            let tmux_present = cli::tmux_on_path();
            let clawgs_defaults = cli::check_clawgs_defaults();
            let data_dir = startup::resolve_data_dir();
            let data_dir_writable = cli::check_data_dir_writable(&data_dir);
            let mut findings = cli::config_diagnostic_findings(&load.diagnostics);
            findings.extend(cli::run_doctor_checks(
                &load.config,
                tmux_present,
                clawgs_defaults,
                data_dir_writable,
            ));
            cli::print_doctor_findings(&findings)
        }
    }
}

fn run_tmux_subcommand(action: TmuxAction) -> i32 {
    run_tmux_subcommand_with(
        action,
        cli::next_numeric_tmux_name,
        cli::create_numbered_tmux_session,
        cli::attach_tmux_session,
        |line| println!("{line}"),
        |line| eprintln!("{line}"),
    )
}

fn run_tmux_subcommand_with<NextName, CreateSession, AttachSession, Stdout, Stderr>(
    action: TmuxAction,
    mut next_name: NextName,
    mut create_session: CreateSession,
    mut attach_session: AttachSession,
    mut stdout: Stdout,
    mut stderr: Stderr,
) -> i32
where
    NextName: FnMut() -> Result<String, String>,
    CreateSession: FnMut(Option<&Path>) -> Result<String, String>,
    AttachSession: FnMut(&str) -> Result<i32, String>,
    Stdout: FnMut(&str),
    Stderr: FnMut(&str),
{
    match action {
        TmuxAction::NextName => run_tmux_next_name(&mut next_name, &mut stdout, &mut stderr),
        TmuxAction::New { cwd } => run_tmux_new(
            cwd.as_deref(),
            &mut create_session,
            &mut attach_session,
            &mut stderr,
        ),
    }
}

fn run_tmux_next_name(
    next_name: &mut impl FnMut() -> Result<String, String>,
    stdout: &mut impl FnMut(&str),
    stderr: &mut impl FnMut(&str),
) -> i32 {
    match next_name() {
        Ok(name) => {
            stdout(&name);
            0
        }
        Err(err) => emit_tmux_generic_error(&err, stderr),
    }
}

fn run_tmux_new(
    cwd: Option<&Path>,
    create_session: &mut impl FnMut(Option<&Path>) -> Result<String, String>,
    attach_session: &mut impl FnMut(&str) -> Result<i32, String>,
    stderr: &mut impl FnMut(&str),
) -> i32 {
    let name = match create_session(cwd) {
        Ok(name) => name,
        Err(err) => return emit_tmux_generic_error(&err, stderr),
    };

    attach_created_tmux_session(&name, attach_session, stderr)
}

fn attach_created_tmux_session(
    name: &str,
    attach_session: &mut impl FnMut(&str) -> Result<i32, String>,
    stderr: &mut impl FnMut(&str),
) -> i32 {
    match attach_session(name) {
        Ok(code) => code,
        Err(err) => {
            stderr(&format!(
                "swimmers: created tmux session {name}, but attach failed: {err}"
            ));
            1
        }
    }
}

fn emit_tmux_generic_error(err: &str, stderr: &mut impl FnMut(&str)) -> i32 {
    stderr(&format!("swimmers: {err}"));
    1
}

#[cfg(test)]
mod tmux_subcommand_tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn unused_next_name() -> Result<String, String> {
        panic!("next-name operation should not be called")
    }

    fn unused_create_session(_: Option<&Path>) -> Result<String, String> {
        panic!("create-session operation should not be called")
    }

    fn unused_attach_session(_: &str) -> Result<i32, String> {
        panic!("attach-session operation should not be called")
    }

    #[test]
    fn tmux_next_name_success_prints_name_and_returns_zero() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = run_tmux_subcommand_with(
            TmuxAction::NextName,
            || Ok("7".to_string()),
            unused_create_session,
            unused_attach_session,
            |line| stdout.push(line.to_string()),
            |line| stderr.push(line.to_string()),
        );

        assert_eq!(code, 0);
        assert_eq!(stdout, vec!["7"]);
        assert!(stderr.is_empty());
    }

    #[test]
    fn tmux_next_name_failure_prints_generic_error_and_returns_one() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = run_tmux_subcommand_with(
            TmuxAction::NextName,
            || Err("tmux unavailable".to_string()),
            unused_create_session,
            unused_attach_session,
            |line| stdout.push(line.to_string()),
            |line| stderr.push(line.to_string()),
        );

        assert_eq!(code, 1);
        assert!(stdout.is_empty());
        assert_eq!(stderr, vec!["swimmers: tmux unavailable"]);
    }

    #[test]
    fn tmux_new_success_creates_then_attaches_and_returns_attach_code() {
        let cwd = PathBuf::from("/tmp/project");
        let calls = RefCell::new(Vec::new());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = run_tmux_subcommand_with(
            TmuxAction::New {
                cwd: Some(cwd.clone()),
            },
            unused_next_name,
            |cwd_arg| {
                calls.borrow_mut().push(format!(
                    "create:{}",
                    cwd_arg.expect("cwd should be forwarded").display()
                ));
                Ok("8".to_string())
            },
            |name| {
                calls.borrow_mut().push(format!("attach:{name}"));
                Ok(23)
            },
            |line| stdout.push(line.to_string()),
            |line| stderr.push(line.to_string()),
        );

        assert_eq!(code, 23);
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
        assert_eq!(
            calls.into_inner(),
            vec![format!("create:{}", cwd.display()), "attach:8".to_string()]
        );
    }

    #[test]
    fn tmux_new_create_failure_prints_generic_error_and_returns_one() {
        let attached = RefCell::new(false);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = run_tmux_subcommand_with(
            TmuxAction::New { cwd: None },
            unused_next_name,
            |_| Err("cannot allocate".to_string()),
            |_| {
                *attached.borrow_mut() = true;
                Ok(0)
            },
            |line| stdout.push(line.to_string()),
            |line| stderr.push(line.to_string()),
        );

        assert_eq!(code, 1);
        assert!(stdout.is_empty());
        assert_eq!(stderr, vec!["swimmers: cannot allocate"]);
        assert!(!attached.into_inner());
    }

    #[test]
    fn tmux_new_attach_failure_prints_created_error_and_returns_one() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let code = run_tmux_subcommand_with(
            TmuxAction::New { cwd: None },
            unused_next_name,
            |_| Ok("9".to_string()),
            |_| Err("terminal refused".to_string()),
            |line| stdout.push(line.to_string()),
            |line| stderr.push(line.to_string()),
        );

        assert_eq!(code, 1);
        assert!(stdout.is_empty());
        assert_eq!(
            stderr,
            vec!["swimmers: created tmux session 9, but attach failed: terminal refused"]
        );
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
    let load = Config::from_env_report();
    cli::print_config_diagnostics(&load.diagnostics);

    // Refuse trusted auth modes on bind addresses outside their trust boundary.
    // The pre-clap version only emitted a stderr warning here, which the
    // README's own external-access example silently relied on; that left
    // the API exposed to the network with no auth. Now we exit with
    // sysexits EX_CONFIG instead.
    if let Err(msg) = cli::enforce_startup_config(&load.config, &load.diagnostics) {
        eprintln!("swimmers: {msg}");
        std::process::exit(cli::EXIT_CONFIG);
    }

    let config = load.config;
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
        Some(ServerCommand::Tmux { action }) => {
            std::process::exit(run_tmux_subcommand(action));
        }
    }
}
