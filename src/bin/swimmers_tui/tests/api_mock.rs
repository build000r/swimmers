type CreateBatchCall = (Vec<String>, SpawnTool, Option<String>, Option<String>);

#[derive(Default)]
struct MockApiState {
    fetch_session_snapshot_results: VecDeque<Result<SessionListResponse, String>>,
    fetch_sessions_results: VecDeque<Result<Vec<SessionSummary>, String>>,
    backend_health_results: VecDeque<Result<BackendHealthResponse, String>>,
    fetch_thought_config_results: VecDeque<Result<ThoughtConfigResponse, String>>,
    update_thought_config_results: VecDeque<Result<ThoughtConfig, String>>,
    test_thought_config_results: VecDeque<Result<ThoughtConfigTestResponse, String>>,
    refresh_openrouter_candidates_results: VecDeque<Result<Vec<String>, String>>,
    mermaid_artifact_results: VecDeque<Result<MermaidArtifactResponse, String>>,
    session_skill_results: VecDeque<Result<SessionSkillListResponse, String>>,
    plan_file_results: VecDeque<Result<PlanFileResponse, String>>,
    native_status_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    set_native_app_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    set_native_mode_results: VecDeque<Result<NativeDesktopStatusResponse, String>>,
    publish_selection_results: VecDeque<Result<(), String>>,
    open_session_results: VecDeque<Result<NativeDesktopOpenResponse, String>>,
    open_attention_group_results: VecDeque<Result<NativeAttentionGroupOpenResponse, String>>,
    list_dirs_results: VecDeque<Result<DirListResponse, String>>,
    list_repo_dirs_results: VecDeque<Result<DirRepoSearchResponse, String>>,
    list_repo_dirs_delay: Option<Duration>,
    update_dir_group_memberships_results:
        VecDeque<Result<DirGroupMembershipUpdateResponse, String>>,
    start_repo_action_results: VecDeque<Result<DirRepoActionResponse, String>>,
    overlay_plans_results: VecDeque<Result<Vec<PlanPanelEntry>, String>>,
    create_session_results: VecDeque<Result<CreateSessionResponse, String>>,
    adopt_session_results: VecDeque<Result<AdoptSessionResponse, String>>,
    create_sessions_batch_results: VecDeque<Result<CreateSessionsBatchResponse, String>>,
    send_group_input_results: VecDeque<Result<SessionGroupInputResponse, String>>,
    update_thought_config_calls: Vec<ThoughtConfig>,
    test_thought_config_calls: Vec<ThoughtConfig>,
    mermaid_artifact_calls: Vec<String>,
    session_skill_calls: Vec<String>,
    native_status_calls: usize,
    set_native_app_calls: Vec<NativeDesktopApp>,
    set_native_mode_calls: Vec<GhosttyOpenMode>,
    publish_calls: Vec<Option<String>>,
    open_calls: Vec<String>,
    open_attention_group_calls: Vec<(usize, Vec<String>, bool, bool, AttentionGroupLayout)>,
    list_calls: Vec<(Option<String>, bool)>,
    list_targets: Vec<Option<String>>,
    list_repo_dirs_calls: usize,
    update_dir_group_memberships_calls: Vec<(String, Vec<String>, Vec<String>)>,
    start_repo_action_calls: Vec<(String, RepoActionKind)>,
    create_calls: Vec<(String, SpawnTool, Option<String>, Option<String>)>,
    adopt_calls: Vec<(String, Option<String>)>,
    create_batch_calls: Vec<CreateBatchCall>,
    send_group_input_calls: Vec<(Vec<String>, String)>,
    backend_health_calls: usize,
}

#[derive(Clone, Default)]
struct MockApi {
    state: Arc<Mutex<MockApiState>>,
}

impl MockApi {
    fn new() -> Self {
        Self::default()
    }

    fn push_fetch_sessions(&self, result: Result<Vec<SessionSummary>, String>) {
        self.state
            .lock()
            .unwrap()
            .fetch_sessions_results
            .push_back(result);
    }

    fn push_fetch_session_snapshot(&self, result: Result<SessionListResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .fetch_session_snapshot_results
            .push_back(result);
    }

    fn push_backend_health(&self, result: Result<BackendHealthResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .backend_health_results
            .push_back(result);
    }

    fn push_mermaid_artifact(&self, result: Result<MermaidArtifactResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .mermaid_artifact_results
            .push_back(result);
    }

    fn push_session_skills(&self, result: Result<SessionSkillListResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .session_skill_results
            .push_back(result);
    }

    fn push_fetch_thought_config(&self, result: Result<ThoughtConfigResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .fetch_thought_config_results
            .push_back(result);
    }

    fn push_update_thought_config(&self, result: Result<ThoughtConfig, String>) {
        self.state
            .lock()
            .unwrap()
            .update_thought_config_results
            .push_back(result);
    }

    fn push_test_thought_config(&self, result: Result<ThoughtConfigTestResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .test_thought_config_results
            .push_back(result);
    }

    fn push_refresh_openrouter_candidates(&self, result: Result<Vec<String>, String>) {
        self.state
            .lock()
            .unwrap()
            .refresh_openrouter_candidates_results
            .push_back(result);
    }

    fn push_plan_file(&self, result: Result<PlanFileResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .plan_file_results
            .push_back(result);
    }

    fn push_native_status(&self, result: Result<NativeDesktopStatusResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .native_status_results
            .push_back(result);
    }

    fn push_set_native_app(&self, result: Result<NativeDesktopStatusResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .set_native_app_results
            .push_back(result);
    }

    fn push_set_native_mode(&self, result: Result<NativeDesktopStatusResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .set_native_mode_results
            .push_back(result);
    }

    fn push_list_dirs(&self, result: Result<DirListResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .list_dirs_results
            .push_back(result);
    }

    fn push_list_repo_dirs(&self, result: Result<DirRepoSearchResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .list_repo_dirs_results
            .push_back(result);
    }

    fn set_list_repo_dirs_delay(&self, delay: Duration) {
        self.state.lock().unwrap().list_repo_dirs_delay = Some(delay);
    }

    fn push_update_dir_group_memberships(
        &self,
        result: Result<DirGroupMembershipUpdateResponse, String>,
    ) {
        self.state
            .lock()
            .unwrap()
            .update_dir_group_memberships_results
            .push_back(result);
    }

    fn push_create_session(&self, result: Result<CreateSessionResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .create_session_results
            .push_back(result);
    }

    fn push_adopt_session(&self, result: Result<AdoptSessionResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .adopt_session_results
            .push_back(result);
    }

    fn push_create_sessions_batch(&self, result: Result<CreateSessionsBatchResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .create_sessions_batch_results
            .push_back(result);
    }

    fn push_send_group_input(&self, result: Result<SessionGroupInputResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .send_group_input_results
            .push_back(result);
    }

    fn push_start_repo_action(&self, result: Result<DirRepoActionResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .start_repo_action_results
            .push_back(result);
    }

    fn push_overlay_plans(&self, result: Result<Vec<PlanPanelEntry>, String>) {
        self.state
            .lock()
            .unwrap()
            .overlay_plans_results
            .push_back(result);
    }

    fn push_open_session(&self, result: Result<NativeDesktopOpenResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .open_session_results
            .push_back(result);
    }

    fn push_open_attention_group(&self, result: Result<NativeAttentionGroupOpenResponse, String>) {
        self.state
            .lock()
            .unwrap()
            .open_attention_group_results
            .push_back(result);
    }

    fn push_publish_selection(&self, result: Result<(), String>) {
        self.state
            .lock()
            .unwrap()
            .publish_selection_results
            .push_back(result);
    }

    fn list_calls(&self) -> Vec<(Option<String>, bool)> {
        self.state.lock().unwrap().list_calls.clone()
    }

    fn list_targets(&self) -> Vec<Option<String>> {
        self.state.lock().unwrap().list_targets.clone()
    }

    fn list_repo_dirs_calls(&self) -> usize {
        self.state.lock().unwrap().list_repo_dirs_calls
    }

    fn create_calls(&self) -> Vec<(String, SpawnTool, Option<String>)> {
        self.state
            .lock()
            .unwrap()
            .create_calls
            .iter()
            .map(|(cwd, tool, _target, request)| (cwd.clone(), *tool, request.clone()))
            .collect()
    }

    fn create_batch_calls(&self) -> Vec<(Vec<String>, SpawnTool, Option<String>)> {
        self.state
            .lock()
            .unwrap()
            .create_batch_calls
            .iter()
            .map(|(dirs, tool, _target, request)| (dirs.clone(), *tool, request.clone()))
            .collect()
    }

    fn send_group_input_calls(&self) -> Vec<(Vec<String>, String)> {
        self.state.lock().unwrap().send_group_input_calls.clone()
    }

    fn update_dir_group_memberships_calls(&self) -> Vec<(String, Vec<String>, Vec<String>)> {
        self.state
            .lock()
            .unwrap()
            .update_dir_group_memberships_calls
            .clone()
    }

    fn create_calls_with_targets(
        &self,
    ) -> Vec<(String, SpawnTool, Option<String>, Option<String>)> {
        self.state.lock().unwrap().create_calls.clone()
    }

    fn adopt_calls(&self) -> Vec<(String, Option<String>)> {
        self.state.lock().unwrap().adopt_calls.clone()
    }

    fn create_batch_calls_with_targets(&self) -> Vec<CreateBatchCall> {
        self.state.lock().unwrap().create_batch_calls.clone()
    }

    fn start_repo_action_calls(&self) -> Vec<(String, RepoActionKind)> {
        self.state.lock().unwrap().start_repo_action_calls.clone()
    }

    fn publish_calls(&self) -> Vec<Option<String>> {
        self.state.lock().unwrap().publish_calls.clone()
    }

    fn open_calls(&self) -> Vec<String> {
        self.state.lock().unwrap().open_calls.clone()
    }

    fn open_attention_group_calls(
        &self,
    ) -> Vec<(usize, Vec<String>, bool, bool, AttentionGroupLayout)> {
        self.state
            .lock()
            .unwrap()
            .open_attention_group_calls
            .clone()
    }

    fn update_thought_config_calls(&self) -> Vec<ThoughtConfig> {
        self.state
            .lock()
            .unwrap()
            .update_thought_config_calls
            .clone()
    }

    fn test_thought_config_calls(&self) -> Vec<ThoughtConfig> {
        self.state.lock().unwrap().test_thought_config_calls.clone()
    }

    fn native_status_calls(&self) -> usize {
        self.state.lock().unwrap().native_status_calls
    }

    fn backend_health_calls(&self) -> usize {
        self.state.lock().unwrap().backend_health_calls
    }

    fn mermaid_artifact_calls(&self) -> Vec<String> {
        self.state.lock().unwrap().mermaid_artifact_calls.clone()
    }

    fn session_skill_calls(&self) -> Vec<String> {
        self.state.lock().unwrap().session_skill_calls.clone()
    }

    fn set_native_app_calls(&self) -> Vec<NativeDesktopApp> {
        self.state.lock().unwrap().set_native_app_calls.clone()
    }

    fn set_native_mode_calls(&self) -> Vec<GhosttyOpenMode> {
        self.state.lock().unwrap().set_native_mode_calls.clone()
    }
}

impl TuiApi for MockApi {
    fn fetch_session_snapshot(&self) -> BoxFuture<'_, Result<SessionListResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let result = {
                let mut state = state.lock().unwrap();
                state
                    .fetch_session_snapshot_results
                    .pop_front()
                    .map_or_else(
                        || {
                            state
                                .fetch_sessions_results
                                .pop_front()
                                .unwrap_or_else(|| Ok(Vec::new()))
                                .map(|sessions| SessionListResponse {
                                    fleet_lens: swimmers::fleet_lens::build_fleet_lens_summary(
                                        &sessions,
                                    ),
                                    fleet_presets:
                                        swimmers::fleet_lens::build_fleet_lens_presets(Vec::new()),
                                    sessions,
                                    version: 0,
                                    repo_themes: Default::default(),
                                    environments: Vec::new(),
                                })
                        },
                        |result| result,
                    )
            };
            result
        })
    }

    fn fetch_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionSummary>, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .fetch_sessions_results
                .pop_front()
                .unwrap_or_else(|| Ok(Vec::new()))
        })
    }

    fn fetch_backend_health(&self) -> BoxFuture<'_, Result<BackendHealthResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.backend_health_calls += 1;
            state
                .backend_health_results
                .pop_front()
                .unwrap_or_else(|| Ok(healthy_backend_health()))
        })
    }

    fn fetch_thought_config(&self) -> BoxFuture<'_, Result<ThoughtConfigResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .fetch_thought_config_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(ThoughtConfigResponse {
                        config: ThoughtConfig::default(),
                        daemon_defaults: None,
                        ui: swimmers::types::ThoughtConfigUiMetadata::default(),
                    })
                })
        })
    }

    fn update_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfig, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.update_thought_config_calls.push(config.clone());
            state
                .update_thought_config_results
                .pop_front()
                .unwrap_or(Ok(config))
        })
    }

    fn test_thought_config(
        &self,
        config: ThoughtConfig,
    ) -> BoxFuture<'_, Result<ThoughtConfigTestResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.test_thought_config_calls.push(config.clone());
            state
                .test_thought_config_results
                .pop_front()
                .unwrap_or(Ok(ThoughtConfigTestResponse {
                    ok: true,
                    message: "probe succeeded".to_string(),
                    last_backend_error: None,
                    llm_calls: 1,
                }))
        })
    }

    fn refresh_openrouter_candidates(&self) -> BoxFuture<'_, Result<Vec<String>, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .refresh_openrouter_candidates_results
                .pop_front()
                .unwrap_or_else(|| Ok(default_openrouter_candidates()))
        })
    }

    fn fetch_mermaid_artifact(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<MermaidArtifactResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.mermaid_artifact_calls.push(session_id.clone());
            state.mermaid_artifact_results.pop_front().unwrap_or({
                Ok(MermaidArtifactResponse {
                    session_id,
                    available: false,
                    path: None,
                    updated_at: None,
                    source: None,
                    error: None,
                    slice_name: None,
                    plan_files: None,
                })
            })
        })
    }

    fn fetch_session_skills(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<SessionSkillListResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.session_skill_calls.push(session_id.clone());
            state.session_skill_results.pop_front().unwrap_or({
                Ok(SessionSkillListResponse {
                    session_id,
                    source: "sbp".to_string(),
                    cwd: String::new(),
                    available: false,
                    query: None,
                    skills: Vec::new(),
                    issues: Vec::new(),
                    message: Some("no mock result configured".to_string()),
                })
            })
        })
    }

    fn fetch_plan_file(
        &self,
        session_id: &str,
        name: &str,
    ) -> BoxFuture<'_, Result<PlanFileResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        let name = name.to_string();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .plan_file_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(PlanFileResponse {
                        session_id,
                        name,
                        content: None,
                        error: Some("no mock result configured".to_string()),
                    })
                })
        })
    }

    fn fetch_native_status(&self) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut locked = state.lock().unwrap();
            locked.native_status_calls += 1;
            locked.native_status_results.pop_front().unwrap_or_else(|| {
                Ok(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("test".to_string()),
                    app_id: Some(NativeDesktopApp::Iterm),
                    ghostty_mode: None,
                    app: Some(NativeDesktopApp::Iterm.display_name().to_string()),
                    reason: None,
                })
            })
        })
    }

    fn set_native_app(
        &self,
        app: NativeDesktopApp,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.set_native_app_calls.push(app);
            state.set_native_app_results.pop_front().unwrap_or_else(|| {
                Ok(NativeDesktopStatusResponse {
                    supported: true,
                    platform: Some("test".to_string()),
                    app_id: Some(app),
                    ghostty_mode: (app == NativeDesktopApp::Ghostty)
                        .then_some(GhosttyOpenMode::Swap),
                    app: Some(app.display_name().to_string()),
                    reason: None,
                })
            })
        })
    }

    fn set_native_mode(
        &self,
        mode: GhosttyOpenMode,
    ) -> BoxFuture<'_, Result<NativeDesktopStatusResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.set_native_mode_calls.push(mode);
            state
                .set_native_mode_results
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(NativeDesktopStatusResponse {
                        supported: true,
                        platform: Some("test".to_string()),
                        app_id: Some(NativeDesktopApp::Ghostty),
                        ghostty_mode: Some(mode),
                        app: Some(NativeDesktopApp::Ghostty.display_name().to_string()),
                        reason: None,
                    })
                })
        })
    }

    fn publish_selection(&self, session_id: Option<&str>) -> BoxFuture<'_, Result<(), String>> {
        let state = self.state.clone();
        let session_id = session_id.map(|value| value.to_string());
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.publish_calls.push(session_id);
            state
                .publish_selection_results
                .pop_front()
                .unwrap_or(Ok(()))
        })
    }

    fn open_session(
        &self,
        session_id: &str,
    ) -> BoxFuture<'_, Result<NativeDesktopOpenResponse, String>> {
        let state = self.state.clone();
        let session_id = session_id.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.open_calls.push(session_id);
            state
                .open_session_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected open_session".to_string()))
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
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.open_attention_group_calls.push((
                max_sessions,
                current_session_ids,
                focus,
                include_unnumbered_sessions,
                layout,
            ));
            state
                .open_attention_group_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected open_attention_group".to_string()))
        })
    }

    fn list_dirs(
        &self,
        path: Option<&str>,
        managed_only: bool,
        _group: Option<&str>,
        target: Option<&str>,
    ) -> BoxFuture<'_, Result<DirListResponse, String>> {
        let state = self.state.clone();
        let path = path.map(|value| value.to_string());
        let target = target.map(|value| value.to_string());
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.list_calls.push((path, managed_only));
            state.list_targets.push(target);
            state
                .list_dirs_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected list_dirs".to_string()))
        })
    }

    fn list_repo_dirs(&self) -> BoxFuture<'_, Result<DirRepoSearchResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let (delay, result) = {
                let mut state = state.lock().unwrap();
                state.list_repo_dirs_calls += 1;
                (
                    state.list_repo_dirs_delay,
                    state.list_repo_dirs_results.pop_front().unwrap_or_else(|| {
                        Ok(DirRepoSearchResponse {
                            roots: Vec::new(),
                            entries: Vec::new(),
                        })
                    }),
                )
            };
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            result
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
            let mut state = state.lock().unwrap();
            state
                .update_dir_group_memberships_calls
                .push((path, add, remove));
            state
                .update_dir_group_memberships_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected update_dir_group_memberships".to_string()))
        })
    }

    fn start_repo_action(
        &self,
        path: &str,
        kind: RepoActionKind,
    ) -> BoxFuture<'_, Result<DirRepoActionResponse, String>> {
        let state = self.state.clone();
        let path = path.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.start_repo_action_calls.push((path, kind));
            state
                .start_repo_action_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected start_repo_action".to_string()))
        })
    }

    fn fetch_overlay_plans(&self) -> BoxFuture<'_, Result<Vec<PlanPanelEntry>, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            state
                .lock()
                .unwrap()
                .overlay_plans_results
                .pop_front()
                .unwrap_or_else(|| Ok(Vec::new()))
        })
    }

    fn create_session(
        &self,
        cwd: &str,
        spawn_tool: SpawnTool,
        launch_target: Option<String>,
        initial_request: Option<String>,
    ) -> BoxFuture<'_, Result<CreateSessionResponse, String>> {
        let state = self.state.clone();
        let cwd = cwd.to_string();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state
                .create_calls
                .push((cwd, spawn_tool, launch_target, initial_request));
            state
                .create_session_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected create_session".to_string()))
        })
    }

    fn adopt_session(
        &self,
        tmux_name: &str,
        session_id: Option<&str>,
    ) -> BoxFuture<'_, Result<AdoptSessionResponse, String>> {
        let state = self.state.clone();
        let tmux_name = tmux_name.to_string();
        let session_id = session_id.map(str::to_string);
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.adopt_calls.push((tmux_name, session_id));
            state
                .adopt_session_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected adopt_session".to_string()))
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
            let mut state = state.lock().unwrap();
            state
                .create_batch_calls
                .push((dirs, spawn_tool, launch_target, initial_request));
            state
                .create_sessions_batch_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected create_sessions_batch".to_string()))
        })
    }

    fn send_group_input(
        &self,
        session_ids: Vec<String>,
        text: String,
    ) -> BoxFuture<'_, Result<SessionGroupInputResponse, String>> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            state.send_group_input_calls.push((session_ids, text));
            state
                .send_group_input_results
                .pop_front()
                .unwrap_or_else(|| Err("unexpected send_group_input".to_string()))
        })
    }
}
