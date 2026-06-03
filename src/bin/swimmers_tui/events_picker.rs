use super::*;

pub(super) fn handle_picker_priority_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    let command = picker_priority_key_command(app.picker.as_ref()?, key.code)?;
    apply_picker_priority_key(app, layout.overview_field, command);
    Some(true)
}

pub(super) fn handle_picker_search_key<C: TuiApi>(app: &mut App<C>, key: KeyEvent) -> Option<bool> {
    if !app.picker.is_some() || !picker_search_accepts_modifiers(key.modifiers) {
        return None;
    }

    let KeyCode::Char(ch) = key.code else {
        return None;
    };
    if ch.is_control() {
        return None;
    }

    app.picker_search_push(ch);
    Some(true)
}

pub(super) fn handle_selection_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    let command = selection_key_command(key.code)?;
    apply_selection_key(app, layout.overview_field, command);
    Some(true)
}

pub(super) fn handle_picker_command_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    key: KeyEvent,
) -> Option<bool> {
    let command = picker_command_key(app.picker.is_some(), key.code)?;
    apply_picker_command_key(app, layout, command);
    Some(true)
}

#[derive(Clone, Copy)]
enum PickerPriorityKeyCommand {
    BatchVisible,
    ToggleBatchExcludeMode,
    CycleGroupEditTarget,
    AddSelectedToGroupTarget,
    RemoveSelectedFromGroupTarget,
    MoveSelectedToGroupTarget,
    ToggleBatchExclude(Option<usize>),
}

#[derive(Clone, Copy)]
enum SelectionKeyCommand {
    Backspace,
    PreviousContainer,
    MovePrevious,
    MoveNext,
    Activate,
}

#[derive(Clone, Copy)]
enum PickerCommandKey {
    ManagedOnly(bool),
    Commit,
    Restart,
    OpenUrl,
    ReloadOrRefresh,
}

const PRIORITY_CHAR_COMMANDS: &[(char, PickerPriorityKeyCommand)] = &[
    ('B', PickerPriorityKeyCommand::BatchVisible),
    ('X', PickerPriorityKeyCommand::ToggleBatchExcludeMode),
    ('G', PickerPriorityKeyCommand::CycleGroupEditTarget),
    ('+', PickerPriorityKeyCommand::AddSelectedToGroupTarget),
    ('=', PickerPriorityKeyCommand::AddSelectedToGroupTarget),
    ('-', PickerPriorityKeyCommand::RemoveSelectedFromGroupTarget),
    ('M', PickerPriorityKeyCommand::MoveSelectedToGroupTarget),
];

const SELECTION_CHAR_COMMANDS: &[(char, SelectionKeyCommand)] = &[
    ('h', SelectionKeyCommand::PreviousContainer),
    ('k', SelectionKeyCommand::MovePrevious),
    ('j', SelectionKeyCommand::MoveNext),
    ('l', SelectionKeyCommand::Activate),
    ('o', SelectionKeyCommand::Activate),
];

const ALWAYS_PICKER_COMMANDS: &[(char, PickerCommandKey)] = &[
    ('e', PickerCommandKey::ManagedOnly(true)),
    ('a', PickerCommandKey::ManagedOnly(false)),
    ('r', PickerCommandKey::ReloadOrRefresh),
];

const PICKER_ONLY_COMMANDS: &[(char, PickerCommandKey)] = &[
    ('c', PickerCommandKey::Commit),
    ('R', PickerCommandKey::Restart),
    ('O', PickerCommandKey::OpenUrl),
];

fn picker_priority_key_command(
    picker: &PickerState,
    code: KeyCode,
) -> Option<PickerPriorityKeyCommand> {
    match code {
        KeyCode::Char(' ') if picker.batch_exclude_mode => Some(
            PickerPriorityKeyCommand::ToggleBatchExclude(selected_entry_index(picker)),
        ),
        KeyCode::Char(ch) => char_command(PRIORITY_CHAR_COMMANDS, ch),
        _ => None,
    }
}

fn apply_picker_priority_key<C: TuiApi>(
    app: &mut App<C>,
    field: Rect,
    command: PickerPriorityKeyCommand,
) {
    match command {
        PickerPriorityKeyCommand::BatchVisible => {
            app.open_batch_initial_request_for_visible_entries();
        }
        PickerPriorityKeyCommand::ToggleBatchExcludeMode => {
            app.handle_picker_action(PickerAction::ToggleBatchExcludeMode, field);
        }
        PickerPriorityKeyCommand::CycleGroupEditTarget => {
            app.picker_cycle_group_edit_target();
        }
        PickerPriorityKeyCommand::AddSelectedToGroupTarget => {
            app.picker_add_selected_to_group_target();
        }
        PickerPriorityKeyCommand::RemoveSelectedFromGroupTarget => {
            app.picker_remove_selected_from_group_target();
        }
        PickerPriorityKeyCommand::MoveSelectedToGroupTarget => {
            app.picker_move_selected_to_group_target();
        }
        PickerPriorityKeyCommand::ToggleBatchExclude(Some(index)) => {
            app.handle_picker_action(PickerAction::ToggleBatchExclude(index), field);
        }
        PickerPriorityKeyCommand::ToggleBatchExclude(None) => {}
    }
}

fn picker_search_accepts_modifiers(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

fn selection_key_command(code: KeyCode) -> Option<SelectionKeyCommand> {
    match code {
        KeyCode::Backspace => Some(SelectionKeyCommand::Backspace),
        KeyCode::Left => Some(SelectionKeyCommand::PreviousContainer),
        KeyCode::Up => Some(SelectionKeyCommand::MovePrevious),
        KeyCode::Down => Some(SelectionKeyCommand::MoveNext),
        KeyCode::Right | KeyCode::Enter => Some(SelectionKeyCommand::Activate),
        KeyCode::Char(ch) => char_command(SELECTION_CHAR_COMMANDS, ch),
        _ => None,
    }
}

fn apply_selection_key<C: TuiApi>(app: &mut App<C>, field: Rect, command: SelectionKeyCommand) {
    match command {
        SelectionKeyCommand::Backspace => {
            if !app.picker_search_pop() {
                move_to_previous_container(app, field);
            }
        }
        SelectionKeyCommand::PreviousContainer => move_to_previous_container(app, field),
        SelectionKeyCommand::MovePrevious => app.move_selection(-1, field),
        SelectionKeyCommand::MoveNext => app.move_selection(1, field),
        SelectionKeyCommand::Activate => activate_current_selection(app, field),
    }
}

fn move_to_previous_container<C: TuiApi>(app: &mut App<C>, field: Rect) {
    if app.picker.is_some() {
        app.picker_up();
    } else {
        app.move_selection(-1, field);
    }
}

fn activate_current_selection<C: TuiApi>(app: &mut App<C>, field: Rect) {
    if app.picker.is_some() {
        app.picker_activate_selection(field);
    } else {
        app.open_selected();
    }
}

fn picker_command_key(has_picker: bool, code: KeyCode) -> Option<PickerCommandKey> {
    let KeyCode::Char(ch) = code else {
        return None;
    };

    char_command(ALWAYS_PICKER_COMMANDS, ch).or_else(|| {
        has_picker
            .then(|| char_command(PICKER_ONLY_COMMANDS, ch))
            .flatten()
    })
}

fn apply_picker_command_key<C: TuiApi>(
    app: &mut App<C>,
    layout: WorkspaceLayout,
    command: PickerCommandKey,
) {
    match command {
        PickerCommandKey::ManagedOnly(managed_only) => {
            app.picker_set_managed_only(managed_only);
        }
        PickerCommandKey::Commit => {
            app.picker_start_action_for_selection(RepoActionKind::Commit);
        }
        PickerCommandKey::Restart => {
            app.picker_start_action_for_selection(RepoActionKind::Restart);
        }
        PickerCommandKey::OpenUrl => {
            app.picker_open_url_for_selection();
        }
        PickerCommandKey::ReloadOrRefresh => reload_picker_or_refresh_workspace(app, layout),
    }
}

fn reload_picker_or_refresh_workspace<C: TuiApi>(app: &mut App<C>, layout: WorkspaceLayout) {
    if let Some((path, managed_only, group)) = app.picker.as_ref().map(|picker| {
        (
            picker.current_path.clone(),
            picker.managed_only,
            picker.current_group.clone(),
        )
    }) {
        app.picker_reload(Some(path), managed_only, group);
    } else {
        app.manual_refresh(layout);
    }
}

fn selected_entry_index(picker: &PickerState) -> Option<usize> {
    match picker.selection {
        PickerSelection::Entry(index) => Some(index),
        PickerSelection::SpawnHere => None,
    }
}

fn char_command<T: Copy>(commands: &[(char, T)], ch: char) -> Option<T> {
    commands
        .iter()
        .find_map(|(candidate, command)| (*candidate == ch).then_some(*command))
}
