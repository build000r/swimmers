use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum PickerGroupEditTargetOutcome {
    Target(String),
    NoGroups,
}

impl PickerGroupEditTargetOutcome {
    fn message(self) -> String {
        match self {
            Self::Target(target) => format!("directory group target: {target}"),
            Self::NoGroups => "no directory groups available".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PickerOpenUrlPlan {
    Open(String),
    MissingUrl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerSelectedOpenUrlPlan {
    OpenIndex(usize),
    NoSelection,
}

impl<C: TuiApi> App<C> {
    pub(crate) fn handle_picker_action(&mut self, action: PickerAction, field: Rect) {
        match action {
            PickerAction::Close => self.close_picker(),
            PickerAction::Up => self.picker_up(),
            PickerAction::ToggleManaged(managed_only) => {
                self.picker_set_managed_only(managed_only);
            }
            PickerAction::ActivateGroup(name) => {
                self.picker_set_group(name);
            }
            PickerAction::CycleGroupEditTarget => self.cycle_picker_group_edit_target_from_action(),
            PickerAction::ToggleTool => self.toggle_picker_tool_from_action(),
            PickerAction::ToggleLaunchTarget => self.toggle_picker_launch_target_from_action(),
            PickerAction::ToggleBatchExcludeMode => self.toggle_picker_batch_exclude_mode(),
            PickerAction::BatchVisible => self.open_batch_initial_request_for_visible_entries(),
            PickerAction::ActivateCurrentPath => self.spawn_session_from_picker(field),
            PickerAction::ActivateEntry(index) => self.activate_picker_entry(index, field),
            PickerAction::ToggleBatchExclude(index) => self.toggle_picker_batch_exclusion(index),
            PickerAction::StartRepoAction(index, kind) => {
                self.start_picker_repo_action(index, kind)
            }
        }
    }

    pub(crate) fn picker_search_pop(&mut self) -> bool {
        pop_picker_search(self.picker.as_mut())
    }

    pub(crate) fn picker_up(&mut self) {
        if let Some(plan) = picker_up_reload_plan(self.picker.as_ref()) {
            self.picker_reload_from_plan(plan);
        }
    }

    pub(crate) fn picker_set_group(&mut self, name: String) {
        if let Some(plan) = picker_set_group_reload_plan(self.picker.as_ref(), name) {
            self.picker_reload_from_plan(plan);
        }
    }

    pub(crate) fn picker_cycle_group_edit_target(&mut self) {
        if let Some(outcome) = cycle_picker_group_edit_target(self.picker.as_mut()) {
            self.set_message(outcome.message());
        }
    }

    pub(super) fn open_picker_url_at(&mut self, index: usize) {
        match picker_open_url_at_plan(self.picker.as_ref(), index) {
            PickerOpenUrlPlan::Open(url) => self.open_picker_url(url),
            PickerOpenUrlPlan::MissingUrl => self.set_message("no open URL for this entry"),
        }
    }

    pub(crate) fn picker_open_url_for_selection(&mut self) {
        match picker_selected_open_url_plan(self.picker.as_ref()) {
            PickerSelectedOpenUrlPlan::OpenIndex(index) => self.open_picker_url_at(index),
            PickerSelectedOpenUrlPlan::NoSelection => self.set_message("select an entry first"),
        }
    }

    fn cycle_picker_group_edit_target_from_action(&mut self) {
        if let Some(outcome @ PickerGroupEditTargetOutcome::Target(_)) =
            cycle_picker_group_edit_target(self.picker.as_mut())
        {
            self.set_message(outcome.message());
        }
    }

    fn toggle_picker_tool_from_action(&mut self) {
        self.spawn_tool = self.spawn_tool.toggle();
        self.sync_picker_spawn_tool();
    }

    fn sync_picker_spawn_tool(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.spawn_tool = self.spawn_tool;
        }
    }

    fn toggle_picker_launch_target_from_action(&mut self) {
        let previous_inventory_target = selected_remote_inventory_target(
            self.picker.as_ref(),
            self.launch_target.as_deref(),
            &self.launch_targets,
            &self.environments,
        );
        let Some((path, managed_only, group, launch_target)) = self.picker.as_mut().map(|picker| {
            let launch_target = picker.toggle_launch_target();
            (
                picker.current_path.clone(),
                picker.managed_only,
                picker.current_group.clone(),
                launch_target,
            )
        }) else {
            return;
        };
        self.launch_target = launch_target;

        let next_inventory_target = selected_remote_inventory_target(
            self.picker.as_ref(),
            self.launch_target.as_deref(),
            &self.launch_targets,
            &self.environments,
        );
        if previous_inventory_target != next_inventory_target {
            self.picker_reload_with_options(Some(path), managed_only, group, true, true);
        }
    }

    fn toggle_picker_batch_exclude_mode(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.batch_exclude_mode = !picker.batch_exclude_mode;
            if !picker.batch_exclude_mode {
                // Leaving exclude mode clears the staged exclusions: their
                // badge/UI disappears with the mode, but batch_dirs_for_visible_
                // entries filters on them unconditionally, so a later batch
                // launch (B, reachable while the mode is off) would otherwise
                // silently drop them (swimmers-oswr). Exclusions therefore do
                // not persist across re-entry into the mode.
                picker.batch_excluded_paths.clear();
            }
        }
    }

    fn toggle_picker_batch_exclusion(&mut self, index: usize) {
        if let Some(picker) = &mut self.picker {
            picker.toggle_batch_exclusion(index);
        }
    }

    fn picker_reload_from_plan(&mut self, plan: PickerReloadPlan) {
        self.picker_reload(plan.path, plan.managed_only, plan.group);
    }

    fn open_picker_url(&mut self, url: String) {
        let message = match open::that(&url) {
            Ok(_) => picker_open_url_success_message(&url),
            Err(err) => picker_open_url_failure_message(&url, err),
        };
        self.set_message(message);
    }
}

fn pop_picker_search(picker: Option<&mut PickerState>) -> bool {
    let Some(picker) = picker else {
        return false;
    };
    if picker.search.is_empty() {
        return false;
    }
    picker.search.pop();
    picker.snap_selection_to_visible();
    true
}

fn picker_up_reload_plan(picker: Option<&PickerState>) -> Option<PickerReloadPlan> {
    let picker = picker?;
    let managed_only = picker.managed_only;
    if picker.current_group.is_some() {
        return Some(PickerReloadPlan {
            path: None,
            managed_only,
            group: None,
        });
    }

    Some(PickerReloadPlan {
        path: Some(picker.parent_path()?),
        managed_only,
        group: None,
    })
}

fn picker_set_group_reload_plan(
    picker: Option<&PickerState>,
    name: String,
) -> Option<PickerReloadPlan> {
    let picker = picker?;
    if picker.current_group.as_ref() == Some(&name) {
        return None;
    }
    Some(PickerReloadPlan {
        path: None,
        managed_only: picker.managed_only,
        group: Some(name),
    })
}

fn cycle_picker_group_edit_target(
    picker: Option<&mut PickerState>,
) -> Option<PickerGroupEditTargetOutcome> {
    let picker = picker?;
    Some(match picker.cycle_group_edit_target() {
        Some(target) => PickerGroupEditTargetOutcome::Target(target),
        None => PickerGroupEditTargetOutcome::NoGroups,
    })
}

fn picker_open_url_at_plan(picker: Option<&PickerState>, index: usize) -> PickerOpenUrlPlan {
    picker
        .and_then(|picker| picker.entry_at(index))
        .and_then(|entry| entry.open_url.clone())
        .map(PickerOpenUrlPlan::Open)
        .unwrap_or(PickerOpenUrlPlan::MissingUrl)
}

fn picker_selected_open_url_plan(picker: Option<&PickerState>) -> PickerSelectedOpenUrlPlan {
    let Some(PickerSelection::Entry(index)) = picker.map(|picker| picker.selection) else {
        return PickerSelectedOpenUrlPlan::NoSelection;
    };
    PickerSelectedOpenUrlPlan::OpenIndex(index)
}

fn picker_open_url_success_message(url: &str) -> String {
    format!("opened {url}")
}

fn picker_open_url_failure_message(url: &str, err: impl std::fmt::Display) -> String {
    format!("failed to open {url}: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/Users/tester/repos";
    const OPEN_URL: &str = "http://localhost:3210";

    #[test]
    fn picker_search_pop_handles_no_picker_empty_search_and_pop() {
        assert!(!pop_picker_search(None));

        let mut picker = test_picker(vec![test_entry("swimmers", None)], Vec::new());
        assert!(!pop_picker_search(Some(&mut picker)));

        picker.search = "sw".to_string();
        assert!(pop_picker_search(Some(&mut picker)));
        assert_eq!(picker.search, "s");
    }

    #[test]
    fn picker_up_plan_handles_no_picker_group_root_and_parent_path() {
        assert!(picker_up_reload_plan(None).is_none());

        let mut group_picker = test_picker(vec![test_entry("swimmers", None)], Vec::new());
        group_picker.managed_only = false;
        group_picker.current_group = Some("work".to_string());
        let group_plan = picker_up_reload_plan(Some(&group_picker)).expect("group plan");
        assert_eq!(group_plan.path, None);
        assert!(!group_plan.managed_only);
        assert_eq!(group_plan.group, None);

        let mut child_picker = test_picker(vec![test_entry("swimmers", None)], Vec::new());
        child_picker.current_path = format!("{ROOT}/opensource");
        let parent_plan = picker_up_reload_plan(Some(&child_picker)).expect("parent plan");
        assert_eq!(parent_plan.path.as_deref(), Some(ROOT));
        assert!(parent_plan.managed_only);
        assert_eq!(parent_plan.group, None);
    }

    #[test]
    fn picker_set_group_plan_noops_for_active_group_and_reloads_for_change() {
        let mut picker = test_picker(vec![test_entry("swimmers", None)], Vec::new());
        picker.current_group = Some("frontend".to_string());

        assert!(picker_set_group_reload_plan(Some(&picker), "frontend".to_string()).is_none());

        let plan =
            picker_set_group_reload_plan(Some(&picker), "backend".to_string()).expect("plan");
        assert_eq!(plan.path, None);
        assert!(plan.managed_only);
        assert_eq!(plan.group.as_deref(), Some("backend"));
    }

    #[test]
    fn picker_group_cycle_outcome_formats_target_and_empty_group_messages() {
        let mut picker = test_picker(
            vec![test_entry("swimmers", None)],
            vec!["frontend".to_string(), "backend".to_string()],
        );
        let outcome = cycle_picker_group_edit_target(Some(&mut picker)).expect("outcome");
        assert_eq!(outcome.message(), "directory group target: backend");
        assert_eq!(picker.group_edit_target.as_deref(), Some("backend"));

        let mut empty_group_picker = test_picker(vec![test_entry("swimmers", None)], Vec::new());
        let outcome =
            cycle_picker_group_edit_target(Some(&mut empty_group_picker)).expect("outcome");
        assert_eq!(outcome.message(), "no directory groups available");
        assert_eq!(empty_group_picker.group_edit_target, None);
    }

    #[test]
    fn picker_open_url_plans_cover_missing_url_and_selection() {
        assert_eq!(
            picker_open_url_at_plan(None, 0),
            PickerOpenUrlPlan::MissingUrl
        );

        let mut picker = test_picker(vec![test_entry("swimmers", Some(OPEN_URL))], Vec::new());
        assert_eq!(
            picker_open_url_at_plan(Some(&picker), 0),
            PickerOpenUrlPlan::Open(OPEN_URL.to_string())
        );
        assert_eq!(
            picker_open_url_at_plan(Some(&picker), 1),
            PickerOpenUrlPlan::MissingUrl
        );
        assert_eq!(
            picker_selected_open_url_plan(Some(&picker)),
            PickerSelectedOpenUrlPlan::NoSelection
        );

        picker.selection = PickerSelection::Entry(0);
        assert_eq!(
            picker_selected_open_url_plan(Some(&picker)),
            PickerSelectedOpenUrlPlan::OpenIndex(0)
        );
    }

    #[test]
    fn picker_open_url_messages_preserve_success_and_error_text() {
        assert_eq!(
            picker_open_url_success_message(OPEN_URL),
            format!("opened {OPEN_URL}")
        );
        assert_eq!(
            picker_open_url_failure_message(OPEN_URL, "browser unavailable"),
            format!("failed to open {OPEN_URL}: browser unavailable")
        );
    }

    fn test_picker(entries: Vec<DirEntry>, groups: Vec<String>) -> PickerState {
        PickerState::new(
            0,
            0,
            DirListResponse {
                path: ROOT.to_string(),
                entries,
                inventory_source: DirInventorySource::local(),
                overlay_label: None,
                groups,
                launch_targets: Vec::new(),
                default_launch_target: None,
            },
            true,
            SpawnTool::Codex,
            None,
        )
    }

    fn test_entry(name: &str, open_url: Option<&str>) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            has_children: false,
            is_running: None,
            repo_dirty: None,
            repo_action: None,
            group: None,
            groups: Vec::new(),
            full_path: None,
            has_restart: None,
            open_url: open_url.map(str::to_string),
        }
    }
}
