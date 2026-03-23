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

struct ListedTmuxSessions {
    reliable: bool,
    names: Vec<String>,
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

    async fn list_tmux_session_names(&self, reason: &'static str) -> ListedTmuxSessions {
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

        match output {
            Ok(output) if output.status.success() => {
                let names = parse_tmux_session_names(&output.stdout);
                log_tmux_list_success(reason, list_started.elapsed(), names.len());
                ListedTmuxSessions {
                    reliable: true,
                    names,
                }
            }
            Ok(output) => {
                let elapsed = list_started.elapsed();
                let stderr = String::from_utf8_lossy(&output.stderr);
                if tmux_list_reports_no_sessions(&stderr) {
                    info!(
                        reason,
                        phase = "tmux_list_sessions",
                        elapsed_ms = elapsed.as_millis() as u64,
                        "no existing tmux sessions found"
                    );
                    ListedTmuxSessions {
                        reliable: true,
                        names: Vec::new(),
                    }
                } else {
                    warn!(
                        reason,
                        phase = "tmux_list_sessions",
                        elapsed_ms = elapsed.as_millis() as u64,
                        "tmux list-sessions returned error: {}",
                        stderr
                    );
                    ListedTmuxSessions {
                        reliable: false,
                        names: Vec::new(),
                    }
                }
            }
            Err(error) => {
                let elapsed = list_started.elapsed();
                warn!(
                    reason,
                    phase = "tmux_list_sessions",
                    elapsed_ms = elapsed.as_millis() as u64,
                    "tmux list-sessions failed: {}",
                    error
                );
                ListedTmuxSessions {
                    reliable: false,
                    names: Vec::new(),
                }
            }
        }
    }

    async fn tracked_tmux_names(&self) -> HashSet<String> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .map(|handle| handle.tmux_name.clone())
            .collect()
    }

    async fn stale_session_ids_by_tmux(&self) -> HashMap<String, String> {
        let stale = self.stale_sessions.read().await;
        let mut by_tmux = HashMap::new();
        for summary in stale.iter() {
            by_tmux
                .entry(summary.tmux_name.clone())
                .or_insert_with(|| summary.session_id.clone());
        }
        by_tmux
    }

    async fn attach_discovered_sessions(
        self: &Arc<Self>,
        reason: &'static str,
        listed_tmux_names: &[String],
    ) -> u64 {
        let tracked_tmux_names = self.tracked_tmux_names().await;
        let stale_session_ids_by_tmux = self.stale_session_ids_by_tmux().await;
        let (candidates, highest_numeric) = plan_tmux_discovery_candidates(
            listed_tmux_names,
            &tracked_tmux_names,
            &stale_session_ids_by_tmux,
        );

        for candidate in candidates {
            self.attach_discovery_candidate(candidate, reason).await;
        }

        highest_numeric
    }

    async fn attach_discovery_candidate(
        self: &Arc<Self>,
        candidate: DiscoveryCandidate,
        reason: &'static str,
    ) {
        let tmux_name = candidate.tmux_name;
        let session_id = match candidate.reuse_session_id {
            Some(id) => {
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
            true,
            None,
            None,
            self.config.clone(),
        ) {
            Ok(handle) => {
                if !self
                    .insert_discovered_handle(session_id.clone(), tmux_name.clone(), handle)
                    .await
                {
                    return;
                }
                self.emit_discovered_created_event(session_id, tmux_name, reason);
            }
            Err(error) => {
                error!(tmux_name = %tmux_name, "failed to attach to tmux session: {}", error);
            }
        }
    }

    async fn insert_discovered_handle(
        &self,
        session_id: String,
        tmux_name: String,
        handle: ActorHandle,
    ) -> bool {
        let mut sessions = self.sessions.write().await;
        if sessions
            .values()
            .any(|existing| existing.tmux_name == tmux_name)
        {
            debug!(
                tmux_name = %tmux_name,
                "skipping duplicate discovered tmux session"
            );
            drop(sessions);
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
            return false;
        }
        sessions.insert(session_id, handle);
        true
    }

    fn emit_discovered_created_event(
        &self,
        session_id: String,
        tmux_name: String,
        reason: &'static str,
    ) {
        let summary = self.build_placeholder_summary(&session_id, &tmux_name);
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id,
            summary,
            reason: reason.into(),
            sprite_pack: None,
            repo_theme: None,
        });
    }

    async fn reconcile_stale_sessions_after_discovery(
        &self,
        discovery_reliable: bool,
        listed_tmux_names: Vec<String>,
    ) {
        if !discovery_reliable {
            warn!("skipping stale reconciliation due unreliable tmux discovery");
            return;
        }

        let discovered_tmux_names: HashSet<String> = listed_tmux_names.into_iter().collect();
        let unresolved_stale = {
            let mut stale = self.stale_sessions.write().await;
            stale.retain(|summary| !discovered_tmux_names.contains(&summary.tmux_name));
            let unresolved = stale.clone();
            stale.clear();
            unresolved
        };

        if !unresolved_stale.is_empty() {
            debug!(
                remaining_stale = unresolved_stale.len(),
                "stale sessions after discovery"
            );
        }

        for summary in unresolved_stale {
            self.emit_startup_missing_tmux_events(summary);
        }
    }

    fn emit_startup_missing_tmux_events(&self, summary: SessionSummary) {
        let payload = SessionStatePayload {
            state: SessionState::Exited,
            previous_state: summary.state,
            current_command: summary.current_command.clone(),
            transport_health: TransportHealth::Disconnected,
            exit_reason: Some("startup_missing_tmux".to_string()),
            at: Utc::now(),
        };
        let event = ControlEvent {
            event: "session_state".to_string(),
            session_id: summary.session_id.clone(),
            payload: serde_json::to_value(&payload).unwrap_or_default(),
        };
        let _ = self.thought_tx.send(event);

        let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
            session_id: summary.session_id,
            reason: "startup_missing_tmux".to_string(),
            delete_mode: crate::config::SessionDeleteMode::DetachBridge,
            tmux_session_alive: false,
        });
    }

    async fn finish_tmux_discovery(&self, discovery_reliable: bool, highest_numeric: u64) {
        self.next_name_counter
            .fetch_max(highest_numeric, Ordering::SeqCst);

        let sessions = self.sessions.read().await;
        crate::metrics::set_active_sessions(sessions.len());
        info!(count = sessions.len(), "tmux session discovery complete");

        if discovery_reliable {
            self.persist_registry().await;
        }
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
        let listed = self.list_tmux_session_names(reason).await;
        let highest_numeric = self.attach_discovered_sessions(reason, &listed.names).await;
        self.reconcile_stale_sessions_after_discovery(listed.reliable, listed.names)
            .await;
        self.finish_tmux_discovery(listed.reliable, highest_numeric)
            .await;

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
        let tmux_name = self.allocate_tmux_name(name);
        let session_id = self.allocate_unique_session_id().await;

        info!(session_id = %session_id, tmux_name = %tmux_name, "creating new session");

        let initial_tool = initial_tool_name(spawn_tool.as_ref());
        let handle = crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            false, // create new
            start_cwd.clone(),
            initial_tool.clone(),
            self.config.clone(),
        )?;
        let bootstrap_handle = handle.clone();

        self.insert_active_handle(session_id.clone(), handle).await;
        let mut summary = self
            .build_created_summary(
                &session_id,
                &tmux_name,
                start_cwd.as_deref(),
                initial_tool.as_deref(),
            )
            .await;
        let (sprite_pack, repo_theme) = self.resolve_repo_assets_for_summary(&mut summary).await;
        let initial_request_delay = initial_request_delay(spawn_tool, initial_request.as_ref());
        self.maybe_spawn_initial_tool(
            &session_id,
            &tmux_name,
            &bootstrap_handle,
            spawn_tool,
            &mut initial_request,
        )
        .await;
        self.enqueue_initial_request_if_present(
            bootstrap_handle,
            &session_id,
            &tmux_name,
            initial_request,
            initial_request_delay,
        );
        self.emit_created_session(
            &session_id,
            &summary,
            sprite_pack.clone(),
            repo_theme.clone(),
        );
        self.persist_registry().await;

        Ok((summary, sprite_pack, repo_theme))
    }

    fn allocate_tmux_name(&self, requested_name: Option<String>) -> String {
        normalize_requested_tmux_name(requested_name).unwrap_or_else(|| {
            let n = self.next_name_counter.fetch_add(1, Ordering::SeqCst);
            n.to_string()
        })
    }

    async fn insert_active_handle(&self, session_id: String, handle: ActorHandle) {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id, handle);
        crate::metrics::set_active_sessions(sessions.len());
    }

    async fn build_created_summary(
        &self,
        session_id: &str,
        tmux_name: &str,
        start_cwd: Option<&str>,
        initial_tool: Option<&str>,
    ) -> SessionSummary {
        let mut summary = self.build_placeholder_summary(session_id, tmux_name);
        if let Some(cwd) = start_cwd {
            summary.cwd = cwd.to_string();
        }
        if let Some(display) = initial_tool {
            summary.tool = Some(display.to_string());
            summary.context_limit = crate::types::context_limit_for_tool(Some(display));
        }
        summary
    }

    async fn maybe_spawn_initial_tool(
        &self,
        session_id: &str,
        tmux_name: &str,
        bootstrap_handle: &ActorHandle,
        spawn_tool: Option<crate::types::SpawnTool>,
        initial_request: &mut Option<String>,
    ) {
        let Some(tool) = spawn_tool else {
            return;
        };

        let spawn_command = build_spawn_tool_command(tool, initial_request.as_deref());
        if spawn_tool_consumes_initial_request(tool) {
            *initial_request = None;
        }

        if let Err(e) = send_spawn_tool_command(tmux_name, tool, &spawn_command).await {
            warn!(
                session_id = %session_id,
                tmux_name = %tmux_name,
                tool = ?tool,
                "tmux send-keys failed, falling back to PTY input: {}",
                e
            );
            self.enqueue_spawn_command_fallback(
                session_id,
                tmux_name,
                tool,
                bootstrap_handle,
                spawn_command,
            )
            .await;
        }
    }

    async fn enqueue_spawn_command_fallback(
        &self,
        session_id: &str,
        tmux_name: &str,
        tool: crate::types::SpawnTool,
        bootstrap_handle: &ActorHandle,
        mut spawn_command: String,
    ) {
        spawn_command.push('\n');
        if let Err(e) = bootstrap_handle
            .send(SessionCommand::WriteInput(spawn_command.into_bytes()))
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

    fn enqueue_initial_request_if_present(
        &self,
        bootstrap_handle: ActorHandle,
        session_id: &str,
        tmux_name: &str,
        initial_request: Option<String>,
        delay: Duration,
    ) {
        let Some(initial_request) = initial_request else {
            return;
        };
        enqueue_initial_request_input(
            bootstrap_handle,
            session_id.to_string(),
            tmux_name.to_string(),
            initial_request,
            delay,
        );
    }

    fn emit_created_session(
        &self,
        session_id: &str,
        summary: &SessionSummary,
        sprite_pack: Option<SpritePack>,
        repo_theme: Option<RepoTheme>,
    ) {
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id: session_id.to_string(),
            summary: summary.clone(),
            reason: "api_create".into(),
            sprite_pack,
            repo_theme,
        });
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

    #[cfg(test)]
    pub async fn insert_test_handle(&self, handle: ActorHandle) {
        let mut sessions = self.sessions.write().await;
        sessions.insert(handle.session_id.clone(), handle);
        crate::metrics::set_active_sessions(sessions.len());
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

fn normalize_requested_tmux_name(requested_name: Option<String>) -> Option<String> {
    requested_name.and_then(|name| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn initial_tool_name(spawn_tool: Option<&crate::types::SpawnTool>) -> Option<String> {
    spawn_tool.map(|tool| {
        crate::types::detect_tool_name(tool.command())
            .unwrap_or(tool.command())
            .to_string()
    })
}

fn initial_request_delay(
    spawn_tool: Option<crate::types::SpawnTool>,
    initial_request: Option<&String>,
) -> Duration {
    if spawn_tool.is_some() && initial_request.is_some() {
        INITIAL_REQUEST_INPUT_DELAY
    } else {
        Duration::ZERO
    }
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

fn parse_tmux_session_names(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn tmux_list_reports_no_sessions(stderr: &str) -> bool {
    stderr.contains("no server running") || stderr.contains("no sessions")
}

fn log_tmux_list_success(reason: &'static str, elapsed: Duration, listed_sessions: usize) {
    let elapsed_ms = elapsed.as_millis() as u64;
    if elapsed >= Duration::from_secs(2) {
        warn!(
            reason,
            phase = "tmux_list_sessions",
            elapsed_ms,
            listed_sessions,
            "tmux list-sessions completed slowly"
        );
    } else {
        info!(
            reason,
            phase = "tmux_list_sessions",
            elapsed_ms,
            listed_sessions,
            "tmux list-sessions completed"
        );
    }
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
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    fn test_summary(session_id: &str, state: SessionState) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: format!("tmux-{session_id}"),
            state,
            current_command: Some("cargo test".to_string()),
            cwd: "/tmp/project".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: fallback_rest_state(state, ThoughtState::Holding),
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            sprite_pack_id: None,
            repo_theme_id: None,
        }
    }

    async fn spawn_summary_handle(summary: SessionSummary) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(
            summary.session_id.clone(),
            summary.tmux_name.clone(),
            cmd_tx,
        );
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = reply.send(summary.clone());
                    }
                    SessionCommand::GetSnapshot(reply) => {
                        let _ = reply.send(TerminalSnapshot {
                            session_id: summary.session_id.clone(),
                            latest_seq: 17,
                            truncated: false,
                            screen_text: "0123456789 replay tail".to_string(),
                        });
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    async fn spawn_dropped_summary_handle(
        session_id: &str,
        tmux_name: &str,
        state: SessionState,
    ) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(session_id, tmux_name, cmd_tx);
        let summary = test_summary(session_id, state);
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(_reply) => {}
                    SessionCommand::GetSnapshot(reply) => {
                        let _ = reply.send(TerminalSnapshot {
                            session_id: summary.session_id.clone(),
                            latest_seq: 0,
                            truncated: false,
                            screen_text: String::new(),
                        });
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    async fn spawn_closed_summary_handle(session_id: &str, tmux_name: &str) -> ActorHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel(8);
        drop(cmd_rx);
        ActorHandle::test_handle(session_id, tmux_name, cmd_tx)
    }

    fn write_executable(path: &std::path::Path, contents: &str) {
        std::fs::write(path, contents).expect("write executable");
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    fn prepend_test_path(bin_dir: &std::path::Path, original_path: Option<&std::ffi::OsStr>) {
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = original_path {
            entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
        }
        std::env::set_var("PATH", std::env::join_paths(entries).expect("path"));
    }

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

    #[tokio::test]
    async fn list_sessions_merges_thought_snapshots_and_skips_exited_summaries() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
            )
            .await;
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
            )
            .await;

        supervisor.thought_snapshots.write().await.insert(
            "sess-live".to_string(),
            ThoughtSnapshot {
                thought: Some("checking logs".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                objective_fingerprint: None,
                token_count: 44,
                context_limit: 200_000,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        );

        let sessions = supervisor.list_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-live");
        assert_eq!(sessions[0].thought.as_deref(), Some("checking logs"));
        assert_eq!(sessions[0].thought_state, ThoughtState::Active);
        assert_eq!(sessions[0].token_count, 44);
    }

    #[tokio::test]
    async fn list_sessions_skips_dropped_summary_replies() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_dropped_summary_handle("sess-drop", "tmux-drop", SessionState::Idle).await,
            )
            .await;

        let sessions = supervisor.list_sessions().await;

        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn list_sessions_skips_closed_command_channels() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(spawn_closed_summary_handle("sess-closed", "").await)
            .await;

        let sessions = supervisor.list_sessions().await;

        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn collect_session_snapshots_uses_summary_snapshot_and_thought_cache() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-1", SessionState::Busy)).await,
            )
            .await;
        supervisor.thought_snapshots.write().await.insert(
            "sess-1".to_string(),
            ThoughtSnapshot {
                thought: Some("building release".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                objective_fingerprint: Some("obj-1".to_string()),
                token_count: 55,
                context_limit: 210_000,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        );

        let infos = supervisor.collect_session_snapshots().await;
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].session_id, "sess-1");
        assert!(infos[0].replay_text.ends_with("replay tail"));
        assert_eq!(infos[0].thought.as_deref(), Some("building release"));
        assert_eq!(infos[0].token_count, 55);
        assert_eq!(infos[0].objective_fingerprint.as_deref(), Some("obj-1"));
    }

    #[tokio::test]
    async fn collect_exited_session_ids_reports_only_exited_sessions() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-idle", SessionState::Idle)).await,
            )
            .await;
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
            )
            .await;

        let exited = supervisor
            .collect_exited_session_ids(Duration::from_millis(50))
            .await;
        assert_eq!(exited, HashSet::from_iter(["sess-exited".to_string()]));
    }

    #[tokio::test]
    async fn reap_exited_sessions_removes_ready_actor_handles() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
            )
            .await;

        supervisor.reap_exited_sessions().await;
        assert!(supervisor.get_session("sess-exited").await.is_none());
    }

    #[tokio::test]
    async fn collect_repo_assets_reads_repo_theme_and_sprite_pack() {
        let dir = tempdir().expect("tempdir");
        let repo_root = dir.path().join("repo");
        let sprites_dir = repo_root.join(".throngterm").join("sprites");
        std::fs::create_dir_all(&sprites_dir).expect("sprites dir");
        std::fs::write(
            repo_root.join(".throngterm").join("colors.json"),
            r##"{"palette":{"body":"#123456","outline":"#234567","accent":"#345678","shirt":"#456789"}}"##,
        )
        .expect("colors");
        for (name, value) in [
            ("active.svg", "<svg id='active'/>"),
            ("drowsy.svg", "<svg id='drowsy'/>"),
            ("sleeping.svg", "<svg id='sleeping'/>"),
            ("deep_sleep.svg", "<svg id='deep_sleep'/>"),
        ] {
            std::fs::write(sprites_dir.join(name), value).expect("sprite");
        }

        let project_id = repo_root.to_string_lossy().into_owned();
        let mut summary = test_summary("sess-assets", SessionState::Idle);
        summary.cwd = repo_root.to_string_lossy().into_owned();
        summary.repo_theme_id = Some(project_id.clone());
        summary.sprite_pack_id = Some(project_id.clone());

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let (sprite_packs, repo_themes) = supervisor.collect_repo_assets(&[summary]).await;
        assert!(sprite_packs.contains_key(&project_id));
        assert!(repo_themes.contains_key(&project_id));
    }

    #[tokio::test]
    async fn create_session_uses_fake_tmux_and_bootstraps_codex_spawn() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        write_executable(
            &bin_dir.join("tmux"),
            r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  new-session|attach-session)
    while IFS= read -r line; do
      printf '%s\r\n' "$line"
    done
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_path}") printf '%s\n' "${THRONGTERM_FAKE_TMUX_CWD:-/tmp/project}" ;;
      "#{pane_current_command}") printf '%s\n' "${THRONGTERM_FAKE_TMUX_COMMAND:-codex}" ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
  send-keys|kill-session)
    exit 0
    ;;
  capture-pane)
    printf 'captured pane\n'
    ;;
  list-sessions)
    if [ -f "${THRONGTERM_FAKE_TMUX_SESSIONS:-}" ]; then
      while IFS= read -r line || [ -n "$line" ]; do
        printf '%s\n' "$line"
      done < "${THRONGTERM_FAKE_TMUX_SESSIONS}"
    fi
    ;;
esac
"##,
        );

        let original_path = std::env::var_os("PATH");
        let original_cwd = std::env::var_os("THRONGTERM_FAKE_TMUX_CWD");
        let original_cmd = std::env::var_os("THRONGTERM_FAKE_TMUX_COMMAND");
        prepend_test_path(&bin_dir, original_path.as_deref());
        std::env::set_var("THRONGTERM_FAKE_TMUX_CWD", dir.path());
        std::env::set_var("THRONGTERM_FAKE_TMUX_COMMAND", "codex");

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let created = supervisor
            .create_session(
                None,
                Some(dir.path().to_string_lossy().into_owned()),
                Some(crate::types::SpawnTool::Codex),
                Some("investigate startup".to_string()),
            )
            .await
            .expect("create session");

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_cwd {
            Some(value) => std::env::set_var("THRONGTERM_FAKE_TMUX_CWD", value),
            None => std::env::remove_var("THRONGTERM_FAKE_TMUX_CWD"),
        }
        match original_cmd {
            Some(value) => std::env::set_var("THRONGTERM_FAKE_TMUX_COMMAND", value),
            None => std::env::remove_var("THRONGTERM_FAKE_TMUX_COMMAND"),
        }

        assert_eq!(created.0.session_id, "sess_0");
        assert_eq!(created.0.tmux_name, "0");
        assert_eq!(created.0.tool.as_deref(), Some("Codex"));
        assert_eq!(created.0.cwd, dir.path().to_string_lossy());
        supervisor
            .delete_session(
                &created.0.session_id,
                crate::config::SessionDeleteMode::DetachBridge,
            )
            .await
            .expect("cleanup session");
    }

    #[tokio::test]
    async fn discover_tmux_sessions_with_reason_uses_fake_tmux_listings() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        let sessions_file = dir.path().join("sessions.txt");
        std::fs::write(&sessions_file, "11\nworkspace\n").expect("sessions");
        write_executable(
            &bin_dir.join("tmux"),
            r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
    while IFS= read -r line || [ -n "$line" ]; do
      printf '%s\n' "$line"
    done < "${THRONGTERM_FAKE_TMUX_SESSIONS}"
    ;;
  attach-session|new-session)
    while IFS= read -r line; do
      printf '%s\r\n' "$line"
    done
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_command}") printf 'codex\n' ;;
      "#{pane_current_path}") printf '/tmp/project\n' ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
  send-keys|kill-session|capture-pane)
    exit 0
    ;;
esac
"##,
        );

        let original_path = std::env::var_os("PATH");
        let original_sessions = std::env::var_os("THRONGTERM_FAKE_TMUX_SESSIONS");
        prepend_test_path(&bin_dir, original_path.as_deref());
        std::env::set_var("THRONGTERM_FAKE_TMUX_SESSIONS", &sessions_file);

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .discover_tmux_sessions_with_reason("test_discovery")
            .await
            .expect("discover sessions");

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_sessions {
            Some(value) => std::env::set_var("THRONGTERM_FAKE_TMUX_SESSIONS", value),
            None => std::env::remove_var("THRONGTERM_FAKE_TMUX_SESSIONS"),
        }

        let sessions = supervisor.sessions.read().await;
        assert_eq!(sessions.len(), 2);
        assert!(sessions.values().any(|handle| handle.tmux_name == "11"));
        assert!(sessions
            .values()
            .any(|handle| handle.tmux_name == "workspace"));
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
