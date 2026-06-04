use super::*;

#[test]
fn thought_panel_groups_by_pwd_by_default() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let swimmers = session_summary_with_thought(
        "sess-swimmers",
        "7",
        TEST_REPO_SWIMMERS,
        "patching rail",
        "2026-03-08T14:00:05Z",
    );
    let skills = session_summary_with_thought(
        "sess-skills",
        "9",
        TEST_REPO_SKILLS,
        "checking docs",
        "2026-03-08T14:00:06Z",
    );

    app.capture_thought_updates(&[swimmers, skills], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(app.thought_group_by, ThoughtGroupBy::Pwd);
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "v swimmers (1)",
            "[work] [swimmers/7] codex",
            "  patching rail",
            "v skills (1)",
            "[work] [skills/9] codex",
            "  checking docs",
        ]
    );
}

#[test]
fn thought_panel_places_plans_action_on_matching_pwd_group_header() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.cached_plans = vec![PlanPanelEntry {
        slug: "agent-billing".to_string(),
        client_label: "swimmers".to_string(),
        kind: "released".to_string(),
        schema_path: "/tmp/plans/agent-billing/schema.mmd".to_string(),
    }];

    let session = session_summary_with_thought(
        "sess-swimmers",
        "7",
        TEST_REPO_SWIMMERS,
        "patching rail",
        "2026-03-08T14:00:05Z",
    );
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "v swimmers (1) [plans]",
            "[work] [swimmers/7] codex",
            "  patching rail",
        ]
    );

    let plan_rect = panel.rows[0].plan_rect.expect("plans badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            plan_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenPlanFromDisk {
            schema_path: "/tmp/plans/agent-billing/schema.mmd".to_string(),
            slug: "agent-billing".to_string(),
        })
    );

    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );
    assert_eq!(cell_at(&renderer, plan_rect.x, row_y).ch, '[');
    assert_eq!(cell_at(&renderer, plan_rect.x, row_y).fg, Color::Cyan);
}

#[test]
fn thought_panel_places_plans_action_on_pwd_header_for_live_plan_artifact() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    let schema_path = "/tmp/repos/swimmers/plans/draft/agent_billing/schema.mmd";

    let session = session_summary_with_thought(
        "sess-swimmers",
        "7",
        TEST_REPO_SWIMMERS,
        "done implementing agent billing",
        "2026-03-08T14:00:05Z",
    );
    app.mermaid_artifacts.insert(
        "sess-swimmers".to_string(),
        mermaid_artifact(
            "sess-swimmers",
            schema_path,
            "2026-03-08T14:00:06Z",
            "graph TD\nA-->B\n",
        ),
    );
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v swimmers (1) [plans]");
    let plan_rect = panel.rows[0].plan_rect.expect("plans badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            plan_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenPlanFromDisk {
            schema_path: schema_path.to_string(),
            slug: "agent_billing".to_string(),
        })
    );
}

#[test]
fn thought_panel_keeps_plans_action_off_batch_group_headers() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;
    app.cached_plans = vec![PlanPanelEntry {
        slug: "agent-billing".to_string(),
        client_label: "swimmers".to_string(),
        kind: "released".to_string(),
        schema_path: "/tmp/plans/agent-billing/schema.mmd".to_string(),
    }];

    let session = with_batch(
        session_summary_with_thought(
            "sess-swimmers",
            "7",
            TEST_REPO_SWIMMERS,
            "patching rail",
            "2026-03-08T14:00:05Z",
        ),
        "batch-billing",
        "agent-billing",
        0,
        1,
    );
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v agent-billing (1)");
    assert!(panel.rows[0].plan_rect.is_none());
}

#[test]
fn thought_panel_keeps_send_action_off_unbatched_batch_group() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let first = sleeping_session_with_thought(
        "sess-swimmers",
        "7",
        TEST_REPO_SWIMMERS,
        "patching rail",
        "2026-03-08T14:00:05Z",
    );
    let second = sleeping_session_with_thought(
        "sess-skills",
        "9",
        TEST_REPO_SKILLS,
        "checking docs",
        "2026-03-08T14:00:06Z",
    );
    let batched = with_batch(
        session_summary_with_thought(
            "sess-alpha",
            "2",
            TEST_REPO_ALPHA,
            "different batch",
            "2026-03-08T14:00:07Z",
        ),
        "batch-alpha",
        "alpha-work",
        0,
        1,
    );
    app.capture_thought_updates(&[first, second, batched], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let unbatched_row = panel
        .rows
        .iter()
        .find(|row| row.line == "v unbatched (2)")
        .expect("unbatched header");
    assert!(unbatched_row.send_rect.is_none());
}

#[test]
fn thought_panel_keeps_send_action_off_remote_batch_groups() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let local = with_batch(
        sleeping_session_with_thought(
            "sess-local",
            "7",
            TEST_REPO_SWIMMERS,
            "patching rail",
            "2026-03-08T14:00:05Z",
        ),
        "batch-remote",
        "remote-school",
        0,
        2,
    );
    let remote = with_batch(
        sleeping_session_with_thought(
            &remote_sessions::namespace_session_id("jeremy-skillbox", "sess-remote"),
            "[Jeremy] 9",
            TEST_REPO_SKILLS,
            "checking docs",
            "2026-03-08T14:00:06Z",
        ),
        "batch-remote",
        "remote-school",
        1,
        2,
    );
    app.capture_thought_updates(&[local, remote], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v remote-school (2)");
    assert!(panel.rows[0].send_rect.is_none());
}

#[test]
fn thought_panel_keeps_send_action_off_active_batch_groups() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let first = with_batch(
        session_summary_with_thought(
            "sess-active",
            "7",
            TEST_REPO_SWIMMERS,
            "still working",
            "2026-03-08T14:00:05Z",
        ),
        "batch-active",
        "active-school",
        0,
        2,
    );
    let second = with_batch(
        {
            let mut session = session_summary_with_thought(
                "sess-drowsy",
                "9",
                TEST_REPO_SKILLS,
                "not waiting yet",
                "2026-03-08T14:00:06Z",
            );
            session.rest_state = RestState::Drowsy;
            session
        },
        "batch-active",
        "active-school",
        1,
        2,
    );
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v active-school (2)");
    assert!(panel.rows[0].send_rect.is_none());
}

#[test]
fn thought_panel_keeps_send_action_off_groups_without_two_ready_targets() {
    let api = MockApi::new();
    let layout = test_layout(160, 80);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let stale_ready = with_batch(
        sleeping_session_with_thought(
            "sess-stale-ready",
            "1",
            TEST_REPO_SWIMMERS,
            "waiting",
            "2026-03-08T14:00:01Z",
        ),
        "batch-stale",
        "stale-school",
        0,
        2,
    );
    let stale_bad = with_batch(
        {
            let mut session = sleeping_session_with_thought(
                "sess-stale-bad",
                "2",
                TEST_REPO_SKILLS,
                "stale",
                "2026-03-08T14:00:02Z",
            );
            session.is_stale = true;
            session
        },
        "batch-stale",
        "stale-school",
        1,
        2,
    );
    let disconnected_ready = with_batch(
        sleeping_session_with_thought(
            "sess-disconnected-ready",
            "3",
            TEST_REPO_SWIMMERS,
            "waiting",
            "2026-03-08T14:00:03Z",
        ),
        "batch-disconnected",
        "disconnected-school",
        0,
        2,
    );
    let disconnected_bad = with_batch(
        {
            let mut session = sleeping_session_with_thought(
                "sess-disconnected-bad",
                "4",
                TEST_REPO_SKILLS,
                "disconnected",
                "2026-03-08T14:00:04Z",
            );
            session.transport_health = TransportHealth::Disconnected;
            session
        },
        "batch-disconnected",
        "disconnected-school",
        1,
        2,
    );
    let degraded_ready = with_batch(
        sleeping_session_with_thought(
            "sess-degraded-ready",
            "4a",
            TEST_REPO_SWIMMERS,
            "waiting",
            "2026-03-08T14:00:04Z",
        ),
        "batch-degraded",
        "degraded-school",
        0,
        2,
    );
    let degraded_bad = with_batch(
        {
            let mut session = sleeping_session_with_thought(
                "sess-degraded-bad",
                "4b",
                TEST_REPO_SKILLS,
                "degraded",
                "2026-03-08T14:00:04Z",
            );
            session.transport_health = TransportHealth::Degraded;
            session
        },
        "batch-degraded",
        "degraded-school",
        1,
        2,
    );
    let exited_ready = with_batch(
        sleeping_session_with_thought(
            "sess-exited-ready",
            "5",
            TEST_REPO_SWIMMERS,
            "waiting",
            "2026-03-08T14:00:05Z",
        ),
        "batch-exited",
        "exited-school",
        0,
        2,
    );
    let exited_bad = with_batch(
        {
            let mut session = sleeping_session_with_thought(
                "sess-exited-bad",
                "6",
                TEST_REPO_SKILLS,
                "exited",
                "2026-03-08T14:00:06Z",
            );
            session.state = SessionState::Exited;
            session
        },
        "batch-exited",
        "exited-school",
        1,
        2,
    );
    let deep_ready = with_batch(
        sleeping_session_with_thought(
            "sess-deep-ready",
            "7",
            TEST_REPO_SWIMMERS,
            "waiting",
            "2026-03-08T14:00:07Z",
        ),
        "batch-deep",
        "deep-school",
        0,
        2,
    );
    let deep_bad = with_batch(
        {
            let mut session = sleeping_session_with_thought(
                "sess-deep-bad",
                "8",
                TEST_REPO_SKILLS,
                "deep",
                "2026-03-08T14:00:08Z",
            );
            session.rest_state = RestState::DeepSleep;
            session
        },
        "batch-deep",
        "deep-school",
        1,
        2,
    );
    let active_ready = with_batch(
        sleeping_session_with_thought(
            "sess-active-ready",
            "9",
            TEST_REPO_SWIMMERS,
            "waiting",
            "2026-03-08T14:00:09Z",
        ),
        "batch-one-ready",
        "one-ready-school",
        0,
        2,
    );
    let active_bad = with_batch(
        session_summary_with_thought(
            "sess-active-bad",
            "10",
            TEST_REPO_SKILLS,
            "working",
            "2026-03-08T14:00:10Z",
        ),
        "batch-one-ready",
        "one-ready-school",
        1,
        2,
    );

    app.capture_thought_updates(
        &[
            stale_ready,
            stale_bad,
            disconnected_ready,
            disconnected_bad,
            degraded_ready,
            degraded_bad,
            exited_ready,
            exited_bad,
            deep_ready,
            deep_bad,
            active_ready,
            active_bad,
        ],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    for label in [
        "stale-school",
        "disconnected-school",
        "degraded-school",
        "exited-school",
        "deep-school",
        "one-ready-school",
    ] {
        let header = format!("v {label} (2)");
        let row = panel
            .rows
            .iter()
            .find(|row| row.line == header)
            .unwrap_or_else(|| panic!("missing header {header}"));
        assert!(row.send_rect.is_none(), "{label} should not be sendable");
    }
}

#[test]
fn thought_panel_treats_attention_sessions_as_sendable() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let first = with_batch(
        {
            let mut session = session_summary_with_thought(
                "sess-attention-a",
                "7",
                TEST_REPO_SWIMMERS,
                "needs input",
                "2026-03-08T14:00:05Z",
            );
            session.state = SessionState::Attention;
            session.rest_state = RestState::Drowsy;
            session
        },
        "batch-attention",
        "attention-school",
        0,
        2,
    );
    let second = with_batch(
        {
            let mut session = session_summary_with_thought(
                "sess-attention-b",
                "9",
                TEST_REPO_SKILLS,
                "also needs input",
                "2026-03-08T14:00:06Z",
            );
            session.state = SessionState::Attention;
            session.rest_state = RestState::Active;
            session
        },
        "batch-attention",
        "attention-school",
        1,
        2,
    );
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v attention-school (2) [send]");
    assert_eq!(
        panel.rows[0].group_session_ids.as_deref(),
        Some(
            &[
                "sess-attention-a".to_string(),
                "sess-attention-b".to_string()
            ][..]
        )
    );
}

#[test]
fn thought_panel_defaults_to_stopped_only_and_counts_sleeping_agents() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = App::new(test_runtime(), api);

    let working = session_summary_with_thought(
        "sess-working",
        "7",
        TEST_REPO_SWIMMERS,
        "patching rail",
        "2026-03-08T14:00:05Z",
    );
    let mut stopped = session_summary_with_thought(
        "sess-stopped",
        "9",
        TEST_REPO_SKILLS,
        "went quiet",
        "2026-03-08T14:00:06Z",
    );
    stopped.thought_state = ThoughtState::Sleeping;
    stopped.rest_state = RestState::Sleeping;
    let mut done = session_summary_with_thought(
        "sess-done",
        "3",
        TEST_REPO_ALPHA,
        "finished the batch item",
        "2026-03-08T14:00:07Z",
    );
    done.state = SessionState::Exited;
    done.rest_state = RestState::DeepSleep;

    app.capture_thought_updates(&[working, stopped, done], layout.thought_entry_capacity());

    assert!(!app.thought_show_all);
    assert_eq!(
        thought_panel_header(&app),
        "clawgs / pwd / asleep · 1/3 asleep · > all"
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec!["[asleep] [launch] [skills/9] codex", "  went quiet"]
    );
}

#[test]
fn thought_panel_show_all_toggle_restores_working_agents() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = App::new(test_runtime(), api);

    let working = session_summary_with_thought(
        "sess-working",
        "7",
        TEST_REPO_SWIMMERS,
        "patching rail",
        "2026-03-08T14:00:05Z",
    );
    let mut stopped = session_summary_with_thought(
        "sess-stopped",
        "9",
        TEST_REPO_SKILLS,
        "went quiet",
        "2026-03-08T14:00:06Z",
    );
    stopped.thought_state = ThoughtState::Sleeping;
    stopped.rest_state = RestState::Sleeping;

    app.capture_thought_updates(&[working, stopped], layout.thought_entry_capacity());
    app.toggle_thought_show_all();

    assert!(app.thought_show_all);
    assert_eq!(
        thought_panel_header(&app),
        "clawgs / pwd / all · 1/2 asleep · > asleep"
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "v swimmers (1)",
            "[work] [swimmers/7] codex",
            "  patching rail",
            "v skills (1)",
            "[asleep] [launch] [skills/9] codex",
            "  went quiet",
        ]
    );
}

#[test]
fn clicking_thought_launch_badge_opens_composer_for_selected_launch_target() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.launch_target = Some("jeremy-skillbox".to_string());
    let mut session = session_summary_with_thought(
        "sess-sleeping",
        "9",
        TEST_REPO_SKILLS,
        "needs input",
        "2026-03-08T14:00:06Z",
    );
    session.thought_state = ThoughtState::Sleeping;
    session.rest_state = RestState::Sleeping;
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let launch_rect = panel.rows[0].launch_rect.expect("launch badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: launch_rect.x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );

    let request = app.initial_request.as_ref().expect("composer opened");
    assert_eq!(request.cwd, normalize_path(TEST_REPO_SKILLS));
    assert_eq!(request.launch_target.as_deref(), Some("jeremy-skillbox"));
    assert!(api.create_calls().is_empty());
}

#[test]
fn thought_panel_groups_by_batch_when_toggled() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let first = with_batch(
        session_summary_with_thought(
            "sess-swimmers",
            "7",
            TEST_REPO_SWIMMERS,
            "patching rail",
            "2026-03-08T14:00:05Z",
        ),
        "batch-auth",
        "auth-rebuild",
        0,
        2,
    );
    let second = with_batch(
        session_summary_with_thought(
            "sess-skills",
            "9",
            TEST_REPO_SKILLS,
            "checking docs",
            "2026-03-08T14:00:06Z",
        ),
        "batch-auth",
        "auth-rebuild",
        1,
        2,
    );
    let unbatched = session_summary_with_thought(
        "sess-alpha",
        "2",
        TEST_REPO_ALPHA,
        "routine update",
        "2026-03-08T14:00:07Z",
    );

    app.capture_thought_updates(&[first, second, unbatched], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "v auth-rebuild (2)",
            "[work] [swimmers/7] codex",
            "  patching rail",
            "[work] [skills/9] codex",
            "  checking docs",
            "v unbatched (1)",
            "[work] [alpha/2] codex",
            "  routine update",
        ]
    );
    assert!(panel.rows[0].send_rect.is_none());
}

#[test]
fn clicking_thought_group_send_opens_group_composer() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
    app.thought_group_by = ThoughtGroupBy::Batch;

    let first = with_batch(
        sleeping_session_with_thought(
            "sess-swimmers",
            "7",
            TEST_REPO_SWIMMERS,
            "patching rail",
            "2026-03-08T14:00:05Z",
        ),
        "batch-auth",
        "auth-rebuild",
        0,
        2,
    );
    let second = with_batch(
        sleeping_session_with_thought(
            "sess-skills",
            "9",
            TEST_REPO_SKILLS,
            "checking docs",
            "2026-03-08T14:00:06Z",
        ),
        "batch-auth",
        "auth-rebuild",
        1,
        2,
    );
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let send_rect = panel.rows[0].send_rect.expect("send badge");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    handle_workspace_click(
        &mut app,
        layout,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: send_rect.x,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
    );

    let targets = app.group_input_targets.as_ref().expect("group targets");
    assert_eq!(targets.label, "auth-rebuild");
    assert_eq!(
        targets.session_ids,
        vec!["sess-swimmers".to_string(), "sess-skills".to_string()]
    );
    let request = app.initial_request.as_ref().expect("composer opened");
    assert_eq!(request.cwd, "auth-rebuild");
    assert!(api.create_calls().is_empty());
    assert!(api.create_batch_calls().is_empty());
}

#[test]
fn objective_shift_entries_stay_working_status_badges() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    let mut session = session_summary_with_thought(
        "sess-shift",
        "2",
        TEST_REPO_SWIMMERS,
        "reframed the plan",
        "2026-03-29T14:00:05Z",
    );
    session.objective_changed_at = session.thought_updated_at;

    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let text_rect = panel.rows[0].text_rect.expect("row text");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert_eq!(panel.rows[0].line, "[work] [swimmers/2] codex");
    assert_eq!(panel.rows[1].line, "  reframed the plan");
    assert_eq!(text_rect.x, thought_content.x);
    assert_eq!(cell_at(&renderer, thought_content.x, row_y).ch, '[');
    assert_eq!(
        cell_at(&renderer, thought_content.x, row_y).fg,
        panel.rows[0].color
    );
}

#[test]
fn objective_shift_entries_keep_timestamp_order_in_the_visible_rail() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let mut shift = session_summary_with_thought(
        "sess-shift",
        "2",
        TEST_REPO_ALPHA,
        "reframed the plan",
        "2026-03-29T14:00:05Z",
    );
    shift.objective_changed_at = shift.thought_updated_at;

    let plain = session_summary_with_thought(
        "sess-plain",
        "9",
        TEST_REPO_SWIMMERS,
        "routine update",
        "2026-03-29T14:00:06Z",
    );

    app.capture_thought_updates(&[shift, plain], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let shift_index = panel
        .rows
        .iter()
        .position(|row| row.line == "[work] [alpha/2] codex")
        .expect("shift row");
    let plain_index = panel
        .rows
        .iter()
        .position(|row| row.line == "[work] [swimmers/9] codex")
        .expect("plain row");

    assert!(plain_index > shift_index);
    assert_eq!(
        panel.rows[shift_index].text_rect.expect("shift row text").x,
        thought_content.x
    );
}

#[test]
fn refresh_builds_synthetic_mermaid_row_and_preserves_text_click_behavior() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut session = session_summary("sess-1", "7", TEST_REPO_SWIMMERS);
    session.commit_candidate = true;
    api.push_fetch_sessions(Ok(vec![session]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph TD\nA-->B\n",
    )));
    let mut app = make_app(api);

    app.refresh(layout);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 2);
    assert_eq!(panel.rows[0].line, "[art] [work] [commit] [swimmers/7] ~");
    assert_eq!(panel.rows[1].line, "  artifacts ready");
    let mermaid_rect = panel.rows[0].mermaid_rect.expect("mermaid button");
    let commit_rect = panel.rows[0].commit_rect.expect("commit badge");
    let session_rect = panel.rows[0]
        .session_rect
        .expect("synthetic row session label");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            mermaid_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenMermaid("sess-1".to_string()))
    );
    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            commit_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::LaunchCommitCodex("sess-1".to_string()))
    );
    assert_eq!(
        thought_panel_action_at(
            &app,
            thought_content,
            layout.thought_entry_capacity(),
            session_rect.x,
            row_y,
        ),
        Some(ThoughtPanelAction::OpenSession {
            session_id: "sess-1".to_string(),
            label: "swimmers/7".to_string(),
        })
    );
}

#[test]
fn tagged_thought_rows_render_metadata_above_full_width_body_with_matching_color() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    let mut session = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching the clawgs rail layout",
        "2026-03-29T14:00:05Z",
    );
    session.repo_theme_id = Some("/tmp/swimmers".to_string());
    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    app.merge_sessions(vec![session.clone()], layout.overview_field);
    app.mermaid_artifacts.insert(
        session.session_id.clone(),
        mermaid_artifact(
            &session.session_id,
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-29T14:00:06Z",
            "graph TD\nA-->B\n",
        ),
    );
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "[art] [work] [swimmers/7] codex",
            "  patching the clawgs rail layout",
        ]
    );

    let mermaid_rect = panel.rows[0].mermaid_rect.expect("mermaid button");
    let row_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert_eq!(
        cell_at(&renderer, mermaid_rect.x, row_y).fg,
        panel.rows[0].color
    );
    assert_eq!(
        cell_at(
            &renderer,
            panel.rows[1].text_rect.expect("thought body").x,
            row_y + 1
        )
        .fg,
        panel.rows[1].color
    );
}

#[test]
fn thought_panel_detail_lines_preserve_command_and_status_fallbacks() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let mut command = session_summary("sess-command", "7", TEST_REPO_SWIMMERS);
    command.current_command = Some(" cargo   test --bin swimmers-tui ".to_string());
    command.rest_state = RestState::Active;

    let mut stale = session_summary("sess-stale", "9", TEST_REPO_SKILLS);
    stale.is_stale = true;
    stale.rest_state = RestState::Sleeping;
    stale.transport_health = TransportHealth::Disconnected;

    let mut exited = session_summary("sess-exited", "3", TEST_REPO_ALPHA);
    exited.state = SessionState::Exited;
    exited.rest_state = RestState::Active;

    let mut sleeping = session_summary("sess-sleeping", "5", TEST_REPO_BETA);
    sleeping.rest_state = RestState::DeepSleep;

    app.merge_sessions(
        vec![
            command.clone(),
            stale.clone(),
            exited.clone(),
            sleeping.clone(),
        ],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[command, stale, exited, sleeping],
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let lines = panel
        .rows
        .iter()
        .map(|row| row.line.as_str())
        .collect::<Vec<_>>();

    assert!(lines.contains(&"  cmd: cargo test --bin swimmers-tui"));
    assert!(lines.contains(&"  stale session"));
    assert!(lines.contains(&"  no daemon"));
    assert!(lines.contains(&"  sleeping"));
}

#[test]
fn thought_wrap_text_preserves_word_long_word_and_wide_char_breaks() {
    let lines = |values: &[&str]| {
        values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    };

    assert_eq!(wrap_text("", 8), lines(&[""]));
    assert_eq!(wrap_text("   ", 8), lines(&[""]));
    assert_eq!(wrap_text("anything", 0), Vec::<String>::new());
    assert_eq!(
        wrap_text("alpha beta gamma", 10),
        lines(&["alpha", "beta gamma"])
    );
    assert_eq!(
        wrap_text("superlongword", 5),
        lines(&["super", "longw", "ord"])
    );
    assert_eq!(wrap_text("表a", 1), lines(&["表", "a"]));
}
