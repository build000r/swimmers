use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::persistence::file_store::{FileStore, PersistedSession, ThoughtSnapshot};
use crate::repo_theme::discover_repo_theme;
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::sprites::discover_sprite_pack;
use crate::thought::loop_runner::{SessionInfo, SessionProvider};
use crate::thought::protocol::ThoughtDeliveryState;
use crate::types::{
    fallback_rest_state, ControlEvent, RepoTheme, RestState, SessionState, SessionStatePayload,
    SessionSummary, SpritePack, TerminalSnapshot, ThoughtSource, ThoughtState, TransportHealth,
};

const PROCESS_EXIT_REAP_INTERVAL: Duration = Duration::from_millis(250);
const PROCESS_EXIT_DELETE_GRACE: Duration = Duration::ZERO;
const PROCESS_EXIT_SUMMARY_TIMEOUT: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------------------
// Bootstrap result
// ---------------------------------------------------------------------------

/// Returned by [`SessionSupervisor::bootstrap`]; bundles the session list with
/// any per-repository sprite packs discovered from session cwds.
pub struct BootstrapData {
    pub sessions: Vec<SessionSummary>,
    pub sprite_packs: HashMap<String, SpritePack>,
    pub repo_themes: HashMap<String, RepoTheme>,
}

fn thought_snapshot_for_summary<'a>(
    summary: &SessionSummary,
    active_pane_session_id: Option<&str>,
    thought_snapshots: &'a HashMap<String, ThoughtSnapshot>,
) -> Option<&'a ThoughtSnapshot> {
    thought_snapshots
        .get(&summary.session_id)
        .or_else(|| active_pane_session_id.and_then(|session_id| thought_snapshots.get(session_id)))
}

async fn query_tmux_active_pane_session_id(tmux_name: &str) -> anyhow::Result<String> {
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            tmux_name,
            "#{window_index}.#{pane_index}:#{pane_id}",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to run tmux display-message: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux display-message failed: {}",
            stderr.trim()
        ));
    }

    let pane_selector = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if pane_selector.is_empty() {
        return Err(anyhow::anyhow!("tmux returned empty active pane selector"));
    }

    Ok(format!("tmux:{tmux_name}:{pane_selector}"))
}

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
        sprite_pack: Option<SpritePack>,
        repo_theme: Option<RepoTheme>,
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

    /// Serializes tmux discovery so concurrent callers cannot race and attach
    /// duplicate actors to the same tmux session.
    discovery_lock: Mutex<()>,
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
            discovery_lock: Mutex::new(()),
        })
    }

    async fn resolve_repo_assets_for_summary(
        &self,
        summary: &mut SessionSummary,
    ) -> (Option<SpritePack>, Option<RepoTheme>) {
        if summary.cwd.is_empty() {
            summary.sprite_pack_id = None;
            summary.repo_theme_id = None;
            return (None, None);
        }

        let repo_theme = discover_repo_theme(&summary.cwd).map(|(theme_id, theme)| {
            summary.repo_theme_id = Some(theme_id);
            theme
        });
        if repo_theme.is_none() {
            summary.repo_theme_id = None;
        }

        let sprite_pack = discover_sprite_pack(&summary.cwd)
            .await
            .map(|(pack_id, pack)| {
                summary.sprite_pack_id = Some(pack_id);
                pack
            });
        if sprite_pack.is_none() {
            summary.sprite_pack_id = None;
        }

        (sprite_pack, repo_theme)
    }

    async fn collect_repo_assets(
        &self,
        summaries: &[SessionSummary],
    ) -> (HashMap<String, SpritePack>, HashMap<String, RepoTheme>) {
        let mut sprite_packs = HashMap::new();
        let mut repo_themes = HashMap::new();

        for summary in summaries {
            if let Some(ref theme_id) = summary.repo_theme_id {
                if !repo_themes.contains_key(theme_id) && !summary.cwd.is_empty() {
                    if let Some((_root, theme)) = discover_repo_theme(&summary.cwd) {
                        repo_themes.insert(theme_id.clone(), theme);
                    }
                }
            }

            if let Some(ref pack_id) = summary.sprite_pack_id {
                if !sprite_packs.contains_key(pack_id) && !summary.cwd.is_empty() {
                    if let Some((_root, pack)) = discover_sprite_pack(&summary.cwd).await {
                        sprite_packs.insert(pack_id.clone(), pack);
                    }
                }
            }
        }

        (sprite_packs, repo_themes)
    }

    /// Initialize persistence store and load persisted sessions as stale entries.
    pub async fn init_persistence(self: &Arc<Self>, store: Arc<FileStore>) {
        let persisted = store.load_sessions().await;
        let thoughts = store.load_thoughts().await;

        // Keep the ID counter ahead of any IDs we have ever persisted.
        for ps in &persisted {
            self.bump_id_counter_from_session_id(&ps.session_id);
        }
        // Thoughts can outlive the session registry. If we don't also advance from
        // thought snapshot keys, a fresh boot with an empty registry can reuse
        // an old `sess_N` and immediately inherit stale thought text.
        for session_id in thoughts.keys() {
            self.bump_id_counter_from_session_id(session_id);
        }

        if !persisted.is_empty() {
            let mut stale = Vec::new();
            for ps in &persisted {
                let thought_data = thoughts.get(&ps.session_id);
                let mut summary = SessionSummary {
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
                    rest_state: thought_data.map(|t| t.rest_state).unwrap_or_else(|| {
                        fallback_rest_state(SessionState::Exited, ps.thought_state)
                    }),
                    last_skill: ps.last_skill.clone(),
                    is_stale: true,
                    attached_clients: 0,
                    transport_health: crate::types::TransportHealth::Disconnected,
                    last_activity_at: ps.last_activity_at,
                    sprite_pack_id: None,
                    repo_theme_id: None,
                };
                self.resolve_repo_assets_for_summary(&mut summary).await;
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
        self.discover_tmux_sessions_with_reason("startup_discovery")
            .await
    }

    /// Discover existing tmux sessions, attaching only sessions not already
    /// tracked by an in-memory actor, and emit Created events with `reason`.
    pub async fn discover_tmux_sessions_with_reason(
        self: &Arc<Self>,
        reason: &'static str,
    ) -> anyhow::Result<()> {
        let _discovery_guard = self.discovery_lock.lock().await;
        let list_started = Instant::now();
        info!(
            reason,
            phase = "tmux_list_sessions",
            "running tmux list-sessions"
        );
        let output = Command::new("tmux")
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
            .await;

        let mut discovery_reliable = true;
        let mut highest_numeric: u64 = 0;
        let mut listed_tmux_names: Vec<String> = Vec::new();
        match output {
            Ok(output) if output.status.success() => {
                let elapsed = list_started.elapsed();
                let elapsed_ms = elapsed.as_millis() as u64;
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let tmux_name = line.trim().to_string();
                    if tmux_name.is_empty() {
                        continue;
                    }
                    listed_tmux_names.push(tmux_name);
                }
                if elapsed >= Duration::from_secs(2) {
                    warn!(
                        reason,
                        phase = "tmux_list_sessions",
                        elapsed_ms,
                        listed_sessions = listed_tmux_names.len(),
                        "tmux list-sessions completed slowly"
                    );
                } else {
                    info!(
                        reason,
                        phase = "tmux_list_sessions",
                        elapsed_ms,
                        listed_sessions = listed_tmux_names.len(),
                        "tmux list-sessions completed"
                    );
                }

                let tracked_tmux_names: HashSet<String> = {
                    let sessions = self.sessions.read().await;
                    sessions.values().map(|h| h.tmux_name.clone()).collect()
                };
                let stale_session_ids_by_tmux: HashMap<String, String> = {
                    let stale = self.stale_sessions.read().await;
                    let mut by_tmux = HashMap::new();
                    for s in stale.iter() {
                        by_tmux
                            .entry(s.tmux_name.clone())
                            .or_insert_with(|| s.session_id.clone());
                    }
                    by_tmux
                };

                let (candidates, planned_highest_numeric) = plan_tmux_discovery_candidates(
                    &listed_tmux_names,
                    &tracked_tmux_names,
                    &stale_session_ids_by_tmux,
                );
                highest_numeric = planned_highest_numeric;

                for candidate in candidates {
                    let tmux_name = candidate.tmux_name;
                    let session_id = match candidate.reuse_session_id {
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
                        None, // tool detected from process tree
                        self.config.clone(),
                    ) {
                        Ok(handle) => {
                            let mut sessions = self.sessions.write().await;
                            if sessions.values().any(|h| h.tmux_name == tmux_name) {
                                debug!(
                                    tmux_name = %tmux_name,
                                    "skipping duplicate discovered tmux session"
                                );
                                drop(sessions);
                                let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
                                continue;
                            }
                            sessions.insert(session_id.clone(), handle);

                            // Broadcast lifecycle event.
                            let summary = self.build_placeholder_summary(&session_id, &tmux_name);
                            let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
                                session_id,
                                summary,
                                reason: reason.into(),
                                sprite_pack: None,
                                repo_theme: None,
                            });
                        }
                        Err(e) => {
                            error!(tmux_name = %tmux_name, "failed to attach to tmux session: {}", e);
                        }
                    }
                }
            }
            Ok(output) => {
                let elapsed_ms = list_started.elapsed().as_millis() as u64;
                let stderr = String::from_utf8_lossy(&output.stderr);
                // "no server running" / "no sessions" means tmux has no live sessions.
                if stderr.contains("no server running") || stderr.contains("no sessions") {
                    info!(
                        reason,
                        phase = "tmux_list_sessions",
                        elapsed_ms,
                        "no existing tmux sessions found"
                    );
                } else {
                    discovery_reliable = false;
                    warn!(
                        reason,
                        phase = "tmux_list_sessions",
                        elapsed_ms,
                        "tmux list-sessions returned error: {}",
                        stderr
                    );
                }
            }
            Err(e) => {
                let elapsed_ms = list_started.elapsed().as_millis() as u64;
                discovery_reliable = false;
                warn!(
                    reason,
                    phase = "tmux_list_sessions",
                    elapsed_ms,
                    "tmux list-sessions failed: {}",
                    e
                );
            }
        }

        // Remove stale sessions that were upgraded to live.
        // For sessions still missing from tmux, emit startup exit/deletion events
        // and then drop them so bootstrap does not keep phantom entries forever.
        if discovery_reliable {
            let discovered_tmux_names: HashSet<String> = listed_tmux_names.into_iter().collect();
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
        initial_request: Option<String>,
    ) -> anyhow::Result<(SessionSummary, Option<SpritePack>, Option<RepoTheme>)> {
        let start_cwd = cwd.or_else(current_working_dir);
        let mut initial_request = normalize_initial_request(initial_request);
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

        let initial_tool = spawn_tool.as_ref().map(|t| {
            crate::types::detect_tool_name(t.command())
                .unwrap_or(t.command())
                .to_string()
        });

        let handle = crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            false, // create new
            start_cwd.clone(),
            initial_tool.clone(),
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
        if let Some(ref display) = initial_tool {
            summary.tool = Some(display.clone());
            summary.context_limit = crate::types::context_limit_for_tool(Some(display));
        }

        let (sprite_pack, repo_theme) = self.resolve_repo_assets_for_summary(&mut summary).await;

        let initial_request_delay = if spawn_tool.is_some() && initial_request.is_some() {
            INITIAL_REQUEST_INPUT_DELAY
        } else {
            Duration::ZERO
        };
        if let Some(tool) = spawn_tool {
            let spawn_command = build_spawn_tool_command(tool, initial_request.as_deref());
            if spawn_tool_consumes_initial_request(tool) {
                initial_request = None;
            }

            if let Err(e) = send_spawn_tool_command(&tmux_name, tool, &spawn_command).await {
                warn!(
                    session_id = %session_id,
                    tmux_name = %tmux_name,
                    tool = ?tool,
                    "tmux send-keys failed, falling back to PTY input: {}",
                    e
                );
                let mut command = spawn_command;
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
        if let Some(initial_request) = initial_request {
            enqueue_initial_request_input(
                bootstrap_handle,
                session_id.clone(),
                tmux_name.clone(),
                initial_request,
                initial_request_delay,
            );
        }

        // Broadcast lifecycle event.
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id: session_id.clone(),
            summary: summary.clone(),
            reason: "api_create".into(),
            sprite_pack: sprite_pack.clone(),
            repo_theme: repo_theme.clone(),
        });

        // Persist the updated registry.
        self.persist_registry().await;

        Ok((summary, sprite_pack, repo_theme))
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

        let thought_snapshots = self.thought_snapshots.read().await.clone();
        for summary in &mut summaries {
            let active_pane_session_id = if thought_snapshots.contains_key(&summary.session_id)
                || summary.tmux_name.is_empty()
            {
                None
            } else {
                query_tmux_active_pane_session_id(&summary.tmux_name)
                    .await
                    .ok()
            };
            if let Some(thought_data) = thought_snapshot_for_summary(
                summary,
                active_pane_session_id.as_deref(),
                &thought_snapshots,
            ) {
                if summary.thought.is_none() {
                    summary.thought = thought_data.thought.clone();
                }
                summary.thought_state = thought_data.thought_state;
                summary.thought_source = thought_data.thought_source;
                summary.thought_updated_at = Some(thought_data.updated_at);
                summary.rest_state = thought_data.rest_state;
                if thought_data.token_count > 0 || summary.token_count == 0 {
                    summary.token_count = thought_data.token_count;
                }
                if thought_data.context_limit > 0 {
                    summary.context_limit = thought_data.context_limit;
                }
            }
        }

        // Resolve per-repo sprite pack IDs by walking up from each session's
        // cwd.  We do this after the thought merge so that the cwd is the
        // actor-reported value.
        for summary in &mut summaries {
            self.resolve_repo_assets_for_summary(summary).await;
        }

        summaries
    }

    pub async fn list_session_data(&self) -> BootstrapData {
        let sessions = self.list_sessions().await;
        let (sprite_packs, repo_themes) = self.collect_repo_assets(&sessions).await;
        BootstrapData {
            sessions,
            sprite_packs,
            repo_themes,
        }
    }

    /// Return all sessions for the bootstrap response, including stale
    /// (exited) sessions from persistence, plus a deduplicated map of
    /// per-repository sprite packs.
    pub async fn bootstrap(&self) -> BootstrapData {
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

        // Build the sprite_packs map: for each unique sprite_pack_id present
        // across all sessions, look up the SpritePack from the discovery cache.
        let (sprite_packs, repo_themes) = self.collect_repo_assets(&all).await;

        BootstrapData {
            sessions: all,
            sprite_packs,
            repo_themes,
        }
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
                let active_pane_session_id = if thought_snapshots.contains_key(&summary.session_id)
                    || summary.tmux_name.is_empty()
                    || summary.state == crate::types::SessionState::Exited
                {
                    None
                } else {
                    query_tmux_active_pane_session_id(&summary.tmux_name)
                        .await
                        .ok()
                };
                let thought_data = thought_snapshot_for_summary(
                    &summary,
                    active_pane_session_id.as_deref(),
                    &thought_snapshots,
                );

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
                    rest_state: thought_data
                        .map(|t| t.rest_state)
                        .unwrap_or(summary.rest_state),
                    thought: thought_data
                        .and_then(|t| t.thought.clone())
                        .or_else(|| summary.thought.clone()),
                    thought_updated_at: thought_data
                        .map(|t| t.updated_at)
                        .or(summary.thought_updated_at),
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
                rest_state: s.rest_state,
                last_skill: s.last_skill.clone(),
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
        thought: Option<&str>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        rest_state: RestState,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_fingerprint: Option<String>,
    ) {
        {
            let mut thought_snapshots = self.thought_snapshots.write().await;
            thought_snapshots.insert(
                session_id.to_string(),
                ThoughtSnapshot {
                    thought: thought.map(|value| value.to_string()),
                    thought_state,
                    thought_source,
                    rest_state,
                    objective_fingerprint: objective_fingerprint.clone(),
                    token_count,
                    context_limit,
                    updated_at,
                    delivery: delivery.clone(),
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
                rest_state,
                updated_at,
                delivery,
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

    /// Spawn a background task that reaps exited sessions once actors report
    /// them as exited.
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
            rest_state: fallback_rest_state(SessionState::Idle, ThoughtState::Holding),
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: crate::types::TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            sprite_pack_id: None,
            repo_theme_id: None,
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
    thought: Option<String>,
    token_count: u64,
    context_limit: u64,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    rest_state: RestState,
    updated_at: DateTime<Utc>,
    delivery: ThoughtDeliveryState,
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
                        req.thought.as_deref(),
                        req.token_count,
                        req.context_limit,
                        req.thought_state,
                        req.thought_source,
                        req.rest_state,
                        req.updated_at,
                        req.delivery,
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
        thought: Option<&str>,
        token_count: u64,
        context_limit: u64,
        thought_state: ThoughtState,
        thought_source: ThoughtSource,
        rest_state: RestState,
        updated_at: DateTime<Utc>,
        delivery: ThoughtDeliveryState,
        objective_fingerprint: Option<String>,
    ) {
        if self
            .persist_tx
            .try_send(PersistThoughtRequest {
                session_id: session_id.to_string(),
                thought: thought.map(|value| value.to_string()),
                token_count,
                context_limit,
                thought_state,
                thought_source,
                rest_state,
                updated_at,
                delivery,
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

    fn thought_delivery_states(&self) -> HashMap<String, ThoughtDeliveryState> {
        let supervisor = self.supervisor.clone();
        let handle = self.handle.clone();
        std::thread::scope(|s| {
            s.spawn(|| {
                handle.block_on(async {
                    supervisor
                        .thought_snapshots
                        .read()
                        .await
                        .iter()
                        .map(|(session_id, snapshot)| {
                            (session_id.clone(), snapshot.delivery.clone())
                        })
                        .collect()
                })
            })
            .join()
            .expect("thought_delivery_states thread panicked")
        })
    }
}

fn current_working_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

const INITIAL_REQUEST_INPUT_DELAY: Duration = Duration::from_millis(200);

fn normalize_initial_request(initial_request: Option<String>) -> Option<String> {
    initial_request.and_then(|request| {
        let trimmed = request.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn build_initial_request_input(initial_request: &str) -> Vec<u8> {
    let mut input = initial_request.as_bytes().to_vec();
    input.push(b'\r');
    input
}

fn spawn_tool_consumes_initial_request(tool: crate::types::SpawnTool) -> bool {
    matches!(tool, crate::types::SpawnTool::Codex)
}

fn build_spawn_tool_command(
    tool: crate::types::SpawnTool,
    initial_request: Option<&str>,
) -> String {
    if spawn_tool_consumes_initial_request(tool) {
        if let Some(initial_request) = initial_request {
            return format!("{} {}", tool.command(), shell_single_quote(initial_request));
        }
    }

    tool.command().to_string()
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn enqueue_initial_request_input(
    handle: ActorHandle,
    session_id: String,
    tmux_name: String,
    initial_request: String,
    delay: Duration,
) {
    tokio::spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        if let Err(e) = handle
            .send(SessionCommand::WriteInput(build_initial_request_input(
                &initial_request,
            )))
            .await
        {
            warn!(
                session_id = %session_id,
                tmux_name = %tmux_name,
                "failed to enqueue initial request input: {}",
                e
            );
        }
    });
}

async fn send_spawn_tool_command(
    tmux_name: &str,
    tool: crate::types::SpawnTool,
    command: &str,
) -> anyhow::Result<()> {
    const ATTEMPTS: usize = 8;
    const RETRY_DELAY_MS: u64 = 75;

    for attempt in 1..=ATTEMPTS {
        match tmux_send_keys(tmux_name, &["-l", "--", command]).await {
            Ok(()) => match tmux_send_keys(tmux_name, &["Enter"]).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    debug!(
                        tmux_name = %tmux_name,
                        tool = ?tool,
                        command,
                        attempt,
                        "failed to execute tmux Enter send-keys: {}",
                        e
                    );
                }
            },
            Err(e) => {
                debug!(
                    tmux_name = %tmux_name,
                    tool = ?tool,
                    command,
                    attempt,
                    "failed to execute tmux literal send-keys: {}",
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

async fn tmux_send_keys(tmux_name: &str, key_args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("tmux")
        .args(["send-keys", "-t", tmux_name])
        .args(key_args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("failed to execute tmux send-keys: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow::anyhow!(
        "tmux send-keys failed (status {:?}): {}",
        output.status,
        stderr.trim()
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveryCandidate {
    tmux_name: String,
    reuse_session_id: Option<String>,
}

fn plan_tmux_discovery_candidates(
    listed_tmux_names: &[String],
    tracked_tmux_names: &HashSet<String>,
    stale_session_ids_by_tmux: &HashMap<String, String>,
) -> (Vec<DiscoveryCandidate>, u64) {
    let mut seen_tmux_names = HashSet::new();
    let mut highest_numeric = 0_u64;
    let mut candidates = Vec::new();

    for tmux_name in listed_tmux_names {
        if tmux_name.is_empty() {
            continue;
        }

        if let Ok(n) = tmux_name.parse::<u64>() {
            highest_numeric = highest_numeric.max(n.saturating_add(1));
        }

        if !seen_tmux_names.insert(tmux_name.clone()) {
            continue;
        }

        if tracked_tmux_names.contains(tmux_name) {
            continue;
        }

        candidates.push(DiscoveryCandidate {
            tmux_name: tmux_name.clone(),
            reuse_session_id: stale_session_ids_by_tmux.get(tmux_name).cloned(),
        });
    }

    (candidates, highest_numeric)
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
    use chrono::{DateTime, Utc};
    use std::iter::FromIterator;
    use tempfile::tempdir;

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
    fn ready_process_exit_ids_reaps_immediately_when_grace_is_zero() {
        let mut seen = HashMap::new();
        let exited = HashSet::from_iter(["sess_1".to_string()]);
        let start = Instant::now();

        let ready = ready_process_exit_ids(&mut seen, &exited, start, Duration::ZERO);
        assert_eq!(ready, vec!["sess_1".to_string()]);
    }

    #[test]
    fn spawn_tool_roundtrip_sets_correct_display_name() {
        use crate::types::{context_limit_for_tool, detect_tool_name, SpawnTool};

        for (tool, expected_name, expected_limit) in [
            (SpawnTool::Claude, "Claude Code", 200_000),
            (SpawnTool::Codex, "Codex", 192_000),
        ] {
            let display = detect_tool_name(tool.command()).unwrap_or(tool.command());
            assert_eq!(display, expected_name);
            assert_eq!(context_limit_for_tool(Some(display)), expected_limit);
        }
    }

    #[test]
    fn normalize_initial_request_trims_blank_values() {
        assert_eq!(normalize_initial_request(None), None);
        assert_eq!(normalize_initial_request(Some("   ".to_string())), None);
        assert_eq!(
            normalize_initial_request(Some("  investigate tmux  ".to_string())),
            Some("investigate tmux".to_string())
        );
    }

    #[test]
    fn build_initial_request_input_appends_carriage_return() {
        assert_eq!(
            build_initial_request_input("hello codex"),
            b"hello codex\r".to_vec()
        );
    }

    #[test]
    fn build_spawn_tool_command_inlines_codex_initial_request() {
        assert_eq!(
            build_spawn_tool_command(
                crate::types::SpawnTool::Codex,
                Some("investigate tmux startup")
            ),
            "codex 'investigate tmux startup'"
        );
    }

    #[test]
    fn build_spawn_tool_command_escapes_single_quotes_for_shell() {
        assert_eq!(
            build_spawn_tool_command(
                crate::types::SpawnTool::Codex,
                Some("fix Bob's tmux startup")
            ),
            "codex 'fix Bob'\\''s tmux startup'"
        );
    }

    #[test]
    fn build_spawn_tool_command_keeps_other_tools_on_follow_up_input_path() {
        assert_eq!(
            build_spawn_tool_command(
                crate::types::SpawnTool::Claude,
                Some("investigate tmux startup")
            ),
            "claude"
        );
        assert!(!spawn_tool_consumes_initial_request(
            crate::types::SpawnTool::Claude
        ));
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
                Utc::now(),
                ThoughtDeliveryState::default(),
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
                state: SessionState::Idle,
                tool: Some("Codex".to_string()),
                token_count: 0,
                context_limit: 192_000,
                thought: None,
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::CarryForward,
                thought_updated_at: None,
                rest_state: RestState::Drowsy,
                last_skill: None,
                objective_fingerprint: None,
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
                updated_at,
                ThoughtDeliveryState::default(),
                Some("obj-1".to_string()),
            )
            .await;

        let thoughts = supervisor.thought_snapshots.read().await;
        let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
        assert_eq!(snapshot.updated_at, updated_at);
        assert_eq!(snapshot.thought.as_deref(), Some("reading logs"));
    }

    #[test]
    fn thought_snapshot_for_summary_matches_active_tmux_pane() {
        let summary = SessionSummary {
            session_id: "sess_1".to_string(),
            tmux_name: "work".to_string(),
            state: SessionState::Idle,
            current_command: None,
            cwd: "/tmp".to_string(),
            tool: None,
            token_count: 0,
            context_limit: 0,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: crate::types::TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            sprite_pack_id: None,
            repo_theme_id: None,
        };

        let older = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let newer = DateTime::parse_from_rfc3339("2026-03-08T14:00:06Z")
            .expect("timestamp")
            .with_timezone(&Utc);

        let snapshots = HashMap::from([
            (
                "tmux:work:1.0:%1".to_string(),
                ThoughtSnapshot {
                    thought: Some("pane one".to_string()),
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    objective_fingerprint: None,
                    token_count: 10,
                    context_limit: 100,
                    updated_at: older,
                    delivery: ThoughtDeliveryState {
                        stream_instance_id: Some("stream-a".to_string()),
                        emission_seq: 1,
                    },
                },
            ),
            (
                "tmux:work:1.1:%2".to_string(),
                ThoughtSnapshot {
                    thought: Some("pane two".to_string()),
                    thought_state: ThoughtState::Active,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Active,
                    objective_fingerprint: None,
                    token_count: 10,
                    context_limit: 100,
                    updated_at: newer,
                    delivery: ThoughtDeliveryState {
                        stream_instance_id: Some("stream-a".to_string()),
                        emission_seq: 2,
                    },
                },
            ),
        ]);

        let matched = thought_snapshot_for_summary(&summary, Some("tmux:work:1.1:%2"), &snapshots)
            .expect("tmux pane snapshot");
        assert_eq!(matched.thought.as_deref(), Some("pane two"));
        assert_eq!(matched.delivery.emission_seq, 2);
    }

    #[test]
    fn thought_snapshot_for_summary_does_not_fall_back_to_latest_tmux_pane_without_active_binding()
    {
        let summary = SessionSummary {
            session_id: "sess_1".to_string(),
            tmux_name: "work".to_string(),
            state: SessionState::Idle,
            current_command: None,
            cwd: "/tmp".to_string(),
            tool: None,
            token_count: 0,
            context_limit: 0,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: crate::types::TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            sprite_pack_id: None,
            repo_theme_id: None,
        };

        let snapshots = HashMap::from([
            (
                "tmux:work:1.0:%1".to_string(),
                ThoughtSnapshot {
                    thought: Some("pane one".to_string()),
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    objective_fingerprint: None,
                    token_count: 10,
                    context_limit: 100,
                    updated_at: Utc::now(),
                    delivery: ThoughtDeliveryState::default(),
                },
            ),
            (
                "tmux:work:1.1:%2".to_string(),
                ThoughtSnapshot {
                    thought: Some("pane two".to_string()),
                    thought_state: ThoughtState::Active,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Active,
                    objective_fingerprint: None,
                    token_count: 10,
                    context_limit: 100,
                    updated_at: Utc::now(),
                    delivery: ThoughtDeliveryState::default(),
                },
            ),
        ]);

        assert!(thought_snapshot_for_summary(&summary, None, &snapshots).is_none());
    }

    #[test]
    fn plan_tmux_discovery_skips_tracked_and_dedupes_names() {
        let listed = vec![
            "main".to_string(),
            "main".to_string(),
            "codex-123".to_string(),
        ];
        let tracked = HashSet::from_iter(["main".to_string()]);
        let stale_by_tmux = HashMap::new();

        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &tracked, &stale_by_tmux);

        assert_eq!(highest_numeric, 0);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].tmux_name, "codex-123");
        assert_eq!(candidates[0].reuse_session_id, None);
    }

    #[test]
    fn plan_tmux_discovery_reuses_stale_id_and_bumps_numeric_counter() {
        let listed = vec![
            "7".to_string(),
            "7".to_string(),
            "codex-20260302-162713".to_string(),
        ];
        let tracked = HashSet::new();
        let stale_by_tmux =
            HashMap::from_iter([("codex-20260302-162713".to_string(), "sess_12".to_string())]);

        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &tracked, &stale_by_tmux);

        assert_eq!(highest_numeric, 8);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].tmux_name, "7");
        assert_eq!(candidates[0].reuse_session_id, None);
        assert_eq!(candidates[1].tmux_name, "codex-20260302-162713");
        assert_eq!(candidates[1].reuse_session_id.as_deref(), Some("sess_12"));
    }
}
