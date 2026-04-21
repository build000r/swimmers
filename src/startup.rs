use std::io::Write;
use std::sync::Arc;
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

pub fn resolve_data_dir() -> std::path::PathBuf {
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

async fn finalize_shutdown(supervisor: &Arc<SessionSupervisor>, thought_backend: JoinHandle<()>) {
    thought_backend.abort();
    let _ = thought_backend.await;
    supervisor
        .wait_for_pending_thought_persists(SHUTDOWN_PERSIST_TIMEOUT)
        .await;
    supervisor.persist_registry().await;
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

pub fn signal_readiness() {
    let fd_raw = match std::env::var("SWIMMERS_READY_FD") {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) => {
            tracing::trace!("SWIMMERS_READY_FD is empty; skipping readiness signal");
            return;
        }
        Err(_) => {
            tracing::trace!("SWIMMERS_READY_FD not set; skipping readiness signal");
            return;
        }
    };

    let fd = match fd_raw.parse::<i32>() {
        Ok(fd) => fd,
        Err(err) => {
            tracing::trace!(
                value = %fd_raw,
                error = %err,
                "SWIMMERS_READY_FD is not a valid i32; skipping readiness signal"
            );
            return;
        }
    };

    #[cfg(unix)]
    {
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
        return;
    }

    #[cfg(not(unix))]
    {
        tracing::warn!(
            fd,
            "SWIMMERS_READY_FD signaling is only implemented on unix platforms"
        );
    }
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

pub fn spawn_deferred_init(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Handle::current();
        runtime.block_on(async move {
            tracing::info!(phase = "deferred_init", "startup phase begin");
            let deferred_started = Instant::now();

            let (persistence_store, (), daemon_defaults) =
                run_deferred_init_phases(state.supervisor.clone(), state.thought_config.clone())
                    .await;

            log_startup_phase_complete("deferred_init", deferred_started);
            attach_deferred_init_results(&state, persistence_store, daemon_defaults);
            drop(start_deferred_thought_backend(&state));
        });
    })
}

pub async fn run_server(
    config: Arc<Config>,
    prom_handle: metrics_exporter_prometheus::PrometheusHandle,
) -> anyhow::Result<()> {
    let startup_started = Instant::now();
    let port = config.port;
    let bind = config.bind.clone();

    let (state, thought_backend, bridge_health) = init_app_state(config.clone()).await;
    let supervisor = state.supervisor.clone();
    let app = build_app_router(config, state, prom_handle);
    let listener = bind_listener(&bind, port).await?;
    signal_readiness();

    tracing::info!(
        elapsed_ms = startup_started.elapsed().as_millis() as u64,
        "startup complete; listener ready"
    );
    tracing::info!("Swimmers running on http://{bind}:{port}");

    let server_result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(bridge_health.clone()))
        .await;

    finalize_shutdown(&supervisor, thought_backend).await;

    server_result.map_err(|err| anyhow::anyhow!("server error: {err}"))?;
    if let Some(reason) = bridge_health.shutdown_reason() {
        return Err(anyhow::anyhow!(
            "thought bridge requested shutdown: {reason}"
        ));
    }

    Ok(())
}

async fn shutdown_signal(bridge_health: Arc<BridgeHealthState>) {
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

    let bridge_unhealthy = async move {
        let reason = bridge_health.wait_for_shutdown_request().await;
        tracing::error!(reason, "thought bridge requested process shutdown");
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = bridge_unhealthy => {},
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
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
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

        finalize_shutdown(&supervisor, thought_backend).await;

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
