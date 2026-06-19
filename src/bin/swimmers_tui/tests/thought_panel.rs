use super::*;

fn remote_target(id: &str, label: &str) -> swimmers::types::LaunchTargetSummary {
    swimmers::types::LaunchTargetSummary {
        id: id.to_string(),
        label: label.to_string(),
        kind: "swimmers_api".to_string(),
        base_url: Some("http://127.0.0.1:3210".to_string()),
        auth_token_env: None,
        path_mappings: Vec::new(),
    }
}

fn with_remote_environment(
    mut session: SessionSummary,
    target: &swimmers::types::LaunchTargetSummary,
    remote_session_id: &str,
) -> SessionSummary {
    session.environment = swimmers::types::SessionEnvironmentSummary::remote(
        target,
        remote_session_id,
        "/srv/remote/repos/swimmers",
        Some(TEST_REPO_SWIMMERS.to_string()),
        "remote_swimmers_api",
    );
    session
}

fn ssh_only_environment(id: &str, label: &str) -> swimmers::types::EnvironmentSummary {
    swimmers::types::EnvironmentSummary {
        id: id.to_string(),
        label: label.to_string(),
        kind: "ssh_only".to_string(),
        backend_mode: "ssh_handoff".to_string(),
        display_host: label.to_string(),
        capabilities: swimmers::types::EnvironmentCapabilitySummary::ssh_handoff(true),
        base_url: None,
        auth: swimmers::types::EnvironmentAuthSummary {
            mode: "none".to_string(),
            token_env_present: None,
        },
        path_mapping_count: 0,
        ssh_alias: Some(id.to_string()),
        attach_hint: Some(format!("ssh {id}")),
        bootstrap_hint: Some(format!("ssh {id} 'swimmers serve'")),
        status: swimmers::types::DependencyHealthStatus::NotConfigured,
        last_seen_at: None,
        last_error_at: None,
        last_error: None,
        freshness_ms: None,
        advisory: Vec::new(),
    }
}

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
fn thought_panel_keeps_send_action_off_mixed_local_remote_batch_groups() {
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
    let target = remote_target("jeremy-skillbox", "Jeremy Skillbox");
    let remote = with_batch(
        with_remote_environment(
            sleeping_session_with_thought(
                &remote_sessions::namespace_session_id("jeremy-skillbox", "sess-remote"),
                "[Jeremy] 9",
                TEST_REPO_SKILLS,
                "checking docs",
                "2026-03-08T14:00:06Z",
            ),
            &target,
            "sess-remote",
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
fn thought_panel_shows_send_action_for_same_target_remote_batch_groups() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let target = remote_target("jeremy-skillbox", "Jeremy Skillbox");
    let first = with_batch(
        with_remote_environment(
            sleeping_session_with_thought(
                &remote_sessions::namespace_session_id("jeremy-skillbox", "sess-a"),
                "[Jeremy] 7",
                TEST_REPO_SWIMMERS,
                "patching rail",
                "2026-03-08T14:00:05Z",
            ),
            &target,
            "sess-a",
        ),
        "batch-remote",
        "remote-school",
        0,
        2,
    );
    let second = with_batch(
        with_remote_environment(
            sleeping_session_with_thought(
                &remote_sessions::namespace_session_id("jeremy-skillbox", "sess-b"),
                "[Jeremy] 9",
                TEST_REPO_SKILLS,
                "checking docs",
                "2026-03-08T14:00:06Z",
            ),
            &target,
            "sess-b",
        ),
        "batch-remote",
        "remote-school",
        1,
        2,
    );
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows[0].line, "v remote-school (2) [send]");
    assert!(panel.rows[0].send_rect.is_some());
    assert_eq!(
        panel.rows[0].group_session_ids,
        Some(vec![
            remote_sessions::namespace_session_id("jeremy-skillbox", "sess-a"),
            remote_sessions::namespace_session_id("jeremy-skillbox", "sess-b"),
        ])
    );
}

#[test]
fn thought_panel_hides_send_action_for_mixed_nested_remote_targets() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    app.thought_group_by = ThoughtGroupBy::Batch;

    let west = remote_target("zone::west", "West Zone");
    let east = remote_target("zone::east", "East Zone");
    let first = with_batch(
        with_remote_environment(
            sleeping_session_with_thought(
                &remote_sessions::namespace_session_id("zone::west", "sess-a"),
                "[West] 7",
                TEST_REPO_SWIMMERS,
                "patching rail",
                "2026-03-08T14:00:05Z",
            ),
            &west,
            "sess-a",
        ),
        "batch-remote",
        "remote-school",
        0,
        2,
    );
    let second = with_batch(
        with_remote_environment(
            sleeping_session_with_thought(
                &remote_sessions::namespace_session_id("zone::east", "sess-b"),
                "[East] 9",
                TEST_REPO_SKILLS,
                "checking docs",
                "2026-03-08T14:00:06Z",
            ),
            &east,
            "sess-b",
        ),
        "batch-remote",
        "remote-school",
        1,
        2,
    );
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

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
    let mut app = make_app(api);
    app.thought_show_all = false;

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
    let mut app = make_app(api);
    app.thought_show_all = false;

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
fn thought_panel_header_summarizes_cross_host_inbox() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = App::new(test_runtime(), api);

    let local = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "local is working",
        "2026-03-08T14:00:05Z",
    );
    let mut remote = session_summary_with_thought(
        &remote_sessions::namespace_session_id("skillbox", "sess-remote"),
        "9",
        "/srv/skillbox/repos/swimmers",
        "remote needs review",
        "2026-03-08T14:00:06Z",
    );
    remote.state = SessionState::Attention;
    remote.environment = swimmers::types::SessionEnvironmentSummary::remote(
        &LaunchTargetSummary {
            id: "skillbox".to_string(),
            label: "Skillbox devbox".to_string(),
            kind: "swimmers_api".to_string(),
            base_url: None,
            auth_token_env: None,
            path_mappings: Vec::new(),
        },
        "sess-remote",
        "/srv/skillbox/repos/swimmers".to_string(),
        Some(TEST_REPO_SWIMMERS.to_string()),
        "remote_swimmers_api",
    );

    app.capture_thought_updates(
        &[local.clone(), remote.clone()],
        layout.thought_entry_capacity(),
    );
    app.entities = vec![
        SessionEntity::new(local, layout.overview_field),
        SessionEntity::new(remote, layout.overview_field),
    ];

    assert_eq!(
        thought_panel_header(&app),
        "clawgs / pwd / asleep · 0/2 asleep · > all · fleet 2 hosts / 1 project · inbox 1"
    );

    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let rows = panel
        .rows
        .iter()
        .map(|row| row.line.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].starts_with("[attn]"));
    assert!(rows[0].contains("Skillbox devbox"));
    assert_eq!(rows[1], "  remote needs review");
}

#[test]
fn thought_panel_marks_advisory_metadata_as_external_and_stale() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let mut session = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "x",
        "2026-03-08T14:00:05Z",
    );
    session.environment.advisory = vec![swimmers::types::AdvisoryMetadataSummary {
        source: "c0".to_string(),
        label: "c0".to_string(),
        value: "w".to_string(),
        status: "external".to_string(),
        stale: true,
    }];

    app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
    app.entities = vec![SessionEntity::new(session, layout.overview_field)];

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec!["[work] [swimmers/7] codex", "  x · external c0: w stale"]
    );
    assert_eq!(
        thought_panel_header(&app),
        "clawgs / pwd / all · 0/1 asleep · > asleep · fleet 1 host / 1 project · ext 1"
    );
}

#[test]
fn thought_panel_header_counts_zero_session_ssh_only_environments() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    let local = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "local is working",
        "2026-03-08T14:00:05Z",
    );
    app.capture_thought_updates(&[local.clone()], layout.thought_entry_capacity());
    app.entities = vec![SessionEntity::new(local, layout.overview_field)];
    app.environments = vec![
        swimmers::types::EnvironmentSummary::local(),
        ssh_only_environment("skillbox-devbox", "Skillbox devbox"),
    ];

    assert_eq!(
        thought_panel_header(&app),
        "clawgs / pwd / all · 0/1 asleep · > asleep · fleet 2 hosts / 1 project · 1 handoff"
    );
}

#[test]
fn header_filter_strip_exposes_zero_session_ssh_only_targets_without_fake_sessions() {
    let api = MockApi::new();
    let layout = test_layout(160, 32);
    let mut app = make_app(api);

    let local = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "local is working",
        "2026-03-08T14:00:05Z",
    );
    app.capture_thought_updates(&[local.clone()], layout.thought_entry_capacity());
    app.entities = vec![SessionEntity::new(local.clone(), layout.overview_field)];
    app.environments = vec![
        swimmers::types::EnvironmentSummary::local(),
        ssh_only_environment("skillbox-devbox", "Skillbox devbox"),
    ];

    let strip = build_header_filter_layout(&app, 160);
    let chip = strip
        .chips
        .iter()
        .find(|chip| chip.label == "host:Skillbox devbox ssh/bootstrap/external")
        .expect("ssh-only target chip");
    assert_eq!(
        header_filter_action_at(&app, 160, chip.rect.x, chip.rect.y),
        Some(ThoughtPanelAction::FilterByFleet(ThoughtFleetFilter {
            kind: swimmers::types::FleetLensBucketKind::Target,
            key: "skillbox-devbox".to_string(),
            label: "Skillbox devbox".to_string(),
        }))
    );

    app.thought_filter.fleet = Some(ThoughtFleetFilter {
        kind: swimmers::types::FleetLensBucketKind::Target,
        key: "skillbox-devbox".to_string(),
        label: "Skillbox devbox".to_string(),
    });
    assert!(!app.thought_filter.matches_session(&local));
    let active_strip = build_header_filter_layout(&app, 160);
    assert!(active_strip.chips.iter().any(|chip| chip.label == "host ."));
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
