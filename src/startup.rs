use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::api::{once_lock_with, AppState};
use crate::config::{Config, ThoughtBackend};
use crate::host_actions;
use crate::metrics;
use crate::native;
use crate::persistence::file_store::FileStore;
use crate::session::supervisor::{SessionSupervisor, SupervisorProvider};
use crate::thought::bridge_runner::BridgeRunner;
use crate::thought::emitter_client::{fetch_daemon_defaults, EmitterClient};
use crate::thought::health::BridgeHealthState;
use crate::thought::loop_runner::ThoughtLoopRunner;
use crate::thought::protocol::SyncRequestSequence;
use crate::thought::runtime_config::{DaemonDefaults, ThoughtConfig};
use crate::{api, web};

const STARTUP_PHASE_WARN_THRESHOLD: Duration = Duration::from_secs(2);
const SHUTDOWN_PERSIST_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_REGISTRY_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TASK_ABORT_TIMEOUT: Duration = Duration::from_secs(1);
const EMBEDDED_DEFERRED_INIT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const FALLBACK_DATA_DIR: &str = "./data/swimmers/";

#[derive(Clone, Copy)]
struct ShutdownTimeouts {
    pending_persists: Duration,
    registry: Duration,
    flush: Duration,
    task_abort: Duration,
    embedded_deferred_init: Duration,
}

impl Default for ShutdownTimeouts {
    fn default() -> Self {
        Self {
            pending_persists: SHUTDOWN_PERSIST_TIMEOUT,
            registry: SHUTDOWN_REGISTRY_TIMEOUT,
            flush: SHUTDOWN_FLUSH_TIMEOUT,
            task_abort: SHUTDOWN_TASK_ABORT_TIMEOUT,
            embedded_deferred_init: EMBEDDED_DEFERRED_INIT_SHUTDOWN_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeferredInitShutdown {
    Completed,
    Aborted,
    Failed,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PersistenceShutdownOutcome {
    pending_thoughts_drained: bool,
    registry_persisted: bool,
    file_store_flushed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmbeddedTuiShutdownOutcome {
    deferred_init: DeferredInitShutdown,
    thought_backend_aborted: bool,
    persistence: PersistenceShutdownOutcome,
}

pub struct EmbeddedTuiShutdown {
    state: Arc<AppState>,
    deferred_init: Option<JoinHandle<()>>,
    thought_backend: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Drop for EmbeddedTuiShutdown {
    fn drop(&mut self) {
        if let Some(deferred_init) = self.deferred_init.take() {
            deferred_init.abort();
        }
        if let Ok(mut thought_backend) = self.thought_backend.lock() {
            if let Some(thought_backend) = thought_backend.take() {
                thought_backend.abort();
            }
        }
    }
}

pub fn resolve_data_dir() -> PathBuf {
    data_dir_from_env(std::env::var("SWIMMERS_DATA_DIR")).unwrap_or_else(platform_data_dir)
}

fn data_dir_from_env(value: Result<String, std::env::VarError>) -> Option<PathBuf> {
    value
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn platform_data_dir() -> PathBuf {
    data_dir_from_platform_base(dirs::data_dir())
}

fn data_dir_from_platform_base(base: Option<PathBuf>) -> PathBuf {
    base.map(|base| base.join("swimmers"))
        .unwrap_or_else(fallback_data_dir)
}

fn fallback_data_dir() -> PathBuf {
    tracing::warn!(
        "dirs::data_dir() returned None (HOME may be unset); \
         falling back to ./data/swimmers/"
    );
    PathBuf::from(FALLBACK_DATA_DIR)
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

async fn open_file_store_for_startup(data_dir: PathBuf) -> anyhow::Result<Arc<FileStore>> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| anyhow::anyhow!("failed to build file-store init runtime: {err}"))?;
        runtime.block_on(FileStore::new(data_dir))
    })
    .await
    .map_err(|err| anyhow::anyhow!("file-store init task failed: {err}"))?
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
    let store = match open_file_store_for_startup(data_dir.clone()).await {
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
) -> (JoinHandle<()>, Arc<BridgeHealthState>) {
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
            let bridge_health = runner.bridge_health();
            (runner.spawn(provider), bridge_health)
        }
        ThoughtBackend::Daemon => {
            tracing::info!("thought backend=daemon: starting thought bridge runner");
            let bridge_runner = BridgeRunner::with_tick(
                thought_tx,
                Duration::from_millis(config.thought_tick_ms),
                thought_config,
            );
            let bridge_health = bridge_runner.health();
            (
                bridge_runner.spawn(
                    provider,
                    EmitterClient::with_request_sequence(sync_request_sequence),
                ),
                bridge_health,
            )
        }
    }
}

async fn abort_task(
    mut handle: JoinHandle<()>,
    timeout: Duration,
    task_name: &'static str,
) -> bool {
    handle.abort();
    match tokio::time::timeout(timeout, &mut handle).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            if !err.is_cancelled() {
                tracing::warn!(task_name, error = %err, "shutdown task failed while aborting");
            }
            true
        }
        Err(_) => {
            tracing::warn!(
                task_name,
                timeout_ms = timeout.as_millis() as u64,
                "timed out waiting for shutdown task abort"
            );
            false
        }
    }
}

async fn run_persistence_shutdown_barrier(
    supervisor: &Arc<SessionSupervisor>,
    file_store: Option<Arc<FileStore>>,
    timeouts: ShutdownTimeouts,
) -> anyhow::Result<PersistenceShutdownOutcome> {
    let pending_thoughts_drained = supervisor
        .wait_for_pending_thought_persists(timeouts.pending_persists)
        .await;

    let registry_persisted =
        match tokio::time::timeout(timeouts.registry, supervisor.persist_registry()).await {
            Ok(()) => true,
            Err(_) => {
                tracing::warn!(
                    timeout_ms = timeouts.registry.as_millis() as u64,
                    "timed out persisting session registry during shutdown"
                );
                false
            }
        };

    let mut file_store_flushed = false;
    if let Some(store) = file_store {
        match tokio::time::timeout(timeouts.flush, store.flush_barrier()).await {
            Ok(Ok(())) => {
                file_store_flushed = true;
            }
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "timed out flushing persistence store during shutdown"
                ));
            }
        }
    }

    Ok(PersistenceShutdownOutcome {
        pending_thoughts_drained,
        registry_persisted,
        file_store_flushed,
    })
}

async fn finalize_shutdown(
    supervisor: &Arc<SessionSupervisor>,
    thought_backend: JoinHandle<()>,
    file_store: Option<Arc<FileStore>>,
) -> anyhow::Result<()> {
    let timeouts = ShutdownTimeouts::default();
    abort_task(
        thought_backend,
        timeouts.task_abort,
        "standalone_thought_backend",
    )
    .await;
    run_persistence_shutdown_barrier(supervisor, file_store, timeouts).await?;
    Ok(())
}

async fn await_or_abort_deferred_init(
    shutdown: &mut EmbeddedTuiShutdown,
    timeouts: ShutdownTimeouts,
) -> DeferredInitShutdown {
    let Some(mut deferred_init) = shutdown.deferred_init.take() else {
        return DeferredInitShutdown::Missing;
    };

    match tokio::time::timeout(timeouts.embedded_deferred_init, &mut deferred_init).await {
        Ok(Ok(())) => DeferredInitShutdown::Completed,
        Ok(Err(err)) => {
            if err.is_cancelled() {
                DeferredInitShutdown::Aborted
            } else {
                tracing::warn!(error = %err, "embedded deferred init failed during shutdown");
                DeferredInitShutdown::Failed
            }
        }
        Err(_) => {
            deferred_init.abort();
            let aborted =
                abort_task(deferred_init, timeouts.task_abort, "embedded_deferred_init").await;
            if aborted {
                DeferredInitShutdown::Aborted
            } else {
                DeferredInitShutdown::Failed
            }
        }
    }
}

async fn finalize_embedded_tui_shutdown_with_timeouts(
    mut shutdown: EmbeddedTuiShutdown,
    timeouts: ShutdownTimeouts,
) -> anyhow::Result<EmbeddedTuiShutdownOutcome> {
    let deferred_init = await_or_abort_deferred_init(&mut shutdown, timeouts).await;
    let thought_backend = shutdown
        .thought_backend
        .lock()
        .map(|mut guard| guard.take())
        .unwrap_or(None);
    let thought_backend_aborted = match thought_backend {
        Some(handle) => abort_task(handle, timeouts.task_abort, "embedded_thought_backend").await,
        None => false,
    };

    let persistence = run_persistence_shutdown_barrier(
        &shutdown.state.supervisor,
        shutdown.state.current_file_store(),
        timeouts,
    )
    .await?;

    Ok(EmbeddedTuiShutdownOutcome {
        deferred_init,
        thought_backend_aborted,
        persistence,
    })
}

pub async fn finalize_embedded_tui_shutdown(shutdown: EmbeddedTuiShutdown) -> anyhow::Result<()> {
    finalize_embedded_tui_shutdown_with_timeouts(shutdown, ShutdownTimeouts::default())
        .await
        .map(|_| ())
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

pub fn listener_addr(bind: &str, port: u16) -> String {
    // Tolerate `host:port`, `[ipv6]:port`, or bare-host inputs in `bind` and
    // always emit `host:port` with IPv6 literals correctly bracketed.
    let host = crate::cli::bind_host(bind);
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

async fn bind_listener(addr: &str, port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(listener_addr(addr, port))
        .await
        .map_err(|err| anyhow::anyhow!("failed to bind listener: {err}"))
}

pub fn signal_readiness() {
    if let Some(fd) = readiness_fd_from_env() {
        write_readiness_signal(fd);
    }
}

fn readiness_fd_from_env() -> Option<i32> {
    readiness_fd_raw().and_then(parse_readiness_fd)
}

fn readiness_fd_raw() -> Option<String> {
    readiness_fd_raw_from_result(std::env::var("SWIMMERS_READY_FD"))
}

fn readiness_fd_raw_from_result(value: Result<String, std::env::VarError>) -> Option<String> {
    value.map_or_else(
        |_| {
            tracing::trace!("SWIMMERS_READY_FD not set; skipping readiness signal");
            None
        },
        readiness_fd_raw_non_empty,
    )
}

fn readiness_fd_raw_non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        tracing::trace!("SWIMMERS_READY_FD is empty; skipping readiness signal");
        None
    } else {
        Some(value)
    }
}

fn parse_readiness_fd(fd_raw: String) -> Option<i32> {
    match fd_raw.parse::<i32>() {
        Ok(fd) => fd,
        Err(err) => {
            tracing::trace!(
                value = %fd_raw,
                error = %err,
                "SWIMMERS_READY_FD is not a valid i32; skipping readiness signal"
            );
            return None;
        }
    }
    .into()
}

#[cfg(unix)]
fn write_readiness_signal(fd: i32) {
    use std::os::fd::FromRawFd;

    // SAFETY: The launcher passes SWIMMERS_READY_FD as an owned pipe writer
    // fd intended for a one-shot readiness byte. We consume exactly once and
    // drop immediately to close the descriptor.
    let mut writer = unsafe { os_pipe::PipeWriter::from_raw_fd(fd) };
    match writer.write_all(b"R") {
        Ok(()) => {
            drop(writer);
            tracing::info!(fd, "sent readiness signal");
        }
        Err(err) => {
            tracing::warn!(fd, error = %err, "failed to write readiness signal");
        }
    }
}

#[cfg(not(unix))]
fn write_readiness_signal(fd: i32) {
    tracing::warn!(
        fd,
        "SWIMMERS_READY_FD signaling is only implemented on unix platforms"
    );
}

pub async fn init_app_state(
    config: Arc<Config>,
) -> (Arc<AppState>, JoinHandle<()>, Arc<BridgeHealthState>) {
    tracing::info!(phase = "clawgs_defaults", "startup phase begin");
    let daemon_defaults_started = Instant::now();
    let daemon_defaults = fetch_daemon_defaults().await;
    log_startup_phase_complete("clawgs_defaults", daemon_defaults_started);
    if daemon_defaults.is_some() {
        tracing::info!("loaded daemon defaults from clawgs");
    } else {
        tracing::info!("continuing without daemon defaults from clawgs");
    }

    let supervisor = SessionSupervisor::new(config.clone());
    let thought_config = Arc::new(RwLock::new(ThoughtConfig::default()));
    let sync_request_sequence = Arc::new(SyncRequestSequence::new());
    let persistence_store = init_persistence_store(&supervisor, &thought_config).await;
    run_startup_tmux_discovery(&supervisor).await;

    supervisor.spawn_persistence_checkpoint();
    supervisor.spawn_process_exit_reaper();
    supervisor.spawn_tmux_reconcile_loop();

    let (thought_backend, bridge_health) = start_thought_backend(
        &config,
        &supervisor,
        thought_config.clone(),
        sync_request_sequence.clone(),
    );

    let state = Arc::new(AppState {
        supervisor,
        config: config.clone(),
        thought_config,
        native_desktop_app: Arc::new(RwLock::new(native::default_native_app())),
        ghostty_open_mode: Arc::new(RwLock::new(native::default_ghostty_open_mode())),
        sync_request_sequence,
        daemon_defaults: once_lock_with(daemon_defaults),
        file_store: once_lock_with(persistence_store),
        bridge_health: bridge_health.clone(),
        published_selection: Arc::new(RwLock::new(api::PublishedSelectionState::default())),
        repo_actions: host_actions::RepoActionTracker::default(),
    });

    (state, thought_backend, bridge_health)
}

pub fn init_app_state_skeleton(config: Arc<Config>) -> Arc<AppState> {
    let supervisor = SessionSupervisor::new(config.clone());
    supervisor.spawn_persistence_checkpoint();
    supervisor.spawn_process_exit_reaper();
    supervisor.spawn_tmux_reconcile_loop();

    Arc::new(AppState {
        supervisor,
        config: config.clone(),
        thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
        native_desktop_app: Arc::new(RwLock::new(native::default_native_app())),
        ghostty_open_mode: Arc::new(RwLock::new(native::default_ghostty_open_mode())),
        sync_request_sequence: Arc::new(SyncRequestSequence::new()),
        daemon_defaults: once_lock_with(None),
        file_store: once_lock_with(None),
        bridge_health: Arc::new(BridgeHealthState::new_with_tick(Duration::from_millis(
            config.thought_tick_ms,
        ))),
        published_selection: Arc::new(RwLock::new(api::PublishedSelectionState::default())),
        repo_actions: host_actions::RepoActionTracker::default(),
    })
}

async fn run_deferred_phase_join<P, D, F, PFut, DFut, FFut, POut, DOut, FOut>(
    persistence_phase: P,
    discovery_phase: D,
    defaults_phase: F,
) -> (POut, DOut, FOut)
where
    P: FnOnce() -> PFut,
    D: FnOnce() -> DFut,
    F: FnOnce() -> FFut,
    PFut: std::future::Future<Output = POut>,
    DFut: std::future::Future<Output = DOut>,
    FFut: std::future::Future<Output = FOut>,
{
    tokio::join!(persistence_phase(), discovery_phase(), defaults_phase())
}

async fn run_deferred_init_phases(
    supervisor: Arc<SessionSupervisor>,
    thought_config: Arc<RwLock<ThoughtConfig>>,
) -> (Option<Arc<FileStore>>, (), Option<DaemonDefaults>) {
    let persistence_supervisor = supervisor.clone();
    let persistence_config = thought_config.clone();
    let discovery_supervisor = supervisor;

    run_deferred_phase_join(
        move || async move {
            init_persistence_store(&persistence_supervisor, &persistence_config).await
        },
        move || async move { run_startup_tmux_discovery(&discovery_supervisor).await },
        fetch_daemon_defaults,
    )
    .await
}

fn log_deferred_attachment(subject: &'static str, attached: bool) {
    if attached {
        tracing::info!("deferred init attached {subject} to AppState");
    } else {
        tracing::info!("deferred init found AppState {subject} already initialized");
    }
}

fn attach_deferred_file_store(state: &Arc<AppState>, persistence_store: Option<Arc<FileStore>>) {
    if let Some(store) = persistence_store {
        log_deferred_attachment("persistence store", state.set_file_store(store));
    }
}

fn attach_deferred_daemon_defaults(state: &Arc<AppState>, daemon_defaults: Option<DaemonDefaults>) {
    if let Some(defaults) = daemon_defaults {
        log_deferred_attachment("daemon defaults", state.set_daemon_defaults(defaults));
    }
}

fn attach_deferred_init_results(
    state: &Arc<AppState>,
    persistence_store: Option<Arc<FileStore>>,
    daemon_defaults: Option<DaemonDefaults>,
) {
    attach_deferred_file_store(state, persistence_store);
    attach_deferred_daemon_defaults(state, daemon_defaults);
}

fn start_deferred_thought_backend(state: &Arc<AppState>) -> JoinHandle<()> {
    let thought_tx = state.supervisor.thought_event_sender();
    let provider = Arc::new(SupervisorProvider::new(state.supervisor.clone()));

    match state.config.thought_backend {
        ThoughtBackend::Inproc => {
            tracing::warn!("thought backend=inproc is deprecated; using daemon compatibility shim");
            BridgeRunner::with_existing_health(
                thought_tx,
                state.thought_config.clone(),
                state.bridge_health.clone(),
            )
            .spawn(
                provider,
                EmitterClient::with_request_sequence(state.sync_request_sequence.clone()),
            )
        }
        ThoughtBackend::Daemon => {
            tracing::info!("thought backend=daemon: starting deferred thought bridge runner");
            BridgeRunner::with_existing_health(
                thought_tx,
                state.thought_config.clone(),
                state.bridge_health.clone(),
            )
            .spawn(
                provider,
                EmitterClient::with_request_sequence(state.sync_request_sequence.clone()),
            )
        }
    }
}

fn spawn_deferred_init_task(
    state: Arc<AppState>,
    thought_backend_slot: Option<Arc<Mutex<Option<JoinHandle<()>>>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(phase = "deferred_init", "startup phase begin");
        let deferred_started = Instant::now();

        let (persistence_store, (), daemon_defaults) =
            run_deferred_init_phases(state.supervisor.clone(), state.thought_config.clone()).await;

        log_startup_phase_complete("deferred_init", deferred_started);
        attach_deferred_init_results(&state, persistence_store, daemon_defaults);
        let thought_backend = start_deferred_thought_backend(&state);
        if let Some(slot) = thought_backend_slot {
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(thought_backend);
            } else {
                thought_backend.abort();
            }
        } else {
            drop(thought_backend);
        }
    })
}

pub fn spawn_deferred_init(state: Arc<AppState>) -> JoinHandle<()> {
    spawn_deferred_init_task(state, None)
}

pub fn spawn_deferred_init_for_embedded_tui(state: Arc<AppState>) -> EmbeddedTuiShutdown {
    let thought_backend = Arc::new(Mutex::new(None));
    let deferred_init = spawn_deferred_init_task(state.clone(), Some(thought_backend.clone()));
    EmbeddedTuiShutdown {
        state,
        deferred_init: Some(deferred_init),
        thought_backend,
    }
}

pub async fn run_server(
    config: Arc<Config>,
    prom_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> anyhow::Result<()> {
    let startup_started = Instant::now();
    let port = config.port;
    let bind = config.bind.clone();

    let (state, thought_backend, _bridge_health) = init_app_state(config.clone()).await;
    let supervisor = state.supervisor.clone();
    let file_store = state.current_file_store();
    let app = build_app_router(config, state, prom_handle);
    let listener = bind_listener(&bind, port).await?;
    signal_readiness();

    tracing::info!(
        elapsed_ms = startup_started.elapsed().as_millis() as u64,
        "startup complete; listener ready"
    );
    tracing::info!("Swimmers running on http://{}", listener_addr(&bind, port));

    let server_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;

    finalize_shutdown(&supervisor, thought_backend, file_store).await?;

    server_result.map_err(|err| anyhow::anyhow!("server error: {err}"))?;
    Ok(())
}

async fn shutdown_signal() {
    shutdown_signal_from(wait_for_ctrl_c_signal(), wait_for_terminate_signal()).await;
}

async fn wait_for_ctrl_c_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!("failed to install Ctrl-C handler: {err}");
    }
}

#[cfg(unix)]
async fn wait_for_terminate_signal() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut sig) => {
            sig.recv().await;
        }
        Err(err) => {
            tracing::error!("failed to install SIGTERM handler: {err}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_terminate_signal() {
    std::future::pending::<()>().await
}

async fn shutdown_signal_from<C, T>(ctrl_c: C, terminate: T)
where
    C: std::future::Future<Output = ()>,
    T: std::future::Future<Output = ()>,
{
    tokio::pin!(ctrl_c);
    tokio::pin!(terminate);

    tokio::select! {
        _ = &mut ctrl_c => {},
        _ = &mut terminate => {},
    }

    tracing::info!("received shutdown signal; draining");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::Utc;
    use tokio::sync::mpsc;

    use crate::session::actor::{ActorHandle, SessionCommand};
    use crate::thought::health::BridgeTiming;
    use crate::thought::loop_runner::SessionProvider;
    use crate::thought::protocol::ThoughtDeliveryState;
    use crate::types::{
        fallback_rest_state, RestState, SessionState, SessionSummary, ThoughtSource, ThoughtState,
        TransportHealth,
    };

    struct AbortFlag(Arc<AtomicBool>);

    impl Drop for AbortFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn listener_addr_brackets_ipv6_literals() {
        assert_eq!(listener_addr("127.0.0.1", 3210), "127.0.0.1:3210");
        assert_eq!(listener_addr("::1", 3210), "[::1]:3210");
        assert_eq!(listener_addr("[::1]", 3210), "[::1]:3210");
    }

    #[test]
    fn listener_addr_strips_port_from_host_port_inputs() {
        // Operators commonly export SWIMMERS_BIND="host:port" even though
        // the docs say "interface only" — the resulting socket address must
        // still parse, not become double-bracketed gibberish.
        assert_eq!(listener_addr("127.0.0.1:3210", 3210), "127.0.0.1:3210");
        assert_eq!(listener_addr("127.0.0.1:9999", 3210), "127.0.0.1:3210");
        assert_eq!(listener_addr("[::1]:9999", 3210), "[::1]:3210");
    }

    #[test]
    fn listener_addr_trims_whitespace_around_input() {
        assert_eq!(listener_addr("  127.0.0.1  ", 3210), "127.0.0.1:3210");
        assert_eq!(listener_addr("\t[::1]\n", 3210), "[::1]:3210");
    }

    #[test]
    fn resolve_data_dir_env_value_wins_when_non_empty() {
        let data_dir = PathBuf::from("custom-swimmers-data");

        assert_eq!(
            data_dir_from_env(Ok(data_dir.to_string_lossy().into_owned())),
            Some(data_dir)
        );
    }

    #[test]
    fn resolve_data_dir_env_empty_or_missing_is_ignored() {
        assert_eq!(data_dir_from_env(Ok(String::new())), None);
        assert_eq!(data_dir_from_env(Err(std::env::VarError::NotPresent)), None);
    }

    #[test]
    fn resolve_data_dir_platform_base_appends_swimmers() {
        let platform_base = PathBuf::from("platform-data");

        assert_eq!(
            data_dir_from_platform_base(Some(platform_base.clone())),
            platform_base.join("swimmers")
        );
    }

    #[test]
    fn resolve_data_dir_local_fallback_matches_documented_path() {
        assert_eq!(
            data_dir_from_platform_base(None),
            PathBuf::from(FALLBACK_DATA_DIR)
        );
    }

    #[test]
    fn readiness_fd_raw_keeps_non_empty_env_value() {
        assert_eq!(
            readiness_fd_raw_from_result(Ok("  42  ".to_string())).as_deref(),
            Some("  42  ")
        );
    }

    #[test]
    fn readiness_fd_raw_ignores_missing_or_blank_env_value() {
        assert_eq!(readiness_fd_raw_from_result(Ok(" \t\n ".to_string())), None);
        assert_eq!(
            readiness_fd_raw_from_result(Err(std::env::VarError::NotPresent)),
            None
        );
    }

    #[test]
    fn readiness_fd_parser_accepts_i32_and_rejects_invalid_values() {
        assert_eq!(parse_readiness_fd("7".to_string()), Some(7));
        assert_eq!(parse_readiness_fd("-1".to_string()), Some(-1));
        assert_eq!(parse_readiness_fd(" 7 ".to_string()), None);
        assert_eq!(parse_readiness_fd("not-a-fd".to_string()), None);
    }

    async fn spawn_summary_handle(summary: SessionSummary) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(
            summary.session_id.clone(),
            summary.tmux_name.clone(),
            cmd_tx,
        );
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    #[tokio::test]
    async fn finalize_shutdown_aborts_backend_drains_pending_persists_and_flushes_registry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor.init_persistence(store.clone()).await;

        let summary = SessionSummary {
            session_id: "sess_1".to_string(),
            tmux_name: "work".to_string(),
            state: SessionState::Idle,
            current_command: Some("cargo test".to_string()),
            state_evidence: Default::default(),
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: fallback_rest_state(SessionState::Idle, ThoughtState::Holding),
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
            environment: Default::default(),
        };
        supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;

        let provider = Arc::new(SupervisorProvider::new(supervisor.clone()));
        provider.persist_thought(
            "sess_1",
            Some("queued thought"),
            17,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            true,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        );

        let aborted = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let task_provider = provider.clone();
        let task_aborted = aborted.clone();
        let thought_backend = tokio::spawn(async move {
            let _flag = AbortFlag(task_aborted);
            let _ = started_tx.send(());
            let _provider = task_provider;
            pending::<()>().await;
        });
        drop(provider);
        started_rx.await.expect("backend task should start");

        finalize_shutdown(&supervisor, thought_backend, Some(store.clone()))
            .await
            .expect("shutdown flush");

        assert!(aborted.load(Ordering::SeqCst));

        let thoughts = store.load_thoughts().await;
        let thought = thoughts.get("sess_1").expect("persisted thought");
        assert_eq!(thought.thought.as_deref(), Some("queued thought"));
        assert_eq!(thought.objective_fingerprint.as_deref(), Some("obj-1"));

        let sessions = store.load_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess_1");
        assert_eq!(sessions[0].thought.as_deref(), Some("queued thought"));
        assert_eq!(sessions[0].objective_fingerprint.as_deref(), Some("obj-1"));
    }

    #[tokio::test]
    async fn embedded_shutdown_cancels_deferred_init_and_runs_persistence_barrier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");
        let state = init_app_state_skeleton(Arc::new(Config::default()));
        state.supervisor.init_persistence(store.clone()).await;
        assert!(state.set_file_store(store.clone()));

        let summary = SessionSummary {
            session_id: "sess_1".to_string(),
            tmux_name: "work".to_string(),
            state: SessionState::Idle,
            current_command: Some("cargo test".to_string()),
            state_evidence: Default::default(),
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: fallback_rest_state(SessionState::Idle, ThoughtState::Holding),
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
            environment: Default::default(),
        };
        state
            .supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;

        let provider = Arc::new(SupervisorProvider::new(state.supervisor.clone()));
        provider.persist_thought(
            "sess_1",
            Some("embedded queued thought"),
            17,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            true,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            Some("embedded-obj".to_string()),
        );

        let deferred_aborted = Arc::new(AtomicBool::new(false));
        let deferred_flag = deferred_aborted.clone();
        let deferred_init = tokio::spawn(async move {
            let _flag = AbortFlag(deferred_flag);
            pending::<()>().await;
        });

        let backend_aborted = Arc::new(AtomicBool::new(false));
        let backend_flag = backend_aborted.clone();
        let thought_backend = tokio::spawn(async move {
            let _flag = AbortFlag(backend_flag);
            pending::<()>().await;
        });
        drop(provider);

        let shutdown = EmbeddedTuiShutdown {
            state: state.clone(),
            deferred_init: Some(deferred_init),
            thought_backend: Arc::new(Mutex::new(Some(thought_backend))),
        };

        let outcome = finalize_embedded_tui_shutdown_with_timeouts(
            shutdown,
            ShutdownTimeouts {
                pending_persists: Duration::from_secs(1),
                registry: Duration::from_secs(1),
                flush: Duration::from_secs(1),
                task_abort: Duration::from_millis(100),
                embedded_deferred_init: Duration::from_millis(25),
            },
        )
        .await
        .expect("embedded shutdown finalizer");

        assert_eq!(outcome.deferred_init, DeferredInitShutdown::Aborted);
        assert!(deferred_aborted.load(Ordering::SeqCst));
        assert!(backend_aborted.load(Ordering::SeqCst));
        assert!(outcome.thought_backend_aborted);
        assert!(outcome.persistence.pending_thoughts_drained);
        assert!(outcome.persistence.registry_persisted);
        assert!(outcome.persistence.file_store_flushed);

        let thoughts = store.load_thoughts().await;
        let thought = thoughts.get("sess_1").expect("persisted thought");
        assert_eq!(thought.thought.as_deref(), Some("embedded queued thought"));

        let sessions = store.load_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].objective_fingerprint.as_deref(),
            Some("embedded-obj")
        );
    }

    #[tokio::test]
    async fn shutdown_signal_returns_on_process_signal_future() {
        tokio::time::timeout(
            Duration::from_millis(25),
            shutdown_signal_from(async {}, pending::<()>()),
        )
        .await
        .expect("shutdown signal should complete when process signal future resolves");
    }

    #[tokio::test]
    async fn shutdown_signal_returns_on_terminate_signal_future() {
        tokio::time::timeout(
            Duration::from_millis(25),
            shutdown_signal_from(pending::<()>(), async {}),
        )
        .await
        .expect("shutdown signal should complete when terminate signal future resolves");
    }

    #[tokio::test]
    async fn bridge_self_fence_does_not_complete_server_shutdown_signal() {
        let timing = BridgeTiming {
            tick: Duration::from_millis(5),
            sync_timeout: Duration::from_millis(20),
            min_failure_backoff: Duration::from_millis(5),
            max_failure_backoff: Duration::from_millis(10),
            unhealthy_after: Duration::from_millis(10),
            self_fence_after: Duration::from_millis(15),
        };
        let bridge_health = BridgeHealthState::with_timing(timing);

        bridge_health.record_failure("spawn failed", Duration::from_millis(5));
        tokio::time::sleep(Duration::from_millis(12)).await;
        bridge_health.record_failure("timeout", Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(20)).await;
        bridge_health.record_failure("still timing out", Duration::from_millis(10));
        assert!(
            bridge_health.snapshot().shutdown_requested,
            "setup should produce a thought bridge self-fence request"
        );

        let result = tokio::time::timeout(
            Duration::from_millis(25),
            shutdown_signal_from(pending::<()>(), pending::<()>()),
        )
        .await;
        assert!(
            result.is_err(),
            "thought bridge self-fence must not stop the HTTP API"
        );
    }

    #[tokio::test]
    async fn init_app_state_skeleton_returns_quickly() {
        let config = Arc::new(Config::default());
        let started = Instant::now();
        let state = init_app_state_skeleton(config.clone());
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(100),
            "expected skeleton init under 100ms, got {}ms",
            elapsed.as_millis()
        );
        assert_eq!(state.config.port, config.port);
        assert!(state.current_file_store().is_none());
        assert!(state.current_daemon_defaults().is_none());
    }

    #[tokio::test]
    async fn deferred_phase_join_executes_phases_concurrently() {
        let started = Instant::now();
        let (persistence, discovery, defaults) = run_deferred_phase_join(
            || async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "persistence"
            },
            || async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "discovery"
            },
            || async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "defaults"
            },
        )
        .await;
        let elapsed = started.elapsed();

        assert_eq!(persistence, "persistence");
        assert_eq!(discovery, "discovery");
        assert_eq!(defaults, "defaults");
        assert!(
            elapsed < Duration::from_millis(450),
            "expected concurrent join under 450ms, got {}ms",
            elapsed.as_millis()
        );
    }
}
