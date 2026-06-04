use super::*;

#[test]
fn set_message_deduplicates_repeated_errors() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.set_message("backend unavailable");
    let first = app.message.as_ref().expect("message").1;

    std::thread::sleep(Duration::from_millis(5));
    app.set_message("backend unavailable");

    let second = app.message.as_ref().expect("message").1;
    assert_eq!(first, second);
}

#[test]
fn auto_refresh_keeps_existing_footer_message() {
    let api = MockApi::new();
    let layout = test_layout(160, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-7", "7", TEST_REPO_SWIMMERS)]));
    let mut app = make_app(api);
    app.set_message("sticky status");

    app.refresh(layout);

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("sticky status")
    );
}

#[test]
fn manual_refresh_reports_session_count() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![
        session_summary("sess-7", "7", TEST_REPO_SWIMMERS),
        session_summary("sess-8", "8", TEST_REPO_OPENSOURCE),
    ]));
    let mut app = make_app(api);

    app.manual_refresh(layout);
    poll_until_refresh(&mut app, layout);

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("refreshed 2 sessions")
    );
}

#[test]
fn refresh_skips_native_status_when_sessions_fail() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Err("timed out while trying to refresh sessions".to_string()));
    let mut app = make_app(api.clone());

    app.refresh_initial_frame(layout);

    assert_eq!(
        api.native_status_calls(),
        0,
        "terminal handoff status should not be called when sessions failed"
    );
    assert!(
        app.message
            .as_ref()
            .map(|(m, _)| m.contains("refresh sessions"))
            .unwrap_or(false),
        "sessions error should be in message"
    );
}

#[test]
fn refresh_calls_native_status_when_sessions_succeed() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![]));
    api.push_native_status(Ok(NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    }));
    let mut app = make_app(api.clone());

    app.refresh(layout);

    assert_eq!(
        api.native_status_calls(),
        1,
        "terminal handoff status should be called when sessions succeeded"
    );
    assert!(app.native_status.is_some());
}

#[test]
fn refresh_renders_backend_health_degraded_and_recovered_banner() {
    let api = MockApi::new();
    let mut degraded = healthy_backend_health();
    degraded.persistence.ok = false;
    degraded.persistence.consecutive_failures = 1;
    degraded.persistence.last_failed_operation = Some("save_sessions".to_string());
    degraded.persistence.last_error = Some("disk full".to_string());
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    api.push_backend_health(Ok(degraded));

    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.refresh_with_feedback(layout, false);

    assert_eq!(api.backend_health_calls(), 1);
    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "persistence degraded: save_sessions: disk full").is_some()
    );

    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    api.push_backend_health(Ok(healthy_backend_health()));
    app.refresh_with_feedback(layout, false);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "persistence degraded").is_none(),
        "healthy backend health should clear the degraded banner"
    );
}

#[test]
fn empty_aquarium_shows_tmux_unavailable_when_discovery_fails() {
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
        "should show tmux unavailable message when discovery reports unavailable"
    );
}

#[test]
fn empty_aquarium_shows_normal_message_when_tmux_healthy() {
    let api = MockApi::new();
    api.push_fetch_sessions(Ok(vec![]));
    api.push_backend_health(Ok(healthy_backend_health()));

    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.refresh_with_feedback(layout, false);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "no tmux sessions found").is_some(),
        "should show normal empty message when tmux is healthy"
    );
    assert!(
        find_text_position(&renderer, "tmux unavailable").is_none(),
        "should not show tmux unavailable when tmux is healthy"
    );
}

#[test]
fn dependency_degradation_renders_in_banner() {
    let api = MockApi::new();
    let mut health = healthy_backend_health();
    health.dependencies.as_mut().unwrap().remote_targets.status = "degraded".to_string();
    health
        .dependencies
        .as_mut()
        .unwrap()
        .remote_targets
        .last_error = Some("connection refused".to_string());
    api.push_fetch_sessions(Ok(vec![session_summary("sess-1", "1", TEST_REPO_SWIMMERS)]));
    api.push_backend_health(Ok(health));

    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.refresh_with_feedback(layout, false);

    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, layout);
    assert!(
        find_text_position(&renderer, "remote targets degraded").is_some(),
        "should show dependency degradation banner"
    );
}

#[test]
fn refresh_sessions_error_not_overwritten_by_native_status_error() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Err("timed out while trying to refresh sessions".to_string()));
    let mut app = make_app(api);

    app.refresh(layout);

    let msg = app.message.as_ref().map(|(m, _)| m.as_str()).unwrap_or("");
    assert!(
        msg.contains("refresh sessions"),
        "expected sessions error, got: {msg}"
    );
    assert!(
        !msg.contains("terminal handoff status"),
        "native-status error must not overwrite sessions error: {msg}"
    );
}

#[test]
fn refresh_retains_cached_native_status_when_sessions_fail() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let cached = NativeDesktopStatusResponse {
        supported: true,
        platform: Some("macos".to_string()),
        app_id: Some(NativeDesktopApp::Iterm),
        ghostty_mode: None,
        app: Some("iTerm".to_string()),
        reason: None,
    };
    api.push_fetch_sessions(Ok(vec![]));
    api.push_native_status(Ok(cached));
    let mut app = make_app(api.clone());
    app.refresh(layout);
    assert!(
        app.native_status.is_some(),
        "setup: native_status should be populated"
    );

    api.push_fetch_sessions(Err("backend down".to_string()));
    app.refresh(layout);

    assert!(
        app.native_status.is_some(),
        "cached native_status should be retained after a failed refresh"
    );
    assert_eq!(
        app.native_status.as_ref().unwrap().app.as_deref(),
        Some("iTerm"),
        "cached value should match what was last successfully fetched"
    );
}

fn thought_config_test(ok: bool, message: &str) -> ThoughtConfigTestResponse {
    ThoughtConfigTestResponse {
        ok,
        message: message.to_string(),
        last_backend_error: None,
        llm_calls: if ok { 1 } else { 0 },
    }
}

fn empty_refresh_result() -> RefreshResult {
    RefreshResult {
        sessions: Ok(Vec::new()),
        mermaid_artifacts: Vec::new(),
        session_skills: Vec::new(),
        backend_health: Ok(healthy_backend_health()),
        native_status: None,
        daemon_defaults_status: None,
        show_success_message: false,
        force_asset_refresh: false,
    }
}

fn repo_theme(body: &str) -> RepoTheme {
    RepoTheme {
        body: body.to_string(),
        outline: "#3D2F24".to_string(),
        accent: "#1D1914".to_string(),
        shirt: "#AA9370".to_string(),
        sprite: None,
    }
}

#[test]
fn thought_config_target_summary_preserves_blank_and_literal_fields() {
    let mut config = ThoughtConfig {
        backend: String::new(),
        model: String::new(),
        ..ThoughtConfig::default()
    };
    assert_eq!(
        App::<MockApi>::thought_config_target_summary(&config),
        "auto / daemon default"
    );

    config.backend = "  ".to_string();
    config.model = "\t".to_string();
    assert_eq!(
        App::<MockApi>::thought_config_target_summary(&config),
        "auto / daemon default"
    );

    config.backend = " openrouter ".to_string();
    config.model = " model/free ".to_string();
    assert_eq!(
        App::<MockApi>::thought_config_target_summary(&config),
        " openrouter  /  model/free "
    );
}

#[test]
fn thought_config_save_action_preserves_success_failure_and_test_messages() {
    let runtime = test_runtime();
    let mut config = ThoughtConfig {
        backend: "openrouter".to_string(),
        model: "model/free".to_string(),
        ..ThoughtConfig::default()
    };

    let ok_api = MockApi::new();
    ok_api.push_update_thought_config(Ok(config.clone()));
    ok_api.push_test_thought_config(Ok(thought_config_test(true, "probe succeeded")));
    let ok = runtime.block_on(App::run_thought_config_save_action(
        Arc::new(ok_api),
        config.clone(),
        None,
    ));
    assert_eq!(ok.message, "saved openrouter / model/free | test ok");
    assert!(ok.close_editor);
    assert!(ok.refresh_sessions);
    assert!(ok.updated_config.is_none());
    assert!(ok.openrouter_candidates.is_none());

    let failed_test_api = MockApi::new();
    failed_test_api.push_update_thought_config(Ok(config.clone()));
    failed_test_api.push_test_thought_config(Ok(thought_config_test(false, "probe failed")));
    let failed_test = runtime.block_on(App::run_thought_config_save_action(
        Arc::new(failed_test_api),
        config.clone(),
        None,
    ));
    assert_eq!(
        failed_test.message,
        "saved openrouter / model/free | probe failed"
    );
    assert!(failed_test.close_editor);
    assert!(failed_test.refresh_sessions);

    let test_error_api = MockApi::new();
    test_error_api.push_update_thought_config(Ok(config.clone()));
    test_error_api.push_test_thought_config(Err("network down".to_string()));
    let test_error = runtime.block_on(App::run_thought_config_save_action(
        Arc::new(test_error_api),
        config.clone(),
        None,
    ));
    assert_eq!(
        test_error.message,
        "saved openrouter / model/free | test error: network down"
    );
    assert!(test_error.close_editor);
    assert!(test_error.refresh_sessions);

    config.model = "bad/model".to_string();
    let save_error_api = MockApi::new();
    save_error_api.push_update_thought_config(Err("save failed".to_string()));
    let save_error = runtime.block_on(App::run_thought_config_save_action(
        Arc::new(save_error_api),
        config,
        None,
    ));
    assert_eq!(save_error.message, "save failed");
    assert!(!save_error.close_editor);
    assert!(!save_error.refresh_sessions);
}

#[test]
fn thought_config_action_outcome_updates_open_editor_and_sets_message() {
    let mut app = make_app(MockApi::new());
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "old/free".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    let candidates = vec!["new/free".to_string(), "backup/free".to_string()];

    app.apply_thought_config_action_outcome(ThoughtConfigActionOutcome {
        message: "test complete".to_string(),
        updated_config: Some(ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "new/free".to_string(),
            ..ThoughtConfig::default()
        }),
        openrouter_candidates: Some(candidates.clone()),
        close_editor: false,
        refresh_sessions: false,
    });

    let editor = app
        .thought_config_editor
        .as_ref()
        .expect("editor remains open");
    assert_eq!(editor.config.model, "new/free");
    assert_eq!(editor.openrouter_model_presets, candidates);
    assert_eq!(app.visible_message(), Some("test complete"));
    assert!(app.pending_refresh.is_none());
}

#[test]
fn thought_config_action_outcome_ignores_editor_updates_when_editor_closed() {
    let mut app = make_app(MockApi::new());

    app.apply_thought_config_action_outcome(ThoughtConfigActionOutcome {
        message: "save complete".to_string(),
        updated_config: Some(ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "ignored/free".to_string(),
            ..ThoughtConfig::default()
        }),
        openrouter_candidates: Some(vec!["ignored/free".to_string()]),
        close_editor: true,
        refresh_sessions: false,
    });

    assert!(app.thought_config_editor.is_none());
    assert!(app.pending_refresh.is_none());
    assert_eq!(app.visible_message(), Some("save complete"));
}

#[test]
fn thought_config_action_outcome_closes_editor_and_restarts_refresh() {
    let mut app = make_app(MockApi::new());
    app.daemon_defaults_status = DaemonDefaultsStatus::Available;
    app.thought_config_editor = Some(ThoughtConfigEditorState::new(
        ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "old/free".to_string(),
            ..ThoughtConfig::default()
        },
        None,
    ));
    let (old_tx, old_rx) = tokio::sync::oneshot::channel();
    app.pending_refresh = Some(old_rx);

    app.apply_thought_config_action_outcome(ThoughtConfigActionOutcome {
        message: "saved".to_string(),
        updated_config: Some(ThoughtConfig {
            backend: "openrouter".to_string(),
            model: "new/free".to_string(),
            ..ThoughtConfig::default()
        }),
        openrouter_candidates: Some(vec!["new/free".to_string()]),
        close_editor: true,
        refresh_sessions: true,
    });

    assert!(app.thought_config_editor.is_none());
    assert!(app.pending_refresh.is_some());
    assert!(old_tx.send(empty_refresh_result()).is_err());
    assert_eq!(app.visible_message(), Some("saved"));
}

#[test]
fn sync_repo_themes_reuses_cached_theme_unless_forced() {
    let mut app = make_app(MockApi::new());
    let tmp = tempdir().expect("tempdir");
    let theme_id = tmp.path().to_string_lossy().into_owned();
    let mut session = session_summary("s1", "swimmers-1", TEST_REPO_SWIMMERS);
    session.cwd = tmp.path().join("child").to_string_lossy().into_owned();
    session.repo_theme_id = Some(theme_id.clone());
    app.repo_themes
        .insert(theme_id.clone(), repo_theme("#B89875"));

    app.sync_repo_themes(&[session.clone()], false);

    assert_eq!(
        app.repo_themes.get(&theme_id).expect("fallback theme").body,
        "#B89875"
    );

    app.sync_repo_themes(&[session], true);

    assert!(
        !app.repo_themes.contains_key(&theme_id),
        "force refresh must not reuse stale fallback theme"
    );
}

#[test]
fn apply_refresh_result_sequences_success_side_effects_and_metadata() {
    let mut app = make_app(MockApi::new());
    let tmp = tempdir().expect("tempdir");
    let mut session = session_summary("s1", "swimmers-1", TEST_REPO_SWIMMERS);
    session.cwd = tmp.path().to_string_lossy().into_owned();
    let layout = WorkspaceLayout::for_terminal_without_thought_panel(100, 32);
    let result = RefreshResult {
        sessions: Ok(vec![session]),
        mermaid_artifacts: Vec::new(),
        session_skills: Vec::new(),
        backend_health: Ok(healthy_backend_health()),
        native_status: Some(Ok(NativeDesktopStatusResponse {
            supported: true,
            platform: Some("linux".to_string()),
            app_id: Some(NativeDesktopApp::Iterm),
            ghostty_mode: None,
            app: Some("iTerm".to_string()),
            reason: None,
        })),
        daemon_defaults_status: Some(DaemonDefaultsStatus::Available),
        show_success_message: true,
        force_asset_refresh: false,
    };

    app.apply_refresh_result(result, layout);

    assert_eq!(app.entities.len(), 1);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("refreshed 1 session")
    );
    assert!(app.last_successful_refresh.is_some());
    assert!(app.last_refresh.is_some());
    assert_eq!(
        app.native_status
            .as_ref()
            .and_then(|status| status.app.as_deref()),
        Some("iTerm")
    );
    assert!(app.backend_health.is_some());
    assert_eq!(app.daemon_defaults_status, DaemonDefaultsStatus::Available);
}

#[test]
fn apply_refresh_result_records_session_failure_but_still_applies_available_metadata() {
    let mut app = make_app(MockApi::new());
    let layout = WorkspaceLayout::for_terminal_without_thought_panel(100, 32);
    let result = RefreshResult {
        sessions: Err("refresh failed".to_string()),
        mermaid_artifacts: Vec::new(),
        session_skills: Vec::new(),
        backend_health: Ok(healthy_backend_health()),
        native_status: None,
        daemon_defaults_status: Some(DaemonDefaultsStatus::Unavailable),
        show_success_message: true,
        force_asset_refresh: false,
    };

    app.apply_refresh_result(result, layout);

    assert!(app.entities.is_empty());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("refresh failed")
    );
    assert!(app.last_successful_refresh.is_none());
    assert!(app.last_refresh.is_some());
    assert!(app.backend_health.is_some());
    assert_eq!(
        app.daemon_defaults_status,
        DaemonDefaultsStatus::Unavailable
    );
}
