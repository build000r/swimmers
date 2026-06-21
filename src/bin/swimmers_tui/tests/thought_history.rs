use super::*;

#[test]
fn refresh_keeps_latest_thought_per_session_in_timestamp_order() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-2",
            "beta",
            TEST_REPO_BETA,
            "indexing repo",
            "2026-03-08T14:00:05Z",
        ),
        session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
            "writing tests",
            "2026-03-08T14:00:06Z",
        ),
    ]));
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-2",
            "beta",
            TEST_REPO_BETA,
            "indexing repo",
            "2026-03-08T14:00:05Z",
        ),
        session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
            "patching sidebar",
            "2026-03-08T14:00:07Z",
        ),
    ]));
    let mut app = make_app(api);

    app.refresh(layout);
    app.refresh(layout);

    assert_eq!(
        app.thought_log
            .iter()
            .map(|entry| (entry.session_id.as_str(), entry.thought.as_str()))
            .collect::<Vec<_>>(),
        vec![("sess-2", "indexing repo"), ("sess-1", "patching sidebar"),]
    );
}

#[test]
fn refresh_updates_native_status_label_when_backend_app_changes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)]));
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    }));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Ghostty),
        ghostty_mode: Some(GhosttyOpenMode::Swap),
        app: Some("Ghostty".to_string()),
        reason: None,
    }));
    let mut app = make_app(api);

    app.refresh(layout);
    assert_eq!(app.native_status_text(), "terminal handoff: iTerm");

    app.refresh(layout);
    assert_eq!(app.native_status_text(), "terminal handoff: Ghostty (swap)");
}

#[test]
fn refresh_labels_remote_linux_native_status_as_tmux_attach_fallback() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: false,
        platform: Some("linux".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: Some("native iTerm control is only supported on macOS".to_string()),
    }));
    let mut app = make_app(api);

    app.refresh(layout);

    assert_eq!(
        app.native_status_text(),
        "terminal handoff: tmux attach only"
    );
}

#[test]
fn refresh_ignores_null_duplicate_and_stale_thoughts() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
        "sess-3",
        "gamma",
        TEST_REPO_GAMMA,
        "reading logs",
        "2026-03-08T14:00:05Z",
    )]));

    let mut duplicate = session_summary_with_thought(
        "sess-3",
        "gamma",
        TEST_REPO_GAMMA,
        "reading logs",
        "2026-03-08T14:00:05Z",
    );
    let mut stale = session_summary_with_thought(
        "sess-3",
        "gamma",
        TEST_REPO_GAMMA,
        "reading logs",
        "2026-03-08T14:00:04Z",
    );
    let mut cleared = session_summary("sess-3", "gamma", TEST_REPO_GAMMA);
    duplicate.last_activity_at = timestamp("2026-03-08T14:00:06Z");
    stale.last_activity_at = timestamp("2026-03-08T14:00:07Z");
    cleared.last_activity_at = timestamp("2026-03-08T14:00:08Z");

    api.push_fetch_sessions(Ok(vec![duplicate]));
    api.push_fetch_sessions(Ok(vec![stale]));
    api.push_fetch_sessions(Ok(vec![cleared]));

    let mut app = make_app(api);
    app.refresh(layout);
    app.refresh(layout);
    app.refresh(layout);
    app.refresh(layout);

    assert_eq!(app.thought_log.len(), 1);
    assert_eq!(app.thought_log[0].thought, "reading logs");
}

#[test]
fn selection_changes_do_not_reset_global_thought_timeline() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![
            session_summary("sess-1", "alpha", TEST_REPO_ALPHA),
            session_summary("sess-2", "beta", TEST_REPO_BETA),
        ],
        layout.overview_field,
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-1",
            "alpha",
            TEST_REPO_ALPHA,
            "patching sidebar",
            "2026-03-08T14:00:07Z",
        )],
        layout.thought_entry_capacity(),
    );
    app.selected_id = Some("sess-1".to_string());
    let before = app.thought_log.clone();

    app.move_selection(1, layout.overview_field);

    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
    assert_eq!(app.thought_log, before);
}

#[test]
fn thought_timeline_trims_to_visible_capacity() {
    let api = MockApi::new();
    let layout = test_layout(120, 24);
    let mut app = make_app(api);
    assert_eq!(layout.thought_entry_capacity(), 10);

    for idx in 0..15 {
        let second = idx + 1;
        let updated_at = format!("2026-03-08T14:00:{second:02}Z");
        let thought = format!("thought {idx}");
        let session_id = format!("sess-{idx}");
        let tmux_name = format!("alpha-{idx}");
        let session = session_summary_with_thought(
            &session_id,
            &tmux_name,
            TEST_REPO_ALPHA,
            &thought,
            &updated_at,
        );
        app.capture_thought_updates(&[session], layout.thought_entry_capacity());
    }

    assert_eq!(app.thought_log.len(), 10);
    assert_eq!(
        app.thought_log.first().map(|entry| entry.thought.as_str()),
        Some("thought 5")
    );
    assert_eq!(
        app.thought_log.last().map(|entry| entry.thought.as_str()),
        Some("thought 14")
    );
}

#[test]
fn header_filter_strip_uses_active_sessions_not_trimmed_thought_log() {
    let api = MockApi::new();
    let layout = test_layout(220, 24);
    let mut app = make_app(api);
    assert_eq!(layout.thought_entry_capacity(), 10);

    let sessions = (0..11)
        .map(|idx| {
            let session_id = format!("sess-{idx:02}");
            let tmux_name = format!("{idx:02}");
            let cwd = format!("{TEST_REPOS_ROOT}/r{idx:02}");
            let thought = format!("thought {idx}");
            let updated_at = format!("2026-03-08T14:00:{:02}Z", idx + 1);
            session_summary_with_thought(&session_id, &tmux_name, &cwd, &thought, &updated_at)
        })
        .collect::<Vec<_>>();

    app.merge_sessions(sessions.clone(), layout.overview_field);
    app.capture_thought_updates(&sessions, layout.thought_entry_capacity());

    assert_eq!(app.thought_log.len(), 10);
    assert!(!app
        .thought_log
        .iter()
        .any(|entry| entry.cwd == format!("{TEST_REPOS_ROOT}/r00")));

    let header = build_header_filter_layout(&app, 220);
    let labels = header
        .chips
        .iter()
        .map(|chip| chip.label.clone())
        .collect::<Vec<_>>();
    assert_eq!(labels.len(), 11);
    assert!(labels.contains(&"1xr00".to_string()));
    assert!(labels.contains(&"1xr10".to_string()));
}

#[test]
fn refresh_prunes_exited_sessions_from_thought_timeline_and_header_filter_chips() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        ),
        session_summary_with_thought(
            "sess-2",
            "9",
            TEST_REPO_SKILLS,
            "indexing docs",
            "2026-03-08T14:00:06Z",
        ),
    ]));
    api.push_fetch_sessions(Ok(vec![session_summary_with_thought(
        "sess-2",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:06Z",
    )]));
    let mut app = make_app(api);

    app.refresh(layout);
    let initial_header = build_header_filter_layout(&app, 120);
    assert!(initial_header
        .chips
        .iter()
        .any(|chip| chip.label == "1xswimmers"));
    assert!(initial_header
        .chips
        .iter()
        .any(|chip| chip.label == "1xskills"));

    app.refresh(layout);

    assert_eq!(
        app.thought_log
            .iter()
            .map(|entry| entry.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["sess-2"]
    );

    let header = build_header_filter_layout(&app, 120);
    assert_eq!(
        header
            .chips
            .iter()
            .map(|chip| chip.label.as_str())
            .collect::<Vec<_>>(),
        vec!["1xskills"]
    );
    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec!["[work] [skills/9] codex", "  indexing docs"]
    );
}

#[test]
fn refresh_header_filter_strip_includes_active_repo_without_thought_history() {
    let api = MockApi::new();
    let layout = test_layout(160, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary_with_thought(
            "sess-1",
            "7",
            TEST_REPO_SWIMMERS,
            "patching tui",
            "2026-03-08T14:00:05Z",
        ),
        session_summary("sess-2", "9", TEST_REPO_SKILLS),
    ]));
    let mut app = make_app(api);

    app.refresh(layout);

    let header = build_header_filter_layout(&app, 160);
    let labels = header
        .chips
        .iter()
        .map(|chip| chip.label.clone())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"1xswimmers".to_string()));
    let skills_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist even without thought history")
        .clone();

    app.handle_header_filter_click(160, skills_chip.rect.x, skills_chip.rect.y);

    assert_eq!(app.thought_filter.cwd.as_deref(), Some(TEST_REPO_SKILLS));
}

#[test]
fn render_header_filter_strip_shows_repo_chips_and_thought_rows() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let swimmers_theme_id = "/tmp/swimmers".to_string();
    let skills_theme_id = "/tmp/skills".to_string();
    let swimmers_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    let skills_color = Color::Rgb {
        r: 79,
        g: 166,
        b: 106,
    };
    app.repo_themes
        .insert(swimmers_theme_id.clone(), repo_theme("#B89875"));
    app.repo_themes
        .insert(skills_theme_id.clone(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some(swimmers_theme_id.clone());

    let mut second = session_summary_with_thought(
        "sess-2",
        "2",
        TEST_REPO_SWIMMERS,
        "wiring filter state",
        "2026-03-08T14:00:06Z",
    );
    second.repo_theme_id = Some(swimmers_theme_id);

    let mut third = session_summary_with_thought(
        "sess-3",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    third.repo_theme_id = Some(skills_theme_id);

    app.merge_sessions(
        vec![first.clone(), second.clone(), third.clone()],
        layout.overview_field,
    );
    app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec![
            "v swimmers (2)",
            "[work] [swimmers/7] codex",
            "  patching tui",
            "[work] [swimmers/2] codex",
            "  wiring filter state",
            "v skills (1)",
            "[work] [skills/9] codex",
            "  indexing docs",
        ]
    );

    let header = build_header_filter_layout(&app, 120);
    let swimmers_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "2xswimmers")
        .expect("swimmers chip should exist");
    let skills_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist");
    assert_eq!(swimmers_chip.color, swimmers_color);
    assert_eq!(skills_chip.color, skills_color);

    let mut renderer = test_renderer(120, 32);
    render_header_filter_strip(&app, &mut renderer, 120);

    assert_eq!(
        cell_at(&renderer, swimmers_chip.rect.x, swimmers_chip.rect.y).fg,
        swimmers_color
    );
    assert_eq!(
        cell_at(&renderer, skills_chip.rect.x, skills_chip.rect.y).fg,
        skills_color
    );
    assert!(row_text(&renderer, 2).contains("[filter out]"));
    assert!(row_text(&renderer, 2).ends_with("[filter out]  1xskills  2xswimmers"));
}

#[test]
fn active_repo_header_chip_maps_to_code_open_action() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    let session = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    app.merge_sessions(vec![session.clone()], layout.overview_field);
    app.capture_thought_updates(&[session], layout.thought_entry_capacity());
    app.set_thought_filter_cwd(TEST_REPO_SWIMMERS.to_string());

    let header = build_header_filter_layout(&app, 120);
    let active_chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "code .")
        .expect("active repo chip should expose code dot")
        .clone();

    assert_eq!(
        header_filter_action_at(&app, 120, active_chip.rect.x, active_chip.rect.y),
        Some(ThoughtPanelAction::OpenRepoInEditor(
            TEST_REPO_SWIMMERS.to_string()
        ))
    );
}

#[test]
fn header_filter_strip_and_thought_rows_apply_and_clear_filters() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());

    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    app.repo_themes
        .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some("/tmp/swimmers".to_string());

    let mut second = session_summary_with_thought(
        "sess-2",
        "2",
        TEST_REPO_SWIMMERS,
        "wiring filter state",
        "2026-03-08T14:00:06Z",
    );
    second.repo_theme_id = Some("/tmp/swimmers".to_string());

    let mut third = session_summary_with_thought(
        "sess-3",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    third.repo_theme_id = Some("/tmp/skills".to_string());

    app.merge_sessions(
        vec![first.clone(), second.clone(), third.clone()],
        layout.overview_field,
    );
    app.capture_thought_updates(&[first, second, third], layout.thought_entry_capacity());

    let initial_header = build_header_filter_layout(&app, 120);
    let chip = initial_header
        .chips
        .iter()
        .find(|chip| chip.label == "2xswimmers")
        .expect("swimmers chip should exist")
        .clone();
    app.handle_header_filter_click(120, chip.rect.x, chip.rect.y);

    assert_eq!(app.thought_filter.cwd.as_deref(), Some(TEST_REPO_SWIMMERS));
    assert_eq!(app.active_thought_filter_text(), "filter: pwd=swimmers");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "2"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-2".to_string(), "sess-1".to_string()]
    );

    let filtered_header = build_header_filter_layout(&app, 120);
    let active_chip = filtered_header
        .chips
        .iter()
        .find(|chip| chip.label == "code .")
        .expect("active repo chip should become code dot");
    let dimmed_chip = filtered_header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("inactive repo chip should stay visible");
    assert_eq!(dimmed_chip.color, Color::DarkGrey);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(!row_text(&renderer, 1).contains("filter: pwd"));
    assert_eq!(
        cell_at(&renderer, active_chip.rect.x, active_chip.rect.y).fg,
        active_chip.color
    );
    assert_eq!(
        cell_at(&renderer, dimmed_chip.rect.x, dimmed_chip.rect.y).fg,
        Color::DarkGrey
    );
    assert!(row_text(&renderer, 2).contains("code ."));
    assert!(row_text(&renderer, 2).contains("1xskills"));
    assert!(row_text(&renderer, 2).contains("[filter out]"));
    assert!(row_text(&renderer, 2).contains("[clear filters]"));

    let filtered_panel =
        build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_index = filtered_panel
        .rows
        .iter()
        .position(|row| row.tmux_name == "2")
        .expect("session 2 row should exist");
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(filtered_panel.rows.len() as u16);
    let row_rect = filtered_panel.rows[row_index]
        .session_rect
        .expect("row should have a click target");
    app.selected_id = Some("sess-3".to_string());
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-2".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    app.handle_thought_click(
        row_rect.x.saturating_add(4),
        row_start_y + row_index as u16,
        thought_content,
        layout.thought_entry_capacity(),
    );
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(app.thought_filter.cwd.as_deref(), Some(TEST_REPO_SWIMMERS));
    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: pwd=swimmers");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "2"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-2".to_string(), "sess-1".to_string()]
    );
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
    assert_eq!(api.open_calls(), vec!["sess-2".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/2")
    );

    let cleared_header = build_header_filter_layout(&app, 120);
    let clear_rect = cleared_header
        .clear_filters_rect
        .expect("clear filters button should exist");
    app.handle_header_filter_click(120, clear_rect.x, clear_rect.y);

    assert_eq!(app.thought_filter, ThoughtFilter::default());
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "2", "9"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec![
            "sess-2".to_string(),
            "sess-1".to_string(),
            "sess-3".to_string(),
        ]
    );
}

#[test]
fn header_filter_strip_toggles_filter_out_mode_and_excludes_selected_projects() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    app.repo_themes
        .insert("/tmp/swimmers".to_string(), repo_theme("#B89875"));
    app.repo_themes
        .insert("/tmp/skills".to_string(), repo_theme("#4FA66A"));

    let mut first = session_summary_with_thought(
        "sess-1",
        "7",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    first.repo_theme_id = Some("/tmp/swimmers".to_string());

    let mut second = session_summary_with_thought(
        "sess-2",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    second.repo_theme_id = Some("/tmp/skills".to_string());

    app.merge_sessions(vec![first.clone(), second.clone()], layout.overview_field);
    app.capture_thought_updates(&[first, second], layout.thought_entry_capacity());

    let initial_header = build_header_filter_layout(&app, 120);
    let filter_out_rect = initial_header
        .filter_out_rect
        .expect("filter out toggle should exist");
    assert_eq!(
        header_filter_action_at(&app, 120, filter_out_rect.x, filter_out_rect.y),
        Some(ThoughtPanelAction::ToggleFilterOutMode)
    );

    app.handle_header_filter_click(120, filter_out_rect.x, filter_out_rect.y);

    assert!(app.thought_filter.filter_out_mode);
    assert_eq!(app.active_thought_filter_text(), "filter: none");

    let filter_out_header = build_header_filter_layout(&app, 120);
    let skills_chip = filter_out_header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist")
        .clone();
    assert_eq!(
        header_filter_action_at(&app, 120, skills_chip.rect.x, skills_chip.rect.y),
        Some(ThoughtPanelAction::ToggleFilterOutCwd(
            TEST_REPO_SKILLS.to_string()
        ))
    );

    app.handle_header_filter_click(120, skills_chip.rect.x, skills_chip.rect.y);

    assert!(app.thought_filter.filter_out_mode);
    assert!(app.thought_filter.excluded_cwds.contains(TEST_REPO_SKILLS));
    assert_eq!(app.active_thought_filter_text(), "filter: hide=skills");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7"]
    );
    assert_eq!(visible_entity_ids(&app), vec!["sess-1".to_string()]);

    let excluded_header = build_header_filter_layout(&app, 120);
    let excluded_chip = excluded_header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should stay visible");
    assert_eq!(excluded_chip.color, Color::DarkGrey);

    let clear_rect = excluded_header
        .clear_filters_rect
        .expect("clear filters button should exist");
    app.handle_header_filter_click(120, clear_rect.x, clear_rect.y);

    assert_eq!(app.thought_filter, ThoughtFilter::default());
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.tmux_name.as_str())
            .collect::<Vec<_>>(),
        vec!["7", "9"]
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-1".to_string(), "sess-2".to_string()]
    );
}

#[test]
fn active_thought_filter_text_combines_labels_in_stable_order() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.thought_filter.cwd = Some("/tmp/swimmers".to_string());
    app.thought_filter.tmux_name = Some("7".to_string());
    app.thought_filter
        .excluded_cwds
        .insert("/tmp/zeta".to_string());
    app.thought_filter
        .excluded_cwds
        .insert("/tmp/alpha".to_string());

    assert_eq!(
        app.active_thought_filter_text(),
        "filter: pwd=swimmers, hide=alpha,zeta, num=7"
    );
}

#[test]
fn header_filter_strip_applies_native_fleet_filters() {
    let api = MockApi::new();
    let layout = test_layout(240, 32);
    let mut app = make_app(api);

    let mut local = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "local is working",
        "2026-03-08T14:00:05Z",
    );
    local.state = SessionState::Busy;

    let remote_id = remote_sessions::namespace_session_id("skillbox", "sess-remote");
    let mut remote = session_summary_with_thought(
        &remote_id,
        "9",
        "/srv/skillbox/repos/swimmers",
        "remote needs review",
        "2026-03-08T14:00:06Z",
    );
    remote.state = SessionState::Attention;
    remote.transport_health = TransportHealth::Degraded;
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

    app.capture_thought_updates(
        &[local.clone(), remote.clone()],
        layout.thought_entry_capacity(),
    );
    app.entities = vec![
        SessionEntity::new(local, layout.overview_field),
        SessionEntity::new(remote, layout.overview_field),
    ];

    for (chip_label, filter_text) in [
        ("host:Skillbox devbox", "filter: host=Skillbox devbox"),
        ("state:attention", "filter: state=attention"),
        ("ready:needs attention", "filter: ready=needs attention"),
        ("health:degraded", "filter: health=degraded"),
    ] {
        app.clear_thought_filters();
        let header = build_header_filter_layout(&app, 240);
        let chip = header
            .chips
            .iter()
            .find(|chip| chip.label == chip_label)
            .cloned()
            .unwrap_or_else(|| panic!("missing header chip {chip_label}"));

        app.handle_header_filter_click(240, chip.rect.x, chip.rect.y);

        assert_eq!(app.active_thought_filter_text(), filter_text);
        assert_eq!(visible_entity_ids(&app), vec![remote_id.clone()]);
        assert_eq!(
            app.visible_thought_entries(layout.thought_entry_capacity())
                .into_iter()
                .map(|entry| entry.session_id.clone())
                .collect::<Vec<_>>(),
            vec![remote_id.clone()]
        );
    }
}

#[test]
fn saved_fleet_lens_current_repo_matches_local_and_remote_sessions() {
    let api = MockApi::new();
    let layout = test_layout(240, 32);
    let mut app = make_app(api);

    let local = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "local is working",
        "2026-03-08T14:00:05Z",
    );
    let remote_id = remote_sessions::namespace_session_id("skillbox", "sess-remote");
    let mut remote = session_summary_with_thought(
        &remote_id,
        "9",
        "/srv/skillbox/repos/swimmers",
        "remote is mapped",
        "2026-03-08T14:00:06Z",
    );
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
    let beta = session_summary_with_thought(
        "sess-beta",
        "11",
        TEST_REPO_BETA,
        "other repo",
        "2026-03-08T14:00:07Z",
    );

    app.capture_thought_updates(
        &[local.clone(), remote.clone(), beta.clone()],
        layout.thought_entry_capacity(),
    );
    app.entities = vec![
        SessionEntity::new(local, layout.overview_field),
        SessionEntity::new(remote, layout.overview_field),
        SessionEntity::new(beta, layout.overview_field),
    ];
    app.selected_id = Some("sess-local".to_string());

    assert!(app.set_thought_filter_preset(FleetLensPreset {
        id: "current-repo".to_string(),
        label: "Current repo".to_string(),
        source: "builtin".to_string(),
        matchers: vec![FleetLensPresetMatcher::CurrentRepo],
    }));

    assert_eq!(
        app.active_thought_filter_text(),
        "filter: lens=Current repo"
    );
    assert_eq!(
        visible_entity_ids(&app),
        vec!["sess-local".to_string(), remote_id.clone()]
    );
}

#[test]
fn saved_fleet_lens_capability_matches_thought_entries_and_entities() {
    let api = MockApi::new();
    let layout = test_layout(240, 32);
    let mut app = make_app(api);
    let target = LaunchTargetSummary {
        id: "skillbox".to_string(),
        label: "Skillbox devbox".to_string(),
        kind: "swimmers_api".to_string(),
        base_url: None,
        auth_token_env: None,
        ssh_alias: None,
        remote_attach_command_template: None,
        bootstrap_hint: None,
        path_mappings: Vec::new(),
    };

    let mapped_id = remote_sessions::namespace_session_id("skillbox", "sess-mapped");
    let mut mapped = session_summary_with_thought(
        &mapped_id,
        "9",
        "/srv/skillbox/repos/swimmers",
        "remote dirs are mapped",
        "2026-03-08T14:00:06Z",
    );
    mapped.environment = swimmers::types::SessionEnvironmentSummary::remote(
        &target,
        "sess-mapped",
        "/srv/skillbox/repos/swimmers".to_string(),
        Some(TEST_REPO_SWIMMERS.to_string()),
        "remote_swimmers_api",
    );

    let unmapped_id = remote_sessions::namespace_session_id("skillbox", "sess-unmapped");
    let mut unmapped = session_summary_with_thought(
        &unmapped_id,
        "10",
        "/srv/skillbox/repos/unknown",
        "remote dirs are not mapped",
        "2026-03-08T14:00:07Z",
    );
    unmapped.environment = swimmers::types::SessionEnvironmentSummary::remote(
        &target,
        "sess-unmapped",
        "/srv/skillbox/repos/unknown".to_string(),
        None,
        "remote_swimmers_api",
    );

    app.capture_thought_updates(
        &[mapped.clone(), unmapped.clone()],
        layout.thought_entry_capacity(),
    );
    app.entities = vec![
        SessionEntity::new(mapped, layout.overview_field),
        SessionEntity::new(unmapped, layout.overview_field),
    ];

    assert!(app.set_thought_filter_preset(FleetLensPreset {
        id: "remote-dirs".to_string(),
        label: "Remote dirs".to_string(),
        source: "overlay".to_string(),
        matchers: vec![FleetLensPresetMatcher::Capability {
            key: "remote_dir_inventory".to_string(),
        }],
    }));

    assert_eq!(app.active_thought_filter_text(), "filter: lens=Remote dirs");
    assert_eq!(visible_entity_ids(&app), vec![mapped_id.clone()]);
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.session_id.clone())
            .collect::<Vec<_>>(),
        vec![mapped_id]
    );
}

#[test]
fn observe_capability_lens_keeps_degraded_remote_sessions_visible_without_send() {
    let api = MockApi::new();
    let layout = test_layout(240, 32);
    let mut app = make_app(api);
    let target = LaunchTargetSummary {
        id: "skillbox".to_string(),
        label: "Skillbox devbox".to_string(),
        kind: "swimmers_api".to_string(),
        base_url: None,
        auth_token_env: None,
        ssh_alias: None,
        remote_attach_command_template: None,
        bootstrap_hint: None,
        path_mappings: Vec::new(),
    };

    let remote_id = remote_sessions::namespace_session_id("skillbox", "sess-degraded");
    let mut remote = session_summary_with_thought(
        &remote_id,
        "9",
        "/srv/skillbox/repos/swimmers",
        "cached stale remote session remains visible",
        "2026-03-08T14:00:06Z",
    );
    remote.environment = swimmers::types::SessionEnvironmentSummary::remote(
        &target,
        "sess-degraded",
        "/srv/skillbox/repos/swimmers".to_string(),
        Some(TEST_REPO_SWIMMERS.to_string()),
        "remote_swimmers_api",
    );
    remote.is_stale = true;
    remote.transport_health = swimmers::types::TransportHealth::Degraded;

    app.capture_thought_updates(
        std::slice::from_ref(&remote),
        layout.thought_entry_capacity(),
    );
    app.entities = vec![SessionEntity::new(remote, layout.overview_field)];

    assert!(app.set_thought_filter_preset(FleetLensPreset {
        id: "observe".to_string(),
        label: "Observe".to_string(),
        source: "overlay".to_string(),
        matchers: vec![FleetLensPresetMatcher::Capability {
            key: "observe_sessions".to_string(),
        }],
    }));
    assert_eq!(visible_entity_ids(&app), vec![remote_id.clone()]);
    assert_eq!(
        app.visible_thought_entries(layout.thought_entry_capacity())
            .into_iter()
            .map(|entry| entry.session_id.clone())
            .collect::<Vec<_>>(),
        vec![remote_id]
    );

    assert!(app.set_thought_filter_preset(FleetLensPreset {
        id: "send".to_string(),
        label: "Send".to_string(),
        source: "overlay".to_string(),
        matchers: vec![FleetLensPresetMatcher::Capability {
            key: "send_input".to_string(),
        }],
    }));
    assert_eq!(visible_entity_ids(&app), Vec::<String>::new());
    assert!(app
        .visible_thought_entries(layout.thought_entry_capacity())
        .is_empty());
}

#[test]
fn saved_fleet_lens_cycle_applies_builtin_presets() {
    let api = MockApi::new();
    let layout = test_layout(240, 32);
    let mut app = make_app(api);
    let local = session_summary_with_thought(
        "sess-local",
        "7",
        TEST_REPO_SWIMMERS,
        "local is working",
        "2026-03-08T14:00:05Z",
    );
    let mut remote = session_summary_with_thought(
        "sess-remote",
        "9",
        "/srv/skillbox/repos/swimmers",
        "remote is mapped",
        "2026-03-08T14:00:06Z",
    );
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
    app.entities = vec![
        SessionEntity::new(local, layout.overview_field),
        SessionEntity::new(remote, layout.overview_field),
    ];

    app.cycle_fleet_preset(1);
    assert_eq!(app.active_thought_filter_text(), "filter: lens=Local");
    assert_eq!(visible_entity_ids(&app), vec!["sess-local".to_string()]);

    app.cycle_fleet_preset(1);
    assert_eq!(app.active_thought_filter_text(), "filter: lens=Remote API");
    assert_eq!(visible_entity_ids(&app), vec!["sess-remote".to_string()]);
}

#[test]
fn clicking_bracketed_thought_label_opens_that_session() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
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
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let label_x = panel.rows[0]
        .session_rect
        .expect("row should have a clickable session label")
        .x
        .saturating_add(1);

    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-1".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    app.handle_thought_click(
        label_x,
        row_start_y,
        thought_content,
        layout.thought_entry_capacity(),
    );
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/7")
    );
}

#[test]
fn clicking_plain_thought_body_does_not_open_that_session() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
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
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let selected_before = app.selected_id.clone();
    let session_rect = panel.rows[0]
        .session_rect
        .expect("row should have a clickable session label");
    let body_x = session_rect.right().saturating_add(1);

    app.handle_thought_click(
        body_x,
        row_start_y,
        thought_content,
        layout.thought_entry_capacity(),
    );

    assert!(app.pending_interaction.is_none());
    assert_eq!(api.open_calls(), Vec::<String>::new());
    assert_eq!(app.selected_id, selected_before);
}

#[test]
fn wrapped_latest_thought_stays_bottom_aligned() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let thought_content = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 5,
    };

    app.capture_thought_updates(
        &[
            session_summary_with_thought(
                "sess-1",
                "7",
                TEST_REPO_SWIMMERS,
                "older",
                "2026-03-08T14:00:05Z",
            ),
            session_summary_with_thought(
                "sess-2",
                "9",
                TEST_REPO_SWIMMERS,
                "latest thought stays at bottom",
                "2026-03-08T14:00:06Z",
            ),
        ],
        4,
    );

    let panel = build_thought_panel(&app, thought_content, 4);

    assert_eq!(
        panel
            .rows
            .iter()
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>(),
        vec!["[work] [swi~", "  older", "[work] [swi~", "  latest th~"]
    );
    assert_eq!(
        panel.rows.last().map(|row| row.line.as_str()),
        Some("  latest th~")
    );
}

#[test]
fn clicking_wrapped_thought_session_label_opens_that_session() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    let thought_content = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 6,
    };
    app.merge_sessions(
        vec![session_summary("sess-2", "9", TEST_REPO_SWIMMERS)],
        test_field(),
    );
    app.capture_thought_updates(
        &[session_summary_with_thought(
            "sess-2",
            "9",
            TEST_REPO_SWIMMERS,
            "latest thought stays at bottom",
            "2026-03-08T14:00:06Z",
        )],
        5,
    );

    let panel = build_thought_panel(&app, thought_content, 5);
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    assert_eq!(panel.rows[0].line, "[work] [swi~");
    let session_rect = panel.rows[0]
        .session_rect
        .expect("truncated session label should remain clickable");

    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-2".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    app.handle_thought_click(session_rect.x, row_start_y, thought_content, 5);
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(app.thought_filter.tmux_name, None);
    assert_eq!(app.active_thought_filter_text(), "filter: none");
    assert_eq!(app.selected_id.as_deref(), Some("sess-2"));
    assert_eq!(api.open_calls(), vec!["sess-2".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused swimmers/9")
    );
}

#[test]
fn clicking_thought_row_surfaces_native_open_errors() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api.clone());
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
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    let body_x = panel.rows[0]
        .session_rect
        .expect("row should have text")
        .x
        .saturating_add(1);

    api.push_open_session(Err("terminal handoff unavailable".to_string()));
    app.handle_thought_click(
        body_x,
        row_start_y,
        thought_content,
        layout.thought_entry_capacity(),
    );
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert_eq!(api.open_calls(), vec!["sess-1".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("terminal handoff unavailable")
    );
    assert_eq!(app.active_thought_filter_text(), "filter: none");
}

#[test]
fn repo_theme_colors_override_state_colors_in_thought_history() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let theme_id = "/tmp/buildooor".to_string();
    let theme_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    app.repo_themes.insert(
        theme_id.clone(),
        RepoTheme {
            body: "#B89875".to_string(),
            outline: "#3D2F24".to_string(),
            accent: "#1D1914".to_string(),
            shirt: "#AA9370".to_string(),
            sprite: None,
        },
    );

    let mut busy = session_summary_with_thought(
        "sess-1",
        "alpha",
        TEST_REPO_ALPHA,
        "indexing repo",
        "2026-03-08T14:00:05Z",
    );
    busy.state = SessionState::Busy;
    busy.repo_theme_id = Some(theme_id.clone());

    let mut attention = session_summary_with_thought(
        "sess-1",
        "alpha",
        TEST_REPO_ALPHA,
        "needs input",
        "2026-03-08T14:00:06Z",
    );
    attention.state = SessionState::Attention;
    attention.repo_theme_id = Some(theme_id);

    app.capture_thought_updates(&[busy], layout.thought_entry_capacity());
    app.capture_thought_updates(&[attention], layout.thought_entry_capacity());

    assert_eq!(
        app.thought_log
            .iter()
            .map(|entry| entry.color)
            .collect::<Vec<_>>(),
        vec![theme_color]
    );

    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    assert_eq!(panel.rows.len(), 2);
    assert_eq!(cell_at(&renderer, thought_content.x, row_start_y).ch, '[');
    assert_eq!(
        cell_at(&renderer, thought_content.x, row_start_y).fg,
        theme_color
    );
}

#[test]
fn parse_hex_rgb_accepts_valid_and_rejects_multibyte_without_panic() {
    assert_eq!(parse_hex_rgb("#3930B5"), Some((0x39, 0x30, 0xB5)));
    assert_eq!(parse_hex_rgb("  #FFFFFF  "), Some((0xFF, 0xFF, 0xFF)));
    assert_eq!(parse_hex_rgb("3930B5"), None); // missing '#'
    assert_eq!(parse_hex_rgb("#12345"), None); // too short
                                               // 7-byte multibyte input must return None, not panic on a byte-slice that
                                               // splits a multi-byte char ("€" is 3 bytes, so "#€abc" is exactly 7 bytes).
    assert_eq!(parse_hex_rgb("#\u{20ac}abc"), None);
    assert_eq!(parse_hex_rgb("#\u{e9}\u{e9}\u{e9}"), None);
}

#[test]
fn low_contrast_repo_theme_color_is_adjusted_in_thought_history_and_header() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);
    let theme_id = "/tmp/skills".to_string();
    let raw_color = rgb_color((0x39, 0x30, 0xB5));
    let expected = repo_theme_display_color("#3930B5").expect("display color");
    app.repo_themes
        .insert(theme_id.clone(), repo_theme("#3930B5"));

    let mut session = session_summary_with_thought(
        "sess-1",
        "9",
        TEST_REPO_SKILLS,
        "indexing docs",
        "2026-03-08T14:00:07Z",
    );
    session.state = SessionState::Busy;
    session.repo_theme_id = Some(theme_id);

    app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
    app.merge_sessions(vec![session], layout.overview_field);

    assert_ne!(expected, raw_color);
    assert_dark_terminal_readable(expected);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 2);
    assert_eq!(panel.rows[0].color, expected);

    let header = build_header_filter_layout(&app, 120);
    let chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xskills")
        .expect("skills chip should exist");
    assert_eq!(chip.color, expected);

    let mut renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);
    assert_eq!(
        cell_at(&renderer, thought_content.x, row_start_y).fg,
        expected
    );

    render_header_filter_strip(&app, &mut renderer, 120);
    assert_eq!(cell_at(&renderer, chip.rect.x, chip.rect.y).fg, expected);
}

#[test]
fn thought_history_rows_follow_live_session_color() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let mut app = make_app(api);

    let mut session = session_summary_with_thought(
        "sess-1",
        "alpha",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    session.state = SessionState::Busy;

    app.capture_thought_updates(&[session.clone()], layout.thought_entry_capacity());
    app.merge_sessions(vec![session.clone()], layout.overview_field);

    session.state = SessionState::Attention;
    session.last_activity_at = timestamp("2026-03-08T14:00:06Z");
    app.merge_sessions(vec![session], layout.overview_field);

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    let header = build_header_filter_layout(&app, 120);
    let chip = header
        .chips
        .iter()
        .find(|chip| chip.label == "1xswimmers")
        .expect("repo chip should exist");

    assert_eq!(panel.rows.len(), 2);
    // Without a repo theme the color is derived from the tmux name, so it stays
    // stable across state transitions.
    let expected = name_based_color("alpha");
    assert_eq!(panel.rows[0].color, expected);
    assert_eq!(chip.color, expected);
}

#[test]
fn render_entity_uses_repo_theme_body_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_BUILDOOOR);
    session.state = SessionState::Busy;
    session.repo_theme_id = Some("/tmp/buildooor".to_string());
    let entity = SessionEntity::new(session, field);
    let mut repo_themes = HashMap::new();
    repo_themes.insert(
        "/tmp/buildooor".to_string(),
        RepoTheme {
            body: "#B89875".to_string(),
            outline: "#3D2F24".to_string(),
            accent: "#1D1914".to_string(),
            shirt: "#AA9370".to_string(),
            sprite: None,
        },
    );
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, false, 0, &repo_themes);

    assert_eq!(
        cell_at(&renderer, rect.x, rect.y).fg,
        Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        }
    );
}

#[test]
fn render_entity_adjusts_low_contrast_repo_theme_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_SKILLS);
    session.state = SessionState::Busy;
    session.repo_theme_id = Some("/tmp/skills".to_string());
    let entity = SessionEntity::new(session, field);
    let mut repo_themes = HashMap::new();
    repo_themes.insert("/tmp/skills".to_string(), repo_theme("#3930B5"));
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);
    let expected = session_display_color(&entity.session, &repo_themes);

    render_entity(&mut renderer, &entity, rect, false, 0, &repo_themes);

    assert_ne!(expected, rgb_color((0x39, 0x30, 0xB5)));
    assert_dark_terminal_readable(expected);
    assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, expected);
}

#[test]
fn selected_entity_preserves_repo_theme_body_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_BUILDOOOR);
    session.state = SessionState::Busy;
    session.repo_theme_id = Some("/tmp/buildooor".to_string());
    let entity = SessionEntity::new(session, field);
    let mut repo_themes = HashMap::new();
    repo_themes.insert("/tmp/buildooor".to_string(), repo_theme("#B89875"));
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, true, 0, &repo_themes);

    assert_eq!(
        cell_at(&renderer, rect.x, rect.y).fg,
        Color::Rgb {
            r: 184,
            g: 152,
            b: 117,
        }
    );
    assert_eq!(cell_at(&renderer, rect.x - 1, rect.y + 1).fg, Color::White);
    assert_eq!(
        cell_at(&renderer, rect.x, rect.y + SPRITE_HEIGHT).fg,
        Color::White
    );
}

#[test]
fn selected_entity_preserves_fallback_state_color() {
    let field = test_layout(120, 32).overview_field;
    let mut session = session_summary("sess-1", "alpha", TEST_REPO_SWIMMERS);
    session.state = SessionState::Attention;
    session.rest_state = RestState::Active;
    let expected = name_based_color("alpha");
    let entity = SessionEntity::new(session, field);
    let rect = entity.screen_rect(field);
    let mut renderer = test_renderer(120, 32);

    render_entity(&mut renderer, &entity, rect, true, 0, &HashMap::new());

    assert_eq!(cell_at(&renderer, rect.x, rect.y).fg, expected);
    assert_eq!(cell_at(&renderer, rect.x - 1, rect.y + 1).fg, Color::White);
    assert_eq!(
        cell_at(&renderer, rect.x, rect.y + SPRITE_HEIGHT).fg,
        Color::White
    );
}

#[test]
fn spawned_selected_entity_matches_thought_color() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let thought_content = layout
        .thought_content
        .expect("wide layout enables thought rail");
    let field = layout.overview_field;
    let theme_id = "/tmp/swimmers".to_string();
    let theme_color = Color::Rgb {
        r: 184,
        g: 152,
        b: 117,
    };
    let mut spawned_session = session_summary("sess-42", "42", TEST_REPO_SWIMMERS);
    spawned_session.repo_theme_id = Some(theme_id.clone());
    api.push_create_session(Ok(create_response_with_theme(
        spawned_session,
        repo_theme("#B89875"),
    )));
    let mut app = make_app(api);

    app.spawn_session(TEST_REPO_SWIMMERS, None, None, field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    let mut thought_session = session_summary_with_thought(
        "sess-42",
        "42",
        TEST_REPO_SWIMMERS,
        "patching tui",
        "2026-03-08T14:00:05Z",
    );
    thought_session.repo_theme_id = Some(theme_id);
    app.capture_thought_updates(&[thought_session.clone()], layout.thought_entry_capacity());
    app.merge_sessions(vec![thought_session], field);

    let entity = app
        .selected()
        .expect("spawned session should be selected")
        .clone();
    let rect = entity.screen_rect(field);
    let mut entity_renderer = test_renderer(120, 32);
    render_entity(
        &mut entity_renderer,
        &entity,
        rect,
        true,
        0,
        &app.repo_themes,
    );

    let panel = build_thought_panel(&app, thought_content, layout.thought_entry_capacity());
    assert_eq!(panel.rows.len(), 2);
    assert_eq!(panel.rows[0].color, theme_color);

    let mut thought_renderer = test_renderer(120, 32);
    render_thought_panel(
        &app,
        &mut thought_renderer,
        thought_content,
        layout.thought_entry_capacity(),
    );
    let row_start_y = thought_content
        .bottom()
        .saturating_sub(panel.rows.len() as u16);

    assert_eq!(cell_at(&entity_renderer, rect.x, rect.y).fg, theme_color);
    assert_eq!(
        cell_at(&thought_renderer, thought_content.x, row_start_y).fg,
        theme_color
    );
}

#[test]
fn sleeping_entity_pins_to_bottom_left_grid_slot() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![sleeping_session(
            "sess-sleep-1",
            "7",
            TEST_REPO_SWIMMERS,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    assert_eq!(
        entity_rect_for(&app, "sess-sleep-1", field),
        sleep_grid_rect(field, 0)
    );
}

#[test]
fn attention_sleeping_entity_keeps_attention_motion() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![attention_session(
            "sess-attn-sleep-1",
            "7",
            TEST_REPO_SWIMMERS,
            RestState::Sleeping,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    let entity = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-attn-sleep-1")
        .expect("entity should exist");
    assert_eq!(entity.sprite_kind(), SpriteKind::Attention);
    assert_eq!(entity.rest_anchor(), RestAnchor::FreeSwim);
}

#[test]
fn deep_sleep_entity_floats_to_top_left_grid_slot() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);

    app.merge_sessions(
        vec![deep_sleep_session(
            "sess-deep-1",
            "7",
            TEST_REPO_SWIMMERS,
            "2026-03-08T12:00:00Z",
        )],
        field,
    );

    let entity = app
        .entities
        .iter()
        .find(|entity| entity.session.session_id == "sess-deep-1")
        .expect("entity should exist");
    assert_eq!(entity.rest_anchor(), RestAnchor::Top);
    assert_eq!(
        entity_rect_for(&app, "sess-deep-1", field),
        deep_sleep_grid_rect(field, 0)
    );
}

#[test]
fn attention_session_state_text_overrides_rest_state() {
    let active = attention_session(
        "sess-attn-active",
        "7",
        TEST_REPO_SWIMMERS,
        RestState::Active,
        "2026-03-08T12:40:00Z",
    );
    let drowsy = attention_session(
        "sess-attn-drowsy",
        "8",
        TEST_REPO_SWIMMERS,
        RestState::Drowsy,
        "2026-03-08T12:20:00Z",
    );
    let sleeping = attention_session(
        "sess-attn-sleep",
        "9",
        TEST_REPO_SWIMMERS,
        RestState::Sleeping,
        "2026-03-08T12:00:00Z",
    );
    let deep_sleep = attention_session(
        "sess-attn-deep",
        "10",
        TEST_REPO_SWIMMERS,
        RestState::DeepSleep,
        "2026-03-08T11:00:00Z",
    );

    assert_eq!(session_state_text(&active), "attention");
    assert_eq!(session_state_text(&drowsy), "attention");
    assert_eq!(session_state_text(&sleeping), "attention");
    assert_eq!(session_state_text(&deep_sleep), "attention");
}
