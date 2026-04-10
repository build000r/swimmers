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
    pub(crate) current_theme_color: Option<Color>,
    pub(crate) entry_theme_colors: Vec<Option<Color>>,
    pub(crate) managed_only: bool,
    pub(crate) overlay_label: Option<String>,
    pub(crate) spawn_tool: SpawnTool,
    pub(crate) selection: PickerSelection,
    pub(crate) scroll: usize,
}

impl PickerState {
    pub(crate) fn new(
        anchor_x: u16,
        anchor_y: u16,
        response: DirListResponse,
        managed_only: bool,
        spawn_tool: SpawnTool,
    ) -> Self {
        Self {
            anchor_x,
            anchor_y,
            base_path: response.path.clone(),
            current_path: response.path,
            entries: response.entries,
            current_theme_color: None,
            entry_theme_colors: Vec::new(),
            managed_only,
            overlay_label: response.overlay_label,
            spawn_tool,
            selection: PickerSelection::SpawnHere,
            scroll: 0,
        }
    }

    pub(crate) fn apply_response(&mut self, response: DirListResponse) {
        self.current_path = response.path;
        self.entries = response.entries;
        self.overlay_label = response.overlay_label;
        self.current_theme_color = None;
        self.entry_theme_colors.clear();
        self.selection = PickerSelection::SpawnHere;
        self.scroll = 0;
    }

    pub(crate) fn sync_theme_colors(&mut self, repo_themes: &mut HashMap<String, RepoTheme>) {
        self.current_theme_color = picker_theme_color_for_path(&self.current_path, repo_themes);
        self.entry_theme_colors = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.path_for_entry(index)
                    .and_then(|path| picker_theme_color_for_path(&path, repo_themes))
            })
            .collect();
    }

    pub(crate) fn at_root(&self) -> bool {
        normalize_path(&self.current_path) == normalize_path(&self.base_path)
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
        let entry = self.entries.get(index)?;
        Some(join_path(&self.current_path, &entry.name))
    }

    pub(crate) fn move_selection(&mut self, delta: isize, visible_rows: usize) {
        if self.entries.is_empty() && matches!(self.selection, PickerSelection::SpawnHere) {
            return;
        }

        let total = self.entries.len() as isize + 1;
        let current = match self.selection {
            PickerSelection::SpawnHere => 0,
            PickerSelection::Entry(index) => index as isize + 1,
        };
        let next = (current + delta).clamp(0, total.saturating_sub(1));
        self.selection = if next == 0 {
            PickerSelection::SpawnHere
        } else {
            PickerSelection::Entry((next - 1) as usize)
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

        if index < self.scroll {
            self.scroll = index;
            return;
        }

        let last_visible = self.scroll + visible_rows.saturating_sub(1);
        if index > last_visible {
            self.scroll = index + 1 - visible_rows;
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PickerAction {
    Close,
    Up,
    ToggleManaged(bool),
    ToggleTool,
    ActivateCurrentPath,
    ActivateEntry(usize),
}

#[derive(Clone, Copy)]
pub(crate) struct PickerLayout {
    pub(crate) frame: Rect,
    pub(crate) content: Rect,
    pub(crate) back_button: Option<Rect>,
    pub(crate) close_button: Rect,
    pub(crate) env_button: Rect,
    pub(crate) all_button: Rect,
    pub(crate) tool_button: Rect,
    pub(crate) spawn_here_button: Rect,
    pub(crate) first_entry_y: u16,
    pub(crate) visible_entry_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InitialRequestState {
    pub(crate) cwd: String,
    pub(crate) value: String,
}

impl InitialRequestState {
    pub(crate) fn new(cwd: String) -> Self {
        Self {
            cwd,
            value: String::new(),
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

pub(crate) fn picker_layout(picker: &PickerState, field: Rect) -> PickerLayout {
    let width = PICKER_WIDTH.min(field.width);
    let max_height = PICKER_MAX_HEIGHT.min(field.height);
    let header_rows = 4;
    let entry_capacity = max_height.saturating_sub(2 + header_rows).max(1);
    let list_rows = picker.entries.len().max(1).min(entry_capacity as usize) as u16;
    let height = 2 + header_rows + list_rows;

    let max_x = field.right().saturating_sub(width);
    let max_y = field.bottom().saturating_sub(height);

    let mut x = picker.anchor_x;
    if x + width > field.right() {
        x = picker.anchor_x.saturating_sub(width.saturating_sub(1));
    }
    x = x.max(field.x).min(max_x);

    let mut y = picker.anchor_y;
    if y + height > field.bottom() {
        y = picker.anchor_y.saturating_sub(height.saturating_sub(1));
    }
    y = y.max(field.y).min(max_y);

    let frame = Rect {
        x,
        y,
        width,
        height,
    };
    let content = frame.inset(1);
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
        None => 9, // [managed]
    };
    let env_button = Rect {
        x: content.x,
        y: content.y + 2,
        width: managed_label_width,
        height: 1,
    };
    let all_button = Rect {
        x: (content.x + managed_label_width + 2).min(content.right().saturating_sub(13)),
        y: content.y + 2,
        width: 13,
        height: 1,
    };
    let tool_label_width = tool_button_label(picker.spawn_tool).len() as u16;
    let tool_button = Rect {
        x: close_button.x.saturating_sub(tool_label_width + 1),
        y: content.y,
        width: tool_label_width,
        height: 1,
    };

    PickerLayout {
        frame,
        content,
        back_button,
        close_button,
        env_button,
        all_button,
        tool_button,
        spawn_here_button: Rect {
            x: content.x,
            y: content.y + 3,
            width: content.width,
            height: 1,
        },
        first_entry_y: content.y + 4,
        visible_entry_rows: list_rows as usize,
    }
}

pub(crate) fn picker_action_at(
    picker: &PickerState,
    layout: PickerLayout,
    x: u16,
    y: u16,
) -> Option<PickerAction> {
    if layout.close_button.contains(x, y) {
        return Some(PickerAction::Close);
    }
    if layout.tool_button.contains(x, y) {
        return Some(PickerAction::ToggleTool);
    }
    if layout
        .back_button
        .map(|button| button.contains(x, y))
        .unwrap_or(false)
    {
        return Some(PickerAction::Up);
    }
    if layout.env_button.contains(x, y) {
        return Some(PickerAction::ToggleManaged(true));
    }
    if layout.all_button.contains(x, y) {
        return Some(PickerAction::ToggleManaged(false));
    }
    if layout.spawn_here_button.contains(x, y) {
        return Some(PickerAction::ActivateCurrentPath);
    }
    if y >= layout.first_entry_y
        && y < layout.first_entry_y + layout.visible_entry_rows as u16
        && x >= layout.content.x
        && x < layout.content.right()
    {
        let index = picker.scroll + (y - layout.first_entry_y) as usize;
        if index < picker.entries.len() {
            return Some(PickerAction::ActivateEntry(index));
        }
    }
    None
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
        layout.close_button.x,
        layout.close_button.y,
        "[x]",
        Color::DarkGrey,
    );

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

    let managed_label = match &picker.overlay_label {
        Some(label) => format!("[{}]", label.to_lowercase()),
        None => "[managed]".to_string(),
    };
    renderer.draw_text(
        layout.env_button.x,
        layout.env_button.y,
        &managed_label,
        if picker.managed_only {
            Color::White
        } else {
            Color::DarkGrey
        },
    );
    renderer.draw_text(
        layout.all_button.x,
        layout.all_button.y,
        "[all folders]",
        if picker.managed_only {
            Color::DarkGrey
        } else {
            Color::White
        },
    );

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

    if picker.entries.is_empty() {
        renderer.draw_text(
            layout.content.x,
            layout.first_entry_y,
            "  empty",
            Color::DarkGrey,
        );
        return;
    }

    for row in 0..layout.visible_entry_rows {
        let index = picker.scroll + row;
        if index >= picker.entries.len() {
            break;
        }
        let entry = &picker.entries[index];
        let marker = if picker.selection == PickerSelection::Entry(index) {
            ">"
        } else {
            " "
        };
        let icon = if entry.has_children { ">" } else { "+" };
        let running = match entry.is_running {
            Some(true) => " *",
            Some(false) => " -",
            None => "",
        };
        let line = format!("{marker} {icon} {}{}", entry.name, running);
        let themed_color = picker.entry_theme_colors.get(index).copied().flatten();
        let color = if picker.selection == PickerSelection::Entry(index) {
            themed_color.unwrap_or(Color::White)
        } else if let Some(theme_color) = themed_color {
            theme_color
        } else if entry.has_children {
            Color::Cyan
        } else {
            Color::DarkGrey
        };
        renderer.draw_text(
            layout.content.x,
            layout.first_entry_y + row as u16,
            &truncate_label(&line, layout.content.width as usize),
            color,
        );
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
    field: Rect,
) {
    let layout = initial_request_layout(field);
    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, Color::White);
    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        "initial request",
        Color::Cyan,
    );

    let cwd_line = format!(
        "cwd: {}",
        shorten_path(
            &initial_request.cwd,
            layout.content.width.saturating_sub(5) as usize,
        )
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 1,
        &truncate_label(&cwd_line, layout.content.width as usize),
        Color::DarkGrey,
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 2,
        "enter creates hidden swimmer esc cancels",
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
}
