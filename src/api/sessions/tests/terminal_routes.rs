use super::*;

#[tokio::test]
async fn get_snapshot_returns_actor_snapshot() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-snap", "tmux-snap", cmd_tx))
        .await;

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let SessionCommand::GetSnapshot(reply) = cmd {
                let _ = reply.send(TerminalSnapshot {
                    session_id: "sess-snap".to_string(),
                    latest_seq: 9,
                    truncated: false,
                    screen_text: "hello from tmux".to_string(),
                });
                break;
            }
        }
    });

    let response = get_snapshot(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-snap".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-snap");
    assert_eq!(json["screen_text"], "hello from tmux");
}

#[tokio::test]
async fn get_snapshot_returns_not_found_for_missing_session() {
    let response = get_snapshot(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-missing".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn get_snapshot_returns_actor_unavailable_error() {
    let state = test_state();
    let (cmd_tx, cmd_rx) = mpsc::channel(1);
    drop(cmd_rx);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-dead", "tmux-dead", cmd_tx))
        .await;

    let response = get_snapshot(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-dead".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "session actor unavailable");
}

#[tokio::test]
async fn request_terminal_snapshot_detects_dropped_reply() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-drop", "tmux-drop", cmd_tx))
        .await;
    tokio::spawn(async move {
        if let Some(SessionCommand::GetSnapshot(reply)) = cmd_rx.recv().await {
            drop(reply);
        }
    });

    let handle = state
        .supervisor
        .get_session("sess-drop")
        .await
        .expect("test handle");
    let err = request_terminal_snapshot(&handle)
        .await
        .expect_err("reply should be dropped");

    assert_eq!(err, SnapshotRequestError::ReplyDropped);
}

#[tokio::test]
async fn snapshot_error_response_maps_timeout_detail() {
    let response = snapshot_error_response(SnapshotRequestError::Timeout);

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let json = response_json(response).await;
    assert_eq!(json["code"], "INTERNAL_ERROR");
    assert_eq!(json["message"], "snapshot request timed out");
}

#[tokio::test]
async fn list_sessions_perf_gate_batches_tmux_lookup_within_budget() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, _path_guard) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 0.20
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
work-3	1	1	0.0:%3
work-4	1	1	0.0:%4
work-5	1	1	0.0:%5
work-6	1	1	0.0:%6
EOF
    ;;
  display-message)
    sleep 0.20
    printf '0.0:%%1\n'
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
    );

    let state = test_state();
    let mut expected_ids = Vec::new();
    for index in 1..=6 {
        let session_id = format!("sess-{index}");
        let mut live_summary = summary(&session_id, SessionState::Idle);
        live_summary.tmux_name = format!("work-{index}");
        state
            .supervisor
            .insert_test_handle(spawn_summary_handle(live_summary).await)
            .await;
        expected_ids.push(session_id);
    }
    expected_ids.sort();

    let mut samples = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let Json(payload) = list_sessions(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(state.clone()),
        )
        .await
        .expect("session list should succeed");
        let elapsed = started.elapsed();
        samples.push(elapsed);

        let mut actual_ids = payload
            .sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect::<Vec<_>>();
        actual_ids.sort();
        assert_eq!(actual_ids, expected_ids);
    }

    let p95 = p95_duration(samples);
    eprintln!("/v1/sessions p95: {p95:?} (budget 500ms)");
    assert!(
        p95 < Duration::from_millis(500),
        "expected /v1/sessions p95 under 500ms, got {:?}",
        p95
    );
}

#[tokio::test]
async fn list_sessions_perf_gate_skips_hung_tmux_active_pane_lookup() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, _path_guard) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sleep 2
    cat <<'EOF'
work-1	1	1	0.0:%1
work-2	1	1	0.0:%2
EOF
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
    );

    let state = test_state();
    let mut expected_ids = Vec::new();
    for index in 1..=2 {
        let session_id = format!("sess-{index}");
        let mut live_summary = summary(&session_id, SessionState::Idle);
        live_summary.tmux_name = format!("work-{index}");
        state
            .supervisor
            .insert_test_handle(spawn_summary_handle(live_summary).await)
            .await;
        expected_ids.push(session_id);
    }

    let started = Instant::now();
    let Json(payload) = list_sessions(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(state),
    )
    .await
    .expect("session list should succeed when tmux stalls");
    let elapsed = started.elapsed();

    let mut actual_ids = payload
        .sessions
        .iter()
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();
    actual_ids.sort();
    expected_ids.sort();

    assert_eq!(actual_ids, expected_ids);
    assert!(
        elapsed < Duration::from_millis(900),
        "expected /v1/sessions to degrade gracefully when tmux list-panes stalls, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn get_pane_tail_returns_actor_text() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx))
        .await;

    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if let SessionCommand::GetPaneTail { lines, reply } = cmd {
                assert_eq!(lines, 300);
                let _ = reply.send("recent pane output".to_string());
                break;
            }
        }
    });

    let response = get_pane_tail(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-tail".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["session_id"], "sess-tail");
    assert_eq!(json["text"], "recent pane output");
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_actor_text() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    tokio::spawn(async move {
        if let Some(SessionCommand::GetPaneTail { lines, reply }) = cmd_rx.recv().await {
            assert_eq!(lines, PANE_TAIL_LINES);
            let _ = reply.send("recent pane output".to_string());
        }
    });

    let text = request_pane_tail_from_actor(&handle)
        .await
        .expect("pane tail");

    assert_eq!(text, "recent pane output");
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_actor_unavailable_when_send_fails() {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    drop(cmd_rx);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    let result = request_pane_tail_from_actor(&handle).await;

    assert!(matches!(result, Err(PaneTailError::ActorUnavailable)));
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_reply_dropped_when_actor_drops_reply() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    tokio::spawn(async move {
        if let Some(SessionCommand::GetPaneTail { lines, reply }) = cmd_rx.recv().await {
            assert_eq!(lines, PANE_TAIL_LINES);
            drop(reply);
        }
    });

    let result = request_pane_tail_from_actor(&handle).await;

    assert!(matches!(result, Err(PaneTailError::ReplyDropped)));
}

#[tokio::test]
async fn request_pane_tail_from_actor_returns_timed_out_when_actor_keeps_reply() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle("sess-tail", "tmux-tail", cmd_tx);

    tokio::spawn(async move {
        if let Some(SessionCommand::GetPaneTail { lines, reply }) = cmd_rx.recv().await {
            assert_eq!(lines, PANE_TAIL_LINES);
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(reply);
        }
    });

    let result = request_pane_tail_from_actor_with_timeout(&handle, Duration::from_millis(1)).await;

    assert!(matches!(result, Err(PaneTailError::TimedOut)));
}

#[tokio::test]
async fn dismiss_attention_requires_write_scope() {
    let response = dismiss_attention(
        Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
        State(test_state()),
        Path("sess-1".to_string()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn dismiss_attention_returns_not_found_for_unknown_session() {
    let response = dismiss_attention(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(test_state()),
        Path("missing".to_string()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = response_json(response).await;
    assert_eq!(json["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn dismiss_attention_forwards_command_and_returns_ok() {
    let state = test_state();
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    state
        .supervisor
        .insert_test_handle(ActorHandle::test_handle("sess-att", "tmux-att", cmd_tx))
        .await;

    let received = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if matches!(cmd, SessionCommand::DismissAttention) {
                return true;
            }
        }
        false
    });

    let response = dismiss_attention(
        Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
        State(state),
        Path("sess-att".to_string()),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["ok"], true);
    assert!(
        received.await.expect("worker"),
        "actor never saw DismissAttention"
    );
}
