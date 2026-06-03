use super::*;

#[test]
fn render_thought_config_editor_enabled_field_focused() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            enabled: true,
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Enabled;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_thought_config_editor_model_field_focused_with_model() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            enabled: false,
            model: "claude-opus-4-6".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Model;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_thought_config_editor_model_field_focused_empty_model() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            model: String::new(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Model;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_thought_config_editor_save_and_cancel_focused() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut renderer = test_renderer(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig::default(),
        None,
    ));
    for focus in [
        ThoughtConfigEditorField::Save,
        ThoughtConfigEditorField::Cancel,
        ThoughtConfigEditorField::Test,
        ThoughtConfigEditorField::Backend,
    ] {
        if let Some(editor) = &mut app.thought_config_editor {
            editor.focus = focus;
        }
        app.render(&mut renderer, layout);
    }
}
