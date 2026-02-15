use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot, RwLock};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::types::SessionSummary;

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

    /// Monotonic counter for generating numeric session names (matches Node.js
    /// behaviour where sessions are named "0", "1", "2", ...).
    next_name_counter: AtomicU64,

    /// Monotonic counter for session IDs (separate from tmux names).
    next_id_counter: AtomicU64,

    /// Broadcast channel for lifecycle events. Subscribers (e.g. the WebSocket
    /// hub) can listen for session_created / session_deleted.
    lifecycle_tx: broadcast::Sender<LifecycleEvent>,
}

impl SessionSupervisor {
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        let (lifecycle_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            config,
            sessions: RwLock::new(HashMap::new()),
            next_name_counter: AtomicU64::new(0),
            next_id_counter: AtomicU64::new(0),
            lifecycle_tx,
        })
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

            let session_id = self.allocate_session_id();
            info!(session_id = %session_id, tmux_name = %tmux_name, "discovered existing tmux session");

            match crate::session::actor::SessionActor::spawn(
                session_id.clone(),
                tmux_name.clone(),
                true, // attach to existing
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

        // Advance the name counter past any existing numeric names.
        self.next_name_counter.fetch_max(highest_numeric, Ordering::SeqCst);

        let sessions = self.sessions.read().await;
        info!(count = sessions.len(), "tmux session discovery complete");

        Ok(())
    }

    // -----------------------------------------------------------------------
    // CRUD
    // -----------------------------------------------------------------------

    /// Create a new tmux session (optionally with a specific name) and spawn
    /// an actor for it.
    pub async fn create_session(
        self: &Arc<Self>,
        name: Option<String>,
    ) -> anyhow::Result<SessionSummary> {
        let tmux_name = name.unwrap_or_else(|| {
            let n = self.next_name_counter.fetch_add(1, Ordering::SeqCst);
            n.to_string()
        });

        let session_id = self.allocate_session_id();

        info!(session_id = %session_id, tmux_name = %tmux_name, "creating new session");

        let handle = crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            false, // create new
            self.config.clone(),
        )?;

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), handle);

        let summary = self.build_placeholder_summary(&session_id, &tmux_name);

        // Broadcast lifecycle event.
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id: session_id.clone(),
            summary: summary.clone(),
            reason: "api_create".into(),
        });

        Ok(summary)
    }

    /// Shut down a session actor and remove it from the registry.
    /// This detaches the bridge but does NOT kill the tmux session.
    pub async fn delete_session(self: &Arc<Self>, session_id: &str) -> anyhow::Result<()> {
        let handle = {
            let mut sessions = self.sessions.write().await;
            sessions
                .remove(session_id)
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?
        };

        info!(session_id = %session_id, "deleting session");

        // Send shutdown command; if the channel is closed, the actor is already gone.
        let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;

        // Broadcast lifecycle event.
        let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
            session_id: session_id.to_string(),
            reason: "api_delete".into(),
        });

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
            if handle.cmd_tx.send(SessionCommand::GetSummary(tx)).await.is_ok() {
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

    /// Return all sessions for the bootstrap response.
    pub async fn bootstrap(&self) -> Vec<SessionSummary> {
        self.list_sessions().await
    }

    // -----------------------------------------------------------------------
    // Event subscription
    // -----------------------------------------------------------------------

    /// Subscribe to lifecycle events (session created/deleted).
    pub fn subscribe_events(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.lifecycle_tx.subscribe()
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
