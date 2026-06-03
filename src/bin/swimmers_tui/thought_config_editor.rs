use super::*;
use swimmers::openrouter_models::cached_or_default_openrouter_candidates;
use swimmers::thought_ui::{
    normalize_thought_model_for_backend, thought_backend_cycle_options, thought_backend_label,
    thought_model_presets, thought_model_presets_hint,
};

const THOUGHT_CONFIG_WIDTH: u16 = 58;
const THOUGHT_CONFIG_HEIGHT: u16 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThoughtConfigEditorField {
    Enabled,
    Backend,
    Model,
    Test,
    Save,
    Cancel,
}

#[derive(Clone, Debug)]
pub(crate) struct ThoughtConfigEditorState {
    pub(crate) config: ThoughtConfig,
    pub(crate) daemon_defaults: Option<DaemonDefaults>,
    pub(crate) focus: ThoughtConfigEditorField,
    pub(crate) openrouter_model_presets: Vec<String>,
}

impl ThoughtConfigEditorState {
    pub(crate) fn new(mut config: ThoughtConfig, daemon_defaults: Option<DaemonDefaults>) -> Self {
        config.normalize();
        Self {
            config,
            daemon_defaults,
            focus: ThoughtConfigEditorField::Backend,
            openrouter_model_presets: cached_or_default_openrouter_candidates(),
        }
    }

    pub(crate) fn move_focus(&mut self, delta: isize) {
        const FIELDS: [ThoughtConfigEditorField; 6] = [
            ThoughtConfigEditorField::Enabled,
            ThoughtConfigEditorField::Backend,
            ThoughtConfigEditorField::Model,
            ThoughtConfigEditorField::Test,
            ThoughtConfigEditorField::Save,
            ThoughtConfigEditorField::Cancel,
        ];
        let current = FIELDS
            .iter()
            .position(|field| *field == self.focus)
            .unwrap_or(0) as isize;
        let len = FIELDS.len() as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.focus = FIELDS[next];
    }

    pub(crate) fn cycle_backend(&mut self, delta: isize) {
        let options = thought_backend_cycle_options();
        let current = options
            .iter()
            .position(|option| option.eq_ignore_ascii_case(self.config.backend.trim()))
            .unwrap_or(0) as isize;
        let len = options.len() as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.config.backend = options[next].to_string();
        self.normalize_model_for_backend();
    }

    pub(crate) fn backend_label(&self) -> &'static str {
        thought_backend_label(&self.config.backend)
    }

    pub(crate) fn daemon_label(&self) -> String {
        match &self.daemon_defaults {
            Some(defaults) => {
                let backend = thought_backend_label(&defaults.backend);
                let model = if defaults.model.trim().is_empty() {
                    "(empty)"
                } else {
                    defaults.model.as_str()
                };
                format!("daemon default: {backend} / {model}")
            }
            None => "daemon default: unavailable".to_string(),
        }
    }

    pub(crate) fn cycle_model_preset(&mut self, delta: isize) -> bool {
        let options = self.model_preset_values();
        if options.is_empty() {
            return false;
        }
        let current = options
            .iter()
            .position(|option| option.eq_ignore_ascii_case(self.config.model.trim()))
            .unwrap_or(0) as isize;
        let len = options.len() as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.config.model = options[next].clone();
        true
    }

    pub(crate) fn model_presets_hint(&self) -> Option<&'static str> {
        Some(thought_model_presets_hint(&self.config.backend))
    }

    fn model_preset_values(&self) -> Vec<String> {
        thought_model_presets(&self.config.backend, &self.openrouter_model_presets)
    }

    pub(crate) fn replace_openrouter_model_presets(&mut self, models: Vec<String>) {
        self.openrouter_model_presets = if models.is_empty() {
            cached_or_default_openrouter_candidates()
        } else {
            models
        };
    }

    fn normalize_model_for_backend(&mut self) {
        self.config.model =
            normalize_thought_model_for_backend(&self.config.backend, &self.config.model);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThoughtConfigEditorLayout {
    pub(crate) frame: Rect,
    pub(crate) content: Rect,
    pub(crate) enabled_y: u16,
    pub(crate) backend_y: u16,
    pub(crate) model_y: u16,
    pub(crate) buttons_y: u16,
}

pub(crate) fn thought_config_editor_layout(field: Rect) -> ThoughtConfigEditorLayout {
    let width = THOUGHT_CONFIG_WIDTH
        .min(field.width.saturating_sub(2))
        .max(32);
    let height = THOUGHT_CONFIG_HEIGHT
        .min(field.height.saturating_sub(2))
        .max(9);
    let x = field.x + field.width.saturating_sub(width) / 2;
    let y = field.y + field.height.saturating_sub(height) / 2;
    let frame = Rect {
        x,
        y,
        width,
        height,
    };
    let content = Rect {
        x: frame.x + 2,
        y: frame.y + 1,
        width: frame.width.saturating_sub(4),
        height: frame.height.saturating_sub(2),
    };
    ThoughtConfigEditorLayout {
        frame,
        content,
        enabled_y: content.y + 2,
        backend_y: content.y + 3,
        model_y: content.y + 4,
        buttons_y: content.bottom().saturating_sub(1),
    }
}

pub(crate) fn render_thought_config_editor(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    field: Rect,
) {
    let layout = thought_config_editor_layout(field);
    render_editor_frame(renderer, editor, &layout);
    render_config_fields(renderer, editor, &layout);
    render_status_help(renderer, editor, &layout);
    render_action_buttons(renderer, editor, &layout);
}

fn render_editor_frame(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
) {
    renderer.fill_rect(layout.frame, ' ', Color::Reset);
    renderer.draw_box(layout.frame, Color::White);
    renderer.draw_text(
        layout.content.x,
        layout.content.y,
        "thought config",
        Color::Cyan,
    );
    renderer.draw_text(
        layout.content.x,
        layout.content.y + 1,
        &truncate_label(&editor.daemon_label(), layout.content.width as usize),
        Color::DarkGrey,
    );
}

fn render_config_fields(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
) {
    let enabled_label = if editor.config.enabled { "on" } else { "off" };
    render_field_row(
        renderer,
        layout,
        layout.enabled_y,
        ThoughtConfigEditorField::Enabled,
        &format!("enabled: {enabled_label}"),
        editor.focus,
    );
    render_field_row(
        renderer,
        layout,
        layout.backend_y,
        ThoughtConfigEditorField::Backend,
        &format!("backend: {}", editor.backend_label()),
        editor.focus,
    );
    render_model_row(renderer, editor, layout);
}

fn render_field_row(
    renderer: &mut Renderer,
    layout: &ThoughtConfigEditorLayout,
    y: u16,
    field: ThoughtConfigEditorField,
    text: &str,
    focus: ThoughtConfigEditorField,
) {
    let (prefix, color) = selected_row_style(focus == field);
    renderer.draw_text(layout.content.x, y, &format!("{prefix} {text}"), color);
}

fn selected_row_style(is_selected: bool) -> (&'static str, Color) {
    if is_selected {
        (">", Color::White)
    } else {
        (" ", Color::DarkGrey)
    }
}

fn render_model_row(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
) {
    let input_x = layout.content.x;
    let is_selected = editor.focus == ThoughtConfigEditorField::Model;
    let (model_prefix, label_color) = selected_row_style(is_selected);
    renderer.draw_text(
        input_x,
        layout.model_y,
        &format!("{model_prefix} model: "),
        label_color,
    );
    render_model_value(renderer, editor, layout, input_x + 9, is_selected);
}

fn render_model_value(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
    model_x: u16,
    is_selected: bool,
) {
    let available = layout.content.right().saturating_sub(model_x) as usize;
    let (model_text, model_color) = if editor.config.model.is_empty() {
        ("daemon default".to_string(), Color::DarkGrey)
    } else {
        (tail_text(&editor.config.model, available), Color::White)
    };
    let visible = truncate_label(&model_text, available);
    renderer.draw_text(model_x, layout.model_y, &visible, model_color);
    render_model_cursor(renderer, editor, layout, model_x, &visible, is_selected);
}

fn render_model_cursor(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
    model_x: u16,
    visible: &str,
    is_selected: bool,
) {
    if !is_selected {
        return;
    }
    let cursor_x = model_cursor_x(editor, model_x, visible);
    if cursor_x < layout.content.right() {
        renderer.draw_char(cursor_x, layout.model_y, '|', Color::Yellow);
    }
}

fn model_cursor_x(editor: &ThoughtConfigEditorState, model_x: u16, visible: &str) -> u16 {
    if editor.config.model.is_empty() {
        model_x
    } else {
        model_x + visible.chars().count() as u16
    }
}

fn render_status_help(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
) {
    renderer.draw_text(
        layout.content.x,
        layout.model_y + 1,
        &truncate_label(
            editor.model_presets_hint().unwrap_or_default(),
            layout.content.width as usize,
        ),
        Color::DarkGrey,
    );
    renderer.draw_text(
        layout.content.x,
        layout.model_y + 2,
        "tab moves  arrows adjust  enter acts  esc cancels",
        Color::DarkGrey,
    );
}

fn render_action_buttons(
    renderer: &mut Renderer,
    editor: &ThoughtConfigEditorState,
    layout: &ThoughtConfigEditorLayout,
) {
    renderer.draw_text(
        layout.content.x,
        layout.buttons_y,
        "[test]",
        button_color(editor, ThoughtConfigEditorField::Test),
    );
    renderer.draw_text(
        layout.content.x + 8,
        layout.buttons_y,
        "[save]",
        button_color(editor, ThoughtConfigEditorField::Save),
    );
    renderer.draw_text(
        layout.content.x + 16,
        layout.buttons_y,
        "[cancel]",
        button_color(editor, ThoughtConfigEditorField::Cancel),
    );
}

fn button_color(editor: &ThoughtConfigEditorState, field: ThoughtConfigEditorField) -> Color {
    if editor.focus == field {
        Color::White
    } else {
        Color::DarkGrey
    }
}
