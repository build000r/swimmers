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
