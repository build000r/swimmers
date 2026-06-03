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
    let mut remove = memberships
        .iter()
        .filter(|group| group.as_str() != target)
        .cloned()
        .collect::<Vec<_>>();
    if let Some(current) = current_group {
        let removes_current = remove.iter().any(|group| group == current);
        if current != target && !removes_current {
            remove.push(current.to_string());
        }
    }
    remove
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
        if self.search.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.search.to_lowercase();
        let mut seen_local_paths = HashSet::new();
        let mut visible = Vec::new();
        for index in 0..self.entries.len() {
            if let Some(path) = self.path_for_entry(index) {
                seen_local_paths.insert(normalize_path(&path));
            }
            if self.entry_matches_search(index, &needle) {
                visible.push(index);
            }
        }

        for index in self.entries.len()..self.total_entry_count() {
            let Some(path) = self.path_for_entry(index) else {
                continue;
            };
            if seen_local_paths.contains(&normalize_path(&path)) {
                continue;
            }
            if self.entry_matches_search(index, &needle) {
                visible.push(index);
            }
        }

        visible
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
        let previous_selection = self.selection;
        let previous_scroll = self.scroll;
        let selected_path = match previous_selection {
            PickerSelection::Entry(index) => {
                self.path_for_entry(index).map(|path| normalize_path(&path))
            }
            PickerSelection::SpawnHere => None,
        };
        self.current_path = response.path;
        self.entries = response.entries;
        self.overlay_label = response.overlay_label;
        let previous_launch_target = self.launch_target.clone();
        self.launch_targets = normalized_launch_targets(response.launch_targets);
        self.launch_target = select_launch_target(
            preserve_selection
                .then_some(previous_launch_target.as_deref())
                .flatten(),
            response.default_launch_target.as_deref(),
            &self.launch_targets,
        );
        if !response.groups.is_empty() {
            self.available_groups = response.groups;
            if !self
                .group_edit_target
                .as_ref()
                .map(|target| self.available_groups.iter().any(|group| group == target))
                .unwrap_or(false)
            {
                self.group_edit_target = self.available_groups.first().cloned();
            }
        }
        self.current_theme_color = None;
        self.entry_theme_colors.clear();
        if preserve_selection {
            let total_entries = self.total_entry_count();
            self.selection = selected_path
                .as_ref()
                .and_then(|path| {
                    (0..total_entries).find(|index| {
                        self.path_for_entry(*index)
                            .map(|candidate| normalize_path(&candidate) == *path)
                            .unwrap_or(false)
                    })
                })
                .map(PickerSelection::Entry)
                .unwrap_or(match previous_selection {
                    PickerSelection::SpawnHere => PickerSelection::SpawnHere,
                    PickerSelection::Entry(_) if total_entries == 0 => PickerSelection::SpawnHere,
                    PickerSelection::Entry(index) => {
                        PickerSelection::Entry(index.min(total_entries.saturating_sub(1)))
                    }
                });
            self.scroll = previous_scroll.min(total_entries.saturating_sub(1));
        } else {
            self.selection = PickerSelection::SpawnHere;
            self.scroll = 0;
        }
        if !self.search.is_empty() {
            self.snap_selection_to_visible();
        }
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
        if self.at_root() {
            return None;
        }

        let normalized = normalize_path(&self.current_path);
        let path = std::path::Path::new(&normalized);
        path.parent().map(|parent| {
            let raw = parent.to_string_lossy();
            if raw.is_empty() {
                "/".to_string()
            } else {
                raw.into_owned()
            }
        })
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
    if layout.close_button.contains(x, y) {
        return Some(PickerAction::Close);
    }
    if layout.tool_button.contains(x, y) {
        return Some(PickerAction::ToggleTool);
    }
    if layout.launch_target_button.contains(x, y) {
        return Some(PickerAction::ToggleLaunchTarget);
    }
    if layout.exclude_button.contains(x, y) {
        return Some(PickerAction::ToggleBatchExcludeMode);
    }
    if layout.batch_button.contains(x, y) {
        return Some(PickerAction::BatchVisible);
    }
    if layout
        .back_button
        .map(|button| button.contains(x, y))
        .unwrap_or(false)
    {
        return Some(PickerAction::Up);
    }
    None
}

fn picker_filter_action_at(layout: &PickerLayout, x: u16, y: u16) -> Option<PickerAction> {
    if layout.env_button.contains(x, y) {
        return Some(PickerAction::ToggleManaged(true));
    }
    for (name, rect) in &layout.group_buttons {
        if rect.contains(x, y) {
            return Some(PickerAction::ActivateGroup(name.clone()));
        }
    }
    if layout.all_button.contains(x, y) {
        return Some(PickerAction::ToggleManaged(false));
    }
    if layout
        .group_target_button
        .map(|button| button.contains(x, y))
        .unwrap_or(false)
    {
        return Some(PickerAction::CycleGroupEditTarget);
    }
    if layout.spawn_here_button.contains(x, y) {
        return Some(PickerAction::ActivateCurrentPath);
    }
    None
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
    if let Some(action) =
        picker_batch_exclude_action_at(picker, layout, visible_pos, raw_index, x, y)
    {
        return action;
    }
    if let Some(kind) = picker_repo_action_kind_at(picker, layout, visible_pos, raw_index, x, y) {
        return PickerAction::StartRepoAction(raw_index, kind);
    }
    PickerAction::ActivateEntry(raw_index)
}

fn picker_batch_exclude_action_at(
    picker: &PickerState,
    layout: &PickerLayout,
    visible_pos: usize,
    raw_index: usize,
    x: u16,
    y: u16,
) -> Option<PickerAction> {
    picker_batch_exclude_contains(picker, layout, visible_pos, raw_index, x, y)
        .then_some(PickerAction::ToggleBatchExclude(raw_index))
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
    let managed_label = match &picker.overlay_label {
        Some(label) => format!("[{}]", label.to_lowercase()),
        None => "[managed]".to_string(),
    };
    let in_group = picker.current_group.is_some();
    renderer.draw_text(
        layout.env_button.x,
        layout.env_button.y,
        &managed_label,
        if picker.managed_only && !in_group {
            Color::White
        } else {
            Color::DarkGrey
        },
    );
    for (name, rect) in &layout.group_buttons {
        let label = format!("[{name}]");
        let active = picker.current_group.as_deref() == Some(name);
        renderer.draw_text(
            rect.x,
            rect.y,
            &label,
            if active {
                Color::White
            } else {
                Color::DarkGrey
            },
        );
    }
    renderer.draw_text(
        layout.all_button.x,
        layout.all_button.y,
        "[all folders]",
        if !picker.managed_only && !in_group {
            Color::White
        } else {
            Color::DarkGrey
        },
    );
    if let (Some(target), Some(rect)) = (&picker.group_edit_target, layout.group_target_button) {
        let label = format!("[target:{target}]");
        renderer.draw_text(rect.x, rect.y, &label, Color::Yellow);
    }
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
        if group_context.is_some() {
            "send to school"
        } else {
            "initial request"
        },
        Color::Cyan,
    );

    let cwd_line = if let Some((label, count)) = group_context {
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
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 1,
        &truncate_label(&cwd_line, layout.content.width as usize),
        Color::DarkGrey,
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 2,
        &if group_context.is_some() {
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
        },
        Color::DarkGrey,
    );

    let input_x = layout.content.x;
    renderer.draw_text(input_x, layout.input_y, "> ", Color::White);
    let available = layout.content.width.saturating_sub(3) as usize;
    let (text, color) = if initial_request.value.is_empty() {
        ("type initial request".to_string(), Color::DarkGrey)
    } else {
        (tail_text(&initial_request.value, available), Color::White)
    };
    let visible = truncate_label(&text, available);
    renderer.draw_text(input_x + 2, layout.input_y, &visible, color);
    let cursor_x = if initial_request.value.is_empty() {
        input_x + 2
    } else {
        input_x + 2 + visible.chars().count() as u16
    };
    if cursor_x < layout.content.right() {
        renderer.draw_char(cursor_x, layout.input_y, '|', Color::Yellow);
    }
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 4,
        &truncate_label(&voice_state.status_line(), layout.content.width as usize),
        match voice_state {
            VoiceUiState::Transcribing => Color::Yellow,
            VoiceUiState::Recording | VoiceUiState::Failed(_) => Color::Red,
            VoiceUiState::Unsupported => Color::DarkGrey,
            VoiceUiState::Ready => Color::Cyan,
        },
    );
}
