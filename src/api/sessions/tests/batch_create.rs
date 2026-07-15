use super::*;

#[tokio::test]
async fn create_sessions_batch_requires_write_scope() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: vec!["/tmp/project".to_string()],
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_sessions_batch_rejects_empty_dirs() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: Vec::new(),
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not be empty");
}

#[tokio::test]
async fn create_remote_sessions_batch_response_maps_validation_errors() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: Vec::new(),
            spawn_tool: None,
            tmux_target: None,
            launch_target: Some("remote-target".to_string()),
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not be empty");
}

#[tokio::test]
async fn create_sessions_batch_rejects_blank_dirs() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: vec!["/tmp/project".to_string(), " \t\n".to_string()],
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "dirs must not include blank entries");
}

#[tokio::test]
async fn create_sessions_batch_rejects_oversized_batches() {
    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(CreateSessionsBatchRequest {
            dirs: (0..=BATCH_CREATE_MAX_DIRS)
                .map(|idx| format!("/tmp/project-{idx}"))
                .collect(),
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(
        json["message"],
        format!("dirs must include at most {BATCH_CREATE_MAX_DIRS} entries")
    );
}

#[tokio::test]
async fn create_sessions_batch_assigns_shared_batch_metadata() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);

    let response = create_sessions_batch(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(CreateSessionsBatchRequest {
            dirs,
            spawn_tool: None,
            tmux_target: None,
            launch_target: None,
            initial_request: Some("wire jwt refresh + tests".to_string()),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CREATED);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results");
    let first_batch = &results[0]["session"]["batch"];
    let second_batch = &results[1]["session"]["batch"];

    assert!(first_batch["id"]
        .as_str()
        .expect("batch id")
        .starts_with("batch-"));
    assert_eq!(second_batch["id"], first_batch["id"]);
    assert_eq!(first_batch["label"], "wire jwt refresh + tests");
    assert_eq!(first_batch["prompt_excerpt"], "wire jwt refresh + tests");
    assert_eq!(first_batch["index"], 0);
    assert_eq!(second_batch["index"], 1);
    assert_eq!(first_batch["total"], 2);
    assert_eq!(second_batch["total"], 2);
    assert!(first_batch["created_at"].is_string());

    cleanup_created_sessions(&state, &json).await;
}

#[tokio::test]
async fn create_sessions_batch_mr_permutation_preserves_cwd_result_classes() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");

    for (case_index, names) in generated_dir_name_sets().into_iter().enumerate() {
        let dirs = create_case_dirs(root.path(), case_index, &names);
        let reversed_dirs = dirs.iter().rev().cloned().collect::<Vec<_>>();

        let response = create_batch(state.clone(), dirs.clone()).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let forward_json = response_json(response).await;

        let response = create_batch(state.clone(), reversed_dirs).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let reversed_json = response_json(response).await;

        assert_eq!(
            cwd_result_classes(&forward_json),
            cwd_result_classes(&reversed_json)
        );

        cleanup_created_sessions(&state, &forward_json).await;
        cleanup_created_sessions(&state, &reversed_json).await;
    }
}

#[tokio::test]
async fn create_sessions_batch_mr_additive_valid_dir_increases_success_count() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let base_dirs = create_case_dirs(root.path(), 0, &["api".to_string(), "worker".to_string()]);
    let mut extended_dirs = base_dirs.clone();
    extended_dirs.extend(create_case_dirs(root.path(), 1, &["docs".to_string()]));

    let response = create_batch(state.clone(), base_dirs).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let base_json = response_json(response).await;

    let response = create_batch(state.clone(), extended_dirs).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let extended_json = response_json(response).await;

    assert_eq!(success_count(&extended_json), success_count(&base_json) + 1);
    assert_eq!(
        extended_json["results"].as_array().expect("results").len(),
        base_json["results"].as_array().expect("results").len() + 1
    );

    cleanup_created_sessions(&state, &base_json).await;
    cleanup_created_sessions(&state, &extended_json).await;
}

#[tokio::test]
async fn create_sessions_batch_mr_invalid_dir_injection_is_exclusive() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_tmux_dir, _path_guard) = install_fake_tmux(FAKE_TMUX_FOR_CREATE);
    let state = test_state();
    let root = tempdir().expect("tempdir");
    let valid_dirs = create_case_dirs(
        root.path(),
        0,
        &["frontend".to_string(), "backend".to_string()],
    );
    let missing_dir = root.path().join("missing").to_string_lossy().into_owned();
    let dirs = vec![
        valid_dirs[0].clone(),
        missing_dir.clone(),
        valid_dirs[1].clone(),
    ];

    let response = create_batch(state.clone(), dirs).await;
    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    let results = json["results"].as_array().expect("results");

    assert_eq!(results.len(), 3);
    assert_eq!(success_count(&json), 2);
    assert_eq!(results[1]["index"], 1);
    assert_eq!(results[1]["cwd"], missing_dir);
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["error"]["code"], "VALIDATION_FAILED");
    assert!(results[0]["session"]["session_id"].is_string());
    assert!(results[2]["session"]["session_id"].is_string());

    cleanup_created_sessions(&state, &json).await;
}
