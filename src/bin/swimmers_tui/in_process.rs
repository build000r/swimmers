use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::future::BoxFuture;
use tokio::sync::oneshot;

use swimmers::api::remote_sessions;
use swimmers::api::service::{
    create_local_sessions_batch, list_dirs as list_dirs_service,
    list_repo_search_entries as list_repo_search_entries_service, list_sessions_for_client,
    native_status_for_host as native_status_for_host_service, open_native_attention_group_for_host,
    open_native_session_for_host, request_plan_file,
    start_dir_repo_action as start_dir_repo_action_service,
    test_thought_config as test_thought_config_service, thought_config_response,
    update_dir_group_memberships as update_dir_group_memberships_service,
    update_thought_config as update_thought_config_service, ApiServiceError,
    NativeOpenServiceError,
};
use swimmers::api::sessions::send_group_input_service;
use swimmers::api::{AppState, PublishedSelectionState};
use swimmers::openrouter_models::{
    cached_or_default_openrouter_candidates, refresh_openrouter_model_cache, OpenRouterModelCache,
};
use swimmers::session::actor::SessionCommand;
use swimmers::thought::runtime_config::ThoughtConfig;
use swimmers::types::{
    AdoptSessionResponse, AttentionGroupLayout, CreateSessionRequest, CreateSessionResponse,
    CreateSessionsBatchRequest, CreateSessionsBatchResponse, DirGroupMembershipUpdateRequest,
    DirGroupMembershipUpdateResponse, DirListResponse, DirRepoActionResponse,
    DirRepoSearchResponse, ErrorResponse, GhosttyOpenMode, MermaidArtifactResponse,
    NativeAttentionGroupOpenRequest, NativeAttentionGroupOpenResponse, NativeDesktopApp,
    NativeDesktopOpenResponse, NativeDesktopStatusResponse, PlanFileResponse, RepoActionKind,
    SessionGroupInputRequest, SessionGroupInputResponse, SessionSkillListResponse, SessionSummary,
    SpawnTool,
};

use super::api::{ThoughtConfigTestResponse, TuiApi};
use super::{
    load_overlay_plan_entries, BackendDependencyLedger, BackendDependencySnapshot,
    BackendHealthResponse, BackendPersistenceHealth, BackendThoughtBridgeHealth, PlanPanelEntry,
};
pub(crate) use swimmers::types::ThoughtConfigResponse;

pub(crate) struct InProcessApi {
    state: Arc<AppState>,
    http: reqwest::Client,
}

impl InProcessApi {
    pub(crate) fn new(state: Arc<AppState>) -> Self {
        let http = reqwest::Client::builder()
            .build()
            .expect("failed to build reqwest client for in-process API");
        Self { state, http }
    }

    fn fetch_local_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        Box::pin(async move { Ok(list_sessions_for_client(&self.state, false).await) })
    }
}

fn bridge_status_label(status: swimmers::thought::health::BridgeStatus) -> String {
    match status {
        swimmers::thought::health::BridgeStatus::Starting => "starting",
        swimmers::thought::health::BridgeStatus::Healthy => "healthy",
        swimmers::thought::health::BridgeStatus::Degraded => "degraded",
        swimmers::thought::health::BridgeStatus::Unhealthy => "unhealthy",
    }
    .to_string()
}

fn api_service_error_message(error: ApiServiceError) -> String {
    ErrorResponse::with_message(error.code(), error.message()).display_message("in-process API")
}

fn openrouter_candidates_from_refresh_result(
    result: Result<OpenRouterModelCache, String>,
) -> Result<Vec<String>, String> {
    match result {
        Ok(cache) if !cache.models.is_empty() => Ok(cache.models),
        Ok(_) => Ok(cached_or_default_openrouter_candidates()),
        Err(err) => Err(err),
    }
}

fn native_open_error_message(error: NativeOpenServiceError) -> String {
    match error {
        NativeOpenServiceError::Unsupported { reason } => {
            reason.unwrap_or_else(|| "native desktop unavailable".to_string())
        }
        NativeOpenServiceError::NoAttentionSessions => {
            "no sessions are waiting for operator input".to_string()
        }
        NativeOpenServiceError::SessionNotFound => "session not found".to_string(),
        NativeOpenServiceError::SessionExited => "session has already exited".to_string(),
        NativeOpenServiceError::Internal(message) => message,
    }
}

fn remote_session_skills_response(session_id: String) -> SessionSkillListResponse {
    SessionSkillListResponse {
        session_id,
        source: "sbp".to_string(),
        cwd: String::new(),
        available: false,
        query: None,
        skills: Vec::new(),
        issues: Vec::new(),
        message: Some("remote session skills must be queried on the target host".to_string()),
    }
}

async fn fetch_mermaid_artifact_for_session(
    state: Arc<AppState>,
    session_id: String,
) -> Result<MermaidArtifactResponse, String> {
    if let Some((target, remote_session_id)) = remote_sessions::denamespace_for_target(&session_id)
        .map_err(|err| err.display_message("in-process API"))?
    {
        return remote_sessions::fetch_remote_mermaid_artifact(&target, remote_session_id)
            .await
            .map_err(|err| err.display_message("in-process API"));
    }
    let handle = state
        .supervisor
        .get_session(&session_id)
        .await
        .ok_or_else(|| "session not found".to_string())?;
    let (tx, rx) = oneshot::channel();
    handle
        .send(SessionCommand::GetMermaidArtifact(tx))
        .await
        .map_err(|_| "session actor unavailable".to_string())?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .map_err(|_| "mermaid artifact request timed out".to_string())?
        .map_err(|_| "actor dropped mermaid artifact reply".to_string())
}

async fn fetch_session_skills_for_session(
    state: Arc<AppState>,
    session_id: String,
) -> Result<SessionSkillListResponse, String> {
    if remote_sessions::split_remote_session_id(&session_id).is_some() {
        return Ok(remote_session_skills_response(session_id));
    }
    let summary = state
        .supervisor
        .list_sessions()
        .await
        .into_iter()
        .find(|summary| summary.session_id == session_id)
        .ok_or_else(|| "session not found".to_string())?;
    Ok(swimmers::api::skills::read_sbp_session_skills(&session_id, &summary.cwd, None).await)
}

fn native_attention_group_request(
    max_sessions: usize,
    current_session_ids: Vec<String>,
    focus: bool,
    include_unnumbered_sessions: bool,
    layout: AttentionGroupLayout,
) -> NativeAttentionGroupOpenRequest {
    NativeAttentionGroupOpenRequest {
        max_sessions: Some(max_sessions),
        current_session_ids,
        include_unnumbered_sessions,
        layout: Some(layout),
        focus,
    }
}

// ---------------------------------------------------------------------------
// TuiApi implementation
// ---------------------------------------------------------------------------

impl TuiApi for InProcessApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        Box::pin(async move { Ok(list_sessions_for_client(&self.state, true).await) })
    }

    fn fetch_sessions_for_initial_frame(
        &self,
    ) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        self.fetch_local_sessions()
    }

    fn fetch_backend_health(&self) -> BoxFuture<'_, Result<BackendHealthResponse, String>> {
        Box::pin(async move {
            let thought_bridge = self.state.bridge_health.snapshot();
            let persistence = self
                .state
                .current_file_store()
                .map(|store| store.health_snapshot());
            // Share the HTTP /health surface's ledger builder so the embedded
            // adapter never re-derives dependency statuses on its own. The
            // builder produces the COMPLETE server ledger; we then project it
            // onto the four-field shape this surface's BackendDependencyLedger
            // deserialize type carries (the same projection serde performs when
            // the external client parses the /health JSON over the wire).
            let ledger = swimmers::api::health::build_dependency_ledger(&self.state).await;
            let dep_snap =
                |h: &swimmers::types::DependencyHealthSnapshot| BackendDependencySnapshot {
                    status: swimmers::api::health::dependency_status_label(h.status).to_string(),
                    last_error: h.last_error.clone(),
                };
            Ok(BackendHealthResponse {
                status: bridge_status_label(thought_bridge.status),
                thought_bridge: BackendThoughtBridgeHealth {
                    status: bridge_status_label(thought_bridge.status),
                    consecutive_failures: thought_bridge.consecutive_failures,
                    last_error: thought_bridge.last_error,
                    last_backend_error: thought_bridge.last_backend_error,
                    shutdown_requested: thought_bridge.shutdown_requested,
                    shutdown_reason: thought_bridge.shutdown_reason,
                },
                persistence: match persistence {
                    Some(snapshot) => BackendPersistenceHealth {
                        available: true,
                        ok: snapshot.ok,
                        consecutive_failures: snapshot.consecutive_failures,
                        last_successful_operation: snapshot.last_successful_operation,
                        last_failed_operation: snapshot.last_failed_operation,
                        last_error: snapshot.last_error,
                    },
                    None => BackendPersistenceHealth {
                        available: false,
                        ok: false,
                        ..BackendPersistenceHealth::default()
                    },
                },
                dependencies: Some(BackendDependencyLedger {
                    tmux_discovery: dep_snap(&ledger.tmux_discovery),
                    tmux_capture: dep_snap(&ledger.tmux_capture),
                    native_scripts: dep_snap(&ledger.native_scripts),
                    remote_targets: dep_snap(&ledger.remote_targets),
                }),
            })
        })
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        Box::pin(async move { Ok(thought_config_response(&self.state).await) })
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        Box::pin(async move {
            update_thought_config_service(&self.state, config)
                .await
                .map_err(api_service_error_message)
        })
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        Box::pin(async move {
            test_thought_config_service(config)
                .await
                .map_err(api_service_error_message)
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        // Mirrors: src/bin/swimmers_tui/api.rs:493 (ApiClient::refresh_openrouter_candidates)
        Box::pin(async move {
            openrouter_candidates_from_refresh_result(
                refresh_openrouter_model_cache(&self.http).await,
            )
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        // Mirrors: src/api/sessions.rs:434 (get_mermaid_artifact)
        let session_id = session_id.to_string();
        let state = self.state.clone();
        Box::pin(async move { fetch_mermaid_artifact_for_session(state, session_id).await })
    }

    fn fetch_session_skills(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<SessionSkillListResponse, String>> {
        let session_id = session_id.to_string();
        let state = self.state.clone();
        Box::pin(async move { fetch_session_skills_for_session(state, session_id).await })
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
            request_plan_file(&self.state, &session_id, &name)
                .await
                .map_err(|err| err.message())
        })
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        // Mirrors: src/api/native.rs:48 (native_status)
        Box::pin(async move { Ok(native_status_for_host_service(&self.state, "localhost").await) })
    }

    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        // Mirrors: src/api/native.rs:58 (set_native_app)
        Box::pin(async move {
            {
                let mut native_app = self.state.native_desktop_app.write().await;
                *native_app = app;
            }
            Ok(native_status_for_host_service(&self.state, "localhost").await)
        })
    }

    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        // Mirrors: src/api/native.rs:77 (set_native_mode)
        Box::pin(async move {
            {
                let mut ghostty_mode = self.state.ghostty_open_mode.write().await;
                *ghostty_mode = mode;
            }
            Ok(native_status_for_host_service(&self.state, "localhost").await)
        })
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        // Mirrors: src/api/selection.rs:68 (publish_selection)
        let session_id = session_id.and_then(|v| {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        Box::pin(async move {
            let published_at = session_id.as_ref().map(|_| Utc::now());
            let mut selection = self.state.published_selection.write().await;
            *selection = PublishedSelectionState {
                session_id,
                published_at,
            };
            Ok(())
        })
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        // Mirrors: src/api/native.rs:96 (native_open)
        let session_id = session_id.to_string();
        Box::pin(async move {
            if remote_sessions::split_remote_session_id(&session_id).is_some() {
                return Err(
                    "remote sessions are visible locally, but native terminal handoff must be opened on the target host"
                        .to_string(),
                );
            }
            open_native_session_for_host(&self.state, "localhost", &session_id)
                .await
                .map_err(native_open_error_message)
        })
    }

    fn open_attention_group(
        &self,
        max_sessions: usize,
        current_session_ids: Vec<String>,
        focus: bool,
        include_unnumbered_sessions: bool,
        layout: AttentionGroupLayout,
    ) -> BoxFuture<'_, Result<NativeAttentionGroupOpenResponse, String>> {
        let request = native_attention_group_request(
            max_sessions,
            current_session_ids,
            focus,
            include_unnumbered_sessions,
            layout,
        );
        Box::pin(async move {
            open_native_attention_group_for_host(&self.state, "localhost", request)
                .await
                .map_err(native_open_error_message)
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        group: Option<&str>,
        target: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let path = path.map(str::to_owned);
        let group = group.map(str::to_owned);
        let target = target.map(str::to_owned);
        Box::pin(async move {
            if let Some(target) = target
                .as_deref()
                .map(str::trim)
                .filter(|target| !target.is_empty() && *target != "local")
            {
                return remote_sessions::list_remote_dirs(
                    target,
                    path.as_deref(),
                    managed_only,
                    group.as_deref(),
                )
                .await
                .map_err(|err| err.display_message("in-process API"));
            }
            list_dirs_service(&self.state, path.as_deref(), managed_only, group.as_deref())
                .await
                .map_err(api_service_error_message)
        })
    }

    fn list_repo_dirs(&self) -> BoxFuture<'_, Result<DirRepoSearchResponse, String>> {
        Box::pin(async move {
            list_repo_search_entries_service()
                .await
                .map_err(api_service_error_message)
        })
    }

    fn start_repo_action(
        &self,
        path: &str,
        kind: RepoActionKind,
    ) -> BoxFuture<'_, Result<DirRepoActionResponse, String>> {
        let path = path.to_string();
        Box::pin(async move {
            start_dir_repo_action_service(self.state.clone(), &path, kind)
                .await
                .map_err(api_service_error_message)
        })
    }

    fn update_dir_group_memberships(
        &self,
        path: &str,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> BoxFuture<'_, Result<DirGroupMembershipUpdateResponse, String>> {
        let state = self.state.clone();
        let path = path.to_string();
        Box::pin(async move {
            update_dir_group_memberships_service(
                state,
                DirGroupMembershipUpdateRequest {
                    path,
                    target: None,
                    add,
                    remove,
                },
            )
            .await
            .map_err(api_service_error_message)
        })
    }

    fn fetch_overlay_plans(&self) -> BoxFuture<'_, Result<Vec<PlanPanelEntry>, String>> {
        Box::pin(async move {
            tokio::task::spawn_blocking(load_overlay_plan_entries)
                .await
                .map_err(|err| format!("overlay plan scan failed: {err}"))
        })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        // Mirrors: src/api/sessions.rs:46 (create_session)
        let cwd = cwd.to_string();
        Box::pin(async move {
            if remote_sessions::is_remote_launch_target(launch_target.as_deref()) {
                return remote_sessions::create_remote_session(CreateSessionRequest {
                    name: None,
                    cwd: Some(cwd),
                    spawn_tool: Some(spawn_tool),
                    launch_target,
                    initial_request,
                })
                .await
                .map_err(|err| err.display_message("in-process API"));
            }
            let (session, repo_theme) = self
                .state
                .supervisor
                .create_session(None, Some(cwd), Some(spawn_tool), initial_request)
                .await
                .map_err(|err| err.to_string())?;
            Ok(CreateSessionResponse {
                session,
                repo_theme,
            })
        })
    }

    fn adopt_session(
        &self,
        tmux_name: &str,
        session_id: Option<&str>,
    ) -> BoxFuture<'_, Result<AdoptSessionResponse, String>> {
        let tmux_name = tmux_name.to_string();
        let session_id = session_id.map(str::to_string);
        Box::pin(async move {
            let adopted = self
                .state
                .supervisor
                .adopt_tmux_session(tmux_name, session_id)
                .await
                .map_err(|err| err.to_string())?;
            Ok(AdoptSessionResponse {
                session: adopted.session,
                repo_theme: adopted.repo_theme,
                reused_session_id: adopted.reused_session_id,
            })
        })
    }

    fn create_sessions_batch(
        &self,
        dirs: Vec<String>,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionsBatchResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            if remote_sessions::is_remote_launch_target(launch_target.as_deref()) {
                return remote_sessions::create_remote_sessions_batch(CreateSessionsBatchRequest {
                    dirs,
                    spawn_tool: Some(spawn_tool),
                    launch_target,
                    initial_request,
                })
                .await
                .map_err(|err| err.display_message("in-process API"));
            }
            create_local_sessions_batch(state, dirs, Some(spawn_tool), initial_request)
                .await
                .map_err(api_service_error_message)
        })
    }

    fn send_group_input(
        &self,
        session_ids: Vec<String>,
        text: String,
    ) -> BoxFuture<'_, Result<SessionGroupInputResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            send_group_input_service(state, SessionGroupInputRequest { session_ids, text })
                .await
                .map_err(|err| err.display_message("in-process API"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path as FsPath;
    use std::sync::{LazyLock, Mutex};
    use swimmers::config::Config;
    use swimmers::persistence::file_store::FileStore;
    use swimmers::session::actor::{ActorHandle, InputDeliveryResult};
    use swimmers::session::supervisor::SessionSupervisor;
    use swimmers::thought::protocol::SyncRequestSequence;
    use swimmers::types::{
        ErrorResponse, RestState, SessionGroupInputResult, SessionState, StateEvidence,
        ThoughtSource, ThoughtState, TransportHealth, MAX_SESSION_INPUT_BYTES,
    };
    use tokio::sync::mpsc;
    use tokio::sync::RwLock;

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn test_state() -> Arc<AppState> {
        test_state_with_store(None)
    }

    fn test_state_with_store(file_store: Option<Arc<FileStore>>) -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: swimmers::api::once_lock_with(None),
            file_store: swimmers::api::once_lock_with(file_store),
            bridge_health: Arc::new(swimmers::thought::health::BridgeHealthState::new_with_tick(
                Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: swimmers::host_actions::RepoActionTracker::default(),
        })
    }

    fn summary(session_id: &str, state: SessionState) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: format!("tmux-{session_id}"),
            state,
            current_command: None,
            state_evidence: StateEvidence::new("test"),
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Sleeping,
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
        }
    }

    async fn insert_summary_test_handle(
        state: &Arc<AppState>,
        summary: SessionSummary,
    ) -> mpsc::Receiver<Vec<u8>> {
        let session_id = summary.session_id.clone();
        let tmux_name = summary.tmux_name.clone();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let (write_tx, write_rx) = mpsc::channel(1);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::WriteInputAck { data, ack } => {
                        let _ = write_tx.send(data).await;
                        let _ = ack.send(InputDeliveryResult {
                            delivered: true,
                            method: "test",
                            message: None,
                        });
                    }
                    _ => {}
                }
            }
        });
        write_rx
    }

    struct TestEnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl TestEnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_executable(path: &FsPath, contents: &str) {
        fs::write(path, contents).expect("write executable");
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod executable");
    }

    #[tokio::test]
    async fn fetch_sessions_returns_empty_list() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_sessions().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_sessions_returns_local_sessions_without_http_envelope() {
        let state = test_state();
        let _write_rx =
            insert_summary_test_handle(&state, summary("sess-1", SessionState::Idle)).await;
        let api = InProcessApi::new(state);

        let sessions = api.fetch_sessions().await.expect("sessions");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-1");
        // HTTP adapters wrap the same shared list in SessionListResponse and
        // enforce auth; the embedded TUI adapter intentionally returns the
        // route-independent Vec<SessionSummary> directly.
    }

    #[tokio::test]
    async fn fetch_native_status_returns_ok() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_native_status().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn fetch_backend_health_matches_health_snapshot_shape() {
        let state = test_state();
        state.bridge_health.record_success(None);
        let api = InProcessApi::new(state);

        let health = api.fetch_backend_health().await.expect("backend health");

        assert_eq!(health.status, "healthy");
        assert_eq!(health.thought_bridge.status, "healthy");
        assert!(!health.persistence.available);
        assert!(!health.persistence.ok);

        // Dependency statuses now come from the shared health.rs builder /
        // label helper rather than an inline mapping, so the embedded surface
        // labels each dependency exactly as the HTTP /health JSON would.
        let dependencies = health
            .dependencies
            .as_ref()
            .expect("embedded health should expose the dependency ledger");
        let expected = swimmers::api::health::build_dependency_ledger(&api.state).await;
        assert_eq!(
            dependencies.tmux_discovery.status,
            swimmers::api::health::dependency_status_label(expected.tmux_discovery.status)
        );
        assert_eq!(
            dependencies.remote_targets.status,
            swimmers::api::health::dependency_status_label(expected.remote_targets.status)
        );
        assert_eq!(
            dependencies.native_scripts.status,
            swimmers::api::health::dependency_status_label(expected.native_scripts.status)
        );
    }

    #[tokio::test]
    async fn publish_selection_round_trip() {
        let state = test_state();
        let published = state.published_selection.clone();
        let api = InProcessApi::new(state);

        let result = api.publish_selection(Some("test-session")).await;
        assert!(result.is_ok());
        {
            let sel = published.read().await;
            assert_eq!(sel.session_id.as_deref(), Some("test-session"));
            assert!(sel.published_at.is_some());
        }

        let result = api.publish_selection(None).await;
        assert!(result.is_ok());
        {
            let sel = published.read().await;
            assert!(sel.session_id.is_none());
            assert!(sel.published_at.is_none());
        }
    }

    #[tokio::test]
    async fn fetch_thought_config_returns_defaults() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_thought_config().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.daemon_defaults.is_none());
    }

    #[tokio::test]
    async fn update_thought_config_rejects_invalid_payloads_like_http_service() {
        let api = InProcessApi::new(test_state());

        let err = api
            .update_thought_config(ThoughtConfig {
                cadence_hot_ms: 1,
                ..ThoughtConfig::default()
            })
            .await
            .expect_err("invalid config should fail");

        assert!(err.contains("cadence_hot_ms"));
        assert!(err.contains("must be between"));
    }

    #[tokio::test]
    async fn update_thought_config_reports_persistence_failures_without_committing_memory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");
        std::fs::create_dir(dir.path().join("thought_config.json"))
            .expect("create directory at thought config path");
        let state = test_state_with_store(Some(store));
        let api = InProcessApi::new(state.clone());

        let err = api
            .update_thought_config(ThoughtConfig {
                enabled: false,
                ..ThoughtConfig::default()
            })
            .await
            .expect_err("disk failure should fail");

        assert_eq!(err, "INTERNAL_ERROR: failed to persist thought config");
        assert!(state.thought_config.read().await.enabled);
    }

    #[tokio::test]
    async fn fetch_plan_file_reports_actor_reply_errors_from_shared_service() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-plan", "tmux-plan", cmd_tx))
            .await;
        tokio::spawn(async move {
            if let Some(SessionCommand::GetPlanFile { name, reply }) = cmd_rx.recv().await {
                assert_eq!(name, "plan.md");
                drop(reply);
            }
        });
        let api = InProcessApi::new(state);

        let err = api
            .fetch_plan_file("sess-plan", "plan.md")
            .await
            .expect_err("dropped reply should fail");

        assert_eq!(err, "actor dropped plan file reply");
    }

    #[test]
    fn openrouter_refresh_result_preserves_cache_error_and_fallback_semantics() {
        let models = openrouter_candidates_from_refresh_result(Ok(OpenRouterModelCache {
            generated_at_epoch_ms: 7,
            models: vec!["provider/model".to_string()],
        }))
        .expect("non-empty cache should return models");
        assert_eq!(models, vec!["provider/model"]);

        let fallback = openrouter_candidates_from_refresh_result(Ok(OpenRouterModelCache {
            generated_at_epoch_ms: 8,
            models: Vec::new(),
        }))
        .expect("empty cache should fall back");
        assert_eq!(fallback, cached_or_default_openrouter_candidates());

        let err = openrouter_candidates_from_refresh_result(Err("catalog unavailable".to_string()))
            .expect_err("refresh errors should pass through");
        assert_eq!(err, "catalog unavailable");
    }

    #[test]
    fn native_open_error_message_matches_api_error_strings() {
        assert_eq!(
            native_open_error_message(NativeOpenServiceError::Unsupported { reason: None }),
            "native desktop unavailable"
        );
        assert_eq!(
            native_open_error_message(NativeOpenServiceError::Unsupported {
                reason: Some("not on this host".to_string())
            }),
            "not on this host"
        );
        assert_eq!(
            native_open_error_message(NativeOpenServiceError::NoAttentionSessions),
            "no sessions are waiting for operator input"
        );
        assert_eq!(
            native_open_error_message(NativeOpenServiceError::SessionNotFound),
            "session not found"
        );
        assert_eq!(
            native_open_error_message(NativeOpenServiceError::SessionExited),
            "session has already exited"
        );
        assert_eq!(
            native_open_error_message(NativeOpenServiceError::Internal("boom".to_string())),
            "boom"
        );
    }

    #[tokio::test]
    async fn fetch_mermaid_artifact_reports_missing_local_session() {
        let api = InProcessApi::new(test_state());

        let err = api
            .fetch_mermaid_artifact("missing")
            .await
            .expect_err("missing local session should fail");

        assert_eq!(err, "session not found");
    }

    #[tokio::test]
    async fn fetch_mermaid_artifact_returns_actor_payload() {
        let state = test_state();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle("sess-art", "tmux-art", cmd_tx))
            .await;
        tokio::spawn(async move {
            if let Some(SessionCommand::GetMermaidArtifact(reply)) = cmd_rx.recv().await {
                let _ = reply.send(MermaidArtifactResponse {
                    session_id: "sess-art".to_string(),
                    available: true,
                    path: Some("/tmp/project/diagram.mmd".to_string()),
                    updated_at: None,
                    source: Some("flowchart TD".to_string()),
                    error: None,
                    slice_name: Some("slice".to_string()),
                    plan_files: Some(vec!["PLAN.md".to_string()]),
                });
            }
        });
        let api = InProcessApi::new(state);

        let artifact = api
            .fetch_mermaid_artifact("sess-art")
            .await
            .expect("actor artifact");

        assert!(artifact.available);
        assert_eq!(artifact.session_id, "sess-art");
        assert_eq!(artifact.path.as_deref(), Some("/tmp/project/diagram.mmd"));
        assert_eq!(artifact.source.as_deref(), Some("flowchart TD"));
        assert_eq!(artifact.slice_name.as_deref(), Some("slice"));
        assert_eq!(artifact.plan_files, Some(vec!["PLAN.md".to_string()]));
    }

    #[tokio::test]
    async fn fetch_session_skills_reports_remote_sessions_as_target_host_work() {
        let api = InProcessApi::new(test_state());

        let skills = api
            .fetch_session_skills("target-host::sess-1")
            .await
            .expect("remote skills response");

        assert_eq!(skills.session_id, "target-host::sess-1");
        assert_eq!(skills.source, "sbp");
        assert!(!skills.available);
        assert_eq!(skills.cwd, "");
        assert_eq!(skills.skills.len(), 0);
        assert_eq!(
            skills.message.as_deref(),
            Some("remote session skills must be queried on the target host")
        );
    }

    #[tokio::test]
    async fn fetch_session_skills_reports_missing_local_session() {
        let api = InProcessApi::new(test_state());

        let err = api
            .fetch_session_skills("missing")
            .await
            .expect_err("missing local session should fail");

        assert_eq!(err, "session not found");
    }

    #[tokio::test]
    async fn fetch_session_skills_reads_local_summary_cwd() {
        let _lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let sbp = dir.path().join("sbp");
        write_executable(
            &sbp,
            r#"#!/bin/sh
printf '{"effective":[],"recommendations":[]}\n'
"#,
        );
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", sbp.as_os_str());

        let state = test_state();
        let _write_rx =
            insert_summary_test_handle(&state, summary("sess-skills", SessionState::Idle)).await;
        let api = InProcessApi::new(state);

        let skills = api
            .fetch_session_skills("sess-skills")
            .await
            .expect("local skills response");

        assert_eq!(skills.session_id, "sess-skills");
        assert_eq!(skills.cwd, "/tmp/project");
        assert_eq!(skills.source, "sbp");
        assert!(skills.available);
    }

    #[tokio::test]
    async fn open_session_keeps_remote_native_handoff_as_adapter_error_string() {
        let api = InProcessApi::new(test_state());

        let err = api
            .open_session("target-host::sess-1")
            .await
            .expect_err("remote native open should fail locally");

        assert_eq!(
            err,
            "remote sessions are visible locally, but native terminal handoff must be opened on the target host"
        );
    }

    #[tokio::test]
    async fn open_attention_group_reports_empty_native_attention_plan() {
        let api = InProcessApi::new(test_state());

        let err = api
            .open_attention_group(6, Vec::new(), true, false, AttentionGroupLayout::Tiled)
            .await
            .expect_err("empty attention group should fail");

        assert_eq!(err, "no sessions are waiting for operator input");
    }

    #[test]
    fn native_attention_group_request_preserves_tui_options() {
        let request = native_attention_group_request(
            3,
            vec!["current".to_string()],
            false,
            true,
            AttentionGroupLayout::MainVertical,
        );

        assert_eq!(request.max_sessions, Some(3));
        assert_eq!(request.current_session_ids, vec!["current"]);
        assert!(!request.focus);
        assert!(request.include_unnumbered_sessions);
        assert_eq!(request.layout, Some(AttentionGroupLayout::MainVertical));
    }

    #[tokio::test]
    async fn create_sessions_batch_reuses_api_validation_messages() {
        let api = InProcessApi::new(test_state());

        let err = api
            .create_sessions_batch(Vec::new(), SpawnTool::Codex, None, None)
            .await
            .expect_err("empty batch should fail");

        assert_eq!(err, "VALIDATION_FAILED: dirs must not be empty");
    }

    #[tokio::test]
    async fn create_sessions_batch_rejects_oversized_remote_batches_before_target_lookup() {
        let api = InProcessApi::new(test_state());
        let dirs = (0..=swimmers::api::service::BATCH_CREATE_MAX_DIRS)
            .map(|index| format!("/tmp/project-{index}"))
            .collect::<Vec<_>>();

        let err = api
            .create_sessions_batch(
                dirs,
                SpawnTool::Codex,
                Some("missing-remote".to_string()),
                None,
            )
            .await
            .expect_err("oversized remote batch should fail locally");

        assert_eq!(
            err,
            format!(
                "VALIDATION_FAILED: dirs must include at most {} entries",
                swimmers::api::service::BATCH_CREATE_MAX_DIRS
            )
        );
    }

    #[tokio::test]
    async fn send_group_input_returns_service_validation_as_tui_error_string() {
        let api = InProcessApi::new(test_state());

        let err = api
            .send_group_input(Vec::new(), "continue".to_string())
            .await
            .expect_err("empty group should fail");

        assert_eq!(err, "VALIDATION_FAILED: session_ids must not be empty");
    }

    #[tokio::test]
    async fn send_group_input_returns_oversized_input_as_tui_error_string() {
        let api = InProcessApi::new(test_state());

        let err = api
            .send_group_input(
                vec!["first".to_string(), "second".to_string()],
                "x".repeat(MAX_SESSION_INPUT_BYTES + 1),
            )
            .await
            .expect_err("oversized group input should fail");

        assert_eq!(
            err,
            format!("INPUT_TOO_LARGE: terminal input exceeds {MAX_SESSION_INPUT_BYTES} byte limit")
        );
    }

    #[tokio::test]
    async fn send_group_input_delivers_to_ready_batch_sessions() {
        let state = test_state();
        let batch_id = "batch-shared";
        let mut first = summary("first", SessionState::Idle);
        first.batch = Some(swimmers::api::sessions::session_batch_membership(
            batch_id.to_string(),
            "test batch".to_string(),
            0,
            2,
            Utc::now(),
            Some("continue".to_string()),
        ));
        let mut second = summary("second", SessionState::Idle);
        second.batch = Some(swimmers::api::sessions::session_batch_membership(
            batch_id.to_string(),
            "test batch".to_string(),
            1,
            2,
            Utc::now(),
            Some("continue".to_string()),
        ));
        let mut first_writes = insert_summary_test_handle(&state, first).await;
        let mut second_writes = insert_summary_test_handle(&state, second).await;
        let api = InProcessApi::new(state);

        let response = api
            .send_group_input(
                vec!["first".to_string(), "second".to_string()],
                "continue".to_string(),
            )
            .await
            .expect("group input response");

        assert_eq!(response.delivered, 2);
        assert_eq!(response.skipped, 0);
        assert_eq!(
            first_writes.recv().await.expect("first write"),
            b"continue\r\r".to_vec()
        );
        assert_eq!(
            second_writes.recv().await.expect("second write"),
            b"continue\r\r".to_vec()
        );
    }

    #[test]
    fn group_input_response_counts_match_http_payload_semantics() {
        let response = SessionGroupInputResponse::from_results(vec![
            SessionGroupInputResult {
                session_id: "ok".to_string(),
                ok: true,
                error: None,
            },
            SessionGroupInputResult {
                session_id: "skipped".to_string(),
                ok: false,
                error: Some(ErrorResponse {
                    code: "SESSION_NOT_READY".to_string(),
                    message: Some("session is not waiting for input".to_string()),
                }),
            },
        ]);

        assert_eq!(response.delivered, 1);
        assert_eq!(response.skipped, 1);
    }
}
