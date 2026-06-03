use super::*;

#[test]
fn mermaid_viewer_renders_inline_unsupported_state_and_back_button_restores_aquarium() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
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
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = Some("unsupported terminal backend".to_string());

    app.render(&mut renderer, layout);

    let message_row = mermaid_content_rect(layout.overview_field).y;
    assert!(row_text(&renderer, message_row).contains("unsupported terminal backend"));

    let back_rect = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.back_rect.expect("back rect"),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: back_rect.x,
            row: back_rect.y,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn mermaid_keyboard_controls_pan_zoom_reset_and_escape() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
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

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
    ));
    let (zoom_after_plus, center_after_plus) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.zoom, (viewer.center_x, viewer.center_y)),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(zoom_after_plus, 1.5);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    ));
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    ));
    let (center_after_pan_x, center_after_pan_y) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(center_after_pan_x > center_after_plus.0);
    assert!(center_after_pan_y > center_after_plus.1);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
    ));
    let (zoom_after_reset, center_after_reset_x, center_after_reset_y) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.zoom, viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(zoom_after_reset, 1.0);
    assert_eq!(center_after_reset_x, 0.0);
    assert_eq!(center_after_reset_y, 0.0);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn mermaid_mouse_drag_and_scroll_update_viewport() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
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
    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);

    let (start_column, start_row) =
        find_blank_position(&renderer, content_rect).expect("empty Mermaid canvas cell");
    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: start_column,
            row: start_row,
            modifiers: KeyModifiers::NONE,
        },
    ));
    assert!(app.handle_mermaid_mouse_drag(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: start_column + 5,
            row: start_row + 2,
            modifiers: KeyModifiers::NONE,
        },
    ));
    let (center_after_drag_x, center_after_drag_y) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_ne!(center_after_drag_x, 500.0);
    assert_ne!(center_after_drag_y, 400.0);
    assert!(app.handle_mermaid_mouse_up());

    let zoom_before_scroll = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.zoom,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(app.handle_mermaid_scroll(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: start_column,
            row: start_row,
            modifiers: KeyModifiers::NONE,
        },
        MermaidZoomDirection::In,
    ));
    let zoom_after_scroll = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.zoom,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(zoom_after_scroll > zoom_before_scroll);
    assert_eq!(zoom_after_scroll, 1.25);
}

#[test]
fn mermaid_clicking_visible_owner_label_focuses_it() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    app.render(&mut renderer, layout);

    let beta = find_text_position(&renderer, "Beta Node").expect("Beta Node overlay");
    let center_before = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: beta.0,
            row: beta.1,
            modifiers: KeyModifiers::NONE,
        },
    ));

    let (focus_status, focused_source_index, center_after) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            viewer.focused_source_index,
            (viewer.center_x, viewer.center_y),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Beta Node"));
    assert!(focused_source_index.is_some());
    assert_ne!(center_after, center_before);
}

// ── render_thought_config_editor coverage ────────────────────────────────────
