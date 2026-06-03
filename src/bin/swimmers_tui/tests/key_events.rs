use super::*;

#[test]
fn renderer_flush_copies_drawn_cells_into_last_buffer() {
    let mut renderer = test_renderer(4, 2);
    renderer.draw_char(0, 0, 'A', Color::Green);
    renderer.draw_char(1, 0, 'B', Color::Yellow);

    renderer.flush().expect("flush should succeed");

    assert!(renderer
        .buffer
        .iter()
        .zip(renderer.last_buffer.iter())
        .all(|(current, previous)| current == previous));
}

#[test]
fn move_selection_updates_picker_and_visible_session_selection() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![
            session_summary("sess-1", "1", TEST_REPO_ALPHA),
            session_summary("sess-2", "2", TEST_REPO_BETA),
        ],
        layout.overview_field,
    );

    app.move_selection(1, layout.overview_field);
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

    let mut picker = PickerState::new(
        3,
        3,
        dir_response("/tmp", &[("alpha", false), ("beta", false)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.selection = PickerSelection::SpawnHere;
    app.picker = Some(picker);

    app.move_selection(1, layout.overview_field);

    assert!(matches!(
        app.picker.as_ref().map(|picker| picker.selection),
        Some(PickerSelection::Entry(0))
    ));
}

#[test]
fn handle_key_event_covers_initial_request_picker_and_quit_paths() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![
            session_summary("sess-1", "1", TEST_REPO_ALPHA),
            session_summary("sess-2", "2", TEST_REPO_BETA),
        ],
        layout.overview_field,
    );

    app.open_initial_request("/tmp/project".to_string(), None);
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    ));
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("x")
    );

    app.close_initial_request();
    app.picker = Some(PickerState::new(
        3,
        3,
        dir_response("/tmp", &[("alpha", false)]),
        true,
        SpawnTool::Codex,
        None,
    ));
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(app.picker.is_none());

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    ));
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));

    assert!(!handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    ));
}

#[test]
fn help_overlay_toggles_renders_and_dismisses_as_modal() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('?'))));
    assert!(app.show_help);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);

    assert!(find_text_position(&renderer, "keybindings").is_some());
    assert!(find_text_position(&renderer, "thought config editor").is_some());
    assert!(
        row_text(&renderer, layout.footer_start_y + 2).contains("press any key to dismiss help")
    );

    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('x'))));
    assert!(!app.show_help);
}

#[test]
fn help_overlay_keeps_core_rows_visible_in_narrow_and_wide_layouts() {
    for (width, height) in [(70, 20), (140, 40)] {
        let api = MockApi::new();
        let mut app = make_app(api);
        app.show_help = true;
        let layout = app.layout_for_terminal(width, height);
        let mut renderer = test_renderer(width, height);

        app.render(&mut renderer, layout);

        assert!(
            find_text_position(&renderer, "keybindings").is_some(),
            "help title should render at {width}x{height}"
        );
        assert!(
            find_text_position(&renderer, "move selection").is_some(),
            "first help row should render at {width}x{height}"
        );
        assert!(
            row_text(&renderer, layout.footer_start_y + 2).contains("press any key"),
            "footer dismissal hint should render at {width}x{height}"
        );
    }
}

#[test]
fn initial_request_footer_hides_voice_shortcut_when_voice_is_unsupported() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.open_initial_request(TEST_REPO_SWIMMERS.to_string(), None);
    app.voice_state = VoiceUiState::Unsupported;
    let mut renderer = test_renderer(120, 32);

    render_footer(&app, &mut renderer, layout.footer_start_y);
    let unsupported_footer = row_text(&renderer, layout.footer_start_y + 2);
    assert!(unsupported_footer.contains("request: type prompt"));
    assert!(!unsupported_footer.contains("ctrl-v voice"));

    app.voice_state = VoiceUiState::Ready;
    render_footer(&app, &mut renderer, layout.footer_start_y);
    assert!(row_text(&renderer, layout.footer_start_y + 2).contains("ctrl-v voice"));
}

#[test]
fn ctrl_c_does_not_quit_without_embedded_shutdown() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ));
}

#[test]
fn handle_key_event_routes_plain_picker_chars_to_search_before_hotkeys() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    let mut picker = PickerState::new(
        3,
        3,
        DirListResponse {
            path: TEST_REPOS_ROOT.to_string(),
            entries: vec![repo_dir_entry("codex", true, Some(true), None)],
            overlay_label: None,
            groups: Vec::new(),
            launch_targets: Vec::new(),
            default_launch_target: None,
        },
        true,
        SpawnTool::Codex,
        None,
    );
    picker.selection = PickerSelection::Entry(0);
    app.picker = Some(picker);

    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('c'))));

    assert!(api.start_repo_action_calls().is_empty());
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.search.as_str()),
        Some("c")
    );
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.visible_entries()),
        Some(vec![0])
    );

    assert!(app.picker_search_clear());
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('q'))));
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.search.as_str()),
        Some("q")
    );
}

#[test]
fn handle_key_event_plan_tab_bracket_navigation() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    // `[` cycles backward, `]` cycles forward
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('['))));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char(']'))));
}

#[test]
fn handle_key_event_plan_tab_digit_selects_tab() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    // '1' selects the first tab (Schema); '2' selects Plan; '9' is out of range → noop
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('1'))));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('2'))));
    // Out-of-range digit: tabs has 3 entries, '7' is index 6 → doesn't exist → still returns true
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('7'))));
}

#[test]
fn handle_key_event_text_tab_scroll_keys() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    // Switch to a non-Schema tab to enter text-tab mode
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.active_tab = DomainPlanTab::Plan;
        viewer.plan_text_content = Some("line1\nline2\nline3".to_string());
        viewer.plan_text_lines = vec![
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
        ];
        viewer.plan_text_cached_width = 120;
    }
    // Down/j, Up/k, PageDown, PageUp, Home, End, 'o', and fallthrough
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Down)));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('j'))));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Up)));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('k'))));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::PageDown)));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::PageUp)));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Home)));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::End)));
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('o'))));
    // Arbitrary key → falls through to `_ => true`
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Char('x'))));
}

#[test]
fn handle_key_event_text_tab_esc_closes_viewer() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.active_tab = DomainPlanTab::Plan;
    }
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Esc)));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn handle_key_event_text_tab_q_returns_false() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.active_tab = DomainPlanTab::Plan;
    }
    assert!(!handle_key_event(&mut app, layout, key(KeyCode::Char('q'))));
}

#[test]
fn handle_key_event_opens_thought_config_editor() {
    let api = MockApi::new();
    api.push_fetch_thought_config(Ok(ThoughtConfigResponse {
        config: ThoughtConfig {
            backend: "claude".to_string(),
            model: "haiku".to_string(),
            ..ThoughtConfig::default()
        },
        daemon_defaults: Some(DaemonDefaults {
            model: "haiku".to_string(),
            backend: "claude".to_string(),
            agent_prompt: "agent".to_string(),
            terminal_prompt: "terminal".to_string(),
        }),
        ui: swimmers::types::ThoughtConfigUiMetadata::default(),
    }));
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
    ));
    assert!(app.pending_interaction.is_some());
    assert!(app.thought_config_editor.is_none());
    poll_until_interaction(&mut app);

    let editor = app
        .thought_config_editor
        .as_ref()
        .expect("thought config editor should open");
    assert_eq!(editor.config.backend, "grok");
    assert_eq!(editor.config.model, "haiku");
}

#[test]
fn handle_key_event_toggles_native_app_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    });
    api.push_set_native_app(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
    ));
    assert!(app.pending_interaction.is_some());
    assert_eq!(
        app.native_status.as_ref().and_then(|status| status.app_id),
        Some(NativeDesktopApp::Iterm)
    );
    poll_until_interaction(&mut app);

    assert_eq!(api.set_native_app_calls(), vec![NativeDesktopApp::Ghostty]);
    assert_eq!(
        app.native_status.as_ref().and_then(|status| status.app_id),
        Some(NativeDesktopApp::Ghostty)
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("terminal handoff target: Ghostty (swap)")
    );
}

#[test]
fn handle_key_event_toggles_ghostty_mode_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    });
    api.push_set_native_mode(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Add),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
    ));
    assert!(app.pending_interaction.is_some());
    assert_eq!(
        app.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode),
        Some(GhosttyOpenMode::Swap)
    );
    poll_until_interaction(&mut app);

    assert_eq!(api.set_native_mode_calls(), vec![GhosttyOpenMode::Add]);
    assert_eq!(
        app.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode),
        Some(GhosttyOpenMode::Add)
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("Ghostty placement: new split")
    );
}

#[test]
fn thought_config_editor_updates_backend_and_model_then_saves() {
    let api = MockApi::new();
    api.push_fetch_thought_config(Ok(ThoughtConfigResponse {
        config: ThoughtConfig::default(),
        daemon_defaults: Some(DaemonDefaults {
            model: "openrouter/free".to_string(),
            backend: "openrouter".to_string(),
            agent_prompt: "agent".to_string(),
            terminal_prompt: "terminal".to_string(),
        }),
        ui: swimmers::types::ThoughtConfigUiMetadata::default(),
    }));
    api.push_update_thought_config(Ok(ThoughtConfig {
        backend: "grok".to_string(),
        model: String::new(),
        ..ThoughtConfig::default()
    }));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());

    app.open_thought_config_editor();
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert!(app.thought_config_editor.is_some());

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    );
    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert!(app.pending_refresh.is_some());
    poll_until_refresh(&mut app, layout);

    assert!(app.thought_config_editor.is_none());
    assert_eq!(api.update_thought_config_calls().len(), 1);
    let saved = api
        .update_thought_config_calls()
        .into_iter()
        .next()
        .expect("saved config");
    assert_eq!(saved.backend, "grok");
    assert!(saved.model.is_empty());
    assert_eq!(api.test_thought_config_calls().len(), 1);
}

pub(super) fn thought_config_test_editor(
    focus: ThoughtConfigEditorField,
) -> ThoughtConfigEditorState {
    let mut editor = ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: String::new(),
            ..ThoughtConfig::default()
        },
        None,
    );
    editor.focus = focus;
    editor
}

#[test]
fn thought_config_key_pending_action_blocks_edits_but_escape_closes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(thought_config_test_editor(ThoughtConfigEditorField::Model));
    let (_tx, rx) = tokio::sync::oneshot::channel();
    app.pending_interaction = Some(rx);

    app.handle_thought_config_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        layout,
    );

    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free")
    );
    assert_eq!(
        app.visible_message(),
        Some("wait for the current action to finish")
    );

    app.handle_thought_config_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), layout);

    assert!(app.thought_config_editor.is_none());
    assert!(
        app.pending_interaction.is_some(),
        "escape only closes the editor; the in-flight action remains pending"
    );
}

#[test]
fn thought_config_key_moves_focus_and_edits_fields() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(thought_config_test_editor(
        ThoughtConfigEditorField::Backend,
    ));

    app.handle_thought_config_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), layout);
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.focus),
        Some(ThoughtConfigEditorField::Model)
    );

    app.handle_thought_config_key(
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
        layout,
    );
    app.handle_thought_config_key(
        KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
        layout,
    );
    app.handle_thought_config_key(
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        layout,
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/freeg")
    );

    app.handle_thought_config_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), layout);
    app.handle_thought_config_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), layout);
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.backend.as_str()),
        Some("grok")
    );
    app.handle_thought_config_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), layout);
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.backend.as_str()),
        Some("openrouter")
    );

    app.handle_thought_config_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), layout);
    app.handle_thought_config_key(
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        layout,
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.enabled),
        Some(false)
    );
}

#[test]
fn thought_config_key_enter_on_cancel_closes_editor() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(thought_config_test_editor(ThoughtConfigEditorField::Cancel));

    app.handle_thought_config_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), layout);

    assert!(app.thought_config_editor.is_none());
}

#[test]
fn thought_config_editor_test_button_probes_without_saving() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Test;
    }
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    assert!(app.thought_config_editor.is_some());
    assert!(api.update_thought_config_calls().is_empty());
    let tested = api
        .test_thought_config_calls()
        .into_iter()
        .next()
        .expect("tested config");
    assert_eq!(tested.backend, "openrouter");
    assert_eq!(tested.model, "openrouter/free");
}

#[test]
fn thought_config_editor_test_button_rotates_openrouter_model_after_invalid_model_error() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "old/expired:free".to_string(),
            ..ThoughtConfig::default()
        },
        Some(DaemonDefaults {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            agent_prompt: String::new(),
            terminal_prompt: String::new(),
        }),
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Test;
    }
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: false,
        message: "probe failed: old/expired:free is not a valid model ID".to_string(),
        last_backend_error: Some("old/expired:free is not a valid model ID".to_string()),
        llm_calls: 0,
    }));
    api.push_refresh_openrouter_candidates(Ok(vec![
        "openrouter/free".to_string(),
        "google/gemma-3-4b-it:free".to_string(),
    ]));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free")
    );
    assert!(app
        .visible_message()
        .unwrap_or_default()
        .contains("rotated to openrouter/free"));
}

#[test]
fn thought_config_editor_save_rotates_and_persists_openrouter_model_after_invalid_model_error() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "old/expired:free".to_string(),
            ..ThoughtConfig::default()
        },
        Some(DaemonDefaults {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            agent_prompt: String::new(),
            terminal_prompt: String::new(),
        }),
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Save;
    }
    api.push_update_thought_config(Ok(ThoughtConfig {
        backend: "openrouter".to_string(),
        model: "old/expired:free".to_string(),
        ..ThoughtConfig::default()
    }));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: false,
        message: "probe failed: old/expired:free is not a valid model ID".to_string(),
        last_backend_error: Some("old/expired:free is not a valid model ID".to_string()),
        llm_calls: 0,
    }));
    api.push_refresh_openrouter_candidates(Ok(vec![
        "openrouter/free".to_string(),
        "google/gemma-3-4b-it:free".to_string(),
    ]));
    api.push_test_thought_config(Ok(ThoughtConfigTestResponse {
        ok: true,
        message: "probe succeeded".to_string(),
        last_backend_error: None,
        llm_calls: 1,
    }));
    api.push_update_thought_config(Ok(ThoughtConfig {
        backend: "openrouter".to_string(),
        model: "openrouter/free".to_string(),
        ..ThoughtConfig::default()
    }));

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert!(app.pending_refresh.is_some());
    poll_until_refresh(&mut app, layout);

    assert!(app.thought_config_editor.is_none());
    assert_eq!(api.update_thought_config_calls().len(), 2);
    assert_eq!(
        api.update_thought_config_calls()
            .last()
            .map(|config| config.model.as_str()),
        Some("openrouter/free")
    );
    assert!(app
        .visible_message()
        .unwrap_or_default()
        .contains("rotated to openrouter/free"));
}

#[test]
fn thought_config_editor_cycles_current_openrouter_model_presets() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: String::new(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    if let Some(editor) = &mut app.thought_config_editor {
        editor.focus = ThoughtConfigEditorField::Model;
        editor.config.model.clear();
        editor.replace_openrouter_model_presets(vec![
            "openrouter/free".to_string(),
            "nvidia/nemotron-3-super-120b-a12b:free".to_string(),
            "arcee-ai/trinity-large-preview:free".to_string(),
        ]);
    }

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("openrouter/free")
    );

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("nvidia/nemotron-3-super-120b-a12b:free")
    );

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    );
    assert_eq!(
        app.thought_config_editor
            .as_ref()
            .map(|editor| editor.config.model.as_str()),
        Some("arcee-ai/trinity-large-preview:free")
    );
}

#[test]
fn thought_config_editor_clears_incompatible_model_when_backend_changes() {
    let mut editor = ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "openrouter/free".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    );

    editor.cycle_backend(1);
    assert_eq!(editor.backend_label(), "grok");
    assert!(editor.config.model.is_empty());

    editor.config.model = "gpt-5.4".to_string();
    editor.cycle_backend(-1);
    assert_eq!(editor.backend_label(), "openrouter");
    assert!(editor.config.model.is_empty());
}

#[test]
fn picker_activate_selection_opens_initial_request_and_reloads_children() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        2,
        2,
        dir_response("/tmp", &[("child", true), ("leaf", false)]),
        true,
        SpawnTool::Codex,
        None,
    ));

    app.picker_activate_selection(layout.overview_field);
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some("/tmp")
    );

    app.close_initial_request();
    if let Some(picker) = &mut app.picker {
        picker.selection = PickerSelection::Entry(0);
    }
    api.push_list_dirs(Ok(dir_response("/tmp/child", &[("nested", false)])));
    app.picker_activate_selection(layout.overview_field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert_eq!(
        api.list_calls(),
        vec![(Some("/tmp/child".to_string()), true)]
    );

    if let Some(picker) = &mut app.picker {
        picker.apply_response(dir_response("/tmp", &[("leaf", false)]), false);
        picker.selection = PickerSelection::Entry(0);
    }
    app.picker_activate_selection(layout.overview_field);
    assert_eq!(
        app.initial_request.as_ref().map(|state| state.cwd.as_str()),
        Some("/tmp/leaf")
    );
}

#[test]
fn handle_workspace_click_routes_thought_and_overview_interactions() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.sprite_theme_override = Some(SpriteTheme::Jelly);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        )],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let body_x = panel.rows[0]
        .session_rect
        .expect("row should have clickable session")
        .x
        .saturating_add(1);
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-1".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: body_x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);
    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/7")
    );

    let entity_rect = entity_rect_for(&app, "sess-1", layout.overview_field);
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-1".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: entity_rect.x,
            row: entity_rect.y,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(
        api.open_calls(),
        vec!["sess-1".to_string(), "sess-1".to_string()]
    );
}

#[test]
fn clicking_native_status_label_toggles_native_app_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    });
    api.push_set_native_app(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));
    let rect = app
        .native_status_rect(120)
        .expect("terminal handoff status should render in header");

    assert!(handle_split_or_header_click(
        &mut app,
        120,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert_eq!(api.set_native_app_calls(), vec![NativeDesktopApp::Ghostty]);
    assert_eq!(
        app.native_status.as_ref().and_then(|status| status.app_id),
        Some(NativeDesktopApp::Ghostty)
    );
}

#[test]
fn clicking_ghostty_mode_label_toggles_preview_mode_live() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    });
    api.push_set_native_mode(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Add),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));
    let rect = app
        .ghostty_mode_rect(120)
        .expect("Ghostty placement should render in header");

    assert!(handle_split_or_header_click(
        &mut app,
        120,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert_eq!(api.set_native_mode_calls(), vec![GhosttyOpenMode::Add]);
    assert_eq!(
        app.native_status
            .as_ref()
            .and_then(|status| status.ghostty_mode),
        Some(GhosttyOpenMode::Add)
    );
}

#[test]
fn clicking_attention_group_label_opens_managed_group() {
    let _lock = TEST_ENV_LOCK.lock().expect("env lock");
    let _size = EnvVarGuard::remove("SWIMMERS_ATTENTION_GROUP_SIZE");
    let _layout_env = EnvVarGuard::remove("SWIMMERS_ATTENTION_GROUP_LAYOUT");
    let _unnumbered = EnvVarGuard::remove("SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED");

    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    });
    api.push_open_attention_group(Ok(NativeAttentionGroupOpenResponse {
        session_id: "attention-group".to_string(),
        tmux_name: "swimmers-attention".to_string(),
        session_count: 3,
        session_ids: vec![
            "sess-1".to_string(),
            "sess-2".to_string(),
            "sess-3".to_string(),
        ],
        backlog_session_ids: Vec::new(),
        status: "swapped".to_string(),
        focused: true,
        pane_id: Some("pane-attention".to_string()),
        attach_command: Some("tmux attach -t swimmers-attention".to_string()),
    }));
    let attention_rect = app
        .attention_group_rect(120)
        .expect("attention group should render in header");
    let native_rect = app
        .native_status_rect(120)
        .expect("terminal handoff status should render in header");
    assert!(
        attention_rect.x >= native_rect.x.saturating_add(native_rect.width),
        "attention group click target should not overlap terminal handoff target"
    );

    assert!(handle_split_or_header_click(
        &mut app,
        120,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: attention_rect.x,
            row: attention_rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);
    assert_eq!(
        api.open_attention_group_calls(),
        vec![(
            6,
            Vec::<String>::new(),
            true,
            false,
            AttentionGroupLayout::Tiled
        )]
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("swapped attention group: 3 sessions")
    );
}

#[test]
fn attention_group_click_uses_env_size_layout_and_unnumbered_policy() {
    let _lock = TEST_ENV_LOCK.lock().expect("env lock");
    let _size = EnvVarGuard::set("SWIMMERS_ATTENTION_GROUP_SIZE", "4");
    let _layout = EnvVarGuard::set("SWIMMERS_ATTENTION_GROUP_LAYOUT", "main-left");
    let _unnumbered = EnvVarGuard::set("SWIMMERS_ATTENTION_GROUP_INCLUDE_UNNUMBERED", "1");

    let api = MockApi::new();
    let mut app = make_app(api.clone());
    api.push_open_attention_group(Ok(NativeAttentionGroupOpenResponse {
        session_id: "attention-group".to_string(),
        tmux_name: "swimmers-attention".to_string(),
        session_count: 4,
        session_ids: vec![
            "sess-1".to_string(),
            "sess-2".to_string(),
            "sess-3".to_string(),
            "sess-4".to_string(),
        ],
        backlog_session_ids: Vec::new(),
        status: "refreshed".to_string(),
        focused: true,
        pane_id: Some("pane-attention".to_string()),
        attach_command: Some("tmux attach -t swimmers-attention".to_string()),
    }));

    app.open_attention_group();
    poll_until_interaction(&mut app);

    assert_eq!(
        api.open_attention_group_calls(),
        vec![(
            4,
            Vec::<String>::new(),
            true,
            true,
            AttentionGroupLayout::MainVertical
        )]
    );
}

#[test]
fn attention_group_click_shows_attach_command_when_native_focus_is_unavailable() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    api.push_open_attention_group(Ok(NativeAttentionGroupOpenResponse {
        session_id: "attention-group".to_string(),
        tmux_name: "swimmers-attention".to_string(),
        session_count: 1,
        session_ids: vec!["sess-1".to_string()],
        backlog_session_ids: Vec::new(),
        status: "refreshed".to_string(),
        focused: false,
        pane_id: None,
        attach_command: Some("tmux attach -t swimmers-attention".to_string()),
    }));

    app.open_attention_group();
    poll_until_interaction(&mut app);

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("refreshed attention group: 1 sessions | tmux attach -t swimmers-attention")
    );
}

#[test]
fn ghostty_ready_sessions_do_not_auto_open_attention_group() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.native_status = Some(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    });

    app.merge_sessions(
        vec![session_summary("sess-ready", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert!(app.attention_group_session_ids.is_empty());
}

#[test]
fn attention_group_does_not_add_ready_session_until_click() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.attention_group_session_ids = vec!["sess-visible".to_string()];

    app.merge_sessions(
        vec![
            session_summary("sess-visible", "7", TEST_REPO_SWIMMERS),
            session_summary("sess-new", "8", TEST_REPO_SWIMMERS),
        ],
        layout.overview_field,
    );

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert_eq!(app.attention_group_session_ids, vec!["sess-visible"]);
}

#[test]
fn attention_group_ignores_ready_churn_without_click() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.attention_group_session_ids = vec!["sess-visible".to_string()];

    app.merge_sessions(
        vec![
            session_summary("sess-visible", "7", TEST_REPO_SWIMMERS),
            session_summary("sess-new", "8", TEST_REPO_SWIMMERS),
        ],
        layout.overview_field,
    );

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert_eq!(app.attention_group_session_ids, vec!["sess-visible"]);
}

#[test]
fn attention_group_visible_session_clear_does_not_refresh_without_click() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.attention_group_session_ids = vec!["sess-visible".to_string(), "sess-cleared".to_string()];

    let visible = session_summary("sess-visible", "7", TEST_REPO_SWIMMERS);
    let mut cleared = session_summary_with_thought(
        "sess-cleared",
        "8",
        TEST_REPO_SWIMMERS,
        "running a tool",
        "2026-05-12T20:00:00Z",
    );
    cleared.current_command = Some("cargo test".to_string());
    let next = session_summary("sess-next", "9", TEST_REPO_SWIMMERS);

    app.merge_sessions(vec![visible, cleared, next], layout.overview_field);

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert_eq!(
        app.attention_group_session_ids,
        vec!["sess-visible".to_string(), "sess-cleared".to_string()]
    );
}

#[test]
fn attention_group_queue_drain_does_not_clear_tracking_without_click() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.attention_group_session_ids = vec!["sess-cleared".to_string()];

    let mut cleared = session_summary_with_thought(
        "sess-cleared",
        "8",
        TEST_REPO_SWIMMERS,
        "running a tool",
        "2026-05-12T20:00:00Z",
    );
    cleared.current_command = Some("cargo test".to_string());

    app.merge_sessions(vec![cleared], layout.overview_field);

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert_eq!(app.attention_group_session_ids, vec!["sess-cleared"]);
}

#[test]
fn clicking_commit_badge_launches_commit_codex_without_opening_session() {
    let api = MockApi::new();
    let launcher = Arc::new(MockCommitLauncher::default());
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app_with_commit_launcher(api.clone(), launcher.clone());
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.commit_candidate = true;
    app.merge_sessions(vec![session.clone()], layout.overview_field);
    let mut thought_session = session.clone();
    thought_session.thought = Some("ready to commit".to_string());
    thought_session.thought_updated_at = Some(
        DateTime::parse_from_rfc3339("2026-03-29T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc),
    );
    app.capture_thought_updates(&[thought_session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let commit_rect = panel.rows[0].commit_rect.expect("commit badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: commit_rect.x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );

    let launch_calls = launcher.calls();
    assert_eq!(api.open_calls(), Vec::<String>::new());
    assert_eq!(launch_calls.len(), 1);
    assert_eq!(launch_calls[0].session_id, session.session_id);
    assert_eq!(launch_calls[0].cwd, session.cwd);
    assert_eq!(launch_calls[0].tmux_name, session.tmux_name);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("commit grok: tmux a -t commit-7-123")
    );
}

#[test]
fn clicking_commit_badge_surfaces_commit_launch_errors() {
    let api = MockApi::new();
    let launcher = Arc::new(MockCommitLauncher::default());
    launcher.fail_with("tmux not found");
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app_with_commit_launcher(api, launcher);
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.commit_candidate = true;
    app.merge_sessions(vec![session], layout.overview_field);
    let mut thought_session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    thought_session.commit_candidate = true;
    thought_session.thought = Some("ready to commit".to_string());
    thought_session.thought_updated_at = Some(
        DateTime::parse_from_rfc3339("2026-03-29T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc),
    );
    app.capture_thought_updates(&[thought_session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let commit_rect = panel.rows[0].commit_rect.expect("commit badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: commit_rect.x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("failed to launch commit grok: tmux not found")
    );
}
