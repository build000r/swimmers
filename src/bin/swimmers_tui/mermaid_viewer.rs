use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MermaidCacheContext {
    tmux_name: String,
    cwd: String,
}

impl MermaidCacheContext {
    fn from_session(session: &SessionSummary) -> Self {
        Self {
            tmux_name: session.tmux_name.clone(),
            cwd: normalize_path(&session.cwd),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct MermaidCacheEntry {
    pub(super) context: MermaidCacheContext,
    artifact: Option<MermaidArtifactResponse>,
}

impl<C: TuiApi> App<C> {
    fn refresh_mermaid_viewer_from_cache(&mut self) {
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

    pub(super) fn rebuild_mermaid_artifacts_from_cache(&mut self) {
        self.mermaid_artifacts = self
            .session_mermaid_cache
            .iter()
            .filter_map(|(session_id, entry)| {
                entry
                    .artifact
                    .clone()
                    .map(|artifact| (session_id.clone(), artifact))
            })
            .collect();
        self.refresh_mermaid_viewer_from_cache();
    }

    pub(super) fn should_refresh_mermaid_with_contexts(
        cached_contexts: &HashMap<String, MermaidCacheContext>,
        session: &SessionSummary,
        force: bool,
    ) -> bool {
        if force {
            return true;
        }

        let context = MermaidCacheContext::from_session(session);
        cached_contexts
            .get(&session.session_id)
            .map(|cached| cached != &context)
            .unwrap_or(true)
    }

    pub(super) fn apply_mermaid_artifact_result(
        &mut self,
        session: &SessionSummary,
        result: Result<MermaidArtifactResponse, String>,
    ) {
        let context = MermaidCacheContext::from_session(session);
        let previous = self.session_mermaid_cache.get(&session.session_id).cloned();
        let preserve_cached = previous
            .as_ref()
            .map(|entry| entry.context == context)
            .unwrap_or(false);

        let artifact = match result {
            Ok(artifact) if artifact.available => Some(artifact),
            Ok(_) => None,
            Err(err) => {
                self.set_message(self.refresh_error_message(err));
                if preserve_cached {
                    previous.and_then(|entry| entry.artifact)
                } else {
                    None
                }
            }
        };

        self.session_mermaid_cache.insert(
            session.session_id.clone(),
            MermaidCacheEntry { context, artifact },
        );
    }

    fn refresh_single_mermaid_artifact(&mut self, session: &SessionSummary, force: bool) {
        let cached_contexts = self
            .session_mermaid_cache
            .iter()
            .map(|(session_id, entry)| (session_id.clone(), entry.context.clone()))
            .collect::<HashMap<_, _>>();
        if !Self::should_refresh_mermaid_with_contexts(&cached_contexts, session, force) {
            return;
        }

        let result = self
            .runtime
            .block_on(self.client.fetch_mermaid_artifact(&session.session_id));
        self.apply_mermaid_artifact_result(session, result);
        self.rebuild_mermaid_artifacts_from_cache();
    }

    pub(crate) fn refresh_mermaid_artifacts(&mut self, sessions: &[SessionSummary]) {
        self.retain_cached_assets(sessions);
        // Fan out the per-session artifact fetches concurrently. The previous
        // implementation `block_on`'d each session in sequence, so initial
        // frame paint scaled as `N * fetch_mermaid_artifact_timeout`. With ~16
        // sessions and a 5s per-call ceiling that pushed first paint past 30s,
        // long enough that the TUI looked hung on the `Launching TUI` line.
        // `spawn_background_refresh_with_policy` already uses this same
        // `join_all` shape; we mirror it here so the initial frame matches.
        let pending: Vec<&SessionSummary> = sessions
            .iter()
            .filter(|session| {
                let context = MermaidCacheContext::from_session(session);
                self.session_mermaid_cache
                    .get(&session.session_id)
                    .map(|entry| entry.context != context)
                    .unwrap_or(true)
            })
            .collect();

        if !pending.is_empty() {
            let client = &self.client;
            let results = self.runtime.block_on(async {
                futures::future::join_all(
                    pending
                        .iter()
                        .map(|session| client.fetch_mermaid_artifact(&session.session_id)),
                )
                .await
            });
            for (session, result) in pending.iter().zip(results) {
                self.apply_mermaid_artifact_result(session, result);
            }
        }
        self.rebuild_mermaid_artifacts_from_cache();
    }

    pub(crate) fn open_mermaid_artifact(&mut self) {
        let Some(path) = (match &self.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => {
                if viewer.active_tab != DomainPlanTab::Schema {
                    swimmers::session::artifacts::resolve_viewer_text_path(
                        &viewer.cwd,
                        viewer.path.as_deref(),
                        viewer.active_tab.filename(),
                    )
                    .map(|path| path.to_string_lossy().into_owned())
                } else {
                    viewer.openable_path().map(str::to_string)
                }
            }
            FishBowlMode::Aquarium => None,
        }) else {
            self.set_message("artifact path unavailable");
            return;
        };

        let path_label = path_tail_label(&path).unwrap_or_else(|| path.clone());
        match self.artifact_opener.open(&path) {
            Ok(_) => self.set_message(format!("open artifact -> {path_label}")),
            Err(err) => self.set_message(format!("failed to open artifact: {err}")),
        }
    }

    fn mermaid_viewer_state(
        session_id: String,
        tmux_name: String,
        cwd: String,
        path: Option<String>,
        source: Option<String>,
        artifact_error: Option<String>,
        plan_tabs: Option<Vec<DomainPlanTab>>,
        disk_only: bool,
        inline_plan_files: BTreeMap<DomainPlanTab, String>,
    ) -> MermaidViewerState {
        MermaidViewerState {
            session_id,
            tmux_name,
            cwd,
            path,
            source,
            artifact_error,
            render_error: None,
            unsupported_reason: detect_mermaid_backend_support(),
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
            inline_plan_files,
            plan_text_content: None,
            plan_text_lines: Vec::new(),
            plan_text_scroll: 0,
            plan_text_cached_width: 0,
            tab_rects: Vec::new(),
            disk_only,
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

        let should_revalidate = self.session_mermaid_cache.contains_key(&session.session_id)
            || !self.mermaid_artifacts.contains_key(&session.session_id);
        if should_revalidate {
            self.refresh_single_mermaid_artifact(&session, true);
        }

        let Some(artifact) = self.mermaid_artifacts.get(&session.session_id).cloned() else {
            self.set_message("no Mermaid artifact found");
            return;
        };

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
        self.fish_bowl_mode = FishBowlMode::Mermaid(Self::mermaid_viewer_state(
            session.session_id.clone(),
            session.tmux_name.clone(),
            session.cwd.clone(),
            artifact.path,
            artifact.source,
            artifact.error,
            plan_tabs,
            false,
            BTreeMap::new(),
        ));
    }

    /// Open the Mermaid/plan viewer directly from a `schema.mmd` path on disk.
    ///
    /// Unlike `open_mermaid_viewer`, this has no backing tmux session — the
    /// source is a skillbox-overlay plan directory. Plan tabs are populated by
    /// stat'ing sibling files, and tab content is read straight from disk.
    pub(crate) fn open_plan_viewer(&mut self, schema_path: String, slug: String) {
        let path = std::path::PathBuf::from(&schema_path);
        let Some(parent) = path.parent() else {
            self.set_message("plan path has no parent directory");
            return;
        };
        let cwd = parent.to_string_lossy().into_owned();
        let siblings = swimmers::session::artifacts::list_plan_siblings(&schema_path);
        let session_id = format!("plan::{schema_path}");
        let (source, artifact_error) = match std::fs::read_to_string(&path) {
            Ok(source) => (Some(source), None),
            Err(err) => (None, Some(format!("read {}: {err}", path.display()))),
        };

        let plan_tabs = {
            let mut tabs = vec![DomainPlanTab::Schema];
            for name in &siblings {
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
        };

        self.fish_bowl_mode = FishBowlMode::Mermaid(Self::mermaid_viewer_state(
            session_id,
            slug,
            cwd,
            Some(schema_path),
            source,
            artifact_error,
            plan_tabs,
            true,
            BTreeMap::new(),
        ));
    }

    pub(crate) fn open_skill_atlas_viewer(&mut self, action: SkillPanelAction) {
        let source = skill_atlas_mermaid_source(self, &action);
        let plan_text = skill_atlas_plan_text(self, &action);
        let cwd = self
            .selected()
            .map(|entity| entity.session.cwd.clone())
            .unwrap_or_default();
        let title = skill_atlas_focus_title(&action);
        let mut inline_plan_files = BTreeMap::new();
        inline_plan_files.insert(DomainPlanTab::Plan, plan_text);
        self.fish_bowl_mode = FishBowlMode::Mermaid(Self::mermaid_viewer_state(
            format!("skill-atlas::{title}"),
            format!("skill atlas: {title}"),
            cwd,
            skill_atlas_focus_path(&action),
            Some(source),
            None,
            Some(vec![DomainPlanTab::Schema, DomainPlanTab::Plan]),
            true,
            inline_plan_files,
        ));
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
                    .is_some_and(|tabs| tabs.contains(&tab))
                    && viewer.active_tab != tab
            }
            FishBowlMode::Aquarium => false,
        };
        if !is_valid {
            return;
        }

        if tab != DomainPlanTab::Schema {
            let (session_id, schema_path, disk_only, inline_content) = match &self.fish_bowl_mode {
                FishBowlMode::Mermaid(v) => (
                    v.session_id.clone(),
                    v.path.clone(),
                    v.disk_only,
                    v.inline_plan_files.get(&tab).cloned(),
                ),
                _ => return,
            };
            let result = if let Some(content) = inline_content {
                Ok(PlanFileResponse {
                    session_id: session_id.clone(),
                    name: tab.filename().to_string(),
                    content: Some(content),
                    error: None,
                })
            } else if disk_only {
                read_plan_file_from_disk(schema_path.as_deref(), tab.filename())
            } else {
                self.runtime
                    .block_on(self.client.fetch_plan_file(&session_id, tab.filename()))
            };
            let viewer = self.mermaid_viewer_mut().unwrap();
            viewer.active_tab = tab;
            viewer.plan_text_scroll = 0;
            viewer.plan_text_lines.clear();
            viewer.plan_text_cached_width = 0;
            match result {
                Ok(response) => {
                    viewer.plan_text_content = response.content;
                    if let Some(err) = response.error {
                        self.set_message(format!("artifact file: {err}"));
                    }
                }
                Err(err) => {
                    viewer.plan_text_content = None;
                    self.set_message(format!("artifact file fetch failed: {err}"));
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
}

/// Read a plan sibling file from disk, matching the shape of the server's
/// `fetch_plan_file` response so disk-backed and session-backed viewers can
/// share the tab-switching code path.
pub(crate) fn read_plan_file_from_disk(
    schema_path: Option<&str>,
    filename: &str,
) -> Result<PlanFileResponse, String> {
    let Some(schema_path) = schema_path else {
        return Err("plan viewer has no schema path".to_string());
    };
    let Some(dir) = std::path::Path::new(schema_path).parent() else {
        return Err("plan schema path has no parent".to_string());
    };
    let response = PlanFileResponse {
        session_id: format!("plan::{schema_path}"),
        name: filename.to_string(),
        content: None,
        error: None,
    };

    if !swimmers::session::artifacts::VIEWER_TEXT_FILENAMES.contains(&filename) {
        return Ok(PlanFileResponse {
            error: Some(format!("artifact file name not allowed: {filename}")),
            ..response
        });
    }

    let cwd = dir.to_string_lossy();
    let Some(target) =
        swimmers::session::artifacts::resolve_viewer_text_path(&cwd, Some(schema_path), filename)
    else {
        return Ok(PlanFileResponse {
            error: Some(format!("artifact file unavailable: {filename}")),
            ..response
        });
    };

    match std::fs::read_to_string(&target) {
        Ok(content) => Ok(PlanFileResponse {
            content: Some(content),
            ..response
        }),
        Err(err) => Ok(PlanFileResponse {
            error: Some(format!("read {}: {err}", target.display())),
            ..response
        }),
    }
}
