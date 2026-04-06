use super::*;
use swimmers::openrouter_models::should_rotate_openrouter_model;

pub(crate) struct RefreshResult {
    pub(crate) sessions: Result<Vec<SessionSummary>, String>,
    pub(crate) mermaid_artifacts: Vec<(String, Result<MermaidArtifactResponse, String>)>,
    pub(crate) native_status: Option<Result<NativeDesktopStatusResponse, String>>,
    pub(crate) show_success_message: bool,
}

pub(crate) struct PendingSelectionPublicationResult {
    pub(crate) session_id: Option<String>,
    pub(crate) response: Result<(), String>,
}

pub(crate) struct ThoughtConfigActionOutcome {
    pub(crate) message: String,
    pub(crate) updated_config: Option<ThoughtConfig>,
    pub(crate) openrouter_candidates: Option<Vec<String>>,
    pub(crate) close_editor: bool,
    pub(crate) refresh_sessions: bool,
}

pub(crate) enum PendingInteractionResult {
    OpenPicker {
        x: u16,
        y: u16,
        response: Result<DirListResponse, String>,
    },
    ReloadPicker {
        managed_only: bool,
        response: Result<DirListResponse, String>,
    },
    CreateSession {
        field: Rect,
        response: Result<CreateSessionResponse, String>,
    },
    OpenSession {
        label: String,
        response: Result<NativeDesktopOpenResponse, String>,
    },
    ToggleNativeApp {
        next_app: NativeDesktopApp,
        response: Result<NativeDesktopStatusResponse, String>,
    },
    ToggleGhosttyMode {
        next_mode: GhosttyOpenMode,
        response: Result<NativeDesktopStatusResponse, String>,
    },
    OpenThoughtConfig {
        response: Result<ThoughtConfigResponse, String>,
    },
    TestThoughtConfig {
        outcome: ThoughtConfigActionOutcome,
    },
    SaveThoughtConfig {
        outcome: ThoughtConfigActionOutcome,
    },
}

pub(crate) struct App<C: TuiApi> {
    pub(crate) runtime: Runtime,
    pub(crate) client: Arc<C>,
    pub(crate) artifact_opener: Arc<dyn ArtifactOpener>,
    pub(crate) commit_launcher: Arc<dyn CommitLauncher>,
    pub(crate) entities: Vec<SessionEntity>,
    pub(crate) thought_log: Vec<ThoughtLogEntry>,
    pub(crate) thought_filter: ThoughtFilter,
    pub(crate) last_logged_thoughts: HashMap<String, ThoughtFingerprint>,
    pub(crate) mermaid_artifacts: HashMap<String, MermaidArtifactResponse>,
    pub(crate) repo_themes: HashMap<String, RepoTheme>,
    pub(crate) selected_id: Option<String>,
    pub(crate) published_selected_id: Option<String>,
    pub(crate) native_status: Option<NativeDesktopStatusResponse>,
    pub(crate) thought_config_editor: Option<ThoughtConfigEditorState>,
    pub(crate) picker: Option<PickerState>,
    pub(crate) spawn_tool: SpawnTool,
    pub(crate) initial_request: Option<InitialRequestState>,
    pub(crate) fish_bowl_mode: FishBowlMode,
    pub(crate) mermaid_drag: Option<MermaidDragState>,
    pub(crate) message: Option<(String, Instant)>,
    pub(crate) last_refresh: Option<Instant>,
    pub(crate) thought_panel_ratio: f32,
    pub(crate) split_drag_active: bool,
    pub(crate) tick: u64,
    pub(crate) pending_refresh: Option<oneshot::Receiver<RefreshResult>>,
    pub(crate) pending_interaction: Option<oneshot::Receiver<PendingInteractionResult>>,
    pub(crate) pending_selection_publication:
        Option<oneshot::Receiver<PendingSelectionPublicationResult>>,
    pub(crate) queued_selection_publication: Option<(Option<String>, bool)>,
}

impl<C: TuiApi> App<C> {
    pub(crate) fn new(runtime: Runtime, client: C) -> Self {
        Self::with_helpers(
            runtime,
            client,
            Arc::new(SystemArtifactOpener),
            Arc::new(SystemCommitLauncher),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn with_artifact_opener(
        runtime: Runtime,
        client: C,
        artifact_opener: Arc<dyn ArtifactOpener>,
    ) -> Self {
        Self::with_helpers(
            runtime,
            client,
            artifact_opener,
            Arc::new(SystemCommitLauncher),
        )
    }

    pub(crate) fn with_helpers(
        runtime: Runtime,
        client: C,
        artifact_opener: Arc<dyn ArtifactOpener>,
        commit_launcher: Arc<dyn CommitLauncher>,
    ) -> Self {
        Self {
            runtime,
            client: Arc::new(client),
            artifact_opener,
            commit_launcher,
            entities: Vec::new(),
            thought_log: Vec::new(),
            thought_filter: ThoughtFilter::default(),
            last_logged_thoughts: HashMap::new(),
            mermaid_artifacts: HashMap::new(),
            repo_themes: HashMap::new(),
            selected_id: None,
            published_selected_id: None,
            native_status: None,
            thought_config_editor: None,
            picker: None,
            spawn_tool: SpawnTool::Codex,
            initial_request: None,
            fish_bowl_mode: FishBowlMode::Aquarium,
            mermaid_drag: None,
            message: None,
            last_refresh: None,
            thought_panel_ratio: THOUGHT_RAIL_DEFAULT_RATIO,
            split_drag_active: false,
            tick: 0,
            pending_refresh: None,
            pending_interaction: None,
            pending_selection_publication: None,
            queued_selection_publication: None,
        }
    }

    pub(crate) fn layout_for_terminal(&self, width: u16, height: u16) -> WorkspaceLayout {
        WorkspaceLayout::for_terminal_with_ratio(width, height, self.thought_panel_ratio)
    }

    pub(crate) fn set_message(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self
            .message
            .as_ref()
            .map(|(existing, _)| existing == &message)
            .unwrap_or(false)
        {
            return;
        }
        self.message = Some((message, Instant::now()));
    }

    pub(crate) fn visible_message(&self) -> Option<&str> {
        self.message.as_ref().and_then(|(message, at)| {
            if at.elapsed() <= MESSAGE_TTL {
                Some(message.as_str())
            } else {
                None
            }
        })
    }

    pub(crate) fn should_refresh(&self) -> bool {
        self.last_refresh
            .map(|last| last.elapsed() >= REFRESH_INTERVAL)
            .unwrap_or(true)
    }

    pub(crate) fn native_status_text(&self) -> String {
        match &self.native_status {
            Some(status) => {
                let app_label = status.app.as_deref().unwrap_or("available");
                let mode_suffix = status
                    .ghostty_mode
                    .filter(|_| self.current_native_app() == NativeDesktopApp::Ghostty)
                    .map(|mode| format!(" ({})", mode.label()))
                    .unwrap_or_default();
                if status.supported {
                    format!("native open: {app_label}{mode_suffix}")
                } else {
                    format!(
                        "native open: {app_label}{mode_suffix} unavailable: {}",
                        status.reason.as_deref().unwrap_or("unknown reason")
                    )
                }
            }
            None => "native open: checking".to_string(),
        }
    }

    pub(crate) fn header_right_text(&self) -> String {
        self.thought_filter
            .tmux_name
            .as_deref()
            .map(|tmux_name| format!("num={tmux_name} | {}", self.native_status_text()))
            .unwrap_or_else(|| self.native_status_text())
    }

    pub(crate) fn native_status_rect(&self, width: u16) -> Option<Rect> {
        let max_right_width = width.saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let text_width = display_width(&right_text);
        if text_width == 0 {
            return None;
        }

        Some(Rect {
            x: width.saturating_sub(text_width).saturating_sub(2),
            y: 1,
            width: text_width,
            height: 1,
        })
    }

    pub(crate) fn ghostty_mode_rect(&self, width: u16) -> Option<Rect> {
        if self.current_native_app() != NativeDesktopApp::Ghostty {
            return None;
        }

        let max_right_width = width.saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let marker = format!("({})", self.current_ghostty_mode().label());
        let marker_idx = right_text.find(&marker)?;
        let prefix_width = display_width(&right_text[..marker_idx]);
        let marker_width = display_width(&marker);
        let full_width = display_width(&right_text);
        let x = width
            .saturating_sub(full_width)
            .saturating_sub(2)
            .saturating_add(prefix_width);

        Some(Rect {
            x,
            y: 1,
            width: marker_width,
            height: 1,
        })
    }

    fn current_native_app(&self) -> NativeDesktopApp {
        self.native_status
            .as_ref()
            .and_then(|status| {
                status
                    .app_id
                    .or_else(|| status.app.as_deref().map(NativeDesktopApp::from_env_value))
            })
            .unwrap_or(NativeDesktopApp::Iterm)
    }

    fn current_ghostty_mode(&self) -> GhosttyOpenMode {
        self.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode)
            .unwrap_or(GhosttyOpenMode::Swap)
    }

    pub(crate) fn toggle_native_app(&mut self) {
        let next_app = self.current_native_app().toggle();
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!(
            "switching native open target to {}...",
            next_app.display_name()
        ));
        self.runtime.spawn(async move {
            let response = client.set_native_app(next_app).await;
            let _ = tx.send(PendingInteractionResult::ToggleNativeApp { next_app, response });
        });
    }

    pub(crate) fn toggle_ghostty_mode(&mut self) {
        if self.current_native_app() != NativeDesktopApp::Ghostty {
            return;
        }

        let next_mode = self.current_ghostty_mode().toggle();
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message(format!(
            "switching Ghostty preview mode to {}...",
            next_mode.label()
        ));
        self.runtime.spawn(async move {
            let response = client.set_native_mode(next_mode).await;
            let _ = tx.send(PendingInteractionResult::ToggleGhosttyMode {
                next_mode,
                response,
            });
        });
    }

    pub(crate) fn visible_entities(&self) -> Vec<&SessionEntity> {
        self.entities
            .iter()
            .filter(|entity| self.thought_filter.matches_session(&entity.session))
            .collect()
    }

    fn begin_pending_interaction(&mut self) -> Option<oneshot::Sender<PendingInteractionResult>> {
        if self.pending_interaction.is_some() {
            self.set_message("wait for the current action to finish");
            return None;
        }

        let (tx, rx) = oneshot::channel();
        self.pending_interaction = Some(rx);
        Some(tx)
    }

    pub(crate) fn publish_selection(&mut self, session_id: Option<String>, force: bool) {
        if !force && session_id == self.published_selected_id {
            return;
        }

        match self
            .runtime
            .block_on(self.client.publish_selection(session_id.as_deref()))
        {
            Ok(()) => {
                self.published_selected_id = session_id;
            }
            Err(err) => self.set_message(err),
        }
    }

    pub(crate) fn sync_selection_publication(&mut self) {
        self.queue_selection_publication(self.selected_id.clone(), false);
    }

    pub(crate) fn clear_published_selection(&mut self) {
        self.publish_selection(None, true);
    }

    fn queue_selection_publication(&mut self, session_id: Option<String>, force: bool) {
        self.queued_selection_publication = Some((session_id, force));
        self.maybe_spawn_selection_publication();
    }

    fn maybe_spawn_selection_publication(&mut self) {
        if self.pending_selection_publication.is_some() {
            return;
        }

        let Some((session_id, force)) = self.queued_selection_publication.take() else {
            return;
        };
        if !force && session_id == self.published_selected_id {
            return;
        }

        let (tx, rx) = oneshot::channel();
        self.pending_selection_publication = Some(rx);
        let client = Arc::clone(&self.client);
        self.runtime.spawn(async move {
            let response = client.publish_selection(session_id.as_deref()).await;
            let _ = tx.send(PendingSelectionPublicationResult {
                session_id,
                response,
            });
        });
    }

    pub(crate) fn poll_pending_selection_publication(&mut self) {
        let Some(rx) = &mut self.pending_selection_publication else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_selection_publication = None;
                self.maybe_spawn_selection_publication();
                return;
            }
        };

        self.pending_selection_publication = None;
        match result.response {
            Ok(()) => {
                self.published_selected_id = result.session_id;
            }
            Err(err) => self.set_message(err),
        }
        self.maybe_spawn_selection_publication();
    }

    pub(crate) fn reconcile_selection(&mut self) {
        let selected_visible = self
            .selected_id
            .as_ref()
            .map(|selected| {
                self.entities.iter().any(|entity| {
                    entity.session.session_id == *selected
                        && self.thought_filter.matches_session(&entity.session)
                })
            })
            .unwrap_or(false);

        if !selected_visible {
            self.selected_id = self
                .entities
                .iter()
                .find(|entity| self.thought_filter.matches_session(&entity.session))
                .map(|entity| entity.session.session_id.clone());
        }
    }

    pub(crate) fn trim_thought_log(&mut self, capacity: usize) {
        if capacity == 0 || self.thought_log.len() <= capacity {
            return;
        }

        let drop_count = self.thought_log.len() - capacity;
        self.thought_log.drain(0..drop_count);
    }

    pub(crate) fn upsert_thought_log_entries(
        &mut self,
        entries: impl IntoIterator<Item = ThoughtLogEntry>,
        capacity: usize,
    ) {
        for entry in entries {
            if let Some(index) = self
                .thought_log
                .iter()
                .position(|existing| existing.session_id == entry.session_id)
            {
                self.thought_log.remove(index);
            }
            self.thought_log.push(entry);
        }
        self.thought_log.sort_by(compare_thought_log_entries);
        self.trim_thought_log(capacity);
    }

    #[allow(dead_code)]
    pub(crate) fn visible_thought_entries(&self, capacity: usize) -> Vec<&ThoughtLogEntry> {
        if capacity == 0 {
            return Vec::new();
        }

        let filtered = self
            .thought_log
            .iter()
            .filter(|entry| self.thought_filter.matches(entry))
            .collect::<Vec<_>>();
        let start = filtered.len().saturating_sub(capacity);
        filtered[start..].to_vec()
    }

    pub(crate) fn thought_entry_display_color(&self, entry: &ThoughtLogEntry) -> Color {
        self.entities
            .iter()
            .find(|entity| entity.session.session_id == entry.session_id)
            .map(|entity| session_display_color(&entity.session, &self.repo_themes))
            .unwrap_or(entry.color)
    }

    pub(crate) fn header_repo_summaries(&self) -> Vec<ThoughtRepoSummary> {
        let mut grouped = HashMap::<String, ThoughtRepoSummary>::new();
        for (index, entity) in self.entities.iter().enumerate() {
            let session = &entity.session;
            let Some(label) = path_tail_label(&session.cwd) else {
                continue;
            };
            let cwd = normalize_path(&session.cwd);
            let color = session_display_color(session, &self.repo_themes);

            let summary = grouped
                .entry(cwd.clone())
                .or_insert_with(|| ThoughtRepoSummary {
                    cwd: cwd.clone(),
                    label,
                    count: 0,
                    color,
                    last_seen: index,
                });
            summary.count += 1;
            summary.color = color;
            summary.last_seen = index;
        }

        let mut summaries = grouped.into_values().collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .last_seen
                .cmp(&left.last_seen)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.cwd.cmp(&right.cwd))
        });
        summaries
    }

    #[allow(dead_code)]
    pub(crate) fn active_thought_filter_text(&self) -> String {
        if !self.thought_filter.is_active() {
            return "filter: none".to_string();
        }

        let mut parts = Vec::new();
        if let Some(cwd) = self.thought_filter.cwd.as_deref() {
            parts.push(format!(
                "pwd={}",
                path_tail_label(cwd).unwrap_or_else(|| cwd.to_string())
            ));
        }
        if !self.thought_filter.excluded_cwds.is_empty() {
            let mut hidden = self
                .thought_filter
                .excluded_cwds
                .iter()
                .map(|cwd| path_tail_label(cwd).unwrap_or_else(|| cwd.to_string()))
                .collect::<Vec<_>>();
            hidden.sort();
            parts.push(format!("hide={}", hidden.join(",")));
        }
        if let Some(tmux_name) = self.thought_filter.tmux_name.as_deref() {
            parts.push(format!("num={tmux_name}"));
        }
        format!("filter: {}", parts.join(", "))
    }

    pub(crate) fn set_thought_filter_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = Some(cwd);
        self.thought_filter.excluded_cwds.clear();
        self.thought_filter.filter_out_mode = false;
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_filter_out_mode(&mut self) {
        self.thought_filter.filter_out_mode = !self.thought_filter.filter_out_mode;
        if self.thought_filter.filter_out_mode {
            self.thought_filter.cwd = None;
        } else {
            self.thought_filter.excluded_cwds.clear();
        }
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn toggle_thought_filter_out_cwd(&mut self, cwd: String) {
        self.thought_filter.cwd = None;
        self.thought_filter.filter_out_mode = true;
        if !self.thought_filter.excluded_cwds.insert(cwd.clone()) {
            self.thought_filter.excluded_cwds.remove(&cwd);
        }
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn clear_thought_filters(&mut self) {
        self.thought_filter.clear();
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn refresh_mermaid_artifacts(&mut self, sessions: &[SessionSummary]) {
        let mut next = HashMap::new();
        for session in sessions {
            match self
                .runtime
                .block_on(self.client.fetch_mermaid_artifact(&session.session_id))
            {
                Ok(artifact) if artifact.available => {
                    next.insert(session.session_id.clone(), artifact);
                }
                Ok(_) => {}
                Err(err) => self.set_message(err),
            }
        }
        self.mermaid_artifacts = next;

        if let FishBowlMode::Mermaid(viewer) = &mut self.fish_bowl_mode {
            if let Some(artifact) = self.mermaid_artifacts.get(&viewer.session_id) {
                let path_changed = viewer.path != artifact.path;
                let source_changed = viewer.source != artifact.source;
                let error_changed = viewer.artifact_error != artifact.error;
                viewer.path = artifact.path.clone();
                viewer.source = artifact.source.clone();
                viewer.artifact_error = artifact.error.clone();
                viewer.render_error = None;
                if source_changed || error_changed {
                    viewer.invalidate_source_cache();
                } else if path_changed {
                    viewer.invalidate_viewport_cache();
                }
            }
        }
    }

    pub(crate) fn reconcile_thought_log_sessions(&mut self, sessions: &[SessionSummary]) {
        let session_by_id = sessions
            .iter()
            .map(|session| (session.session_id.as_str(), session))
            .collect::<HashMap<_, _>>();

        self.thought_log
            .retain(|entry| session_by_id.contains_key(entry.session_id.as_str()));
        self.last_logged_thoughts
            .retain(|session_id, _| session_by_id.contains_key(session_id.as_str()));

        for entry in &mut self.thought_log {
            let Some(session) = session_by_id.get(entry.session_id.as_str()) else {
                continue;
            };
            entry.tmux_name = session.tmux_name.clone();
            entry.cwd = normalize_path(&session.cwd);
            entry.pwd_label = path_tail_label(&session.cwd);
            entry.color = session_display_color(session, &self.repo_themes);
            entry.commit_candidate = session.commit_candidate;
        }

        self.thought_log.sort_by(compare_thought_log_entries);
    }

    pub(crate) fn capture_thought_updates(
        &mut self,
        sessions: &[SessionSummary],
        thought_capacity: usize,
    ) {
        let mut pending = Vec::new();
        for session in sessions {
            let Some(thought) = normalize_thought_text(session.thought.as_deref()) else {
                continue;
            };

            let incoming = ThoughtFingerprint {
                thought: thought.clone(),
                updated_at: session.thought_updated_at,
            };
            if !should_append_thought(
                self.last_logged_thoughts.get(&session.session_id),
                &incoming,
            ) {
                continue;
            }

            self.last_logged_thoughts
                .insert(session.session_id.clone(), incoming);
            pending.push(ThoughtLogEntry::from_session(
                session,
                thought,
                &self.repo_themes,
            ));
        }

        pending.sort_by(compare_thought_log_entries);

        if !pending.is_empty() {
            self.upsert_thought_log_entries(pending, thought_capacity);
        }
    }

    pub(crate) fn refresh(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback(layout, false);
    }

    pub(crate) fn manual_refresh(&mut self, _layout: WorkspaceLayout) {
        self.pending_refresh = None;
        self.spawn_background_refresh(true);
    }

    pub(crate) fn refresh_with_feedback(
        &mut self,
        layout: WorkspaceLayout,
        show_success_message: bool,
    ) {
        let sessions_ok = match self.runtime.block_on(self.client.fetch_sessions()) {
            Ok(sessions) => {
                self.sync_repo_themes(&sessions);
                self.refresh_mermaid_artifacts(&sessions);
                self.reconcile_thought_log_sessions(&sessions);
                self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
                self.merge_sessions(sessions, layout.overview_field);
                if show_success_message {
                    let count = self.entities.len();
                    self.set_message(format!("refreshed {count} session{}", pluralize(count)));
                }
                true
            }
            Err(err) => {
                self.set_message(err);
                false
            }
        };

        if sessions_ok {
            match self.runtime.block_on(self.client.fetch_native_status()) {
                Ok(status) => {
                    self.native_status = Some(status);
                }
                Err(err) => {
                    self.set_message(err);
                }
            }
        }

        self.last_refresh = Some(Instant::now());
    }

    pub(crate) fn spawn_background_refresh(&mut self, show_success_message: bool) {
        let client = Arc::clone(&self.client);
        let (tx, rx) = oneshot::channel();
        self.pending_refresh = Some(rx);
        self.runtime.spawn(async move {
            let sessions_result = client.fetch_sessions().await;

            let (mermaid_artifacts, native_status) = match &sessions_result {
                Ok(sessions) => {
                    let mermaid_futs: Vec<_> = sessions
                        .iter()
                        .map(|s| {
                            let client = Arc::clone(&client);
                            let sid = s.session_id.clone();
                            async move {
                                let result = client.fetch_mermaid_artifact(&sid).await;
                                (sid, result)
                            }
                        })
                        .collect();

                    let (mermaid_results, native_result) = tokio::join!(
                        futures::future::join_all(mermaid_futs),
                        client.fetch_native_status(),
                    );

                    (mermaid_results, Some(native_result))
                }
                Err(_) => (Vec::new(), None),
            };

            let _ = tx.send(RefreshResult {
                sessions: sessions_result,
                mermaid_artifacts,
                native_status,
                show_success_message,
            });
        });
    }

    pub(crate) fn poll_pending_interaction(&mut self) {
        let Some(rx) = &mut self.pending_interaction else {
            return;
        };

        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(oneshot::error::TryRecvError::Empty) => return,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.pending_interaction = None;
                return;
            }
        };

        self.pending_interaction = None;
        self.apply_pending_interaction_result(result);
    }

    fn apply_pending_interaction_result(&mut self, result: PendingInteractionResult) {
        match result {
            PendingInteractionResult::OpenPicker { x, y, response } => match response {
                Ok(response) => {
                    let mut picker = PickerState::new(x, y, response, true, self.spawn_tool);
                    picker.sync_theme_colors(&mut self.repo_themes);
                    self.picker = Some(picker);
                }
                Err(err) => {
                    self.set_message(err);
                    self.picker = None;
                }
            },
            PendingInteractionResult::ReloadPicker {
                managed_only,
                response,
            } => match response {
                Ok(response) => {
                    if let Some(picker) = &mut self.picker {
                        picker.managed_only = managed_only;
                        picker.apply_response(response);
                        picker.sync_theme_colors(&mut self.repo_themes);
                    }
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::CreateSession { field, response } => match response {
                Ok(response) => {
                    let repo_theme = response.repo_theme.clone();
                    let session = response.session;
                    let session_id = session.session_id.clone();
                    let tmux_name = session.tmux_name.clone();
                    self.remember_repo_theme(&session, repo_theme);
                    self.upsert_session(session, field);
                    self.selected_id = Some(session_id);
                    self.reconcile_selection();
                    self.sync_selection_publication();
                    self.close_picker();
                    self.set_message(format!("created {tmux_name}"));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::OpenSession { label, response } => match response {
                Ok(response) => {
                    self.set_message(format!("{} {}", response.status, label));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::ToggleNativeApp { next_app, response } => match response {
                Ok(status) => {
                    self.native_status = Some(status.clone());
                    self.set_message(Self::native_status_message(&status, next_app));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::ToggleGhosttyMode {
                next_mode,
                response,
            } => match response {
                Ok(status) => {
                    self.native_status = Some(status);
                    self.set_message(format!("Ghostty preview mode: {}", next_mode.label()));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::OpenThoughtConfig { response } => match response {
                Ok(response) => {
                    self.thought_config_editor = Some(ThoughtConfigEditorState::new(
                        response.config,
                        response.daemon_defaults,
                    ));
                }
                Err(err) => self.set_message(err),
            },
            PendingInteractionResult::TestThoughtConfig { outcome }
            | PendingInteractionResult::SaveThoughtConfig { outcome } => {
                if let Some(candidates) = outcome.openrouter_candidates {
                    if let Some(editor) = &mut self.thought_config_editor {
                        editor.replace_openrouter_model_presets(candidates);
                    }
                }
                if let Some(config) = outcome.updated_config {
                    if let Some(editor) = &mut self.thought_config_editor {
                        editor.config = config;
                    }
                }
                if outcome.close_editor {
                    self.close_thought_config_editor();
                }
                if outcome.refresh_sessions {
                    self.pending_refresh = None;
                    self.spawn_background_refresh(false);
                }
                self.set_message(outcome.message);
            }
        }
    }

    fn native_status_message(
        status: &NativeDesktopStatusResponse,
        fallback_app: NativeDesktopApp,
    ) -> String {
        let app_label = status
            .app
            .clone()
            .unwrap_or_else(|| fallback_app.display_name().to_string());
        if status.supported {
            match status.ghostty_mode {
                Some(mode) => format!("native open target: {app_label} ({})", mode.label()),
                None => format!("native open target: {app_label}"),
            }
        } else {
            format!(
                "native open target: {app_label} | {}",
                status.reason.as_deref().unwrap_or("unavailable")
            )
        }
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
            Ok(sessions) => {
                self.sync_repo_themes(&sessions);

                let mut next_artifacts = HashMap::new();
                for (session_id, artifact_result) in result.mermaid_artifacts {
                    match artifact_result {
                        Ok(artifact) if artifact.available => {
                            next_artifacts.insert(session_id, artifact);
                        }
                        Ok(_) => {}
                        Err(err) => self.set_message(err),
                    }
                }
                self.mermaid_artifacts = next_artifacts;

                if let FishBowlMode::Mermaid(viewer) = &mut self.fish_bowl_mode {
                    if let Some(artifact) = self.mermaid_artifacts.get(&viewer.session_id) {
                        let path_changed = viewer.path != artifact.path;
                        let source_changed = viewer.source != artifact.source;
                        let error_changed = viewer.artifact_error != artifact.error;
                        viewer.path = artifact.path.clone();
                        viewer.source = artifact.source.clone();
                        viewer.artifact_error = artifact.error.clone();
                        viewer.render_error = None;
                        if source_changed || error_changed {
                            viewer.invalidate_source_cache();
                        } else if path_changed {
                            viewer.invalidate_viewport_cache();
                        }
                    }
                }

                self.reconcile_thought_log_sessions(&sessions);
                self.capture_thought_updates(&sessions, layout.thought_entry_capacity());
                self.merge_sessions(sessions, layout.overview_field);

                if result.show_success_message {
                    let count = self.entities.len();
                    self.set_message(format!("refreshed {count} session{}", pluralize(count)));
                }
            }
            Err(err) => {
                self.set_message(err);
            }
        }

        if let Some(native_result) = result.native_status {
            match native_result {
                Ok(status) => {
                    self.native_status = Some(status);
                }
                Err(err) => {
                    self.set_message(err);
                }
            }
        }

        self.last_refresh = Some(Instant::now());
    }

    pub(crate) fn merge_sessions(&mut self, sessions: Vec<SessionSummary>, field: Rect) {
        let mut existing = HashMap::new();
        for entity in self.entities.drain(..) {
            existing.insert(entity.session.session_id.clone(), entity);
        }

        let mut next = Vec::with_capacity(sessions.len());
        for session in sessions {
            if let Some(mut entity) = existing.remove(&session.session_id) {
                entity.session = session;
                next.push(entity);
            } else {
                next.push(SessionEntity::new(session, field));
            }
        }

        next.sort_by(|a, b| a.session.tmux_name.cmp(&b.session.tmux_name));
        self.entities = next;
        self.layout_resting_entities(field);
        self.reconcile_selection();
        self.sync_selection_publication();
    }

    pub(crate) fn upsert_session(&mut self, session: SessionSummary, field: Rect) {
        let mut sessions: Vec<SessionSummary> = self
            .entities
            .iter()
            .map(|entity| entity.session.clone())
            .collect();
        if let Some(existing) = sessions
            .iter_mut()
            .find(|existing| existing.session_id == session.session_id)
        {
            *existing = session;
        } else {
            sessions.push(session);
        }
        self.merge_sessions(sessions, field);
    }

    pub(crate) fn sync_repo_themes(&mut self, sessions: &[SessionSummary]) {
        let mut next = HashMap::new();
        for session in sessions {
            if let Some((theme_id, theme)) = discover_repo_theme(&session.cwd) {
                next.insert(theme_id, theme);
                continue;
            }

            let Some(theme_id) = session.repo_theme_id.as_ref() else {
                continue;
            };
            let Some(theme) = self.repo_themes.get(theme_id).cloned() else {
                continue;
            };
            next.insert(theme_id.clone(), theme);
        }
        self.repo_themes = next;
    }

    pub(crate) fn remember_repo_theme(
        &mut self,
        session: &SessionSummary,
        theme: Option<RepoTheme>,
    ) {
        if let (Some(theme_id), Some(theme)) = (session.repo_theme_id.as_ref(), theme) {
            self.repo_themes.insert(theme_id.clone(), theme);
            return;
        }

        if let Some((theme_id, resolved)) = discover_repo_theme(&session.cwd) {
            self.repo_themes.insert(theme_id, resolved);
        }
    }

    pub(crate) fn tick(&mut self, field: Rect) {
        self.tick = self.tick.wrapping_add(1);
        self.layout_resting_entities(field);
        for entity in &mut self.entities {
            entity.tick(field, self.tick);
        }
        self.resolve_collisions(field);
    }

    pub(crate) fn layout_resting_entities(&mut self, field: Rect) {
        let mut bottom_resting = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.rest_anchor() == RestAnchor::Bottom).then_some(index)
            })
            .collect::<Vec<_>>();
        let mut top_resting = self
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.rest_anchor() == RestAnchor::Top).then_some(index)
            })
            .collect::<Vec<_>>();

        bottom_resting.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });
        top_resting.sort_by(|left, right| {
            compare_sleepiness(
                &self.entities[*left].session,
                &self.entities[*right].session,
            )
        });

        for (slot, entity_index) in bottom_resting.into_iter().enumerate() {
            let (x, y) = bottom_rest_origin(field, slot);
            self.entities[entity_index].set_relative_position(x, y);
        }
        for (slot, entity_index) in top_resting.into_iter().enumerate() {
            let (x, y) = top_rest_origin(field, slot);
            self.entities[entity_index].set_relative_position(x, y);
        }
    }

    pub(crate) fn resolve_collisions(&mut self, field: Rect) {
        for idx in 0..self.entities.len() {
            let (left, right) = self.entities.split_at_mut(idx + 1);
            let a = &mut left[idx];
            for b in right {
                let a_rect = a.screen_rect(field);
                let b_rect = b.screen_rect(field);
                if intersects(a_rect, b_rect) {
                    match (a.is_stationary(), b.is_stationary()) {
                        (true, true) => {}
                        (true, false) => separate_from_fixed_entity(b, a_rect, field),
                        (false, true) => separate_from_fixed_entity(a, b_rect, field),
                        (false, false) => {
                            std::mem::swap(&mut a.vx, &mut b.vx);
                            std::mem::swap(&mut a.vy, &mut b.vy);
                            a.x = (a.x - 1.0).max(0.0);
                            b.x = (b.x + 1.0).min(field.width.saturating_sub(ENTITY_WIDTH) as f32);
                            a.swim_anchor_x = a.x;
                            b.swim_anchor_x = b.x;
                            a.swim_anchor_y = a.y;
                            b.swim_anchor_y = b.y;
                            a.swim_center_y = a.y;
                            b.swim_center_y = b.y;
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn move_selection(&mut self, delta: isize, field: Rect) {
        if let Some(picker) = &mut self.picker {
            let layout = picker_layout(picker, field);
            picker.move_selection(delta, layout.visible_entry_rows);
            return;
        }

        if self.entities.is_empty() {
            self.selected_id = None;
            self.sync_selection_publication();
            return;
        }

        let visible_entities = self.visible_entities();
        if visible_entities.is_empty() {
            self.selected_id = None;
            self.sync_selection_publication();
            return;
        }

        let current_index = self
            .selected_id
            .as_ref()
            .and_then(|selected| {
                visible_entities
                    .iter()
                    .position(|entity| entity.session.session_id == *selected)
            })
            .unwrap_or(0) as isize;

        let len = visible_entities.len() as isize;
        let next_index = (current_index + delta).rem_euclid(len) as usize;
        self.selected_id = Some(visible_entities[next_index].session.session_id.clone());
        self.sync_selection_publication();
    }

    pub(crate) fn selected(&self) -> Option<&SessionEntity> {
        let selected = self.selected_id.as_ref()?;
        self.entities.iter().find(|entity| {
            entity.session.session_id == *selected
                && self.thought_filter.matches_session(&entity.session)
        })
    }

    pub(crate) fn close_picker(&mut self) {
        self.picker = None;
        self.initial_request = None;
    }

    pub(crate) fn open_thought_config_editor(&mut self) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("loading thought config...");
        self.runtime.spawn(async move {
            let response = client.fetch_thought_config().await;
            let _ = tx.send(PendingInteractionResult::OpenThoughtConfig { response });
        });
    }

    pub(crate) fn close_thought_config_editor(&mut self) {
        self.thought_config_editor = None;
    }

    pub(crate) fn handle_thought_config_key(&mut self, key: KeyEvent, layout: WorkspaceLayout) {
        if self.pending_interaction.is_some() {
            if key.code == KeyCode::Esc {
                self.close_thought_config_editor();
            } else {
                self.set_message("wait for the current action to finish");
            }
            return;
        }

        match key.code {
            KeyCode::Esc => self.close_thought_config_editor(),
            KeyCode::Up => {
                if let Some(editor) = &mut self.thought_config_editor {
                    editor.move_focus(-1);
                }
            }
            KeyCode::Down | KeyCode::Tab => {
                if let Some(editor) = &mut self.thought_config_editor {
                    editor.move_focus(1);
                }
            }
            KeyCode::BackTab => {
                if let Some(editor) = &mut self.thought_config_editor {
                    editor.move_focus(-1);
                }
            }
            KeyCode::Left => self.adjust_thought_config_field(-1),
            KeyCode::Right => self.adjust_thought_config_field(1),
            KeyCode::Backspace => {
                if let Some(editor) = &mut self.thought_config_editor {
                    if editor.focus == ThoughtConfigEditorField::Model {
                        editor.config.model.pop();
                    }
                }
            }
            KeyCode::Enter => self.activate_thought_config_field(layout),
            KeyCode::Char(' ') => self.adjust_thought_config_field(1),
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(editor) = &mut self.thought_config_editor {
                    if editor.focus == ThoughtConfigEditorField::Model {
                        editor.config.model.push(ch);
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_thought_config_paste(&mut self, text: &str) {
        if self.pending_interaction.is_some() {
            self.set_message("wait for the current action to finish");
            return;
        }
        if let Some(editor) = &mut self.thought_config_editor {
            if editor.focus == ThoughtConfigEditorField::Model {
                editor.config.model.push_str(text);
            }
        }
    }

    fn adjust_thought_config_field(&mut self, delta: isize) {
        let Some(editor) = &mut self.thought_config_editor else {
            return;
        };
        match editor.focus {
            ThoughtConfigEditorField::Enabled => editor.config.enabled = !editor.config.enabled,
            ThoughtConfigEditorField::Backend => editor.cycle_backend(delta),
            ThoughtConfigEditorField::Model => {
                let _ = editor.cycle_model_preset(delta);
            }
            ThoughtConfigEditorField::Test
            | ThoughtConfigEditorField::Save
            | ThoughtConfigEditorField::Cancel => {}
        }
    }

    fn activate_thought_config_field(&mut self, layout: WorkspaceLayout) {
        let Some(focus) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.focus)
        else {
            return;
        };
        match focus {
            ThoughtConfigEditorField::Enabled => self.adjust_thought_config_field(1),
            ThoughtConfigEditorField::Backend => self.adjust_thought_config_field(1),
            ThoughtConfigEditorField::Model => {}
            ThoughtConfigEditorField::Test => self.test_thought_config(),
            ThoughtConfigEditorField::Save => self.submit_thought_config(layout),
            ThoughtConfigEditorField::Cancel => self.close_thought_config_editor(),
        }
    }

    fn test_thought_config(&mut self) {
        let Some(mut config) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.config.clone())
        else {
            return;
        };
        let daemon_defaults = self
            .thought_config_editor
            .as_ref()
            .and_then(|editor| editor.daemon_defaults.clone());
        config.model = config.model.trim().to_string();
        if let Some(editor) = &mut self.thought_config_editor {
            editor.config.model = config.model.clone();
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("testing thought config...");
        self.runtime.spawn(async move {
            let outcome =
                Self::run_thought_config_test_action(client, config, daemon_defaults).await;
            let _ = tx.send(PendingInteractionResult::TestThoughtConfig { outcome });
        });
    }

    pub(crate) fn submit_thought_config(&mut self, _layout: WorkspaceLayout) {
        let Some(mut config) = self
            .thought_config_editor
            .as_ref()
            .map(|editor| editor.config.clone())
        else {
            return;
        };
        let daemon_defaults = self
            .thought_config_editor
            .as_ref()
            .and_then(|editor| editor.daemon_defaults.clone());
        config.model = config.model.trim().to_string();
        if let Some(editor) = &mut self.thought_config_editor {
            editor.config.model = config.model.clone();
        }
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("saving thought config...");
        self.runtime.spawn(async move {
            let outcome =
                Self::run_thought_config_save_action(client, config, daemon_defaults).await;
            let _ = tx.send(PendingInteractionResult::SaveThoughtConfig { outcome });
        });
    }

    async fn run_thought_config_test_action(
        client: Arc<C>,
        config: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
    ) -> ThoughtConfigActionOutcome {
        let target = Self::thought_config_target_summary(&config);
        match client.test_thought_config(config.clone()).await {
            Ok(test) if test.ok => ThoughtConfigActionOutcome {
                message: format!("test ok: {target}"),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
            Ok(test) => Self::try_openrouter_rotation(
                client,
                &config,
                daemon_defaults,
                false,
                target.clone(),
                test.message.clone(),
            )
            .await
            .unwrap_or_else(|| ThoughtConfigActionOutcome {
                message: format!("test failed: {target} | {}", test.message),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            }),
            Err(err) => ThoughtConfigActionOutcome {
                message: format!("test error: {target} | {err}"),
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
        }
    }

    async fn run_thought_config_save_action(
        client: Arc<C>,
        config: ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
    ) -> ThoughtConfigActionOutcome {
        match client.update_thought_config(config).await {
            Ok(saved) => {
                let save_summary = Self::thought_config_target_summary(&saved);
                let maybe_rotation = match client.test_thought_config(saved.clone()).await {
                    Ok(test) if test.ok => None,
                    Ok(test) => Self::try_openrouter_rotation(
                        Arc::clone(&client),
                        &saved,
                        daemon_defaults,
                        true,
                        save_summary.clone(),
                        test.message.clone(),
                    )
                    .await
                    .or_else(|| {
                        Some(ThoughtConfigActionOutcome {
                            message: format!("saved {save_summary} | {}", test.message),
                            updated_config: None,
                            openrouter_candidates: None,
                            close_editor: true,
                            refresh_sessions: true,
                        })
                    }),
                    Err(err) => Some(ThoughtConfigActionOutcome {
                        message: format!("saved {save_summary} | test error: {err}"),
                        updated_config: None,
                        openrouter_candidates: None,
                        close_editor: true,
                        refresh_sessions: true,
                    }),
                };
                maybe_rotation.unwrap_or(ThoughtConfigActionOutcome {
                    message: format!("saved {save_summary} | test ok"),
                    updated_config: None,
                    openrouter_candidates: None,
                    close_editor: true,
                    refresh_sessions: true,
                })
            }
            Err(err) => ThoughtConfigActionOutcome {
                message: err,
                updated_config: None,
                openrouter_candidates: None,
                close_editor: false,
                refresh_sessions: false,
            },
        }
    }

    async fn try_openrouter_rotation(
        client: Arc<C>,
        config: &ThoughtConfig,
        daemon_defaults: Option<DaemonDefaults>,
        persist: bool,
        target: String,
        failure_message: String,
    ) -> Option<ThoughtConfigActionOutcome> {
        if !Self::is_effective_openrouter_backend(config, daemon_defaults.as_ref())
            || !should_rotate_openrouter_model(&failure_message)
        {
            return None;
        }

        let candidates = client.refresh_openrouter_candidates().await.ok()?;
        for candidate in &candidates {
            if candidate.eq_ignore_ascii_case(config.model.trim()) {
                continue;
            }

            let mut rotated = config.clone();
            rotated.model = candidate.clone();
            let test = match client.test_thought_config(rotated.clone()).await {
                Ok(test) => test,
                Err(_) => continue,
            };
            if !test.ok {
                continue;
            }

            if persist {
                return Some(match client.update_thought_config(rotated).await {
                    Ok(_) => ThoughtConfigActionOutcome {
                        message: format!(
                            "saved {target} | rotated to {candidate} after OpenRouter catalog refresh | test ok"
                        ),
                        updated_config: None,
                        openrouter_candidates: Some(candidates),
                        close_editor: true,
                        refresh_sessions: true,
                    },
                    Err(err) => ThoughtConfigActionOutcome {
                        message: format!(
                            "saved {target} | rotated probe found {candidate}, but save failed: {err}"
                        ),
                        updated_config: None,
                        openrouter_candidates: Some(candidates),
                        close_editor: true,
                        refresh_sessions: true,
                    },
                });
            }

            return Some(ThoughtConfigActionOutcome {
                message: format!(
                    "test failed: {target} | rotated to {candidate} after OpenRouter catalog refresh | test ok"
                ),
                updated_config: Some(rotated),
                openrouter_candidates: Some(candidates),
                close_editor: false,
                refresh_sessions: false,
            });
        }

        None
    }

    fn is_effective_openrouter_backend(
        config: &ThoughtConfig,
        daemon_defaults: Option<&DaemonDefaults>,
    ) -> bool {
        if config.backend.eq_ignore_ascii_case("openrouter") {
            return true;
        }
        config.backend.trim().is_empty()
            && daemon_defaults
                .map(|defaults| defaults.backend.eq_ignore_ascii_case("openrouter"))
                .unwrap_or(false)
    }

    fn thought_config_target_summary(config: &ThoughtConfig) -> String {
        format!(
            "{} / {}",
            if config.backend.trim().is_empty() {
                "auto"
            } else {
                config.backend.as_str()
            },
            if config.model.trim().is_empty() {
                "daemon default"
            } else {
                config.model.as_str()
            }
        )
    }

    pub(crate) fn open_picker(&mut self, x: u16, y: u16) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("loading directories...");
        self.runtime.spawn(async move {
            let response = client.list_dirs(None, true).await;
            let _ = tx.send(PendingInteractionResult::OpenPicker { x, y, response });
        });
    }

    pub(crate) fn picker_reload(&mut self, path: Option<String>, managed_only: bool) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        self.set_message("loading directories...");
        self.runtime.spawn(async move {
            let response = client.list_dirs(path.as_deref(), managed_only).await;
            let _ = tx.send(PendingInteractionResult::ReloadPicker {
                managed_only,
                response,
            });
        });
    }

    pub(crate) fn picker_up(&mut self) {
        let Some(parent_path) = self.picker.as_ref().and_then(PickerState::parent_path) else {
            return;
        };
        let managed_only = self
            .picker
            .as_ref()
            .map(|picker| picker.managed_only)
            .unwrap_or(true);
        self.picker_reload(Some(parent_path), managed_only);
    }

    pub(crate) fn picker_set_managed_only(&mut self, managed_only: bool) {
        let Some(picker) = &self.picker else {
            return;
        };
        if picker.managed_only == managed_only {
            return;
        }
        self.picker_reload(Some(picker.current_path.clone()), managed_only);
    }

    pub(crate) fn open_initial_request(&mut self, cwd: String) {
        self.initial_request = Some(InitialRequestState::new(cwd));
    }

    pub(crate) fn close_initial_request(&mut self) {
        self.initial_request = None;
    }

    pub(crate) fn handle_initial_request_key(&mut self, key: KeyEvent, field: Rect) {
        match key.code {
            KeyCode::Esc => self.close_initial_request(),
            KeyCode::Enter => self.submit_initial_request(field),
            KeyCode::Backspace => {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.pop();
                }
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(initial_request) = &mut self.initial_request {
                    initial_request.value.push(ch);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_paste(&mut self, text: &str) {
        if self.thought_config_editor.is_some() {
            self.handle_thought_config_paste(text);
            return;
        }
        if let Some(initial_request) = &mut self.initial_request {
            initial_request.value.push_str(text);
        }
    }

    pub(crate) fn submit_initial_request(&mut self, field: Rect) {
        let Some(initial_request) = self
            .initial_request
            .as_ref()
            .and_then(InitialRequestState::trimmed_value)
        else {
            self.set_message("enter an initial request");
            return;
        };
        let Some(cwd) = self.initial_request.as_ref().map(|state| state.cwd.clone()) else {
            return;
        };
        self.spawn_session(&cwd, Some(initial_request), field);
    }

    pub(crate) fn picker_activate_selection(&mut self, _field: Rect) {
        let Some((selection, current_path, entry_path, has_children)) =
            self.picker.as_ref().map(|picker| match picker.selection {
                PickerSelection::SpawnHere => (
                    PickerSelection::SpawnHere,
                    picker.current_path.clone(),
                    None,
                    false,
                ),
                PickerSelection::Entry(index) => (
                    PickerSelection::Entry(index),
                    picker.current_path.clone(),
                    picker.path_for_entry(index),
                    picker
                        .entries
                        .get(index)
                        .map(|entry| entry.has_children)
                        .unwrap_or(false),
                ),
            })
        else {
            return;
        };

        match selection {
            PickerSelection::SpawnHere => self.open_initial_request(current_path),
            PickerSelection::Entry(_) if has_children => {
                if let Some(path) = entry_path {
                    let managed_only = self
                        .picker
                        .as_ref()
                        .map(|picker| picker.managed_only)
                        .unwrap_or(true);
                    self.picker_reload(Some(path), managed_only);
                }
            }
            PickerSelection::Entry(_) => {
                if let Some(path) = entry_path {
                    self.open_initial_request(path);
                }
            }
        }
    }

    pub(crate) fn spawn_session(
        &mut self,
        cwd: &str,
        initial_request: Option<String>,
        field: Rect,
    ) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let cwd = cwd.to_string();
        let spawn_tool = self.spawn_tool;
        self.set_message("creating session...");
        self.runtime.spawn(async move {
            let response = client
                .create_session(&cwd, spawn_tool, initial_request)
                .await;
            let _ = tx.send(PendingInteractionResult::CreateSession { field, response });
        });
    }

    pub(crate) fn open_session_for_label(&mut self, session_id: &str, label: &str) {
        let Some(tx) = self.begin_pending_interaction() else {
            return;
        };

        let client = Arc::clone(&self.client);
        let session_id = session_id.to_string();
        let label = label.to_string();
        self.set_message(format!("opening {label}..."));
        self.runtime.spawn(async move {
            let response = client.open_session(&session_id).await;
            let _ = tx.send(PendingInteractionResult::OpenSession { label, response });
        });
    }

    pub(crate) fn open_selected(&mut self) {
        let Some((selected_id, label)) = self.selected().map(|entity| {
            (
                entity.session.session_id.clone(),
                selected_label(Some(&entity.session.tmux_name)),
            )
        }) else {
            self.set_message("no session selected");
            return;
        };

        self.select_and_open_session(selected_id, label);
    }

    pub(crate) fn select_and_open_session(&mut self, session_id: String, label: String) {
        self.selected_id = Some(session_id.clone());
        self.sync_selection_publication();
        self.open_session_for_label(&session_id, &label);
    }

    pub(crate) fn handle_thought_click(
        &mut self,
        x: u16,
        y: u16,
        thought_content: Rect,
        entry_capacity: usize,
    ) {
        if let Some(action) = thought_panel_action_at(self, thought_content, entry_capacity, x, y) {
            self.apply_thought_filter_action(action);
        }
    }

    pub(crate) fn handle_header_filter_click(&mut self, renderer_width: u16, x: u16, y: u16) {
        if let Some(action) = header_filter_action_at(self, renderer_width, x, y) {
            self.apply_thought_filter_action(action);
        }
    }

    pub(crate) fn apply_thought_filter_action(&mut self, action: ThoughtPanelAction) {
        match action {
            ThoughtPanelAction::FilterByCwd(cwd) => self.set_thought_filter_cwd(cwd),
            ThoughtPanelAction::ToggleFilterOutMode => self.toggle_thought_filter_out_mode(),
            ThoughtPanelAction::ToggleFilterOutCwd(cwd) => self.toggle_thought_filter_out_cwd(cwd),
            ThoughtPanelAction::OpenSession { session_id, label } => {
                self.select_and_open_session(session_id, label);
            }
            ThoughtPanelAction::LaunchCommitCodex(session_id) => {
                self.launch_commit_codex_for_session(&session_id);
            }
            ThoughtPanelAction::OpenMermaid(session_id) => self.open_mermaid_viewer(session_id),
            ThoughtPanelAction::OpenRepoInEditor(cwd) => self.open_repo_in_editor(&cwd),
            ThoughtPanelAction::ClearFilters => self.clear_thought_filters(),
        }
    }

    pub(crate) fn launch_commit_codex_for_session(&mut self, session_id: &str) {
        let Some(session) = self
            .entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .map(|entity| entity.session.clone())
        else {
            self.set_message("missing session for commit codex launch");
            return;
        };

        match self.commit_launcher.launch(&session) {
            Ok(launch) => self.set_message(format!("commit codex: {}", launch.watch_command)),
            Err(err) => self.set_message(format!("failed to launch commit codex: {err}")),
        }
    }

    pub(crate) fn open_repo_in_editor(&mut self, cwd: &str) {
        let repo_label = path_tail_label(cwd).unwrap_or_else(|| cwd.to_string());
        match ProcessCommand::new("code")
            .arg(".")
            .current_dir(cwd)
            .spawn()
        {
            Ok(_) => self.set_message(format!("code . -> {repo_label}")),
            Err(err) => self.set_message(format!("failed to run code .: {err}")),
        }
    }

    pub(crate) fn open_mermaid_artifact(&mut self) {
        let Some(path) = (match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                if viewer.active_tab != DomainPlanTab::Schema {
                    // Derive the plan file path from schema.mmd's directory
                    viewer.path.as_ref().and_then(|p| {
                        let dir = std::path::Path::new(p).parent()?;
                        let file_path = dir.join(viewer.active_tab.filename());
                        Some(file_path.to_string_lossy().into_owned())
                    })
                } else {
                    viewer.openable_path().map(str::to_string)
                }
            }
            FishBowlMode::Aquarium => None,
        }) else {
            self.set_message("Mermaid artifact path unavailable");
            return;
        };

        let path_label = path_tail_label(&path).unwrap_or_else(|| path.clone());
        match self.artifact_opener.open(&path) {
            Ok(_) => self.set_message(format!("open Mermaid artifact -> {path_label}")),
            Err(err) => self.set_message(format!("failed to open Mermaid artifact: {err}")),
        }
    }

    pub(crate) fn open_mermaid_viewer(&mut self, session_id: String) {
        let Some(session) = self
            .entities
            .iter()
            .find(|entity| entity.session.session_id == session_id)
            .map(|entity| entity.session.clone())
        else {
            self.set_message("missing session for Mermaid viewer");
            return;
        };

        let Some(artifact) = self.mermaid_artifacts.get(&session.session_id).cloned() else {
            self.set_message("no Mermaid artifact found");
            return;
        };

        let unsupported_reason = detect_mermaid_backend_support();
        let plan_tabs = artifact.plan_files.and_then(|files| {
            let mut tabs = vec![DomainPlanTab::Schema];
            for name in &files {
                if let Some(tab) = DomainPlanTab::from_filename(name) {
                    if tab != DomainPlanTab::Schema {
                        tabs.push(tab);
                    }
                }
            }
            if tabs.len() > 1 {
                Some(tabs)
            } else {
                None
            }
        });
        self.fish_bowl_mode = FishBowlMode::Mermaid(MermaidViewerState {
            session_id: session.session_id.clone(),
            tmux_name: session.tmux_name.clone(),
            path: artifact.path,
            source: artifact.source,
            artifact_error: artifact.error,
            render_error: None,
            unsupported_reason,
            zoom: 1.0,
            center_x: 0.0,
            center_y: 0.0,
            diagram_width: 0.0,
            diagram_height: 0.0,
            back_rect: None,
            content_rect: None,
            cached_rect: None,
            cached_zoom: 1.0,
            cached_center_x: 0.0,
            cached_center_y: 0.0,
            cached_lines: Vec::new(),
            cached_background_cells: Vec::new(),
            cached_semantic_lines: Vec::new(),
            focused_source_index: None,
            focus_status: None,
            prepared_render: None,
            source_prepare_count: 0,
            viewport_render_count: 0,
            plan_tabs,
            active_tab: DomainPlanTab::Schema,
            plan_text_content: None,
            plan_text_lines: Vec::new(),
            plan_text_scroll: 0,
            plan_text_cached_width: 0,
            tab_rects: Vec::new(),
        });
    }

    pub(crate) fn close_mermaid_viewer(&mut self) {
        self.fish_bowl_mode = FishBowlMode::Aquarium;
        self.mermaid_drag = None;
    }

    pub(crate) fn mermaid_viewer_mut(&mut self) -> Option<&mut MermaidViewerState> {
        match &mut self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => Some(viewer),
            FishBowlMode::Aquarium => None,
        }
    }

    pub(crate) fn switch_plan_tab(&mut self, tab: DomainPlanTab) {
        let is_valid = match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                viewer
                    .plan_tabs
                    .as_ref()
                    .map_or(false, |tabs| tabs.contains(&tab))
                    && viewer.active_tab != tab
            }
            FishBowlMode::Aquarium => false,
        };
        if !is_valid {
            return;
        }

        if tab != DomainPlanTab::Schema {
            let (session_id, _) = match &self.fish_bowl_mode {
                FishBowlMode::Mermaid(v) => (v.session_id.clone(), ()),
                _ => return,
            };
            let result = self
                .runtime
                .block_on(self.client.fetch_plan_file(&session_id, tab.filename()));
            let viewer = self.mermaid_viewer_mut().unwrap();
            viewer.active_tab = tab;
            viewer.plan_text_scroll = 0;
            viewer.plan_text_lines.clear();
            viewer.plan_text_cached_width = 0;
            match result {
                Ok(response) => {
                    viewer.plan_text_content = response.content;
                    if let Some(err) = response.error {
                        self.set_message(format!("plan file: {err}"));
                    }
                }
                Err(err) => {
                    viewer.plan_text_content = None;
                    self.set_message(format!("plan file fetch failed: {err}"));
                }
            }
        } else {
            let viewer = self.mermaid_viewer_mut().unwrap();
            viewer.active_tab = DomainPlanTab::Schema;
            viewer.plan_text_content = None;
            viewer.plan_text_lines.clear();
            viewer.plan_text_scroll = 0;
            viewer.plan_text_cached_width = 0;
        }
    }

    pub(crate) fn cycle_plan_tab(&mut self, delta: isize) {
        let (tabs, current) = match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => match &viewer.plan_tabs {
                Some(tabs) => (tabs.clone(), viewer.active_tab),
                None => return,
            },
            FishBowlMode::Aquarium => return,
        };
        let current_idx = tabs.iter().position(|t| *t == current).unwrap_or(0);
        let next_idx = (current_idx as isize + delta).rem_euclid(tabs.len() as isize) as usize;
        let next_tab = tabs[next_idx];
        self.switch_plan_tab(next_tab);
    }

    pub(crate) fn scroll_plan_text(&mut self, delta: isize) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        let max = viewer.plan_text_lines.len().saturating_sub(1);
        viewer.plan_text_scroll =
            (viewer.plan_text_scroll as isize + delta).clamp(0, max as isize) as usize;
    }

    pub(crate) fn scroll_plan_text_page(&mut self, delta: isize) {
        let page_size = match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                viewer.content_rect.map(|r| r.height as isize).unwrap_or(20)
            }
            _ => return,
        };
        self.scroll_plan_text(delta * page_size);
    }

    pub(crate) fn pan_mermaid_viewer(&mut self, dx: f32, dy: f32) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        viewer.center_x += dx;
        viewer.center_y += dy;
        viewer.invalidate_viewport_cache();
    }

    pub(crate) fn zoom_mermaid_viewer(
        &mut self,
        delta_percent: i16,
        pointer: Option<(u16, u16)>,
        content_rect: Rect,
    ) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        let old_zoom = viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM);
        if mermaid_is_er_viewer(viewer) {
            let direction = delta_percent.signum() as i8;
            if direction == 0 {
                return;
            }
            let new_zoom = mermaid_er_zoom_step(old_zoom, direction);
            if (new_zoom - old_zoom).abs() < f32::EPSILON {
                return;
            }
            viewer.zoom = new_zoom;
            viewer.center_x = 0.0;
            viewer.center_y = 0.0;
            viewer.invalidate_viewport_cache();
            return;
        }
        let old_percent = mermaid_zoom_percent(old_zoom);
        let min_percent = mermaid_zoom_percent(MERMAID_MIN_ZOOM);
        let max_percent = mermaid_zoom_percent(MERMAID_MAX_ZOOM);
        let new_percent = (old_percent + delta_percent).clamp(min_percent, max_percent);
        let new_zoom = new_percent as f32 / 100.0;
        if (new_zoom - old_zoom).abs() < f32::EPSILON {
            return;
        }

        if let Some((column, row)) = pointer {
            let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
            let base_scale = mermaid_fit_scale(
                viewer.diagram_width,
                viewer.diagram_height,
                sample_width as f32,
                sample_height as f32,
            );
            let old_scale = base_scale * old_zoom;
            let new_scale = base_scale * new_zoom;
            if old_scale > 0.0 && new_scale > 0.0 {
                let anchor_x = (column.saturating_sub(content_rect.x) as f32) * 2.0;
                let anchor_y = (row.saturating_sub(content_rect.y) as f32) * 4.0;
                let dx = anchor_x - sample_width as f32 / 2.0;
                let dy = anchor_y - sample_height as f32 / 2.0;
                let diagram_x = viewer.center_x + dx / old_scale;
                let diagram_y = viewer.center_y + dy / old_scale;
                viewer.center_x = diagram_x - dx / new_scale;
                viewer.center_y = diagram_y - dy / new_scale;
            }
        }

        viewer.zoom = new_zoom;
        viewer.invalidate_viewport_cache();
    }

    pub(crate) fn reset_mermaid_viewer_fit(&mut self) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };
        viewer.zoom = 1.0;
        viewer.center_x = 0.0;
        viewer.center_y = 0.0;
        viewer.invalidate_viewport_cache();
    }

    pub(crate) fn cycle_mermaid_focus(&mut self, content_rect: Rect, direction: i8) {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return;
        };

        let targets = match mermaid_visible_focus_targets(viewer, content_rect) {
            Ok(targets) => targets,
            Err(err) => {
                viewer.render_error = Some(err);
                viewer.focused_source_index = None;
                viewer.focus_status = Some("no semantic targets".to_string());
                return;
            }
        };

        if targets.is_empty() {
            viewer.focused_source_index = None;
            viewer.focus_status = Some("no semantic targets".to_string());
            return;
        }

        let current_index = viewer.focused_source_index.and_then(|source_index| {
            targets
                .iter()
                .position(|target| target.source_index == source_index)
        });
        let next_index = match (current_index, direction.is_negative()) {
            (Some(index), false) => (index + 1) % targets.len(),
            (Some(index), true) => index.checked_sub(1).unwrap_or(targets.len() - 1),
            (None, false) => 0,
            (None, true) => targets.len() - 1,
        };
        let target = &targets[next_index];
        Self::apply_mermaid_focus_target(viewer, target);
    }

    pub(crate) fn focus_next_mermaid_target(&mut self, content_rect: Rect) {
        self.cycle_mermaid_focus(content_rect, 1);
    }

    pub(crate) fn focus_previous_mermaid_target(&mut self, content_rect: Rect) {
        self.cycle_mermaid_focus(content_rect, -1);
    }

    fn apply_mermaid_focus_target(viewer: &mut MermaidViewerState, target: &MermaidFocusTarget) {
        viewer.focused_source_index = Some(target.source_index);
        viewer.focus_status = Some(format!("focus {}", target.text));

        let recenter_x = (viewer.center_x - target.diagram_x).abs() > f32::EPSILON;
        let recenter_y = (viewer.center_y - target.diagram_y).abs() > f32::EPSILON;
        viewer.center_x = target.diagram_x;
        viewer.center_y = target.diagram_y;
        if recenter_x || recenter_y {
            viewer.invalidate_viewport_cache();
        }
    }

    pub(crate) fn clear_mermaid_focus(&mut self) -> bool {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        if viewer.focused_source_index.is_none() {
            return false;
        }
        viewer.focused_source_index = None;
        viewer.focus_status = None;
        true
    }

    pub(crate) fn handle_mermaid_mouse_down(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let back_rect = viewer.back_rect.unwrap_or(Rect {
            x: field.x,
            y: field.y,
            width: display_width(MERMAID_BACK_LABEL),
            height: 1,
        });
        if back_rect.contains(mouse.column, mouse.row) {
            self.close_mermaid_viewer();
            return true;
        }

        // Check tab clicks
        let clicked_tab = viewer
            .tab_rects
            .iter()
            .find(|(_, rect)| rect.contains(mouse.column, mouse.row))
            .map(|(tab, _)| *tab);
        if let Some(tab) = clicked_tab {
            self.switch_plan_tab(tab);
            return true;
        }

        let viewer = self.mermaid_viewer_mut().unwrap();
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        if content_rect.contains(mouse.column, mouse.row) {
            match mermaid_visible_focus_targets(viewer, content_rect) {
                Ok(targets) => {
                    if let Some(target) = targets
                        .iter()
                        .find(|target| target.hitbox.contains(mouse.column, mouse.row))
                    {
                        Self::apply_mermaid_focus_target(viewer, target);
                        self.mermaid_drag = None;
                        return true;
                    }
                }
                Err(err) => {
                    viewer.render_error = Some(err);
                    return true;
                }
            }
            self.mermaid_drag = Some(MermaidDragState {
                start_column: mouse.column,
                start_row: mouse.row,
                start_center_x: viewer.center_x,
                start_center_y: viewer.center_y,
            });
            return true;
        }

        false
    }

    pub(crate) fn handle_mermaid_mouse_drag(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
    ) -> bool {
        let Some(drag) = self.mermaid_drag else {
            return false;
        };
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
        let scale = mermaid_fit_scale(
            viewer.diagram_width,
            viewer.diagram_height,
            sample_width as f32,
            sample_height as f32,
        ) * viewer.zoom.max(MERMAID_MIN_ZOOM);
        if scale <= 0.0 {
            return false;
        }
        let dx = (mouse.column as i32 - drag.start_column as i32) as f32 * 2.0;
        let dy = (mouse.row as i32 - drag.start_row as i32) as f32 * 4.0;
        viewer.center_x = drag.start_center_x - dx / scale;
        viewer.center_y = drag.start_center_y - dy / scale;
        viewer.invalidate_viewport_cache();
        true
    }

    pub(crate) fn handle_mermaid_mouse_up(&mut self) -> bool {
        let active = self.mermaid_drag.is_some();
        self.mermaid_drag = None;
        active
    }

    pub(crate) fn handle_mermaid_scroll(
        &mut self,
        field: Rect,
        mouse: crossterm::event::MouseEvent,
        direction: MermaidZoomDirection,
    ) -> bool {
        let (content_rect, is_text_tab) = {
            let Some(viewer) = self.mermaid_viewer_mut() else {
                return false;
            };
            let rect = viewer
                .content_rect
                .unwrap_or_else(|| mermaid_content_rect(field));
            (rect, viewer.active_tab != DomainPlanTab::Schema)
        };
        if !content_rect.contains(mouse.column, mouse.row) {
            return false;
        }

        // On text tabs, scroll text instead of zooming
        if is_text_tab {
            let delta: isize = match direction {
                MermaidZoomDirection::In => -3,
                MermaidZoomDirection::Out => 3,
            };
            self.scroll_plan_text(delta);
            return true;
        }

        let delta_percent = match direction {
            MermaidZoomDirection::In => MERMAID_SCROLL_ZOOM_STEP_PERCENT,
            MermaidZoomDirection::Out => -MERMAID_SCROLL_ZOOM_STEP_PERCENT,
        };
        self.zoom_mermaid_viewer(delta_percent, Some((mouse.column, mouse.row)), content_rect);
        true
    }

    pub(crate) fn start_split_drag(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        let resized = self.resize_thought_panel(layout, x);
        self.split_drag_active = resized;
        resized
    }

    pub(crate) fn drag_split(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        if !self.split_drag_active {
            return false;
        }
        self.resize_thought_panel(layout, x)
    }

    pub(crate) fn stop_split_drag(&mut self) {
        self.split_drag_active = false;
    }

    pub(crate) fn resize_thought_panel(&mut self, layout: WorkspaceLayout, x: u16) -> bool {
        let Some(ratio) = layout.thought_ratio_for_divider_x(x) else {
            return false;
        };
        self.thought_panel_ratio = ratio;
        true
    }

    pub(crate) fn handle_field_click(&mut self, x: u16, y: u16, field: Rect) {
        if self.initial_request.is_some() {
            return;
        }

        if let Some(picker) = &self.picker {
            let layout = picker_layout(picker, field);
            if layout.frame.contains(x, y) {
                if let Some(action) = picker_action_at(picker, layout, x, y) {
                    self.handle_picker_action(action, field);
                }
                return;
            }
            self.close_picker();
            return;
        }

        let hit = self
            .visible_entities()
            .into_iter()
            .find(|entity| entity.screen_rect(field).contains(x, y))
            .map(|entity| {
                (
                    entity.session.session_id.clone(),
                    selected_label(Some(&entity.session.tmux_name)),
                )
            });

        if let Some((session_id, label)) = hit {
            self.select_and_open_session(session_id, label);
            return;
        }

        self.open_picker(x, y);
    }

    pub(crate) fn handle_picker_action(&mut self, action: PickerAction, field: Rect) {
        match action {
            PickerAction::Close => self.close_picker(),
            PickerAction::Up => self.picker_up(),
            PickerAction::ToggleManaged(managed_only) => {
                self.picker_set_managed_only(managed_only);
            }
            PickerAction::ToggleTool => {
                self.spawn_tool = self.spawn_tool.toggle();
                if let Some(picker) = &mut self.picker {
                    picker.spawn_tool = self.spawn_tool;
                }
            }
            PickerAction::ActivateCurrentPath => self.spawn_session_from_picker(field),
            PickerAction::ActivateEntry(index) => self.activate_picker_entry(index, field),
        }
    }

    pub(crate) fn spawn_session_from_picker(&mut self, _field: Rect) {
        let Some(path) = self
            .picker
            .as_ref()
            .map(|picker| picker.current_path.clone())
        else {
            return;
        };
        self.open_initial_request(path);
    }

    pub(crate) fn activate_picker_entry(&mut self, index: usize, _field: Rect) {
        let Some((path, has_children, managed_only)) = self.picker.as_ref().and_then(|picker| {
            Some((
                picker.path_for_entry(index)?,
                picker.entries.get(index)?.has_children,
                picker.managed_only,
            ))
        }) else {
            return;
        };

        if has_children {
            self.picker_reload(Some(path), managed_only);
        } else {
            self.open_initial_request(path);
        }
    }

    pub(crate) fn render(&mut self, renderer: &mut Renderer, layout: WorkspaceLayout) {
        renderer.clear();

        if renderer.width() < MIN_WIDTH || renderer.height() < MIN_HEIGHT {
            render_too_small(renderer);
            return;
        }

        let frame = frame_rect(renderer.width(), renderer.height());

        renderer.draw_box(frame, Color::DarkGrey);
        renderer.draw_text(2, 1, "swimmers tui", Color::Cyan);

        let max_right_width = renderer.width().saturating_sub(22) as usize;
        let right_text = truncate_label(&self.header_right_text(), max_right_width);
        let right_x = renderer
            .width()
            .saturating_sub(display_width(&right_text))
            .saturating_sub(2);
        renderer.draw_text(right_x, 1, &right_text, Color::DarkGrey);
        render_header_filter_strip(self, renderer, renderer.width());

        renderer.draw_box(layout.workspace_box, Color::DarkGrey);

        if let (Some(thought_box), Some(thought_content)) =
            (layout.thought_box, layout.thought_content)
        {
            renderer.draw_box(thought_box, Color::DarkGrey);
            renderer.draw_box(layout.overview_box, Color::DarkGrey);
            if let Some(split_divider) = layout.split_divider {
                let divider_color = if self.split_drag_active {
                    Color::Cyan
                } else {
                    Color::DarkGrey
                };
                renderer.draw_vline(
                    split_divider.x,
                    split_divider.y,
                    split_divider.height,
                    ':',
                    divider_color,
                );
            }
            render_thought_panel(
                self,
                renderer,
                thought_content,
                layout.thought_entry_capacity(),
            );
        }

        match &mut self.fish_bowl_mode {
            FishBowlMode::Aquarium => {
                render_aquarium_background(renderer, layout.overview_field, self.tick);

                let visible_entities = self.visible_entities();
                if visible_entities.is_empty() {
                    let empty = if self.entities.is_empty() {
                        "no tmux sessions found - press r after starting one"
                    } else if self.thought_filter.is_active() {
                        "no swimmers match filters"
                    } else {
                        "no tmux sessions found - press r after starting one"
                    };
                    let x = layout.overview_field.x.saturating_add(
                        layout
                            .overview_field
                            .width
                            .saturating_sub(empty.len() as u16)
                            / 2,
                    );
                    let y = layout.overview_field.y + layout.overview_field.height / 2;
                    renderer.draw_text(x, y, empty, Color::DarkGrey);
                }

                for entity in visible_entities {
                    let rect = entity.screen_rect(layout.overview_field);
                    let selected = self
                        .selected_id
                        .as_ref()
                        .map(|selected| *selected == entity.session.session_id)
                        .unwrap_or(false);
                    render_entity(
                        renderer,
                        entity,
                        rect,
                        selected,
                        self.tick,
                        &self.repo_themes,
                    );
                }
            }
            FishBowlMode::Mermaid(viewer) => {
                render_mermaid_viewer(renderer, layout.overview_field, viewer);
            }
        }

        if let Some(picker) = &self.picker {
            render_picker(renderer, picker, layout.overview_field);
        }
        if let Some(initial_request) = &self.initial_request {
            render_initial_request(renderer, initial_request, layout.overview_field);
        }
        if let Some(editor) = &self.thought_config_editor {
            render_thought_config_editor(renderer, editor, layout.overview_field);
        }

        render_footer(self, renderer, layout.footer_start_y);
    }
}
