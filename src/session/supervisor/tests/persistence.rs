use super::*;

#[tokio::test]
async fn init_persistence_bumps_id_counter_from_thought_snapshot_ids() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    store
        .save_thought(
            "sess_42",
            Some("stale thought"),
            7,
            128_000,
            ThoughtState::Holding,
            ThoughtSource::CarryForward,
            RestState::Drowsy,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let allocated = supervisor.allocate_unique_session_id().await;
    assert_eq!(allocated, "sess_43");
}

#[tokio::test]
async fn init_persistence_keeps_persisted_session_id_progression() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    store
        .save_sessions(&[PersistedSession {
            session_id: "sess_7".to_string(),
            tmux_name: "7".to_string(),
            tmux_target: crate::tmux_target::TmuxTarget::Default,
            state: SessionState::Idle,
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            objective_fingerprint: None,
            batch: None,
            cwd: "/tmp".to_string(),
            last_activity_at: Utc::now(),
        }])
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let allocated = supervisor.allocate_unique_session_id().await;
    assert_eq!(allocated, "sess_8");
}

#[tokio::test]
async fn init_persistence_preserves_batch_membership_on_stale_sessions() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    store
        .save_sessions(&[PersistedSession {
            session_id: "sess_7".to_string(),
            tmux_name: "7".to_string(),
            tmux_target: crate::tmux_target::TmuxTarget::Default,
            state: SessionState::Idle,
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            objective_fingerprint: None,
            batch: Some(SessionBatchMembership {
                id: "batch-auth".to_string(),
                label: "auth-rebuild".to_string(),
                index: 0,
                total: 2,
                created_at: Utc::now(),
                prompt_excerpt: Some("auth-rebuild".to_string()),
            }),
            cwd: "/tmp".to_string(),
            last_activity_at: Utc::now(),
        }])
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let stale = supervisor.stale_sessions.read().await;
    let batch = stale[0].batch.as_ref().expect("batch membership");
    assert_eq!(batch.id, "batch-auth");
    assert_eq!(batch.label, "auth-rebuild");
    assert_eq!(batch.index, 0);
    assert_eq!(batch.total, 2);
    assert_eq!(
        stale[0].state_evidence.cause,
        SUMMARY_CAUSE_PERSISTENCE_STALE
    );
    assert!(stale[0].state_evidence.observed_at.is_none());
    assert_eq!(
        stale[0].state_evidence.confidence,
        crate::types::StateConfidence::Low
    );
}

#[tokio::test]
async fn init_persistence_hydrates_stale_session_from_thought_snapshot() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    let persisted_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:00Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let thought_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let objective_changed_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:02Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let action_cues = vec![commit_ready_cue()];

    store
        .save_sessions(&[PersistedSession {
            session_id: "sess_7".to_string(),
            tmux_name: "7".to_string(),
            tmux_target: crate::tmux_target::TmuxTarget::Default,
            state: SessionState::Idle,
            tool: Some("Codex".to_string()),
            token_count: 12,
            context_limit: 192_000,
            thought: Some("persisted thought".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: Some(persisted_at),
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: Some("rust".to_string()),
            objective_fingerprint: Some("old-objective".to_string()),
            batch: None,
            cwd: "/tmp".to_string(),
            last_activity_at: persisted_at,
        }])
        .await;
    store
        .save_thought(
            "sess_7",
            Some("snapshot thought"),
            88,
            256_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            true,
            action_cues.clone(),
            thought_at,
            ThoughtDeliveryState::default(),
            Some(objective_changed_at),
            Some("new-objective".to_string()),
        )
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let stale = supervisor.stale_sessions.read().await;
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].session_id, "sess_7");
    assert_eq!(stale[0].state, SessionState::Exited);
    assert_eq!(stale[0].thought.as_deref(), Some("snapshot thought"));
    assert_eq!(stale[0].thought_state, ThoughtState::Active);
    assert_eq!(stale[0].thought_source, ThoughtSource::Llm);
    assert_eq!(stale[0].thought_updated_at, Some(thought_at));
    assert_eq!(stale[0].rest_state, RestState::Active);
    assert_eq!(stale[0].token_count, 88);
    assert_eq!(stale[0].context_limit, 256_000);
    assert!(stale[0].commit_candidate);
    assert_eq!(stale[0].action_cues, action_cues);
    assert_eq!(stale[0].objective_changed_at, Some(objective_changed_at));
    assert_eq!(stale[0].last_skill.as_deref(), Some("rust"));
    assert_eq!(stale[0].last_activity_at, persisted_at);
}

#[tokio::test]
async fn persist_thought_preserves_supplied_updated_at() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);

    supervisor
        .persist_thought(
            "sess_1",
            Some("reading logs"),
            12,
            192_000,
            ThoughtState::Holding,
            ThoughtSource::Llm,
            RestState::Drowsy,
            false,
            Vec::new(),
            updated_at,
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        )
        .await;

    let thoughts = supervisor.thought_snapshots.read().await;
    let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
    assert_eq!(snapshot.updated_at, updated_at);
    assert_eq!(snapshot.thought.as_deref(), Some("reading logs"));
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_provider_coalesces_latest_thought_when_persist_queue_is_full() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let provider = SupervisorProvider::new_with_persist_queue_capacity(supervisor.clone(), 1);
    let first_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:01Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);
    let second_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:02Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);
    let third_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:03Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);

    assert!(provider.persist_thought(
        "sess_1",
        Some("first queued"),
        1,
        192_000,
        ThoughtState::Active,
        ThoughtSource::Llm,
        RestState::Active,
        false,
        Vec::new(),
        first_at,
        ThoughtDeliveryState {
            stream_instance_id: Some("stream-a".to_string()),
            emission_seq: 1,
        },
        None,
        Some("obj-1".to_string()),
    ));
    assert!(
        !provider.persist_thought(
            "sess_1",
            Some("second overflow"),
            2,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            second_at,
            ThoughtDeliveryState {
                stream_instance_id: Some("stream-a".to_string()),
                emission_seq: 2,
            },
            None,
            Some("obj-2".to_string()),
        ),
        "queue-full writes should be accepted for coalesced persistence but reported as degraded"
    );
    assert!(
        !provider.persist_thought(
            "sess_1",
            Some("third latest"),
            3,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            third_at,
            ThoughtDeliveryState {
                stream_instance_id: Some("stream-a".to_string()),
                emission_seq: 3,
            },
            None,
            Some("obj-3".to_string()),
        ),
        "overwriting an overflow slot remains a degraded durability path"
    );

    let pressure = supervisor.thought_persistence_backpressure_snapshot();
    assert_eq!(
        pressure.queue_capacity, 1,
        "snapshot must report the configured queue capacity, not the default"
    );
    assert_eq!(pressure.queue_depth, 1);
    assert_eq!(pressure.pending_count, 2);
    assert_eq!(pressure.overflow_slots, 1);
    assert_eq!(pressure.queue_full_count, 2);
    assert_eq!(pressure.coalesced_count, 1);
    assert_eq!(pressure.dropped_count, 0);

    assert!(
        supervisor
            .wait_for_pending_thought_persists(Duration::from_secs(1))
            .await,
        "queued and coalesced thought writes should drain"
    );

    let thoughts = supervisor.thought_snapshots.read().await;
    let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
    assert_eq!(snapshot.thought.as_deref(), Some("third latest"));
    assert_eq!(snapshot.token_count, 3);
    assert_eq!(snapshot.updated_at, third_at);
    assert_eq!(snapshot.delivery.emission_seq, 3);
    assert_eq!(
        snapshot.delivery.stream_instance_id.as_deref(),
        Some("stream-a")
    );
    drop(thoughts);

    let drained = supervisor.thought_persistence_backpressure_snapshot();
    assert_eq!(drained.pending_count, 0);
    assert_eq!(drained.overflow_slots, 0);
}

#[tokio::test]
async fn persist_thought_retains_objective_shift_timestamp_until_next_shift() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let shifted_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);
    let later_update = DateTime::parse_from_rfc3339("2026-03-08T14:00:09Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);

    supervisor
        .persist_thought(
            "sess_1",
            Some("reframed objective"),
            12,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            shifted_at,
            ThoughtDeliveryState::default(),
            Some(shifted_at),
            Some("obj-1".to_string()),
        )
        .await;
    supervisor
        .persist_thought(
            "sess_1",
            Some("continuing work"),
            14,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            later_update,
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        )
        .await;

    let thoughts = supervisor.thought_snapshots.read().await;
    let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
    assert_eq!(snapshot.updated_at, later_update);
    assert_eq!(snapshot.objective_changed_at, Some(shifted_at));
    assert_eq!(snapshot.thought.as_deref(), Some("continuing work"));
}

#[tokio::test]
async fn persist_registry_uses_actor_state_without_querying_tmux() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin");
    let command_file = dir.path().join("tmux-command.txt");
    write_executable(
        &bin_dir.join("tmux"),
        &format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" > \"{}\"\nexit 1\n",
            command_file.display()
        ),
    );
    let original_path = std::env::var_os("PATH");
    prepend_test_path(&bin_dir, original_path.as_deref());

    let store = FileStore::new(dir.path()).await.expect("file store");
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store.clone()).await;
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
        )
        .await;

    supervisor.persist_registry().await;
    restore_test_path(original_path);

    let persisted = store.load_sessions().await;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].session_id, "sess-live");
    assert!(
        !command_file.exists(),
        "persist_registry should not shell out to tmux"
    );
}

#[tokio::test]
async fn persist_registry_merges_direct_thought_snapshot_into_registry() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store.clone()).await;
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
        )
        .await;

    let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    supervisor
        .persist_thought(
            "sess-live",
            Some("reading logs"),
            12,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            true,
            Vec::new(),
            updated_at,
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        )
        .await;

    supervisor.persist_registry().await;

    let persisted = store.load_sessions().await;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].thought.as_deref(), Some("reading logs"));
    assert_eq!(persisted[0].thought_updated_at, Some(updated_at));
    assert_eq!(persisted[0].rest_state, RestState::Active);
    assert!(persisted[0].commit_candidate);
    assert_eq!(persisted[0].objective_fingerprint.as_deref(), Some("obj-1"));
}
