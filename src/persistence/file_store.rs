//! File-based JSON persistence for session registry and thought snapshots.
//!
//! All disk I/O is performed via `tokio::task::spawn_blocking` to avoid
//! blocking the async runtime. Writes use atomic rename (write to temp file,
//! then rename) for crash safety.

use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
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
use crate::thought::runtime_config::{ThoughtConfig, ThoughtConfigValidationError};
use crate::tmux_target::TmuxTarget;
use crate::types::{
    ActionCue, DirGroupMemberships, RestState, SessionBatchMembership, SessionState, ThoughtSource,
    ThoughtState,
};

const OP_SESSION_REGISTRY: &str = "session_registry";
const OP_THOUGHTS: &str = "thoughts";
const OP_THOUGHT_CONFIG: &str = "thought_config";
const OP_DIR_GROUPS: &str = "dir_groups";
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

// ---------------------------------------------------------------------------
// Persisted data types
// ---------------------------------------------------------------------------

/// A persisted snapshot of a single session's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub session_id: String,
    pub tmux_name: String,
    #[serde(default, skip_serializing_if = "TmuxTarget::is_default")]
    pub tmux_target: TmuxTarget,
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

#[derive(Debug)]
enum ThoughtConfigLoadOutcome {
    Loaded(ThoughtConfig),
    Missing,
    DecodeFailed(serde_json::Error),
    Invalid(ThoughtConfigValidationError),
    ReadFailed(anyhow::Error),
}

impl FileStore {
    /// Create a new FileStore with the given base directory.
    /// Creates the directory if it does not exist.
    pub async fn new(base_dir: impl Into<PathBuf>) -> anyhow::Result<Arc<Self>> {
        let base_dir = base_dir.into();

        // Create and harden directory structure in a blocking task.
        let dir = base_dir.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            prepare_persistence_dir(&dir)?;
            harden_existing_store_files(&dir)?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!("failed to prepare persistence directory: {e:#}"))?;

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

    /// Recompute `ok` by folding the latest write outcome with all recorded
    /// load outcomes. A successful write must not clear a known corrupt-load
    /// condition (e.g. a startup decode failure), so the health state stays
    /// degraded until the failing load is re-resolved.
    fn recompute_ok(health: &mut PersistenceHealthState) {
        health.ok = !health
            .load_outcomes
            .values()
            .any(|outcome| load_status_is_failure(outcome.status));
    }

    fn record_write_success(&self, operation: &'static str) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::recompute_ok(&mut health);
        if health.ok {
            health.consecutive_failures = 0;
        }
        health.last_success_at = Some(Utc::now());
        health.last_successful_operation = Some(operation.to_string());
        if health.ok {
            health.last_error = None;
        }
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
        Self::recompute_ok(&mut health);
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
        Self::recompute_ok(&mut health);
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

        let path = self.registry_path();
        let data = match serde_json::to_string_pretty(sessions) {
            Ok(d) => d,
            Err(e) => {
                self.record_write_failure(OP_SESSION_REGISTRY, &e);
                error!("failed to serialize session registry: {e}");
                return;
            }
        };

        // Only commit the in-memory cache after the durable write succeeds, so
        // `current_sessions()` never reports state that did not persist.
        if let Err(e) = atomic_write_blocking(path, data).await {
            self.record_write_failure(OP_SESSION_REGISTRY, &e);
            error!("failed to write session registry: {e}");
        } else {
            {
                let mut cache = self.cache.write().await;
                *cache = sessions.to_vec();
            }
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

        // Apply the update to the current on-disk state (falling back to the
        // in-memory cache on a missing/corrupt file), so a concurrent write from
        // another process sharing the data dir is merged rather than silently
        // lost — mirroring save_thought's disk-based read-modify-write.
        let path = self.dir_group_memberships_path();
        let cache_snapshot = self.dir_group_memberships_cache.read().await.clone();
        let mut memberships = match read_file_blocking(path.clone()).await {
            Ok(Some(data)) => {
                serde_json::from_str::<DirGroupMemberships>(&data).unwrap_or_else(|err| {
                    warn!("corrupt directory group memberships file, merging from cache: {err}");
                    cache_snapshot
                })
            }
            _ => cache_snapshot,
        };
        update(&mut memberships);

        let data = match serde_json::to_string_pretty(&memberships) {
            Ok(data) => data,
            Err(e) => {
                let err = anyhow::anyhow!("failed to serialize directory groups: {e}");
                self.record_write_failure(OP_DIR_GROUPS, &err);
                return Err(err);
            }
        };
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
        self.finish_thought_config_load(read_thought_config_file(self.thought_config_path()).await)
    }

    fn finish_thought_config_load(&self, outcome: ThoughtConfigLoadOutcome) -> ThoughtConfig {
        match outcome {
            ThoughtConfigLoadOutcome::Loaded(config) => self.loaded_thought_config(config),
            ThoughtConfigLoadOutcome::Missing => self.missing_thought_config(),
            ThoughtConfigLoadOutcome::DecodeFailed(err) => self.decode_failed_thought_config(err),
            ThoughtConfigLoadOutcome::Invalid(err) => self.invalid_thought_config(err),
            ThoughtConfigLoadOutcome::ReadFailed(err) => self.read_failed_thought_config(err),
        }
    }

    fn loaded_thought_config(&self, config: ThoughtConfig) -> ThoughtConfig {
        self.record_load_success(OP_THOUGHT_CONFIG, 1);
        config
    }

    fn missing_thought_config(&self) -> ThoughtConfig {
        self.record_load_missing(OP_THOUGHT_CONFIG);
        ThoughtConfig::default()
    }

    fn decode_failed_thought_config(&self, err: serde_json::Error) -> ThoughtConfig {
        self.record_load_failure(OP_THOUGHT_CONFIG, PersistenceLoadStatus::DecodeFailed, &err);
        warn!("corrupt thought config file, using defaults: {err}");
        ThoughtConfig::default()
    }

    fn invalid_thought_config(&self, err: ThoughtConfigValidationError) -> ThoughtConfig {
        self.record_load_failure(OP_THOUGHT_CONFIG, PersistenceLoadStatus::Invalid, &err);
        warn!("invalid thought config file, using defaults: {err}");
        ThoughtConfig::default()
    }

    fn read_failed_thought_config(&self, err: anyhow::Error) -> ThoughtConfig {
        self.record_load_failure(
            OP_THOUGHT_CONFIG,
            load_failure_status(&err),
            format!("{err:#}"),
        );
        warn!("failed to read thought config file, using defaults: {err}");
        ThoughtConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Blocking I/O helpers (run inside spawn_blocking)
// ---------------------------------------------------------------------------

async fn read_thought_config_file(path: PathBuf) -> ThoughtConfigLoadOutcome {
    match read_file_blocking(path).await {
        Ok(Some(data)) => decode_thought_config_payload(&data),
        Ok(None) => ThoughtConfigLoadOutcome::Missing,
        Err(err) => ThoughtConfigLoadOutcome::ReadFailed(err),
    }
}

fn decode_thought_config_payload(data: &str) -> ThoughtConfigLoadOutcome {
    match serde_json::from_str::<ThoughtConfig>(data) {
        Ok(config) => normalize_thought_config_load(config),
        Err(err) => ThoughtConfigLoadOutcome::DecodeFailed(err),
    }
}

fn normalize_thought_config_load(config: ThoughtConfig) -> ThoughtConfigLoadOutcome {
    match config.normalize_and_validate() {
        Ok(config) => ThoughtConfigLoadOutcome::Loaded(config),
        Err(err) => ThoughtConfigLoadOutcome::Invalid(err),
    }
}

fn load_failure_status(error: &anyhow::Error) -> PersistenceLoadStatus {
    match error.downcast_ref::<FileStoreIoError>() {
        // A genuine on-disk corruption: the checksum envelope did not verify.
        Some(FileStoreIoError::ChecksumMismatch { .. }) => PersistenceLoadStatus::DecodeFailed,
        // A transient cross-process lock contention is not permanent corruption.
        Some(FileStoreIoError::LockBusy { .. }) => PersistenceLoadStatus::ReadFailed,
        None => {
            if error
                .to_string()
                .contains("decode checksummed payload failed")
            {
                PersistenceLoadStatus::DecodeFailed
            } else {
                PersistenceLoadStatus::ReadFailed
            }
        }
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
        let _lock_file = acquire_persistence_lock(&path)?;
        write_checksummed_payload_locked(&path, data)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

async fn merge_write_thought_blocking(
    path: PathBuf,
    session_id: String,
    snapshot: ThoughtSnapshot,
    fallback: HashMap<String, ThoughtSnapshot>,
) -> anyhow::Result<HashMap<String, ThoughtSnapshot>> {
    tokio::task::spawn_blocking(move || {
        merge_write_thought_locked(path, session_id, snapshot, fallback)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

fn merge_write_thought_locked(
    path: PathBuf,
    session_id: String,
    snapshot: ThoughtSnapshot,
    fallback: HashMap<String, ThoughtSnapshot>,
) -> anyhow::Result<HashMap<String, ThoughtSnapshot>> {
    ensure_parent(&path).map_err(|e| anyhow::anyhow!("ensure parent failed: {e}"))?;
    let _lock_file = acquire_persistence_lock(&path)?;
    let mut thoughts = read_thoughts_or_fallback(&path, fallback)?;
    merge_thought_snapshot(&mut thoughts, session_id, snapshot);
    write_thoughts_locked(&path, &thoughts)?;
    Ok(thoughts)
}

fn read_thoughts_or_fallback(
    path: &Path,
    fallback: HashMap<String, ThoughtSnapshot>,
) -> anyhow::Result<HashMap<String, ThoughtSnapshot>> {
    match std::fs::read_to_string(path) {
        Ok(data) => match decode_thoughts_payload(path, data) {
            Ok(thoughts) => Ok(thoughts),
            Err(err) => {
                // A corrupt or checksum-mismatched file is survivable at startup
                // (the loader starts fresh); make it survivable at runtime too by
                // falling back to the in-memory snapshot so the next write
                // overwrites the bad payload instead of permanently wedging
                // thought persistence on a single byte flip.
                warn!("corrupt thoughts file, rewriting from cache: {err}");
                Ok(fallback)
            }
        },
        Err(err) => read_thoughts_error_or_fallback(err, fallback),
    }
}

fn read_thoughts_error_or_fallback(
    err: std::io::Error,
    fallback: HashMap<String, ThoughtSnapshot>,
) -> anyhow::Result<HashMap<String, ThoughtSnapshot>> {
    if err.kind() == std::io::ErrorKind::NotFound {
        Ok(fallback)
    } else {
        Err(anyhow::anyhow!("read thoughts failed: {err}"))
    }
}

fn decode_thoughts_payload(
    path: &Path,
    data: String,
) -> anyhow::Result<HashMap<String, ThoughtSnapshot>> {
    let payload = decode_file_payload(path.to_path_buf(), data)?;
    serde_json::from_str::<HashMap<String, ThoughtSnapshot>>(&payload)
        .map_err(|e| anyhow::anyhow!("decode thoughts failed: {e}"))
}

fn merge_thought_snapshot(
    thoughts: &mut HashMap<String, ThoughtSnapshot>,
    session_id: String,
    mut snapshot: ThoughtSnapshot,
) {
    carry_forward_objective_changed_at(thoughts, &session_id, &mut snapshot);
    thoughts.insert(session_id, snapshot);
}

fn carry_forward_objective_changed_at(
    thoughts: &HashMap<String, ThoughtSnapshot>,
    session_id: &str,
    snapshot: &mut ThoughtSnapshot,
) {
    if snapshot.objective_changed_at.is_none() {
        snapshot.objective_changed_at = thoughts
            .get(session_id)
            .and_then(|existing| existing.objective_changed_at);
    }
}

fn write_thoughts_locked(
    path: &Path,
    thoughts: &HashMap<String, ThoughtSnapshot>,
) -> anyhow::Result<()> {
    let data = serde_json::to_string_pretty(thoughts)
        .map_err(|e| anyhow::anyhow!("serialize thoughts failed: {e}"))?;
    write_checksummed_payload_locked(path, data)
}

fn acquire_persistence_lock(path: &Path) -> anyhow::Result<std::fs::File> {
    let lock_path = lock_path_for(path)?;
    let lock_file = open_lock_file(&lock_path)?;
    lock_exclusive_with_retry(lock_file, lock_path)
}

fn lock_exclusive_with_retry(
    lock_file: std::fs::File,
    lock_path: PathBuf,
) -> anyhow::Result<std::fs::File> {
    for _ in 0..3 {
        match lock_file.try_lock_exclusive() {
            Ok(()) => return Ok(lock_file),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(err) => return lock_error(lock_path, err),
        }
    }
    match lock_file.try_lock_exclusive() {
        Ok(()) => Ok(lock_file),
        Err(err) => lock_error(lock_path, err),
    }
}

fn lock_error(lock_path: PathBuf, err: std::io::Error) -> anyhow::Result<std::fs::File> {
    if err.kind() == std::io::ErrorKind::WouldBlock {
        Err(anyhow::Error::new(FileStoreIoError::LockBusy {
            path: lock_path,
        }))
    } else {
        Err(anyhow::anyhow!(
            "acquire lock {} failed: {err}",
            lock_path.display()
        ))
    }
}

fn write_checksummed_payload_locked(path: &Path, data: String) -> anyhow::Result<()> {
    let tmp_path = path.with_extension(format!("json.tmp.{}", Uuid::new_v4()));
    let envelope = ChecksummedPayload {
        checksum_crc32: crc32fast::hash(data.as_bytes()),
        payload: data,
    };
    let encoded = serde_json::to_vec_pretty(&envelope)
        .map_err(|e| anyhow::anyhow!("serialize checksummed payload failed: {e}"))?;
    harden_existing_private_file(path)?;
    write_private_file_synced(&tmp_path, &encoded)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(anyhow::anyhow!("rename failed: {e}"));
    }
    harden_existing_private_file(path)?;
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
fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        prepare_persistence_dir(parent)?;
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
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        options.mode(PRIVATE_FILE_MODE);
    }
    let file = options
        .open(path)
        .map_err(|e| anyhow::anyhow!("open lock file {} failed: {e}", path.display()))?;
    harden_open_private_file(&file, path)?;
    Ok(file)
}

fn sync_parent_dir(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent for {}", path.display()))?;
    std::fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| anyhow::anyhow!("sync parent directory {} failed: {e}", parent.display()))
}

fn prepare_persistence_dir(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)
        .map_err(|e| anyhow::anyhow!("create directory {} failed: {e}", path.display()))?;
    harden_private_dir(path)
}

fn harden_existing_store_files(base_dir: &Path) -> anyhow::Result<()> {
    for file_name in [
        "session_registry.json",
        "thoughts.json",
        "thought_config.json",
        "dir_groups.json",
        ".lock",
    ] {
        harden_existing_private_file(&base_dir.join(file_name))?;
    }
    Ok(())
}

#[cfg(unix)]
fn harden_private_dir(path: &Path) -> anyhow::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        .map_err(|e| anyhow::anyhow!("chmod directory {} failed: {e}", path.display()))
}

#[cfg(not(unix))]
fn harden_private_dir(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_existing_private_file(path: &Path) -> anyhow::Result<()> {
    let before = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(anyhow::anyhow!("metadata {} failed: {err}", path.display())),
    };
    if !before.file_type().is_file() {
        return Ok(());
    }

    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|e| anyhow::anyhow!("open existing file {} failed: {e}", path.display()))?;
    let after = file
        .metadata()
        .map_err(|e| anyhow::anyhow!("metadata opened file {} failed: {e}", path.display()))?;
    if before.dev() != after.dev() || before.ino() != after.ino() {
        return Err(anyhow::anyhow!(
            "refusing to chmod {} because it changed while hardening",
            path.display()
        ));
    }
    harden_open_private_file(&file, path)
}

#[cfg(not(unix))]
fn harden_existing_private_file(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn harden_open_private_file(file: &std::fs::File, path: &Path) -> anyhow::Result<()> {
    file.set_permissions(std::fs::Permissions::from_mode(PRIVATE_FILE_MODE))
        .map_err(|e| anyhow::anyhow!("chmod file {} failed: {e}", path.display()))
}

#[cfg(not(unix))]
fn harden_open_private_file(_file: &std::fs::File, _path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn write_private_file_synced(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(PRIVATE_FILE_MODE);
    }
    let mut file = options
        .open(path)
        .map_err(|e| anyhow::anyhow!("open tmp file {} failed: {e}", path.display()))?;
    harden_open_private_file(&file, path)?;
    file.write_all(contents)
        .map_err(|e| anyhow::anyhow!("write tmp file {} failed: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| anyhow::anyhow!("sync tmp file {} failed: {e}", path.display()))
}

#[cfg(test)]
mod tests;
