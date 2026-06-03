use super::*;

#[test]
fn help_overlay_shows_keybindings_and_dismisses_on_any_key() {
    let api = MockApi::new();
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    api.push_backend_health(Ok(healthy_backend_health()));

    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.refresh_with_feedback(layout, false);

    assert!(!app.show_help);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "? help").is_some(),
        "footer should contain help hint"
    );
    assert!(
        find_text_position(&renderer, "keybindings").is_none(),
        "help overlay should not be visible initially"
    );

    app.show_help = true;
    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "keybindings").is_some(),
        "help overlay should render keybinding header"
    );
    assert!(
        find_text_position(&renderer, "arrows/hjkl").is_some(),
        "help overlay should list movement keys"
    );
    assert!(
        find_text_position(&renderer, "press any key to dismiss").is_some(),
        "footer should show dismiss hint when help is visible"
    );

    handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert!(!app.show_help, "any key should dismiss help overlay");
}

#[test]
fn help_overlay_q_dismisses_and_quits() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.show_help = true;

    let should_continue = handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
    );
    assert!(!app.show_help);
    assert!(!should_continue, "q in help overlay should quit");
}

#[test]
fn help_overlay_renders_in_narrow_terminal() {
    let api = MockApi::new();
    let layout = test_layout(80, 24);
    let mut app = make_app(api);
    app.show_help = true;

    let mut renderer = test_renderer(80, 24);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "keybindings").is_some(),
        "help overlay should render in 80-col terminal"
    );
}

#[test]
fn footer_hides_voice_hint_when_unsupported() {
    let api = MockApi::new();
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    api.push_backend_health(Ok(healthy_backend_health()));

    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.refresh_with_feedback(layout, false);

    app.open_initial_request(TEST_REPO_SWIMMERS.to_string(), None);
    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);

    let footer_row = layout.footer_start_y + 2;
    let footer_text = row_text(&renderer, footer_row);
    if matches!(app.voice_state, VoiceUiState::Unsupported) {
        assert!(
            !footer_text.contains("ctrl-v voice"),
            "voice hint should be hidden when voice feature is off"
        );
    }
}

#[test]
fn empty_aquarium_and_tmux_unavailable_do_not_overlap() {
    let api = MockApi::new();
    let mut health = healthy_backend_health();
    health.dependencies.as_mut().unwrap().tmux_discovery.status = "unavailable".to_string();
    api.push_fetch_sessions(Ok(vec![]));
    api.push_backend_health(Ok(health));

    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.refresh_with_feedback(layout, false);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "tmux unavailable").is_some(),
        "should show tmux unavailable"
    );
    assert!(
        find_text_position(&renderer, "no tmux sessions found").is_none(),
        "should not also show the generic empty message"
    );
}

#[test]
fn thought_rail_shows_configure_hint_when_clawgs_unavailable() {
    let api = MockApi::new();
    let mut app = App::new(test_runtime(), api);
    app.daemon_defaults_status = DaemonDefaultsStatus::Unavailable;

    let layout = app.layout_for_terminal(120, 32);
    let thought_content = layout
        .thought_content
        .expect("unavailable clawgs should show thought rail");
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let message = panel
        .empty_message
        .as_deref()
        .expect("should have an empty message");
    assert!(
        message.contains("press t"),
        "should hint at 't' keybinding for config: got {message:?}"
    );
}

#[test]
fn thought_rail_shows_waiting_hint_with_sleeping_session() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.daemon_defaults_status = DaemonDefaultsStatus::Available;
    let full_layout = WorkspaceLayout::for_terminal_without_thought_panel(120, 32);
    app.merge_sessions(
        vec![sleeping_session(
            "sess-sleeping",
            "7",
            TEST_REPO_SWIMMERS,
            "2026-03-08T12:00:00Z",
        )],
        full_layout.overview_field,
    );

    let layout = app.layout_for_terminal(120, 32);
    let thought_content = layout
        .thought_content
        .expect("sleeping session should show thought rail");
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert!(
        panel.empty_message.is_none(),
        "sleeping session should produce an entry, not an empty message"
    );
}
