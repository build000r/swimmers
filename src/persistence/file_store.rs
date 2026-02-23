//! File-based JSON persistence for session registry and thought snapshots.
//!
//! All disk I/O is performed via `tokio::task::spawn_blocking` to avoid
//! blocking the async runtime. Writes use atomic rename (write to temp file,
//! then rename) for crash safety.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::types::{SessionState, ThoughtSource, ThoughtState};

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
    pub objective_fingerprint: Option<String>,
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
    pub objective_fingerprint: Option<String>,
    pub token_count: u64,
    pub context_limit: u64,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// FileStore
// ---------------------------------------------------------------------------

/// File-based persistence store. Thread-safe via internal RwLock on cached state.
pub struct FileStore {
    base_dir: PathBuf,
    /// In-memory cache of persisted sessions, synced to disk on mutation.
    cache: RwLock<Vec<PersistedSession>>,
    /// In-memory cache of thought snapshots, synced to disk on mutation.
    thought_cache: RwLock<HashMap<String, ThoughtSnapshot>>,
    /// Serialize thought writes to avoid stale read-modify-write races.
    thought_write_lock: Mutex<()>,
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
            cache: RwLock::new(Vec::new()),
            thought_cache: RwLock::new(HashMap::new()),
            thought_write_lock: Mutex::new(()),
        });

        // Load existing data into cache.
        let loaded = store.load_sessions_from_disk().await;
        let loaded_thoughts = store.load_thoughts_from_disk().await;
        {
            let mut cache = store.cache.write().await;
            *cache = loaded;
        }
        {
            let mut thought_cache = store.thought_cache.write().await;
            *thought_cache = loaded_thoughts;
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

    // -----------------------------------------------------------------------
    // Session registry
    // -----------------------------------------------------------------------

    /// Save the full session registry to disk atomically.
    pub async fn save_sessions(&self, sessions: &[PersistedSession]) {
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
    pub async fn save_thought(
        &self,
        session_id: &str,
        thought: Option<&str>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        objective_fingerprint: Option<String>,
    ) {
        let _write_guard = self.thought_write_lock.lock().await;
        let data = {
            let mut thoughts = self.thought_cache.write().await;
            thoughts.insert(
                session_id.to_string(),
                ThoughtSnapshot {
                    thought: thought.map(|value| value.to_string()),
                    thought_state,
                    thought_source,
                    objective_fingerprint,
                    token_count,
                    context_limit,
                    updated_at: Utc::now(),
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
}

// ---------------------------------------------------------------------------
// Blocking I/O helpers (run inside spawn_blocking)
// ---------------------------------------------------------------------------

/// Atomically write data to a file: write to `.tmp`, then rename.
async fn atomic_write_blocking(path: PathBuf, data: String) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, data.as_bytes())
            .map_err(|e| anyhow::anyhow!("write to tmp failed: {e}"))?;
        std::fs::rename(&tmp_path, &path).map_err(|e| anyhow::anyhow!("rename failed: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

/// Read a file's contents, returning None if the file does not exist.
async fn read_file_blocking(path: PathBuf) -> anyhow::Result<Option<String>> {
    tokio::task::spawn_blocking(move || match std::fs::read_to_string(&path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::anyhow!("read failed: {e}")),
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking panicked: {e}"))?
}

/// Convenience: convert a `Path` to an owned `PathBuf`.
#[allow(dead_code)]
fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
