use super::*;

impl<C: TuiApi> App<C> {
    #[allow(dead_code)]
    pub(crate) fn refresh(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback(layout, false);
    }

    pub(crate) fn refresh_initial_frame(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback_with_mode(layout, false, true);
    }

    pub(crate) fn manual_refresh(&mut self, _layout: WorkspaceLayout) {
        self.pending_refresh = None;
        self.spawn_background_refresh_with_policy(true, true);
    }

    #[allow(dead_code)]
    pub(crate) fn refresh_with_feedback(
        &mut self,
        layout: WorkspaceLayout,
        show_success_message: bool,
    ) {
        self.refresh_with_feedback_with_mode(layout, show_success_message, false);
    }

    fn refresh_with_feedback_with_mode(
        &mut self,
        layout: WorkspaceLayout,
        show_success_message: bool,
        initial_frame: bool,
    ) {
        let snapshot_result = self.fetch_snapshot_for_refresh_mode(initial_frame);
        if self.apply_foreground_snapshot_result(snapshot_result, layout, show_success_message) {
            self.refresh_foreground_metadata();
        }
        self.last_refresh = Some(Instant::now());
    }

    fn fetch_snapshot_for_refresh_mode(
        &self,
        initial_frame: bool,
    ) -> Result<SessionListResponse, String> {
        if initial_frame {
            self.runtime
                .block_on(self.client.fetch_session_snapshot_for_initial_frame())
        } else {
            self.runtime.block_on(self.client.fetch_session_snapshot())
        }
    }

    fn apply_foreground_snapshot_result(
        &mut self,
        snapshot_result: Result<SessionListResponse, String>,
        layout: WorkspaceLayout,
        show_success_message: bool,
    ) -> bool {
        match snapshot_result {
            Ok(snapshot) => {
                self.apply_successful_foreground_refresh(snapshot, layout, show_success_message);
                true
            }
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                self.api_refresh_health.record_failure();
                false
            }
        }
    }

    fn apply_successful_foreground_refresh(
        &mut self,
        snapshot: SessionListResponse,
        layout: WorkspaceLayout,
        show_success_message: bool,
    ) {
        let SessionListResponse {
            sessions,
            environments,
            fleet_presets,
            ..
        } = snapshot;
        self.apply_environment_metadata(environments, fleet_presets);
        self.sync_repo_themes(&sessions, false);
        self.refresh_mermaid_artifacts(&sessions);
        self.reconcile_thought_log_sessions(&sessions);
        self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
        self.merge_sessions(sessions, layout.overview_field);
        self.last_successful_refresh = Some(Instant::now());
        self.api_refresh_health.record_success();
        self.maybe_show_refresh_success_message(show_success_message);
        self.refresh_daemon_defaults_status_once();
    }

    fn refresh_foreground_metadata(&mut self) {
        if let Ok(health) = self.runtime.block_on(self.client.fetch_backend_health()) {
            self.backend_health = Some(health);
        }
        match self.runtime.block_on(self.client.fetch_native_status()) {
            Ok(status) => {
                self.native_status = Some(status);
            }
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
            }
        }
    }

    fn apply_environment_metadata(
        &mut self,
        environments: Vec<EnvironmentSummary>,
        fleet_presets: Vec<FleetLensPreset>,
    ) {
        self.environments = environments;
        self.fleet_presets = if fleet_presets.is_empty() {
            swimmers::fleet_lens::build_fleet_lens_presets(Vec::new())
        } else {
            fleet_presets
        };
    }

    fn refresh_daemon_defaults_status_once(&mut self) {
        if self.daemon_defaults_status != DaemonDefaultsStatus::Unknown {
            return;
        }
        if let Ok(response) = self.runtime.block_on(self.client.fetch_thought_config()) {
            self.daemon_defaults_status =
                DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref());
        }
    }

    pub(crate) fn spawn_background_refresh(&mut self, show_success_message: bool) {
        self.spawn_background_refresh_with_policy(show_success_message, false);
    }

    fn spawn_background_refresh_with_policy(
        &mut self,
        show_success_message: bool,
        force_asset_refresh: bool,
    ) {
        let client = Arc::clone(&self.client);
        let check_daemon_defaults = self.daemon_defaults_status == DaemonDefaultsStatus::Unknown;
        let mermaid_contexts = self
            .session_mermaid_cache
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.context.clone()))
            .collect::<HashMap<_, _>>();
        let skill_contexts = self
            .session_skill_cache
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.context.clone()))
            .collect::<HashMap<_, _>>();
        let (tx, rx) = oneshot::channel();
        self.pending_refresh = Some(rx);
        self.runtime.spawn(async move {
            let sessions_result = client.fetch_session_snapshot().await;
            let backend_health = client.fetch_backend_health().await;

            let (mermaid_artifacts, session_skills, native_status) = match &sessions_result {
                Ok(snapshot) => {
                    let sessions = &snapshot.sessions;
                    let mermaid_futs: Vec<_> = sessions
                        .iter()
                        .filter(|session| {
                            Self::should_refresh_mermaid_with_contexts(
                                &mermaid_contexts,
                                session,
                                force_asset_refresh,
                            )
                        })
                        .map(|s| {
                            let client = Arc::clone(&client);
                            let sid = s.session_id.clone();
                            async move {
                                let result = client.fetch_mermaid_artifact(&sid).await;
                                (sid, result)
                            }
                        })
                        .collect();

                    let mut skill_groups = BTreeMap::<String, (String, Vec<String>)>::new();
                    for session in sessions.iter().filter(|session| {
                        Self::should_refresh_skills_with_contexts(
                            &skill_contexts,
                            session,
                            force_asset_refresh,
                        )
                    }) {
                        let cwd = normalize_path(&session.cwd);
                        let entry = skill_groups
                            .entry(cwd)
                            .or_insert_with(|| (session.session_id.clone(), Vec::new()));
                        entry.1.push(session.session_id.clone());
                    }
                    let skill_futs: Vec<_> = skill_groups
                        .into_values()
                        .map(|(representative_id, session_ids)| {
                            let client = Arc::clone(&client);
                            async move {
                                let result = client.fetch_session_skills(&representative_id).await;
                                let mut out = Vec::new();
                                for session_id in session_ids {
                                    let adjusted = result.clone().map(|mut response| {
                                        response.session_id = session_id.clone();
                                        response
                                    });
                                    out.push((session_id, adjusted));
                                }
                                out
                            }
                        })
                        .collect();

                    let (mermaid_results, skill_results, native_result) = tokio::join!(
                        futures::future::join_all(mermaid_futs),
                        futures::future::join_all(skill_futs),
                        client.fetch_native_status(),
                    );

                    (
                        mermaid_results,
                        skill_results.into_iter().flatten().collect(),
                        Some(native_result),
                    )
                }
                Err(_) => (Vec::new(), Vec::new(), None),
            };

            let daemon_defaults_status = if check_daemon_defaults {
                client.fetch_thought_config().await.ok().map(|response| {
                    DaemonDefaultsStatus::from_defaults(response.daemon_defaults.as_ref())
                })
            } else {
                None
            };

            let _ = tx.send(RefreshResult {
                sessions: sessions_result,
                mermaid_artifacts,
                session_skills,
                backend_health,
                native_status,
                daemon_defaults_status,
                show_success_message,
                force_asset_refresh,
            });
        });
    }

    pub(crate) fn poll_refresh(&mut self, layout: WorkspaceLayout) {
        let Some(rx) = &mut self.pending_refresh else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_refresh = None;
                return;
            }
        };

        self.pending_refresh = None;
        self.apply_refresh_result(result, layout);
    }

    pub(crate) fn apply_refresh_result(&mut self, result: RefreshResult, layout: WorkspaceLayout) {
        match result.sessions {
            Ok(sessions) => self.apply_successful_refresh(
                sessions,
                result.mermaid_artifacts,
                result.session_skills,
                result.force_asset_refresh,
                result.show_success_message,
                layout,
            ),
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                self.api_refresh_health.record_failure();
            }
        }

        self.apply_refresh_metadata(
            result.native_status,
            result.backend_health,
            result.daemon_defaults_status,
        );
        self.last_refresh = Some(Instant::now());
    }

    fn apply_successful_refresh(
        &mut self,
        snapshot: SessionListResponse,
        mermaid_artifacts: Vec<(String, Result<MermaidArtifactResponse, String>)>,
        session_skills: Vec<(String, Result<SessionSkillListResponse, String>)>,
        force_asset_refresh: bool,
        show_success_message: bool,
        layout: WorkspaceLayout,
    ) {
        let SessionListResponse {
            sessions,
            environments,
            fleet_presets,
            ..
        } = snapshot;
        self.apply_environment_metadata(environments, fleet_presets);
        self.sync_repo_themes(&sessions, force_asset_refresh);
        self.apply_refresh_assets(&sessions, mermaid_artifacts, session_skills);
        self.reconcile_thought_log_sessions(&sessions);
        self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
        self.merge_sessions(sessions, layout.overview_field);
        self.last_successful_refresh = Some(Instant::now());
        self.api_refresh_health.record_success();
        self.maybe_show_refresh_success_message(show_success_message);
    }

    fn apply_refresh_assets(
        &mut self,
        sessions: &[SessionSummary],
        mermaid_artifacts: Vec<(String, Result<MermaidArtifactResponse, String>)>,
        session_skills: Vec<(String, Result<SessionSkillListResponse, String>)>,
    ) {
        self.retain_cached_assets(sessions);
        let sessions_by_id = sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();
        for (session_id, artifact_result) in mermaid_artifacts {
            if let Some(session) = sessions_by_id.get(session_id.as_str()) {
                self.apply_mermaid_artifact_result(session, artifact_result);
            }
        }
        for (session_id, skills_result) in session_skills {
            if let Some(session) = sessions_by_id.get(session_id.as_str()) {
                self.apply_session_skill_result(session, skills_result);
            }
        }
        self.rebuild_mermaid_artifacts_from_cache();
    }

    fn maybe_show_refresh_success_message(&mut self, show_success_message: bool) {
        if show_success_message {
            let count = self.entities.len();
            self.set_message(format!("refreshed {count} session{}", pluralize(count)));
        }
    }

    fn apply_refresh_metadata(
        &mut self,
        native_status: Option<Result<NativeDesktopStatusResponse, String>>,
        backend_health: Result<BackendHealthResponse, String>,
        daemon_defaults_status: Option<DaemonDefaultsStatus>,
    ) {
        self.apply_refresh_native_status(native_status);
        self.apply_refresh_backend_health(backend_health);
        self.apply_refresh_daemon_defaults_status(daemon_defaults_status);
    }

    fn apply_refresh_native_status(
        &mut self,
        native_status: Option<Result<NativeDesktopStatusResponse, String>>,
    ) {
        match native_status {
            Some(Ok(status)) => self.native_status = Some(status),
            Some(Err(err)) => self.set_message(self.refresh_error_message(err)),
            None => {}
        }
    }

    fn apply_refresh_backend_health(
        &mut self,
        backend_health: Result<BackendHealthResponse, String>,
    ) {
        if let Ok(health) = backend_health {
            self.backend_health = Some(health);
        }
    }

    fn apply_refresh_daemon_defaults_status(&mut self, status: Option<DaemonDefaultsStatus>) {
        if let Some(status) = status {
            self.daemon_defaults_status = status;
        }
    }
}
