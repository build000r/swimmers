use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot, RwLock};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::persistence::file_store::{FileStore, PersistedSession};
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::thought::loop_runner::{SessionInfo, SessionProvider};
use crate::types::{
    ControlEvent, SessionState, SessionStatePayload, SessionSummary, TerminalSnapshot,
    TransportHealth,
};

// ---------------------------------------------------------------------------
// Lifecycle events broadcast to all listeners
// ---------------------------------------------------------------------------

/// Events emitted by the supervisor when sessions are created or removed.
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    Created {
        session_id: String,
        summary: SessionSummary,
        reason: String,
    },
    Deleted {
        session_id: String,
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Session supervisor
// ---------------------------------------------------------------------------

pub struct SessionSupervisor {
    config: Arc<Config>,

    /// Active session actors keyed by session_id.
    sessions: RwLock<HashMap<String, ActorHandle>>,

    /// Stale (exited) sessions from persistence that have no matching live tmux.
    stale_sessions: RwLock<Vec<SessionSummary>>,

    /// Monotonic counter for generating numeric fallback session names.
    next_name_counter: AtomicU64,

    /// Monotonic counter for session IDs (separate from tmux names).
    next_id_counter: AtomicU64,

    /// Broadcast channel for lifecycle events. Subscribers (e.g. the WebSocket
    /// hub) can listen for session_created / session_deleted.
    lifecycle_tx: broadcast::Sender<LifecycleEvent>,

    /// Broadcast channel for thought_update ControlEvents from the thought loop.
    /// WebSocket handlers subscribe to this to forward thought updates to clients.
    thought_tx: broadcast::Sender<ControlEvent>,

    /// File-based persistence store, initialized after construction.
    persistence: RwLock<Option<Arc<FileStore>>>,
}

impl SessionSupervisor {
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        let (lifecycle_tx, _) = broadcast::channel(64);
        let (thought_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            config,
            sessions: RwLock::new(HashMap::new()),
            stale_sessions: RwLock::new(Vec::new()),
            next_name_counter: AtomicU64::new(0),
            next_id_counter: AtomicU64::new(0),
            lifecycle_tx,
            thought_tx,
            persistence: RwLock::new(None),
        })
    }

    /// Initialize persistence store and load persisted sessions as stale entries.
    pub async fn init_persistence(self: &Arc<Self>, store: Arc<FileStore>) {
        let persisted = store.load_sessions().await;
        let thoughts = store.load_thoughts().await;

        if !persisted.is_empty() {
            let mut stale = Vec::new();
            for ps in &persisted {
                let thought_data = thoughts.get(&ps.session_id);
                let summary = SessionSummary {
                    session_id: ps.session_id.clone(),
                    tmux_name: ps.tmux_name.clone(),
                    state: crate::types::SessionState::Exited,
                    current_command: None,
                    cwd: ps.cwd.clone(),
                    tool: ps.tool.clone(),
                    token_count: thought_data
                        .map(|t| t.token_count)
                        .unwrap_or(ps.token_count),
                    context_limit: thought_data
                        .map(|t| t.context_limit)
                        .unwrap_or(ps.context_limit),
                    thought: thought_data
                        .and_then(|t| t.thought.clone())
                        .or_else(|| ps.thought.clone()),
                    is_stale: true,
                    attached_clients: 0,
                    transport_health: crate::types::TransportHealth::Disconnected,
                    last_activity_at: ps.last_activity_at,
                };
                stale.push(summary);
            }
            info!(count = stale.len(), "loaded persisted stale sessions");
            let mut stale_lock = self.stale_sessions.write().await;
            *stale_lock = stale;
        }

        let mut persistence = self.persistence.write().await;
        *persistence = Some(store);
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Discover existing tmux sessions and create actors for each one.
    /// Called once at server startup.
    pub async fn discover_tmux_sessions(self: &Arc<Self>) -> anyhow::Result<()> {
        let output = Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
            .await;

        let output = match output {
            Ok(o) => o,
            Err(e) => {
                // tmux may not be running at all -- that's fine, no sessions to discover.
                warn!("tmux list-sessions failed (tmux may not be running): {}", e);
                return Ok(());
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "no server running" is normal when there are no sessions.
            if stderr.contains("no server running") || stderr.contains("no sessions") {
                info!("no existing tmux sessions found");
                return Ok(());
            }
            warn!("tmux list-sessions returned error: {}", stderr);
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut highest_numeric: u64 = 0;
        let mut discovered_tmux_names: Vec<String> = Vec::new();

        for line in stdout.lines() {
            let tmux_name = line.trim().to_string();
            if tmux_name.is_empty() {
                continue;
            }

            // Track the highest numeric name so our counter stays ahead.
            if let Ok(n) = tmux_name.parse::<u64>() {
                if n >= highest_numeric {
                    highest_numeric = n + 1;
                }
            }

            discovered_tmux_names.push(tmux_name.clone());

            // Check if this tmux session matches a stale persisted session.
            let reuse_id = {
                let stale = self.stale_sessions.read().await;
                stale
                    .iter()
                    .find(|s| s.tmux_name == tmux_name)
                    .map(|s| s.session_id.clone())
            };

            let session_id = reuse_id.unwrap_or_else(|| self.allocate_session_id());
            info!(session_id = %session_id, tmux_name = %tmux_name, "discovered existing tmux session");

            match crate::session::actor::SessionActor::spawn(
                session_id.clone(),
                tmux_name.clone(),
                true, // attach to existing
                None,
                self.config.clone(),
            ) {
                Ok(handle) => {
                    let mut sessions = self.sessions.write().await;
                    sessions.insert(session_id.clone(), handle);

                    // Broadcast lifecycle event.
                    let summary = self.build_placeholder_summary(&session_id, &tmux_name);
                    let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
                        session_id,
                        summary,
                        reason: "startup_discovery".into(),
                    });
                }
                Err(e) => {
                    error!(tmux_name = %tmux_name, "failed to attach to tmux session: {}", e);
                }
            }
        }

        // Remove stale sessions that were upgraded to live.
        // Emit session_state with exit_reason="startup_missing_tmux" for remaining stale sessions.
        {
            let mut stale = self.stale_sessions.write().await;
            stale.retain(|s| !discovered_tmux_names.contains(&s.tmux_name));
            if !stale.is_empty() {
                debug!(
                    remaining_stale = stale.len(),
                    "stale sessions after discovery"
                );
                for s in stale.iter() {
                    // Broadcast session_state with exit_reason for already-connected
                    // clients. Do NOT emit LifecycleEvent::Created for stale sessions
                    // — they are already present in bootstrap payloads from persistence
                    // and emitting Created would confuse client state machines.
                    let payload = SessionStatePayload {
                        state: SessionState::Exited,
                        previous_state: s.state,
                        current_command: s.current_command.clone(),
                        transport_health: TransportHealth::Disconnected,
                        exit_reason: Some("startup_missing_tmux".to_string()),
                        at: Utc::now(),
                    };
                    let event = ControlEvent {
                        event: "session_state".to_string(),
                        session_id: s.session_id.clone(),
                        payload: serde_json::to_value(&payload).unwrap_or_default(),
                    };
                    let _ = self.thought_tx.send(event);
                }
            }
        }

        // Advance the name counter past any existing numeric names.
        self.next_name_counter
            .fetch_max(highest_numeric, Ordering::SeqCst);

        let sessions = self.sessions.read().await;
        crate::metrics::set_active_sessions(sessions.len());
        info!(count = sessions.len(), "tmux session discovery complete");

        // Persist the current session registry.
        self.persist_registry().await;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // CRUD
    // -----------------------------------------------------------------------

    /// Create a new tmux session (optionally with a specific name and/or
    /// working directory) and spawn an actor for it.
    pub async fn create_session(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: Option<String>,
    ) -> anyhow::Result<SessionSummary> {
        let start_cwd = cwd.or_else(current_working_dir);
        let requested_name = name.and_then(|n| {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let tmux_name = match requested_name {
            Some(explicit) => explicit,
            None => {
                let n = self.next_name_counter.fetch_add(1, Ordering::SeqCst);
                n.to_string()
            }
        };

        let session_id = self.allocate_session_id();

        info!(session_id = %session_id, tmux_name = %tmux_name, "creating new session");

        let handle = crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            false, // create new
            start_cwd.clone(),
            self.config.clone(),
        )?;

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), handle);
        crate::metrics::set_active_sessions(sessions.len());
        drop(sessions);

        let mut summary = self.build_placeholder_summary(&session_id, &tmux_name);
        if let Some(cwd) = start_cwd {
            summary.cwd = cwd;
        }

        // Broadcast lifecycle event.
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id: session_id.clone(),
            summary: summary.clone(),
            reason: "api_create".into(),
        });

        // Persist the updated registry.
        self.persist_registry().await;

        Ok(summary)
    }

    /// Shut down a session actor and remove it from the registry.
    /// This detaches the bridge but does NOT kill the tmux session.
    pub async fn delete_session(self: &Arc<Self>, session_id: &str) -> anyhow::Result<()> {
        let handle = {
            let mut sessions = self.sessions.write().await;
            let handle = sessions
                .remove(session_id)
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;
            crate::metrics::set_active_sessions(sessions.len());
            handle
        };

        info!(session_id = %session_id, "deleting session");

        // Send shutdown command; if the channel is closed, the actor is already gone.
        let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;

        // Broadcast lifecycle event.
        let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
            session_id: session_id.to_string(),
            reason: "api_delete".into(),
        });

        // Persist the updated registry.
        self.persist_registry().await;

        Ok(())
    }

    /// Get the actor handle for a session.
    pub async fn get_session(&self, session_id: &str) -> Option<ActorHandle> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    /// List summaries for all active sessions.
    pub async fn list_sessions(&self) -> Vec<SessionSummary> {
        let sessions = self.sessions.read().await;
        let mut summaries = Vec::with_capacity(sessions.len());

        for (_, handle) in sessions.iter() {
            let (tx, rx) = oneshot::channel();
            if handle
                .cmd_tx
                .send(SessionCommand::GetSummary(tx))
                .await
                .is_ok()
            {
                match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
                    Ok(Ok(summary)) => summaries.push(summary),
                    Ok(Err(_)) => {
                        warn!(session_id = %handle.session_id, "actor dropped summary reply");
                    }
                    Err(_) => {
                        warn!(session_id = %handle.session_id, "summary request timed out");
                    }
                }
            }
        }

        summaries
    }

    /// Return all sessions for the bootstrap response, including stale
    /// (exited) sessions from persistence.
    pub async fn bootstrap(&self) -> Vec<SessionSummary> {
        let mut all = self.list_sessions().await;

        // Append stale sessions that haven't been upgraded to live.
        let stale = self.stale_sessions.read().await;
        all.extend(stale.iter().cloned());

        all
    }

    // -----------------------------------------------------------------------
    // Event subscription
    // -----------------------------------------------------------------------

    /// Subscribe to lifecycle events (session created/deleted).
    pub fn subscribe_events(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    /// Subscribe to thought_update ControlEvents from the thought loop.
    pub fn subscribe_thought_events(&self) -> broadcast::Receiver<ControlEvent> {
        self.thought_tx.subscribe()
    }

    /// Get a clone of the thought event sender. Used to wire the ThoughtLoopRunner.
    pub fn thought_event_sender(&self) -> broadcast::Sender<ControlEvent> {
        self.thought_tx.clone()
    }

    // -----------------------------------------------------------------------
    // Session snapshots for thought generation
    // -----------------------------------------------------------------------

    /// Collect session snapshots (summary + replay text) for all live sessions.
    /// Used by the thought loop to generate thoughts.
    pub async fn collect_session_snapshots(&self) -> Vec<SessionInfo> {
        let sessions = self.sessions.read().await;
        let mut infos = Vec::with_capacity(sessions.len());

        for (_, handle) in sessions.iter() {
            // Request summary and snapshot from the actor.
            let (sum_tx, sum_rx) = oneshot::channel();
            let (snap_tx, snap_rx) = oneshot::channel();

            let summary_sent = handle
                .cmd_tx
                .send(SessionCommand::GetSummary(sum_tx))
                .await
                .is_ok();
            let snapshot_sent = handle
                .cmd_tx
                .send(SessionCommand::GetSnapshot(snap_tx))
                .await
                .is_ok();

            if !summary_sent || !snapshot_sent {
                continue;
            }

            let timeout = std::time::Duration::from_secs(2);
            let summary: Option<SessionSummary> = match tokio::time::timeout(timeout, sum_rx).await
            {
                Ok(Ok(s)) => Some(s),
                _ => None,
            };
            let snapshot: Option<TerminalSnapshot> =
                match tokio::time::timeout(timeout, snap_rx).await {
                    Ok(Ok(s)) => Some(s),
                    _ => None,
                };

            if let Some(summary) = summary {
                let replay_text = snapshot
                    .map(|s| {
                        // Take last ~500 chars of screen text.
                        let chars: Vec<char> = s.screen_text.chars().collect();
                        let start = chars.len().saturating_sub(500);
                        chars[start..].iter().collect()
                    })
                    .unwrap_or_default();

                infos.push(SessionInfo {
                    session_id: summary.session_id,
                    state: summary.state,
                    exited: summary.state == crate::types::SessionState::Exited,
                    tool: summary.tool,
                    cwd: summary.cwd,
                    replay_text,
                    token_count: summary.token_count,
                    context_limit: summary.context_limit,
                });
            }
        }

        infos
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Persist the current session registry to disk.
    pub async fn persist_registry(&self) {
        let store = {
            let guard = self.persistence.read().await;
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => return,
            }
        };

        let summaries = self.list_sessions().await;
        let persisted: Vec<PersistedSession> = summaries
            .iter()
            .map(|s| PersistedSession {
                session_id: s.session_id.clone(),
                tmux_name: s.tmux_name.clone(),
                state: s.state,
                tool: s.tool.clone(),
                token_count: s.token_count,
                context_limit: s.context_limit,
                thought: s.thought.clone(),
                cwd: s.cwd.clone(),
                last_activity_at: s.last_activity_at,
            })
            .collect();

        store.save_sessions(&persisted).await;
    }

    /// Persist a thought update for a specific session.
    pub async fn persist_thought(
        &self,
        session_id: &str,
        thought: &str,
        token_count: u64,
        context_limit: u64,
    ) {
        let store = {
            let guard = self.persistence.read().await;
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => return,
            }
        };

        store
            .save_thought(session_id, thought, token_count, context_limit)
            .await;
    }

    /// Spawn a background task that periodically persists the session registry.
    pub fn spawn_persistence_checkpoint(self: &Arc<Self>) {
        let supervisor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                supervisor.persist_registry().await;
            }
        });
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    fn allocate_session_id(&self) -> String {
        let n = self.next_id_counter.fetch_add(1, Ordering::SeqCst);
        format!("sess_{}", n)
    }

    /// Build a minimal placeholder summary. The real summary comes from the
    /// actor via `GetSummary`, but we need something for lifecycle events that
    /// fire before the actor has processed any output.
    fn build_placeholder_summary(&self, session_id: &str, tmux_name: &str) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: tmux_name.to_string(),
            state: crate::types::SessionState::Idle,
            current_command: None,
            cwd: String::new(),
            tool: None,
            token_count: 0,
            context_limit: 128_000,
            thought: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: crate::types::TransportHealth::Healthy,
            last_activity_at: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// SessionProvider implementation for the thought loop
// ---------------------------------------------------------------------------

/// Wrapper that implements the synchronous `SessionProvider` trait by using
/// a dedicated thread to call async supervisor methods without panicking
/// from within the tokio runtime.
pub struct SupervisorProvider {
    supervisor: Arc<SessionSupervisor>,
    handle: tokio::runtime::Handle,
}

impl SupervisorProvider {
    pub fn new(supervisor: Arc<SessionSupervisor>) -> Self {
        Self {
            supervisor,
            handle: tokio::runtime::Handle::current(),
        }
    }
}

impl SessionProvider for SupervisorProvider {
    fn session_snapshots(&self) -> Vec<SessionInfo> {
        // The thought loop runner calls this from a tokio::spawn async task.
        // We cannot call handle.block_on() from within an async context (it
        // would panic). Instead, use std::thread::scope to run block_on from
        // a non-async thread.
        let supervisor = self.supervisor.clone();
        let handle = self.handle.clone();
        std::thread::scope(|s| {
            s.spawn(|| handle.block_on(supervisor.collect_session_snapshots()))
                .join()
                .expect("session_snapshots thread panicked")
        })
    }
}

fn current_working_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}
