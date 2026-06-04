use super::*;

#[tokio::test]
async fn send_group_input_rejects_empty_session_ids() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: Vec::new(),
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "session_ids must not be empty");
}

#[tokio::test]
async fn send_group_input_rejects_whitespace_text() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["one".to_string(), "two".to_string()],
            text: " \n\t ".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "text must not be empty");
}

#[tokio::test]
async fn send_group_input_rejects_fewer_than_two_unique_session_ids() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["only".to_string(), "only".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(
        json["message"],
        "session_ids must include at least two unique sessions"
    );
}

#[tokio::test]
async fn send_group_input_returns_not_found_for_all_missing_sessions() {
    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["missing-a".to_string(), "missing-b".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["skipped"], 2);
    let results = json["results"].as_array().expect("results");
    assert_eq!(results[0]["session_id"], "missing-a");
    assert_eq!(results[0]["ok"], false);
    assert_eq!(results[0]["error"]["code"], "SESSION_NOT_FOUND");
    assert_eq!(results[1]["session_id"], "missing-b");
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["error"]["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn send_group_input_sends_only_ready_sessions() {
    let state = test_state();

    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let mut busy = with_test_batch(summary("busy", SessionState::Busy), "batch-group");
    busy.rest_state = RestState::Active;

    let (ready_cmd_tx, mut ready_cmd_rx) = mpsc::channel(8);
    let (ready_write_tx, mut ready_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "ready",
            "tmux-ready",
            ready_cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = ready_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(ready.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = ready_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = ready_write_tx.send(data).await;
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                }
                _ => {}
            }
        }
    });

    let (busy_cmd_tx, mut busy_cmd_rx) = mpsc::channel(8);
    let (busy_write_tx, mut busy_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("busy", "tmux-busy", busy_cmd_tx))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = busy_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(busy.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = busy_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = busy_write_tx.send(data).await;
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                }
                _ => {}
            }
        }
    });

    state
        .supervisor
        .persist_thought(
            "ready",
            Some("waiting for direction"),
            0,
            192_000,
            ThoughtState::Sleeping,
            ThoughtSource::Llm,
            RestState::Sleeping,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec![
                "ready".to_string(),
                "ready".to_string(),
                "busy".to_string(),
                "missing".to_string(),
                remote_sessions::namespace_session_id("jeremy-skillbox", "remote-ready"),
            ],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 3);
    let results = json["results"].as_array().expect("results");
    assert_eq!(results.len(), 4, "duplicate session IDs should be deduped");
    assert_eq!(results[3]["session_id"], "jeremy-skillbox::remote-ready");
    assert_eq!(results[3]["ok"], false);
    assert_eq!(results[3]["error"]["code"], "SESSION_NOT_FOUND");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    let duplicate_ready_write =
        tokio::time::timeout(Duration::from_millis(25), ready_write_rx.recv()).await;
    assert!(
        matches!(duplicate_ready_write, Err(_) | Ok(None)),
        "duplicate session IDs must not receive duplicate group input"
    );
    let busy_write = tokio::time::timeout(Duration::from_millis(25), busy_write_rx.recv()).await;
    assert!(
        matches!(busy_write, Err(_) | Ok(None)),
        "busy sessions must not receive group input"
    );
}

#[tokio::test]
async fn send_group_input_reports_failed_delivery_ack() {
    let state = test_state();
    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let failed = with_test_batch(summary("failed", SessionState::Idle), "batch-group");
    let mut ready_write_rx = insert_group_input_delivery_test_handle(
        &state,
        ready,
        Some(InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        }),
    )
    .await;
    let mut failed_write_rx = insert_group_input_delivery_test_handle(
        &state,
        failed,
        Some(InputDeliveryResult {
            delivered: false,
            method: "test",
            message: Some("pty write failed".to_string()),
        }),
    )
    .await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "failed".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(json["results"][1]["session_id"], "failed");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(json["results"][1]["error"]["code"], "INPUT_DELIVERY_FAILED");
    assert_eq!(json["results"][1]["error"]["message"], "pty write failed");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    assert_eq!(
        failed_write_rx.recv().await.expect("failed write"),
        b"continue\r\r".to_vec()
    );
}

#[tokio::test]
async fn send_group_input_reports_actor_unavailable_send_failure() {
    let state = test_state();
    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let mut ready_write_rx = insert_group_input_delivery_test_handle(
        &state,
        ready,
        Some(InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        }),
    )
    .await;

    let unavailable = with_test_batch(summary("unavailable", SessionState::Idle), "batch-group");
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "unavailable",
            "tmux-unavailable",
            cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        if let Some(SessionCommand::GetSummary(reply)) = cmd_rx.recv().await {
            let _ = reply.send(unavailable);
        }
    });

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "unavailable".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(json["results"][1]["session_id"], "unavailable");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_FOUND");
    assert_eq!(json["results"][1]["error"]["message"], "channel closed");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
}

#[tokio::test]
async fn send_group_input_reports_dropped_delivery_ack() {
    let state = test_state();
    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let dropped = with_test_batch(summary("dropped", SessionState::Idle), "batch-group");
    let mut ready_write_rx = insert_group_input_delivery_test_handle(
        &state,
        ready,
        Some(InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        }),
    )
    .await;
    let mut dropped_write_rx = insert_group_input_delivery_test_handle(&state, dropped, None).await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "dropped".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][0]["ok"], true);
    assert_eq!(json["results"][1]["session_id"], "dropped");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(
        json["results"][1]["error"]["code"],
        "INPUT_DELIVERY_UNKNOWN"
    );
    assert_eq!(
        json["results"][1]["error"]["message"],
        "session actor dropped input delivery ack"
    );
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    assert_eq!(
        dropped_write_rx.recv().await.expect("dropped write"),
        b"continue\r\r".to_vec()
    );
}

#[tokio::test]
async fn send_group_input_skips_stale_and_disconnected_sessions() {
    let state = test_state();

    let mut ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    ready.rest_state = RestState::Sleeping;
    let mut stale = with_test_batch(summary("stale", SessionState::Idle), "batch-group");
    stale.rest_state = RestState::Sleeping;
    stale.is_stale = true;
    let mut disconnected =
        with_test_batch(summary("disconnected", SessionState::Idle), "batch-group");
    disconnected.rest_state = RestState::Sleeping;
    disconnected.transport_health = TransportHealth::Disconnected;

    let mut ready_write_rx = insert_summary_test_handle(&state, ready).await;
    let mut stale_write_rx = insert_summary_test_handle(&state, stale).await;
    let mut disconnected_write_rx = insert_summary_test_handle(&state, disconnected).await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec![
                "ready".to_string(),
                "stale".to_string(),
                "disconnected".to_string(),
            ],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 2);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(json["results"][2]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    let stale_write = tokio::time::timeout(Duration::from_millis(25), stale_write_rx.recv()).await;
    assert!(
        matches!(stale_write, Err(_) | Ok(None)),
        "stale sessions must not receive group input"
    );
    let disconnected_write =
        tokio::time::timeout(Duration::from_millis(25), disconnected_write_rx.recv()).await;
    assert!(
        matches!(disconnected_write, Err(_) | Ok(None)),
        "disconnected sessions must not receive group input"
    );
}

#[tokio::test]
async fn send_group_input_skips_degraded_overloaded_and_unobserved_sessions() {
    let state = test_state();

    let mut ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    ready.rest_state = RestState::Sleeping;
    let mut degraded = with_test_batch(summary("degraded", SessionState::Idle), "batch-group");
    degraded.rest_state = RestState::Sleeping;
    degraded.transport_health = TransportHealth::Degraded;
    degraded.state_evidence = StateEvidence::unobserved("summary_cache_degraded");
    let mut overloaded = with_test_batch(
        summary("overloaded", SessionState::Attention),
        "batch-group",
    );
    overloaded.transport_health = TransportHealth::Overloaded;
    overloaded.state_evidence = StateEvidence::unobserved("summary_cache_overloaded");
    let mut unobserved = with_test_batch(summary("unobserved", SessionState::Idle), "batch-group");
    unobserved.rest_state = RestState::Sleeping;
    unobserved.state_evidence = StateEvidence::unobserved("initial_state");

    let mut ready_write_rx = insert_summary_test_handle(&state, ready).await;
    let degraded_write_rx = insert_summary_test_handle(&state, degraded).await;
    let overloaded_write_rx = insert_summary_test_handle(&state, overloaded).await;
    let unobserved_write_rx = insert_summary_test_handle(&state, unobserved).await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec![
                "ready".to_string(),
                "degraded".to_string(),
                "overloaded".to_string(),
                "unobserved".to_string(),
            ],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 3);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(json["results"][2]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(json["results"][3]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    for (mut rx, label) in [
        (degraded_write_rx, "degraded"),
        (overloaded_write_rx, "overloaded"),
        (unobserved_write_rx, "unobserved"),
    ] {
        let write = tokio::time::timeout(Duration::from_millis(25), rx.recv()).await;
        assert!(
            matches!(write, Err(_) | Ok(None)),
            "{label} sessions must not receive group input"
        );
    }
}

#[tokio::test]
async fn send_group_input_rejects_attention_deep_sleep_sessions() {
    let state = test_state();

    let ready = with_test_batch(summary("ready", SessionState::Idle), "batch-group");
    let mut deep_attention = with_test_batch(
        summary("deep-attention", SessionState::Attention),
        "batch-group",
    );
    deep_attention.rest_state = RestState::DeepSleep;

    let (ready_cmd_tx, mut ready_cmd_rx) = mpsc::channel(8);
    let (ready_write_tx, mut ready_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "ready",
            "tmux-ready",
            ready_cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = ready_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(ready.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = ready_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = ready_write_tx.send(data).await;
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                }
                _ => {}
            }
        }
    });

    let (deep_cmd_tx, mut deep_cmd_rx) = mpsc::channel(8);
    let (deep_write_tx, mut deep_write_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle(
            "deep-attention",
            "tmux-deep-attention",
            deep_cmd_tx,
        ))
        .await;
    tokio::spawn(async move {
        while let Some(cmd) = deep_cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(deep_attention.clone());
                }
                SessionCommand::WriteInput(bytes) => {
                    let _ = deep_write_tx.send(bytes).await;
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = deep_write_tx.send(data).await;
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                }
                _ => {}
            }
        }
    });

    state
        .supervisor
        .persist_thought(
            "ready",
            Some("waiting for direction"),
            0,
            192_000,
            ThoughtState::Sleeping,
            ThoughtSource::Llm,
            RestState::Sleeping,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["ready".to_string(), "deep-attention".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(response).await;
    assert_eq!(json["delivered"], 1);
    assert_eq!(json["skipped"], 1);
    assert_eq!(json["results"][1]["session_id"], "deep-attention");
    assert_eq!(json["results"][1]["ok"], false);
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_READY");
    assert_eq!(
        ready_write_rx.recv().await.expect("ready write"),
        b"continue\r\r".to_vec()
    );
    let deep_write = tokio::time::timeout(Duration::from_millis(25), deep_write_rx.recv()).await;
    assert!(
        matches!(deep_write, Err(_) | Ok(None)),
        "deep sleep sessions must not receive group input"
    );
}

#[tokio::test]
async fn send_group_input_rejects_unbatched_or_mixed_batch_groups() {
    let state = test_state();

    let unbatched = summary("unbatched", SessionState::Idle);
    let batch_a = with_test_batch(summary("batch-a", SessionState::Idle), "batch-a");
    let batch_b = with_test_batch(summary("batch-b", SessionState::Idle), "batch-b");

    for (session_id, summary) in [
        ("unbatched", unbatched),
        ("batch-a", batch_a),
        ("batch-b", batch_b),
    ] {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(
                session_id,
                format!("tmux-{session_id}"),
                cmd_tx,
            ))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSummary(reply) = cmd {
                    let _ = reply.send(summary.clone());
                }
            }
        });
    }

    let unbatched_response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state.clone()),
        Json(SessionGroupInputRequest {
            session_ids: vec!["unbatched".to_string(), "batch-a".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(unbatched_response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(unbatched_response).await;
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["skipped"], 2);
    assert_eq!(json["results"][0]["error"]["code"], "SESSION_NOT_IN_BATCH");
    assert_eq!(json["results"][1]["error"]["code"], "SESSION_NOT_IN_BATCH");

    let mixed_response = send_group_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Json(SessionGroupInputRequest {
            session_ids: vec!["batch-a".to_string(), "batch-b".to_string()],
            text: "continue".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(mixed_response.status(), StatusCode::MULTI_STATUS);
    let json = response_json(mixed_response).await;
    assert_eq!(json["delivered"], 0);
    assert_eq!(json["skipped"], 2);
    assert_eq!(
        json["results"][0]["error"]["code"],
        "SESSION_BATCH_MISMATCH"
    );
    assert_eq!(
        json["results"][1]["error"]["code"],
        "SESSION_BATCH_MISMATCH"
    );
}
