//! File-based JSON persistence for session registry and thought snapshots.
//!
//! All disk I/O is performed via `tokio::task::spawn_blocking` to avoid
//! blocking the async runtime. Writes use atomic rename (write to temp file,
//! then rename) for crash safety.

use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::thought::protocol::ThoughtDeliveryState;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{
    ActionCue, DirGroupMemberships, RestState, SessionBatchMembership, SessionState, ThoughtSource,
    ThoughtState,
};

const OP_SESSION_REGISTRY: &str = "session_registry";
const OP_THOUGHTS: &str = "thoughts";
const OP_THOUGHT_CONFIG: &str = "thought_config";
const OP_DIR_GROUPS: &str = "dir_groups";

// ---------------------------------------------------------------------------
// Persisted data types
// ---------------------------------------------------------------------------

/// A persisted snapshot of a single session's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub session_id: String,
    pub tmux_name: String,
    pub state: SessionState,
    pub tool: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    pub thought: Option<String>,
    #[serde(default)]
    pub thought_state: ThoughtState,
    #[serde(default)]
    pub thought_source: ThoughtSource,
    #[serde(default)]
    pub thought_updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
    #[serde(default)]
    pub objective_changed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_skill: Option<String>,
    #[serde(default)]
    pub objective_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<SessionBatchMembership>,
    pub cwd: String,
    pub last_activity_at: DateTime<Utc>,
}

/// A persisted thought snapshot for a single session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThoughtSnapshot {
    pub thought: Option<String>,
    #[serde(default)]
    pub thought_state: ThoughtState,
    #[serde(default)]
    pub thought_source: ThoughtSource,
    #[serde(default)]
    pub rest_state: RestState,
    #[serde(default)]
    pub commit_candidate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_cues: Vec<ActionCue>,
    #[serde(default)]
    pub objective_changed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub objective_fingerprint: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    pub updated_at: DateTime<Utc>,
    #[serde(flatten)]
    pub delivery: ThoughtDeliveryState,
}

// ---------------------------------------------------------------------------
// FileStore
// ---------------------------------------------------------------------------

/// File-based persistence store. Thread-safe via internal RwLock on cached state.
pub struct FileStore {
    base_dir: PathBuf,
    /// Last observed write health for all flat-file persistence operations.
    health: StdMutex<PersistenceHealthState>,
    /// Serialize all writes in-process before taking the cross-process lock.
    write_lock: Mutex<()>,
    /// In-memory cache of persisted sessions, synced to disk on mutation.
    cache: RwLock<Vec<PersistedSession>>,
    /// Serialize registry writes to avoid temp-file rename races.
    session_write_lock: Mutex<()>,
    /// In-memory cache of thought snapshots, synced to disk on mutation.
    thought_cache: RwLock<HashMap<String, ThoughtSnapshot>>,
    /// In-memory cache of daemon runtime thought config.
    thought_config_cache: RwLock<ThoughtConfig>,
    /// In-memory cache of operator-managed directory group membership deltas.
    dir_group_memberships_cache: RwLock<DirGroupMemberships>,
    /// Serialize thought writes to avoid stale read-modify-write races.
    thought_write_lock: Mutex<()>,
    /// Serialize directory group writes to avoid stale read-modify-write races.
    dir_group_write_lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistenceHealthSnapshot {
    pub ok: bool,
    pub consecutive_failures: u64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_successful_operation: Option<String>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_failed_operation: Option<String>,
    pub last_error: Option<String>,
    pub load_outcomes: BTreeMap<String, PersistenceLoadSnapshot>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceLoadStatus {
    Loaded,
    Missing,
    DecodeFailed,
    Invalid,
    ReadFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistenceLoadSnapshot {
    pub status: PersistenceLoadStatus,
    pub checked_at: DateTime<Utc>,
    pub records: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct PersistenceHealthState {
    ok: bool,
    consecutive_failures: u64,
    last_success_at: Option<DateTime<Utc>>,
    last_successful_operation: Option<String>,
    last_failure_at: Option<DateTime<Utc>>,
    last_failed_operation: Option<String>,
    last_error: Option<String>,
    load_outcomes: BTreeMap<String, PersistenceLoadSnapshot>,
}

impl Default for PersistenceHealthState {
    fn default() -> Self {
        Self {
            ok: true,
            consecutive_failures: 0,
            last_success_at: None,
            last_successful_operation: None,
            last_failure_at: None,
            last_failed_operation: None,
            last_error: None,
            load_outcomes: BTreeMap::new(),
        }
    }
}

impl From<&PersistenceHealthState> for PersistenceHealthSnapshot {
    fn from(state: &PersistenceHealthState) -> Self {
        Self {
            ok: state.ok,
            consecutive_failures: state.consecutive_failures,
            last_success_at: state.last_success_at,
            last_successful_operation: state.last_successful_operation.clone(),
            last_failure_at: state.last_failure_at,
            last_failed_operation: state.last_failed_operation.clone(),
            last_error: state.last_error.clone(),
            load_outcomes: state.load_outcomes.clone(),
        }
    }
}

#[derive(Debug, Error)]
enum FileStoreIoError {
    #[error("persistence lock is held by another writer: {path}")]
    LockBusy { path: PathBuf },
    #[error("checksum mismatch for {path}: expected {expected:08x}, actual {actual:08x}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChecksummedPayload {
    checksum_crc32: u32,
    payload: String,
}

impl FileStore {
    /// Create a new FileStore with the given base directory.
    /// Creates the directory if it does not exist.
    pub async fn new(base_dir: impl Into<PathBuf>) -> anyhow::Result<Arc<Self>> {
        let base_dir = base_dir.into();

        // Create directory structure in a blocking task.
        let dir = base_dir.clone();
        tokio::task::spawn_blocking(move || std::fs::create_dir_all(&dir))
            .await
            .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
            .map_err(|e| anyhow::anyhow!("failed to create persistence directory: {e}"))?;

        let store = Arc::new(Self {
            base_dir,
            health: StdMutex::new(PersistenceHealthState::default()),
            write_lock: Mutex::new(()),
            cache: RwLock::new(Vec::new()),
            session_write_lock: Mutex::new(()),
            thought_cache: RwLock::new(HashMap::new()),
            thought_config_cache: RwLock::new(ThoughtConfig::default()),
            dir_group_memberships_cache: RwLock::new(DirGroupMemberships::default()),
            thought_write_lock: Mutex::new(()),
            dir_group_write_lock: Mutex::new(()),
        });

        // Load existing data into cache.
        let loaded = store.load_sessions_from_disk().await;
        let loaded_thoughts = store.load_thoughts_from_disk().await;
        let loaded_thought_config = store.load_thought_config_from_disk().await;
        let loaded_dir_groups = store.load_dir_group_memberships_from_disk().await;
        {
            let mut cache = store.cache.write().await;
            *cache = loaded;
        }
        {
            let mut thought_cache = store.thought_cache.write().await;
            *thought_cache = loaded_thoughts;
        }
        {
            let mut thought_config_cache = store.thought_config_cache.write().await;
            *thought_config_cache = loaded_thought_config;
        }
        {
            let mut dir_group_cache = store.dir_group_memberships_cache.write().await;
            *dir_group_cache = loaded_dir_groups;
        }

        info!(
            dir = %store.base_dir.display(),
            sessions = store.cache.read().await.len(),
            thoughts = store.thought_cache.read().await.len(),
            "persistence store initialized"
        );

        Ok(store)
    }

    /// Return the path to the session registry file.
    fn registry_path(&self) -> PathBuf {
        self.base_dir.join("session_registry.json")
    }

    /// Return the path to the thoughts file.
    fn thoughts_path(&self) -> PathBuf {
        self.base_dir.join("thoughts.json")
    }

    /// Return the path to the daemon runtime thought config file.
    fn thought_config_path(&self) -> PathBuf {
        self.base_dir.join("thought_config.json")
    }

    /// Return the path to operator-managed directory group membership deltas.
    fn dir_group_memberships_path(&self) -> PathBuf {
        self.base_dir.join("dir_groups.json")
    }

    pub fn health_snapshot(&self) -> PersistenceHealthSnapshot {
        let health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        PersistenceHealthSnapshot::from(&*health)
    }

    fn record_write_success(&self, operation: &'static str) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.ok = true;
        health.consecutive_failures = 0;
        health.last_success_at = Some(Utc::now());
        health.last_successful_operation = Some(operation.to_string());
        health.last_error = None;
    }

    fn record_write_failure(&self, operation: &'static str, error: impl std::fmt::Display) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.ok = false;
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        health.last_failure_at = Some(Utc::now());
        health.last_failed_operation = Some(operation.to_string());
        health.last_error = Some(error.to_string());
    }

    fn record_load_success(&self, operation: &'static str, records: u64) {
        let now = Utc::now();
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.load_outcomes.insert(
            operation.to_string(),
            PersistenceLoadSnapshot {
                status: PersistenceLoadStatus::Loaded,
                checked_at: now,
                records: Some(records),
                last_error: None,
            },
        );
        health.ok = !health
            .load_outcomes
            .values()
            .any(|outcome| load_status_is_failure(outcome.status));
        if health.ok {
            health.consecutive_failures = 0;
        }
        health.last_success_at = Some(now);
        health.last_successful_operation = Some(operation.to_string());
        if health.ok {
            health.last_error = None;
        }
    }

    fn record_load_missing(&self, operation: &'static str) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.load_outcomes.insert(
            operation.to_string(),
            PersistenceLoadSnapshot {
                status: PersistenceLoadStatus::Missing,
                checked_at: Utc::now(),
                records: Some(0),
                last_error: None,
            },
        );
        health.ok = !health
            .load_outcomes
            .values()
            .any(|outcome| load_status_is_failure(outcome.status));
        if health.ok {
            health.last_error = None;
        }
    }

    fn record_load_failure(
        &self,
        operation: &'static str,
        status: PersistenceLoadStatus,
        error: impl std::fmt::Display,
    ) {
        let now = Utc::now();
        let error = error.to_string();
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.load_outcomes.insert(
            operation.to_string(),
            PersistenceLoadSnapshot {
                status,
                checked_at: now,
                records: None,
                last_error: Some(error.clone()),
            },
        );
        health.ok = false;
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        health.last_failure_at = Some(now);
        health.last_failed_operation = Some(operation.to_string());
        health.last_error = Some(error);
    }

    // -----------------------------------------------------------------------
    // Session registry
    // -----------------------------------------------------------------------

    /// Save the full session registry to disk atomically.
    pub async fn save_sessions(&self, sessions: &[PersistedSession]) {
        let _global_write_guard = self.write_lock.lock().await;
        let _write_guard = self.session_write_lock.lock().await;

        // Update the in-memory cache.
        {
            let mut cache = self.cache.write().await;
            *cache = sessions.to_vec();
        }

        let path = self.registry_path();
        let data = match serde_json::to_string_pretty(sessions) {
            Ok(d) => d,
            Err(e) => {
                self.record_write_failure(OP_SESSION_REGISTRY, &e);
                error!("failed to serialize session registry: {e}");
                return;
            }
        };

        if let Err(e) = atomic_write_blocking(path, data).await {
            self.record_write_failure(OP_SESSION_REGISTRY, &e);
            error!("failed to write session registry: {e}");
        } else {
            self.record_write_success(OP_SESSION_REGISTRY);
            debug!(count = sessions.len(), "persisted session registry");
        }
    }

    /// Load sessions from disk. Returns empty vec if file is missing or corrupt.
    async fn load_sessions_from_disk(&self) -> Vec<PersistedSession> {
        let path = self.registry_path();
        match read_file_blocking(path).await {
            Ok(Some(data)) => match serde_json::from_str::<Vec<PersistedSession>>(&data) {
                Ok(sessions) => {
                    self.record_load_success(OP_SESSION_REGISTRY, sessions.len() as u64);
                    info!(count = sessions.len(), "loaded persisted session registry");
                    sessions
                }
                Err(e) => {
                    self.record_load_failure(
                        OP_SESSION_REGISTRY,
                        PersistenceLoadStatus::DecodeFailed,
                        &e,
                    );
                    warn!("corrupt session registry, starting fresh: {e}");
                    Vec::new()
                }
            },
            Ok(None) => {
                self.record_load_missing(OP_SESSION_REGISTRY);
                debug!("no persisted session registry found");
                Vec::new()
            }
            Err(e) => {
                self.record_load_failure(
                    OP_SESSION_REGISTRY,
                    load_failure_status(&e),
                    format!("{e:#}"),
                );
                warn!("failed to read session registry: {e}");
                Vec::new()
            }
        }
    }

    /// Load sessions from the in-memory cache (populated at startup).
    pub async fn load_sessions(&self) -> Vec<PersistedSession> {
        self.cache.read().await.clone()
    }

    // -----------------------------------------------------------------------
    // Thought snapshots
    // -----------------------------------------------------------------------

    /// Save a single session's thought data. Merges with existing thought data
    /// on disk.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_thought(
        &self,
        session_id: &str,
        thought: Option<&str>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        rest_state: RestState,
        commit_candidate: bool,
        action_cues: Vec<ActionCue>,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_changed_at: Option<DateTime<Utc>>,
        objective_fingerprint: Option<String>,
    ) {
        let _global_write_guard = self.write_lock.lock().await;
        let _write_guard = self.thought_write_lock.lock().await;
        let path = self.thoughts_path();
        let fallback = self.thought_cache.read().await.clone();
        let snapshot = ThoughtSnapshot {
            thought: thought.map(|value| value.to_string()),
            thought_state,
            thought_source,
            rest_state,
            commit_candidate,
            action_cues,
            objective_changed_at,
            objective_fingerprint,
            token_count,
            context_limit,
            updated_at,
            delivery,
        };

        match merge_write_thought_blocking(path, session_id.to_string(), snapshot, fallback).await {
            Ok(thoughts) => {
                let mut cache = self.thought_cache.write().await;
                *cache = thoughts;
                self.record_write_success(OP_THOUGHTS);
                debug!(session_id, "persisted thought snapshot");
            }
            Err(e) => {
                self.record_write_failure(OP_THOUGHTS, &e);
                error!("failed to write thoughts: {e}");
            }
        }
    }

    /// Load all persisted thought snapshots.
    pub async fn load_thoughts(&self) -> HashMap<String, ThoughtSnapshot> {
        self.thought_cache.read().await.clone()
    }

    /// Load all persisted thought snapshots from disk.
    async fn load_thoughts_from_disk(&self) -> HashMap<String, ThoughtSnapshot> {
        let path = self.thoughts_path();
        match read_file_blocking(path).await {
            Ok(Some(data)) => {
                match serde_json::from_str::<HashMap<String, ThoughtSnapshot>>(&data) {
                    Ok(thoughts) => {
                        self.record_load_success(OP_THOUGHTS, thoughts.len() as u64);
                        thoughts
                    }
                    Err(e) => {
                        self.record_load_failure(
                            OP_THOUGHTS,
                            PersistenceLoadStatus::DecodeFailed,
                            &e,
                        );
                        warn!("corrupt thoughts file, starting fresh: {e}");
                        HashMap::new()
                    }
                }
            }
            Ok(None) => {
                self.record_load_missing(OP_THOUGHTS);
                HashMap::new()
            }
            Err(e) => {
                self.record_load_failure(OP_THOUGHTS, load_failure_status(&e), format!("{e:#}"));
                warn!("failed to read thoughts: {e}");
                HashMap::new()
            }
        }
    }

    // -----------------------------------------------------------------------
    // Thought runtime config
    // -----------------------------------------------------------------------

    /// Save daemon runtime thought config to disk atomically.
    pub async fn save_thought_config(&self, config: &ThoughtConfig) -> anyhow::Result<()> {
        let _global_write_guard = self.write_lock.lock().await;
        let normalized = config
            .clone()
            .normalize_and_validate()
            .map_err(|e| anyhow::anyhow!("invalid thought config: {e}"))?;

        let path = self.thought_config_path();
        let data = match serde_json::to_string_pretty(&normalized) {
            Ok(data) => data,
            Err(e) => {
                let err = anyhow::anyhow!("failed to serialize thought config: {e}");
                self.record_write_failure(OP_THOUGHT_CONFIG, &err);
                return Err(err);
            }
        };
        if let Err(err) = atomic_write_blocking(path, data).await {
            self.record_write_failure(OP_THOUGHT_CONFIG, &err);
            return Err(err);
        }

        {
            let mut thought_config_cache = self.thought_config_cache.write().await;
            *thought_config_cache = normalized;
        }

        self.record_write_success(OP_THOUGHT_CONFIG);
        debug!("persisted thought runtime config");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Directory group memberships
    // -----------------------------------------------------------------------

    /// Save operator-managed directory group membership deltas atomically.
    pub async fn save_dir_group_memberships(
        &self,
        memberships: DirGroupMemberships,
    ) -> anyhow::Result<()> {
        let _global_write_guard = self.write_lock.lock().await;
        let _write_guard = self.dir_group_write_lock.lock().await;
        let data = match serde_json::to_string_pretty(&memberships) {
            Ok(data) => data,
            Err(e) => {
                let err = anyhow::anyhow!("failed to serialize directory groups: {e}");
                self.record_write_failure(OP_DIR_GROUPS, &err);
                return Err(err);
            }
        };
        let path = self.dir_group_memberships_path();
        if let Err(err) = atomic_write_blocking(path, data).await {
            self.record_write_failure(OP_DIR_GROUPS, &err);
            return Err(err);
        }

        {
            let mut cache = self.dir_group_memberships_cache.write().await;
            *cache = memberships;
        }

        self.record_write_success(OP_DIR_GROUPS);
        debug!("persisted directory group memberships");
        Ok(())
    }

    /// Apply an in-process read-modify-write to directory group membership
    /// deltas and persist the resulting snapshot atomically.
    pub async fn update_dir_group_memberships<F>(
        &self,
        update: F,
    ) -> anyhow::Result<DirGroupMemberships>
    where
        F: FnOnce(&mut DirGroupMemberships),
    {
        let _global_write_guard = self.write_lock.lock().await;
        let _write_guard = self.dir_group_write_lock.lock().await;

        let mut memberships = self.dir_group_memberships_cache.read().await.clone();
        update(&mut memberships);

        let data = match serde_json::to_string_pretty(&memberships) {
            Ok(data) => data,
            Err(e) => {
                let err = anyhow::anyhow!("failed to serialize directory groups: {e}");
                self.record_write_failure(OP_DIR_GROUPS, &err);
                return Err(err);
            }
        };
        let path = self.dir_group_memberships_path();
        if let Err(err) = atomic_write_blocking(path, data).await {
            self.record_write_failure(OP_DIR_GROUPS, &err);
            return Err(err);
        }

        {
            let mut cache = self.dir_group_memberships_cache.write().await;
            *cache = memberships.clone();
        }

        self.record_write_success(OP_DIR_GROUPS);
        debug!("persisted directory group memberships");
        Ok(memberships)
    }

    /// Load directory group membership deltas from the in-memory cache.
    pub async fn load_dir_group_memberships(&self) -> DirGroupMemberships {
        self.dir_group_memberships_cache.read().await.clone()
    }

    async fn load_dir_group_memberships_from_disk(&self) -> DirGroupMemberships {
        let path = self.dir_group_memberships_path();
        match read_file_blocking(path).await {
            Ok(Some(data)) => match serde_json::from_str::<DirGroupMemberships>(&data) {
                Ok(groups) => {
                    self.record_load_success(OP_DIR_GROUPS, groups.groups.len() as u64);
                    groups
                }
                Err(e) => {
                    self.record_load_failure(
                        OP_DIR_GROUPS,
                        PersistenceLoadStatus::DecodeFailed,
                        &e,
                    );
                    warn!("corrupt directory group memberships file, starting fresh: {e}");
                    DirGroupMemberships::default()
                }
            },
            Ok(None) => {
                self.record_load_missing(OP_DIR_GROUPS);
                DirGroupMemberships::default()
            }
            Err(e) => {
                self.record_load_failure(OP_DIR_GROUPS, load_failure_status(&e), format!("{e:#}"));
                warn!("failed to read directory group memberships: {e}");
                DirGroupMemberships::default()
            }
        }
    }

    /// Sync persistence files and directory entries as a shutdown durability barrier.
    pub async fn flush_barrier(&self) -> anyhow::Result<()> {
        let _global_write_guard = self.write_lock.lock().await;
        let base_dir = self.base_dir.clone();
        let files = [
            self.registry_path(),
            self.thoughts_path(),
            self.thought_config_path(),
            self.dir_group_memberships_path(),
        ];
        tokio::task::spawn_blocking(move || flush_barrier_blocking(&base_dir, &files))
            .await
            .map_err(|err| anyhow::anyhow!("spawn_blocking panicked: {err}"))?
    }

    /// Load daemon runtime thought config from in-memory cache.
    pub async fn load_thought_config(&self) -> ThoughtConfig {
        self.thought_config_cache.read().await.clone()
    }

    /// Load daemon runtime thought config from disk (default on missing/corrupt).
    async fn load_thought_config_from_disk(&self) -> ThoughtConfig {
        let path = self.thought_config_path();
        match read_file_blocking(path).await {
            Ok(Some(data)) => match serde_json::from_str::<ThoughtConfig>(&data) {
                Ok(config) => match config.normalize_and_validate() {
                    Ok(config) => {
                        self.record_load_success(OP_THOUGHT_CONFIG, 1);
                        config
                    }
                    Err(e) => {
                        self.record_load_failure(
                            OP_THOUGHT_CONFIG,
                            PersistenceLoadStatus::Invalid,
                            &e,
                        );
                        warn!("invalid thought config file, using defaults: {e}");
                        ThoughtConfig::default()
                    }
                },
                Err(e) => {
                    self.record_load_failure(
                        OP_THOUGHT_CONFIG,
                        PersistenceLoadStatus::DecodeFailed,
                        &e,
                    );
                    warn!("corrupt thought config file, using defaults: {e}");
                    ThoughtConfig::default()
                }
            },
            Ok(None) => {
                self.record_load_missing(OP_THOUGHT_CONFIG);
                ThoughtConfig::default()
            }
            Err(e) => {
                self.record_load_failure(
                    OP_THOUGHT_CONFIG,
                    load_failure_status(&e),
                    format!("{e:#}"),
                );
                warn!("failed to read thought config file, using defaults: {e}");
                ThoughtConfig::default()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Blocking I/O helpers (run inside spawn_blocking)
// ---------------------------------------------------------------------------

fn load_failure_status(error: &anyhow::Error) -> PersistenceLoadStatus {
    if error.downcast_ref::<FileStoreIoError>().is_some()
        || error
            .to_string()
            .contains("decode checksummed payload failed")
    {
        PersistenceLoadStatus::DecodeFailed
    } else {
        PersistenceLoadStatus::ReadFailed
    }
}

fn load_status_is_failure(status: PersistenceLoadStatus) -> bool {
    matches!(
        status,
        PersistenceLoadStatus::DecodeFailed
            | PersistenceLoadStatus::Invalid
            | PersistenceLoadStatus::ReadFailed
    )
}

fn flush_barrier_blocking(base_dir: &Path, files: &[PathBuf]) -> anyhow::Result<()> {
    for path in files {
        sync_existing_file(path)?;
    }
    sync_directory(base_dir, "persistence")
}

fn sync_existing_file(path: &Path) -> anyhow::Result<()> {
    match std::fs::File::open(path) {
        Ok(file) => file
            .sync_all()
            .map_err(|err| anyhow::anyhow!("sync file {} failed: {err}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(anyhow::anyhow!(
            "open file {} failed: {err}",
            path.display()
        )),
    }
}

fn sync_directory(path: &Path, label: &str) -> anyhow::Result<()> {
    std::fs::File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|err| anyhow::anyhow!("sync {label} directory {} failed: {err}", path.display()))
}

/// Atomically write data to a file: write to `.tmp`, then rename.
async fn atomic_write_blocking(path: PathBuf, data: String) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        ensure_parent(&path).map_err(|e| anyhow::anyhow!("ensure parent failed: {e}"))?;
        let lock_path = lock_path_for(&path)?;
        let lock_file = open_lock_file(&lock_path)?;
        match lock_file.try_lock_exclusive() {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(anyhow::Error::new(FileStoreIoError::LockBusy {
                    path: lock_path,
                }));
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "acquire lock {} failed: {err}",
                    lock_path.display()
                ));
            }
        }

        write_checksummed_payload_locked(&path, data)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

async fn merge_write_thought_blocking(
    path: PathBuf,
    session_id: String,
    mut snapshot: ThoughtSnapshot,
    fallback: HashMap<String, ThoughtSnapshot>,
) -> anyhow::Result<HashMap<String, ThoughtSnapshot>> {
    tokio::task::spawn_blocking(move || {
        ensure_parent(&path).map_err(|e| anyhow::anyhow!("ensure parent failed: {e}"))?;
        let lock_path = lock_path_for(&path)?;
        let lock_file = open_lock_file(&lock_path)?;
        match lock_file.try_lock_exclusive() {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(anyhow::Error::new(FileStoreIoError::LockBusy {
                    path: lock_path,
                }));
            }
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "acquire lock {} failed: {err}",
                    lock_path.display()
                ));
            }
        }

        let mut thoughts = match std::fs::read_to_string(&path) {
            Ok(data) => {
                let payload = decode_file_payload(path.clone(), data)?;
                serde_json::from_str::<HashMap<String, ThoughtSnapshot>>(&payload)
                    .map_err(|e| anyhow::anyhow!("decode thoughts failed: {e}"))?
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => fallback,
            Err(err) => return Err(anyhow::anyhow!("read thoughts failed: {err}")),
        };

        if snapshot.objective_changed_at.is_none() {
            snapshot.objective_changed_at = thoughts
                .get(&session_id)
                .and_then(|existing| existing.objective_changed_at);
        }
        thoughts.insert(session_id, snapshot);

        let data = serde_json::to_string_pretty(&thoughts)
            .map_err(|e| anyhow::anyhow!("serialize thoughts failed: {e}"))?;
        write_checksummed_payload_locked(&path, data)?;
        Ok(thoughts)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

fn write_checksummed_payload_locked(path: &Path, data: String) -> anyhow::Result<()> {
    let tmp_path = path.with_extension(format!("json.tmp.{}", Uuid::new_v4()));
    let envelope = ChecksummedPayload {
        checksum_crc32: crc32fast::hash(data.as_bytes()),
        payload: data,
    };
    let encoded = serde_json::to_vec_pretty(&envelope)
        .map_err(|e| anyhow::anyhow!("serialize checksummed payload failed: {e}"))?;
    std::fs::write(&tmp_path, &encoded).map_err(|e| anyhow::anyhow!("write to tmp failed: {e}"))?;
    std::fs::File::open(&tmp_path)
        .and_then(|f| f.sync_all())
        .map_err(|e| anyhow::anyhow!("sync tmp file failed: {e}"))?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(anyhow::anyhow!("rename failed: {e}"));
    }
    sync_parent_dir(path)?;
    Ok(())
}

/// Read a file's contents, returning None if the file does not exist.
async fn read_file_blocking(path: PathBuf) -> anyhow::Result<Option<String>> {
    tokio::task::spawn_blocking(move || match std::fs::read_to_string(&path) {
        Ok(data) => decode_file_payload(path, data).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("read failed: {e}")),
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

fn decode_file_payload(path: PathBuf, data: String) -> anyhow::Result<String> {
    match serde_json::from_str::<ChecksummedPayload>(&data) {
        Ok(decoded) => {
            let actual = crc32fast::hash(decoded.payload.as_bytes());
            if actual != decoded.checksum_crc32 {
                return Err(anyhow::Error::new(FileStoreIoError::ChecksumMismatch {
                    path,
                    expected: decoded.checksum_crc32,
                    actual,
                }));
            }
            Ok(decoded.payload)
        }
        Err(envelope_error) => {
            let value: serde_json::Value = serde_json::from_str(&data).map_err(|_| {
                anyhow::anyhow!("decode checksummed payload failed: {envelope_error}")
            })?;
            if value.get("checksum_crc32").is_some() || value.get("payload").is_some() {
                return Err(anyhow::anyhow!(
                    "decode checksummed payload failed: {envelope_error}"
                ));
            }
            Ok(data)
        }
    }
}

/// Convenience: convert a `Path` to an owned `PathBuf`.
#[allow(dead_code)]
fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn lock_path_for(path: &Path) -> anyhow::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent for {}", path.display()))?;
    Ok(parent.join(".lock"))
}

fn open_lock_file(path: &Path) -> anyhow::Result<std::fs::File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| anyhow::anyhow!("open lock file {} failed: {e}", path.display()))
}

fn sync_parent_dir(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent for {}", path.display()))?;
    std::fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| anyhow::anyhow!("sync parent directory {} failed: {e}", parent.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;

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

        lock_file.unlock().expect("unlock lock file");

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
}
