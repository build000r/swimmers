use super::*;

#[tokio::test]
async fn send_input_rejects_empty_text() {
    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: String::new(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
}

#[tokio::test]
async fn send_input_rejects_blank_submit_text() {
    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: " \t\n".to_string(),
            submit: true,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = response_json(response).await;
    assert_eq!(json["code"], "VALIDATION_FAILED");
    assert_eq!(json["message"], "submitted text must not be blank");
}

#[tokio::test]
async fn send_input_rejects_oversized_text_before_session_lookup() {
    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "x".repeat(MAX_SESSION_INPUT_BYTES + 1),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_TOO_LARGE");
    assert_eq!(
        json["message"],
        format!("terminal input exceeds {MAX_SESSION_INPUT_BYTES} byte limit")
    );
}

#[tokio::test]
async fn send_input_returns_not_found_for_missing_session() {
    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn send_input_rejects_exited_session() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-exited", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let SessionCommand::GetSummary(reply) = cmd {
                let _ = reply.send(summary("sess-exited", SessionState::Exited));
                return;
            }
        }
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-exited".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_EXITED");
    worker.await.expect("worker");
}

#[tokio::test]
async fn send_input_forwards_text_to_session_actor() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::WriteInputAck { data, ack } => {
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                    return data;
                }
                _ => {}
            }
        }
        Vec::new()
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(worker.await.expect("worker"), b"status".to_vec());
}

#[tokio::test]
async fn send_input_submit_forwards_submit_line_to_session_actor() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::SubmitLineAck { text, ack } => {
                    let _ = ack.send(InputDeliveryResult {
                        delivered: true,
                        method: "test",
                        message: None,
                    });
                    return text;
                }
                _ => {}
            }
        }
        String::new()
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: true,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(worker.await.expect("worker"), "status");
}

#[tokio::test]
async fn send_input_reports_failed_delivery_ack() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::WriteInputAck { ack, .. } => {
                    let _ = ack.send(InputDeliveryResult {
                        delivered: false,
                        method: "test",
                        message: Some("pty write failed".to_string()),
                    });
                    return;
                }
                _ => {}
            }
        }
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_DELIVERY_FAILED");
    assert_eq!(json["message"], "pty write failed");
    worker.await.expect("worker");
}

#[tokio::test]
async fn send_input_reports_dropped_delivery_ack() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-1", "tmux-1", cmd_tx))
        .await;

    let worker = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary("sess-1", SessionState::Idle));
                }
                SessionCommand::WriteInputAck { ack, .. } => {
                    drop(ack);
                    return;
                }
                _ => {}
            }
        }
    });

    let response = send_input(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-1".to_string()),
        Json(SessionInputRequest {
            text: "status".to_string(),
            submit: false,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_DELIVERY_UNKNOWN");
    worker.await.expect("worker");
}

#[tokio::test]
async fn send_input_delivery_response_returns_success_payload() {
    let response = session_input_delivery_response(
        "sess-1".to_string(),
        InputDeliveryResult {
            delivered: true,
            method: "test",
            message: None,
        },
    );

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["session_id"], "sess-1");
    assert_eq!(json["delivered"], true);
    assert_eq!(json["delivery_method"], "test");
    assert_eq!(json["message"], Value::Null);
}

#[tokio::test]
async fn send_input_delivery_response_maps_failed_delivery() {
    let response = session_input_delivery_response(
        "sess-1".to_string(),
        InputDeliveryResult {
            delivered: false,
            method: "test",
            message: Some("pty write failed".to_string()),
        },
    );

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INPUT_DELIVERY_FAILED");
    assert_eq!(json["message"], "pty write failed");
}

#[tokio::test]
async fn send_input_delivery_response_flags_partial_delivery() {
    let response = session_input_delivery_response(
        "sess-1".to_string(),
        InputDeliveryResult {
            delivered: true,
            method: crate::session::actor::TMUX_PARTIAL_DELIVERY_METHOD,
            message: Some("input only partially delivered to tmux".to_string()),
        },
    );

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    // ok/delivered stay true (the some-vs-none contract), but partial is flagged
    // so a caller needing an all-or-nothing submit can retry without ok flipping
    // (swimmers-bjsu).
    assert_eq!(json["ok"], true);
    assert_eq!(json["delivered"], true);
    assert_eq!(json["partial"], true);
    assert_eq!(json["message"], "input only partially delivered to tmux");
}
