use super::*;

#[test]
fn submitting_initial_request_creates_hidden_session_without_native_open() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]),
        true,
        SpawnTool::Codex,
        None,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "add hidden spawn flow".to_string(),
        batch_dirs: None,
        launch_target: None,
    });

    app.submit_initial_request(field);
    assert!(app.pending_interaction.is_some());
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_some());
    assert!(app.picker.is_some());
    assert!(app.selected_id.is_none());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Grok,
            Some("add hidden spawn flow".to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
    assert!(app.picker.is_none());
    assert!(app.initial_request.is_none());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("created 55")
    );
    assert!(app
        .entities
        .iter()
        .any(|entity| entity.session.session_id == "sess-55"));
}

#[test]
fn adopt_tmux_session_reattaches_without_duplicate_entity() {
    let api = MockApi::new();
    let mut adopted = session_summary("sess-stale", "alpha", TEST_REPO_SWIMMERS);
    adopted.is_stale = false;
    adopted.transport_health = TransportHealth::Healthy;
    api.push_adopt_session(Ok(AdoptSessionResponse {
        session: adopted,
        repo_theme: None,
        reused_session_id: true,
    }));

    let layout = test_layout(100, 32);
    let field = layout.overview_field;
    let mut stale = session_summary("sess-stale", "alpha", TEST_REPO_SWIMMERS);
    stale.is_stale = true;
    stale.transport_health = TransportHealth::Disconnected;
    let mut app = make_app(api.clone());
    app.merge_sessions(vec![stale], field);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
    ));
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    assert_eq!(
        api.adopt_calls(),
        vec![("alpha".to_string(), Some("sess-stale".to_string()))]
    );
    assert_eq!(visible_entity_ids(&app), vec!["sess-stale".to_string()]);
    assert_eq!(app.selected_id.as_deref(), Some("sess-stale"));
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("reattached alpha")
    );
    assert!(!app.entities[0].session.is_stale);
    assert_eq!(
        app.entities[0].session.transport_health,
        TransportHealth::Healthy
    );
}

#[test]
fn submitting_batch_initial_request_creates_visible_sessions_without_native_open() {
    let api = MockApi::new();
    api.push_create_sessions_batch(Ok(create_batch_response(&[
        ("sess-alpha", "alpha", TEST_REPO_ALPHA),
        ("sess-beta", "beta", TEST_REPO_BETA),
    ])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.thought_group_by = ThoughtGroupBy::Pwd;
    app.thought_show_all = false;
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("alpha", true), ("beta", true)]),
        true,
        SpawnTool::Grok,
        None,
    ));
    app.initial_request = Some({
        let mut state = InitialRequestState::new_batch(
            vec![TEST_REPO_ALPHA.to_string(), TEST_REPO_BETA.to_string()],
            None,
        );
        state.value = "refactor shared logger".to_string();
        state
    });

    app.submit_initial_request(field);
    assert!(app.pending_interaction.is_some());
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_some());
    assert!(app.picker.is_some());
    assert!(app.selected_id.is_none());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_batch_calls(),
        vec![(
            vec![TEST_REPO_ALPHA.to_string(), TEST_REPO_BETA.to_string()],
            SpawnTool::Grok,
            Some("refactor shared logger".to_string()),
        )]
    );
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert_eq!(app.selected_id.as_deref(), Some("sess-beta"));
    assert!(app.picker.is_none());
    assert!(app.initial_request.is_none());
    assert_eq!(app.thought_group_by, ThoughtGroupBy::Batch);
    assert!(app.thought_show_all);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("created 2 sessions")
    );
    assert!(app
        .entities
        .iter()
        .any(|entity| entity.session.session_id == "sess-alpha"));
    assert!(app
        .entities
        .iter()
        .any(|entity| entity.session.session_id == "sess-beta"));
}

#[test]
fn submitting_group_input_sends_existing_sessions_without_spawning() {
    let api = MockApi::new();
    api.push_send_group_input(Ok(SessionGroupInputResponse::from_results(vec![
        SessionGroupInputResult {
            session_id: "sess-swimmers".to_string(),
            ok: true,
            error: None,
        },
        SessionGroupInputResult {
            session_id: "sess-skills".to_string(),
            ok: true,
            error: None,
        },
    ])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.group_input_targets = Some(GroupInputTargets {
        session_ids: vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
        label: "auth-rebuild".to_string(),
    });
    app.initial_request = Some({
        let mut state = InitialRequestState::new("auth-rebuild".to_string(), None);
        state.value = "keep going, same patch direction".to_string();
        state
    });

    app.submit_initial_request(field);
    assert!(app.pending_interaction.is_some());
    assert!(app.initial_request.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.send_group_input_calls(),
        vec![(
            vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
            "keep going, same patch direction".to_string(),
        )]
    );
    assert!(api.create_calls().is_empty());
    assert!(api.create_batch_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert!(app.group_input_targets.is_none());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("sent to 2 sessions")
    );
}

#[test]
fn submitting_group_input_keeps_composer_when_all_sessions_skip() {
    let api = MockApi::new();
    api.push_send_group_input(Ok(SessionGroupInputResponse::from_results(vec![
        SessionGroupInputResult {
            session_id: "sess-swimmers".to_string(),
            ok: false,
            error: Some(ErrorResponse {
                code: "SESSION_NOT_READY".to_string(),
                message: Some("session is not waiting for input".to_string()),
            }),
        },
        SessionGroupInputResult {
            session_id: "sess-skills".to_string(),
            ok: false,
            error: Some(ErrorResponse {
                code: "SESSION_NOT_READY".to_string(),
                message: Some("session is not waiting for input".to_string()),
            }),
        },
    ])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.group_input_targets = Some(GroupInputTargets {
        session_ids: vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
        label: "auth-rebuild".to_string(),
    });
    app.initial_request = Some({
        let mut state = InitialRequestState::new("auth-rebuild".to_string(), None);
        state.value = "retryable prompt".to_string();
        state
    });

    app.submit_initial_request(field);
    poll_until_interaction(&mut app);

    assert_eq!(
        api.send_group_input_calls(),
        vec![(
            vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
            "retryable prompt".to_string(),
        )]
    );
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|request| request.value.as_str()),
        Some("retryable prompt")
    );
    assert!(app.group_input_targets.is_some());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("sent to 0/2; all skipped")
    );
}

#[test]
fn pressing_enter_submits_group_input_and_reports_partial_skips() {
    let api = MockApi::new();
    api.push_send_group_input(Ok(SessionGroupInputResponse::from_results(vec![
        SessionGroupInputResult {
            session_id: "sess-swimmers".to_string(),
            ok: true,
            error: None,
        },
        SessionGroupInputResult {
            session_id: "sess-skills".to_string(),
            ok: false,
            error: Some(ErrorResponse {
                code: "SESSION_NOT_READY".to_string(),
                message: Some("session is not waiting for input".to_string()),
            }),
        },
    ])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.group_input_targets = Some(GroupInputTargets {
        session_ids: vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
        label: "auth-rebuild".to_string(),
    });
    app.initial_request = Some({
        let mut state = InitialRequestState::new("auth-rebuild".to_string(), None);
        state.value = "ship the partial path".to_string();
        state
    });

    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.send_group_input_calls(),
        vec![(
            vec!["sess-swimmers".to_string(), "sess-skills".to_string()],
            "ship the partial path".to_string(),
        )]
    );
    assert!(app.initial_request.is_none());
    assert!(app.group_input_targets.is_none());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("sent to 1/2; 1 skipped")
    );
}

#[test]
fn open_group_input_composer_blocks_attention_group_refresh_before_enter() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.attention_group_session_ids = vec!["sess-visible".to_string(), "sess-cleared".to_string()];
    app.group_input_targets = Some(GroupInputTargets {
        session_ids: vec!["sess-visible".to_string(), "sess-cleared".to_string()],
        label: "auth-rebuild".to_string(),
    });
    app.initial_request = Some({
        let mut state = InitialRequestState::new("auth-rebuild".to_string(), None);
        state.value = "typed but not submitted yet".to_string();
        state
    });

    let visible = session_summary("sess-visible", "7", TEST_REPO_SWIMMERS);
    let mut cleared = session_summary_with_thought(
        "sess-cleared",
        "8",
        TEST_REPO_SWIMMERS,
        "running a tool",
        "2026-05-12T20:00:00Z",
    );
    cleared.current_command = Some("cargo test".to_string());

    app.merge_sessions(vec![visible, cleared], layout.overview_field);

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|request| request.value.as_str()),
        Some("typed but not submitted yet")
    );
    assert!(app.group_input_targets.is_some());
}

#[test]
fn successful_group_input_does_not_refresh_attention_group_until_click() {
    let api = MockApi::new();
    api.push_send_group_input(Ok(SessionGroupInputResponse::from_results(vec![
        SessionGroupInputResult {
            session_id: "sess-visible".to_string(),
            ok: true,
            error: None,
        },
        SessionGroupInputResult {
            session_id: "sess-cleared".to_string(),
            ok: true,
            error: None,
        },
    ])));
    let field = test_field();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    app.attention_group_session_ids = vec!["sess-visible".to_string(), "sess-cleared".to_string()];
    app.group_input_targets = Some(GroupInputTargets {
        session_ids: vec!["sess-visible".to_string(), "sess-cleared".to_string()],
        label: "auth-rebuild".to_string(),
    });
    app.initial_request = Some({
        let mut state = InitialRequestState::new("auth-rebuild".to_string(), None);
        state.value = "send this to the group".to_string();
        state
    });

    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);
    poll_until_interaction(&mut app);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("sent to 2 sessions")
    );

    let visible = session_summary("sess-visible", "7", TEST_REPO_SWIMMERS);
    let mut cleared = session_summary_with_thought(
        "sess-cleared",
        "8",
        TEST_REPO_SWIMMERS,
        "running a tool",
        "2026-05-12T20:00:00Z",
    );
    cleared.current_command = Some("cargo test".to_string());
    let next = session_summary("sess-next", "9", TEST_REPO_SWIMMERS);

    app.merge_sessions(vec![visible, cleared, next], layout.overview_field);

    assert!(app.pending_interaction.is_none());
    assert!(api.open_attention_group_calls().is_empty());
    assert_eq!(
        app.attention_group_session_ids,
        vec!["sess-visible".to_string(), "sess-cleared".to_string()]
    );
}

#[test]
fn pasting_initial_request_buffers_multiline_without_submitting() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: String::new(),
        batch_dirs: None,
        launch_target: None,
    });

    app.handle_paste(pasted);

    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some(pasted)
    );
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
}

#[test]
fn pressing_enter_after_pasting_initial_request_submits_once() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    let pasted = "it happened when i pasted a bunch of text\n### TC-6\n- Given: foo";
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: String::new(),
        batch_dirs: None,
        launch_target: None,
    });

    app.handle_paste(pasted);
    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);
    assert!(app.pending_interaction.is_some());
    assert!(app.initial_request.is_some());
    assert!(app.selected_id.is_none());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Grok,
            Some(pasted.to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
}

#[cfg(not(feature = "voice"))]
#[test]
fn ctrl_v_in_initial_request_reports_voice_feature_when_not_built() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    app.open_initial_request(TEST_REPO_SWIMMERS.to_string(), None);

    app.handle_initial_request_key(
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        field,
    );

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("voice support is not built; rebuild with `--features voice`")
    );
    assert!(matches!(app.voice_state, VoiceUiState::Failed(_)));
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("")
    );
}

#[cfg(feature = "voice")]
#[test]
fn opening_initial_request_reports_voice_setup_gap_when_model_is_unset() {
    let _lock = TEST_ENV_LOCK.lock().expect("env lock");
    let original_model = env::var("SWIMMERS_VOICE_MODEL").ok();
    env::remove_var("SWIMMERS_VOICE_MODEL");

    let api = MockApi::new();
    let mut app = make_app(api);
    app.open_initial_request(TEST_REPO_SWIMMERS.to_string(), None);

    assert_eq!(
        app.voice_state,
        VoiceUiState::Failed("set SWIMMERS_VOICE_MODEL to enable voice".to_string())
    );
    assert_eq!(toggle_hint(), "ctrl-v voice needs model");

    restore_env_var("SWIMMERS_VOICE_MODEL", original_model);
}

#[cfg(feature = "voice")]
#[test]
fn ctrl_v_in_initial_request_reports_missing_voice_model_when_feature_is_built() {
    let _lock = TEST_ENV_LOCK.lock().expect("env lock");
    let original_model = env::var("SWIMMERS_VOICE_MODEL").ok();
    env::remove_var("SWIMMERS_VOICE_MODEL");

    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api);
    app.open_initial_request(TEST_REPO_SWIMMERS.to_string(), None);

    app.handle_initial_request_key(
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        field,
    );

    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("set SWIMMERS_VOICE_MODEL to enable voice")
    );
    assert_eq!(
        app.voice_state,
        VoiceUiState::Failed("set SWIMMERS_VOICE_MODEL to enable voice".to_string())
    );
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("")
    );

    restore_env_var("SWIMMERS_VOICE_MODEL", original_model);
}

#[test]
fn submit_initial_request_waits_for_voice_transcription() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "draft the request".to_string(),
        batch_dirs: None,
        launch_target: None,
    });
    app.voice_state = VoiceUiState::Transcribing;

    app.submit_initial_request(field);

    assert!(api.create_calls().is_empty());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("wait for voice transcription to finish")
    );
    assert!(app.initial_request.is_some());
}

#[test]
fn stale_voice_transcription_result_is_dropped_after_reopening_composer() {
    let api = MockApi::new();
    let mut app = make_app(api);
    app.open_initial_request(TEST_REPO_SWIMMERS.to_string(), None);
    let stale_generation = app.initial_request_generation;
    app.close_initial_request();
    app.open_initial_request("/tmp/other".to_string(), None);

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_interaction = Some(rx);
    assert!(tx
        .send(PendingInteractionResult::VoiceTranscription {
            generation: stale_generation,
            response: Ok("hello from the old composer".to_string()),
        })
        .is_ok());

    poll_until_interaction(&mut app);

    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("")
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("voice transcript finished after the composer changed")
    );
}

#[test]
fn session_create_failure_does_not_attempt_native_open() {
    let api = MockApi::new();
    api.push_create_session(Err("tmux failed to start".to_string()));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]),
        true,
        SpawnTool::Codex,
        None,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "fix tmux startup".to_string(),
        batch_dirs: None,
        launch_target: None,
    });

    app.submit_initial_request(field);
    assert!(app.pending_interaction.is_some());
    assert!(app.initial_request.is_some());
    assert!(app.entities.is_empty());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Grok,
            Some("fix tmux startup".to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert!(app.entities.is_empty());
    assert_eq!(
        app.initial_request
            .as_ref()
            .map(|state| state.value.as_str()),
        Some("fix tmux startup")
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("tmux failed to start")
    );
}

#[test]
fn blank_initial_request_is_rejected_locally() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "   ".to_string(),
        batch_dirs: None,
        launch_target: None,
    });

    app.submit_initial_request(field);

    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("enter an initial request")
    );
}

#[test]
fn typing_initial_request_and_pressing_enter_still_creates_hidden_session() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-55", "55", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: String::new(),
        batch_dirs: None,
        launch_target: None,
    });

    for ch in "add hidden spawn flow".chars() {
        app.handle_initial_request_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), field);
    }
    app.handle_initial_request_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), field);
    assert!(app.pending_interaction.is_some());
    assert!(app.initial_request.is_some());
    assert!(app.selected_id.is_none());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Grok,
            Some("add hidden spawn flow".to_string()),
        )]
    );
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert_eq!(app.selected_id.as_deref(), Some("sess-55"));
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("created 55")
    );
}

#[test]
fn esc_cancels_initial_request_without_creating_session() {
    let api = MockApi::new();
    let field = test_field();
    let mut app = make_app(api.clone());
    app.picker = Some(PickerState::new(
        10,
        10,
        dir_response(TEST_REPOS_ROOT, &[("swimmers", false)]),
        true,
        SpawnTool::Codex,
        None,
    ));
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "investigate snapshot restore".to_string(),
        batch_dirs: None,
        launch_target: None,
    });

    app.handle_initial_request_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), field);

    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert!(app.picker.is_some());
}

#[test]
fn paste_outside_initial_request_is_ignored() {
    let api = MockApi::new();
    let mut app = make_app(api.clone());
    app.selected_id = Some("sess-7".to_string());

    app.handle_paste("q\n### TC-7\n- Then: shell spill");

    assert_eq!(app.selected_id.as_deref(), Some("sess-7"));
    assert!(api.create_calls().is_empty());
    assert!(api.open_calls().is_empty());
    assert!(app.initial_request.is_none());
    assert!(app.picker.is_none());
}

#[test]
fn clicking_existing_swimmer_still_opens_it_directly() {
    let api = MockApi::new();
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-7".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.sprite_theme_override = Some(SpriteTheme::Jelly);
    app.entities
        .push(entity_at(field, "sess-7", "dev", TEST_REPO_DEV, 30, 8));
    app.selected_id = Some("sess-7".to_string());

    app.handle_field_click(30, 8, field);
    assert!(app.pending_interaction.is_some());
    assert_eq!(app.selected_id.as_deref(), Some("sess-7"));
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("opening dev...")
    );

    poll_until_interaction(&mut app);

    assert!(api.list_calls().is_empty());
    assert!(api.create_calls().is_empty());
    assert_eq!(api.open_calls(), vec!["sess-7".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused dev")
    );
}

#[test]
fn clicking_existing_swimmer_ignores_second_click_while_open_is_pending() {
    let api = MockApi::new();
    api.push_open_session(Ok(NativeDesktopOpenResponse {
        session_id: "sess-7".to_string(),
        status: "focused".to_string(),
        pane_id: None,
    }));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.sprite_theme_override = Some(SpriteTheme::Jelly);
    app.entities
        .push(entity_at(field, "sess-7", "dev", TEST_REPO_DEV, 30, 8));
    app.selected_id = Some("sess-7".to_string());

    app.handle_field_click(30, 8, field);
    app.handle_field_click(30, 8, field);

    assert!(app.pending_interaction.is_some());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("wait for the current action to finish")
    );

    poll_until_interaction(&mut app);

    assert_eq!(api.open_calls(), vec!["sess-7".to_string()]);
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("focused dev")
    );
}

#[test]
fn filtered_out_swimmers_are_not_click_targets() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("swimmers", true)])));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.entities
        .push(entity_at(field, "sess-1", "2", TEST_REPO_SWIMMERS, 12, 6));
    app.entities
        .push(entity_at(field, "sess-3", "9", TEST_REPO_SKILLS, 30, 8));
    app.selected_id = Some("sess-3".to_string());

    app.set_thought_filter_cwd(TEST_REPO_SWIMMERS.to_string());
    app.handle_field_click(30, 8, field);
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(visible_entity_ids(&app), vec!["sess-1".to_string()]);
    assert_eq!(app.selected_id.as_deref(), Some("sess-1"));
    assert!(api.open_calls().is_empty());
    assert!(app.picker.is_some());
}

#[test]
fn refresh_clears_selection_when_filters_hide_all_sessions() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary("sess-3", "9", TEST_REPO_SKILLS)]));
    let mut app = make_app(api.clone());
    app.merge_sessions(
        vec![
            session_summary("sess-1", "7", TEST_REPO_SWIMMERS),
            session_summary("sess-2", "2", TEST_REPO_SWIMMERS),
        ],
        layout.overview_field,
    );
    app.selected_id = Some("sess-1".to_string());
    app.set_thought_filter_cwd(TEST_REPO_SWIMMERS.to_string());

    app.refresh(layout);
    poll_until_selection_publication(&mut app);

    assert!(app.visible_entities().is_empty());
    assert!(app.selected_id.is_none());
    assert_eq!(api.publish_calls(), vec![Some("sess-2".to_string()), None,]);

    app.open_selected();

    assert!(api.open_calls().is_empty());
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("no session selected")
    );
}

#[test]
fn refresh_publishes_selected_session_for_external_dispatch() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    api.push_fetch_sessions(Ok(vec![session_summary(
        "sess-swimmers",
        "7",
        TEST_REPO_SWIMMERS,
    )]));
    let mut app = make_app(api.clone());

    app.refresh(layout);
    poll_until_selection_publication(&mut app);

    assert_eq!(app.selected_id.as_deref(), Some("sess-swimmers"));
    assert_eq!(api.publish_calls(), vec![Some("sess-swimmers".to_string())]);
}

#[test]
fn refresh_keeps_cached_repo_theme_when_session_still_references_it() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let theme_id = "/tmp/buildooor".to_string();
    let mut session = session_summary("sess-buildooor", "7", "/tmp/buildooor/src");
    session.repo_theme_id = Some(theme_id.clone());
    api.push_fetch_sessions(Ok(vec![session]));

    let mut app = make_app(api);
    app.repo_themes
        .insert(theme_id.clone(), repo_theme("#B89875"));

    app.refresh(layout);

    assert_eq!(
        app.repo_themes.get(&theme_id).expect("theme retained").body,
        "#B89875"
    );
    assert_eq!(
        session_display_color(&app.entities[0].session, &app.repo_themes),
        repo_theme_display_color("#B89875").expect("display color")
    );
}

#[test]
fn picker_action_at_resolves_controls_and_entries() {
    let mut picker = PickerState::new(
        4,
        4,
        dir_response("/tmp", &[("alpha", true), ("beta", false)]),
        true,
        SpawnTool::Codex,
        None,
    );
    picker.apply_response(dir_response("/tmp/nested", &[("child", false)]), false);
    let layout = picker_layout(&picker, test_field());

    assert!(matches!(
        picker_action_at(
            &picker,
            &layout,
            layout.close_button.x,
            layout.close_button.y
        ),
        Some(PickerAction::Close)
    ));
    assert!(matches!(
        picker_action_at(&picker, &layout, layout.env_button.x, layout.env_button.y),
        Some(PickerAction::ToggleManaged(true))
    ));
    assert!(matches!(
        picker_action_at(&picker, &layout, layout.all_button.x, layout.all_button.y),
        Some(PickerAction::ToggleManaged(false))
    ));
    assert!(matches!(
        picker_action_at(
            &picker,
            &layout,
            layout.spawn_here_button.x,
            layout.spawn_here_button.y
        ),
        Some(PickerAction::ActivateCurrentPath)
    ));
    assert!(matches!(
        picker_action_at(&picker, &layout, layout.content.x, layout.first_entry_y),
        Some(PickerAction::ActivateEntry(0))
    ));
    assert!(picker_action_at(
        &picker,
        &layout,
        layout.content.right(),
        layout.first_entry_y
    )
    .is_none());
    assert!(matches!(
        layout
            .back_button
            .and_then(|button| picker_action_at(&picker, &layout, button.x, button.y)),
        Some(PickerAction::Up)
    ));
    assert!(matches!(
        picker_action_at(&picker, &layout, layout.tool_button.x, layout.tool_button.y),
        Some(PickerAction::ToggleTool)
    ));
    assert!(matches!(
        picker_action_at(
            &picker,
            &layout,
            layout.launch_target_button.x,
            layout.launch_target_button.y
        ),
        Some(PickerAction::ToggleLaunchTarget)
    ));
    assert!(matches!(
        picker_action_at(
            &picker,
            &layout,
            layout.exclude_button.x,
            layout.exclude_button.y
        ),
        Some(PickerAction::ToggleBatchExcludeMode)
    ));
}

#[test]
fn toggle_launch_target_persists_across_picker_reopen() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response_with_launch_targets(
        TEST_REPOS_ROOT,
        &[("swimmers", false)],
    )));
    api.push_list_dirs(Ok(dir_response_with_launch_targets(
        TEST_REPOS_ROOT,
        &[("swimmers", false)],
    )));
    let field = test_field();
    let mut app = make_app(api.clone());

    app.handle_field_click(10, 10, field);
    poll_until_interaction(&mut app);

    assert_eq!(app.launch_target.as_deref(), Some("local"));
    app.handle_picker_action(PickerAction::ToggleLaunchTarget, field);
    assert_eq!(app.launch_target.as_deref(), Some("jeremy-skillbox"));

    app.close_picker();
    app.handle_field_click(10, 10, field);
    poll_until_interaction(&mut app);

    assert_eq!(
        app.picker
            .as_ref()
            .and_then(|picker| picker.launch_target.as_deref()),
        Some("jeremy-skillbox")
    );
    assert_eq!(
        api.list_targets(),
        vec![None, Some("jeremy-skillbox".to_string())]
    );
}

#[test]
fn picker_reload_without_preserve_uses_response_launch_default() {
    let mut picker = PickerState::new(
        4,
        4,
        dir_response_with_launch_targets("/tmp", &[("alpha", true)]),
        true,
        SpawnTool::Codex,
        None,
    );
    assert_eq!(picker.launch_target.as_deref(), Some("local"));

    let mut response = dir_response_with_launch_targets("/tmp", &[("beta", true)]);
    response.default_launch_target = Some("jeremy-skillbox".to_string());
    picker.apply_response(response, false);

    assert_eq!(picker.launch_target.as_deref(), Some("jeremy-skillbox"));
}

#[test]
fn picker_action_at_prefers_repo_action_badges() {
    let picker = PickerState::new(
        4,
        4,
        DirListResponse {
            path: TEST_REPOS_ROOT.to_string(),
            entries: vec![repo_dir_entry("swimmers", true, Some(true), None)],
            overlay_label: None,
            groups: Vec::new(),
            launch_targets: Vec::new(),
            default_launch_target: None,
        },
        true,
        SpawnTool::Codex,
        None,
    );
    let layout = picker_layout(&picker, test_field());
    let label = "[commit]";
    let action_x = layout.content.right().saturating_sub(label.len() as u16);

    assert!(matches!(
        picker_action_at(&picker, &layout, action_x, layout.first_entry_y),
        Some(PickerAction::StartRepoAction(0, RepoActionKind::Commit))
    ));
}

#[test]
fn toggle_tool_switches_spawn_tool_and_persists_across_picker_reopen() {
    let api = MockApi::new();
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("swimmers", false)])));
    api.push_list_dirs(Ok(dir_response(TEST_REPOS_ROOT, &[("swimmers", false)])));
    let field = test_field();
    let mut app = make_app(api);

    app.handle_field_click(10, 10, field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    assert_eq!(app.spawn_tool, SpawnTool::Grok);
    assert_eq!(
        app.picker.as_ref().map(|p| p.spawn_tool),
        Some(SpawnTool::Grok)
    );

    app.handle_picker_action(PickerAction::ToggleTool, field);
    assert_eq!(app.spawn_tool, SpawnTool::Claude);
    assert_eq!(
        app.picker.as_ref().map(|p| p.spawn_tool),
        Some(SpawnTool::Claude)
    );

    app.close_picker();
    app.handle_field_click(10, 10, field);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    assert_eq!(
        app.picker.as_ref().map(|p| p.spawn_tool),
        Some(SpawnTool::Claude)
    );
}

#[test]
fn picker_commit_action_calls_api_and_preserves_selection() {
    let api = MockApi::new();
    api.push_start_repo_action(Ok(DirRepoActionResponse {
        ok: true,
        path: TEST_REPO_SWIMMERS.to_string(),
        status: RepoActionStatus {
            kind: RepoActionKind::Commit,
            state: RepoActionState::Running,
            detail: None,
        },
    }));
    api.push_list_dirs(Ok(DirListResponse {
        path: TEST_REPOS_ROOT.to_string(),
        entries: vec![repo_dir_entry(
            "swimmers",
            true,
            Some(true),
            Some(RepoActionStatus {
                kind: RepoActionKind::Commit,
                state: RepoActionState::Running,
                detail: None,
            }),
        )],
        overlay_label: None,
        groups: Vec::new(),
        launch_targets: Vec::new(),
        default_launch_target: None,
    }));

    let mut app = make_app(api.clone());
    let mut picker = PickerState::new(
        2,
        2,
        DirListResponse {
            path: TEST_REPOS_ROOT.to_string(),
            entries: vec![repo_dir_entry("swimmers", true, Some(true), None)],
            overlay_label: None,
            groups: Vec::new(),
            launch_targets: Vec::new(),
            default_launch_target: None,
        },
        true,
        SpawnTool::Codex,
        None,
    );
    picker.selection = PickerSelection::Entry(0);
    app.picker = Some(picker);

    app.picker_start_action_for_selection(RepoActionKind::Commit);
    assert!(app.pending_interaction.is_some());
    poll_until_interaction(&mut app);

    assert_eq!(
        api.start_repo_action_calls(),
        vec![(TEST_REPO_SWIMMERS.to_string(), RepoActionKind::Commit)]
    );
    assert_eq!(
        app.picker.as_ref().map(|picker| picker.selection),
        Some(PickerSelection::Entry(0))
    );
    assert_eq!(
        app.picker
            .as_ref()
            .and_then(|picker| picker.entries.first())
            .and_then(|entry| entry.repo_action.as_ref())
            .map(|status| status.state),
        Some(RepoActionState::Running)
    );
    assert_eq!(
        app.message.as_ref().map(|(message, _)| message.as_str()),
        Some("commit started for swimmers")
    );
}

#[test]
fn spawn_session_uses_selected_tool() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-99", "99", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.spawn_tool = SpawnTool::Claude;
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "fix the build".to_string(),
        batch_dirs: None,
        launch_target: None,
    });

    app.submit_initial_request(field);
    assert!(app.pending_interaction.is_some());

    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_calls(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Claude,
            Some("fix the build".to_string()),
        )]
    );
}

#[test]
fn spawn_session_sends_selected_launch_target() {
    let api = MockApi::new();
    api.push_create_session(Ok(create_response("sess-100", "100", TEST_REPO_SWIMMERS)));
    let field = test_field();
    let mut app = make_app(api.clone());
    app.initial_request = Some(InitialRequestState {
        cwd: TEST_REPO_SWIMMERS.to_string(),
        value: "move this off laptop".to_string(),
        batch_dirs: None,
        launch_target: Some("jeremy-skillbox".to_string()),
    });

    app.submit_initial_request(field);
    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_calls_with_targets(),
        vec![(
            TEST_REPO_SWIMMERS.to_string(),
            SpawnTool::Grok,
            Some("jeremy-skillbox".to_string()),
            Some("move this off laptop".to_string()),
        )]
    );
}

#[test]
fn spawn_batch_sends_selected_launch_target() {
    let api = MockApi::new();
    api.push_create_sessions_batch(Ok(create_batch_response(&[(
        "sess-alpha",
        "alpha",
        TEST_REPO_ALPHA,
    )])));
    let field = test_field();
    let mut app = make_app(api.clone());
    let mut request = InitialRequestState::new_batch(
        vec![TEST_REPO_ALPHA.to_string()],
        Some("jeremy-skillbox".to_string()),
    );
    request.value = "fan out remotely".to_string();
    app.initial_request = Some(request);

    app.submit_initial_request(field);
    poll_until_interaction(&mut app);

    assert_eq!(
        api.create_batch_calls_with_targets(),
        vec![(
            vec![TEST_REPO_ALPHA.to_string()],
            SpawnTool::Grok,
            Some("jeremy-skillbox".to_string()),
            Some("fan out remotely".to_string()),
        )]
    );
}
