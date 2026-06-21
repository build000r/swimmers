use super::*;

#[test]
fn handle_thought_config_paste_appends_text_when_model_field_focused() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.thought_config_editor = Some(thought_config_test_editor(ThoughtConfigEditorField::Model));

    app.handle_thought_config_paste("-pro");

    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free-pro"),
        "paste should append to focused model field"
    );
}

#[test]
fn handle_thought_config_paste_no_op_for_non_model_fields() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.thought_config_editor = Some(thought_config_test_editor(
        ThoughtConfigEditorField::Backend,
    ));

    app.handle_thought_config_paste("garbage");

    // Model field is unchanged because focus is on Backend.
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free")
    );
}

#[test]
fn handle_thought_config_paste_blocks_during_pending_interaction() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.thought_config_editor = Some(thought_config_test_editor(ThoughtConfigEditorField::Model));
    let (_tx, rx) = tokio::sync::oneshot::channel();
    app.pending_interaction = Some(rx);

    app.handle_thought_config_paste("attempted-paste");

    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free"),
        "paste must not modify state while an interaction is pending"
    );
    assert_eq!(
        app.visible_message(),
        Some("wait for the current action to finish")
    );
}

#[test]
fn handle_thought_config_paste_no_op_when_editor_closed() {
    let api = MockApi::new();
    let mut app = make_app(api);
    // No editor is open; paste should be a silent no-op (not a panic).
    assert!(app.thought_config_editor.is_none());

    app.handle_thought_config_paste("anything");

    assert!(app.thought_config_editor.is_none());
    assert!(app.visible_message().is_none());
}

#[test]
fn publish_selection_skips_redundant_publish_when_unchanged_and_unforced() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    app.published_selected_id = Some("s1".to_string());

    app.publish_selection(Some("s1".to_string()), false);

    // Same session_id without force ⇒ skipped — no API call recorded.
    assert!(
        api.publish_calls().is_empty(),
        "redundant publish must be skipped, got: {:?}",
        api.publish_calls()
    );
    assert_eq!(app.published_selected_id.as_deref(), Some("s1"));
}

#[test]
fn publish_selection_force_publishes_even_when_unchanged() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    app.published_selected_id = Some("s1".to_string());

    app.publish_selection(Some("s1".to_string()), true);

    assert_eq!(api.publish_calls(), vec![Some("s1".to_string())]);
    assert_eq!(app.published_selected_id.as_deref(), Some("s1"));
}

#[test]
fn publish_selection_records_error_message_on_api_failure() {
    let api = MockApi::new();
    api.push_publish_selection(Err("publish broke".to_string()));
    let mut app = make_app(api);

    app.publish_selection(Some("s7".to_string()), false);

    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        Some("publish broke")
    );
    // The published id stays at whatever it was before — failed publish must
    // not be remembered as successful.
    assert_eq!(app.published_selected_id, None);
}

#[test]
fn resolve_tui_log_path_falls_back_to_tmpdir_when_override_unset() {
    let _lock = lock_test_env();
    let tmp = tempdir().expect("tempdir");
    let prior_dir = env::var_os("SWIMMERS_TUI_LOG_DIR");
    let prior_tmp = env::var_os("TMPDIR");
    env::remove_var("SWIMMERS_TUI_LOG_DIR");
    env::set_var("TMPDIR", tmp.path());

    let (dir, path) = super::resolve_tui_log_path();
    assert_eq!(dir, tmp.path());
    assert_eq!(path.parent(), Some(tmp.path()));

    match prior_dir {
        Some(value) => env::set_var("SWIMMERS_TUI_LOG_DIR", value),
        None => env::remove_var("SWIMMERS_TUI_LOG_DIR"),
    }
    match prior_tmp {
        Some(value) => env::set_var("TMPDIR", value),
        None => env::remove_var("TMPDIR"),
    }
}
