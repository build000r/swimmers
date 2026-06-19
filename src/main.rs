use std::future::IntoFuture;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use axum::{middleware, Router};
use clap::Parser;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing_subscriber::EnvFilter;

use swimmers::api::AppState;
use swimmers::cli::{self, ConfigAction, ServerCli, ServerCommand, TmuxAction};
use swimmers::config::{Config, ConfigLoad};
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
    let metrics_router = authenticated_metrics_router(config.clone(), prom_handle);

    Router::new()
        .merge(swimmers::web::routes())
        .merge(swimmers::api::api_router(config))
        .merge(metrics_router)
        .with_state(state)
}

fn authenticated_metrics_router(
    config: Arc<Config>,
    prom_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> Router<Arc<AppState>> {
    swimmers::metrics::endpoint::metrics_router(prom_handle).layer(middleware::from_fn(
        move |request, next| swimmers::auth::auth_middleware(config.clone(), request, next),
    ))
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
    use std::net::SocketAddr;
    use std::time::Duration;

    use metrics_exporter_prometheus::PrometheusBuilder;
    use swimmers::config::AuthMode;

    fn metrics_auth_test_config(auth_mode: AuthMode) -> Arc<Config> {
        Arc::new(Config {
            auth_mode,
            auth_token: Some("operator-token".to_string()),
            observer_token: Some("observer-token".to_string()),
            ..Config::default()
        })
    }

    async fn spawn_metrics_auth_test_server(
        config: Arc<Config>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let state = startup::init_app_state_skeleton(config.clone());
        let prom_handle = PrometheusBuilder::new().build_recorder().handle();
        let app = build_app_router(config, state, prom_handle);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind metrics auth test server");
        let addr = listener.local_addr().expect("metrics auth test addr");
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("serve metrics auth test server");
        });

        (format!("http://{addr}"), handle)
    }

    async fn get_metrics(base_url: &str, bearer_token: Option<&str>) -> reqwest::Response {
        let mut request = reqwest::Client::new().get(format!("{base_url}/metrics"));
        if let Some(token) = bearer_token {
            request = request.bearer_auth(token);
        }
        request.send().await.expect("metrics response")
    }

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

    #[tokio::test]
    async fn metrics_endpoint_enforces_configured_auth() {
        let (base_url, handle) =
            spawn_metrics_auth_test_server(metrics_auth_test_config(AuthMode::Token)).await;

        let unauthenticated = get_metrics(&base_url, None).await;
        assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

        let invalid = get_metrics(&base_url, Some("wrong-token")).await;
        assert_eq!(invalid.status(), reqwest::StatusCode::UNAUTHORIZED);

        let operator = get_metrics(&base_url, Some("operator-token")).await;
        assert_eq!(operator.status(), reqwest::StatusCode::OK);
        assert!(
            operator
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/plain")),
            "operator metrics response should be Prometheus text"
        );

        let observer = get_metrics(&base_url, Some("observer-token")).await;
        assert_eq!(observer.status(), reqwest::StatusCode::OK);

        handle.abort();

        for auth_mode in [AuthMode::LocalTrust, AuthMode::TailnetTrust] {
            let (base_url, handle) =
                spawn_metrics_auth_test_server(metrics_auth_test_config(auth_mode)).await;
            let response = get_metrics(&base_url, None).await;
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            handle.abort();
        }
    }
}

fn run_config_subcommand(action: Option<ConfigAction>) -> i32 {
    // Load .env so subcommands see the same environment the server would.
    let _ = dotenvy::dotenv();

    match action {
        Some(ConfigAction::SshImport {
            dry_run,
            ssh_config,
        }) => run_config_ssh_import(dry_run, ssh_config),
        action => config_subcommand_runner(action)(Config::from_env_report()),
    }
}

type ConfigSubcommandRunner = fn(ConfigLoad) -> i32;
const CONFIG_SUBCOMMAND_RUNNERS: [ConfigSubcommandRunner; 2] =
    [run_config_report, run_config_doctor];

fn config_subcommand_runner(action: Option<ConfigAction>) -> ConfigSubcommandRunner {
    CONFIG_SUBCOMMAND_RUNNERS[matches!(action, Some(ConfigAction::Doctor)) as usize]
}

fn run_config_report(load: ConfigLoad) -> i32 {
    cli::print_config_table_for_load(&load);
    cli::print_config_diagnostics(&load.diagnostics);
    config_report_exit_code(load.has_errors())
}

fn config_report_exit_code(has_errors: bool) -> i32 {
    has_errors as i32
}

fn run_config_doctor(load: ConfigLoad) -> i32 {
    let tmux_present = cli::tmux_on_path();
    let clawgs_defaults = cli::check_clawgs_defaults();
    let data_dir = startup::resolve_data_dir();
    let data_dir_writable = cli::check_data_dir_writable(&data_dir);
    let remote_targets = swimmers::api::remote_sessions::remote_targets_health_snapshot();
    let findings = config_doctor_findings(
        &load,
        tmux_present,
        clawgs_defaults,
        data_dir_writable,
        remote_targets,
    );
    let printed_code = cli::print_doctor_findings(&findings);
    debug_assert_eq!(printed_code, config_doctor_exit_code(&findings));
    printed_code
}

fn run_config_ssh_import(dry_run: bool, ssh_config: Option<PathBuf>) -> i32 {
    if !dry_run {
        eprintln!("ssh-import is proposal-only; rerun with --dry-run");
        return 2;
    }

    let Some(path) = ssh_config.or_else(cli::default_ssh_config_path) else {
        eprintln!("ssh-import could not resolve ~/.ssh/config; pass --ssh-config <path>");
        return 1;
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) => {
            eprintln!("ssh-import failed to read {}: {err}", path.display());
            return 1;
        }
    };
    let report = cli::ssh_import_report_from_config(path.display().to_string(), &contents);
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(err) => {
            eprintln!("ssh-import failed to serialize report: {err}");
            1
        }
    }
}

fn config_doctor_findings(
    load: &ConfigLoad,
    tmux_present: bool,
    clawgs_defaults: Result<String, String>,
    data_dir_writable: Result<PathBuf, String>,
    remote_targets: swimmers::types::DependencyHealthSnapshot,
) -> Vec<cli::DoctorFinding> {
    let mut findings = cli::config_diagnostic_findings(&load.diagnostics);
    findings.extend(cli::run_doctor_checks(
        &load.config,
        tmux_present,
        clawgs_defaults,
        data_dir_writable,
    ));
    findings.push(cli::doctor_remote_targets_finding(&remote_targets));
    findings
}

fn config_doctor_exit_code(findings: &[cli::DoctorFinding]) -> i32 {
    findings.iter().any(|finding| !finding.ok) as i32
}

#[cfg(test)]
mod config_subcommand_tests {
    use super::*;
    use chrono::Utc;
    use swimmers::config::{ConfigDiagnostic, ConfigDiagnosticLevel};
    use swimmers::types::DependencyHealthSnapshot;

    fn config_load_with_diagnostics(diagnostics: Vec<ConfigDiagnostic>) -> ConfigLoad {
        ConfigLoad {
            config: Config::default(),
            diagnostics,
        }
    }

    fn config_warning() -> ConfigDiagnostic {
        ConfigDiagnostic {
            level: ConfigDiagnosticLevel::Warning,
            key: "PORT",
            message: "ignored for test".to_string(),
        }
    }

    fn config_error() -> ConfigDiagnostic {
        ConfigDiagnostic {
            level: ConfigDiagnosticLevel::Error,
            key: "AUTH_TOKEN",
            message: "missing for test".to_string(),
        }
    }

    fn doctor_finding(ok: bool, level: cli::DoctorLevel) -> cli::DoctorFinding {
        cli::DoctorFinding {
            ok,
            level,
            name: "test",
            detail: "test finding".to_string(),
        }
    }

    fn remote_targets_not_configured() -> DependencyHealthSnapshot {
        DependencyHealthSnapshot::not_configured(Utc::now()).with_detail("configured_targets", "0")
    }

    #[test]
    fn config_subcommand_none_selects_report_runner() {
        assert_eq!(
            config_subcommand_runner(None) as *const (),
            run_config_report as *const ()
        );
    }

    #[test]
    fn config_subcommand_doctor_selects_doctor_runner() {
        assert_eq!(
            config_subcommand_runner(Some(ConfigAction::Doctor)) as *const (),
            run_config_doctor as *const ()
        );
    }

    #[test]
    fn config_report_exit_code_returns_zero_without_errors() {
        assert_eq!(config_report_exit_code(false), 0);
    }

    #[test]
    fn config_report_exit_code_returns_one_with_errors() {
        assert_eq!(config_report_exit_code(true), 1);
    }

    #[test]
    fn config_doctor_exit_code_allows_warnings_and_successes() {
        let findings = [
            doctor_finding(true, cli::DoctorLevel::Ok),
            cli::DoctorFinding {
                ok: true,
                level: cli::DoctorLevel::Warn,
                name: "warning",
                detail: "warning finding".to_string(),
            },
        ];

        assert_eq!(config_doctor_exit_code(&findings), 0);
    }

    #[test]
    fn config_doctor_exit_code_fails_on_any_failed_finding() {
        let findings = [
            doctor_finding(true, cli::DoctorLevel::Ok),
            doctor_finding(false, cli::DoctorLevel::Fail),
        ];

        assert_eq!(config_doctor_exit_code(&findings), 1);
    }

    #[test]
    fn config_doctor_findings_keeps_config_diagnostics_before_doctor_checks() {
        let load = config_load_with_diagnostics(vec![config_warning(), config_error()]);

        let findings = config_doctor_findings(
            &load,
            true,
            Ok("clawgs defaults ok".to_string()),
            Ok(PathBuf::from("/tmp/swimmers-test")),
            remote_targets_not_configured(),
        );

        let names: Vec<_> = findings.iter().map(|finding| finding.name).collect();
        assert_eq!(
            names,
            vec![
                "config/env",
                "config/env",
                "auth/bind",
                "auth/token",
                "tmux",
                "clawgs",
                "data_dir",
                "remote_targets"
            ]
        );
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
