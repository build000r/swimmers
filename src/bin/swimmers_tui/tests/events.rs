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

fn mouse_drag(column: u16, row: u16) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

fn app_with_mermaid_drag(layout: WorkspaceLayout) -> App<MockApi> {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );
    app.open_mermaid_viewer("sess-1".to_string());
    let content_rect = mermaid_content_rect(layout.overview_field);
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.content_rect = Some(content_rect);
    viewer.diagram_width = 1000.0;
    viewer.diagram_height = 800.0;
    viewer.center_x = 500.0;
    viewer.center_y = 400.0;
    viewer.unsupported_reason = None;
    app.mermaid_drag = Some(MermaidDragState {
        start_column: 1,
        start_row: 1,
        start_center_x: 500.0,
        start_center_y: 400.0,
    });
    app
}

fn mermaid_center(app: &App<MockApi>) -> (f32, f32) {
    match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    }
}

#[test]
fn left_drag_dispatch_plan_completes_after_split_drag() {
    assert_eq!(
        plan_left_drag_dispatch(true),
        LeftDragDispatchPlan::Complete
    );
}

#[test]
fn left_drag_dispatch_plan_tries_mermaid_when_split_drag_misses() {
    assert_eq!(
        plan_left_drag_dispatch(false),
        LeftDragDispatchPlan::TryMermaid
    );
}

#[test]
fn left_drag_event_skips_mermaid_drag_when_split_drag_handles() {
    let layout = test_layout(120, 32);
    let mut app = app_with_mermaid_drag(layout);
    let mut renderer = test_renderer(120, 32);
    let divider = layout
        .split_divider
        .expect("wide layout should expose a divider");
    assert!(app.start_split_drag(layout, divider.x));
    let before = mermaid_center(&app);

    assert!(handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Mouse(mouse_drag(divider.x + 5, divider.y)),
    )
    .expect("left drag should be handled"));

    assert_eq!(mermaid_center(&app), before);
}

#[test]
fn left_drag_event_attempts_mermaid_drag_when_split_drag_misses() {
    let layout = test_layout(120, 32);
    let mut app = app_with_mermaid_drag(layout);
    let mut renderer = test_renderer(120, 32);
    let content_rect = mermaid_content_rect(layout.overview_field);
    let before = mermaid_center(&app);

    assert!(handle_tui_event(
        &mut app,
        &mut renderer,
        layout,
        Event::Mouse(mouse_drag(content_rect.x + 8, content_rect.y + 3)),
    )
    .expect("left drag should be handled"));

    assert_ne!(mermaid_center(&app), before);
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
