use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::Utc;
#[cfg(test)]
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot, Mutex, Notify, RwLock};
use tracing::{debug, info, warn};
#[cfg(test)]
use uuid::Uuid;

use crate::config::Config;
#[cfg(test)]
use crate::launcher::SpawnToolLauncher;
use crate::persistence::file_store::{FileStore, PersistedSession, ThoughtSnapshot};
use crate::repo_theme::discover_repo_theme;
use crate::session::actor::{run_bounded_tmux_command_for_target, ActorHandle, SessionCommand};
#[cfg(test)]
use crate::session::spawn_command::{
    build_initial_request_input, build_spawn_tool_command, build_spawn_tool_command_with_launcher,
    schedule_prelaunch_file_cleanup_after, shell_single_quote,
};
use crate::session::spawn_command::{
    current_working_dir, enqueue_initial_request_input, initial_request_delay, initial_tool_name,
    normalize_initial_request, normalize_requested_tmux_name, prepare_spawn_tool_command,
    schedule_prelaunch_file_cleanup, spawn_tool_consumes_initial_request,
    wrap_spawn_tool_command_for_tmux,
};
use crate::thought::loop_runner::SessionInfo;
#[cfg(test)]
use crate::thought::loop_runner::SessionProvider;
#[cfg(test)]
use crate::thought::protocol::ThoughtDeliveryState;
use crate::tmux_target::{exact_session_target, TmuxTarget};
#[cfg(test)]
use crate::types::SUMMARY_CAUSE_TMUX_RECONCILE_MISSING;
use crate::types::{
    fallback_rest_state, ControlEvent, DependencyHealthSnapshot, RepoTheme, SessionBatchMembership,
    SessionState, SessionSummary, SummaryFallbackReason, TerminalSnapshot, TransportHealth,
    SUMMARY_CAUSE_PERSISTENCE_STALE,
};
#[cfg(test)]
use crate::types::{ActionCue, RestState, ThoughtSource, ThoughtState};

mod active_panes;
mod discovery;
mod process_exit;
mod summary;
mod thought_persistence;
use self::active_panes::{filter_active_panes_to_requested, query_all_active_pane_session_ids};
#[cfg(test)]
use self::discovery::{
    classify_tmux_list_sessions_command_error, classify_tmux_list_sessions_output,
    TmuxListSessionsOutcome,
};
use self::summary::{
    active_pane_session_id_for_summary, merge_summary_with_thought_snapshot,
    merge_thought_snapshots_into_summaries, persisted_session_from_summary,
    session_info_from_summary, thought_snapshot_for_summary,
    tmux_names_requiring_active_pane_lookup,
};
pub use self::thought_persistence::SupervisorProvider;
use self::thought_persistence::THOUGHT_PERSIST_QUEUE_CAP;

const PROCESS_EXIT_SUMMARY_TIMEOUT: Duration = Duration::from_millis(250);
const TMUX_REDISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
const TMUX_KILL_SESSION_TIMEOUT: Duration = Duration::from_millis(500);

enum SummaryCollectOutcome {
    Live(SessionSummary),
    Fallback(SessionSummary),
    Exited(String),
    Missing,
}

#[derive(Debug, Clone)]
pub struct TmuxDependencyHealthSnapshot {
    pub discovery: DependencyHealthSnapshot,
    pub capture: DependencyHealthSnapshot,
}

struct TmuxDependencyHealthState {
    discovery: DependencyHealthSnapshot,
    capture: DependencyHealthSnapshot,
}

impl Default for TmuxDependencyHealthState {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            discovery: DependencyHealthSnapshot::unknown(now)
                .with_detail("dependency", "tmux_discovery"),
            capture: DependencyHealthSnapshot::unknown(now)
                .with_detail("dependency", "tmux_capture"),
        }
    }
}

#[cfg(test)]
fn tmux_query_command(args: &[&str]) -> Command {
    let mut command = Command::new("tmux");
    command
        .args(args)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .kill_on_drop(true);
    command
}

// ---------------------------------------------------------------------------
// Lifecycle events broadcast to all listeners
// ---------------------------------------------------------------------------

/// Events emitted by the supervisor when sessions are created or removed.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum LifecycleEvent {
    Created {
        session_id: String,
        summary: SessionSummary,
        reason: String,
        repo_theme: Option<RepoTheme>,
    },
    Deleted {
        session_id: String,
        reason: String,
        delete_mode: crate::config::SessionDeleteMode,
        tmux_session_alive: bool,
    },
}

#[derive(Debug, Clone)]
pub struct AdoptedTmuxSession {
    pub session: SessionSummary,
    pub repo_theme: Option<RepoTheme>,
    pub reused_session_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxAdoptError {
    EmptyTmuxName,
    DiscoveryUnavailable,
    TargetNotFound {
        tmux_name: String,
    },
    AmbiguousTarget {
        tmux_name: String,
        matches: usize,
    },
    AlreadyTracked {
        tmux_name: String,
        session_id: String,
    },
    StaleSessionNotFound {
        session_id: String,
    },
    StaleSessionConflict {
        session_id: String,
        stale_tmux_name: String,
        requested_tmux_name: String,
    },
    SpawnFailed {
        tmux_name: String,
        message: String,
    },
    InvalidTarget {
        message: String,
    },
}

impl fmt::Display for TmuxAdoptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTmuxName => write!(f, "tmux_name is required"),
            Self::DiscoveryUnavailable => {
                write!(f, "tmux session listing is unavailable; cannot safely adopt")
            }
            Self::TargetNotFound { tmux_name } => {
                write!(f, "tmux session `{tmux_name}` was not found")
            }
            Self::AmbiguousTarget { tmux_name, matches } => write!(
                f,
                "tmux session `{tmux_name}` is ambiguous ({matches} matches)"
            ),
            Self::AlreadyTracked {
                tmux_name,
                session_id,
            } => write!(
                f,
                "tmux session `{tmux_name}` is already tracked as `{session_id}`"
            ),
            Self::StaleSessionNotFound { session_id } => {
                write!(f, "stale session `{session_id}` was not found")
            }
            Self::StaleSessionConflict {
                session_id,
                stale_tmux_name,
                requested_tmux_name,
            } => write!(
                f,
                "stale session `{session_id}` is bound to tmux `{stale_tmux_name}`, not `{requested_tmux_name}`"
            ),
            Self::SpawnFailed { tmux_name, message } => {
                write!(f, "failed to adopt tmux session `{tmux_name}`: {message}")
            }
            Self::InvalidTarget { message } => write!(f, "invalid tmux target: {message}"),
        }
    }
}

impl std::error::Error for TmuxAdoptError {}

// ---------------------------------------------------------------------------
// Session supervisor
// ---------------------------------------------------------------------------

pub struct SessionSupervisor {
    config: Arc<Config>,

    /// Active session actors keyed by session_id.
    sessions: RwLock<HashMap<String, ActorHandle>>,

    /// Stale (exited) sessions from persistence that have no matching live tmux.
    stale_sessions: RwLock<Vec<SessionSummary>>,

    /// Last successful live summaries keyed by session_id. Session listing uses
    /// this to avoid treating transient actor backpressure as deletion.
    summary_cache: RwLock<HashMap<String, SessionSummary>>,

    /// Monotonic counter for generating numeric fallback session names.
    next_name_counter: AtomicU64,

    /// Monotonic counter for session IDs (separate from tmux names).
    next_id_counter: AtomicU64,

    /// Broadcast channel for lifecycle events. Subscribers can listen for
    /// session_created / session_deleted.
    lifecycle_tx: broadcast::Sender<LifecycleEvent>,

    /// Broadcast channel for thought_update ControlEvents from the thought loop.
    /// UI surfaces or other listeners subscribe to this to react to updates.
    thought_tx: broadcast::Sender<ControlEvent>,

    /// File-based persistence store, initialized after construction.
    persistence: RwLock<Option<Arc<FileStore>>>,

    /// Latest thought snapshots keyed by session_id.
    thought_snapshots: RwLock<HashMap<String, ThoughtSnapshot>>,

    /// Number of accepted thought-persist writes still queued or in flight.
    pending_thought_persists: AtomicUsize,

    /// Configured capacity of the bounded thought-persist channel. Defaults to
    /// `THOUGHT_PERSIST_QUEUE_CAP` but may differ when the provider is built via
    /// `with_persist_queue_capacity`; used for both the depth clamp and the
    /// backpressure snapshot so neither lies about a non-default capacity.
    thought_persist_queue_capacity: AtomicUsize,

    /// Last observed bounded thought-persist queue depth.
    thought_persist_queue_depth: AtomicUsize,

    /// Number of per-session overwrite slots currently holding coalesced writes.
    thought_persist_overflow_slots: AtomicUsize,

    /// Number of times the bounded thought-persist queue was full.
    thought_persist_queue_full_count: AtomicU64,

    /// Number of queued overflow writes replaced by a newer write for the same session.
    thought_persist_coalesced_count: AtomicU64,

    /// Number of thought writes that could not be queued or coalesced.
    thought_persist_dropped_count: AtomicU64,

    /// Wakes shutdown waiters when the pending thought-persist count changes.
    pending_thought_persists_notify: Notify,

    /// First-observed timestamps for sessions that have entered Exited state.
    process_exit_seen_at: RwLock<HashMap<String, Instant>>,

    /// Serializes tmux discovery so concurrent callers cannot race and attach
    /// duplicate actors to the same tmux session.
    discovery_lock: Mutex<()>,

    /// Memoizes `tmux list-panes -a` output so the TUI's polling cadence
    /// (every ~1–2s) doesn't pay the subprocess fork+exec on every call.
    /// Bounded staleness is 1s; active_pane_session_id only feeds the
    /// thought-snapshot merge fallback, where it's tolerated.
    active_pane_cache: Mutex<ActivePaneCache>,

    /// Latest tmux dependency observations for /health.
    tmux_dependency_health: StdMutex<TmuxDependencyHealthState>,
}

#[derive(Default)]
struct ActivePaneCache {
    fetched_at: Option<Instant>,
    tmux_target: TmuxTarget,
    panes: HashMap<String, String>,
}

const ACTIVE_PANE_CACHE_TTL: Duration = Duration::from_millis(1000);

impl SessionSupervisor {
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        let (lifecycle_tx, _) = broadcast::channel(64);
        let (thought_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            config,
            sessions: RwLock::new(HashMap::new()),
            stale_sessions: RwLock::new(Vec::new()),
            summary_cache: RwLock::new(HashMap::new()),
            next_name_counter: AtomicU64::new(0),
            next_id_counter: AtomicU64::new(0),
            lifecycle_tx,
            thought_tx,
            persistence: RwLock::new(None),
            thought_snapshots: RwLock::new(HashMap::new()),
            pending_thought_persists: AtomicUsize::new(0),
            thought_persist_queue_capacity: AtomicUsize::new(THOUGHT_PERSIST_QUEUE_CAP),
            thought_persist_queue_depth: AtomicUsize::new(0),
            thought_persist_overflow_slots: AtomicUsize::new(0),
            thought_persist_queue_full_count: AtomicU64::new(0),
            thought_persist_coalesced_count: AtomicU64::new(0),
            thought_persist_dropped_count: AtomicU64::new(0),
            pending_thought_persists_notify: Notify::new(),
            process_exit_seen_at: RwLock::new(HashMap::new()),
            discovery_lock: Mutex::new(()),
            active_pane_cache: Mutex::new(ActivePaneCache::default()),
            tmux_dependency_health: StdMutex::new(TmuxDependencyHealthState::default()),
        })
    }

    pub fn tmux_dependency_health_snapshot(&self) -> TmuxDependencyHealthSnapshot {
        let health = self
            .tmux_dependency_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        TmuxDependencyHealthSnapshot {
            discovery: health.discovery.clone(),
            capture: health.capture.clone(),
        }
    }

    fn record_tmux_discovery_success(&self, reason: &'static str, session_count: usize) {
        let now = Utc::now();
        let mut health = self
            .tmux_dependency_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.discovery = DependencyHealthSnapshot::healthy(now)
            .with_detail("reason", reason)
            .with_detail("session_count", session_count.to_string());
    }

    fn record_tmux_discovery_failure(&self, reason: &'static str, error: impl Into<String>) {
        let now = Utc::now();
        let mut health = self
            .tmux_dependency_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.discovery =
            DependencyHealthSnapshot::unavailable(now, error).with_detail("reason", reason);
    }

    fn record_tmux_capture_success(&self, reason: &'static str, pane_count: usize) {
        let now = Utc::now();
        let mut health = self
            .tmux_dependency_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.capture = DependencyHealthSnapshot::healthy(now)
            .with_detail("reason", reason)
            .with_detail("pane_count", pane_count.to_string());
    }

    fn record_tmux_capture_failure(&self, reason: &'static str, error: impl Into<String>) {
        let now = Utc::now();
        let mut health = self
            .tmux_dependency_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.capture =
            DependencyHealthSnapshot::degraded(now, error).with_detail("reason", reason);
    }

    /// Returns the active-pane session-id map for the requested `tmux_names`,
    /// reusing a recent `tmux list-panes -a` result when it is within TTL.
    async fn active_pane_session_ids_cached(
        &self,
        tmux_names: &HashSet<String>,
        reason: &'static str,
    ) -> HashMap<String, String> {
        if tmux_names.is_empty() {
            return HashMap::new();
        }

        {
            let cache = self.active_pane_cache.lock().await;
            if let Some(at) = cache.fetched_at {
                if cache.tmux_target == self.config.tmux_target
                    && at.elapsed() < ACTIVE_PANE_CACHE_TTL
                {
                    return filter_active_panes_to_requested(&cache.panes, tmux_names);
                }
            }
        }

        let fresh = match query_all_active_pane_session_ids(&self.config.tmux_target).await {
            Ok(panes) => {
                self.record_tmux_capture_success(reason, panes.len());
                panes
            }
            Err(err) => {
                self.record_tmux_capture_failure(reason, err.to_string());
                warn!(
                    reason,
                    tmux_names = tmux_names.len(),
                    "skipping tmux active pane lookup: {err}"
                );
                return HashMap::new();
            }
        };

        let filtered = filter_active_panes_to_requested(&fresh, tmux_names);

        {
            let mut cache = self.active_pane_cache.lock().await;
            cache.fetched_at = Some(Instant::now());
            cache.tmux_target = self.config.tmux_target.clone();
            cache.panes = fresh;
        }

        filtered
    }

    async fn active_pane_session_ids_for_summaries<'a, I>(
        &self,
        summaries: I,
        thought_snapshots: &HashMap<String, ThoughtSnapshot>,
        reason: &'static str,
    ) -> HashMap<String, String>
    where
        I: IntoIterator<Item = &'a SessionSummary>,
    {
        let tmux_names = tmux_names_requiring_active_pane_lookup(
            summaries
                .into_iter()
                .filter(|summary| summary.tmux_target == self.config.tmux_target),
            thought_snapshots,
        );
        self.active_pane_session_ids_cached(&tmux_names, reason)
            .await
    }

    fn resolve_repo_theme_for_summary(&self, summary: &mut SessionSummary) -> Option<RepoTheme> {
        if summary.cwd.is_empty() {
            summary.repo_theme_id = None;
            return None;
        }

        let repo_theme = discover_repo_theme(&summary.cwd).map(|(theme_id, theme)| {
            summary.repo_theme_id = Some(theme_id);
            theme
        });
        if repo_theme.is_none() {
            summary.repo_theme_id = None;
        }
        repo_theme
    }

    fn resolve_repo_themes_for_summaries(&self, summaries: &mut [SessionSummary]) {
        for summary in summaries {
            self.resolve_repo_theme_for_summary(summary);
        }
    }

    /// Initialize persistence store and load persisted sessions as stale entries.
    pub async fn init_persistence(self: &Arc<Self>, store: Arc<FileStore>) {
        let persisted = store.load_sessions().await;
        let thoughts = store.load_thoughts().await;

        self.advance_id_counter_from_persisted_state(&persisted, &thoughts);
        self.assign_stale_sessions_from_persistence(&persisted, &thoughts)
            .await;
        self.assign_thought_snapshots(thoughts).await;
        self.install_persistence_store(store).await;
    }

    fn advance_id_counter_from_persisted_state(
        &self,
        persisted: &[PersistedSession],
        thoughts: &HashMap<String, ThoughtSnapshot>,
    ) {
        for ps in persisted {
            self.bump_id_counter_from_session_id(&ps.session_id);
        }

        for session_id in thoughts.keys() {
            self.bump_id_counter_from_session_id(session_id);
        }
    }

    async fn assign_stale_sessions_from_persistence(
        &self,
        persisted: &[PersistedSession],
        thoughts: &HashMap<String, ThoughtSnapshot>,
    ) {
        if persisted.is_empty() {
            return;
        }

        let stale = self.hydrate_stale_summaries(persisted, thoughts);
        info!(count = stale.len(), "loaded persisted stale sessions");
        let mut stale_lock = self.stale_sessions.write().await;
        *stale_lock = stale;
    }

    fn hydrate_stale_summaries(
        &self,
        persisted: &[PersistedSession],
        thoughts: &HashMap<String, ThoughtSnapshot>,
    ) -> Vec<SessionSummary> {
        persisted
            .iter()
            .map(|ps| self.hydrate_stale_summary(ps, thoughts.get(&ps.session_id)))
            .collect()
    }

    fn hydrate_stale_summary(
        &self,
        ps: &PersistedSession,
        thought_data: Option<&ThoughtSnapshot>,
    ) -> SessionSummary {
        let thought_state = thought_data
            .map(|t| t.thought_state)
            .unwrap_or(ps.thought_state);
        let rest_state = thought_data
            .map(|t| t.rest_state)
            .unwrap_or_else(|| fallback_rest_state(SessionState::Exited, ps.thought_state));
        let mut summary =
            SessionSummary::placeholder(&ps.session_id, &ps.tmux_name, ps.last_activity_at);
        summary.tmux_target = ps.tmux_target.clone();
        summary.cwd = ps.cwd.clone();
        summary.tool = ps.tool.clone();
        summary.context_limit = thought_data
            .map(|t| t.context_limit)
            .unwrap_or(ps.context_limit);
        summary.token_count = thought_data
            .map(|t| t.token_count)
            .unwrap_or(ps.token_count);
        summary.thought = thought_data
            .and_then(|t| t.thought.clone())
            .or_else(|| ps.thought.clone());
        summary.thought_state = thought_state;
        summary.thought_source = thought_data
            .map(|t| t.thought_source)
            .unwrap_or(ps.thought_source);
        summary.thought_updated_at = thought_data.map(|t| t.updated_at).or(ps.thought_updated_at);
        summary.commit_candidate = thought_data
            .map(|t| t.commit_candidate)
            .unwrap_or(ps.commit_candidate);
        summary.action_cues = thought_data
            .map(|t| t.action_cues.clone())
            .unwrap_or_else(|| ps.action_cues.clone());
        summary.objective_changed_at = thought_data
            .and_then(|t| t.objective_changed_at)
            .or(ps.objective_changed_at);
        summary.last_skill = ps.last_skill.clone();
        summary.batch = ps.batch.clone();
        let mut summary = summary.into_stale_exited_with_rest_state(
            SUMMARY_CAUSE_PERSISTENCE_STALE,
            None,
            TransportHealth::Disconnected,
            rest_state,
        );
        self.resolve_repo_theme_for_summary(&mut summary);
        summary
    }

    async fn assign_thought_snapshots(&self, thoughts: HashMap<String, ThoughtSnapshot>) {
        let mut thought_cache = self.thought_snapshots.write().await;
        *thought_cache = thoughts;
    }

    async fn install_persistence_store(&self, store: Arc<FileStore>) {
        let mut persistence = self.persistence.write().await;
        *persistence = Some(store);
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
    ) -> anyhow::Result<(SessionSummary, Option<RepoTheme>)> {
        self.create_session_with_batch(name, cwd, spawn_tool, initial_request, None)
            .await
    }

    pub async fn create_session_with_batch(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: Option<String>,
        spawn_tool: Option<crate::types::SpawnTool>,
        initial_request: Option<String>,
        batch: Option<SessionBatchMembership>,
    ) -> anyhow::Result<(SessionSummary, Option<RepoTheme>)> {
        self.create_session_with_target_and_batch(
            name,
            cwd,
            spawn_tool,
            initial_request,
            None,
            batch,
        )
        .await
    }

    pub async fn create_session_with_target_and_batch(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: Option<String>,
        spawn_tool: Option<crate::types::SpawnTool>,
        initial_request: Option<String>,
        tmux_target: Option<TmuxTarget>,
        batch: Option<SessionBatchMembership>,
    ) -> anyhow::Result<(SessionSummary, Option<RepoTheme>)> {
        let start_cwd = cwd.or_else(current_working_dir);
        let mut initial_request = normalize_initial_request(initial_request);
        let tmux_target = tmux_target.unwrap_or_else(|| self.config.tmux_target.clone());
        tmux_target.validate()?;
        let tmux_name = self.allocate_tmux_name(name);
        let session_id = self.allocate_unique_session_id().await;

        if let Some(dir) = start_cwd.as_deref() {
            if !Path::new(dir).is_dir() {
                return Err(anyhow::anyhow!(
                    "session cwd does not exist or is not a directory: {dir}"
                ));
            }
        }

        info!(
            session_id = %session_id,
            tmux_name = %tmux_name,
            tmux_target = %tmux_target.display_label(),
            "creating new session"
        );

        let initial_tool = initial_tool_name(spawn_tool.as_ref());
        let mut prelaunch_cleanup_paths = Vec::new();
        let initial_command = spawn_tool.map(|tool| {
            let command =
                prepare_spawn_tool_command(tool, start_cwd.as_deref(), initial_request.as_deref());
            prelaunch_cleanup_paths = command.cleanup_paths;
            if spawn_tool_consumes_initial_request(tool) {
                initial_request = None;
            }
            wrap_spawn_tool_command_for_tmux(&command.command)
        });
        let handle = match crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            tmux_target.clone(),
            false, // create new
            start_cwd.clone(),
            initial_tool.clone(),
            initial_command,
            self.config.clone(),
            None,
            batch.clone(),
        ) {
            Ok(handle) => handle,
            Err(err) => {
                schedule_prelaunch_file_cleanup(prelaunch_cleanup_paths);
                return Err(err);
            }
        };
        let bootstrap_handle = handle.clone();

        self.insert_active_handle(session_id.clone(), handle).await;
        let mut summary = self
            .build_created_summary(
                &session_id,
                &tmux_name,
                &tmux_target,
                start_cwd.as_deref(),
                initial_tool.as_deref(),
                batch,
            )
            .await;
        let repo_theme = self.resolve_repo_theme_for_summary(&mut summary);
        let initial_request_delay = initial_request_delay(spawn_tool, initial_request.as_ref());
        self.enqueue_initial_request_if_present(
            bootstrap_handle,
            &session_id,
            &tmux_name,
            initial_request,
            initial_request_delay,
        );
        self.emit_created_session(&session_id, &summary, repo_theme.clone());
        self.persist_registry().await;

        Ok((summary, repo_theme))
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
        tmux_target: &TmuxTarget,
        start_cwd: Option<&str>,
        initial_tool: Option<&str>,
        batch: Option<SessionBatchMembership>,
    ) -> SessionSummary {
        let mut summary = self.build_placeholder_summary(session_id, tmux_name);
        summary.tmux_target = tmux_target.clone();
        if let Some(cwd) = start_cwd {
            summary.cwd = cwd.to_string();
        }
        if let Some(display) = initial_tool {
            summary.tool = Some(display.to_string());
            summary.context_limit = crate::types::context_limit_for_tool(Some(display));
        }
        summary.batch = batch;
        summary
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
        repo_theme: Option<RepoTheme>,
    ) {
        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id: session_id.to_string(),
            summary: summary.clone(),
            reason: "api_create".into(),
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
            if let Err(e) = kill_tmux_session(&handle.tmux_name, &handle.tmux_target).await {
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

    fn cached_summary_fallback(
        cached: Option<SessionSummary>,
        reason: SummaryFallbackReason,
    ) -> SummaryCollectOutcome {
        match cached {
            Some(summary) => {
                crate::metrics::increment_summary_fallback(reason);
                SummaryCollectOutcome::Fallback(summary.into_cached_collection_fallback(reason))
            }
            None => {
                crate::metrics::increment_summary_fallback(SummaryFallbackReason::Missing);
                SummaryCollectOutcome::Missing
            }
        }
    }

    async fn collect_summary_from_handle(
        handle: ActorHandle,
        cached: Option<SessionSummary>,
        timeout: Duration,
    ) -> SummaryCollectOutcome {
        let (tx, rx) = oneshot::channel();
        if handle
            .cmd_tx
            .send(SessionCommand::GetSummary(tx))
            .await
            .is_err()
        {
            warn!(session_id = %handle.session_id, "actor summary command channel closed");
            return Self::cached_summary_fallback(cached, SummaryFallbackReason::ChannelClosed);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(summary)) => Self::summary_reply_outcome(summary),
            Ok(Err(_)) => {
                warn!(session_id = %handle.session_id, "actor dropped summary reply");
                Self::cached_summary_fallback(cached, SummaryFallbackReason::Dropped)
            }
            Err(_) => {
                warn!(session_id = %handle.session_id, "summary request timed out");
                Self::cached_summary_fallback(cached, SummaryFallbackReason::Timeout)
            }
        }
    }

    fn summary_reply_outcome(summary: SessionSummary) -> SummaryCollectOutcome {
        if summary.state == SessionState::Exited {
            SummaryCollectOutcome::Exited(summary.session_id)
        } else {
            SummaryCollectOutcome::Live(summary)
        }
    }

    async fn collect_live_summaries(&self, timeout: Duration) -> Vec<SessionSummary> {
        let handles: Vec<ActorHandle> = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect()
        };
        let live_session_ids = handles
            .iter()
            .map(|handle| handle.session_id.clone())
            .collect::<HashSet<_>>();
        let handles_with_cached = {
            let cache = self.summary_cache.read().await;
            handles
                .into_iter()
                .map(|handle| {
                    let cached = cache.get(&handle.session_id).cloned();
                    (handle, cached)
                })
                .collect::<Vec<_>>()
        };

        let futs: Vec<_> = handles_with_cached
            .into_iter()
            .map(|(handle, cached)| Self::collect_summary_from_handle(handle, cached, timeout))
            .collect();

        let mut summaries = Vec::new();
        let mut live_updates = Vec::new();
        let mut exited_ids = Vec::new();
        for outcome in futures::future::join_all(futs).await {
            match outcome {
                SummaryCollectOutcome::Live(summary) => {
                    live_updates.push(summary.clone());
                    summaries.push(summary);
                }
                SummaryCollectOutcome::Fallback(summary) => summaries.push(summary),
                SummaryCollectOutcome::Exited(session_id) => exited_ids.push(session_id),
                SummaryCollectOutcome::Missing => {}
            }
        }

        {
            let mut cache = self.summary_cache.write().await;
            cache.retain(|session_id, _| live_session_ids.contains(session_id));
            for session_id in exited_ids {
                cache.remove(&session_id);
            }
            for summary in live_updates {
                cache.insert(summary.session_id.clone(), summary);
            }
        }

        summaries
    }

    /// List summaries for all active sessions.
    pub async fn list_sessions(&self) -> Vec<SessionSummary> {
        let mut summaries = self.collect_live_summaries(Duration::from_secs(2)).await;

        let thought_snapshots = self.thought_snapshots.read().await.clone();
        let active_pane_session_ids = self
            .active_pane_session_ids_for_summaries(
                summaries.iter(),
                &thought_snapshots,
                "list_sessions",
            )
            .await;
        merge_thought_snapshots_into_summaries(
            &mut summaries,
            &thought_snapshots,
            &active_pane_session_ids,
        );
        // Keep repo theme discovery after thought enrichment so the summary
        // exposes a final cwd/theme-id pair to API and TUI callers.
        self.resolve_repo_themes_for_summaries(&mut summaries);

        summaries
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

    #[cfg(any(test, debug_assertions))]
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
        self.collect_session_snapshots_with_timeout(Duration::from_secs(2))
            .await
    }

    async fn collect_summary_and_replay(
        handle: ActorHandle,
        timeout: Duration,
    ) -> Option<(SessionSummary, String)> {
        let (sum_rx, snap_rx) = Self::send_snapshot_collection_requests(handle).await?;
        let (summary, snapshot) = tokio::join!(
            tokio::time::timeout(timeout, sum_rx),
            tokio::time::timeout(timeout, snap_rx)
        );

        let summary = Self::summary_from_reply(summary)?;
        let replay_text = Self::snapshot_from_reply(snapshot)
            .map(Self::snapshot_replay_tail)
            .unwrap_or_default();
        Some((summary, replay_text))
    }

    async fn send_snapshot_collection_requests(
        handle: ActorHandle,
    ) -> Option<(
        oneshot::Receiver<SessionSummary>,
        oneshot::Receiver<TerminalSnapshot>,
    )> {
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

        if summary_sent && snapshot_sent {
            Some((sum_rx, snap_rx))
        } else {
            None
        }
    }

    fn summary_from_reply(
        reply: Result<
            Result<SessionSummary, oneshot::error::RecvError>,
            tokio::time::error::Elapsed,
        >,
    ) -> Option<SessionSummary> {
        match reply {
            Ok(Ok(summary)) => Some(summary),
            _ => None,
        }
    }

    fn snapshot_from_reply(
        reply: Result<
            Result<TerminalSnapshot, oneshot::error::RecvError>,
            tokio::time::error::Elapsed,
        >,
    ) -> Option<TerminalSnapshot> {
        match reply {
            Ok(Ok(snapshot)) => Some(snapshot),
            _ => None,
        }
    }

    fn snapshot_replay_tail(snapshot: TerminalSnapshot) -> String {
        let chars: Vec<char> = snapshot.screen_text.chars().collect();
        let start = chars.len().saturating_sub(500);
        chars[start..].iter().collect()
    }

    async fn collect_session_snapshots_with_timeout(&self, timeout: Duration) -> Vec<SessionInfo> {
        let handles: Vec<ActorHandle> = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect()
        };
        let thought_snapshots = self.thought_snapshots.read().await.clone();

        let futs: Vec<_> = handles
            .into_iter()
            .map(|handle| Self::collect_summary_and_replay(handle, timeout))
            .collect();

        let summaries_with_replay: Vec<(SessionSummary, String)> = futures::future::join_all(futs)
            .await
            .into_iter()
            .flatten()
            .collect();

        let active_pane_session_ids = self
            .active_pane_session_ids_for_summaries(
                summaries_with_replay.iter().map(|(summary, _)| summary),
                &thought_snapshots,
                "collect_session_infos",
            )
            .await;

        summaries_with_replay
            .into_iter()
            .map(|(summary, replay_text)| {
                let active_pane_session_id = active_pane_session_id_for_summary(
                    &summary,
                    &thought_snapshots,
                    &active_pane_session_ids,
                );
                let thought_data = thought_snapshot_for_summary(
                    &summary,
                    active_pane_session_id.as_deref(),
                    &thought_snapshots,
                );
                session_info_from_summary(summary, replay_text, thought_data)
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    /// Look up the persisted `last_activity_at` for a session id so a
    /// discovered/adopted actor can resume in the correct rest state instead
    /// of resetting its fatigue ladder to "Active" on every restart.
    async fn persisted_last_activity(&self, session_id: &str) -> Option<chrono::DateTime<Utc>> {
        let store = {
            let guard = self.persistence.read().await;
            guard.as_ref().cloned()?
        };
        store
            .load_sessions()
            .await
            .into_iter()
            .find(|ps| ps.session_id == session_id)
            .map(|ps| ps.last_activity_at)
    }

    async fn persisted_batch(&self, session_id: &str) -> Option<SessionBatchMembership> {
        let store = {
            let guard = self.persistence.read().await;
            guard.as_ref().cloned()?
        };
        store
            .load_sessions()
            .await
            .into_iter()
            .find(|ps| ps.session_id == session_id)
            .and_then(|ps| ps.batch)
    }

    /// Persist the current session registry to disk.
    pub async fn persist_registry(&self) {
        let store = {
            let guard = self.persistence.read().await;
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => return,
            }
        };

        let thought_snapshots = self.thought_snapshots.read().await.clone();
        let persisted: Vec<PersistedSession> = self
            .collect_live_summaries(Duration::from_secs(2))
            .await
            .into_iter()
            .map(|mut summary| {
                let thought_data = thought_snapshots.get(&summary.session_id);
                if let Some(thought_data) = thought_data {
                    merge_summary_with_thought_snapshot(&mut summary, thought_data);
                }
                persisted_session_from_summary(&summary, thought_data)
            })
            .collect();

        store.save_sessions(&persisted).await;
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

    /// Spawn a bounded background task that keeps in-memory actors reconciled
    /// with tmux sessions created or removed outside swimmers after startup.
    pub fn spawn_tmux_reconcile_loop(self: &Arc<Self>) {
        let supervisor = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TMUX_REDISCOVERY_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = supervisor
                    .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
                    .await
                {
                    warn!("periodic tmux reconcile failed: {err}");
                }
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

    /// Build a minimal placeholder summary. The real summary comes from the
    /// actor via `GetSummary`, but we need something for lifecycle events that
    /// fire before the actor has processed any output.
    fn build_placeholder_summary(&self, session_id: &str, tmux_name: &str) -> SessionSummary {
        SessionSummary::placeholder(session_id, tmux_name, Utc::now())
    }
}

async fn kill_tmux_session(tmux_name: &str, tmux_target: &TmuxTarget) -> anyhow::Result<()> {
    let target = exact_session_target(tmux_name);
    let output = run_bounded_tmux_command_for_target(
        "tmux",
        tmux_target,
        &["kill-session", "-t", &target],
        TMUX_KILL_SESSION_TIMEOUT,
        "kill-session",
    )
    .await?;

    classify_kill_tmux_session_result(output.status.success(), &output.stderr)
}

fn classify_kill_tmux_session_result(success: bool, stderr: &[u8]) -> anyhow::Result<()> {
    if success {
        Ok(())
    } else {
        classify_failed_kill_tmux_session(stderr)
    }
}

fn classify_failed_kill_tmux_session(stderr: &[u8]) -> anyhow::Result<()> {
    let stderr = String::from_utf8_lossy(stderr);
    if tmux_kill_reports_missing_session(&stderr) {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "tmux kill-session failed: {}",
        stderr.trim()
    ))
}

fn tmux_kill_reports_missing_session(stderr: &str) -> bool {
    stderr.contains("can't find session") || stderr.contains("no server running")
}

fn next_session_counter(session_id: &str) -> Option<u64> {
    let n = session_id.strip_prefix("sess_")?.parse::<u64>().ok()?;
    Some(n.saturating_add(1))
}

#[cfg(test)]
mod tests;
