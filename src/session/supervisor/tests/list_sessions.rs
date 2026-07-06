use super::*;

#[tokio::test]
async fn list_sessions_merges_thought_snapshots_and_skips_exited_summaries() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
        )
        .await;
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
        )
        .await;

    supervisor.thought_snapshots.write().await.insert(
        "sess-live".to_string(),
        ThoughtSnapshot {
            thought: Some("checking logs".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: vec![commit_ready_cue()],
            objective_changed_at: Some(Utc::now()),
            objective_fingerprint: None,
            token_count: 44,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-live");
    assert_eq!(sessions[0].thought.as_deref(), Some("checking logs"));
    assert_eq!(sessions[0].thought_state, ThoughtState::Active);
    assert_eq!(sessions[0].token_count, 44);
    assert_eq!(sessions[0].action_cues, vec![commit_ready_cue()]);
    assert!(sessions[0].objective_changed_at.is_some());
}

#[tokio::test]
async fn list_sessions_resolves_repo_theme_after_thought_merge_when_theme_id_missing() {
    let repo = tempdir().expect("tempdir");
    write_test_repo_theme_colors(repo.path(), "#B89875");
    let expected_theme_id = repo.path().to_string_lossy().into_owned();
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-themed", SessionState::Idle);
    summary.cwd = expected_theme_id.clone();
    summary.repo_theme_id = None;
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "sess-themed".to_string(),
        test_thought_snapshot("checking themed repo", ThoughtState::Active),
    );

    let sessions = supervisor.list_sessions().await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].thought.as_deref(), Some("checking themed repo"));
    assert_eq!(
        sessions[0].repo_theme_id.as_deref(),
        Some(expected_theme_id.as_str())
    );
}

#[tokio::test]
async fn list_sessions_clears_repo_theme_id_after_thought_merge_when_theme_missing() {
    let repo = tempdir().expect("tempdir");
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-unthemed", SessionState::Idle);
    summary.cwd = repo.path().to_string_lossy().into_owned();
    summary.repo_theme_id = Some("/stale/theme".to_string());
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "sess-unthemed".to_string(),
        test_thought_snapshot("checking missing theme", ThoughtState::Active),
    );

    let sessions = supervisor.list_sessions().await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].thought.as_deref(),
        Some("checking missing theme")
    );
    assert_eq!(sessions[0].repo_theme_id, None);
}

#[tokio::test]
async fn startup_idle_session_only_sleeps_after_waiting_thought_snapshot() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let aged = DateTime::parse_from_rfc3339("2026-03-08T13:55:00Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let mut summary = test_summary("sess-startup", SessionState::Idle);
    summary.rest_state = RestState::Drowsy;
    summary.last_activity_at = aged;
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;

    let sessions = supervisor.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-startup");
    assert!(sessions[0].thought.is_none());
    assert_eq!(sessions[0].thought_state, ThoughtState::Holding);
    assert_eq!(sessions[0].rest_state, RestState::Drowsy);
    assert_eq!(sessions[0].last_activity_at, aged);

    let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    supervisor
        .persist_thought(
            "sess-startup",
            Some("Need your approval to continue."),
            12,
            192_000,
            ThoughtState::Sleeping,
            ThoughtSource::CarryForward,
            RestState::Sleeping,
            false,
            Vec::new(),
            updated_at,
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let sessions = supervisor.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].thought.as_deref(),
        Some("Need your approval to continue.")
    );
    assert_eq!(sessions[0].thought_state, ThoughtState::Sleeping);
    assert_eq!(sessions[0].thought_source, ThoughtSource::CarryForward);
    assert_eq!(sessions[0].rest_state, RestState::Sleeping);
    assert_eq!(sessions[0].thought_updated_at, Some(updated_at));
    assert_eq!(sessions[0].last_activity_at, aged);
}

#[tokio::test]
async fn list_sessions_merges_thought_snapshot_from_active_tmux_pane_batch_lookup() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
sep=$(printf '\037')
name=$(printf 'work\tspace')
printf '%s%s0%s1%s1.0:%%1\n' "$name" "$sep" "$sep" "$sep"
printf '%s%s1%s1%s1.1:%%2\n' "$name" "$sep" "$sep" "$sep"
;;
  *)
printf 'unexpected tmux command: %s\n' "${1-}" >&2
exit 1
;;
esac
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Idle);
    summary.tmux_name = "work\tspace".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work\tspace:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("pane two".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: None,
            token_count: 77,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;

    restore_test_path(original_path);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].thought.as_deref(), Some("pane two"));
    assert_eq!(sessions[0].thought_state, ThoughtState::Active);
    assert_eq!(sessions[0].rest_state, RestState::Active);
    assert_eq!(sessions[0].token_count, 77);
}

#[tokio::test]
async fn list_sessions_merges_active_tmux_pane_snapshot_for_non_config_target() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-} ${2-} ${3-}" in
  "-L isolated list-panes")
sep=$(printf '\037')
printf 'work%s1%s1%s1.1:%%2\n' "$sep" "$sep" "$sep"
;;
  *)
printf 'unexpected tmux command: %s\n' "$*" >&2
exit 1
;;
esac
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-isolated", SessionState::Idle);
    summary.tmux_name = "work".to_string();
    summary.tmux_target = crate::tmux_target::TmuxTarget::socket_name("isolated");
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("isolated pane".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: None,
            token_count: 99,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;

    restore_test_path(original_path);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].thought.as_deref(), Some("isolated pane"));
    assert_eq!(sessions[0].thought_state, ThoughtState::Active);
    assert_eq!(sessions[0].token_count, 99);
}

#[tokio::test]
async fn list_sessions_keeps_summary_when_active_tmux_pane_batch_lookup_fails() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
printf 'boom\n' >&2
exit 1
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Idle);
    summary.tmux_name = "work".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("pane two".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: None,
            token_count: 77,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;

    restore_test_path(original_path);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-live");
    assert_eq!(sessions[0].thought.as_deref(), None);
    assert_eq!(sessions[0].thought_state, ThoughtState::Holding);
}

#[tokio::test]
async fn list_sessions_skips_dropped_summary_replies() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(
            spawn_dropped_summary_handle("sess-drop", "tmux-drop", SessionState::Idle).await,
        )
        .await;

    let sessions = supervisor.list_sessions().await;

    assert!(sessions.is_empty());
}

#[tokio::test]
async fn list_sessions_keeps_cached_summary_when_live_reply_drops() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Idle);
    summary.tmux_name = "tmux-live".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;

    let initial = supervisor.list_sessions().await;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].transport_health, TransportHealth::Healthy);

    supervisor
        .insert_test_handle(
            spawn_dropped_summary_handle("sess-live", "tmux-live", SessionState::Idle).await,
        )
        .await;

    let sessions = supervisor.list_sessions().await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-live");
    assert_eq!(sessions[0].tmux_name, "tmux-live");
    assert_eq!(sessions[0].transport_health, TransportHealth::Degraded);
    assert_eq!(
        sessions[0].state_evidence.cause,
        SummaryFallbackReason::Dropped
            .cached_fallback()
            .expect("dropped fallback cause")
            .0
    );
    assert!(sessions[0].state_evidence.observed_at.is_none());
    assert!(!sessions[0].is_stale);
}

#[tokio::test]
async fn collect_live_summaries_keeps_cached_summary_when_live_reply_times_out() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-timeout", SessionState::Busy)).await,
        )
        .await;

    let initial = supervisor
        .collect_live_summaries(Duration::from_millis(10))
        .await;
    assert_eq!(initial.len(), 1);

    supervisor
        .insert_test_handle(spawn_hung_summary_handle("sess-timeout", "tmux-sess-timeout").await)
        .await;

    let sessions = supervisor
        .collect_live_summaries(Duration::from_millis(10))
        .await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-timeout");
    assert_eq!(sessions[0].transport_health, TransportHealth::Overloaded);
    assert_eq!(
        sessions[0].state_evidence.cause,
        SummaryFallbackReason::Timeout
            .cached_fallback()
            .expect("timeout fallback cause")
            .0
    );
    assert!(sessions[0].state_evidence.observed_at.is_none());
    assert!(!sessions[0].is_stale);
}

#[tokio::test]
async fn list_sessions_skips_closed_command_channels() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(spawn_closed_summary_handle("sess-closed", "").await)
        .await;

    let sessions = supervisor.list_sessions().await;

    assert!(sessions.is_empty());
}

#[tokio::test]
async fn collect_session_snapshots_uses_summary_snapshot_and_thought_cache() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(spawn_summary_handle(test_summary("sess-1", SessionState::Busy)).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "sess-1".to_string(),
        ThoughtSnapshot {
            thought: Some("building release".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: Some("obj-1".to_string()),
            token_count: 55,
            context_limit: 210_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let infos = supervisor.collect_session_snapshots().await;
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].session_id, "sess-1");
    assert!(infos[0].replay_text.ends_with("replay tail"));
    assert_eq!(infos[0].thought.as_deref(), Some("building release"));
    assert_eq!(infos[0].token_count, 55);
    assert_eq!(infos[0].objective_fingerprint.as_deref(), Some("obj-1"));
}

#[tokio::test]
async fn collect_session_snapshots_merges_thought_snapshot_from_active_tmux_pane_batch_lookup() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
sep=$(printf '\037')
name=$(printf 'work\tspace')
printf '%s%s0%s1%s1.0:%%1\n' "$name" "$sep" "$sep" "$sep"
printf '%s%s1%s1%s1.1:%%2\n' "$name" "$sep" "$sep" "$sep"
;;
  *)
printf 'unexpected tmux command: %s\n' "${1-}" >&2
exit 1
;;
esac
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Busy);
    summary.tmux_name = "work\tspace".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work\tspace:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("pane two".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: Some("obj-pane".to_string()),
            token_count: 88,
            context_limit: 199_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let infos = supervisor.collect_session_snapshots().await;

    restore_test_path(original_path);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].session_id, "sess-live");
    assert_eq!(infos[0].thought.as_deref(), Some("pane two"));
    assert_eq!(infos[0].thought_state, ThoughtState::Active);
    assert_eq!(infos[0].rest_state, RestState::Active);
    assert_eq!(infos[0].objective_fingerprint.as_deref(), Some("obj-pane"));
    assert_eq!(infos[0].token_count, 88);
}

#[tokio::test]
async fn collect_session_snapshots_fans_out_actor_requests_before_timeouts() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();

    for session_id in ["sess-a", "sess-b", "sess-c"] {
        supervisor
            .insert_test_handle(
                spawn_observed_hung_summary_handle(session_id, "", observed_tx.clone()).await,
            )
            .await;
    }
    drop(observed_tx);

    let collect = supervisor.collect_session_snapshots_with_timeout(Duration::from_secs(10));
    tokio::pin!(collect);
    let observations = async {
        let mut observed = Vec::new();
        for _ in 0..3 {
            observed.push(observed_rx.recv().await.expect("observed summary request"));
        }
        observed
    };
    tokio::pin!(observations);

    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            _ = &mut collect => panic!("hung actors should keep collection pending"),
            observed = &mut observations => observed,
        }
    })
    .await
    .expect("snapshot collection should request every actor before the first timeout");

    let observed: HashSet<_> = observed.into_iter().collect();
    let expected = HashSet::from_iter([
        "sess-a".to_string(),
        "sess-b".to_string(),
        "sess-c".to_string(),
    ]);
    assert_eq!(observed, expected);
}

#[tokio::test]
async fn collect_session_snapshots_bounds_in_flight_actor_requests() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let cap = super::super::THOUGHT_SNAPSHOT_COLLECTION_CONCURRENCY;

    for idx in 0..(cap + 1) {
        supervisor
            .insert_test_handle(
                spawn_observed_hung_summary_handle(&format!("sess-{idx}"), "", observed_tx.clone())
                    .await,
            )
            .await;
    }
    drop(observed_tx);

    let collect = supervisor.collect_session_snapshots_with_timeout(Duration::from_secs(10));
    tokio::pin!(collect);
    let observations = async {
        let mut observed = Vec::new();
        for _ in 0..cap {
            let session_id = tokio::time::timeout(Duration::from_secs(1), observed_rx.recv())
                .await
                .expect("bounded snapshot collection should fill the first batch")
                .expect("observed summary request");
            observed.push(session_id);
        }
        let extra_request =
            tokio::time::timeout(Duration::from_millis(100), observed_rx.recv()).await;
        (observed, extra_request)
    };
    tokio::pin!(observations);

    let (observed, extra_request) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::select! {
            _ = &mut collect => panic!("hung actors should keep collection pending"),
            result = &mut observations => result,
        }
    })
    .await
    .expect("snapshot collection should start only the bounded first batch");

    assert_eq!(observed.len(), cap);
    assert!(
        extra_request.is_err(),
        "snapshot collection should wait for an in-flight slot before requesting another actor"
    );
}
