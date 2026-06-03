use super::*;

#[test]
fn handle_tui_event_covers_key_paste_mouse_and_resize_paths() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
    app.open_initial_request("/tmp/project".to_string(), None);

    assert!(handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Paste("hello".to_string()),
    )
    .expect("paste event should succeed"));
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("hello")
    );

    app.close_initial_request();
    assert!(!handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
    )
    .expect("quit key should succeed"));

    assert!(handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }),
    )
    .expect("mouse up should succeed"));

    assert!(
        handle_tui_event(&mut app, &mut renderer, layout, Event::Resize(90, 20),)
            .expect("resize should succeed")
    );
    assert_eq!(renderer.width(), 90);
    assert_eq!(renderer.height(), 20);
}

fn mouse_down(column: u16, row: u16) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

#[test]
fn handle_mouse_down_early_returns_when_thought_config_editor_open() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let renderer = test_renderer(120, 32);
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig::default(),
        None,
    ));
    assert!(app.thought_config_editor.is_some());
    // Should return immediately without panicking
    handle_mouse_down(&mut app, &renderer, layout, mouse_down(10, 10));
    // State unchanged — still in editor
    assert!(app.thought_config_editor.is_some());
}

#[test]
fn handle_mouse_down_early_returns_when_initial_request_open() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let renderer = test_renderer(120, 32);
    app.open_initial_request("/tmp/project".to_string(), None);
    assert!(app.initial_request.is_some());
    handle_mouse_down(&mut app, &renderer, layout, mouse_down(10, 10));
    assert!(app.initial_request.is_some());
}

#[test]
fn handle_mouse_down_plain_app_reaches_workspace_click() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let renderer = test_renderer(120, 32);
    // Click at (0,0) hits workspace area — should not panic
    handle_mouse_down(&mut app, &renderer, layout, mouse_down(0, 0));
}

#[test]
fn handle_key_event_schema_tab_q_returns_false() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    // Default active_tab is Schema
    assert!(!handle_key_event(&mut app, layout, key(KeyCode::Char('q'))));
}

#[test]
fn handle_key_event_schema_tab_esc_closes_viewer() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = open_mermaid_with_plan_tabs(api);
    // Schema tab, no focus → Esc closes viewer
    assert!(handle_key_event(&mut app, layout, key(KeyCode::Esc)));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn handle_key_event_schema_tab_navigation_keys() {
    let layout = test_layout(120, 32);
    let (mut app, _, _) = open_mermaid_on_plan_tab(Some("graph LR\nA-->B"), DomainPlanTab::Schema);
    // These pan/zoom keys should return true and not panic
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Char('h'),
        KeyCode::Char('l'),
        KeyCode::Char('k'),
        KeyCode::Char('j'),
        KeyCode::Char('+'),
        KeyCode::Char('='),
        KeyCode::Char('-'),
        KeyCode::Char('0'),
        KeyCode::Char('o'),
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Char('x'), // unknown → true
    ] {
        assert!(
            handle_key_event(&mut app, layout, key(code)),
            "code: {code:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Async background refresh tests
