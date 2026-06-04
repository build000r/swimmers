use super::*;

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

    fn cycle_picker_group_edit_target_from_action(&mut self) {
        let Some(target) = self
            .picker
            .as_mut()
            .and_then(|picker| picker.cycle_group_edit_target())
        else {
            return;
        };

        self.set_message(format!("directory group target: {target}"));
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
        if let Some(picker) = &mut self.picker {
            self.launch_target = picker.toggle_launch_target();
        }
    }

    fn toggle_picker_batch_exclude_mode(&mut self) {
        if let Some(picker) = &mut self.picker {
            picker.batch_exclude_mode = !picker.batch_exclude_mode;
        }
    }

    fn toggle_picker_batch_exclusion(&mut self, index: usize) {
        if let Some(picker) = &mut self.picker {
            picker.toggle_batch_exclusion(index);
        }
    }
}
