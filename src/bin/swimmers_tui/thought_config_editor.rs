use super::*;
use swimmers::openrouter_models::cached_or_default_openrouter_candidates;

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
    pub(crate) fn new(config: ThoughtConfig, daemon_defaults: Option<DaemonDefaults>) -> Self {
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
        const OPTIONS: [&str; 4] = ["", "claude", "codex", "openrouter"];
        let current = OPTIONS
            .iter()
            .position(|option| option.eq_ignore_ascii_case(self.config.backend.trim()))
            .unwrap_or(0) as isize;
        let len = OPTIONS.len() as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.config.backend = OPTIONS[next].to_string();
        self.normalize_model_for_backend();
    }

    pub(crate) fn backend_label(&self) -> &'static str {
        if self.config.backend.trim().is_empty() {
            "auto"
        } else if self.config.backend.eq_ignore_ascii_case("claude") {
            "claude"
        } else if self.config.backend.eq_ignore_ascii_case("codex") {
            "codex"
        } else if self.config.backend.eq_ignore_ascii_case("openrouter") {
            "openrouter"
        } else {
            "custom"
        }
    }

    pub(crate) fn daemon_label(&self) -> String {
        match &self.daemon_defaults {
            Some(defaults) => {
                let backend = if defaults.backend.trim().is_empty() {
                    "auto"
                } else {
                    defaults.backend.as_str()
                };
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
        match self.config.backend.trim().to_ascii_lowercase().as_str() {
            "openrouter" => Some("presets: auto  router  cached free models"),
            "codex" | "codex-cli" | "codex_cli" => Some("presets: auto  5.1-mini  5.3-codex  5.4"),
            "claude" | "claude-cli" | "claude_cli" => Some("presets: auto  haiku  sonnet"),
            _ => Some("auto backend uses daemon default model"),
        }
    }

    fn model_preset_values(&self) -> Vec<String> {
        if self.config.backend.eq_ignore_ascii_case("openrouter") {
            let mut values = vec![String::new()];
            values.extend(self.openrouter_model_presets.iter().cloned());
            values
        } else if self.config.backend.eq_ignore_ascii_case("codex") {
            vec![
                String::new(),
                "gpt-5.1-codex-mini".to_string(),
                "gpt-5.3-codex".to_string(),
                "gpt-5.4".to_string(),
            ]
        } else if self.config.backend.eq_ignore_ascii_case("claude") {
            vec![String::new(), "haiku".to_string(), "sonnet".to_string()]
        } else {
            vec![String::new()]
        }
    }

    pub(crate) fn replace_openrouter_model_presets(&mut self, models: Vec<String>) {
        self.openrouter_model_presets = if models.is_empty() {
            cached_or_default_openrouter_candidates()
        } else {
            models
        };
    }

    fn normalize_model_for_backend(&mut self) {
        let trimmed = self.config.model.trim();
        if trimmed.is_empty() {
            self.config.model.clear();
            return;
        }

        let keep = match self.config.backend.trim().to_ascii_lowercase().as_str() {
            "" => false,
            "openrouter" => trimmed.contains('/'),
            "codex" | "codex-cli" | "codex_cli" => trimmed.starts_with("gpt-"),
            "claude" | "claude-cli" | "claude_cli" => {
                matches!(trimmed, "haiku" | "sonnet" | "opus") || trimmed.starts_with("claude-")
            }
            _ => true,
        };

        if !keep {
            self.config.model.clear();
        }
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

    let enabled_prefix = if editor.focus == ThoughtConfigEditorField::Enabled {
        ">"
    } else {
        " "
    };
    let enabled_label = if editor.config.enabled { "on" } else { "off" };
    renderer.draw_text(
        layout.content.x,
        layout.enabled_y,
        &format!("{enabled_prefix} enabled: {enabled_label}"),
        if editor.focus == ThoughtConfigEditorField::Enabled {
            Color::White
        } else {
            Color::DarkGrey
        },
    );

    let backend_prefix = if editor.focus == ThoughtConfigEditorField::Backend {
        ">"
    } else {
        " "
    };
    renderer.draw_text(
        layout.content.x,
        layout.backend_y,
        &format!("{backend_prefix} backend: {}", editor.backend_label()),
        if editor.focus == ThoughtConfigEditorField::Backend {
            Color::White
        } else {
            Color::DarkGrey
        },
    );

    let model_prefix = if editor.focus == ThoughtConfigEditorField::Model {
        ">"
    } else {
        " "
    };
    let input_x = layout.content.x;
    renderer.draw_text(
        input_x,
        layout.model_y,
        &format!("{model_prefix} model: "),
        if editor.focus == ThoughtConfigEditorField::Model {
            Color::White
        } else {
            Color::DarkGrey
        },
    );
    let model_x = input_x + 9;
    let available = layout.content.right().saturating_sub(model_x) as usize;
    let (model_text, model_color) = if editor.config.model.is_empty() {
        ("daemon default".to_string(), Color::DarkGrey)
    } else {
        (tail_text(&editor.config.model, available), Color::White)
    };
    let visible = truncate_label(&model_text, available);
    renderer.draw_text(model_x, layout.model_y, &visible, model_color);
    if editor.focus == ThoughtConfigEditorField::Model {
        let cursor_x = if editor.config.model.is_empty() {
            model_x
        } else {
            model_x + visible.chars().count() as u16
        };
        if cursor_x < layout.content.right() {
            renderer.draw_char(cursor_x, layout.model_y, '|', Color::Yellow);
        }
    }

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

    let test_color = if editor.focus == ThoughtConfigEditorField::Test {
        Color::White
    } else {
        Color::DarkGrey
    };
    let save_color = if editor.focus == ThoughtConfigEditorField::Save {
        Color::White
    } else {
        Color::DarkGrey
    };
    let cancel_color = if editor.focus == ThoughtConfigEditorField::Cancel {
        Color::White
    } else {
        Color::DarkGrey
    };
    renderer.draw_text(layout.content.x, layout.buttons_y, "[test]", test_color);
    renderer.draw_text(layout.content.x + 8, layout.buttons_y, "[save]", save_color);
    renderer.draw_text(
        layout.content.x + 16,
        layout.buttons_y,
        "[cancel]",
        cancel_color,
    );
}
