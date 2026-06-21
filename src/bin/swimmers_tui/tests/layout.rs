use super::*;

#[test]
fn wide_layout_enables_global_thought_rail() {
    let layout = test_layout(120, 32);

    assert!(layout.thought_box.is_some());
    assert!(layout.thought_content.is_some());
    assert!(layout.thought_entry_capacity() > 0);
    assert!(layout.overview_box.x > layout.workspace_box.x);
}

#[test]
fn app_layout_hides_thought_rail_until_a_session_needs_input() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let layout = app.layout_for_terminal(120, 32);

    assert!(layout.thought_box.is_none());
    assert_eq!(layout.overview_box, layout.workspace_box);

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

    let attention_layout = app.layout_for_terminal(120, 32);
    assert!(attention_layout.thought_box.is_some());
    assert!(attention_layout.overview_box.x > attention_layout.workspace_box.x);
}

#[test]
fn app_layout_shows_thought_rail_for_remote_attention_inbox() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let full_layout = WorkspaceLayout::for_terminal_without_thought_panel(120, 32);
    let mut remote = session_summary_with_thought(
        &remote_sessions::namespace_session_id("skillbox", "sess-remote"),
        "9",
        "/srv/skillbox/repos/swimmers",
        "remote needs review",
        "2026-03-08T14:00:06Z",
    );
    remote.state = SessionState::Attention;
    remote.rest_state = RestState::Drowsy;
    remote.environment = swimmers::types::SessionEnvironmentSummary::remote(
        &LaunchTargetSummary {
            id: "skillbox".to_string(),
            label: "Skillbox devbox".to_string(),
            kind: "swimmers_api".to_string(),
            base_url: None,
            auth_token_env: None,
            ssh_alias: None,
            remote_attach_command_template: None,
            bootstrap_hint: None,
            path_mappings: Vec::new(),
        },
        "sess-remote",
        "/srv/skillbox/repos/swimmers".to_string(),
        Some(TEST_REPO_SWIMMERS.to_string()),
        "remote_swimmers_api",
    );

    app.merge_sessions(vec![remote], full_layout.overview_field);

    let layout = app.layout_for_terminal(120, 32);
    assert!(layout.thought_box.is_some());
    assert!(layout.overview_box.x > layout.workspace_box.x);
}

#[test]
fn app_layout_shows_thought_rail_for_sendable_local_batch() {
    let api = MockApi::new();
    let mut app = App::new(test_runtime(), api);
    let full_layout = WorkspaceLayout::for_terminal_without_thought_panel(120, 32);
    let first = with_batch(
        sleeping_session(
            "sess-alpha",
            "alpha",
            TEST_REPO_ALPHA,
            "2026-03-08T14:00:05Z",
        ),
        "batch-ui",
        "ui-discovery",
        0,
        2,
    );
    let second = with_batch(
        sleeping_session("sess-beta", "beta", TEST_REPO_BETA, "2026-03-08T14:00:06Z"),
        "batch-ui",
        "ui-discovery",
        1,
        2,
    );

    app.merge_sessions(vec![first, second], full_layout.overview_field);

    let layout = app.layout_for_terminal(120, 32);
    assert!(layout.thought_box.is_some());
    assert!(layout.overview_box.x > layout.workspace_box.x);
    let thought_content = layout
        .thought_content
        .expect("sendable batch opens thought rail");
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v ui-discovery (2) [send]");
    let send_rect = panel.rows[0].send_rect.expect("send badge");
    let header_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    match thought_panel_action_at(
        &app,
        thought_content,
        layout.thought_entry_capacity(),
        send_rect.x,
        header_y,
    ) {
        Some(ThoughtPanelAction::SendGroup { session_ids, label }) => {
            assert_eq!(label, "ui-discovery");
            assert_eq!(
                session_ids,
                vec!["sess-alpha".to_string(), "sess-beta".to_string()]
            );
        }
        _ => panic!("expected clickable send action"),
    }
}

#[test]
fn app_layout_shows_thought_rail_setup_hint_when_clawgs_is_unavailable() {
    let api = MockApi::new();
    let mut app = App::new(test_runtime(), api);
    app.daemon_defaults_status = DaemonDefaultsStatus::Unavailable;

    let layout = app.layout_for_terminal(120, 32);
    assert!(layout.thought_box.is_some());

    let thought_content = layout
        .thought_content
        .expect("unavailable clawgs status should show thought rail");
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel.empty_message.as_deref(),
        Some("clawgs unavailable - press t to configure")
    );
}

#[test]
fn app_layout_hides_thought_rail_again_after_sleeping_session_wakes() {
    let api = MockApi::new();
    let mut app = make_app(api);
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
    assert!(app.layout_for_terminal(120, 32).thought_box.is_some());

    app.merge_sessions(
        vec![session_summary_with_thought(
            "sess-sleeping",
            "7",
            TEST_REPO_SWIMMERS,
            "patching rail",
            "2026-03-08T14:00:05Z",
        )],
        full_layout.overview_field,
    );

    let layout = app.layout_for_terminal(120, 32);
    assert!(layout.thought_box.is_none());
    assert_eq!(layout.overview_box, layout.workspace_box);
}

#[test]
fn group_composer_keeps_thought_rail_and_draft_stable_when_targets_wake() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    let full_layout = WorkspaceLayout::for_terminal_without_thought_panel(120, 32);
    let first = with_batch(
        sleeping_session(
            "sess-swimmers",
            "7",
            TEST_REPO_SWIMMERS,
            "2026-03-08T14:00:05Z",
        ),
        "batch-ui",
        "ui-discovery",
        0,
        2,
    );
    let second = with_batch(
        sleeping_session("sess-skills", "9", TEST_REPO_SKILLS, "2026-03-08T14:00:06Z"),
        "batch-ui",
        "ui-discovery",
        1,
        2,
    );
    app.merge_sessions(vec![first, second], full_layout.overview_field);
    let initial_layout = app.layout_for_terminal(120, 32);
    assert!(initial_layout.thought_box.is_some());

    app.open_group_input_request(
        vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
        "ui-discovery".to_string(),
    );
    for ch in "stay put".chars() {
        app.handle_initial_request_key(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            initial_layout.overview_field,
        );
    }

    let active_first = with_batch(
        session_summary_with_thought(
            "sess-swimmers",
            "7",
            TEST_REPO_SWIMMERS,
            "running the next command",
            "2026-03-08T14:01:05Z",
        ),
        "batch-ui",
        "ui-discovery",
        0,
        2,
    );
    let active_second = with_batch(
        session_summary_with_thought(
            "sess-skills",
            "9",
            TEST_REPO_SKILLS,
            "also running",
            "2026-03-08T14:01:06Z",
        ),
        "batch-ui",
        "ui-discovery",
        1,
        2,
    );
    app.merge_sessions(
        vec![active_first, active_second],
        initial_layout.overview_field,
    );

    let draft_layout = app.layout_for_terminal(120, 32);
    assert_eq!(draft_layout.thought_box, initial_layout.thought_box);
    assert_eq!(draft_layout.overview_field, initial_layout.overview_field);
    assert_eq!(
        thought_panel_header(&app),
        "clawgs / group draft · ui-discovery · 2 targets"
    );
    let thought_content = draft_layout
        .thought_content
        .expect("group composer pins thought rail");
    let panel = build_thought_panel(&app, thought_content, draft_layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v ui-discovery (2)");
    assert!(panel.rows[0].send_rect.is_none());
    assert!(panel
        .rows
        .iter()
        .any(|row| row.line.contains("[swimmers/7]")));
    assert!(panel.rows.iter().any(|row| row.line.contains("[skills/9]")));
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|request| request.value.as_str()),
        Some("stay put")
    );

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, draft_layout);
    assert!(
        find_text_position(&renderer, "stay put").is_some(),
        "draft text should remain visible after refresh wakes targets"
    );

    app.handle_initial_request_key(
        KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
        draft_layout.overview_field,
    );
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|request| request.value.as_str()),
        Some("stay put!")
    );
    assert!(api.open_attention_group_calls().is_empty());
}

#[test]
fn narrow_layout_keeps_single_overview_field() {
    let layout = test_layout(96, 24);

    assert!(layout.thought_box.is_none());
    assert!(layout.thought_content.is_none());
    assert_eq!(layout.thought_entry_capacity(), 0);
    assert_eq!(layout.overview_box.x, layout.workspace_box.x);
    assert_eq!(layout.overview_field, layout.workspace_box.inset(1));
}

#[test]
fn custom_split_ratio_changes_thought_rail_width() {
    let default_layout = test_layout(120, 32);
    let wider_layout = test_layout_with_ratio(120, 32, 0.5);

    assert_eq!(
        default_layout.split_divider.map(|divider| divider.width),
        Some(THOUGHT_RAIL_GAP)
    );
    assert!(
        wider_layout
            .thought_box
            .expect("wide layout should include thought rail")
            .width
            > default_layout
                .thought_box
                .expect("default layout should include thought rail")
                .width
    );
    assert!(
        wider_layout.overview_field.width < default_layout.overview_field.width,
        "widening the clawgs rail should shrink the swimmers field"
    );
}

#[test]
fn divider_drag_updates_thought_rail_ratio() {
    let api = MockApi::new();
    let mut app = make_app(api);
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
    let initial_layout = app.layout_for_terminal(120, 32);
    let initial_width = initial_layout
        .thought_box
        .expect("wide layout should include thought rail")
        .width;
    let divider = initial_layout
        .split_divider
        .expect("wide layout should expose a divider");
    let hitbox = initial_layout
        .split_hitbox
        .expect("wide layout should expose a divider hitbox");
    assert!(hitbox.contains(divider.x, divider.y));

    assert!(app.start_split_drag(initial_layout, divider.x));
    assert!(app.split_drag_active);
    assert!(app.drag_split(initial_layout, divider.x + 10));

    let dragged_layout = app.layout_for_terminal(120, 32);
    let dragged_width = dragged_layout
        .thought_box
        .expect("dragged layout should include thought rail")
        .width;
    assert!(dragged_width > initial_width);

    app.stop_split_drag();
    assert!(!app.split_drag_active);
}
