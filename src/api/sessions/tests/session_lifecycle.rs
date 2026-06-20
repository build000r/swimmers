use super::*;

#[tokio::test]
async fn create_session_requires_write_scope() {
    let response = create_session(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionRequest {
            name: None,
            cwd: None,
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn adopt_session_requires_write_scope() {
    let response = adopt_session(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(AdoptSessionRequest {
            tmux_name: "alpha".to_string(),
            session_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn adopt_session_rejects_already_tracked_tmux_without_duplication() {
    let state = test_state();
    let active = summary("sess-1", SessionState::Idle);
    let tmux_name = active.tmux_name.clone();
    let _rx = insert_summary_test_handle(&state, active.clone()).await;

    let response = adopt_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(AdoptSessionRequest {
            tmux_name,
            session_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = response_json(response).await;
    assert_eq!(json["code"], "TMUX_SESSION_ALREADY_TRACKED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("sess-1"));
}

#[tokio::test]
async fn create_session_rejects_unknown_non_local_launch_target_explicitly() {
    let response = create_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionRequest {
            name: None,
            // Remote launch now requires an explicit cwd; supply the current
            // dir (what launch_cwd used to inject implicitly) so this test
            // still reaches the unknown-launch-target check rather than the
            // missing-cwd validation that would otherwise preempt it.
            cwd: Some(
                std::env::current_dir()
                    .expect("current dir")
                    .to_string_lossy()
                    .into_owned(),
            ),
            spawn_tool: None,
            launch_target: Some("not-configured-target-for-test".to_string()),
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
    let message = json["message"].as_str().expect("message");
    assert!(
        message.contains("launch target 'not-configured-target-for-test' is not configured")
            || message.contains("no skillbox-config overlay is available"),
        "{message}"
    );
}

#[tokio::test]
async fn create_session_rejects_missing_cwd_as_validation_error() {
    let missing = tempdir().expect("tempdir").path().join("missing");
    let response = create_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionRequest {
            name: None,
            cwd: Some(missing.to_string_lossy().into_owned()),
            spawn_tool: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("cwd does not exist"));
}

#[tokio::test]
async fn delete_session_rejects_invalid_mode() {
    let response = delete_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
        Query(DeleteSessionQuery {
            mode: Some("invalid".to_string()),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
}

#[test]
fn delete_session_mode_parse_accepts_supported_modes() {
    assert!(matches!(
        parse_delete_session_mode(None),
        Ok(SessionDeleteMode::DetachBridge)
    ));
    assert!(matches!(
        parse_delete_session_mode(Some("detach_bridge")),
        Ok(SessionDeleteMode::DetachBridge)
    ));
    assert!(matches!(
        parse_delete_session_mode(Some("kill_tmux")),
        Ok(SessionDeleteMode::KillTmux)
    ));
}

#[tokio::test]
async fn delete_session_returns_not_found_for_missing_session() {
    let response = delete_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
        Query(DeleteSessionQuery { mode: None }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
    assert_eq!(json["message"], Value::Null);
}

#[tokio::test]
async fn delete_session_prefers_remote_namespace_error_over_local_session() {
    let state = test_state();
    let session_id =
        remote_sessions::namespace_session_id("not-configured-delete-target", "shadow");
    let _rx = insert_summary_test_handle(&state, summary(&session_id, SessionState::Idle)).await;

    let response = delete_session(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Path(session_id.clone()),
        Query(DeleteSessionQuery { mode: None }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
    assert!(state.supervisor.get_session(&session_id).await.is_some());
}

#[tokio::test]
async fn delete_session_error_response_maps_internal_errors() {
    let response = delete_session_error_response(anyhow::anyhow!("tmux kill failed"));

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("tmux kill failed"));
}
