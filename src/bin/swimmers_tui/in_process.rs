use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::future::BoxFuture;
use tokio::sync::oneshot;

use swimmers::api::remote_sessions;
use swimmers::api::service::{
    list_dirs as list_dirs_service, list_repo_search_entries as list_repo_search_entries_service,
    native_status_for_host as native_status_for_host_service, open_native_attention_group_for_host,
    open_native_session_for_host, start_dir_repo_action as start_dir_repo_action_service,
    update_dir_group_memberships as update_dir_group_memberships_service, NativeOpenServiceError,
};
use swimmers::api::sessions::{
    create_sessions_batch_result, new_batch_context, send_group_input_service,
    session_batch_membership,
};
use swimmers::api::{AppState, PublishedSelectionState};
use swimmers::openrouter_models::{
    cached_or_default_openrouter_candidates, refresh_openrouter_model_cache,
};
use swimmers::session::actor::SessionCommand;
use swimmers::thought::probe::run_thought_config_probe;
use swimmers::thought::runtime_config::ThoughtConfig;
use swimmers::thought_ui::thought_config_ui_metadata;
use swimmers::types::{
    AdoptSessionResponse, CreateSessionRequest, CreateSessionResponse, CreateSessionsBatchRequest,
    CreateSessionsBatchResponse, DirGroupMembershipUpdateRequest, DirGroupMembershipUpdateResponse,
    DirListResponse, DirRepoActionResponse, DirRepoSearchResponse, GhosttyOpenMode,
    MermaidArtifactResponse, NativeAttentionGroupOpenRequest, NativeAttentionGroupOpenResponse,
    NativeDesktopApp, NativeDesktopOpenResponse, NativeDesktopStatusResponse, PlanFileResponse,
    RepoActionKind, SessionGroupInputRequest, SessionGroupInputResponse, SessionSkillListResponse,
    SessionSummary, SpawnTool,
};

use super::api::{ThoughtConfigTestResponse, TuiApi};
use super::{
    load_overlay_plan_entries, BackendHealthResponse, BackendPersistenceHealth,
    BackendThoughtBridgeHealth, PlanPanelEntry,
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
        Box::pin(async move { Ok(self.state.supervisor.list_sessions().await) })
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

// ---------------------------------------------------------------------------
// TuiApi implementation
// ---------------------------------------------------------------------------

impl TuiApi for InProcessApi {
    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        // Mirrors: src/api/sessions.rs:23 (list_sessions)
        Box::pin(async move {
            let mut sessions = self.state.supervisor.list_sessions().await;
            sessions.extend(remote_sessions::list_remote_sessions().await);
            Ok(sessions)
        })
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
            })
        })
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        // Mirrors: src/api/thought_config.rs:19 (get_thought_config)
        Box::pin(async move {
            let config = self.state.thought_config.read().await.clone();
            Ok(ThoughtConfigResponse {
                config,
                daemon_defaults: self.state.current_daemon_defaults(),
                ui: thought_config_ui_metadata(&cached_or_default_openrouter_candidates()),
            })
        })
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        // Mirrors: src/api/thought_config.rs:43 (put_thought_config)
        Box::pin(async move {
            let config = config
                .normalize_and_validate()
                .map_err(|err| err.to_string())?;

            let store = self
                .state
                .current_file_store()
                .ok_or_else(|| "thought config persistence is unavailable".to_string())?;

            // Hold the runtime-config write lock across the disk save and the
            // in-memory update so disk and in-memory state cannot diverge under
            // concurrent updaters: the last writer to land on disk is also the
            // last writer to land in memory.
            let mut runtime_config = self.state.thought_config.write().await;
            store.save_thought_config(&config).await.map_err(|err| {
                tracing::error!(error = %err, "failed to persist thought runtime config");
                "failed to persist thought config".to_string()
            })?;
            *runtime_config = config.clone();
            drop(runtime_config);

            Ok(config)
        })
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        // Mirrors: src/api/thought_config.rs:100 (post_thought_config_test)
        Box::pin(async move {
            let config = config
                .normalize_and_validate()
                .map_err(|err| err.to_string())?;
            Ok(run_thought_config_probe(&config).await)
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        // Mirrors: src/bin/swimmers_tui/api.rs:493 (ApiClient::refresh_openrouter_candidates)
        Box::pin(async move {
            match refresh_openrouter_model_cache(&self.http).await {
                Ok(cache) if !cache.models.is_empty() => Ok(cache.models),
                Ok(_) => Ok(cached_or_default_openrouter_candidates()),
                Err(err) => Err(err),
            }
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        // Mirrors: src/api/sessions.rs:434 (get_mermaid_artifact)
        let session_id = session_id.to_string();
        Box::pin(async move {
            if let Some((target, remote_session_id)) =
                remote_sessions::denamespace_for_target(&session_id)
                    .map_err(|err| err.message().to_string())?
            {
                return remote_sessions::fetch_remote_mermaid_artifact(&target, remote_session_id)
                    .await
                    .map_err(|err| err.message().to_string());
            }
            let handle = self
                .state
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
        })
    }

    fn fetch_session_skills(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<SessionSkillListResponse, String>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            if remote_sessions::split_remote_session_id(&session_id).is_some() {
                return Ok(SessionSkillListResponse {
                    session_id,
                    source: "sbp".to_string(),
                    cwd: String::new(),
                    available: false,
                    query: None,
                    skills: Vec::new(),
                    issues: Vec::new(),
                    message: Some(
                        "remote session skills must be queried on the target host".to_string(),
                    ),
                });
            }
            let summary = self
                .state
                .supervisor
                .list_sessions()
                .await
                .into_iter()
                .find(|summary| summary.session_id == session_id)
                .ok_or_else(|| "session not found".to_string())?;
            Ok(
                swimmers::api::skills::read_sbp_session_skills(&session_id, &summary.cwd, None)
                    .await,
            )
        })
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        // Mirrors: src/api/sessions.rs:502 (get_plan_file)
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
            if let Some((target, remote_session_id)) =
                remote_sessions::denamespace_for_target(&session_id)
                    .map_err(|err| err.message().to_string())?
            {
                return remote_sessions::fetch_remote_plan_file(&target, remote_session_id, &name)
                    .await
                    .map_err(|err| err.message().to_string());
            }
            let handle = self
                .state
                .supervisor
                .get_session(&session_id)
                .await
                .ok_or_else(|| "session not found".to_string())?;
            let (tx, rx) = oneshot::channel();
            handle
                .send(SessionCommand::GetPlanFile { name, reply: tx })
                .await
                .map_err(|_| "session actor unavailable".to_string())?;
            tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .map_err(|_| "plan file request timed out".to_string())?
                .map_err(|_| "actor dropped plan file reply".to_string())
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
                .map_err(|err| match err {
                    NativeOpenServiceError::Unsupported { reason } => {
                        reason.unwrap_or_else(|| "native desktop unavailable".to_string())
                    }
                    NativeOpenServiceError::NoAttentionSessions => {
                        "no sessions are waiting for operator input".to_string()
                    }
                    NativeOpenServiceError::SessionNotFound => "session not found".to_string(),
                    NativeOpenServiceError::SessionExited => {
                        "session has already exited".to_string()
                    }
                    NativeOpenServiceError::Internal(message) => message,
                })
        })
    }

    fn open_attention_group(
        &self,
        max_sessions: usize,
        current_session_ids: Vec<String>,
        focus: bool,
    ) -> BoxFuture<'_, Result<NativeAttentionGroupOpenResponse, String>> {
        Box::pin(async move {
            open_native_attention_group_for_host(
                &self.state,
                "localhost",
                NativeAttentionGroupOpenRequest {
                    max_sessions: Some(max_sessions),
                    current_session_ids,
                    focus,
                },
            )
            .await
            .map_err(|error| match error {
                NativeOpenServiceError::Unsupported { reason } => {
                    reason.unwrap_or_else(|| "native desktop unavailable".to_string())
                }
                NativeOpenServiceError::NoAttentionSessions => {
                    "no sessions are waiting for operator input".to_string()
                }
                NativeOpenServiceError::SessionNotFound => "session not found".to_string(),
                NativeOpenServiceError::SessionExited => "session has already exited".to_string(),
                NativeOpenServiceError::Internal(message) => message,
            })
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        group: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let path = path.map(str::to_owned);
        let group = group.map(str::to_owned);
        Box::pin(async move {
            list_dirs_service(&self.state, path.as_deref(), managed_only, group.as_deref())
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn list_repo_dirs(&self) -> BoxFuture<'_, Result<DirRepoSearchResponse, String>> {
        Box::pin(async move {
            list_repo_search_entries_service()
                .await
                .map_err(|err| err.to_string())
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
                .map_err(|err| err.to_string())
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
                DirGroupMembershipUpdateRequest { path, add, remove },
            )
            .await
            .map_err(|err| err.to_string())
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
                .map_err(|err| err.message().to_string());
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
                .map_err(|err| err.message().to_string());
            }
            if dirs.is_empty() {
                return Err("dirs must not be empty".to_string());
            }
            let total = dirs.len();
            let (batch_id, batch_label, batch_created_at, prompt_excerpt) =
                new_batch_context(total, initial_request.as_deref());
            let tasks = dirs.into_iter().enumerate().map(|(index, cwd)| {
                let supervisor = state.supervisor.clone();
                let initial_request = initial_request.clone();
                let batch = session_batch_membership(
                    batch_id.clone(),
                    batch_label.clone(),
                    index,
                    total,
                    batch_created_at,
                    prompt_excerpt.clone(),
                );
                async move {
                    let created = supervisor
                        .create_session_with_batch(
                            None,
                            Some(cwd.clone()),
                            Some(spawn_tool),
                            initial_request,
                            Some(batch),
                        )
                        .await;
                    create_sessions_batch_result(index, cwd, created)
                }
            });
            Ok(CreateSessionsBatchResponse {
                results: futures::future::join_all(tasks).await,
            })
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
                .map_err(|err| err.message.unwrap_or(err.code))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swimmers::config::Config;
    use swimmers::session::supervisor::SessionSupervisor;
    use swimmers::thought::protocol::SyncRequestSequence;
    use tokio::sync::RwLock;

    fn test_state() -> Arc<AppState> {
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
            file_store: swimmers::api::once_lock_with(None),
            bridge_health: Arc::new(swimmers::thought::health::BridgeHealthState::new_with_tick(
                Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: swimmers::host_actions::RepoActionTracker::default(),
        })
    }

    #[tokio::test]
    async fn fetch_sessions_returns_empty_list() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_sessions().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fetch_native_status_returns_ok() {
        let api = InProcessApi::new(test_state());
        let result = api.fetch_native_status().await;
        assert!(result.is_ok());
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
}
