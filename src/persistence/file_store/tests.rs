use super::*;
use fs2::FileExt;

fn fixed_utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("valid timestamp")
        .with_timezone(&Utc)
}

fn test_thought_snapshot(
    thought: Option<&str>,
    objective_changed_at: Option<DateTime<Utc>>,
) -> ThoughtSnapshot {
    ThoughtSnapshot {
        thought: thought.map(|value| value.to_string()),
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        rest_state: RestState::Active,
        commit_candidate: false,
        action_cues: Vec::new(),
        objective_changed_at,
        objective_fingerprint: None,
        token_count: 10,
        context_limit: 100,
        updated_at: fixed_utc("2026-01-01T00:00:00Z"),
        delivery: ThoughtDeliveryState::default(),
    }
}

#[tokio::test]
async fn dir_group_memberships_round_trip_through_cache_and_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("store");
    let mut memberships = DirGroupMemberships::default();
    memberships
        .groups
        .entry("frontend".to_string())
        .or_default()
        .include_paths
        .insert("/tmp/frontend".to_string());
    memberships
        .groups
        .entry("backend".to_string())
        .or_default()
        .exclude_paths
        .insert("/tmp/backend".to_string());

    store
        .save_dir_group_memberships(memberships.clone())
        .await
        .expect("save memberships");
    assert_eq!(store.load_dir_group_memberships().await, memberships);

    let reopened = FileStore::new(dir.path()).await.expect("reopen store");
    assert_eq!(reopened.load_dir_group_memberships().await, memberships);
}

#[tokio::test]
async fn dir_group_membership_updates_merge_concurrent_cache_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("store");

    let first_store = store.clone();
    let second_store = store.clone();
    let (first, second) = tokio::join!(
        async move {
            first_store
                .update_dir_group_memberships(|memberships| {
                    memberships
                        .groups
                        .entry("frontend".to_string())
                        .or_default()
                        .include_paths
                        .insert("/tmp/project-a".to_string());
                })
                .await
        },
        async move {
            second_store
                .update_dir_group_memberships(|memberships| {
                    memberships
                        .groups
                        .entry("backend".to_string())
                        .or_default()
                        .include_paths
                        .insert("/tmp/project-b".to_string());
                })
                .await
        }
    );

    first.expect("first update");
    second.expect("second update");

    let stored = store.load_dir_group_memberships().await;
    assert!(stored
        .groups
        .get("frontend")
        .expect("frontend delta")
        .include_paths
        .contains("/tmp/project-a"));
    assert!(stored
        .groups
        .get("backend")
        .expect("backend delta")
        .include_paths
        .contains("/tmp/project-b"));
}

#[tokio::test]
async fn startup_load_outcomes_distinguish_missing_and_corrupt_files() {
    let missing_dir = tempfile::tempdir().expect("missing tempdir");
    let missing_store = FileStore::new(missing_dir.path())
        .await
        .expect("missing store");
    let missing = missing_store.health_snapshot();
    assert!(missing.ok);
    assert_eq!(
        missing.load_outcomes[OP_SESSION_REGISTRY].status,
        PersistenceLoadStatus::Missing
    );
    assert_eq!(
        missing.load_outcomes[OP_THOUGHTS].status,
        PersistenceLoadStatus::Missing
    );

    let corrupt_dir = tempfile::tempdir().expect("corrupt tempdir");
    tokio::fs::write(
        corrupt_dir.path().join("session_registry.json"),
        "{not json",
    )
    .await
    .expect("write corrupt registry");
    let corrupt_store = FileStore::new(corrupt_dir.path())
        .await
        .expect("corrupt store still initializes with empty cache");
    let corrupt = corrupt_store.health_snapshot();

    assert!(!corrupt.ok);
    assert_eq!(
        corrupt.load_outcomes[OP_SESSION_REGISTRY].status,
        PersistenceLoadStatus::DecodeFailed
    );
    assert!(corrupt.load_outcomes[OP_SESSION_REGISTRY]
        .last_error
        .is_some());
    assert_eq!(
        corrupt.last_failed_operation.as_deref(),
        Some(OP_SESSION_REGISTRY)
    );
}

#[tokio::test]
async fn thought_config_missing_load_uses_default_and_records_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("store");

    assert_eq!(store.load_thought_config().await, ThoughtConfig::default());
    let health = store.health_snapshot();
    assert!(health.ok);
    assert_eq!(
        health.load_outcomes[OP_THOUGHT_CONFIG].status,
        PersistenceLoadStatus::Missing
    );
    assert_eq!(health.load_outcomes[OP_THOUGHT_CONFIG].records, Some(0));
}

#[tokio::test]
async fn thought_config_valid_load_normalizes_payload_and_records_success() {
    let dir = tempfile::tempdir().expect("tempdir");
    let raw_config = ThoughtConfig {
        backend: "grok-cli".to_string(),
        model: "local-model".to_string(),
        agent_prompt: Some(String::new()),
        ..ThoughtConfig::default()
    };
    tokio::fs::write(
        dir.path().join("thought_config.json"),
        serde_json::to_string(&raw_config).expect("serialize thought config"),
    )
    .await
    .expect("write thought config");

    let store = FileStore::new(dir.path()).await.expect("store");
    let loaded = store.load_thought_config().await;

    assert_eq!(loaded.backend, "grok");
    assert_eq!(loaded.model, "local-model");
    assert_eq!(loaded.agent_prompt, None);
    let health = store.health_snapshot();
    assert!(health.ok);
    assert_eq!(
        health.load_outcomes[OP_THOUGHT_CONFIG].status,
        PersistenceLoadStatus::Loaded
    );
    assert_eq!(health.load_outcomes[OP_THOUGHT_CONFIG].records, Some(1));
}

#[tokio::test]
async fn thought_config_invalid_load_uses_default_and_records_invalid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let invalid_config = ThoughtConfig {
        cadence_hot_ms: 1,
        ..ThoughtConfig::default()
    };
    tokio::fs::write(
        dir.path().join("thought_config.json"),
        serde_json::to_string(&invalid_config).expect("serialize thought config"),
    )
    .await
    .expect("write invalid thought config");

    let store = FileStore::new(dir.path())
        .await
        .expect("store initializes with invalid thought config");
    let health = store.health_snapshot();

    assert_eq!(store.load_thought_config().await, ThoughtConfig::default());
    assert!(!health.ok);
    assert_eq!(
        health.load_outcomes[OP_THOUGHT_CONFIG].status,
        PersistenceLoadStatus::Invalid
    );
    assert!(health.load_outcomes[OP_THOUGHT_CONFIG]
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("cadence_hot_ms"));
}

#[tokio::test]
async fn thought_config_corrupt_load_uses_default_and_records_decode_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::fs::write(dir.path().join("thought_config.json"), "{not json")
        .await
        .expect("write corrupt thought config");

    let store = FileStore::new(dir.path())
        .await
        .expect("store initializes with corrupt thought config");
    let health = store.health_snapshot();

    assert_eq!(store.load_thought_config().await, ThoughtConfig::default());
    assert!(!health.ok);
    assert_eq!(
        health.load_outcomes[OP_THOUGHT_CONFIG].status,
        PersistenceLoadStatus::DecodeFailed
    );
    assert!(health.load_outcomes[OP_THOUGHT_CONFIG].last_error.is_some());
}

#[tokio::test]
async fn save_sessions_failure_updates_health_and_later_success_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("store");
    let registry_path = dir.path().join("session_registry.json");
    std::fs::create_dir(&registry_path).expect("create directory at registry path");

    store.save_sessions(&[]).await;

    let failed = store.health_snapshot();
    assert!(!failed.ok);
    assert_eq!(failed.consecutive_failures, 1);
    assert_eq!(
        failed.last_failed_operation.as_deref(),
        Some(OP_SESSION_REGISTRY)
    );
    assert!(failed.last_failure_at.is_some());
    assert!(
        failed
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("rename failed"),
        "unexpected error: {:?}",
        failed.last_error
    );

    std::fs::remove_dir(&registry_path).expect("remove blocking directory");
    store.save_sessions(&[]).await;

    let recovered = store.health_snapshot();
    assert!(recovered.ok);
    assert_eq!(recovered.consecutive_failures, 0);
    assert_eq!(
        recovered.last_successful_operation.as_deref(),
        Some(OP_SESSION_REGISTRY)
    );
    assert!(recovered.last_success_at.is_some());
    assert_eq!(recovered.last_error, None);
}

#[tokio::test]
async fn write_success_does_not_clear_corrupt_load_condition() {
    let dir = tempfile::tempdir().expect("tempdir");
    tokio::fs::write(dir.path().join("session_registry.json"), "{not json")
        .await
        .expect("write corrupt registry");

    let store = FileStore::new(dir.path())
        .await
        .expect("store initializes with corrupt registry");

    let after_load = store.health_snapshot();
    assert!(
        !after_load.ok,
        "corrupt startup load should leave health degraded"
    );
    assert_eq!(
        after_load.load_outcomes[OP_SESSION_REGISTRY].status,
        PersistenceLoadStatus::DecodeFailed
    );

    // A successful write to a *different* file must not mask the corrupt
    // session registry that was observed at startup. The 30s checkpoint
    // must keep reporting the degraded condition until it is re-resolved.
    store
        .save_thought_config(&ThoughtConfig::default())
        .await
        .expect("thought config write succeeds");

    let after_write = store.health_snapshot();
    assert!(
        !after_write.ok,
        "a successful write must not clear a known corrupt-load condition"
    );
    assert_eq!(
        after_write.load_outcomes[OP_SESSION_REGISTRY].status,
        PersistenceLoadStatus::DecodeFailed
    );
}

#[test]
fn load_failure_status_classifies_transient_and_corrupt_errors() {
    let lock_busy = anyhow::Error::new(FileStoreIoError::LockBusy {
        path: PathBuf::from("/tmp/.lock"),
    });
    assert_eq!(
        load_failure_status(&lock_busy),
        PersistenceLoadStatus::ReadFailed,
        "a transient lock should not be reported as permanent corruption"
    );

    let checksum = anyhow::Error::new(FileStoreIoError::ChecksumMismatch {
        path: PathBuf::from("/tmp/session_registry.json"),
        expected: 1,
        actual: 2,
    });
    assert_eq!(
        load_failure_status(&checksum),
        PersistenceLoadStatus::DecodeFailed
    );

    let decode = anyhow::anyhow!("decode checksummed payload failed: trailing data");
    assert_eq!(
        load_failure_status(&decode),
        PersistenceLoadStatus::DecodeFailed
    );

    let read = anyhow::anyhow!("read failed: permission denied");
    assert_eq!(
        load_failure_status(&read),
        PersistenceLoadStatus::ReadFailed
    );
}

#[tokio::test]
async fn save_thought_failure_updates_health() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("store");
    std::fs::create_dir(dir.path().join("thoughts.json"))
        .expect("create directory at thoughts path");

    store
        .save_thought(
            "session-1",
            Some("thinking"),
            10,
            100,
            ThoughtState::Holding,
            ThoughtSource::CarryForward,
            RestState::Active,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let failed = store.health_snapshot();
    assert!(!failed.ok);
    assert_eq!(failed.consecutive_failures, 1);
    assert_eq!(failed.last_failed_operation.as_deref(), Some(OP_THOUGHTS));
    assert!(failed.last_failure_at.is_some());
    assert!(failed.last_error.is_some());
}

#[tokio::test]
async fn save_thought_self_heals_a_corrupt_thoughts_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A corrupt-but-readable thoughts.json used to wedge every subsequent write.
    tokio::fs::write(dir.path().join("thoughts.json"), "{not json")
        .await
        .expect("write corrupt thoughts");

    let store = FileStore::new(dir.path()).await.expect("store");
    store
        .save_thought(
            "session-a",
            Some("recovered thought"),
            10,
            100,
            ThoughtState::Holding,
            ThoughtSource::CarryForward,
            RestState::Active,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    // The write falls back to the in-memory snapshot, rewrites a valid payload,
    // and a reopened store round-trips it instead of staying wedged.
    let reopened = FileStore::new(dir.path()).await.expect("reopen store");
    let thoughts = reopened.load_thoughts().await;
    assert_eq!(
        thoughts
            .get("session-a")
            .and_then(|entry| entry.thought.as_deref()),
        Some("recovered thought")
    );
}

#[tokio::test]
async fn save_thought_merges_disk_state_from_other_store_instances() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = FileStore::new(dir.path()).await.expect("first store");
    let second = FileStore::new(dir.path()).await.expect("second store");

    first
        .save_thought(
            "session-a",
            Some("first thought"),
            10,
            100,
            ThoughtState::Holding,
            ThoughtSource::CarryForward,
            RestState::Active,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;
    second
        .save_thought(
            "session-b",
            Some("second thought"),
            20,
            200,
            ThoughtState::Holding,
            ThoughtSource::CarryForward,
            RestState::Active,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let reopened = FileStore::new(dir.path()).await.expect("reopen store");
    let thoughts = reopened.load_thoughts().await;
    assert_eq!(
        thoughts
            .get("session-a")
            .and_then(|entry| entry.thought.as_deref()),
        Some("first thought")
    );
    assert_eq!(
        thoughts
            .get("session-b")
            .and_then(|entry| entry.thought.as_deref()),
        Some("second thought")
    );
}

#[tokio::test]
async fn merge_write_thought_uses_fallback_when_disk_file_is_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("thoughts.json");
    let cached_changed_at = fixed_utc("2026-02-03T04:05:06Z");
    let mut fallback = HashMap::new();
    fallback.insert(
        "session-a".to_string(),
        test_thought_snapshot(Some("cached thought"), Some(cached_changed_at)),
    );

    let written = merge_write_thought_blocking(
        path.clone(),
        "session-b".to_string(),
        test_thought_snapshot(Some("fresh thought"), None),
        fallback,
    )
    .await
    .expect("merge write thoughts");

    assert_eq!(
        written
            .get("session-a")
            .and_then(|entry| entry.thought.as_deref()),
        Some("cached thought")
    );
    assert_eq!(
        written
            .get("session-a")
            .and_then(|entry| entry.objective_changed_at),
        Some(cached_changed_at)
    );
    assert_eq!(
        written
            .get("session-b")
            .and_then(|entry| entry.thought.as_deref()),
        Some("fresh thought")
    );

    let persisted_payload = read_file_blocking(path)
        .await
        .expect("read written thoughts")
        .expect("thoughts file exists");
    let persisted = serde_json::from_str::<HashMap<String, ThoughtSnapshot>>(&persisted_payload)
        .expect("decode persisted thoughts");
    assert_eq!(persisted.len(), 2);
    assert_eq!(
        persisted
            .get("session-a")
            .and_then(|entry| entry.thought.as_deref()),
        Some("cached thought")
    );
}

#[tokio::test]
async fn merge_write_thought_carries_forward_existing_objective_changed_at() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("thoughts.json");
    let original_changed_at = fixed_utc("2026-03-04T05:06:07Z");
    let explicit_changed_at = fixed_utc("2026-04-05T06:07:08Z");
    let mut existing = HashMap::new();
    existing.insert(
        "session-a".to_string(),
        test_thought_snapshot(Some("original thought"), Some(original_changed_at)),
    );
    let existing_payload =
        serde_json::to_string_pretty(&existing).expect("serialize existing thoughts");
    atomic_write_blocking(path.clone(), existing_payload)
        .await
        .expect("seed thoughts file");

    let carried = merge_write_thought_blocking(
        path.clone(),
        "session-a".to_string(),
        test_thought_snapshot(Some("carried thought"), None),
        HashMap::new(),
    )
    .await
    .expect("merge carried thought");
    assert_eq!(
        carried
            .get("session-a")
            .and_then(|entry| entry.objective_changed_at),
        Some(original_changed_at)
    );

    let explicit = merge_write_thought_blocking(
        path,
        "session-a".to_string(),
        test_thought_snapshot(Some("explicit thought"), Some(explicit_changed_at)),
        HashMap::new(),
    )
    .await
    .expect("merge explicit thought");
    assert_eq!(
        explicit
            .get("session-a")
            .and_then(|entry| entry.objective_changed_at),
        Some(explicit_changed_at)
    );
}

#[tokio::test]
async fn save_thought_config_failure_updates_health() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("store");
    std::fs::create_dir(dir.path().join("thought_config.json"))
        .expect("create directory at thought config path");

    let err = store
        .save_thought_config(&ThoughtConfig::default())
        .await
        .expect_err("directory at config path should fail write");

    let failed = store.health_snapshot();
    assert!(!failed.ok);
    assert_eq!(failed.consecutive_failures, 1);
    assert_eq!(
        failed.last_failed_operation.as_deref(),
        Some(OP_THOUGHT_CONFIG)
    );
    assert!(failed.last_failure_at.is_some());
    let err_text = err.to_string();
    assert_eq!(failed.last_error.as_deref(), Some(err_text.as_str()));
}

#[tokio::test]
async fn read_file_blocking_reports_checksum_mismatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session_registry.json");
    let corrupted = ChecksummedPayload {
        checksum_crc32: 1,
        payload: "{\"n\":1}".to_string(),
    };
    tokio::fs::write(
        &path,
        serde_json::to_vec_pretty(&corrupted).expect("serialize corrupted payload"),
    )
    .await
    .expect("write corrupted payload");

    let err = read_file_blocking(path.clone())
        .await
        .expect_err("checksum mismatch should fail");
    let typed = err
        .downcast_ref::<FileStoreIoError>()
        .expect("typed file store error");
    assert!(matches!(typed, FileStoreIoError::ChecksumMismatch { .. }));
}

#[tokio::test]
async fn atomic_write_blocking_fails_fast_when_lock_is_held() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session_registry.json");
    let lock_path = lock_path_for(&path).expect("lock path");
    std::fs::create_dir_all(dir.path()).expect("create parent");
    let lock_file = open_lock_file(&lock_path).expect("open lock file");
    lock_file.lock_exclusive().expect("hold lock");

    let err = atomic_write_blocking(path, "{\"n\":1}".to_string())
        .await
        .expect_err("writer should fail fast under lock contention");

    fs2::FileExt::unlock(&lock_file).expect("unlock lock file");

    let typed = err
        .downcast_ref::<FileStoreIoError>()
        .expect("typed file store error");
    assert!(matches!(typed, FileStoreIoError::LockBusy { .. }));
}

#[tokio::test]
async fn atomic_write_and_read_round_trip_with_checksum_envelope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("thoughts.json");

    atomic_write_blocking(path.clone(), "{\"n\":42}".to_string())
        .await
        .expect("write checksummed payload");
    let decoded = read_file_blocking(path.clone())
        .await
        .expect("read checksummed payload")
        .expect("payload present");
    assert_eq!(decoded, "{\"n\":42}");

    let raw = tokio::fs::read_to_string(&path)
        .await
        .expect("read raw file");
    assert!(raw.contains("checksum_crc32"));
}

#[tokio::test]
async fn read_file_blocking_accepts_legacy_raw_json_payloads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session_registry.json");
    tokio::fs::write(&path, "[{\"session_id\":\"sess-1\"}]")
        .await
        .expect("write legacy payload");

    let decoded = read_file_blocking(path)
        .await
        .expect("read legacy payload")
        .expect("payload present");

    assert_eq!(decoded, "[{\"session_id\":\"sess-1\"}]");
}

#[test]
fn sync_existing_file_ignores_missing_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("missing.json");

    sync_existing_file(&path).expect("missing persistence files are optional");
}
