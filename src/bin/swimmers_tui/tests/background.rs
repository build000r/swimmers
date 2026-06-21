use super::*;

#[test]
fn poll_refresh_noop_when_no_pending() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);

    // Should not panic or modify state when nothing is pending
    app.poll_refresh(layout);
    assert!(app.pending_refresh.is_none());
    assert!(app.entities.is_empty());
}

#[test]
fn background_refresh_delivers_sessions_and_native_status() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary("s1", "1", TEST_REPO_ALPHA),
        session_summary("s2", "2", TEST_REPO_BETA),
        session_summary("s3", "3", TEST_REPO_GAMMA),
    ]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    }));

    let mut app = make_app(api);
    app.spawn_background_refresh(false);
    assert!(app.pending_refresh.is_some());

    poll_until_refresh(&mut app, layout);

    assert_eq!(app.entities.len(), 3);
    assert!(app.native_status.is_some());
    assert!(app.pending_refresh.is_none());
    assert!(app.last_refresh.is_some());
}

#[test]
fn background_refresh_fetches_mermaid_artifacts_concurrently() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary("s1", "1", TEST_REPO_ALPHA),
        session_summary("s2", "2", TEST_REPO_BETA),
        session_summary("s3", "3", TEST_REPO_GAMMA),
    ]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1.mmd",
        "2025-01-01T00:00:00Z",
        "graph TD; A-->B;",
    )));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s2",
        "/tmp/s2.mmd",
        "2025-01-01T00:00:00Z",
        "graph TD; C-->D;",
    )));
    api.push_mermaid_artifact(Ok(MermaidArtifactResponse {
        session_id: "s3".to_string(),
        available: false,
        path: None,
        updated_at: None,
        source: None,
        error: None,
        slice_name: None,
        plan_files: None,
    }));

    let mut app = make_app(api);
    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);

    assert_eq!(app.entities.len(), 3);
    assert_eq!(
        app.mermaid_artifacts.len(),
        2,
        "only available artifacts stored"
    );
    assert!(app.mermaid_artifacts.contains_key("s1"));
    assert!(app.mermaid_artifacts.contains_key("s2"));
}

#[test]
fn background_refresh_fetches_session_skills_once_per_repo_context() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary("s1", "1", TEST_REPO_ALPHA),
        session_summary("s2", "2", TEST_REPO_ALPHA),
        session_summary("s3", "3", TEST_REPO_BETA),
    ]));
    api.push_session_skills(Ok(session_skill_response(
        "s1",
        TEST_REPO_ALPHA,
        vec![session_skill(
            "sbp",
            "skillbox",
            "/Users/tester/repos/skills-private/sbp",
            "/repo-alpha/.codex/skills/sbp",
        )],
    )));
    api.push_session_skills(Ok(session_skill_response(
        "s3",
        TEST_REPO_BETA,
        vec![session_skill(
            "ui",
            "skills-private",
            "/Users/tester/repos/skills-private/ui",
            "/repo-beta/.codex/skills/ui",
        )],
    )));
    let mut app = make_app(api.clone());

    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);

    assert_eq!(
        api.session_skill_calls(),
        vec!["s1".to_string(), "s3".to_string()]
    );
    assert_eq!(app.session_skill_cache.len(), 3);
    assert_eq!(
        app.session_skill_cache
            .get("s2")
            .and_then(|entry| entry.response.as_ref())
            .map(|response| response.session_id.as_str()),
        Some("s2")
    );
    assert_eq!(
        app.session_skill_cache
            .get("s2")
            .and_then(|entry| entry.response.as_ref())
            .and_then(|response| response.skills.first())
            .map(|skill| skill.name.as_str()),
        Some("sbp")
    );
}

#[test]
fn background_refresh_reuses_cached_assets_for_unchanged_sessions() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let repo = tempdir().expect("tempdir");
    let repo_src = repo.path().join("src");
    fs::create_dir_all(&repo_src).expect("create repo src");
    let theme_id = repo.path().to_string_lossy().into_owned();
    let cwd = repo_src.to_string_lossy().into_owned();

    write_repo_theme_colors_with_sprite(repo.path(), "#B89875", Some("jelly"));
    api.push_fetch_sessions(Ok(vec![session_summary_with_theme_id(
        "s1", "1", &cwd, &theme_id,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-a.mmd",
        "2026-04-05T20:00:00Z",
        "graph TD; A-->B;",
    )));
    let mut app = make_app(api.clone());

    app.refresh(layout);
    assert_eq!(
        app.mermaid_artifacts
            .get("s1")
            .and_then(|artifact| artifact.path.as_deref()),
        Some("/tmp/s1-a.mmd")
    );
    assert_eq!(
        app.repo_themes.get(&theme_id).expect("cached theme").body,
        "#B89875"
    );
    assert_eq!(
        app.repo_themes
            .get(&theme_id)
            .and_then(|theme| theme.sprite.as_deref()),
        Some("jelly")
    );

    write_repo_theme_colors_with_sprite(repo.path(), "#44AA88", Some("fish"));
    api.push_fetch_sessions(Ok(vec![session_summary_with_theme_id(
        "s1", "1", &cwd, &theme_id,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-b.mmd",
        "2026-04-05T20:01:00Z",
        "graph TD; B-->C;",
    )));

    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);

    assert_eq!(
        app.mermaid_artifacts
            .get("s1")
            .and_then(|artifact| artifact.path.as_deref()),
        Some("/tmp/s1-a.mmd")
    );
    assert_eq!(
        app.repo_themes.get(&theme_id).expect("cached theme").body,
        "#B89875"
    );
    assert_eq!(
        app.repo_themes
            .get(&theme_id)
            .and_then(|theme| theme.sprite.as_deref()),
        Some("jelly")
    );
    assert_eq!(api.mermaid_artifact_calls(), vec!["s1".to_string()]);
}

#[test]
fn manual_refresh_revalidates_cached_assets() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let repo = tempdir().expect("tempdir");
    let repo_src = repo.path().join("src");
    fs::create_dir_all(&repo_src).expect("create repo src");
    let theme_id = repo.path().to_string_lossy().into_owned();
    let cwd = repo_src.to_string_lossy().into_owned();

    write_repo_theme_colors_with_sprite(repo.path(), "#B89875", Some("jelly"));
    api.push_fetch_sessions(Ok(vec![session_summary_with_theme_id(
        "s1", "1", &cwd, &theme_id,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-a.mmd",
        "2026-04-05T20:00:00Z",
        "graph TD; A-->B;",
    )));
    let mut app = make_app(api.clone());
    app.refresh(layout);

    write_repo_theme_colors_with_sprite(repo.path(), "#44AA88", Some("fish"));
    api.push_fetch_sessions(Ok(vec![session_summary_with_theme_id(
        "s1", "1", &cwd, &theme_id,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-b.mmd",
        "2026-04-05T20:01:00Z",
        "graph TD; B-->C;",
    )));

    app.manual_refresh(layout);
    poll_until_refresh(&mut app, layout);

    assert_eq!(
        app.mermaid_artifacts
            .get("s1")
            .and_then(|artifact| artifact.path.as_deref()),
        Some("/tmp/s1-b.mmd")
    );
    assert_eq!(
        app.repo_themes
            .get(&theme_id)
            .expect("refreshed theme")
            .body,
        "#44AA88"
    );
    assert_eq!(
        app.repo_themes
            .get(&theme_id)
            .and_then(|theme| theme.sprite.as_deref()),
        Some("fish")
    );
    assert_eq!(
        api.mermaid_artifact_calls(),
        vec!["s1".to_string(), "s1".to_string()]
    );
}

#[test]
fn open_mermaid_viewer_revalidates_cached_artifact_for_known_session() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("s1", "1", TEST_REPO_ALPHA)]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-a.mmd",
        "2026-04-05T20:00:00Z",
        "graph TD; A-->B;",
    )));
    let mut app = make_app(api.clone());
    app.refresh(layout);

    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-b.mmd",
        "2026-04-05T20:01:00Z",
        "graph TD; B-->C;",
    )));

    app.open_mermaid_viewer("s1".to_string());

    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    assert_eq!(viewer.path.as_deref(), Some("/tmp/s1-b.mmd"));
    assert_eq!(
        api.mermaid_artifact_calls(),
        vec!["s1".to_string(), "s1".to_string()]
    );
}

#[test]
fn background_refresh_reloads_assets_when_session_context_changes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let repo_a = tempdir().expect("tempdir");
    let repo_a_src = repo_a.path().join("src");
    fs::create_dir_all(&repo_a_src).expect("create repo a src");
    let repo_b = tempdir().expect("tempdir");
    let repo_b_src = repo_b.path().join("src");
    fs::create_dir_all(&repo_b_src).expect("create repo b src");

    let theme_id_a = repo_a.path().to_string_lossy().into_owned();
    let theme_id_b = repo_b.path().to_string_lossy().into_owned();
    let cwd_a = repo_a_src.to_string_lossy().into_owned();
    let cwd_b = repo_b_src.to_string_lossy().into_owned();

    write_repo_theme_colors(repo_a.path(), "#B89875");
    write_repo_theme_colors(repo_b.path(), "#44AA88");

    api.push_fetch_sessions(Ok(vec![session_summary_with_theme_id(
        "s1",
        "1",
        &cwd_a,
        &theme_id_a,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-a.mmd",
        "2026-04-05T20:00:00Z",
        "graph TD; A-->B;",
    )));
    let mut app = make_app(api.clone());
    app.refresh(layout);

    api.push_fetch_sessions(Ok(vec![session_summary_with_theme_id(
        "s1",
        "1",
        &cwd_b,
        &theme_id_b,
    )]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1-b.mmd",
        "2026-04-05T20:01:00Z",
        "graph TD; B-->C;",
    )));

    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);

    assert_eq!(
        app.mermaid_artifacts
            .get("s1")
            .and_then(|artifact| artifact.path.as_deref()),
        Some("/tmp/s1-b.mmd")
    );
    assert_eq!(
        app.repo_themes.get(&theme_id_b).expect("repo b theme").body,
        "#44AA88"
    );
    assert!(!app.repo_themes.contains_key(&theme_id_a));
    assert_eq!(
        api.mermaid_artifact_calls(),
        vec!["s1".to_string(), "s1".to_string()]
    );
}

#[test]
fn background_refresh_error_retains_previous_entities() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);

    // First: populate with a sync refresh
    api.push_fetch_sessions(Ok(vec![session_summary("s1", "1", TEST_REPO_ALPHA)]));
    let mut app = make_app(api.clone());
    app.refresh(layout);
    assert_eq!(app.entities.len(), 1, "setup: one entity");

    assert!(
        app.native_status.is_some(),
        "setup: terminal handoff status populated"
    );

    // Second: background refresh with error
    api.push_fetch_sessions(Err("connection refused".to_string()));
    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);

    assert_eq!(app.entities.len(), 1, "entities retained after error");
    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        Some("backend offline"),
        "recent success should produce short message, not full diagnostic"
    );
    assert!(
        app.native_status.is_some(),
        "native_status not overwritten on error"
    );
}

#[test]
fn refresh_error_escalates_after_prolonged_outage() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);

    // Successful initial refresh sets last_successful_refresh.
    api.push_fetch_sessions(Ok(vec![session_summary("s1", "1", TEST_REPO_ALPHA)]));
    let mut app = make_app(api.clone());
    app.refresh(layout);
    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        None,
        "no message after successful refresh"
    );

    // Immediate failure → short message.
    api.push_fetch_sessions(Err("connection refused".to_string()));
    app.refresh(layout);
    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        Some("backend offline"),
    );

    // Simulate stale last_successful_refresh beyond BACKEND_OFFLINE_ESCALATION.
    app.last_successful_refresh =
        Some(Instant::now() - BACKEND_OFFLINE_ESCALATION - Duration::from_secs(1));
    api.push_fetch_sessions(Err("connection refused".to_string()));
    app.message = None; // clear so set_message dedup doesn't suppress
    app.refresh(layout);
    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        Some("connection refused"),
        "should escalate to full diagnostic after prolonged outage"
    );
}

#[test]
fn refresh_error_shows_full_message_without_prior_success() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);

    // No prior successful refresh → full diagnostic immediately.
    api.push_fetch_sessions(Err("connection refused".to_string()));
    let mut app = make_app(api);
    app.refresh(layout);
    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        Some("connection refused"),
        "first failure without any prior success should show full message"
    );
}

#[test]
fn background_refresh_partial_mermaid_failure() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary("s1", "1", TEST_REPO_ALPHA),
        session_summary("s2", "2", TEST_REPO_BETA),
        session_summary("s3", "3", TEST_REPO_GAMMA),
    ]));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s1",
        "/tmp/s1.mmd",
        "2025-01-01T00:00:00Z",
        "graph TD; A-->B;",
    )));
    api.push_mermaid_artifact(Err("timeout".to_string()));
    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "s3",
        "/tmp/s3.mmd",
        "2025-01-01T00:00:00Z",
        "graph TD; E-->F;",
    )));

    let mut app = make_app(api);
    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);

    assert_eq!(app.entities.len(), 3, "sessions still merged");
    assert_eq!(app.mermaid_artifacts.len(), 2, "two successful artifacts");
    assert_eq!(
        app.message.as_ref().map(|(m, _)| m.as_str()),
        Some("timeout"),
        "mermaid error surfaced"
    );
}

#[test]
fn background_refresh_syncs_selection_publication() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("s1", "1", TEST_REPO_ALPHA)]));
    let mut app = make_app(api.clone());

    // Set a selected session
    app.selected_id = Some("s1".to_string());
    app.spawn_background_refresh(false);
    poll_until_refresh(&mut app, layout);
    poll_until_selection_publication(&mut app);

    let calls = api.publish_calls();
    assert!(
        calls.iter().any(|c| c.as_deref() == Some("s1")),
        "publish_selection should have been called with s1, got: {calls:?}"
    );
}

#[test]
fn sync_selection_publication_runs_in_background() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    app.selected_id = Some("s1".to_string());

    app.sync_selection_publication();

    assert!(app.pending_selection_publication.is_some());
    assert_eq!(app.published_selected_id, None);
    poll_until_selection_publication(&mut app);

    assert_eq!(app.published_selected_id.as_deref(), Some("s1"));
    assert_eq!(api.publish_calls(), vec![Some("s1".to_string())]);
}

#[test]
fn selection_publication_coalesces_to_latest_target() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    app.selected_id = Some("s1".to_string());
    app.sync_selection_publication();
    app.selected_id = Some("s2".to_string());
    app.sync_selection_publication();

    assert!(app.pending_selection_publication.is_some());
    assert_eq!(
        app.queued_selection_publication,
        Some((Some("s2".to_string()), false))
    );
    poll_until_selection_publication(&mut app);

    assert_eq!(app.published_selected_id.as_deref(), Some("s2"));
    assert_eq!(
        api.publish_calls(),
        vec![Some("s1".to_string()), Some("s2".to_string())]
    );
}

#[test]
fn manual_refresh_cancels_inflight_background() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);

    // Queue results for first (cancelled) refresh
    api.push_fetch_sessions(Ok(vec![session_summary("s1", "1", TEST_REPO_ALPHA)]));
    // Queue results for second (manual) refresh
    api.push_fetch_sessions(Ok(vec![
        session_summary("s1", "1", TEST_REPO_ALPHA),
        session_summary("s2", "2", TEST_REPO_BETA),
    ]));

    let mut app = make_app(api);
    app.spawn_background_refresh(false);
    assert!(app.pending_refresh.is_some());

    // Manual refresh should drop old receiver and spawn new one
    app.manual_refresh(layout);
    assert!(app.pending_refresh.is_some());

    poll_until_refresh(&mut app, layout);

    // The manual refresh had show_success_message=true
    assert!(
        app.message
            .as_ref()
            .map(|(m, _)| m.contains("refreshed"))
            .unwrap_or(false),
        "manual refresh message should appear"
    );
}

#[test]
fn frame_duration_is_30fps() {
    assert_eq!(FRAME_DURATION, Duration::from_millis(33));
}

#[test]
fn initial_sync_refresh_populates_state() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("s1", "1", TEST_REPO_ALPHA)]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    }));

    let mut app = make_app(api);
    // Sync refresh (used at startup)
    app.refresh(layout);

    assert_eq!(app.entities.len(), 1, "entities populated synchronously");
    assert!(
        app.native_status.is_some(),
        "terminal handoff status populated synchronously"
    );
}

/// Regression guard for the embedded-mode deferred-init path: the first
/// visible frame must render without waiting for slow startup discovery.
#[test]
fn embedded_mode_first_frame_perf_gate() {
    let _lock = lock_test_env();
    let original_clawgs = env::var_os("CLAWGS_BIN");
    let original_data_dir = env::var_os("SWIMMERS_DATA_DIR");
    let original_tui_url = env::var_os("SWIMMERS_TUI_URL");
    let temp = tempdir().expect("tempdir");
    let args_log = temp.path().join("args.log");
    let input_log = temp.path().join("input.log");
    let fake_clawgs = write_fake_clawgs_script(&args_log, &input_log, temp.path());
    let data_dir = temp.path().join("data");

    env::set_var("CLAWGS_BIN", fake_clawgs.as_os_str());
    env::set_var("SWIMMERS_DATA_DIR", &data_dir);
    env::remove_var("SWIMMERS_TUI_URL");

    let (_tmux_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-sessions)
    sleep 0.25
    ;;
  *)
    exit 0
    ;;
esac
"#,
    );

    let mut samples = Vec::new();
    for _ in 0..5 {
        let runtime = test_runtime();
        let started = Instant::now();
        let (client, shutdown) = {
            let _runtime_guard = runtime.enter();
            build_embedded_client(&runtime)
        };
        let mut app = App::new(runtime, client);
        app.set_embedded_shutdown(shutdown);
        let mut renderer = test_renderer(120, 32);
        let layout = app.layout_for_terminal(renderer.width(), renderer.height());
        app.refresh_initial_frame(layout);
        prepare_frame(&mut app, &mut renderer);
        let elapsed = started.elapsed();
        samples.push(elapsed);
        let header_visible = find_text_position(&renderer, "swimmers tui").is_some();
        let empty_state_visible = find_text_position(&renderer, "no tmux sessions found").is_some();

        app.shutdown_embedded()
            .expect("embedded shutdown should complete");

        assert!(header_visible, "first frame should render the TUI header");
        assert!(
            empty_state_visible,
            "first frame should render the empty aquarium state before deferred discovery completes"
        );
    }

    restore_os_env_var("PATH", original_path);
    restore_os_env_var("SWIMMERS_TUI_URL", original_tui_url);
    restore_os_env_var("SWIMMERS_DATA_DIR", original_data_dir);
    restore_os_env_var("CLAWGS_BIN", original_clawgs);

    let p95 = p95_duration(samples.clone());
    eprintln!("embedded first-frame samples: {:?}", samples);
    eprintln!(
        "embedded first-frame p95: {:?} (budget {:?})",
        p95, EMBEDDED_FIRST_FRAME_BUDGET
    );
    assert!(
        p95 < EMBEDDED_FIRST_FRAME_BUDGET,
        "expected embedded first-frame p95 under {:?}, got {:?}",
        EMBEDDED_FIRST_FRAME_BUDGET,
        p95
    );
}

#[test]
fn embedded_mode_ctrl_c_requests_bounded_shutdown() {
    let _lock = lock_test_env();
    let original_clawgs = env::var_os("CLAWGS_BIN");
    let original_data_dir = env::var_os("SWIMMERS_DATA_DIR");
    let original_tui_url = env::var_os("SWIMMERS_TUI_URL");
    let temp = tempdir().expect("tempdir");
    let args_log = temp.path().join("args.log");
    let input_log = temp.path().join("input.log");
    let fake_clawgs = write_fake_clawgs_script(&args_log, &input_log, temp.path());
    let data_dir = temp.path().join("data");

    env::set_var("CLAWGS_BIN", fake_clawgs.as_os_str());
    env::set_var("SWIMMERS_DATA_DIR", &data_dir);
    env::remove_var("SWIMMERS_TUI_URL");

    let (_tmux_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-sessions)
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
"#,
    );

    let runtime = test_runtime();
    let (client, shutdown) = {
        let _runtime_guard = runtime.enter();
        build_embedded_client(&runtime)
    };
    let mut app = App::new(runtime, client);
    app.set_embedded_shutdown(shutdown);
    let layout = test_layout(120, 32);

    assert!(!handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ));

    let started = Instant::now();
    app.shutdown_embedded()
        .expect("embedded shutdown should complete");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "embedded shutdown must stay bounded"
    );

    restore_os_env_var("PATH", original_path);
    restore_os_env_var("SWIMMERS_TUI_URL", original_tui_url);
    restore_os_env_var("SWIMMERS_DATA_DIR", original_data_dir);
    restore_os_env_var("CLAWGS_BIN", original_clawgs);
}
