use super::*;

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
        if visible.is_empty() && matches!(self.selection, PickerSelection::SpawnHere) {
            return;
        }

        let total = visible.len() as isize + 1;
        let current_pos = match self.selection {
            PickerSelection::SpawnHere => 0_isize,
            PickerSelection::Entry(index) => visible
                .iter()
                .position(|i| *i == index)
                .map(|pos| pos as isize + 1)
                .unwrap_or(0),
        };
        let next_pos = (current_pos + delta).clamp(0, total.saturating_sub(1));
        self.selection = if next_pos == 0 {
            PickerSelection::SpawnHere
        } else {
            PickerSelection::Entry(visible[(next_pos - 1) as usize])
        };
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
    if actions.is_empty() {
        return 0;
    }
    let text_width: u16 = actions.iter().map(|a| a.text.len() as u16).sum();
    text_width + (actions.len() as u16 - 1) // spaces between labels
}

/// Compute click-target rects for all action labels on a given entry row.
fn picker_entry_action_rects(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
) -> Vec<(Rect, RepoActionKind)> {
    if visible_pos < picker.scroll || visible_pos >= picker.scroll + layout.visible_entry_rows {
        return Vec::new();
    }

    let Some(entry) = picker.entry_at(raw_index) else {
        return Vec::new();
    };
    let actions = picker_entry_actions(entry);
    if actions.is_empty() {
        return Vec::new();
    }

    let row_y = layout.first_entry_y + (visible_pos - picker.scroll) as u16;
    let total_width = picker_entry_actions_width(&actions);
    let mut x = layout.content.right().saturating_sub(total_width);
    let mut rects = Vec::new();

    for action in &actions {
        let w = action.text.len() as u16;
        if action.clickable {
            rects.push((
                Rect {
                    x,
                    y: row_y,
                    width: w,
                    height: 1,
                },
                action.kind,
            ));
        }
        x += w + 1; // +1 for space separator
    }
    rects
}

fn picker_batch_exclude_rect(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
) -> Option<Rect> {
    if !picker.batch_exclude_mode
        || visible_pos < picker.scroll
        || visible_pos >= picker.scroll + layout.visible_entry_rows
    {
        return None;
    }

    let entry = picker.entry_at(raw_index)?;
    let actions_width = picker_entry_actions_width(&picker_entry_actions(entry));
    let label = picker_batch_exclude_label(picker, raw_index);
    let label_width = label.len() as u16;
    let row_y = layout.first_entry_y + (visible_pos - picker.scroll) as u16;
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

pub(crate) fn render_picker(renderer: &mut Renderer, picker: &PickerState, field: Rect) {
    let layout = picker_layout(picker, field);
    let picker_color = picker.current_theme_color.unwrap_or(Color::White);
    let picker_accent = picker.current_theme_color.unwrap_or(Color::Cyan);

    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, picker_color);

    render_picker_header_controls(renderer, picker, &layout, picker_accent);
    render_picker_path_row(renderer, picker, &layout, picker_color);
    render_picker_filter_row(renderer, picker, &layout);
    render_picker_spawn_row(renderer, picker, &layout);
    render_picker_search_overlay(renderer, picker, &layout, picker_accent);
    render_picker_entries(renderer, picker, &layout);
}

fn render_picker_header_controls(
    renderer: &mut Renderer,
    picker: &PickerState,
    layout: &PickerLayout,
    picker_accent: Color,
) {
    let spawn_title = format!("spawn {}", picker.spawn_tool.label());
    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        &spawn_title,
        picker_accent,
    );
    renderer.draw_text(
        layout.tool_button.x,
        layout.tool_button.y,
        &tool_button_label(picker.spawn_tool),
        Color::White,
    );
    renderer.draw_text(
        layout.launch_target_button.x,
        layout.launch_target_button.y,
        &launch_target_button_label(picker),
        if picker.launch_targets.len() > 1 {
            Color::White
        } else {
            Color::DarkGrey
        },
    );
    renderer.draw_text(
        layout.batch_button.x,
        layout.batch_button.y,
        &batch_button_label(picker),
        if picker.batch_included_count() == 0 {
            Color::DarkGrey
        } else {
            Color::White
        },
    );
    renderer.draw_text(
        layout.exclude_button.x,
        layout.exclude_button.y,
        exclude_button_label(),
        if picker.batch_exclude_mode {
            Color::Cyan
        } else if layout.visible_entries.is_empty() {
            Color::DarkGrey
        } else {
            Color::White
        },
    );
    renderer.draw_text(
        layout.close_button.x,
        layout.close_button.y,
        "[x]",
        Color::DarkGrey,
    );
}

fn render_picker_path_row(
    renderer: &mut Renderer,
    picker: &PickerState,
    layout: &PickerLayout,
    picker_color: Color,
) {
    let path_x = layout
        .back_button
        .map(|button| {
            renderer.draw_text(button.x, button.y, "[..]", Color::DarkGrey);
            button.right().saturating_add(1)
        })
        .unwrap_or(layout.content.x);
    let path_width = layout.content.right().saturating_sub(path_x) as usize;
    let path_label = truncate_label(&picker.relative_label(), path_width);
    renderer.draw_text(path_x, layout.content.y + 1, &path_label, picker_color);
}

fn render_picker_filter_row(renderer: &mut Renderer, picker: &PickerState, layout: &PickerLayout) {
    for item in picker_filter_render_items(picker, layout) {
        renderer.draw_text(item.rect.x, item.rect.y, &item.label, item.color);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PickerFilterRenderItem {
    rect: Rect,
    label: String,
    color: Color,
}

fn picker_filter_render_items(
    picker: &PickerState,
    layout: &PickerLayout,
) -> Vec<PickerFilterRenderItem> {
    let mut items = Vec::with_capacity(layout.group_buttons.len() + 3);
    items.push(picker_managed_filter_render_item(picker, layout));
    items.extend(
        layout
            .group_buttons
            .iter()
            .map(|(name, rect)| picker_group_filter_render_item(picker, name, *rect)),
    );
    items.push(picker_all_filter_render_item(picker, layout));
    items.extend(picker_group_target_filter_render_item(picker, layout));
    items
}

fn picker_managed_filter_render_item(
    picker: &PickerState,
    layout: &PickerLayout,
) -> PickerFilterRenderItem {
    let managed_label = match &picker.overlay_label {
        Some(label) => format!("[{}]", label.to_lowercase()),
        None => "[managed]".to_string(),
    };
    let in_group = picker.current_group.is_some();
    PickerFilterRenderItem {
        rect: layout.env_button,
        label: managed_label,
        color: picker_filter_active_color(picker.managed_only && !in_group),
    }
}

fn picker_group_filter_render_item(
    picker: &PickerState,
    name: &str,
    rect: Rect,
) -> PickerFilterRenderItem {
    let active = picker.current_group.as_deref() == Some(name);
    PickerFilterRenderItem {
        rect,
        label: format!("[{name}]"),
        color: picker_filter_active_color(active),
    }
}

fn picker_all_filter_render_item(
    picker: &PickerState,
    layout: &PickerLayout,
) -> PickerFilterRenderItem {
    let in_group = picker.current_group.is_some();
    PickerFilterRenderItem {
        rect: layout.all_button,
        label: "[all folders]".to_string(),
        color: picker_filter_active_color(!picker.managed_only && !in_group),
    }
}

fn picker_filter_active_color(active: bool) -> Color {
    [Color::DarkGrey, Color::White][usize::from(active)]
}

fn picker_group_target_filter_render_item(
    picker: &PickerState,
    layout: &PickerLayout,
) -> Option<PickerFilterRenderItem> {
    let target = picker.group_edit_target.as_ref()?;
    let rect = layout.group_target_button?;
    Some(PickerFilterRenderItem {
        rect,
        label: format!("[target:{target}]"),
        color: Color::Yellow,
    })
}

fn render_picker_spawn_row(renderer: &mut Renderer, picker: &PickerState, layout: &PickerLayout) {
    let spawn_color = if matches!(picker.selection, PickerSelection::SpawnHere) {
        picker.current_theme_color.unwrap_or(Color::White)
    } else {
        picker.current_theme_color.unwrap_or(Color::Yellow)
    };
    let spawn_line = format!(
        "{} + spawn here",
        if matches!(picker.selection, PickerSelection::SpawnHere) {
            ">"
        } else {
            " "
        }
    );
    renderer.draw_text(
        layout.spawn_here_button.x,
        layout.spawn_here_button.y,
        &truncate_label(&spawn_line, layout.spawn_here_button.width as usize),
        spawn_color,
    );
}

fn render_picker_search_overlay(
    renderer: &mut Renderer,
    picker: &PickerState,
    layout: &PickerLayout,
    picker_accent: Color,
) {
    if !picker.search.is_empty() {
        let label = format!(
            "search: {}_ ({} match{})",
            picker.search,
            layout.visible_entries.len(),
            if layout.visible_entries.len() == 1 {
                ""
            } else {
                "es"
            },
        );
        let path_y = layout.content.y + 1;
        let overlay_x = layout
            .content
            .right()
            .saturating_sub(label.len() as u16)
            .max(layout.content.x);
        let available = layout.content.right().saturating_sub(overlay_x) as usize;
        renderer.draw_text(
            overlay_x,
            path_y,
            &truncate_label(&label, available),
            picker_accent,
        );
    }
}

fn render_picker_entries(renderer: &mut Renderer, picker: &PickerState, layout: &PickerLayout) {
    if layout.visible_entries.is_empty() {
        let empty_label = if picker.search.is_empty() {
            "  empty"
        } else {
            "  no matches"
        };
        renderer.draw_text(
            layout.content.x,
            layout.first_entry_y,
            empty_label,
            Color::DarkGrey,
        );
        return;
    }

    for row in 0..layout.visible_entry_rows {
        render_picker_entry_row(renderer, picker, layout, row);
    }
}

fn render_picker_entry_row(
    renderer: &mut Renderer,
    picker: &PickerState,
    layout: &PickerLayout,
    row: usize,
) {
    let Some(row) = picker_entry_row_render_model(picker, layout, row) else {
        return;
    };

    render_picker_entry_label(renderer, layout, &row);
    render_picker_entry_exclude_badge(renderer, layout, &row);
    render_picker_entry_action_badges(renderer, layout, &row);
}

struct PickerEntryRowRenderModel {
    y: u16,
    line: String,
    text_width: usize,
    color: Color,
    excluded: bool,
    exclude_label: Option<&'static str>,
    actions: Vec<ActionLabel>,
    actions_width: u16,
}

fn picker_entry_row_render_model(
    picker: &PickerState,
    layout: &PickerLayout,
    row: usize,
) -> Option<PickerEntryRowRenderModel> {
    let index = picker_entry_row_index(picker, layout, row)?;
    let entry = picker.entry_at(index)?;
    let actions = picker_entry_actions(entry);
    let actions_width = picker_entry_actions_width(&actions);
    let exclude_label = picker_entry_row_exclude_label(picker, index);
    let reserved = picker_entry_row_reserved_width(actions_width, exclude_label);
    let excluded = picker.batch_entry_is_excluded(index);

    Some(PickerEntryRowRenderModel {
        y: layout.first_entry_y + row as u16,
        line: picker_entry_row_line(picker, index, entry),
        text_width: layout.content.width.saturating_sub(reserved) as usize,
        color: picker_entry_row_color(picker, index, entry, excluded),
        excluded,
        exclude_label,
        actions,
        actions_width,
    })
}

fn picker_entry_row_index(
    picker: &PickerState,
    layout: &PickerLayout,
    row: usize,
) -> Option<usize> {
    layout.visible_entries.get(picker.scroll + row).copied()
}

fn picker_entry_row_line(picker: &PickerState, index: usize, entry: &DirEntry) -> String {
    format!(
        "{} {} {}{}",
        picker_entry_row_marker(picker, index),
        picker_entry_row_icon(entry),
        entry.name,
        picker_entry_row_running_suffix(entry)
    )
}

fn picker_entry_row_marker(picker: &PickerState, index: usize) -> &'static str {
    if picker.selection == PickerSelection::Entry(index) {
        ">"
    } else {
        " "
    }
}

fn picker_entry_row_icon(entry: &DirEntry) -> &'static str {
    if entry.has_children {
        ">"
    } else {
        "+"
    }
}

fn picker_entry_row_running_suffix(entry: &DirEntry) -> &'static str {
    match entry.is_running {
        Some(true) => " *",
        Some(false) => " -",
        None => "",
    }
}

fn picker_entry_row_exclude_label(picker: &PickerState, index: usize) -> Option<&'static str> {
    picker
        .batch_exclude_mode
        .then(|| picker_batch_exclude_label(picker, index))
}

fn picker_entry_row_reserved_width(actions_width: u16, exclude_label: Option<&str>) -> u16 {
    let exclude_width = picker_entry_row_exclude_width(actions_width, exclude_label);
    if actions_width > 0 {
        actions_width + 1 + exclude_width
    } else if exclude_width > 0 {
        exclude_width + 1
    } else {
        0
    }
}

fn picker_entry_row_exclude_width(actions_width: u16, exclude_label: Option<&str>) -> u16 {
    exclude_label
        .map(|label| label.len() as u16 + u16::from(actions_width > 0))
        .unwrap_or(0)
}

fn picker_entry_row_color(
    picker: &PickerState,
    index: usize,
    entry: &DirEntry,
    excluded: bool,
) -> Color {
    let themed_color = picker.entry_theme_colors.get(index).copied().flatten();
    if excluded {
        Color::DarkGrey
    } else if picker.selection == PickerSelection::Entry(index) {
        themed_color.unwrap_or(Color::White)
    } else {
        themed_color.unwrap_or_else(|| picker_entry_row_default_color(entry))
    }
}

fn picker_entry_row_default_color(entry: &DirEntry) -> Color {
    if entry.has_children {
        Color::Cyan
    } else {
        Color::DarkGrey
    }
}

fn render_picker_entry_label(
    renderer: &mut Renderer,
    layout: &PickerLayout,
    row: &PickerEntryRowRenderModel,
) {
    renderer.draw_text(
        layout.content.x,
        row.y,
        &truncate_label(&row.line, row.text_width),
        row.color,
    );
}

fn render_picker_entry_exclude_badge(
    renderer: &mut Renderer,
    layout: &PickerLayout,
    row: &PickerEntryRowRenderModel,
) {
    let Some(label) = row.exclude_label else {
        return;
    };
    let actions_padding = picker_entry_row_actions_padding(row.actions_width);
    let x = layout
        .content
        .right()
        .saturating_sub(actions_padding + label.len() as u16);
    renderer.draw_text(
        x,
        row.y,
        label,
        picker_entry_row_exclude_color(row.excluded),
    );
}

fn picker_entry_row_actions_padding(actions_width: u16) -> u16 {
    if actions_width > 0 {
        actions_width + 1
    } else {
        0
    }
}

fn picker_entry_row_exclude_color(excluded: bool) -> Color {
    if excluded {
        Color::Cyan
    } else {
        Color::Yellow
    }
}

fn render_picker_entry_action_badges(
    renderer: &mut Renderer,
    layout: &PickerLayout,
    row: &PickerEntryRowRenderModel,
) {
    let mut x = layout.content.right().saturating_sub(row.actions_width);
    for action in &row.actions {
        renderer.draw_text(x, row.y, &action.text, action.color);
        x += action.text.len() as u16 + 1;
    }
}

pub(crate) fn initial_request_layout(field: Rect) -> InitialRequestLayout {
    let width = INITIAL_REQUEST_WIDTH.min(field.width);
    let height = INITIAL_REQUEST_HEIGHT.min(field.height);
    let x = field.x + field.width.saturating_sub(width) / 2;
    let y = field.y + field.height.saturating_sub(height) / 2;
    let frame = Rect {
        x,
        y,
        width,
        height,
    };
    let content = frame.inset(1);

    InitialRequestLayout {
        frame,
        content,
        input_y: content.y + 3,
    }
}

fn initial_request_title(group_context: Option<(&str, usize)>) -> &'static str {
    if group_context.is_some() {
        "send to school"
    } else {
        "initial request"
    }
}

fn initial_request_context_line(
    initial_request: &InitialRequestState,
    layout: &InitialRequestLayout,
    group_context: Option<(&str, usize)>,
) -> String {
    let line = if let Some((label, count)) = group_context {
        format!("school: {} ({} sessions)", label, count)
    } else if let Some(dirs) = initial_request.batch_dirs.as_ref() {
        format!("batch: {} included dirs", dirs.len())
    } else {
        format!(
            "cwd: {}",
            shorten_path(
                &initial_request.cwd,
                layout.content.width.saturating_sub(5) as usize,
            )
        )
    };
    truncate_label(&line, layout.content.width as usize)
}

fn initial_request_hint(
    initial_request: &InitialRequestState,
    group_context: Option<(&str, usize)>,
) -> String {
    if group_context.is_some() {
        format!("enter sends to school  {}  esc cancels", toggle_hint())
    } else {
        format!(
            "enter creates hidden swimmer{}  {}  esc cancels",
            if initial_request.batch_dirs.is_some() {
                "s"
            } else {
                ""
            },
            toggle_hint()
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
struct InitialRequestInputRenderModel {
    visible: String,
    color: Color,
    cursor_x: u16,
}

fn initial_request_input_render_model(
    initial_request: &InitialRequestState,
    layout: &InitialRequestLayout,
) -> InitialRequestInputRenderModel {
    let input_x = layout.content.x;
    let available = layout.content.width.saturating_sub(3) as usize;
    let (text, color) = if initial_request.value.is_empty() {
        ("type initial request".to_string(), Color::DarkGrey)
    } else {
        (tail_text(&initial_request.value, available), Color::White)
    };
    let visible = truncate_label(&text, available);
    let cursor_x = if initial_request.value.is_empty() {
        input_x + 2
    } else {
        input_x + 2 + visible.chars().count() as u16
    };

    InitialRequestInputRenderModel {
        visible,
        color,
        cursor_x,
    }
}

fn render_initial_request_input(
    renderer: &mut Renderer,
    initial_request: &InitialRequestState,
    layout: &InitialRequestLayout,
) {
    let input_x = layout.content.x;
    renderer.draw_text(input_x, layout.input_y, "> ", Color::White);

    let input = initial_request_input_render_model(initial_request, layout);
    renderer.draw_text(input_x + 2, layout.input_y, &input.visible, input.color);
    if input.cursor_x < layout.content.right() {
        renderer.draw_char(input.cursor_x, layout.input_y, '|', Color::Yellow);
    }
}

fn initial_request_voice_color(voice_state: &VoiceUiState) -> Color {
    match voice_state {
        VoiceUiState::Transcribing => Color::Yellow,
        VoiceUiState::Recording | VoiceUiState::Failed(_) => Color::Red,
        VoiceUiState::Unsupported => Color::DarkGrey,
        VoiceUiState::Ready => Color::Cyan,
    }
}

fn render_initial_request_voice_status(
    renderer: &mut Renderer,
    voice_state: &VoiceUiState,
    layout: &InitialRequestLayout,
) {
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 4,
        &truncate_label(&voice_state.status_line(), layout.content.width as usize),
        initial_request_voice_color(voice_state),
    );
}

pub(crate) fn render_initial_request(
    renderer: &mut Renderer,
    initial_request: &InitialRequestState,
    voice_state: &VoiceUiState,
    field: Rect,
    group_context: Option<(&str, usize)>,
) {
    let layout = initial_request_layout(field);
    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, Color::White);
    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        initial_request_title(group_context),
        Color::Cyan,
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 1,
        &initial_request_context_line(initial_request, &layout, group_context),
        Color::DarkGrey,
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 2,
        &initial_request_hint(initial_request, group_context),
        Color::DarkGrey,
    );

    render_initial_request_input(renderer, initial_request, &layout);
    render_initial_request_voice_status(renderer, voice_state, &layout);
}

#[cfg(test)]
mod tests {
    mod picker {
        use super::super::*;

        fn rect(x: u16, y: u16, width: u16) -> Rect {
            Rect {
                x,
                y,
                width,
                height: 1,
            }
        }

        fn field_rect() -> Rect {
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 20,
            }
        }

        fn initial_request_test_layout(width: u16) -> InitialRequestLayout {
            InitialRequestLayout {
                frame: Rect {
                    x: 0,
                    y: 0,
                    width: width + 2,
                    height: 7,
                },
                content: Rect {
                    x: 1,
                    y: 1,
                    width,
                    height: 5,
                },
                input_y: 4,
            }
        }

        fn filter_test_layout() -> PickerLayout {
            PickerLayout {
                frame: rect(0, 0, 80),
                content: rect(1, 1, 78),
                back_button: None,
                close_button: rect(76, 1, 3),
                env_button: rect(2, 3, 9),
                group_buttons: vec![
                    ("alpha".to_string(), rect(12, 3, 7)),
                    ("beta".to_string(), rect(20, 3, 6)),
                ],
                all_button: rect(28, 3, 13),
                group_target_button: Some(rect(42, 3, 14)),
                tool_button: rect(60, 1, 7),
                launch_target_button: rect(48, 1, 11),
                exclude_button: rect(36, 1, 9),
                batch_button: rect(26, 1, 9),
                spawn_here_button: rect(2, 4, 76),
                first_entry_y: 5,
                visible_entry_rows: 1,
                visible_entries: vec![0],
            }
        }

        fn filter_test_picker() -> PickerState {
            PickerState::new(
                0,
                0,
                DirListResponse {
                    path: "/tmp".to_string(),
                    entries: Vec::new(),
                    overlay_label: None,
                    groups: vec!["alpha".to_string(), "beta".to_string()],
                    launch_targets: Vec::new(),
                    default_launch_target: None,
                },
                true,
                SpawnTool::Codex,
                None,
            )
        }

        fn top_action_at(layout: &PickerLayout, rect: Rect) -> Option<PickerAction> {
            picker_top_control_action_at(layout, rect.x, rect.y)
        }

        fn pointer_test_entry() -> DirEntry {
            DirEntry {
                name: "swimmers".to_string(),
                has_children: true,
                is_running: None,
                repo_dirty: Some(true),
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: None,
                has_restart: Some(true),
                open_url: Some("http://127.0.0.1:3210".to_string()),
            }
        }

        fn pointer_test_picker(batch_exclude_mode: bool) -> PickerState {
            let mut picker = PickerState::new(
                0,
                0,
                DirListResponse {
                    path: "/tmp".to_string(),
                    entries: vec![pointer_test_entry()],
                    overlay_label: None,
                    groups: Vec::new(),
                    launch_targets: Vec::new(),
                    default_launch_target: None,
                },
                true,
                SpawnTool::Codex,
                None,
            );
            picker.batch_exclude_mode = batch_exclude_mode;
            picker
        }

        fn apply_response_entry(name: &str, full_path: &str) -> DirEntry {
            DirEntry {
                name: name.to_string(),
                has_children: true,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: Some(full_path.to_string()),
                has_restart: None,
                open_url: None,
            }
        }

        fn apply_response_target(id: &str) -> LaunchTargetSummary {
            LaunchTargetSummary {
                id: id.to_string(),
                label: format!("{id} target"),
                kind: "remote".to_string(),
                base_url: None,
                auth_token_env: None,
                path_mappings: Vec::new(),
            }
        }

        fn apply_response_picker(entries: Vec<DirEntry>) -> PickerState {
            PickerState::new(
                0,
                0,
                DirListResponse {
                    path: "/tmp".to_string(),
                    entries,
                    overlay_label: Some("old overlay".to_string()),
                    groups: vec!["alpha".to_string(), "beta".to_string()],
                    launch_targets: vec![apply_response_target("remote-a")],
                    default_launch_target: Some("remote-a".to_string()),
                },
                true,
                SpawnTool::Codex,
                None,
            )
        }

        fn apply_response_dir_list(entries: Vec<DirEntry>) -> DirListResponse {
            DirListResponse {
                path: "/tmp/next".to_string(),
                entries,
                overlay_label: Some("new overlay".to_string()),
                groups: Vec::new(),
                launch_targets: Vec::new(),
                default_launch_target: None,
            }
        }

        #[test]
        fn apply_response_preserves_selection_by_normalized_path_and_retains_batch_exclusions() {
            let mut picker = apply_response_picker(vec![
                apply_response_entry("a", "/tmp/projects/a"),
                apply_response_entry("b", "/tmp/projects/b/"),
            ]);
            picker.selection = PickerSelection::Entry(1);
            picker.scroll = 5;
            picker.launch_target = Some("remote-a".to_string());
            picker.group_edit_target = Some("beta".to_string());
            picker.current_theme_color = Some(Color::Red);
            picker.entry_theme_colors = vec![Some(Color::Blue), Some(Color::Green)];
            picker
                .batch_excluded_paths
                .insert(normalize_path("/tmp/projects/b/"));
            picker
                .batch_excluded_paths
                .insert(normalize_path("/tmp/projects/missing"));

            let mut response = apply_response_dir_list(vec![
                apply_response_entry("renamed-b", "/tmp/projects/b"),
                apply_response_entry("c", "/tmp/projects/c"),
            ]);
            response.groups = vec!["alpha".to_string(), "beta".to_string()];
            response.launch_targets = vec![
                apply_response_target("remote-a"),
                apply_response_target("remote-b"),
            ];
            response.default_launch_target = Some("remote-b".to_string());
            picker.apply_response(response, true);

            assert_eq!(picker.current_path, "/tmp/next");
            assert_eq!(
                picker
                    .entries
                    .iter()
                    .map(|entry| &entry.name)
                    .collect::<Vec<_>>(),
                vec!["renamed-b", "c"]
            );
            assert_eq!(picker.overlay_label.as_deref(), Some("new overlay"));
            assert_eq!(picker.selection, PickerSelection::Entry(0));
            assert_eq!(picker.scroll, 1);
            assert_eq!(picker.launch_target.as_deref(), Some("remote-a"));
            assert_eq!(picker.launch_targets[0].id, "local");
            assert_eq!(picker.group_edit_target.as_deref(), Some("beta"));
            assert_eq!(
                picker.available_groups,
                vec!["alpha".to_string(), "beta".to_string()]
            );
            assert_eq!(picker.current_theme_color, None);
            assert!(picker.entry_theme_colors.is_empty());
            assert_eq!(picker.batch_excluded_paths.len(), 1);
            assert!(picker
                .batch_excluded_paths
                .contains(&normalize_path("/tmp/projects/b")));
        }

        #[test]
        fn apply_response_resets_selection_uses_default_launch_target_and_clears_batch_exclusions()
        {
            let mut picker =
                apply_response_picker(vec![apply_response_entry("a", "/tmp/projects/a")]);
            picker.selection = PickerSelection::Entry(0);
            picker.scroll = 3;
            picker.launch_target = Some("remote-a".to_string());
            picker.group_edit_target = Some("beta".to_string());
            picker
                .batch_excluded_paths
                .insert(normalize_path("/tmp/projects/a"));

            let mut response =
                apply_response_dir_list(vec![apply_response_entry("b", "/tmp/projects/b")]);
            response.groups = vec!["gamma".to_string(), "delta".to_string()];
            response.launch_targets = vec![apply_response_target("remote-b")];
            response.default_launch_target = Some("remote-b".to_string());
            picker.apply_response(response, false);

            assert_eq!(picker.selection, PickerSelection::SpawnHere);
            assert_eq!(picker.scroll, 0);
            assert_eq!(picker.launch_target.as_deref(), Some("remote-b"));
            assert_eq!(
                picker
                    .launch_targets
                    .iter()
                    .map(|target| target.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["local", "remote-b"]
            );
            assert_eq!(picker.group_edit_target.as_deref(), Some("gamma"));
            assert!(picker.batch_excluded_paths.is_empty());
        }

        #[test]
        fn apply_response_clamps_fallback_selection_and_snaps_search() {
            let mut picker = apply_response_picker(vec![
                apply_response_entry("a", "/tmp/old/a"),
                apply_response_entry("b", "/tmp/old/b"),
                apply_response_entry("c", "/tmp/old/c"),
            ]);
            picker.selection = PickerSelection::Entry(2);
            picker.scroll = 9;
            picker.search = "match".to_string();
            picker.group_edit_target = Some("alpha".to_string());

            let response = apply_response_dir_list(vec![
                apply_response_entry("matchable", "/tmp/new/matchable"),
                apply_response_entry("plain", "/tmp/new/plain"),
            ]);
            picker.apply_response(response, true);

            assert_eq!(picker.selection, PickerSelection::Entry(0));
            assert_eq!(picker.scroll, 0);
            assert_eq!(
                picker.available_groups,
                vec!["alpha".to_string(), "beta".to_string()]
            );
            assert_eq!(picker.group_edit_target.as_deref(), Some("alpha"));
        }

        #[test]
        fn apply_response_preserved_entry_falls_back_to_spawn_here_when_response_is_empty() {
            let mut picker =
                apply_response_picker(vec![apply_response_entry("a", "/tmp/projects/a")]);
            picker.selection = PickerSelection::Entry(0);
            picker.scroll = 4;
            picker
                .batch_excluded_paths
                .insert(normalize_path("/tmp/projects/a"));

            picker.apply_response(apply_response_dir_list(Vec::new()), true);

            assert_eq!(picker.selection, PickerSelection::SpawnHere);
            assert_eq!(picker.scroll, 0);
            assert!(picker.batch_excluded_paths.is_empty());
        }

        #[test]
        fn picker_filter_render_plan_preserves_labels_positions_and_target() {
            let layout = filter_test_layout();
            let mut picker = filter_test_picker();
            picker.overlay_label = Some("Overlay".to_string());
            picker.group_edit_target = Some("beta".to_string());

            let items = picker_filter_render_items(&picker, &layout);

            assert_eq!(
                items,
                vec![
                    PickerFilterRenderItem {
                        rect: layout.env_button,
                        label: "[overlay]".to_string(),
                        color: Color::White,
                    },
                    PickerFilterRenderItem {
                        rect: layout.group_buttons[0].1,
                        label: "[alpha]".to_string(),
                        color: Color::DarkGrey,
                    },
                    PickerFilterRenderItem {
                        rect: layout.group_buttons[1].1,
                        label: "[beta]".to_string(),
                        color: Color::DarkGrey,
                    },
                    PickerFilterRenderItem {
                        rect: layout.all_button,
                        label: "[all folders]".to_string(),
                        color: Color::DarkGrey,
                    },
                    PickerFilterRenderItem {
                        rect: layout.group_target_button.expect("target rect"),
                        label: "[target:beta]".to_string(),
                        color: Color::Yellow,
                    },
                ]
            );
        }

        #[test]
        fn picker_filter_render_plan_uses_active_colors_for_each_mode() {
            let layout = filter_test_layout();
            let mut picker = filter_test_picker();

            let managed_items = picker_filter_render_items(&picker, &layout);
            assert_eq!(managed_items[0].color, Color::White);
            assert_eq!(managed_items[3].color, Color::DarkGrey);

            picker.current_group = Some("alpha".to_string());
            let group_items = picker_filter_render_items(&picker, &layout);
            assert_eq!(group_items[0].color, Color::DarkGrey);
            assert_eq!(group_items[1].color, Color::White);
            assert_eq!(group_items[2].color, Color::DarkGrey);
            assert_eq!(group_items[3].color, Color::DarkGrey);

            picker.current_group = None;
            picker.managed_only = false;
            let all_items = picker_filter_render_items(&picker, &layout);
            assert_eq!(all_items[0].color, Color::DarkGrey);
            assert_eq!(all_items[3].color, Color::White);
        }

        #[test]
        fn picker_filter_render_plan_requires_target_state_and_layout() {
            let mut layout = filter_test_layout();
            let mut picker = filter_test_picker();

            picker.group_edit_target = None;
            assert!(picker_filter_render_items(&picker, &layout)
                .iter()
                .all(|item| !item.label.starts_with("[target:")));

            picker.group_edit_target = Some("alpha".to_string());
            layout.group_target_button = None;
            assert!(picker_filter_render_items(&picker, &layout)
                .iter()
                .all(|item| !item.label.starts_with("[target:")));
        }

        #[test]
        fn picker_action_at_maps_all_top_controls_and_rect_edges() {
            let mut layout = filter_test_layout();
            layout.back_button = Some(rect(4, 2, 4));

            for (control, action) in [
                (layout.close_button, PickerAction::Close),
                (layout.tool_button, PickerAction::ToggleTool),
                (
                    layout.launch_target_button,
                    PickerAction::ToggleLaunchTarget,
                ),
                (layout.exclude_button, PickerAction::ToggleBatchExcludeMode),
                (layout.batch_button, PickerAction::BatchVisible),
                (layout.back_button.expect("back rect"), PickerAction::Up),
            ] {
                assert_eq!(top_action_at(&layout, control), Some(action.clone()));
                assert_eq!(
                    picker_top_control_action_at(&layout, control.right() - 1, control.y),
                    Some(action)
                );
            }

            assert_eq!(
                picker_top_control_action_at(
                    &layout,
                    layout.close_button.right(),
                    layout.close_button.y
                ),
                None
            );
            assert_eq!(
                picker_top_control_action_at(
                    &layout,
                    layout.close_button.x,
                    layout.close_button.bottom()
                ),
                None
            );

            let back_button = layout.back_button.expect("back rect");
            layout.back_button = None;
            assert_eq!(
                picker_top_control_action_at(&layout, back_button.x, back_button.y),
                None
            );
        }

        #[test]
        fn picker_action_at_preserves_top_control_precedence() {
            let overlap = rect(10, 1, 4);
            let away = rect(40, 10, 4);
            let mut layout = filter_test_layout();
            layout.close_button = overlap;
            layout.tool_button = overlap;
            layout.launch_target_button = overlap;
            layout.exclude_button = overlap;
            layout.batch_button = overlap;
            layout.back_button = Some(overlap);

            assert_eq!(top_action_at(&layout, overlap), Some(PickerAction::Close));

            layout.close_button = away;
            assert_eq!(
                top_action_at(&layout, overlap),
                Some(PickerAction::ToggleTool)
            );

            layout.tool_button = away;
            assert_eq!(
                top_action_at(&layout, overlap),
                Some(PickerAction::ToggleLaunchTarget)
            );

            layout.launch_target_button = away;
            assert_eq!(
                top_action_at(&layout, overlap),
                Some(PickerAction::ToggleBatchExcludeMode)
            );

            layout.exclude_button = away;
            assert_eq!(
                top_action_at(&layout, overlap),
                Some(PickerAction::BatchVisible)
            );

            layout.batch_button = away;
            assert_eq!(top_action_at(&layout, overlap), Some(PickerAction::Up));

            layout.back_button = None;
            assert_eq!(top_action_at(&layout, overlap), None);
        }

        #[test]
        fn picker_filter_hit_test_maps_controls_to_actions() {
            let layout = filter_test_layout();

            assert_eq!(
                picker_filter_action_at(&layout, 2, 3),
                Some(PickerAction::ToggleManaged(true))
            );
            assert_eq!(
                picker_filter_action_at(&layout, 12, 3),
                Some(PickerAction::ActivateGroup("alpha".to_string()))
            );
            assert_eq!(
                picker_filter_action_at(&layout, 20, 3),
                Some(PickerAction::ActivateGroup("beta".to_string()))
            );
            assert_eq!(
                picker_filter_action_at(&layout, 28, 3),
                Some(PickerAction::ToggleManaged(false))
            );
            assert_eq!(
                picker_filter_action_at(&layout, 42, 3),
                Some(PickerAction::CycleGroupEditTarget)
            );
            assert_eq!(
                picker_filter_action_at(&layout, 2, 4),
                Some(PickerAction::ActivateCurrentPath)
            );
        }

        #[test]
        fn picker_filter_hit_test_preserves_order_for_overlapping_controls() {
            let mut layout = filter_test_layout();
            layout.env_button = rect(10, 3, 8);
            layout.group_buttons = vec![("work".to_string(), rect(10, 3, 8))];
            layout.all_button = rect(10, 3, 8);
            layout.group_target_button = Some(rect(10, 3, 8));
            layout.spawn_here_button = rect(10, 3, 8);

            assert_eq!(
                picker_filter_action_at(&layout, 10, 3),
                Some(PickerAction::ToggleManaged(true))
            );

            layout.env_button = rect(2, 3, 8);
            assert_eq!(
                picker_filter_action_at(&layout, 10, 3),
                Some(PickerAction::ActivateGroup("work".to_string()))
            );

            layout.group_buttons.clear();
            assert_eq!(
                picker_filter_action_at(&layout, 10, 3),
                Some(PickerAction::ToggleManaged(false))
            );

            layout.all_button = rect(20, 3, 8);
            assert_eq!(
                picker_filter_action_at(&layout, 10, 3),
                Some(PickerAction::CycleGroupEditTarget)
            );

            layout.group_target_button = None;
            assert_eq!(
                picker_filter_action_at(&layout, 10, 3),
                Some(PickerAction::ActivateCurrentPath)
            );
        }

        #[test]
        fn picker_filter_hit_test_ignores_invalid_coordinates() {
            let layout = filter_test_layout();

            assert_eq!(picker_filter_action_at(&layout, 0, 0), None);
            assert_eq!(picker_filter_action_at(&layout, 2, 2), None);
            assert_eq!(picker_filter_action_at(&layout, 78, 4), None);
        }

        #[test]
        fn picker_entry_pointer_action_toggles_batch_exclude_target_first() {
            let picker = pointer_test_picker(true);
            let layout = picker_layout(&picker, field_rect());
            let exclude_rect =
                picker_batch_exclude_rect(&picker, &layout, 0, 0).expect("exclude rect");

            assert_eq!(
                picker_entry_pointer_action(&picker, &layout, 0, 0, exclude_rect.x, exclude_rect.y),
                PickerAction::ToggleBatchExclude(0)
            );
        }

        #[test]
        fn picker_entry_pointer_action_starts_repo_action_target() {
            let picker = pointer_test_picker(false);
            let layout = picker_layout(&picker, field_rect());
            let (commit_rect, kind) = picker_entry_action_rects(&picker, &layout, 0, 0)
                .into_iter()
                .next()
                .expect("commit rect");

            assert_eq!(kind, RepoActionKind::Commit);
            assert_eq!(
                picker_entry_pointer_action(&picker, &layout, 0, 0, commit_rect.x, commit_rect.y),
                PickerAction::StartRepoAction(0, RepoActionKind::Commit)
            );
        }

        #[test]
        fn picker_entry_pointer_action_activates_entry_outside_row_targets() {
            let picker = pointer_test_picker(true);
            let layout = picker_layout(&picker, field_rect());

            assert_eq!(
                picker_entry_pointer_action(
                    &picker,
                    &layout,
                    0,
                    0,
                    layout.content.x,
                    layout.first_entry_y
                ),
                PickerAction::ActivateEntry(0)
            );
        }

        #[test]
        fn initial_request_title_switches_for_group_context() {
            assert_eq!(initial_request_title(None), "initial request");
            assert_eq!(
                initial_request_title(Some(("frontend", 3))),
                "send to school"
            );
        }

        #[test]
        fn initial_request_context_line_preserves_context_priority_and_truncation() {
            let narrow_layout = initial_request_test_layout(20);
            let wide_layout = initial_request_test_layout(56);
            let request = InitialRequestState::new("/fixture/projects/swimmers".to_string(), None);
            let batch_request = InitialRequestState::new_batch(
                vec!["/tmp/a".to_string(), "/tmp/b".to_string()],
                None,
            );

            assert_eq!(
                initial_request_context_line(
                    &request,
                    &narrow_layout,
                    Some(("school-of-long-name", 7))
                ),
                "school: school-of-l~"
            );
            assert_eq!(
                initial_request_context_line(&batch_request, &wide_layout, None),
                "batch: 2 included dirs"
            );
            assert_eq!(
                initial_request_context_line(&request, &narrow_layout, None),
                "cwd: .../swimmers"
            );
        }

        #[test]
        fn initial_request_hint_preserves_toggle_text_and_batch_pluralization() {
            let request = InitialRequestState::new("/tmp/swimmers".to_string(), None);
            let batch_request = InitialRequestState::new_batch(
                vec!["/tmp/a".to_string(), "/tmp/b".to_string()],
                None,
            );
            let voice_hint = toggle_hint();

            assert_eq!(
                initial_request_hint(&request, None),
                format!("enter creates hidden swimmer  {voice_hint}  esc cancels")
            );
            assert_eq!(
                initial_request_hint(&batch_request, None),
                format!("enter creates hidden swimmers  {voice_hint}  esc cancels")
            );
            assert_eq!(
                initial_request_hint(&request, Some(("frontend", 3))),
                format!("enter sends to school  {voice_hint}  esc cancels")
            );
        }

        #[test]
        fn initial_request_input_model_preserves_placeholder_tail_and_cursor() {
            let layout = initial_request_test_layout(10);
            let empty_request = InitialRequestState::new("/tmp/swimmers".to_string(), None);
            let mut typed_request = empty_request.clone();
            typed_request.value = "abcdefghijk".to_string();

            assert_eq!(
                initial_request_input_render_model(&empty_request, &layout),
                InitialRequestInputRenderModel {
                    visible: "type i~".to_string(),
                    color: Color::DarkGrey,
                    cursor_x: layout.content.x + 2,
                }
            );
            assert_eq!(
                initial_request_input_render_model(&typed_request, &layout),
                InitialRequestInputRenderModel {
                    visible: "efghijk".to_string(),
                    color: Color::White,
                    cursor_x: layout.content.x + 9,
                }
            );
        }

        #[test]
        fn initial_request_voice_color_preserves_state_colors() {
            assert_eq!(
                initial_request_voice_color(&VoiceUiState::Transcribing),
                Color::Yellow
            );
            assert_eq!(
                initial_request_voice_color(&VoiceUiState::Recording),
                Color::Red
            );
            assert_eq!(
                initial_request_voice_color(&VoiceUiState::Failed("denied".to_string())),
                Color::Red
            );
            assert_eq!(
                initial_request_voice_color(&VoiceUiState::Unsupported),
                Color::DarkGrey
            );
            assert_eq!(
                initial_request_voice_color(&VoiceUiState::Ready),
                Color::Cyan
            );
        }
    }
}

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
