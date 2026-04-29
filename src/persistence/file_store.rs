//! File-based JSON persistence for session registry and thought snapshots.
//!
//! All disk I/O is performed via `tokio::task::spawn_blocking` to avoid
//! blocking the async runtime. Writes use atomic rename (write to temp file,
//! then rename) for crash safety.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::thought::protocol::ThoughtDeliveryState;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{RestState, SessionBatchMembership, SessionState, ThoughtSource, ThoughtState};

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
    /// Serialize thought writes to avoid stale read-modify-write races.
    thought_write_lock: Mutex<()>,
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
            write_lock: Mutex::new(()),
            cache: RwLock::new(Vec::new()),
            session_write_lock: Mutex::new(()),
            thought_cache: RwLock::new(HashMap::new()),
            thought_config_cache: RwLock::new(ThoughtConfig::default()),
            thought_write_lock: Mutex::new(()),
        });

        // Load existing data into cache.
        let loaded = store.load_sessions_from_disk().await;
        let loaded_thoughts = store.load_thoughts_from_disk().await;
        let loaded_thought_config = store.load_thought_config_from_disk().await;
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
                error!("failed to serialize session registry: {e}");
                return;
            }
        };

        if let Err(e) = atomic_write_blocking(path, data).await {
            error!("failed to write session registry: {e}");
        } else {
            debug!(count = sessions.len(), "persisted session registry");
        }
    }

    /// Load sessions from disk. Returns empty vec if file is missing or corrupt.
    async fn load_sessions_from_disk(&self) -> Vec<PersistedSession> {
        let path = self.registry_path();
        match read_file_blocking(path).await {
            Ok(Some(data)) => match serde_json::from_str::<Vec<PersistedSession>>(&data) {
                Ok(sessions) => {
                    info!(count = sessions.len(), "loaded persisted session registry");
                    sessions
                }
                Err(e) => {
                    warn!("corrupt session registry, starting fresh: {e}");
                    Vec::new()
                }
            },
            Ok(None) => {
                debug!("no persisted session registry found");
                Vec::new()
            }
            Err(e) => {
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
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_changed_at: Option<DateTime<Utc>>,
        objective_fingerprint: Option<String>,
    ) {
        let _global_write_guard = self.write_lock.lock().await;
        let _write_guard = self.thought_write_lock.lock().await;
        let data = {
            let mut thoughts = self.thought_cache.write().await;
            let objective_changed_at = objective_changed_at.or_else(|| {
                thoughts
                    .get(session_id)
                    .and_then(|existing| existing.objective_changed_at)
            });
            thoughts.insert(
                session_id.to_string(),
                ThoughtSnapshot {
                    thought: thought.map(|value| value.to_string()),
                    thought_state,
                    thought_source,
                    rest_state,
                    commit_candidate,
                    objective_changed_at,
                    objective_fingerprint,
                    token_count,
                    context_limit,
                    updated_at,
                    delivery,
                },
            );

            match serde_json::to_string_pretty(&*thoughts) {
                Ok(d) => d,
                Err(e) => {
                    error!("failed to serialize thoughts: {e}");
                    return;
                }
            }
        };

        let path = self.thoughts_path();
        if let Err(e) = atomic_write_blocking(path, data).await {
            error!("failed to write thoughts: {e}");
        } else {
            debug!(session_id, "persisted thought snapshot");
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
                    Ok(thoughts) => thoughts,
                    Err(e) => {
                        warn!("corrupt thoughts file, starting fresh: {e}");
                        HashMap::new()
                    }
                }
            }
            Ok(None) => HashMap::new(),
            Err(e) => {
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
        let data = serde_json::to_string_pretty(&normalized)
            .map_err(|e| anyhow::anyhow!("failed to serialize thought config: {e}"))?;
        atomic_write_blocking(path, data).await?;

        {
            let mut thought_config_cache = self.thought_config_cache.write().await;
            *thought_config_cache = normalized;
        }

        debug!("persisted thought runtime config");
        Ok(())
    }

    /// Sync persistence files and directory entries as a shutdown durability barrier.
    pub async fn flush_barrier(&self) -> anyhow::Result<()> {
        let _global_write_guard = self.write_lock.lock().await;
        let base_dir = self.base_dir.clone();
        let files = [
            self.registry_path(),
            self.thoughts_path(),
            self.thought_config_path(),
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
                    Ok(config) => config,
                    Err(e) => {
                        warn!("invalid thought config file, using defaults: {e}");
                        ThoughtConfig::default()
                    }
                },
                Err(e) => {
                    warn!("corrupt thought config file, using defaults: {e}");
                    ThoughtConfig::default()
                }
            },
            Ok(None) => ThoughtConfig::default(),
            Err(e) => {
                warn!("failed to read thought config file, using defaults: {e}");
                ThoughtConfig::default()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Blocking I/O helpers (run inside spawn_blocking)
// ---------------------------------------------------------------------------

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

        let tmp_path = path.with_extension(format!("json.tmp.{}", Uuid::new_v4()));
        let envelope = ChecksummedPayload {
            checksum_crc32: crc32fast::hash(data.as_bytes()),
            payload: data,
        };
        let encoded = serde_json::to_vec_pretty(&envelope)
            .map_err(|e| anyhow::anyhow!("serialize checksummed payload failed: {e}"))?;
        std::fs::write(&tmp_path, &encoded)
            .map_err(|e| anyhow::anyhow!("write to tmp failed: {e}"))?;
        std::fs::File::open(&tmp_path)
            .and_then(|f| f.sync_all())
            .map_err(|e| anyhow::anyhow!("sync tmp file failed: {e}"))?;
        if let Err(e) = std::fs::rename(&tmp_path, &path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(anyhow::anyhow!("rename failed: {e}"));
        }
        sync_parent_dir(&path)?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
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
