use super::*;

#[tokio::test]
async fn remote_agent_context_response_returns_namespaced_success() {
    let (base_url, handle) = spawn_remote_agent_context_ok_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_agent_context_response(&target, "sess/remote?x#frag").await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "remote-test::sess/remote?x#frag");
    assert_eq!(json["available"], true);
    assert_eq!(json["user_task"], "remote task");
    handle.abort();
}

#[tokio::test]
async fn remote_agent_context_response_maps_remote_failure() {
    let (base_url, handle) = spawn_remote_agent_context_error_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_agent_context_response(&target, "missing-remote").await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "REMOTE_SESSION_REQUEST_FAILED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("missing remote session"));
    handle.abort();
}

#[tokio::test]
async fn get_agent_context_prefers_remote_namespace_error_over_local_session() {
    let state = test_state();
    let session_id =
        remote_sessions::namespace_session_id("not-configured-agent-context-target", "shadow");
    let _write_rx =
        insert_summary_test_handle(&state, summary(&session_id, SessionState::Idle)).await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path(session_id),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
}

#[tokio::test]
async fn get_agent_context_returns_codex_jsonl_snapshot() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempdir().expect("tempdir");
    let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("05")
        .join("07");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    std::fs::write(
            sessions_dir.join("rollout-target.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"build the workbench\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"command\\\":\\\"cargo test agent_context\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":777}},\"model_context_window\":258400}}\n"
            ),
        )
        .expect("target rollout");

    let state = test_state();
    let _write_rx =
        insert_summary_test_handle(&state, summary("sess-context", SessionState::Idle)).await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-context".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-context");
    assert_eq!(json["available"], true);
    assert_eq!(json["tool"], "Codex");
    assert_eq!(json["cwd"], "/tmp/project");
    assert_eq!(json["user_task"], "build the workbench");
    assert_eq!(json["turns"].as_array().unwrap().len(), 1);
    assert_eq!(json["turns"][0]["text"], "build the workbench");
    assert_eq!(json["current_tool"]["tool"], "exec");
    assert_eq!(json["current_tool"]["detail"], "cargo test agent_context");
    assert_eq!(json["recent_actions"][0]["tool"], "exec");
    assert_eq!(json["token_count"], 777);
    assert_eq!(json["context_limit"], 258400);
}

#[tokio::test]
async fn agent_context_read_response_returns_ok_for_successful_read() {
    let response = agent_context_read_response(Ok(agent_context_fixture("sess-read-ok")));

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-read-ok");
    assert_eq!(json["available"], true);
    assert_eq!(json["user_task"], "remote task");
}

#[tokio::test]
async fn get_agent_context_returns_internal_error_when_summary_lookup_fails() {
    let state = test_state();
    let worker = insert_dropping_summary_test_handle(&state, "sess-summary-error").await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-summary-error".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("session summary actor dropped reply"));
    worker.await.expect("summary worker");
}

#[tokio::test]
async fn agent_context_read_response_returns_internal_error_for_read_failure() {
    let response = agent_context_read_response(Err(anyhow::anyhow!("context read failed")));

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "context read failed");
}

#[tokio::test]
async fn fetch_transcript_remote_response_returns_namespaced_success() {
    let (base_url, handle) = spawn_remote_transcript_ok_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_transcript_response(
        &target,
        "remote-ready",
        TranscriptQuery {
            turn_id: Some("turn-1".to_string()),
            after: Some(7),
            limit: Some(3),
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "remote-test::remote-ready");
    assert_eq!(json["selected_turn_id"], "turn-1");
    assert_eq!(json["next_cursor"], 7);
    assert_eq!(json["records"][0]["byte_start"], 7);
    assert_eq!(json["records"][0]["byte_end"], 3);
    handle.abort();
}

#[tokio::test]
async fn fetch_transcript_remote_response_maps_remote_failure() {
    let (base_url, handle) = spawn_remote_transcript_error_server().await;
    let target = remote_agent_context_target(base_url);

    let response = remote_transcript_response(
        &target,
        "missing-remote",
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "REMOTE_SESSION_REQUEST_FAILED");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("missing remote transcript"));
    handle.abort();
}

#[tokio::test]
async fn fetch_transcript_response_prefers_remote_namespace_error_over_local_session() {
    let state = test_state();
    let session_id =
        remote_sessions::namespace_session_id("not-configured-transcript-target", "shadow");
    let _write_rx =
        insert_summary_test_handle(&state, summary(&session_id, SessionState::Idle)).await;

    let response = fetch_transcript_response(
        &state,
        &session_id,
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "LAUNCH_TARGET_UNKNOWN");
}

#[tokio::test]
async fn fetch_transcript_response_returns_not_found_for_missing_local_session() {
    let response = fetch_transcript_response(
        &test_state(),
        "missing-transcript",
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn fetch_transcript_response_returns_internal_error_when_summary_lookup_fails() {
    let state = test_state();
    let worker = insert_dropping_summary_test_handle(&state, "sess-summary-error").await;

    let response = fetch_transcript_response(
        &state,
        "sess-summary-error",
        TranscriptQuery {
            turn_id: None,
            after: None,
            limit: None,
        },
    )
    .await;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert!(json["message"]
        .as_str()
        .expect("message")
        .contains("session summary actor dropped reply"));
    worker.await.expect("summary worker");
}

#[tokio::test]
async fn fetch_transcript_read_response_returns_ok_for_successful_read() {
    let response = transcript_read_response(Ok(transcript_fixture("sess-read-ok")));

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-read-ok");
    assert_eq!(json["available"], true);
    assert_eq!(json["cwd"], "/remote/project");
}

#[tokio::test]
async fn fetch_transcript_read_response_returns_internal_error_for_read_failure() {
    let response = transcript_read_response(Err(anyhow::anyhow!("transcript read failed")));

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "transcript read failed");
}

#[tokio::test]
async fn fetch_transcript_get_returns_records_after_selected_user_turn() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmp = tempdir().expect("tempdir");
    let _home_guard = TestEnvVarGuard::set_path("HOME", tmp.path());
    let sessions_dir = tmp
        .path()
        .join(".codex")
        .join("sessions")
        .join("2026")
        .join("05")
        .join("10");
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
    std::fs::write(
            sessions_dir.join("rollout-transcript.jsonl"),
            [
                json!({"type": "session_meta", "payload": {"cwd": "/tmp/project"}}).to_string(),
                json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>skip me</environment_context>"}]}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "first turn"}}).to_string(),
                json!({"type": "response_item", "payload": {"type": "function_call", "name": "exec", "arguments": "{\"command\":\"cargo test first\"}"}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "user_message", "message": "second turn"}}).to_string(),
                json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "working after second"}}).to_string(),
            ]
            .join("\n")
                + "\n",
        )
        .expect("target rollout");

    let state = test_state();
    let _write_rx =
        insert_summary_test_handle(&state, summary("sess-transcript", SessionState::Idle)).await;

    let context_response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Path("sess-transcript".to_string()),
    )
    .await;
    assert_eq!(context_response.status(), StatusCode::OK);
    let context_json = response_json(context_response).await;
    let turns = context_json["turns"].as_array().expect("turns");
    assert_eq!(
        turns
            .iter()
            .map(|turn| turn["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["first turn", "second turn"]
    );
    assert!(
        !turns.iter().any(|turn| turn["text"]
            .as_str()
            .unwrap()
            .contains("environment_context")),
        "system/environment records must not appear as user turns"
    );

    let first_turn_id = turns[0]["id"].as_str().unwrap().to_string();
    let response = get_transcript(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-transcript".to_string()),
        Query(TranscriptQuery {
            turn_id: Some(first_turn_id),
            after: None,
            limit: Some(10),
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["available"], true);
    assert_eq!(json["selected_turn"]["text"], "first turn");
    let records = json["records"].as_array().expect("records");
    assert_eq!(records[0]["kind"], "function_call");
    assert!(records[0]["summary"]
        .as_str()
        .unwrap()
        .contains("cargo test first"));
    assert!(
        records
            .iter()
            .any(|record| record["summary"].as_str().unwrap().contains("second turn")),
        "stream should include later JSONL records after the selected turn"
    );
    assert!(json["next_cursor"].as_u64().unwrap() > turns[0]["byte_end"].as_u64().unwrap());
}

#[tokio::test]
async fn get_agent_context_returns_unavailable_for_unsupported_tool() {
    let state = test_state();
    let mut unsupported = summary("sess-shell", SessionState::Idle);
    unsupported.tool = Some("shell".to_string());
    unsupported.context_limit = 0;
    let _write_rx = insert_summary_test_handle(&state, unsupported).await;

    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-shell".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-shell");
    assert_eq!(json["available"], false);
    assert_eq!(json["tool"], "shell");
    assert_eq!(json["recent_actions"].as_array().unwrap().len(), 0);
    assert!(json["message"].as_str().unwrap().contains("not supported"));
}

#[tokio::test]
async fn get_agent_context_returns_not_found_for_missing_session() {
    let response = get_agent_context(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("missing-context".to_string()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}
