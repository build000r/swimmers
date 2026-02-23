use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::persistence::file_store::{FileStore, PersistedSession, ThoughtSnapshot};
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::thought::loop_runner::{SessionInfo, SessionProvider};
use crate::types::{
    ControlEvent, SessionState, SessionStatePayload, SessionSummary, TerminalSnapshot,
    ThoughtSource, ThoughtState, TransportHealth,
};

const PROCESS_EXIT_REAP_INTERVAL: Duration = Duration::from_millis(250);
const PROCESS_EXIT_DELETE_GRACE: Duration = Duration::from_millis(2_500);
const PROCESS_EXIT_SUMMARY_TIMEOUT: Duration = Duration::from_millis(250);

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
        delete_mode: crate::config::SessionDeleteMode,
        tmux_session_alive: bool,
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

    /// Latest thought snapshots keyed by session_id.
    thought_snapshots: RwLock<HashMap<String, ThoughtSnapshot>>,

    /// First-observed timestamps for sessions that have entered Exited state.
    process_exit_seen_at: RwLock<HashMap<String, Instant>>,
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
            thought_snapshots: RwLock::new(HashMap::new()),
            process_exit_seen_at: RwLock::new(HashMap::new()),
        })
    }

    /// Initialize persistence store and load persisted sessions as stale entries.
    pub async fn init_persistence(self: &Arc<Self>, store: Arc<FileStore>) {
        let persisted = store.load_sessions().await;
        let thoughts = store.load_thoughts().await;

        // Keep the ID counter ahead of any IDs we have ever persisted.
        for ps in &persisted {
            self.bump_id_counter_from_session_id(&ps.session_id);
        }

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
                    thought_state: thought_data
                        .map(|t| t.thought_state)
                        .unwrap_or(ps.thought_state),
                    thought_source: thought_data
                        .map(|t| t.thought_source)
                        .unwrap_or(ps.thought_source),
                    thought_updated_at: thought_data
                        .map(|t| t.updated_at)
                        .or(ps.thought_updated_at),
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

        {
            let mut thought_cache = self.thought_snapshots.write().await;
            *thought_cache = thoughts;
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

        let mut discovery_reliable = true;
        let mut highest_numeric: u64 = 0;
        let mut discovered_tmux_names: Vec<String> = Vec::new();
        match output {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
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

                    let session_id = match reuse_id {
                        Some(id) => {
                            // Reused IDs must also advance the counter so future creates
                            // don't accidentally collide with restored sessions.
                            self.bump_id_counter_from_session_id(&id);
                            id
                        }
                        None => self.allocate_unique_session_id().await,
                    };
                    info!(
                        session_id = %session_id,
                        tmux_name = %tmux_name,
                        "discovered existing tmux session"
                    );

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
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // "no server running" / "no sessions" means tmux has no live sessions.
                if stderr.contains("no server running") || stderr.contains("no sessions") {
                    info!("no existing tmux sessions found");
                } else {
                    discovery_reliable = false;
                    warn!("tmux list-sessions returned error: {}", stderr);
                }
            }
            Err(e) => {
                discovery_reliable = false;
                warn!("tmux list-sessions failed: {}", e);
            }
        }

        // Remove stale sessions that were upgraded to live.
        // For sessions still missing from tmux, emit startup exit/deletion events
        // and then drop them so bootstrap does not keep phantom entries forever.
        if discovery_reliable {
            let unresolved_stale = {
                let mut stale = self.stale_sessions.write().await;
                stale.retain(|s| !discovered_tmux_names.contains(&s.tmux_name));
                let unresolved = stale.clone();
                stale.clear();
                unresolved
            };

            if !unresolved_stale.is_empty() {
                debug!(
                    remaining_stale = unresolved_stale.len(),
                    "stale sessions after discovery"
                );
                for s in unresolved_stale {
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

                    let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
                        session_id: s.session_id,
                        reason: "startup_missing_tmux".to_string(),
                        delete_mode: crate::config::SessionDeleteMode::DetachBridge,
                        tmux_session_alive: false,
                    });
                }
            }
        } else {
            warn!("skipping stale reconciliation due unreliable tmux discovery");
        }

        // Advance the name counter past any existing numeric names.
        self.next_name_counter
            .fetch_max(highest_numeric, Ordering::SeqCst);

        let sessions = self.sessions.read().await;
        crate::metrics::set_active_sessions(sessions.len());
        info!(count = sessions.len(), "tmux session discovery complete");

        // Persist the current session registry only when discovery succeeded
        // (or definitively found no tmux sessions).
        if discovery_reliable {
            self.persist_registry().await;
        }

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
        spawn_tool: Option<crate::types::SpawnTool>,
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

        let session_id = self.allocate_unique_session_id().await;

        info!(session_id = %session_id, tmux_name = %tmux_name, "creating new session");

        let handle = crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            false, // create new
            start_cwd.clone(),
            self.config.clone(),
        )?;
        let bootstrap_handle = handle.clone();

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), handle);
        crate::metrics::set_active_sessions(sessions.len());
        drop(sessions);

        let mut summary = self.build_placeholder_summary(&session_id, &tmux_name);
        if let Some(cwd) = start_cwd {
            summary.cwd = cwd;
        }

        if let Some(tool) = spawn_tool {
            if let Err(e) = send_spawn_tool_command(&tmux_name, tool).await {
                warn!(
                    session_id = %session_id,
                    tmux_name = %tmux_name,
                    tool = ?tool,
                    "tmux send-keys failed, falling back to PTY input: {}",
                    e
                );
                let mut command = String::from(tool.command());
                command.push('\n');
                if let Err(e) = bootstrap_handle
                    .send(SessionCommand::WriteInput(command.into_bytes()))
                    .await
                {
                    warn!(
                        session_id = %session_id,
                        tmux_name = %tmux_name,
                        tool = ?tool,
                        "failed to enqueue spawn command fallback: {}",
                        e
                    );
                }
            }
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
    /// Depending on `delete_mode`, this either detaches the bridge or also
    /// kills the underlying tmux session.
    pub async fn delete_session(
        self: &Arc<Self>,
        session_id: &str,
        delete_mode: crate::config::SessionDeleteMode,
    ) -> anyhow::Result<()> {
        let handle = {
            let mut sessions = self.sessions.write().await;
            let handle = sessions
                .remove(session_id)
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;
            crate::metrics::set_active_sessions(sessions.len());
            handle
        };
        let mut tmux_session_alive = true;

        if matches!(delete_mode, crate::config::SessionDeleteMode::KillTmux) {
            if let Err(e) = kill_tmux_session(&handle.tmux_name).await {
                let mut sessions = self.sessions.write().await;
                sessions.insert(session_id.to_string(), handle.clone());
                crate::metrics::set_active_sessions(sessions.len());
                return Err(e);
            }
            tmux_session_alive = false;
        }

        self.process_exit_seen_at.write().await.remove(session_id);

        info!(
            session_id = %session_id,
            delete_mode = ?delete_mode,
            "deleting session"
        );

        // Send shutdown command; if the channel is closed, the actor is already gone.
        let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;

        // Broadcast lifecycle event.
        let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
            session_id: session_id.to_string(),
            reason: "api_delete".into(),
            delete_mode,
            tmux_session_alive,
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
        let handles: Vec<ActorHandle> = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect()
        };
        let mut summaries = Vec::with_capacity(handles.len());

        for handle in handles {
            let (tx, rx) = oneshot::channel();
            if handle
                .cmd_tx
                .send(SessionCommand::GetSummary(tx))
                .await
                .is_ok()
            {
                match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
                    Ok(Ok(summary)) => {
                        if summary.state != SessionState::Exited {
                            summaries.push(summary);
                        }
                    }
                    Ok(Err(_)) => {
                        warn!(session_id = %handle.session_id, "actor dropped summary reply");
                    }
                    Err(_) => {
                        warn!(session_id = %handle.session_id, "summary request timed out");
                    }
                }
            }
        }

        let thought_snapshots = self.thought_snapshots.read().await;
        for summary in &mut summaries {
            if let Some(thought_data) = thought_snapshots.get(&summary.session_id) {
                if summary.thought.is_none() {
                    summary.thought = thought_data.thought.clone();
                }
                summary.thought_state = thought_data.thought_state;
                summary.thought_source = thought_data.thought_source;
                summary.thought_updated_at = Some(thought_data.updated_at);
                if summary.token_count == 0 {
                    summary.token_count = thought_data.token_count;
                }
                if summary.context_limit == 0 {
                    summary.context_limit = thought_data.context_limit;
                }
            }
        }

        summaries
    }

    /// Return all sessions for the bootstrap response, including stale
    /// (exited) sessions from persistence.
    pub async fn bootstrap(&self) -> Vec<SessionSummary> {
        let mut all = self.list_sessions().await;
        let mut seen_ids: HashSet<String> = all.iter().map(|s| s.session_id.clone()).collect();

        // Append stale sessions that haven't been upgraded to live.
        let stale = self.stale_sessions.read().await;
        for s in stale.iter() {
            if !seen_ids.insert(s.session_id.clone()) {
                warn!(
                    session_id = %s.session_id,
                    tmux_name = %s.tmux_name,
                    "dropping duplicate stale session_id from bootstrap"
                );
                continue;
            }
            all.push(s.clone());
        }

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
        let thought_snapshots = self.thought_snapshots.read().await.clone();
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
                let session_id = summary.session_id.clone();
                let thought_data = thought_snapshots.get(&session_id);

                infos.push(SessionInfo {
                    session_id,
                    state: summary.state,
                    exited: summary.state == crate::types::SessionState::Exited,
                    tool: summary.tool,
                    cwd: summary.cwd,
                    replay_text,
                    thought_state: thought_data
                        .map(|t| t.thought_state)
                        .unwrap_or(summary.thought_state),
                    thought_source: thought_data
                        .map(|t| t.thought_source)
                        .unwrap_or(summary.thought_source),
                    thought: thought_data
                        .and_then(|t| t.thought.clone())
                        .or_else(|| summary.thought.clone()),
                    thought_updated_at: thought_data.map(|t| t.updated_at).or(summary.thought_updated_at),
                    objective_fingerprint: thought_data
                        .and_then(|t| t.objective_fingerprint.clone()),
                    token_count: thought_data
                        .map(|t| t.token_count)
                        .unwrap_or(summary.token_count),
                    context_limit: thought_data
                        .map(|t| t.context_limit)
                        .unwrap_or(summary.context_limit),
                    last_activity_at: summary.last_activity_at,
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
        let thought_snapshots = self.thought_snapshots.read().await;
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
                thought_state: s.thought_state,
                thought_source: s.thought_source,
                thought_updated_at: s.thought_updated_at,
                objective_fingerprint: thought_snapshots
                    .get(&s.session_id)
                    .and_then(|t| t.objective_fingerprint.clone()),
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
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        objective_fingerprint: Option<String>,
    ) {
        {
            let mut thought_snapshots = self.thought_snapshots.write().await;
            thought_snapshots.insert(
                session_id.to_string(),
                ThoughtSnapshot {
                    thought: Some(thought.to_string()),
                    thought_state,
                    thought_source,
                    objective_fingerprint: objective_fingerprint.clone(),
                    token_count,
                    context_limit,
                    updated_at: Utc::now(),
                },
            );
        }

        let store = {
            let guard = self.persistence.read().await;
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => return,
            }
        };

        store
            .save_thought(
                session_id,
                thought,
                token_count,
                context_limit,
                thought_state,
                thought_source,
                objective_fingerprint,
            )
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

    /// Spawn a background task that reaps exited sessions after a short grace
    /// period so clients can animate state transitions before deletion.
    pub fn spawn_process_exit_reaper(self: &Arc<Self>) {
        let supervisor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PROCESS_EXIT_REAP_INTERVAL);
            loop {
                interval.tick().await;
                supervisor.reap_exited_sessions().await;
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

    async fn allocate_unique_session_id(&self) -> String {
        loop {
            let candidate = self.allocate_session_id();

            {
                let sessions = self.sessions.read().await;
                if sessions.contains_key(&candidate) {
                    continue;
                }
            }

            {
                let stale = self.stale_sessions.read().await;
                if stale.iter().any(|s| s.session_id == candidate) {
                    continue;
                }
            }

            return candidate;
        }
    }

    fn bump_id_counter_from_session_id(&self, session_id: &str) {
        if let Some(next) = next_session_counter(session_id) {
            self.next_id_counter.fetch_max(next, Ordering::SeqCst);
        }
    }

    async fn collect_exited_session_ids(&self, timeout: Duration) -> HashSet<String> {
        let handles: Vec<ActorHandle> = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect()
        };

        let mut exited_ids = HashSet::new();
        for handle in handles {
            let (tx, rx) = oneshot::channel();
            if handle
                .cmd_tx
                .send(SessionCommand::GetSummary(tx))
                .await
                .is_err()
            {
                continue;
            }
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(summary)) if summary.state == SessionState::Exited => {
                    exited_ids.insert(summary.session_id);
                }
                Ok(Ok(_)) => {}
                Ok(Err(_)) => {
                    debug!(
                        session_id = %handle.session_id,
                        "reaper summary channel dropped"
                    );
                }
                Err(_) => {
                    debug!(
                        session_id = %handle.session_id,
                        "reaper summary request timed out"
                    );
                }
            }
        }

        exited_ids
    }

    async fn reap_exited_sessions(&self) {
        let exited_ids = self
            .collect_exited_session_ids(PROCESS_EXIT_SUMMARY_TIMEOUT)
            .await;
        let now = Instant::now();
        let ready = {
            let mut seen = self.process_exit_seen_at.write().await;
            ready_process_exit_ids(&mut seen, &exited_ids, now, PROCESS_EXIT_DELETE_GRACE)
        };
        if ready.is_empty() {
            return;
        }

        let removed: Vec<ActorHandle> = {
            let mut sessions = self.sessions.write().await;
            let mut removed = Vec::with_capacity(ready.len());
            for session_id in &ready {
                if let Some(handle) = sessions.remove(session_id) {
                    removed.push(handle);
                }
            }
            crate::metrics::set_active_sessions(sessions.len());
            removed
        };

        if removed.is_empty() {
            return;
        }

        {
            let mut seen = self.process_exit_seen_at.write().await;
            for handle in &removed {
                seen.remove(&handle.session_id);
            }
        }

        for handle in removed {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
            let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
                session_id: handle.session_id,
                reason: "process_exit".to_string(),
                delete_mode: crate::config::SessionDeleteMode::DetachBridge,
                tmux_session_alive: false,
            });
        }

        self.persist_registry().await;
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
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
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
const THOUGHT_PERSIST_QUEUE_CAP: usize = 256;

pub struct SupervisorProvider {
    supervisor: Arc<SessionSupervisor>,
    handle: tokio::runtime::Handle,
    persist_tx: mpsc::Sender<PersistThoughtRequest>,
}

struct PersistThoughtRequest {
    session_id: String,
    thought: String,
    token_count: u64,
    context_limit: u64,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    objective_fingerprint: Option<String>,
}

impl SupervisorProvider {
    pub fn new(supervisor: Arc<SessionSupervisor>) -> Self {
        let handle = tokio::runtime::Handle::current();
        let (persist_tx, mut persist_rx) =
            mpsc::channel::<PersistThoughtRequest>(THOUGHT_PERSIST_QUEUE_CAP);
        let persist_supervisor = supervisor.clone();
        handle.spawn(async move {
            while let Some(req) = persist_rx.recv().await {
                persist_supervisor
                    .persist_thought(
                        &req.session_id,
                        &req.thought,
                        req.token_count,
                        req.context_limit,
                        req.thought_state,
                        req.thought_source,
                        req.objective_fingerprint,
                    )
                    .await;
            }
        });

        Self {
            supervisor,
            handle,
            persist_tx,
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

    fn persist_thought(
        &self,
        session_id: &str,
        thought: &str,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        objective_fingerprint: Option<String>,
    ) {
        if self.persist_tx.try_send(PersistThoughtRequest {
                session_id: session_id.to_string(),
                thought: thought.to_string(),
                token_count,
                context_limit,
                thought_state,
                thought_source,
                objective_fingerprint,
            })
            .is_err()
        {
            warn!(
                session_id = %session_id,
                "persist_thought queue full/closed; dropping thought snapshot"
            );
        }
    }
}

fn current_working_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

async fn send_spawn_tool_command(
    tmux_name: &str,
    tool: crate::types::SpawnTool,
) -> anyhow::Result<()> {
    const ATTEMPTS: usize = 8;
    const RETRY_DELAY_MS: u64 = 75;

    for attempt in 1..=ATTEMPTS {
        let output = Command::new("tmux")
            .args(["send-keys", "-t", tmux_name, tool.command(), "Enter"])
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .output()
            .await;

        match output {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                debug!(
                    tmux_name = %tmux_name,
                    tool = ?tool,
                    attempt,
                    status = ?output.status,
                    stderr = %stderr.trim(),
                    "tmux send-keys attempt failed"
                );
            }
            Err(e) => {
                debug!(
                    tmux_name = %tmux_name,
                    tool = ?tool,
                    attempt,
                    "failed to execute tmux send-keys: {}",
                    e
                );
            }
        }

        if attempt < ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
        }
    }

    Err(anyhow::anyhow!(
        "unable to inject spawn command via tmux send-keys"
    ))
}

async fn kill_tmux_session(tmux_name: &str) -> anyhow::Result<()> {
    let output = Command::new("tmux")
        .args(["kill-session", "-t", tmux_name])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tmux kill-session: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("can't find session") || stderr.contains("no server running") {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "tmux kill-session failed: {}",
        stderr.trim()
    ))
}

fn next_session_counter(session_id: &str) -> Option<u64> {
    let n = session_id.strip_prefix("sess_")?.parse::<u64>().ok()?;
    Some(n.saturating_add(1))
}

fn ready_process_exit_ids(
    seen: &mut HashMap<String, Instant>,
    exited_ids: &HashSet<String>,
    now: Instant,
    grace: Duration,
) -> Vec<String> {
    seen.retain(|session_id, _| exited_ids.contains(session_id));
    for session_id in exited_ids {
        seen.entry(session_id.clone()).or_insert(now);
    }

    seen.iter()
        .filter_map(|(session_id, first_seen)| {
            (now.duration_since(*first_seen) >= grace).then(|| session_id.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter::FromIterator;

    #[test]
    fn next_session_counter_parses_expected_format() {
        assert_eq!(next_session_counter("sess_0"), Some(1));
        assert_eq!(next_session_counter("sess_41"), Some(42));
    }

    #[test]
    fn next_session_counter_rejects_unexpected_format() {
        assert_eq!(next_session_counter("session_1"), None);
        assert_eq!(next_session_counter("sess_not_a_number"), None);
        assert_eq!(next_session_counter(""), None);
    }

    #[test]
    fn ready_process_exit_ids_waits_for_grace_period() {
        let mut seen = HashMap::new();
        let exited = HashSet::from_iter(["sess_1".to_string()]);
        let start = Instant::now();

        let before = ready_process_exit_ids(&mut seen, &exited, start, Duration::from_secs(2));
        assert!(before.is_empty());

        let near = ready_process_exit_ids(
            &mut seen,
            &exited,
            start + Duration::from_millis(1_999),
            Duration::from_secs(2),
        );
        assert!(near.is_empty());

        let after = ready_process_exit_ids(
            &mut seen,
            &exited,
            start + Duration::from_secs(2),
            Duration::from_secs(2),
        );
        assert_eq!(after, vec!["sess_1".to_string()]);
    }

    #[test]
    fn ready_process_exit_ids_drops_recovered_sessions() {
        let mut seen = HashMap::new();
        let exited = HashSet::from_iter(["sess_1".to_string()]);
        let start = Instant::now();

        let _ = ready_process_exit_ids(&mut seen, &exited, start, Duration::from_secs(2));
        assert!(seen.contains_key("sess_1"));

        let none = HashSet::new();
        let ready = ready_process_exit_ids(
            &mut seen,
            &none,
            start + Duration::from_secs(10),
            Duration::from_secs(2),
        );
        assert!(ready.is_empty());
        assert!(!seen.contains_key("sess_1"));
    }
}
