use super::*;

#[path = "picker/render.rs"]
mod picker_render;

#[cfg(test)]
pub(crate) use picker_render::initial_request_layout;
pub(crate) use picker_render::{render_initial_request, render_picker};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PickerSelection {
    SpawnHere,
    Entry(usize),
}

#[derive(Clone)]
pub(crate) struct PickerState {
    pub(crate) anchor_x: u16,
    pub(crate) anchor_y: u16,
    pub(crate) base_path: String,
    pub(crate) current_path: String,
    pub(crate) entries: Vec<DirEntry>,
    pub(crate) repo_search_entries: Vec<DirEntry>,
    pub(crate) current_theme_color: Option<Color>,
    pub(crate) entry_theme_colors: Vec<Option<Color>>,
    pub(crate) managed_only: bool,
    pub(crate) overlay_label: Option<String>,
    pub(crate) spawn_tool: SpawnTool,
    pub(crate) launch_targets: Vec<LaunchTargetSummary>,
    pub(crate) launch_target: Option<String>,
    pub(crate) selection: PickerSelection,
    pub(crate) scroll: usize,
    /// When set, the picker is showing a virtual group listing.
    pub(crate) current_group: Option<String>,
    /// Group names available from the overlay (shown as header buttons).
    pub(crate) available_groups: Vec<String>,
    /// Group selected for add/remove/move edits from keyboard controls.
    pub(crate) group_edit_target: Option<String>,
    /// Live filter query typed by the user; filters `entries` by substring.
    pub(crate) search: String,
    pub(crate) batch_exclude_mode: bool,
    pub(crate) batch_excluded_paths: HashSet<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum PickerGroupUpdateMode {
    Add,
    Remove,
    Move,
}

pub(crate) struct PickerReloadPlan {
    pub(crate) path: Option<String>,
    pub(crate) managed_only: bool,
    pub(crate) group: Option<String>,
}

struct PickerGroupSelection {
    path: String,
    entry_label: String,
    target: String,
    memberships: Vec<String>,
    current_group: Option<String>,
}

pub(crate) struct PickerGroupMembershipDelta {
    pub(crate) add: Vec<String>,
    pub(crate) remove: Vec<String>,
}

impl PickerGroupMembershipDelta {
    pub(crate) fn has_changes(&self) -> bool {
        !self.add.is_empty() || !self.remove.is_empty()
    }
}

pub(crate) struct PickerGroupUpdatePlan {
    pub(crate) path: String,
    pub(crate) entry_label: String,
    pub(crate) delta: PickerGroupMembershipDelta,
    pub(crate) reload_path: String,
    pub(crate) managed_only: bool,
    pub(crate) group: Option<String>,
}

pub(crate) fn picker_managed_only_reload_plan(
    picker: &PickerState,
    managed_only: bool,
) -> Option<PickerReloadPlan> {
    if picker.managed_only == managed_only && picker.current_group.is_none() {
        return None;
    }
    Some(PickerReloadPlan {
        path: picker
            .current_group
            .is_none()
            .then(|| picker.current_path.clone()),
        managed_only,
        group: None,
    })
}

pub(crate) fn picker_group_update_plan(
    picker: &PickerState,
    mode: PickerGroupUpdateMode,
) -> Option<PickerGroupUpdatePlan> {
    let selection = selected_picker_group_entry(picker)?;
    let delta = picker_group_membership_delta(mode, &selection);
    Some(PickerGroupUpdatePlan {
        path: selection.path,
        entry_label: selection.entry_label,
        delta,
        reload_path: picker.current_path.clone(),
        managed_only: picker.managed_only,
        group: picker.current_group.clone(),
    })
}

fn selected_picker_group_entry(picker: &PickerState) -> Option<PickerGroupSelection> {
    let PickerSelection::Entry(index) = picker.selection else {
        return None;
    };
    let target = picker.group_edit_target.clone()?;
    let entry = picker.entry_at(index)?;
    Some(PickerGroupSelection {
        path: picker.path_for_entry(index)?,
        entry_label: entry.name.clone(),
        target,
        memberships: entry.groups.clone(),
        current_group: picker.current_group.clone(),
    })
}

fn picker_group_membership_delta(
    mode: PickerGroupUpdateMode,
    selection: &PickerGroupSelection,
) -> PickerGroupMembershipDelta {
    PickerGroupMembershipDelta {
        add: picker_group_memberships_to_add(mode, &selection.target),
        remove: picker_group_memberships_to_remove(
            mode,
            &selection.target,
            &selection.memberships,
            selection.current_group.as_deref(),
        ),
    }
}

fn picker_group_memberships_to_add(mode: PickerGroupUpdateMode, target: &str) -> Vec<String> {
    match mode {
        PickerGroupUpdateMode::Add | PickerGroupUpdateMode::Move => vec![target.to_string()],
        PickerGroupUpdateMode::Remove => Vec::new(),
    }
}

fn picker_group_memberships_to_remove(
    mode: PickerGroupUpdateMode,
    target: &str,
    memberships: &[String],
    current_group: Option<&str>,
) -> Vec<String> {
    match mode {
        PickerGroupUpdateMode::Add => Vec::new(),
        PickerGroupUpdateMode::Remove => vec![target.to_string()],
        PickerGroupUpdateMode::Move => {
            picker_group_move_removals(target, memberships, current_group)
        }
    }
}

fn picker_group_move_removals(
    target: &str,
    memberships: &[String],
    current_group: Option<&str>,
) -> Vec<String> {
    let mut remove = picker_groups_except(memberships, target);
    if let Some(current) = current_group.filter(|current| *current != target) {
        push_group_once(&mut remove, current);
    }
    remove
}

fn picker_groups_except(groups: &[String], excluded: &str) -> Vec<String> {
    groups
        .iter()
        .filter(|group| group.as_str() != excluded)
        .cloned()
        .collect()
}

fn push_group_once(groups: &mut Vec<String>, group: &str) {
    if !groups.iter().any(|existing| existing == group) {
        groups.push(group.to_string());
    }
}

fn picker_move_selection_is_noop(selection: PickerSelection, visible: &[usize]) -> bool {
    visible.is_empty() && matches!(selection, PickerSelection::SpawnHere)
}

fn picker_moved_selection(
    selection: PickerSelection,
    delta: isize,
    visible: &[usize],
) -> PickerSelection {
    let current_pos = picker_selection_visible_position(selection, visible);
    let next_pos = clamped_picker_selection_position(current_pos, delta, visible.len());
    picker_selection_for_visible_position(next_pos, visible)
}

fn picker_selection_visible_position(selection: PickerSelection, visible: &[usize]) -> isize {
    match selection {
        PickerSelection::SpawnHere => 0,
        PickerSelection::Entry(index) => picker_entry_visible_position(index, visible).unwrap_or(0),
    }
}

fn picker_entry_visible_position(index: usize, visible: &[usize]) -> Option<isize> {
    visible
        .iter()
        .position(|candidate| *candidate == index)
        .map(|pos| pos as isize + 1)
}

fn clamped_picker_selection_position(
    current_pos: isize,
    delta: isize,
    visible_entry_count: usize,
) -> isize {
    (current_pos + delta).clamp(0, visible_entry_count as isize)
}

fn picker_selection_for_visible_position(position: isize, visible: &[usize]) -> PickerSelection {
    position
        .checked_sub(1)
        .and_then(|entry_pos| usize::try_from(entry_pos).ok())
        .and_then(|entry_pos| visible.get(entry_pos).copied())
        .map(PickerSelection::Entry)
        .unwrap_or(PickerSelection::SpawnHere)
}

impl PickerState {
    pub(crate) fn new(
        anchor_x: u16,
        anchor_y: u16,
        response: DirListResponse,
        managed_only: bool,
        spawn_tool: SpawnTool,
        preferred_launch_target: Option<String>,
    ) -> Self {
        let launch_targets = normalized_launch_targets(response.launch_targets);
        let launch_target = select_launch_target(
            preferred_launch_target.as_deref(),
            response.default_launch_target.as_deref(),
            &launch_targets,
        );
        Self {
            anchor_x,
            anchor_y,
            base_path: response.path.clone(),
            current_path: response.path,
            entries: response.entries,
            repo_search_entries: Vec::new(),
            current_theme_color: None,
            entry_theme_colors: Vec::new(),
            managed_only,
            overlay_label: response.overlay_label,
            spawn_tool,
            launch_targets,
            launch_target,
            selection: PickerSelection::SpawnHere,
            scroll: 0,
            current_group: None,
            group_edit_target: response.groups.first().cloned(),
            available_groups: response.groups,
            search: String::new(),
            batch_exclude_mode: false,
            batch_excluded_paths: HashSet::new(),
        }
    }

    pub(crate) fn visible_entries(&self) -> Vec<usize> {
        match self.search_needle() {
            Some(needle) => self.visible_entries_for_search(&needle),
            None => self.local_entry_indices().collect(),
        }
    }

    fn search_needle(&self) -> Option<String> {
        (!self.search.is_empty()).then(|| self.search.to_lowercase())
    }

    fn visible_entries_for_search(&self, needle: &str) -> Vec<usize> {
        let seen_local_paths = self.local_entry_paths();
        self.matching_local_entry_indices(needle)
            .chain(self.matching_repo_search_entry_indices(needle, &seen_local_paths))
            .collect()
    }

    fn local_entry_indices(&self) -> std::ops::Range<usize> {
        0..self.entries.len()
    }

    fn repo_search_entry_indices(&self) -> std::ops::Range<usize> {
        self.entries.len()..self.total_entry_count()
    }

    fn local_entry_paths(&self) -> HashSet<String> {
        self.local_entry_indices()
            .filter_map(|index| self.path_for_entry(index))
            .map(|path| normalize_path(&path))
            .collect()
    }

    fn matching_local_entry_indices<'a>(
        &'a self,
        needle: &'a str,
    ) -> impl Iterator<Item = usize> + 'a {
        self.local_entry_indices()
            .filter(move |index| self.entry_matches_search(*index, needle))
    }

    fn matching_repo_search_entry_indices<'a>(
        &'a self,
        needle: &'a str,
        seen_local_paths: &'a HashSet<String>,
    ) -> impl Iterator<Item = usize> + 'a {
        self.repo_search_entry_indices()
            .filter(move |index| {
                self.repo_search_entry_is_not_local_duplicate(*index, seen_local_paths)
            })
            .filter(move |index| self.entry_matches_search(*index, needle))
    }

    fn repo_search_entry_is_not_local_duplicate(
        &self,
        index: usize,
        seen_local_paths: &HashSet<String>,
    ) -> bool {
        self.path_for_entry(index)
            .map(|path| !seen_local_paths.contains(&normalize_path(&path)))
            .unwrap_or(false)
    }

    pub(crate) fn total_entry_count(&self) -> usize {
        self.entries.len() + self.repo_search_entries.len()
    }

    pub(crate) fn entry_at(&self, index: usize) -> Option<&DirEntry> {
        if index < self.entries.len() {
            self.entries.get(index)
        } else {
            self.repo_search_entries.get(index - self.entries.len())
        }
    }

    fn entry_matches_search(&self, index: usize, needle: &str) -> bool {
        let Some(entry) = self.entry_at(index) else {
            return false;
        };
        entry.name.to_lowercase().contains(needle)
            || self
                .path_for_entry(index)
                .map(|path| path.to_lowercase().contains(needle))
                .unwrap_or(false)
    }

    pub(crate) fn set_repo_search_entries(&mut self, entries: Vec<DirEntry>) {
        self.repo_search_entries = entries;
        self.entry_theme_colors.clear();
        self.snap_selection_to_visible();
    }

    pub(crate) fn snap_selection_to_visible(&mut self) {
        let visible = self.visible_entries();
        match self.selection {
            PickerSelection::SpawnHere => {}
            PickerSelection::Entry(index) => {
                if !visible.contains(&index) {
                    self.selection = visible
                        .first()
                        .copied()
                        .map(PickerSelection::Entry)
                        .unwrap_or(PickerSelection::SpawnHere);
                }
            }
        }
        self.scroll = 0;
    }

    pub(crate) fn batch_dirs_for_visible_entries(&self) -> Vec<String> {
        self.visible_entries()
            .into_iter()
            .filter_map(|index| {
                let path = self.path_for_entry(index)?;
                (!self.batch_path_is_excluded(&path)).then_some(path)
            })
            .collect()
    }

    pub(crate) fn batch_included_count(&self) -> usize {
        self.batch_dirs_for_visible_entries().len()
    }

    pub(crate) fn batch_path_is_excluded(&self, path: &str) -> bool {
        self.batch_excluded_paths.contains(&normalize_path(path))
    }

    pub(crate) fn batch_entry_is_excluded(&self, index: usize) -> bool {
        self.path_for_entry(index)
            .map(|path| self.batch_path_is_excluded(&path))
            .unwrap_or(false)
    }

    pub(crate) fn toggle_batch_exclusion(&mut self, index: usize) {
        let Some(path) = self.path_for_entry(index) else {
            return;
        };
        let normalized = normalize_path(&path);
        if !self.batch_excluded_paths.insert(normalized.clone()) {
            self.batch_excluded_paths.remove(&normalized);
        }
    }

    fn retain_current_batch_exclusions(&mut self) {
        let valid_paths = (0..self.total_entry_count())
            .filter_map(|index| self.path_for_entry(index))
            .map(|path| normalize_path(&path))
            .collect::<HashSet<_>>();
        self.batch_excluded_paths
            .retain(|path| valid_paths.contains(path));
    }

    pub(crate) fn apply_response(&mut self, response: DirListResponse, preserve_selection: bool) {
        let previous = self.response_snapshot();
        self.replace_response_entries(
            response,
            preserve_selection,
            previous.launch_target.as_deref(),
        );
        self.clear_response_theme_colors();
        self.apply_response_position(&previous, preserve_selection);
        self.apply_response_batch_exclusions(preserve_selection);
    }

    fn response_snapshot(&self) -> PickerResponseSnapshot {
        PickerResponseSnapshot {
            selection: self.selection,
            scroll: self.scroll,
            selected_path: self.selected_response_path(),
            launch_target: self.launch_target.clone(),
        }
    }

    fn selected_response_path(&self) -> Option<String> {
        let PickerSelection::Entry(index) = self.selection else {
            return None;
        };
        self.path_for_entry(index).map(|path| normalize_path(&path))
    }

    fn replace_response_entries(
        &mut self,
        response: DirListResponse,
        preserve_selection: bool,
        previous_launch_target: Option<&str>,
    ) {
        self.current_path = response.path;
        self.entries = response.entries;
        self.overlay_label = response.overlay_label;
        self.launch_targets = normalized_launch_targets(response.launch_targets);
        self.launch_target = select_launch_target(
            preserved_launch_target(preserve_selection, previous_launch_target),
            response.default_launch_target.as_deref(),
            &self.launch_targets,
        );
        self.apply_response_groups(response.groups);
    }

    fn apply_response_groups(&mut self, groups: Vec<String>) {
        let Some(groups) = non_empty_groups(groups) else {
            return;
        };
        self.available_groups = groups;
        self.retain_or_fallback_group_edit_target();
    }

    fn retain_or_fallback_group_edit_target(&mut self) {
        self.group_edit_target = self
            .group_edit_target
            .clone()
            .filter(|target| self.available_groups.iter().any(|group| group == target))
            .or_else(|| self.available_groups.first().cloned());
    }

    fn clear_response_theme_colors(&mut self) {
        self.current_theme_color = None;
        self.entry_theme_colors.clear();
    }

    fn apply_response_position(
        &mut self,
        previous: &PickerResponseSnapshot,
        preserve_selection: bool,
    ) {
        let position = preserve_selection
            .then(|| preserved_response_position(self, previous))
            .unwrap_or_default();
        self.selection = position.selection;
        self.scroll = position.scroll;
        snap_response_selection_to_search(self);
    }

    fn apply_response_batch_exclusions(&mut self, preserve_selection: bool) {
        if preserve_selection {
            self.retain_current_batch_exclusions();
        } else {
            self.batch_excluded_paths.clear();
        }
    }

    pub(crate) fn sync_theme_colors(&mut self, repo_themes: &mut HashMap<String, RepoTheme>) {
        self.current_theme_color = picker_theme_color_for_path(&self.current_path, repo_themes);
        self.entry_theme_colors = vec![None; self.total_entry_count()];
        let mut indices = (0..self.entries.len()).collect::<Vec<_>>();
        if !self.search.is_empty() {
            indices.extend(
                self.visible_entries()
                    .into_iter()
                    .filter(|index| *index >= self.entries.len()),
            );
        }
        for index in indices {
            self.entry_theme_colors[index] = self
                .path_for_entry(index)
                .and_then(|path| picker_theme_color_for_path(&path, repo_themes));
        }
    }

    pub(crate) fn at_root(&self) -> bool {
        self.current_group.is_none()
            && normalize_path(&self.current_path) == normalize_path(&self.base_path)
    }

    pub(crate) fn parent_path(&self) -> Option<String> {
        (!self.at_root())
            .then(|| parent_path_for(&self.current_path))
            .flatten()
    }

    pub(crate) fn relative_label(&self) -> String {
        if let Some(group) = &self.current_group {
            return format!("/{group}");
        }
        let base = normalize_path(&self.base_path);
        let current = normalize_path(&self.current_path);
        if current == base {
            return "/".to_string();
        }
        current
            .strip_prefix(&base)
            .filter(|suffix| !suffix.is_empty())
            .map(|suffix| suffix.to_string())
            .unwrap_or(current)
    }

    pub(crate) fn path_for_entry(&self, index: usize) -> Option<String> {
        let entry = self.entry_at(index)?;
        if let Some(full_path) = &entry.full_path {
            return Some(full_path.clone());
        }
        Some(join_path(&self.current_path, &entry.name))
    }

    pub(crate) fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        let visible = self.visible_entries();
        if picker_move_selection_is_noop(self.selection, &visible) {
            return;
        }

        self.selection = picker_moved_selection(self.selection, delta, &visible);
        self.ensure_selection_visible(visible_rows);
    }

    pub(crate) fn ensure_selection_visible(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            self.scroll = 0;
            return;
        }
        let PickerSelection::Entry(index) = self.selection else {
            self.scroll = 0;
            return;
        };
        let visible = self.visible_entries();
        let Some(pos) = visible.iter().position(|i| *i == index) else {
            self.scroll = 0;
            return;
        };

        if pos < self.scroll {
            self.scroll = pos;
            return;
        }

        let last_visible = self.scroll + visible_rows.saturating_sub(1);
        if pos > last_visible {
            self.scroll = pos + 1 - visible_rows;
        }
    }

    pub(crate) fn toggle_launch_target(&mut self) -> Option<String> {
        if self.launch_targets.is_empty() {
            self.launch_targets = vec![LaunchTargetSummary::local()];
        }
        let current = self.launch_target.as_deref().unwrap_or("local");
        let index = self
            .launch_targets
            .iter()
            .position(|target| target.id == current)
            .unwrap_or(0);
        let next = (index + 1) % self.launch_targets.len();
        self.launch_target = Some(self.launch_targets[next].id.clone());
        self.launch_target.clone()
    }

    pub(crate) fn launch_target_label(&self) -> String {
        let current = self.launch_target.as_deref().unwrap_or("local");
        self.launch_targets
            .iter()
            .find(|target| target.id == current)
            .map(|target| target.label.clone())
            .unwrap_or_else(|| current.to_string())
    }

    pub(crate) fn cycle_group_edit_target(&mut self) -> Option<String> {
        if self.available_groups.is_empty() {
            self.group_edit_target = None;
            return None;
        }
        let current = self.group_edit_target.as_deref();
        let index = current
            .and_then(|target| {
                self.available_groups
                    .iter()
                    .position(|group| group == target)
            })
            .unwrap_or(0);
        let next = if current.is_some() {
            (index + 1) % self.available_groups.len()
        } else {
            index
        };
        self.group_edit_target = Some(self.available_groups[next].clone());
        self.group_edit_target.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PickerAction {
    Close,
    Up,
    ToggleManaged(bool),
    ActivateGroup(String),
    CycleGroupEditTarget,
    ToggleTool,
    ToggleLaunchTarget,
    ToggleBatchExcludeMode,
    BatchVisible,
    ActivateCurrentPath,
    ActivateEntry(usize),
    ToggleBatchExclude(usize),
    StartRepoAction(usize, RepoActionKind),
}

#[derive(Clone)]
pub(crate) struct PickerLayout {
    pub(crate) frame: Rect,
    pub(crate) content: Rect,
    pub(crate) back_button: Option<Rect>,
    pub(crate) close_button: Rect,
    pub(crate) env_button: Rect,
    pub(crate) group_buttons: Vec<(String, Rect)>,
    pub(crate) all_button: Rect,
    pub(crate) group_target_button: Option<Rect>,
    pub(crate) tool_button: Rect,
    pub(crate) launch_target_button: Rect,
    pub(crate) exclude_button: Rect,
    pub(crate) batch_button: Rect,
    pub(crate) spawn_here_button: Rect,
    pub(crate) first_entry_y: u16,
    pub(crate) visible_entry_rows: usize,
    /// Indices into `picker.entries` that pass the current search filter.
    pub(crate) visible_entries: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InitialRequestState {
    pub(crate) cwd: String,
    pub(crate) value: String,
    pub(crate) batch_dirs: Option<Vec<String>>,
    pub(crate) launch_target: Option<String>,
}

impl InitialRequestState {
    pub(crate) fn new(cwd: String, launch_target: Option<String>) -> Self {
        Self {
            cwd,
            value: String::new(),
            batch_dirs: None,
            launch_target,
        }
    }

    pub(crate) fn new_batch(dirs: Vec<String>, launch_target: Option<String>) -> Self {
        Self {
            cwd: dirs.first().cloned().unwrap_or_default(),
            value: String::new(),
            batch_dirs: Some(dirs),
            launch_target,
        }
    }

    pub(crate) fn trimmed_value(&self) -> Option<String> {
        let trimmed = self.value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct InitialRequestLayout {
    pub(crate) frame: Rect,
    pub(crate) content: Rect,
    pub(crate) input_y: u16,
}

pub(crate) fn tool_button_label(tool: SpawnTool) -> String {
    format!("[{}]", tool.label())
}

pub(crate) fn exclude_button_label() -> &'static str {
    "[exclude]"
}

pub(crate) fn batch_button_label(picker: &PickerState) -> String {
    format!("[batch {}]", picker.batch_included_count())
}

pub(crate) fn launch_target_button_label(picker: &PickerState) -> String {
    format!("[{}]", picker.launch_target_label())
}

fn normalized_launch_targets(mut targets: Vec<LaunchTargetSummary>) -> Vec<LaunchTargetSummary> {
    if targets.is_empty() {
        return vec![LaunchTargetSummary::local()];
    }
    if !targets.iter().any(|target| target.id == "local") {
        targets.insert(0, LaunchTargetSummary::local());
    }
    targets
}

fn select_launch_target(
    preferred: Option<&str>,
    fallback: Option<&str>,
    targets: &[LaunchTargetSummary],
) -> Option<String> {
    let candidate = preferred
        .filter(|id| targets.iter().any(|target| target.id == *id))
        .or_else(|| fallback.filter(|id| targets.iter().any(|target| target.id == *id)))
        .unwrap_or("local");
    Some(candidate.to_string())
}

pub(crate) fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parent_path_for(path: &str) -> Option<String> {
    let normalized = normalize_path(path);
    std::path::Path::new(&normalized)
        .parent()
        .map(parent_path_string)
}

fn parent_path_string(parent: &std::path::Path) -> String {
    let raw = parent.to_string_lossy().into_owned();
    (!raw.is_empty())
        .then_some(raw)
        .unwrap_or_else(|| "/".to_string())
}

pub(crate) fn join_path(base: &str, name: &str) -> String {
    let base = normalize_path(base);
    let name = name.trim_matches('/');
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

pub(crate) fn kind_label(kind: RepoActionKind) -> &'static str {
    match kind {
        RepoActionKind::Commit => "commit",
        RepoActionKind::Restart => "restart",
        RepoActionKind::Open => "open",
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ActionLabel {
    pub(crate) text: String,
    pub(crate) kind: RepoActionKind,
    pub(crate) color: Color,
    pub(crate) clickable: bool,
}

fn picker_entry_action_order() -> [RepoActionKind; 3] {
    [
        RepoActionKind::Commit,
        RepoActionKind::Restart,
        RepoActionKind::Open,
    ]
}

fn picker_entry_action_label(
    text: &str,
    kind: RepoActionKind,
    color: Color,
    clickable: bool,
) -> ActionLabel {
    ActionLabel {
        text: text.into(),
        kind,
        color,
        clickable,
    }
}

fn picker_entry_running_action(entry: &DirEntry) -> Option<ActionLabel> {
    let tracked = entry.repo_action.as_ref()?;
    (tracked.state == RepoActionState::Running)
        .then(|| picker_entry_action_label("[running]", tracked.kind, Color::Yellow, false))
}

fn picker_entry_status_action(
    entry: &DirEntry,
    kind: RepoActionKind,
    state: RepoActionState,
) -> Option<ActionLabel> {
    match (kind, state) {
        (RepoActionKind::Commit, RepoActionState::Failed) => Some(picker_entry_action_label(
            "[failed]",
            kind,
            Color::Red,
            false,
        )),
        (RepoActionKind::Commit, RepoActionState::Succeeded)
            if !picker_entry_commit_available(entry) =>
        {
            Some(picker_entry_action_label(
                "[done]",
                kind,
                Color::Green,
                false,
            ))
        }
        (RepoActionKind::Restart, RepoActionState::Failed) => Some(picker_entry_action_label(
            "[failed]",
            kind,
            Color::Red,
            false,
        )),
        (RepoActionKind::Restart, RepoActionState::Succeeded) => Some(picker_entry_action_label(
            "[done]",
            kind,
            Color::Green,
            false,
        )),
        _ => None,
    }
}

fn picker_entry_available_action(entry: &DirEntry, kind: RepoActionKind) -> Option<ActionLabel> {
    match kind {
        RepoActionKind::Commit if picker_entry_commit_available(entry) => Some(
            picker_entry_action_label("[commit]", kind, Color::Green, true),
        ),
        RepoActionKind::Restart if picker_entry_restart_available(entry) => Some(
            picker_entry_action_label("[restart]", kind, Color::Yellow, true),
        ),
        RepoActionKind::Open if picker_entry_open_available(entry) => {
            Some(picker_entry_action_label("[open]", kind, Color::Cyan, true))
        }
        _ => None,
    }
}

fn picker_entry_commit_available(entry: &DirEntry) -> bool {
    entry.repo_dirty.unwrap_or(false)
}

fn picker_entry_restart_available(entry: &DirEntry) -> bool {
    entry.has_restart.unwrap_or(false)
}

fn picker_entry_open_available(entry: &DirEntry) -> bool {
    entry.open_url.is_some()
}

fn picker_entry_action_for_kind(entry: &DirEntry, kind: RepoActionKind) -> Option<ActionLabel> {
    let tracked = entry.repo_action.as_ref();
    if tracked.map(|action| action.kind) != Some(kind) {
        return picker_entry_available_action(entry, kind);
    }

    let state = tracked.map(|action| action.state)?;
    picker_entry_status_action(entry, kind, state)
        .or_else(|| picker_entry_available_action(entry, kind))
}

/// Compute all action labels for a directory entry. Returns labels ordered
/// left-to-right as they should appear in the picker row.
pub(crate) fn picker_entry_actions(entry: &DirEntry) -> Vec<ActionLabel> {
    if let Some(action) = picker_entry_running_action(entry) {
        return vec![action];
    }

    picker_entry_action_order()
        .into_iter()
        .filter_map(|kind| picker_entry_action_for_kind(entry, kind))
        .collect()
}

/// Total display width of all action labels (including spaces between them).
fn picker_entry_actions_width(actions: &[ActionLabel]) -> u16 {
    let text_width: u16 = actions.iter().map(picker_entry_action_width).sum();
    text_width + actions.len().saturating_sub(1) as u16
}

fn picker_entry_action_width(action: &ActionLabel) -> u16 {
    action.text.len() as u16
}

fn picker_entry_action_advance(action: &ActionLabel) -> u16 {
    picker_entry_action_width(action) + 1
}

fn picker_entry_actions_start_x(actions: &[ActionLabel], right: u16) -> u16 {
    right.saturating_sub(picker_entry_actions_width(actions))
}

fn picker_visible_entry_row_y(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
) -> Option<u16> {
    let row = visible_pos.checked_sub(picker.scroll)?;
    (row < layout.visible_entry_rows).then_some(layout.first_entry_y + row as u16)
}

fn picker_clickable_action_rect(
    action: &ActionLabel,
    x: u16,
    y: u16,
) -> Option<(Rect, RepoActionKind)> {
    action.clickable.then_some((
        Rect {
            x,
            y,
            width: picker_entry_action_width(action),
            height: 1,
        },
        action.kind,
    ))
}

fn picker_clickable_action_rects(
    actions: &[ActionLabel],
    right: u16,
    y: u16,
) -> Vec<(Rect, RepoActionKind)> {
    let mut x = picker_entry_actions_start_x(actions, right);
    actions
        .iter()
        .filter_map(|action| {
            let rect = picker_clickable_action_rect(action, x, y);
            x += picker_entry_action_advance(action);
            rect
        })
        .collect()
}

/// Compute click-target rects for all action labels on a given entry row.
fn picker_entry_action_rects(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
) -> Vec<(Rect, RepoActionKind)> {
    match (
        picker_visible_entry_row_y(picker, layout, visible_pos),
        picker.entry_at(raw_index),
    ) {
        (Some(row_y), Some(entry)) => picker_clickable_action_rects(
            &picker_entry_actions(entry),
            layout.content.right(),
            row_y,
        ),
        _ => Vec::new(),
    }
}

fn picker_batch_exclude_rect(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
) -> Option<Rect> {
    if !picker.batch_exclude_mode {
        return None;
    }

    let row_y = picker_visible_entry_row_y(picker, layout, visible_pos)?;
    let entry = picker.entry_at(raw_index)?;
    let actions_width = picker_entry_actions_width(&picker_entry_actions(entry));
    let label = picker_batch_exclude_label(picker, raw_index);
    let label_width = label.len() as u16;
    let total_width = label_width
        + if actions_width > 0 {
            actions_width + 1
        } else {
            0
        };
    Some(Rect {
        x: layout.content.right().saturating_sub(total_width),
        y: row_y,
        width: label_width,
        height: 1,
    })
}

fn picker_batch_exclude_label(picker: &PickerState, raw_index: usize) -> &'static str {
    if picker.batch_entry_is_excluded(raw_index) {
        "[in]"
    } else {
        "[out]"
    }
}

pub(crate) fn picker_layout(picker: &PickerState, field: Rect) -> PickerLayout {
    let _ = (picker.anchor_x, picker.anchor_y);
    let frame = field;
    let content = frame.inset(1);
    let header_rows = 4u16;
    let entry_capacity = content.height.saturating_sub(header_rows).max(1);
    let visible_entries = picker.visible_entries();
    let list_rows = visible_entries.len().min(entry_capacity as usize).max(1) as u16;
    let close_button = Rect {
        x: content.right().saturating_sub(3),
        y: content.y,
        width: 3,
        height: 1,
    };
    let back_button = if picker.at_root() {
        None
    } else {
        Some(Rect {
            x: content.x,
            y: content.y + 1,
            width: 4,
            height: 1,
        })
    };
    let managed_label_width = match &picker.overlay_label {
        Some(label) => label.len() as u16 + 2, // [label]
        None => 9,                             // [managed]
    };
    let env_button = Rect {
        x: content.x,
        y: content.y + 2,
        width: managed_label_width,
        height: 1,
    };
    let mut next_group_x = env_button.right() + 1;
    let mut group_buttons: Vec<(String, Rect)> = Vec::new();
    for name in &picker.available_groups {
        let label_width = name.len() as u16 + 2; // [name]
        let rect = Rect {
            x: next_group_x.min(content.right().saturating_sub(label_width)),
            y: content.y + 2,
            width: label_width,
            height: 1,
        };
        next_group_x = rect.right() + 1;
        group_buttons.push((name.clone(), rect));
    }
    let all_button = Rect {
        x: next_group_x.min(content.right().saturating_sub(13)),
        y: content.y + 2,
        width: 13,
        height: 1,
    };
    let group_target_button = picker.group_edit_target.as_ref().map(|target| {
        let label_width = format!("[target:{target}]").len() as u16;
        let x = all_button
            .right()
            .saturating_add(1)
            .min(content.right().saturating_sub(label_width));
        Rect {
            x,
            y: content.y + 2,
            width: label_width,
            height: 1,
        }
    });
    let tool_label_width = tool_button_label(picker.spawn_tool).len() as u16;
    let tool_button = Rect {
        x: close_button.x.saturating_sub(tool_label_width + 1),
        y: content.y,
        width: tool_label_width,
        height: 1,
    };
    let launch_target_label_width = launch_target_button_label(picker).len() as u16;
    let launch_target_button = Rect {
        x: tool_button.x.saturating_sub(launch_target_label_width + 1),
        y: content.y,
        width: launch_target_label_width,
        height: 1,
    };
    let batch_label_width = batch_button_label(picker).len() as u16;
    let batch_button = Rect {
        x: launch_target_button.x.saturating_sub(batch_label_width + 1),
        y: content.y,
        width: batch_label_width,
        height: 1,
    };
    let exclude_label_width = exclude_button_label().len() as u16;
    let exclude_button = Rect {
        x: batch_button.x.saturating_sub(exclude_label_width + 1),
        y: content.y,
        width: exclude_label_width,
        height: 1,
    };

    PickerLayout {
        frame,
        content,
        back_button,
        close_button,
        env_button,
        group_buttons,
        all_button,
        group_target_button,
        tool_button,
        launch_target_button,
        exclude_button,
        batch_button,
        spawn_here_button: Rect {
            x: content.x,
            y: content.y + 3,
            width: content.width,
            height: 1,
        },
        first_entry_y: content.y + 4,
        visible_entry_rows: list_rows as usize,
        visible_entries,
    }
}

pub(crate) fn picker_action_at(
    picker: &PickerState,
    layout: &PickerLayout,
    x: u16,
    y: u16,
) -> Option<PickerAction> {
    if let Some(action) = picker_top_control_action_at(layout, x, y) {
        return Some(action);
    }
    if let Some(action) = picker_filter_action_at(layout, x, y) {
        return Some(action);
    }
    picker_entry_action_at(picker, layout, x, y)
}

fn picker_top_control_action_at(layout: &PickerLayout, x: u16, y: u16) -> Option<PickerAction> {
    picker_top_ordered_controls(layout)
        .find_map(|(rect, action)| rect.contains(x, y).then_some(action))
}

fn picker_top_ordered_controls(
    layout: &PickerLayout,
) -> impl Iterator<Item = (Rect, PickerAction)> + '_ {
    [
        (layout.close_button, PickerAction::Close),
        (layout.tool_button, PickerAction::ToggleTool),
        (
            layout.launch_target_button,
            PickerAction::ToggleLaunchTarget,
        ),
        (layout.exclude_button, PickerAction::ToggleBatchExcludeMode),
        (layout.batch_button, PickerAction::BatchVisible),
    ]
    .into_iter()
    .chain(
        layout
            .back_button
            .into_iter()
            .map(|button| (button, PickerAction::Up)),
    )
}

fn picker_filter_action_at(layout: &PickerLayout, x: u16, y: u16) -> Option<PickerAction> {
    picker_filter_control_at(layout, x, y).map(picker_filter_control_action)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerFilterControl<'a> {
    ManagedOnly,
    Group(&'a str),
    AllFolders,
    GroupEditTarget,
    SpawnHere,
}

fn picker_filter_control_at<'a>(
    layout: &'a PickerLayout,
    x: u16,
    y: u16,
) -> Option<PickerFilterControl<'a>> {
    picker_filter_ordered_controls(layout)
        .into_iter()
        .find_map(|(rect, control)| rect.contains(x, y).then_some(control))
}

fn picker_filter_control_action(control: PickerFilterControl<'_>) -> PickerAction {
    match control {
        PickerFilterControl::ManagedOnly => PickerAction::ToggleManaged(true),
        PickerFilterControl::Group(name) => PickerAction::ActivateGroup(name.to_string()),
        PickerFilterControl::AllFolders => PickerAction::ToggleManaged(false),
        PickerFilterControl::GroupEditTarget => PickerAction::CycleGroupEditTarget,
        PickerFilterControl::SpawnHere => PickerAction::ActivateCurrentPath,
    }
}

fn picker_entry_action_at(
    picker: &PickerState,
    layout: &PickerLayout,
    x: u16,
    y: u16,
) -> Option<PickerAction> {
    let (visible_pos, raw_index) = picker_visible_entry_at_pointer(picker, layout, x, y)?;
    Some(picker_entry_pointer_action(
        picker,
        layout,
        visible_pos,
        raw_index,
        x,
        y,
    ))
}

fn picker_visible_entry_at_pointer(
    picker: &PickerState,
    layout: &PickerLayout,
    x: u16,
    y: u16,
) -> Option<(usize, usize)> {
    if y < layout.first_entry_y || y >= layout.first_entry_y + layout.visible_entry_rows as u16 {
        return None;
    }
    if x < layout.content.x || x >= layout.content.right() {
        return None;
    }

    let visible_pos = picker.scroll + (y - layout.first_entry_y) as usize;
    let raw_index = *layout.visible_entries.get(visible_pos)?;
    Some((visible_pos, raw_index))
}

fn picker_entry_pointer_action(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
    x: u16,
    y: u16,
) -> PickerAction {
    match picker_entry_pointer_target(picker, layout, visible_pos, raw_index, x, y) {
        PickerEntryPointerTarget::BatchExclude => PickerAction::ToggleBatchExclude(raw_index),
        PickerEntryPointerTarget::RepoAction(kind) => {
            PickerAction::StartRepoAction(raw_index, kind)
        }
        PickerEntryPointerTarget::Entry => PickerAction::ActivateEntry(raw_index),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerEntryPointerTarget {
    BatchExclude,
    RepoAction(RepoActionKind),
    Entry,
}

fn picker_entry_pointer_target(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
    x: u16,
    y: u16,
) -> PickerEntryPointerTarget {
    if picker_batch_exclude_contains(picker, layout, visible_pos, raw_index, x, y) {
        return PickerEntryPointerTarget::BatchExclude;
    }
    if let Some(kind) = picker_repo_action_kind_at(picker, layout, visible_pos, raw_index, x, y) {
        return PickerEntryPointerTarget::RepoAction(kind);
    }
    PickerEntryPointerTarget::Entry
}

fn picker_batch_exclude_contains(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
    x: u16,
    y: u16,
) -> bool {
    picker_batch_exclude_rect(picker, layout, visible_pos, raw_index)
        .map(|rect| rect.contains(x, y))
        .unwrap_or(false)
}

fn picker_repo_action_kind_at(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
    x: u16,
    y: u16,
) -> Option<RepoActionKind> {
    picker_entry_action_rects(picker, layout, visible_pos, raw_index)
        .into_iter()
        .find_map(|(rect, kind)| rect.contains(x, y).then_some(kind))
}

fn picker_filter_ordered_controls(layout: &PickerLayout) -> Vec<(Rect, PickerFilterControl<'_>)> {
    let mut controls = Vec::with_capacity(layout.group_buttons.len() + 4);
    controls.push((layout.env_button, PickerFilterControl::ManagedOnly));
    controls.extend(
        layout
            .group_buttons
            .iter()
            .map(|(name, rect)| (*rect, PickerFilterControl::Group(name.as_str()))),
    );
    controls.push((layout.all_button, PickerFilterControl::AllFolders));
    controls.extend(
        layout
            .group_target_button
            .map(|rect| (rect, PickerFilterControl::GroupEditTarget)),
    );
    controls.push((layout.spawn_here_button, PickerFilterControl::SpawnHere));
    controls
}

pub(crate) fn picker_theme_color_for_path(
    path: &str,
    repo_themes: &mut HashMap<String, RepoTheme>,
) -> Option<Color> {
    let (theme_id, theme) = existing_repo_theme(path)?;
    let color = repo_theme_display_color(&theme.body)?;
    repo_themes.insert(theme_id, theme);
    Some(color)
}

#[cfg(test)]
mod tests;

struct PickerResponseSnapshot {
    selection: PickerSelection,
    scroll: usize,
    selected_path: Option<String>,
    launch_target: Option<String>,
}

#[derive(Clone, Copy)]
struct PickerPosition {
    selection: PickerSelection,
    scroll: usize,
}

impl Default for PickerPosition {
    fn default() -> Self {
        Self {
            selection: PickerSelection::SpawnHere,
            scroll: 0,
        }
    }
}

fn preserved_launch_target<'a>(
    preserve_selection: bool,
    previous_launch_target: Option<&'a str>,
) -> Option<&'a str> {
    preserve_selection
        .then_some(previous_launch_target)
        .flatten()
}

fn non_empty_groups(groups: Vec<String>) -> Option<Vec<String>> {
    (!groups.is_empty()).then_some(groups)
}

fn preserved_response_position(
    picker: &PickerState,
    previous: &PickerResponseSnapshot,
) -> PickerPosition {
    let total_entries = picker.total_entry_count();
    PickerPosition {
        selection: preserved_response_selection(picker, previous, total_entries),
        scroll: previous.scroll.min(total_entries.saturating_sub(1)),
    }
}

fn preserved_response_selection(
    picker: &PickerState,
    previous: &PickerResponseSnapshot,
    total_entries: usize,
) -> PickerSelection {
    previous
        .selected_path
        .as_ref()
        .and_then(|path| entry_index_for_normalized_path(picker, path, total_entries))
        .map(PickerSelection::Entry)
        .unwrap_or_else(|| fallback_picker_selection(previous.selection, total_entries))
}

fn entry_index_for_normalized_path(
    picker: &PickerState,
    normalized_path: &str,
    total_entries: usize,
) -> Option<usize> {
    (0..total_entries).find(|index| entry_normalized_path_matches(picker, *index, normalized_path))
}

fn entry_normalized_path_matches(
    picker: &PickerState,
    index: usize,
    normalized_path: &str,
) -> bool {
    picker
        .path_for_entry(index)
        .map(|candidate| normalize_path(&candidate) == normalized_path)
        .unwrap_or(false)
}

fn fallback_picker_selection(
    previous_selection: PickerSelection,
    total_entries: usize,
) -> PickerSelection {
    match previous_selection {
        PickerSelection::SpawnHere => PickerSelection::SpawnHere,
        PickerSelection::Entry(_) if total_entries == 0 => PickerSelection::SpawnHere,
        PickerSelection::Entry(index) => {
            PickerSelection::Entry(index.min(total_entries.saturating_sub(1)))
        }
    }
}

fn snap_response_selection_to_search(picker: &mut PickerState) {
    if !picker.search.is_empty() {
        picker.snap_selection_to_visible();
    }
}
