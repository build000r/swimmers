use super::{
    batch_button_label, exclude_button_label, launch_target_button_label,
    picker_batch_exclude_label, picker_entry_action_advance, picker_entry_actions,
    picker_entry_actions_start_x, picker_entry_actions_width, picker_layout, shorten_path,
    tail_text, toggle_hint, tool_button_label, truncate_label, ActionLabel, Color, DirEntry,
    InitialRequestLayout, InitialRequestState, PickerLayout, PickerSelection, PickerState, Rect,
    Renderer, VoiceUiState, INITIAL_REQUEST_HEIGHT, INITIAL_REQUEST_WIDTH,
};

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
pub(super) struct PickerFilterRenderItem {
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) color: Color,
}

pub(super) fn picker_filter_render_items(
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
    let mut x = picker_entry_actions_start_x(&row.actions, layout.content.right());
    for action in &row.actions {
        renderer.draw_text(x, row.y, &action.text, action.color);
        x += picker_entry_action_advance(action);
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

pub(super) fn initial_request_title(group_context: Option<(&str, usize)>) -> &'static str {
    if group_context.is_some() {
        "send to school"
    } else {
        "initial request"
    }
}

pub(super) fn initial_request_context_line(
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

pub(super) fn initial_request_hint(
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
pub(super) struct InitialRequestInputRenderModel {
    pub(super) visible: String,
    pub(super) color: Color,
    pub(super) cursor_x: u16,
}

pub(super) fn initial_request_input_render_model(
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

pub(super) fn initial_request_voice_color(voice_state: &VoiceUiState) -> Color {
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
