use super::*;

pub(crate) struct App<C: TuiApi> {
    pub(crate) runtime: Runtime,
    pub(crate) client: C,
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
            client,
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
            Some(status) if status.supported => format!(
                "native open: {}",
                status.app.as_deref().unwrap_or("available")
            ),
            Some(status) => format!(
                "native open unavailable: {}",
                status.reason.as_deref().unwrap_or("unknown reason")
            ),
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

    pub(crate) fn visible_entities(&self) -> Vec<&SessionEntity> {
        self.entities
            .iter()
            .filter(|entity| self.thought_filter.matches_session(&entity.session))
            .collect()
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
        self.publish_selection(self.selected_id.clone(), false);
    }

    pub(crate) fn clear_published_selection(&mut self) {
        self.publish_selection(None, true);
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

    pub(crate) fn manual_refresh(&mut self, layout: WorkspaceLayout) {
        self.refresh_with_feedback(layout, true);
    }

    pub(crate) fn refresh_with_feedback(
        &mut self,
        layout: WorkspaceLayout,
        show_success_message: bool,
    ) {
        match self.runtime.block_on(self.client.fetch_sessions()) {
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
            }
            Err(err) => {
                self.set_message(err);
            }
        }

        if self.native_status.is_none() {
            self.native_status = self
                .runtime
                .block_on(self.client.fetch_native_status())
                .map(Some)
                .unwrap_or_else(|err| {
                    self.set_message(err);
                    None
                });
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

    pub(crate) fn open_picker(&mut self, x: u16, y: u16) {
        match self.runtime.block_on(self.client.list_dirs(None, true)) {
            Ok(response) => {
                let mut picker = PickerState::new(x, y, response, true, self.spawn_tool);
                picker.sync_theme_colors(&mut self.repo_themes);
                self.picker = Some(picker);
            }
            Err(err) => {
                self.set_message(err);
                self.picker = None;
            }
        }
    }

    pub(crate) fn picker_reload(&mut self, path: Option<String>, managed_only: bool) {
        let target = path.clone();
        match self
            .runtime
            .block_on(self.client.list_dirs(target.as_deref(), managed_only))
        {
            Ok(response) => {
                if let Some(picker) = &mut self.picker {
                    picker.managed_only = managed_only;
                    picker.apply_response(response);
                    picker.sync_theme_colors(&mut self.repo_themes);
                }
            }
            Err(err) => self.set_message(err),
        }
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
        match self.runtime.block_on(self.client.create_session(
            cwd,
            self.spawn_tool,
            initial_request,
        )) {
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
        }
    }

    pub(crate) fn open_session_for_label(&mut self, session_id: &str, label: &str) {
        match self.runtime.block_on(self.client.open_session(session_id)) {
            Ok(response) => {
                self.set_message(format!("{} {}", response.status, label));
            }
            Err(err) => self.set_message(err),
        }
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
            FishBowlMode::Mermaid(viewer) => viewer.openable_path().map(str::to_string),
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

    pub(crate) fn focus_next_mermaid_target(&mut self, content_rect: Rect) {
        self.cycle_mermaid_focus(content_rect, 1);
    }

    pub(crate) fn focus_previous_mermaid_target(&mut self, content_rect: Rect) {
        self.cycle_mermaid_focus(content_rect, -1);
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

        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        if content_rect.contains(mouse.column, mouse.row) {
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
        let Some(viewer) = self.mermaid_viewer_mut() else {
            return false;
        };
        let content_rect = viewer
            .content_rect
            .unwrap_or_else(|| mermaid_content_rect(field));
        if !content_rect.contains(mouse.column, mouse.row) {
            return false;
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

        render_footer(self, renderer, layout.footer_start_y);
    }
}
