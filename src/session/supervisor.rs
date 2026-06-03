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
use tracing::{debug, error, info, warn};
#[cfg(test)]
use uuid::Uuid;

use crate::config::Config;
#[cfg(test)]
use crate::launcher::SpawnToolLauncher;
use crate::persistence::file_store::{FileStore, PersistedSession, ThoughtSnapshot};
use crate::repo_theme::discover_repo_theme;
use crate::session::actor::{run_bounded_tmux_command, ActorHandle, SessionCommand};
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
use crate::tmux_target::exact_session_target;
use crate::types::{
    fallback_rest_state, ControlEvent, DependencyHealthSnapshot, RepoTheme, SessionBatchMembership,
    SessionState, SessionStatePayload, SessionSummary, SummaryFallbackReason, TerminalSnapshot,
    TransportHealth, SUMMARY_CAUSE_PERSISTENCE_STALE, SUMMARY_CAUSE_STARTUP_MISSING_TMUX,
    SUMMARY_CAUSE_TMUX_RECONCILE_MISSING,
};
#[cfg(test)]
use crate::types::{ActionCue, RestState, ThoughtSource, ThoughtState};

mod thought_persistence;
pub use self::thought_persistence::SupervisorProvider;
use self::thought_persistence::THOUGHT_PERSIST_QUEUE_CAP;

const PROCESS_EXIT_REAP_INTERVAL: Duration = Duration::from_millis(250);
const PROCESS_EXIT_DELETE_GRACE: Duration = Duration::ZERO;
const PROCESS_EXIT_SUMMARY_TIMEOUT: Duration = Duration::from_millis(250);
const TMUX_REDISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
const TMUX_LIST_SESSIONS_TIMEOUT: Duration = Duration::from_secs(2);
const TMUX_KILL_SESSION_TIMEOUT: Duration = Duration::from_millis(500);
const ACTIVE_PANE_LOOKUP_TIMEOUT: Duration = Duration::from_millis(500);
const ACTIVE_PANE_LOOKUP_WARN_THRESHOLD: Duration = Duration::from_millis(200);
const TMUX_LIST_PANES_FIELD_SEPARATOR: char = '\x1f';

struct ListedTmuxSessions {
    reliable: bool,
    names: Vec<String>,
}

struct MissingTrackedSessionSummary {
    session_id: String,
    previous_state: SessionState,
    summary: SessionSummary,
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

fn thought_snapshot_for_summary<'a>(
    summary: &SessionSummary,
    active_pane_session_id: Option<&str>,
    thought_snapshots: &'a HashMap<String, ThoughtSnapshot>,
) -> Option<&'a ThoughtSnapshot> {
    thought_snapshots
        .get(&summary.session_id)
        .or_else(|| active_pane_session_id.and_then(|session_id| thought_snapshots.get(session_id)))
}

fn merge_summary_with_thought_snapshot(
    summary: &mut SessionSummary,
    thought_data: &ThoughtSnapshot,
) {
    if summary.thought.is_none() {
        summary.thought = thought_data.thought.clone();
    }
    summary.thought_state = thought_data.thought_state;
    summary.thought_source = thought_data.thought_source;
    summary.thought_updated_at = Some(thought_data.updated_at);
    summary.rest_state = thought_data.rest_state;
    summary.commit_candidate = thought_data.commit_candidate;
    summary.action_cues = thought_data.action_cues.clone();
    summary.objective_changed_at = thought_data.objective_changed_at;
    if thought_data.token_count > 0 || summary.token_count == 0 {
        summary.token_count = thought_data.token_count;
    }
    if thought_data.context_limit > 0 {
        summary.context_limit = thought_data.context_limit;
    }
}

fn persisted_session_from_summary(
    summary: &SessionSummary,
    thought_data: Option<&ThoughtSnapshot>,
) -> PersistedSession {
    PersistedSession {
        session_id: summary.session_id.clone(),
        tmux_name: summary.tmux_name.clone(),
        state: summary.state,
        tool: summary.tool.clone(),
        token_count: summary.token_count,
        context_limit: summary.context_limit,
        thought: summary.thought.clone(),
        thought_state: summary.thought_state,
        thought_source: summary.thought_source,
        thought_updated_at: summary.thought_updated_at,
        rest_state: summary.rest_state,
        commit_candidate: summary.commit_candidate,
        action_cues: thought_data
            .map(|snapshot| snapshot.action_cues.clone())
            .unwrap_or_else(|| summary.action_cues.clone()),
        objective_changed_at: summary.objective_changed_at,
        last_skill: summary.last_skill.clone(),
        objective_fingerprint: thought_data
            .and_then(|snapshot| snapshot.objective_fingerprint.clone()),
        batch: summary.batch.clone(),
        cwd: summary.cwd.clone(),
        last_activity_at: summary.last_activity_at,
    }
}

fn format_tmux_active_pane_session_id(tmux_name: &str, pane_selector: &str) -> String {
    format!("tmux:{tmux_name}:{pane_selector}")
}

fn tmux_names_requiring_active_pane_lookup<'a, I>(
    summaries: I,
    thought_snapshots: &HashMap<String, ThoughtSnapshot>,
) -> HashSet<String>
where
    I: IntoIterator<Item = &'a SessionSummary>,
{
    if thought_snapshots.is_empty() {
        return HashSet::new();
    }

    summaries
        .into_iter()
        .filter(|summary| {
            !thought_snapshots.contains_key(&summary.session_id)
                && !summary.tmux_name.is_empty()
                && summary.state != SessionState::Exited
        })
        .map(|summary| summary.tmux_name.clone())
        .collect()
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

/// Runs `tmux list-panes -a` once and returns every session's active-pane id.
/// Callers that only care about specific tmux session names should pair this
/// with [`filter_active_panes_to_requested`]; keeping the query unfiltered
/// lets the supervisor share one tmux call across callers within the cache
/// TTL window.
async fn query_all_active_pane_session_ids() -> anyhow::Result<HashMap<String, String>> {
    let started = Instant::now();
    let pane_format = format!(
        "#{{session_name}}{sep}#{{window_active}}{sep}#{{pane_active}}{sep}#{{window_index}}.#{{pane_index}}:#{{pane_id}}",
        sep = TMUX_LIST_PANES_FIELD_SEPARATOR
    );
    let output = run_bounded_tmux_command(
        "tmux",
        &["list-panes", "-a", "-F", pane_format.as_str()],
        ACTIVE_PANE_LOOKUP_TIMEOUT,
        "list-panes",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("tmux list-panes failed: {}", stderr.trim()));
    }

    let elapsed = started.elapsed();
    if elapsed >= ACTIVE_PANE_LOOKUP_WARN_THRESHOLD {
        warn!(
            phase = "tmux_list_panes",
            elapsed_ms = elapsed.as_millis() as u64,
            "tmux active pane lookup completed slowly"
        );
    }

    Ok(parse_active_pane_session_ids(&output.stdout))
}

fn parse_active_pane_session_ids(stdout: &[u8]) -> HashMap<String, String> {
    let mut active_panes = HashMap::new();
    for line in String::from_utf8_lossy(stdout).lines() {
        let mut fields = line.splitn(4, TMUX_LIST_PANES_FIELD_SEPARATOR);
        let session_name = fields.next().unwrap_or_default();
        let window_active = fields.next().unwrap_or_default();
        let pane_active = fields.next().unwrap_or_default();
        let pane_selector = fields.next().unwrap_or_default();

        if session_name.is_empty()
            || window_active != "1"
            || pane_active != "1"
            || pane_selector.is_empty()
        {
            continue;
        }

        active_panes.insert(
            session_name.to_string(),
            format_tmux_active_pane_session_id(session_name, pane_selector),
        );
    }

    active_panes
}

fn filter_active_panes_to_requested(
    all: &HashMap<String, String>,
    tmux_names: &HashSet<String>,
) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(tmux_names.len().min(all.len()));
    for name in tmux_names {
        if let Some(id) = all.get(name) {
            out.insert(name.clone(), id.clone());
        }
    }
    out
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
                if at.elapsed() < ACTIVE_PANE_CACHE_TTL {
                    return filter_active_panes_to_requested(&cache.panes, tmux_names);
                }
            }
        }

        let fresh = match query_all_active_pane_session_ids().await {
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
            cache.panes = fresh;
        }

        filtered
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
                let thought_state = thought_data
                    .map(|t| t.thought_state)
                    .unwrap_or(ps.thought_state);
                let rest_state = thought_data
                    .map(|t| t.rest_state)
                    .unwrap_or_else(|| fallback_rest_state(SessionState::Exited, ps.thought_state));
                let mut summary =
                    SessionSummary::placeholder(&ps.session_id, &ps.tmux_name, ps.last_activity_at);
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
                summary.thought_updated_at =
                    thought_data.map(|t| t.updated_at).or(ps.thought_updated_at);
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
        let output = run_bounded_tmux_command(
            "tmux",
            &["list-sessions", "-F", "#{session_name}"],
            TMUX_LIST_SESSIONS_TIMEOUT,
            "list-sessions",
        )
        .await;

        match output {
            Ok(output) if output.status.success() => {
                let names = parse_tmux_session_names(&output.stdout);
                log_tmux_list_success(reason, list_started.elapsed(), names.len());
                self.record_tmux_discovery_success(reason, names.len());
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
                    self.record_tmux_discovery_success(reason, 0);
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
                    self.record_tmux_discovery_failure(reason, stderr.trim().to_string());
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
                self.record_tmux_discovery_failure(reason, error.to_string());
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

    async fn active_session_id_for_tmux(&self, tmux_name: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .find(|handle| handle.tmux_name == tmux_name)
            .map(|handle| handle.session_id.clone())
    }

    async fn stale_summary_for_id(&self, session_id: &str) -> Option<SessionSummary> {
        let stale = self.stale_sessions.read().await;
        stale
            .iter()
            .find(|summary| summary.session_id == session_id)
            .cloned()
    }

    async fn stale_summaries_for_tmux(&self, tmux_name: &str) -> Vec<SessionSummary> {
        let stale = self.stale_sessions.read().await;
        stale
            .iter()
            .filter(|summary| summary.tmux_name == tmux_name)
            .cloned()
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

        // Carry the persisted `last_activity_at` forward so long-silent
        // sessions resume in the correct fallback rest state (e.g. discovered
        // at startup after an overnight idle should wake up already drowsy,
        // not reset to Active before transcript sync has a chance to mark it
        // as waiting on the user).
        let last_activity_override = self.persisted_last_activity(&session_id).await;
        let batch = self.persisted_batch(&session_id).await;

        match crate::session::actor::SessionActor::spawn(
            session_id.clone(),
            tmux_name.clone(),
            true,
            None,
            None,
            None,
            self.config.clone(),
            last_activity_override,
            batch,
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
            repo_theme: None,
        });
    }

    async fn reconcile_stale_sessions_after_discovery(
        &self,
        discovery_reliable: bool,
        listed_tmux_names: &[String],
    ) {
        if !discovery_reliable {
            warn!("skipping stale reconciliation due unreliable tmux discovery");
            return;
        }

        let discovered_tmux_names: HashSet<String> = listed_tmux_names.iter().cloned().collect();
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
            let previous_state = summary.state;
            self.emit_missing_tmux_events(
                summary,
                previous_state,
                SUMMARY_CAUSE_STARTUP_MISSING_TMUX,
            );
        }
    }

    async fn summary_for_missing_tracked_handle(
        &self,
        handle: &ActorHandle,
        cached: Option<&SessionSummary>,
    ) -> SessionSummary {
        let (tx, rx) = oneshot::channel();
        if handle
            .cmd_tx
            .send(SessionCommand::GetSummary(tx))
            .await
            .is_ok()
        {
            if let Ok(Ok(summary)) = tokio::time::timeout(PROCESS_EXIT_SUMMARY_TIMEOUT, rx).await {
                return summary;
            }
        }

        cached.cloned().unwrap_or_else(|| {
            self.build_placeholder_summary(&handle.session_id, &handle.tmux_name)
        })
    }

    fn mark_missing_tmux_summary(summary: SessionSummary) -> SessionSummary {
        summary.into_missing_tmux_stale(SUMMARY_CAUSE_TMUX_RECONCILE_MISSING)
    }

    async fn missing_tracked_handles(
        &self,
        listed_tmux_names: &HashSet<String>,
    ) -> Vec<ActorHandle> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|handle| !listed_tmux_names.contains(&handle.tmux_name))
            .cloned()
            .collect()
    }

    async fn stale_summaries_for_missing_tracked_handles(
        &self,
        missing_handles: &[ActorHandle],
    ) -> Vec<MissingTrackedSessionSummary> {
        let cached_summaries = self.summary_cache.read().await.clone();
        let mut stale_summaries = Vec::with_capacity(missing_handles.len());
        for handle in missing_handles {
            let summary = self
                .summary_for_missing_tracked_handle(
                    handle,
                    cached_summaries.get(&handle.session_id),
                )
                .await;
            stale_summaries.push(MissingTrackedSessionSummary {
                session_id: handle.session_id.clone(),
                previous_state: summary.state,
                summary: Self::mark_missing_tmux_summary(summary),
            });
        }
        stale_summaries
    }

    async fn remove_still_missing_tracked_handles(
        &self,
        missing_handles: &[ActorHandle],
        listed_tmux_names: &HashSet<String>,
    ) -> Vec<ActorHandle> {
        let mut sessions = self.sessions.write().await;
        let mut removed = Vec::with_capacity(missing_handles.len());
        for handle in missing_handles {
            let still_missing = sessions
                .get(&handle.session_id)
                .map(|current| !listed_tmux_names.contains(&current.tmux_name))
                .unwrap_or(false);
            if still_missing {
                if let Some(handle) = sessions.remove(&handle.session_id) {
                    removed.push(handle);
                }
            }
        }
        crate::metrics::set_active_sessions(sessions.len());
        removed
    }

    async fn forget_removed_tracked_summary_cache(&self, removed_ids: &HashSet<String>) {
        let mut cache = self.summary_cache.write().await;
        for session_id in removed_ids {
            cache.remove(session_id);
        }
    }

    async fn retain_removed_tracked_stale_summaries(
        &self,
        stale_summaries: &[MissingTrackedSessionSummary],
        removed_ids: &HashSet<String>,
    ) {
        let mut stale = self.stale_sessions.write().await;
        for stale_summary in stale_summaries {
            if !removed_ids.contains(&stale_summary.session_id) {
                continue;
            }
            stale.retain(|existing| {
                existing.session_id != stale_summary.summary.session_id
                    && existing.tmux_name != stale_summary.summary.tmux_name
            });
            stale.push(stale_summary.summary.clone());
        }
    }

    async fn shutdown_removed_tracked_handles(&self, removed_handles: Vec<ActorHandle>) {
        for handle in removed_handles {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
        }
    }

    fn emit_removed_tracked_missing_events(
        &self,
        stale_summaries: Vec<MissingTrackedSessionSummary>,
        removed_ids: &HashSet<String>,
    ) {
        for stale_summary in stale_summaries {
            if removed_ids.contains(&stale_summary.session_id) {
                self.emit_missing_tmux_events(
                    stale_summary.summary,
                    stale_summary.previous_state,
                    SUMMARY_CAUSE_TMUX_RECONCILE_MISSING,
                );
            }
        }
    }

    async fn reconcile_tracked_sessions_after_discovery(
        &self,
        discovery_reliable: bool,
        listed_tmux_names: &[String],
    ) {
        if !discovery_reliable {
            return;
        }

        let listed_tmux_names = listed_tmux_names.iter().cloned().collect::<HashSet<_>>();
        let missing_handles = self.missing_tracked_handles(&listed_tmux_names).await;
        if missing_handles.is_empty() {
            return;
        }

        let stale_summaries = self
            .stale_summaries_for_missing_tracked_handles(&missing_handles)
            .await;
        let removed_handles = self
            .remove_still_missing_tracked_handles(&missing_handles, &listed_tmux_names)
            .await;
        if removed_handles.is_empty() {
            return;
        }

        let removed_ids = removed_handles
            .iter()
            .map(|handle| handle.session_id.clone())
            .collect::<HashSet<_>>();
        self.forget_removed_tracked_summary_cache(&removed_ids)
            .await;
        self.retain_removed_tracked_stale_summaries(&stale_summaries, &removed_ids)
            .await;

        self.shutdown_removed_tracked_handles(removed_handles).await;
        self.emit_removed_tracked_missing_events(stale_summaries, &removed_ids);

        self.persist_registry().await;
    }

    fn emit_missing_tmux_events(
        &self,
        summary: SessionSummary,
        previous_state: SessionState,
        reason: &'static str,
    ) {
        let payload = SessionStatePayload {
            state: SessionState::Exited,
            previous_state,
            current_command: summary.current_command.clone(),
            state_evidence: crate::types::StateEvidence::new(reason),
            transport_health: TransportHealth::Disconnected,
            exit_reason: Some(reason.to_string()),
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
            reason: reason.to_string(),
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

    async fn resolve_adopt_session_identity(
        self: &Arc<Self>,
        tmux_name: &str,
        requested_session_id: Option<String>,
    ) -> Result<(String, Option<SessionSummary>), TmuxAdoptError> {
        if let Some(session_id) = requested_session_id {
            if self.sessions.read().await.contains_key(&session_id) {
                return Err(TmuxAdoptError::AlreadyTracked {
                    tmux_name: tmux_name.to_string(),
                    session_id,
                });
            }

            let stale = self
                .stale_summary_for_id(&session_id)
                .await
                .ok_or_else(|| TmuxAdoptError::StaleSessionNotFound {
                    session_id: session_id.clone(),
                })?;
            if stale.tmux_name != tmux_name {
                return Err(TmuxAdoptError::StaleSessionConflict {
                    session_id,
                    stale_tmux_name: stale.tmux_name,
                    requested_tmux_name: tmux_name.to_string(),
                });
            }
            return Ok((stale.session_id.clone(), Some(stale)));
        }

        let stale_matches = self.stale_summaries_for_tmux(tmux_name).await;
        match stale_matches.len() {
            0 => Ok((self.allocate_unique_session_id().await, None)),
            1 => {
                let stale = stale_matches.into_iter().next().expect("one stale match");
                Ok((stale.session_id.clone(), Some(stale)))
            }
            count => Err(TmuxAdoptError::AmbiguousTarget {
                tmux_name: tmux_name.to_string(),
                matches: count,
            }),
        }
    }

    fn build_adopted_summary(
        &self,
        session_id: &str,
        tmux_name: &str,
        stale_seed: Option<SessionSummary>,
        reason: &'static str,
    ) -> (SessionSummary, Option<RepoTheme>) {
        let mut summary = stale_seed
            .unwrap_or_else(|| self.build_placeholder_summary(session_id, tmux_name))
            .revive_from_stale(session_id, tmux_name, reason);
        let repo_theme = self.resolve_repo_theme_for_summary(&mut summary);
        (summary, repo_theme)
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
        self.reconcile_stale_sessions_after_discovery(listed.reliable, &listed.names)
            .await;
        self.reconcile_tracked_sessions_after_discovery(listed.reliable, &listed.names)
            .await;
        self.finish_tmux_discovery(listed.reliable, highest_numeric)
            .await;

        Ok(())
    }

    /// Explicitly adopt a tmux session that already exists outside swimmers,
    /// optionally reusing a stale swimmers session id when that binding is
    /// unambiguous.
    pub async fn adopt_tmux_session(
        self: &Arc<Self>,
        tmux_name: String,
        session_id: Option<String>,
    ) -> Result<AdoptedTmuxSession, TmuxAdoptError> {
        if tmux_name.is_empty() {
            return Err(TmuxAdoptError::EmptyTmuxName);
        }

        let _discovery_guard = self.discovery_lock.lock().await;
        if let Some(active_id) = self.active_session_id_for_tmux(&tmux_name).await {
            return Err(TmuxAdoptError::AlreadyTracked {
                tmux_name,
                session_id: active_id,
            });
        }

        let listed = self.list_tmux_session_names("manual_tmux_adopt").await;
        if !listed.reliable {
            return Err(TmuxAdoptError::DiscoveryUnavailable);
        }

        let target_matches = listed
            .names
            .iter()
            .filter(|name| *name == &tmux_name)
            .count();
        match target_matches {
            0 => {
                return Err(TmuxAdoptError::TargetNotFound { tmux_name });
            }
            1 => {}
            count => {
                return Err(TmuxAdoptError::AmbiguousTarget {
                    tmux_name,
                    matches: count,
                });
            }
        }

        let (adopt_session_id, stale_seed) = self
            .resolve_adopt_session_identity(&tmux_name, session_id)
            .await?;
        let reused_session_id = stale_seed.is_some();
        if reused_session_id {
            self.bump_id_counter_from_session_id(&adopt_session_id);
        }

        let last_activity_override = match stale_seed.as_ref() {
            Some(summary) => Some(summary.last_activity_at),
            None => self.persisted_last_activity(&adopt_session_id).await,
        };
        let batch = match stale_seed
            .as_ref()
            .and_then(|summary| summary.batch.clone())
        {
            Some(batch) => Some(batch),
            None => self.persisted_batch(&adopt_session_id).await,
        };

        let handle = crate::session::actor::SessionActor::spawn(
            adopt_session_id.clone(),
            tmux_name.clone(),
            true,
            None,
            None,
            None,
            self.config.clone(),
            last_activity_override,
            batch,
        )
        .map_err(|error| TmuxAdoptError::SpawnFailed {
            tmux_name: tmux_name.clone(),
            message: error.to_string(),
        })?;

        if !self
            .insert_discovered_handle(adopt_session_id.clone(), tmux_name.clone(), handle)
            .await
        {
            let active_id = self
                .active_session_id_for_tmux(&tmux_name)
                .await
                .unwrap_or_else(|| "<unknown>".to_string());
            return Err(TmuxAdoptError::AlreadyTracked {
                tmux_name,
                session_id: active_id,
            });
        }

        let reason = if reused_session_id {
            "manual_tmux_reattach"
        } else {
            "manual_tmux_adopt"
        };
        let (summary, repo_theme) =
            self.build_adopted_summary(&adopt_session_id, &tmux_name, stale_seed, reason);
        {
            let mut stale = self.stale_sessions.write().await;
            stale.retain(|existing| {
                existing.session_id != adopt_session_id && existing.tmux_name != tmux_name
            });
        }
        {
            let mut cache = self.summary_cache.write().await;
            cache.insert(adopt_session_id.clone(), summary.clone());
        }
        {
            let sessions = self.sessions.read().await;
            crate::metrics::set_active_sessions(sessions.len());
        }

        let _ = self.lifecycle_tx.send(LifecycleEvent::Created {
            session_id: adopt_session_id,
            summary: summary.clone(),
            reason: reason.to_string(),
            repo_theme: repo_theme.clone(),
        });

        self.persist_registry().await;

        Ok(AdoptedTmuxSession {
            session: summary,
            repo_theme,
            reused_session_id,
        })
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
        let start_cwd = cwd.or_else(current_working_dir);
        let mut initial_request = normalize_initial_request(initial_request);
        let tmux_name = self.allocate_tmux_name(name);
        let session_id = self.allocate_unique_session_id().await;

        if let Some(dir) = start_cwd.as_deref() {
            if !Path::new(dir).is_dir() {
                return Err(anyhow::anyhow!(
                    "session cwd does not exist or is not a directory: {dir}"
                ));
            }
        }

        info!(session_id = %session_id, tmux_name = %tmux_name, "creating new session");

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
        start_cwd: Option<&str>,
        initial_tool: Option<&str>,
        batch: Option<SessionBatchMembership>,
    ) -> SessionSummary {
        let mut summary = self.build_placeholder_summary(session_id, tmux_name);
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

        enum SummaryCollectOutcome {
            Live(SessionSummary),
            Fallback(SessionSummary),
            Exited(String),
            Missing,
        }

        let futs: Vec<_> = handles_with_cached
            .into_iter()
            .map(|(handle, cached)| async move {
                let (tx, rx) = oneshot::channel();
                if handle
                    .cmd_tx
                    .send(SessionCommand::GetSummary(tx))
                    .await
                    .is_err()
                {
                    warn!(session_id = %handle.session_id, "actor summary command channel closed");
                    return match cached {
                        Some(summary) => {
                            let reason = SummaryFallbackReason::ChannelClosed;
                            crate::metrics::increment_summary_fallback(reason);
                            SummaryCollectOutcome::Fallback(
                                summary.into_cached_collection_fallback(reason),
                            )
                        }
                        None => {
                            crate::metrics::increment_summary_fallback(
                                SummaryFallbackReason::Missing,
                            );
                            SummaryCollectOutcome::Missing
                        }
                    };
                }
                match tokio::time::timeout(timeout, rx).await {
                    Ok(Ok(summary)) if summary.state != SessionState::Exited => {
                        SummaryCollectOutcome::Live(summary)
                    }
                    Ok(Ok(summary)) => SummaryCollectOutcome::Exited(summary.session_id),
                    Ok(Err(_)) => {
                        warn!(session_id = %handle.session_id, "actor dropped summary reply");
                        match cached {
                            Some(summary) => {
                                let reason = SummaryFallbackReason::Dropped;
                                crate::metrics::increment_summary_fallback(reason);
                                SummaryCollectOutcome::Fallback(
                                    summary.into_cached_collection_fallback(reason),
                                )
                            }
                            None => {
                                crate::metrics::increment_summary_fallback(
                                    SummaryFallbackReason::Missing,
                                );
                                SummaryCollectOutcome::Missing
                            }
                        }
                    }
                    Err(_) => {
                        warn!(session_id = %handle.session_id, "summary request timed out");
                        match cached {
                            Some(summary) => {
                                let reason = SummaryFallbackReason::Timeout;
                                crate::metrics::increment_summary_fallback(reason);
                                SummaryCollectOutcome::Fallback(
                                    summary.into_cached_collection_fallback(reason),
                                )
                            }
                            None => {
                                crate::metrics::increment_summary_fallback(
                                    SummaryFallbackReason::Missing,
                                );
                                SummaryCollectOutcome::Missing
                            }
                        }
                    }
                }
            })
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
        let tmux_names =
            tmux_names_requiring_active_pane_lookup(summaries.iter(), &thought_snapshots);
        let active_pane_session_ids = self
            .active_pane_session_ids_cached(&tmux_names, "list_sessions")
            .await;
        for summary in &mut summaries {
            let active_pane_session_id = if thought_snapshots.contains_key(&summary.session_id)
                || summary.tmux_name.is_empty()
            {
                None
            } else {
                active_pane_session_ids.get(&summary.tmux_name).cloned()
            };
            if let Some(thought_data) = thought_snapshot_for_summary(
                summary,
                active_pane_session_id.as_deref(),
                &thought_snapshots,
            ) {
                merge_summary_with_thought_snapshot(summary, thought_data);
            }
        }

        // Resolve per-repo theme IDs after the thought merge so the cwd is
        // the actor-reported value.
        for summary in &mut summaries {
            self.resolve_repo_theme_for_summary(summary);
        }

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

    async fn collect_session_snapshots_with_timeout(&self, timeout: Duration) -> Vec<SessionInfo> {
        let handles: Vec<ActorHandle> = {
            let sessions = self.sessions.read().await;
            sessions.values().cloned().collect()
        };
        let thought_snapshots = self.thought_snapshots.read().await.clone();

        let futs: Vec<_> = handles
            .into_iter()
            .map(|handle| async move {
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
                    return None;
                }

                let (summary, snapshot) = tokio::join!(
                    tokio::time::timeout(timeout, sum_rx),
                    tokio::time::timeout(timeout, snap_rx)
                );
                let summary: Option<SessionSummary> = match summary {
                    Ok(Ok(s)) => Some(s),
                    _ => None,
                };
                let snapshot: Option<TerminalSnapshot> = match snapshot {
                    Ok(Ok(s)) => Some(s),
                    _ => None,
                };

                summary.map(|summary| {
                    let replay_text = snapshot
                        .map(|s| {
                            // Take last ~500 chars of screen text.
                            let chars: Vec<char> = s.screen_text.chars().collect();
                            let start = chars.len().saturating_sub(500);
                            chars[start..].iter().collect()
                        })
                        .unwrap_or_default();
                    (summary, replay_text)
                })
            })
            .collect();

        let summaries_with_replay: Vec<(SessionSummary, String)> = futures::future::join_all(futs)
            .await
            .into_iter()
            .flatten()
            .collect();

        let tmux_names = tmux_names_requiring_active_pane_lookup(
            summaries_with_replay.iter().map(|(summary, _)| summary),
            &thought_snapshots,
        );
        let active_pane_session_ids = self
            .active_pane_session_ids_cached(&tmux_names, "collect_session_infos")
            .await;

        let mut infos = Vec::with_capacity(summaries_with_replay.len());
        for (summary, replay_text) in summaries_with_replay {
            let session_id = summary.session_id.clone();
            let active_pane_session_id = if thought_snapshots.contains_key(&summary.session_id)
                || summary.tmux_name.is_empty()
                || summary.state == crate::types::SessionState::Exited
            {
                None
            } else {
                active_pane_session_ids.get(&summary.tmux_name).cloned()
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
                commit_candidate: thought_data
                    .map(|t| t.commit_candidate)
                    .unwrap_or(summary.commit_candidate),
                action_cues: thought_data
                    .map(|t| t.action_cues.clone())
                    .unwrap_or_else(|| summary.action_cues.clone()),
                thought: thought_data
                    .and_then(|t| t.thought.clone())
                    .or_else(|| summary.thought.clone()),
                thought_updated_at: thought_data
                    .map(|t| t.updated_at)
                    .or(summary.thought_updated_at),
                objective_fingerprint: thought_data.and_then(|t| t.objective_fingerprint.clone()),
                token_count: thought_data
                    .map(|t| t.token_count)
                    .unwrap_or(summary.token_count),
                context_limit: thought_data
                    .map(|t| t.context_limit)
                    .unwrap_or(summary.context_limit),
                last_activity_at: summary.last_activity_at,
            });
        }

        infos
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
        SessionSummary::placeholder(session_id, tmux_name, Utc::now())
    }
}

async fn kill_tmux_session(tmux_name: &str) -> anyhow::Result<()> {
    let target = exact_session_target(tmux_name);
    let output = run_bounded_tmux_command(
        "tmux",
        &["kill-session", "-t", &target],
        TMUX_KILL_SESSION_TIMEOUT,
        "kill-session",
    )
    .await?;

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
        .filter(|(_, first_seen)| now.duration_since(**first_seen) >= grace)
        .map(|(session_id, _)| session_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ActionCueConfidence, ActionCueKind, ActionCueSource, ActionCueStatus};
    use chrono::{DateTime, Utc};
    use std::iter::FromIterator;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    fn test_summary(session_id: &str, state: SessionState) -> SessionSummary {
        let mut summary = SessionSummary::live(
            session_id,
            format!("tmux-{session_id}"),
            state,
            Some("cargo test".to_string()),
            Default::default(),
            "/tmp/project",
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        );
        summary.rest_state = fallback_rest_state(state, ThoughtState::Holding);
        summary
    }

    #[test]
    fn summary_lifecycle_helpers_use_shared_fallback_causes() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));

        let placeholder = supervisor.build_placeholder_summary("sess_1", "work");
        assert_eq!(
            placeholder.state_evidence.cause,
            crate::types::SUMMARY_CAUSE_SUPERVISOR_PLACEHOLDER
        );
        assert_eq!(placeholder.transport_health, TransportHealth::Healthy);
        assert!(!placeholder.is_stale);

        let missing = SessionSupervisor::mark_missing_tmux_summary(placeholder);
        assert_eq!(
            missing.state_evidence.cause,
            SUMMARY_CAUSE_TMUX_RECONCILE_MISSING
        );
        assert_eq!(missing.state, SessionState::Exited);
        assert_eq!(missing.transport_health, TransportHealth::Disconnected);
        assert!(missing.is_stale);
    }

    fn commit_ready_cue() -> ActionCue {
        ActionCue {
            kind: ActionCueKind::CommitReady,
            status: ActionCueStatus::Active,
            source: ActionCueSource::Transcript,
            confidence: ActionCueConfidence::Deterministic,
            evidence: ActionCue::expected_evidence(ActionCueKind::CommitReady)
                .iter()
                .map(|item| item.to_string())
                .collect(),
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

    async fn spawn_hung_summary_handle(session_id: &str, tmux_name: &str) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(session_id, tmux_name, cmd_tx);
        tokio::spawn(async move {
            let mut held_replies = Vec::new();
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        held_replies.push(reply);
                    }
                    SessionCommand::Shutdown => break,
                    _ => {}
                }
            }
        });
        handle
    }

    async fn spawn_observed_hung_summary_handle(
        session_id: &str,
        tmux_name: &str,
        observed_tx: mpsc::UnboundedSender<String>,
    ) -> ActorHandle {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let handle = ActorHandle::test_handle(session_id, tmux_name, cmd_tx);
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let mut held_replies = Vec::new();
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    SessionCommand::GetSummary(reply) => {
                        let _ = observed_tx.send(session_id.clone());
                        held_replies.push(reply);
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

    fn test_path_with_prepend(
        bin_dir: &std::path::Path,
        original_path: Option<&std::ffi::OsStr>,
    ) -> std::ffi::OsString {
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = original_path {
            entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
        }
        for system_dir in ["/bin", "/usr/bin"] {
            let system_dir = std::path::Path::new(system_dir);
            if system_dir.is_dir()
                && !entries
                    .iter()
                    .any(|entry| std::path::Path::new(entry) == system_dir)
            {
                entries.push(system_dir.as_os_str().to_os_string());
            }
        }
        std::env::join_paths(entries).expect("path")
    }

    fn prepend_test_path(bin_dir: &std::path::Path, original_path: Option<&std::ffi::OsStr>) {
        std::env::set_var("PATH", test_path_with_prepend(bin_dir, original_path));
    }

    fn install_fake_tmux(script: &str) -> (tempfile::TempDir, Option<std::ffi::OsString>) {
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        write_executable(&bin_dir.join("tmux"), script);
        let original_path = std::env::var_os("PATH");
        prepend_test_path(&bin_dir, original_path.as_deref());
        (dir, original_path)
    }

    fn restore_test_path(original_path: Option<std::ffi::OsString>) {
        if let Some(value) = original_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
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

    #[tokio::test]
    async fn query_tmux_active_pane_session_ids_uses_list_panes_and_supports_numeric_names() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        let command_file = dir.path().join("tmux-command.txt");
        write_executable(
            &bin_dir.join("tmux"),
            &format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" > \"{}\"\nsep=$(printf '\\037')\nprintf '0%s1%s1%s0.0:%%1\\n' \"$sep\" \"$sep\" \"$sep\"\nprintf 'work%s0%s1%s1.0:%%9\\n' \"$sep\" \"$sep\" \"$sep\"\nprintf 'work%s1%s1%s1.1:%%2\\n' \"$sep\" \"$sep\" \"$sep\"\n",
                command_file.display()
            ),
        );
        let original_path = std::env::var_os("PATH");
        prepend_test_path(&bin_dir, original_path.as_deref());

        let requested = HashSet::from_iter(["0".to_string(), "work".to_string()]);
        let all = query_all_active_pane_session_ids()
            .await
            .expect("active pane session ids");
        let pane_ids = filter_active_panes_to_requested(&all, &requested);
        assert_eq!(pane_ids.get("0").map(String::as_str), Some("tmux:0:0.0:%1"));
        assert_eq!(
            pane_ids.get("work").map(String::as_str),
            Some("tmux:work:1.1:%2")
        );
        assert_eq!(
            std::fs::read_to_string(&command_file).expect("command file"),
            "list-panes\n"
        );

        restore_test_path(original_path);
    }

    #[test]
    fn tmux_query_command_scrubs_tmux_env_vars() {
        let command = tmux_query_command(&["list-sessions", "-F", "#{session_name}"]);

        let tmux_value = command
            .as_std()
            .get_envs()
            .find_map(|(key, value)| (key == std::ffi::OsStr::new("TMUX")).then_some(value));
        assert_eq!(tmux_value, Some(None));

        let tmux_pane_value = command
            .as_std()
            .get_envs()
            .find_map(|(key, value)| (key == std::ffi::OsStr::new("TMUX_PANE")).then_some(value));
        assert_eq!(tmux_pane_value, Some(None));
    }

    #[tokio::test]
    async fn bounded_tmux_command_times_out_non_returning_fake_tmux() {
        let dir = tempdir().expect("tempdir");
        let fake_tmux = dir.path().join("tmux");
        write_executable(
            &fake_tmux,
            "#!/bin/sh\nif [ -x /bin/sleep ]; then exec /bin/sleep 10; fi\nexec sleep 10\n",
        );

        let started = Instant::now();
        let err = run_bounded_tmux_command(
            fake_tmux.as_os_str(),
            &["list-sessions", "-F", "#{session_name}"],
            Duration::from_millis(25),
            "test-hanging-list-sessions",
        )
        .await
        .expect_err("hanging tmux should time out");

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "bounded tmux helper should not wait for the fake tmux sleep"
        );
        assert!(
            err.to_string().contains("timed out after 25ms"),
            "timeout error should mention the bounded wait: {err:#}"
        );
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
            (SpawnTool::Grok, "Grok", 128_000),
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

    fn prompt_file_from_spawn_command(command: &str) -> PathBuf {
        let prefix = "prompt_file='";
        let suffix = "'; if prompt=\"$(cat \"$prompt_file\")\"; then rm -f \"$prompt_file\"; if command -v caam >/dev/null 2>&1; then caam run codex -- \"$prompt\" || { echo 'swimmers: caam codex launch failed; falling back to raw codex' >&2; if command -v codex-raw >/dev/null 2>&1; then codex-raw \"$prompt\"; else command codex \"$prompt\"; fi; }; else command codex \"$prompt\"; fi; else rm -f \"$prompt_file\"; echo 'swimmers: failed to read initial request' >&2; false; fi";
        assert!(command.starts_with(prefix), "unexpected command: {command}");
        assert!(command.ends_with(suffix), "unexpected command: {command}");
        PathBuf::from(&command[prefix.len()..command.len() - suffix.len()])
    }

    fn grok_prompt_file_from_spawn_command(command: &str) -> PathBuf {
        let prefix = "prompt_file='";
        let suffix = "'; if [ -r \"$prompt_file\" ]; then";
        assert!(command.starts_with(prefix), "unexpected command: {command}");
        let Some(end) = command.find(suffix) else {
            panic!("unexpected command: {command}");
        };
        PathBuf::from(&command[prefix.len()..end])
    }

    fn spawn_command_test_shell() -> &'static str {
        if cfg!(unix) {
            "/bin/sh"
        } else {
            "sh"
        }
    }

    #[test]
    fn build_spawn_tool_command_uses_prompt_file_for_grok_initial_request() {
        let prompt = "investigate Grok launch\nwithout argv prompt leaks";
        let command = build_spawn_tool_command_with_launcher(
            crate::types::SpawnTool::Grok,
            Some("/tmp/repos/swim mer's"),
            Some(prompt),
            SpawnToolLauncher::with_program_override(
                crate::types::SpawnTool::Grok,
                Some(std::ffi::OsString::from("/tmp/bin/grok wrapper")),
            ),
        );
        let prompt_path = grok_prompt_file_from_spawn_command(&command);

        assert!(!command.contains("investigate Grok launch"));
        assert!(!command.contains('\n'));
        assert!(command.contains("'/tmp/bin/grok wrapper' --prompt-file \"$prompt_file\""));
        assert!(command.contains("--cwd '/tmp/repos/swim mer'\\''s'"));
        assert!(command.contains("--always-approve --no-alt-screen"));
        assert!(!command.contains("--session-id"));
        assert!(!command.contains("--output-format"));
        assert!(!command.contains("--max-turns"));
        assert_eq!(
            std::fs::read_to_string(&prompt_path).expect("prompt file"),
            prompt
        );
        let _ = std::fs::remove_file(prompt_path);
    }

    #[tokio::test]
    async fn create_session_cleans_grok_prompt_file_when_spawn_rejects_cwd() {
        let dir = tempdir().expect("tempdir");
        let missing_cwd = dir.path().join("missing");
        let marker = format!("grok prompt cleanup marker {}", Uuid::new_v4());
        assert!(
            !prompt_dir_contains(&marker),
            "test marker should not exist before session creation"
        );

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let err = supervisor
            .create_session(
                None,
                Some(missing_cwd.to_string_lossy().into_owned()),
                Some(crate::types::SpawnTool::Grok),
                Some(marker.clone()),
            )
            .await
            .expect_err("invalid cwd should reject session creation");

        assert!(err.to_string().contains("session cwd does not exist"));
        assert!(
            !prompt_dir_contains(&marker),
            "failed session spawn must remove Grok prompt file"
        );
    }

    #[test]
    fn build_spawn_tool_command_uses_grok_override_for_no_prompt_launch() {
        let command = build_spawn_tool_command_with_launcher(
            crate::types::SpawnTool::Grok,
            Some("/tmp/repos/swimmers"),
            None,
            SpawnToolLauncher::with_program_override(
                crate::types::SpawnTool::Grok,
                Some(std::ffi::OsString::from("/tmp/bin/grok wrapper")),
            ),
        );

        assert_eq!(command, "'/tmp/bin/grok wrapper'");
    }

    fn prompt_dir_contains(marker: &str) -> bool {
        let dir = std::env::temp_dir().join("swimmers-initial-requests");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            std::fs::read_to_string(entry.path())
                .map(|contents| contents.contains(marker))
                .unwrap_or(false)
        })
    }

    #[test]
    fn grok_prompt_command_removes_prompt_file_after_success() {
        let temp = tempdir().expect("tempdir");
        let grok = temp.path().join("grok");
        let captured_prompt = temp.path().join("captured-prompt.txt");
        let restricted_path = temp.path().join("restricted-path");
        std::fs::create_dir_all(&restricted_path).expect("restricted path");
        let capture_script = format!(
            "#!/bin/sh\nprompt_file=\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--prompt-file\" ]; then shift; prompt_file=$1; fi\n  shift || true\ndone\nif [ -n \"$prompt_file\" ]; then\n  IFS= read -r prompt < \"$prompt_file\" || true\n  printf '%s' \"$prompt\" > {}\nfi\nexit 0\n",
            shell_single_quote(&captured_prompt.to_string_lossy())
        );
        write_executable(&grok, &capture_script);

        let command = build_spawn_tool_command_with_launcher(
            crate::types::SpawnTool::Grok,
            Some(temp.path().to_str().expect("utf8 tempdir")),
            Some("private Grok prompt"),
            SpawnToolLauncher::with_program_override(
                crate::types::SpawnTool::Grok,
                Some(grok.into_os_string()),
            ),
        );
        let prompt_path = grok_prompt_file_from_spawn_command(&command);
        let status = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .env("PATH", &restricted_path)
            .status()
            .expect("run Grok spawn command");

        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(captured_prompt).expect("captured prompt"),
            "private Grok prompt"
        );
        assert!(!prompt_path.exists(), "prompt file should be removed");
    }

    #[test]
    fn grok_prompt_command_removes_prompt_file_after_failure() {
        let temp = tempdir().expect("tempdir");
        let grok = temp.path().join("grok");
        let restricted_path = temp.path().join("restricted-path");
        std::fs::create_dir_all(&restricted_path).expect("restricted path");
        write_executable(&grok, "#!/bin/sh\nexit 42\n");

        let command = build_spawn_tool_command_with_launcher(
            crate::types::SpawnTool::Grok,
            None,
            Some("private Grok prompt"),
            SpawnToolLauncher::with_program_override(
                crate::types::SpawnTool::Grok,
                Some(grok.into_os_string()),
            ),
        );
        let prompt_path = grok_prompt_file_from_spawn_command(&command);
        let status = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .env("PATH", &restricted_path)
            .status()
            .expect("run Grok spawn command");

        assert!(!status.success());
        assert!(!prompt_path.exists(), "prompt file should be removed");
    }

    #[test]
    fn build_spawn_tool_command_uses_prompt_file_for_codex_initial_request() {
        let prompt = "investigate tmux startup\nthen inspect imports";
        let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
        let prompt_path = prompt_file_from_spawn_command(&command);

        assert!(!command.contains("investigate tmux startup"));
        assert!(!command.contains('\n'));
        assert!(command.contains("rm -f \"$prompt_file\""));
        assert_eq!(
            std::fs::read_to_string(&prompt_path).expect("prompt file"),
            prompt
        );
        let _ = std::fs::remove_file(prompt_path);
    }

    #[test]
    fn codex_prompt_command_reports_missing_prelaunch_prompt_file() {
        let command =
            build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some("lost prompt"));
        let prompt_path = prompt_file_from_spawn_command(&command);
        std::fs::remove_file(&prompt_path).expect("remove prompt file before command runs");
        let output = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .output()
            .expect("run spawn command");

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("swimmers: failed to read initial request"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!prompt_path.exists());
    }

    #[test]
    fn delayed_prelaunch_cleanup_allows_codex_prompt_command_to_read_file() {
        let temp = tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let captured_prompt = temp.path().join("captured-prompt.txt");
        let capture_script = format!(
            "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
            shell_single_quote(&captured_prompt.to_string_lossy())
        );
        write_executable(&bin_dir.join("codex"), &capture_script);
        let test_path = test_path_with_prepend(&bin_dir, None);

        let prompt = "prompt survives delayed cleanup";
        let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
        let prompt_path = prompt_file_from_spawn_command(&command);
        schedule_prelaunch_file_cleanup_after(
            vec![prompt_path.clone()],
            Duration::from_millis(100),
        );

        assert!(
            prompt_path.exists(),
            "cleanup must not remove the handoff immediately"
        );
        let output = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .env("PATH", test_path)
            .output()
            .expect("run spawn command");

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(captured_prompt).expect("captured prompt"),
            prompt
        );
        assert!(
            !prompt_path.exists(),
            "prompt file should be removed by the command"
        );
    }

    #[test]
    fn delayed_prelaunch_cleanup_removes_unread_prompt_file() {
        let temp = tempdir().expect("tempdir");
        let prompt_path = temp.path().join("orphaned-prompt.txt");
        std::fs::write(&prompt_path, "orphaned prompt").expect("prompt file");

        schedule_prelaunch_file_cleanup_after(vec![prompt_path.clone()], Duration::from_millis(10));

        for _ in 0..50 {
            if !prompt_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !prompt_path.exists(),
            "orphaned prompt file should be removed"
        );
    }

    #[test]
    fn build_spawn_tool_command_prompt_file_preserves_quote_sensitive_prompt() {
        let prompt = "fix Bob's tmux startup with \"fresh eyes\"";
        let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
        let prompt_path = prompt_file_from_spawn_command(&command);

        assert!(!command.contains("Bob"));
        assert!(!command.contains("fresh eyes"));
        assert_eq!(
            std::fs::read_to_string(&prompt_path).expect("prompt file"),
            prompt
        );
        let _ = std::fs::remove_file(prompt_path);
    }

    #[test]
    fn wrap_spawn_tool_command_for_tmux_keeps_shell_after_tool_exits() {
        assert_eq!(
            wrap_spawn_tool_command_for_tmux("codex 'investigate tmux startup'"),
            "{ codex 'investigate tmux startup'; }; exec \"${SHELL:-/bin/sh}\""
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_prompt_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let prompt = "private prompt";
        let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
        let prompt_path = prompt_file_from_spawn_command(&command);
        let dir_mode = std::fs::metadata(prompt_path.parent().expect("prompt dir"))
            .expect("prompt dir metadata")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = std::fs::metadata(&prompt_path)
            .expect("prompt file metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
        let _ = std::fs::remove_file(prompt_path);
    }

    #[test]
    fn codex_prompt_command_reads_and_removes_prompt_file() {
        let temp = tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let captured_prompt = temp.path().join("captured-prompt.txt");
        let capture_script = format!(
            "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
            shell_single_quote(&captured_prompt.to_string_lossy())
        );
        write_executable(&bin_dir.join("codex"), &capture_script);

        let test_path = test_path_with_prepend(&bin_dir, None);

        let prompt = "fix shell quoting\nwithout leaking prompt text";
        let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
        let prompt_path = prompt_file_from_spawn_command(&command);
        let status = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .env("PATH", test_path)
            .status()
            .expect("run spawn command");

        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(captured_prompt).expect("captured prompt"),
            prompt
        );
        assert!(!prompt_path.exists(), "prompt file should be removed");
    }

    #[test]
    fn codex_prompt_command_prefers_caam_when_available() {
        let temp = tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let captured_args = temp.path().join("caam-args.txt");
        let caam_script = format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > {}\n",
            shell_single_quote(&captured_args.to_string_lossy())
        );
        write_executable(&bin_dir.join("caam"), &caam_script);
        write_executable(
            &bin_dir.join("codex"),
            "#!/usr/bin/env bash\necho 'codex fallback should not run' >&2\nexit 99\n",
        );

        let test_path = test_path_with_prepend(&bin_dir, None);

        let prompt = "route through caam";
        let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
        let prompt_path = prompt_file_from_spawn_command(&command);
        let status = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .env("PATH", test_path)
            .status()
            .expect("run spawn command");

        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(captured_args).expect("captured caam args"),
            "run\ncodex\n--\nroute through caam\n"
        );
        assert!(!prompt_path.exists(), "prompt file should be removed");
    }

    #[test]
    fn codex_prompt_command_falls_back_after_caam_failure() {
        let temp = tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        write_executable(&bin_dir.join("caam"), "#!/usr/bin/env bash\nexit 42\n");
        let captured_prompt = temp.path().join("fallback-prompt.txt");
        let fallback_script = format!(
            "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
            shell_single_quote(&captured_prompt.to_string_lossy())
        );
        write_executable(&bin_dir.join("codex-raw"), &fallback_script);

        let test_path = test_path_with_prepend(&bin_dir, None);

        let command =
            build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some("blocked"));
        let prompt_path = prompt_file_from_spawn_command(&command);
        let output = std::process::Command::new(spawn_command_test_shell())
            .arg("-c")
            .arg(&command)
            .env("PATH", test_path)
            .output()
            .expect("run spawn command");

        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("swimmers: caam codex launch failed; falling back to raw codex"),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(captured_prompt).expect("captured fallback prompt"),
            "blocked"
        );
        assert!(!prompt_path.exists(), "prompt file should be removed");
    }

    #[test]
    fn build_spawn_tool_command_inlines_claude_initial_request() {
        assert_eq!(
            build_spawn_tool_command(
                crate::types::SpawnTool::Claude,
                None,
                Some("investigate tmux startup")
            ),
            "claude 'investigate tmux startup'"
        );
        assert!(spawn_tool_consumes_initial_request(
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
                false,
                Vec::new(),
                Utc::now(),
                ThoughtDeliveryState::default(),
                None,
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
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed_at: None,
                last_skill: None,
                objective_fingerprint: None,
                batch: None,
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
    async fn init_persistence_preserves_batch_membership_on_stale_sessions() {
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
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed_at: None,
                last_skill: None,
                objective_fingerprint: None,
                batch: Some(SessionBatchMembership {
                    id: "batch-auth".to_string(),
                    label: "auth-rebuild".to_string(),
                    index: 0,
                    total: 2,
                    created_at: Utc::now(),
                    prompt_excerpt: Some("auth-rebuild".to_string()),
                }),
                cwd: "/tmp".to_string(),
                last_activity_at: Utc::now(),
            }])
            .await;

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor.init_persistence(store).await;

        let stale = supervisor.stale_sessions.read().await;
        let batch = stale[0].batch.as_ref().expect("batch membership");
        assert_eq!(batch.id, "batch-auth");
        assert_eq!(batch.label, "auth-rebuild");
        assert_eq!(batch.index, 0);
        assert_eq!(batch.total, 2);
        assert_eq!(
            stale[0].state_evidence.cause,
            SUMMARY_CAUSE_PERSISTENCE_STALE
        );
        assert!(stale[0].state_evidence.observed_at.is_none());
        assert_eq!(
            stale[0].state_evidence.confidence,
            crate::types::StateConfidence::Low
        );
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
                false,
                Vec::new(),
                updated_at,
                ThoughtDeliveryState::default(),
                None,
                Some("obj-1".to_string()),
            )
            .await;

        let thoughts = supervisor.thought_snapshots.read().await;
        let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
        assert_eq!(snapshot.updated_at, updated_at);
        assert_eq!(snapshot.thought.as_deref(), Some("reading logs"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn supervisor_provider_coalesces_latest_thought_when_persist_queue_is_full() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let provider = SupervisorProvider::new_with_persist_queue_capacity(supervisor.clone(), 1);
        let first_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:01Z")
            .expect("timestamp should parse")
            .with_timezone(&Utc);
        let second_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:02Z")
            .expect("timestamp should parse")
            .with_timezone(&Utc);
        let third_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:03Z")
            .expect("timestamp should parse")
            .with_timezone(&Utc);

        assert!(provider.persist_thought(
            "sess_1",
            Some("first queued"),
            1,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            first_at,
            ThoughtDeliveryState {
                stream_instance_id: Some("stream-a".to_string()),
                emission_seq: 1,
            },
            None,
            Some("obj-1".to_string()),
        ));
        assert!(
            !provider.persist_thought(
                "sess_1",
                Some("second overflow"),
                2,
                192_000,
                ThoughtState::Active,
                ThoughtSource::Llm,
                RestState::Active,
                false,
                Vec::new(),
                second_at,
                ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 2,
                },
                None,
                Some("obj-2".to_string()),
            ),
            "queue-full writes should be accepted for coalesced persistence but reported as degraded"
        );
        assert!(
            !provider.persist_thought(
                "sess_1",
                Some("third latest"),
                3,
                192_000,
                ThoughtState::Active,
                ThoughtSource::Llm,
                RestState::Active,
                false,
                Vec::new(),
                third_at,
                ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 3,
                },
                None,
                Some("obj-3".to_string()),
            ),
            "overwriting an overflow slot remains a degraded durability path"
        );

        let pressure = supervisor.thought_persistence_backpressure_snapshot();
        assert_eq!(
            pressure.queue_capacity, 1,
            "snapshot must report the configured queue capacity, not the default"
        );
        assert_eq!(pressure.queue_depth, 1);
        assert_eq!(pressure.pending_count, 2);
        assert_eq!(pressure.overflow_slots, 1);
        assert_eq!(pressure.queue_full_count, 2);
        assert_eq!(pressure.coalesced_count, 1);
        assert_eq!(pressure.dropped_count, 0);

        assert!(
            supervisor
                .wait_for_pending_thought_persists(Duration::from_secs(1))
                .await,
            "queued and coalesced thought writes should drain"
        );

        let thoughts = supervisor.thought_snapshots.read().await;
        let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
        assert_eq!(snapshot.thought.as_deref(), Some("third latest"));
        assert_eq!(snapshot.token_count, 3);
        assert_eq!(snapshot.updated_at, third_at);
        assert_eq!(snapshot.delivery.emission_seq, 3);
        assert_eq!(
            snapshot.delivery.stream_instance_id.as_deref(),
            Some("stream-a")
        );
        drop(thoughts);

        let drained = supervisor.thought_persistence_backpressure_snapshot();
        assert_eq!(drained.pending_count, 0);
        assert_eq!(drained.overflow_slots, 0);
    }

    #[tokio::test]
    async fn persist_thought_retains_objective_shift_timestamp_until_next_shift() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let shifted_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
            .expect("timestamp should parse")
            .with_timezone(&Utc);
        let later_update = DateTime::parse_from_rfc3339("2026-03-08T14:00:09Z")
            .expect("timestamp should parse")
            .with_timezone(&Utc);

        supervisor
            .persist_thought(
                "sess_1",
                Some("reframed objective"),
                12,
                192_000,
                ThoughtState::Active,
                ThoughtSource::Llm,
                RestState::Active,
                false,
                Vec::new(),
                shifted_at,
                ThoughtDeliveryState::default(),
                Some(shifted_at),
                Some("obj-1".to_string()),
            )
            .await;
        supervisor
            .persist_thought(
                "sess_1",
                Some("continuing work"),
                14,
                192_000,
                ThoughtState::Active,
                ThoughtSource::Llm,
                RestState::Active,
                false,
                Vec::new(),
                later_update,
                ThoughtDeliveryState::default(),
                None,
                Some("obj-1".to_string()),
            )
            .await;

        let thoughts = supervisor.thought_snapshots.read().await;
        let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
        assert_eq!(snapshot.updated_at, later_update);
        assert_eq!(snapshot.objective_changed_at, Some(shifted_at));
        assert_eq!(snapshot.thought.as_deref(), Some("continuing work"));
    }

    #[tokio::test]
    async fn persist_registry_uses_actor_state_without_querying_tmux() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin");
        let command_file = dir.path().join("tmux-command.txt");
        write_executable(
            &bin_dir.join("tmux"),
            &format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" > \"{}\"\nexit 1\n",
                command_file.display()
            ),
        );
        let original_path = std::env::var_os("PATH");
        prepend_test_path(&bin_dir, original_path.as_deref());

        let store = FileStore::new(dir.path()).await.expect("file store");
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor.init_persistence(store.clone()).await;
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
            )
            .await;

        supervisor.persist_registry().await;
        restore_test_path(original_path);

        let persisted = store.load_sessions().await;
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].session_id, "sess-live");
        assert!(
            !command_file.exists(),
            "persist_registry should not shell out to tmux"
        );
    }

    #[tokio::test]
    async fn persist_registry_merges_direct_thought_snapshot_into_registry() {
        let dir = tempdir().expect("tempdir");
        let store = FileStore::new(dir.path()).await.expect("file store");
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor.init_persistence(store.clone()).await;
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
            )
            .await;

        let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        supervisor
            .persist_thought(
                "sess-live",
                Some("reading logs"),
                12,
                192_000,
                ThoughtState::Active,
                ThoughtSource::Llm,
                RestState::Active,
                true,
                Vec::new(),
                updated_at,
                ThoughtDeliveryState::default(),
                None,
                Some("obj-1".to_string()),
            )
            .await;

        supervisor.persist_registry().await;

        let persisted = store.load_sessions().await;
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].thought.as_deref(), Some("reading logs"));
        assert_eq!(persisted[0].thought_updated_at, Some(updated_at));
        assert_eq!(persisted[0].rest_state, RestState::Active);
        assert!(persisted[0].commit_candidate);
        assert_eq!(persisted[0].objective_fingerprint.as_deref(), Some("obj-1"));
    }

    #[test]
    fn thought_snapshot_for_summary_matches_active_tmux_pane() {
        let summary = SessionSummary {
            session_id: "sess_1".to_string(),
            tmux_name: "work".to_string(),
            state: SessionState::Idle,
            current_command: None,
            state_evidence: Default::default(),
            cwd: "/tmp".to_string(),
            tool: None,
            token_count: 0,
            context_limit: 0,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: crate::types::TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
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
                    commit_candidate: false,
                    action_cues: Vec::new(),
                    objective_changed_at: None,
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
                    commit_candidate: true,
                    action_cues: Vec::new(),
                    objective_changed_at: None,
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
            state_evidence: Default::default(),
            cwd: "/tmp".to_string(),
            tool: None,
            token_count: 0,
            context_limit: 0,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: crate::types::TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
        };

        let snapshots = HashMap::from([
            (
                "tmux:work:1.0:%1".to_string(),
                ThoughtSnapshot {
                    thought: Some("pane one".to_string()),
                    thought_state: ThoughtState::Holding,
                    thought_source: ThoughtSource::Llm,
                    rest_state: RestState::Drowsy,
                    commit_candidate: false,
                    action_cues: Vec::new(),
                    objective_changed_at: None,
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
                    commit_candidate: true,
                    action_cues: Vec::new(),
                    objective_changed_at: None,
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
    fn parse_active_pane_session_ids_preserves_tabs_in_session_names() {
        let stdout = b"work\tspace\x1f1\x1f1\x1f1.1:%2\nother\x1f1\x1f0\x1f1.0:%1\n";

        let panes = parse_active_pane_session_ids(stdout);

        assert_eq!(
            panes.get("work\tspace").map(String::as_str),
            Some("tmux:work\tspace:1.1:%2")
        );
        assert!(!panes.contains_key("other"));
    }

    #[test]
    fn active_pane_lookup_not_required_without_thought_snapshots() {
        let summaries = [
            test_summary("sess-live", SessionState::Idle),
            test_summary("sess-busy", SessionState::Busy),
        ];
        let snapshots = HashMap::new();

        let tmux_names = tmux_names_requiring_active_pane_lookup(summaries.iter(), &snapshots);

        assert!(tmux_names.is_empty());
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
                commit_candidate: true,
                action_cues: vec![commit_ready_cue()],
                objective_changed_at: Some(Utc::now()),
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
        assert_eq!(sessions[0].action_cues, vec![commit_ready_cue()]);
        assert!(sessions[0].objective_changed_at.is_some());
    }

    #[tokio::test]
    async fn startup_idle_session_only_sleeps_after_waiting_thought_snapshot() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let aged = DateTime::parse_from_rfc3339("2026-03-08T13:55:00Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        let mut summary = test_summary("sess-startup", SessionState::Idle);
        summary.rest_state = RestState::Drowsy;
        summary.last_activity_at = aged;
        supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;

        let sessions = supervisor.list_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-startup");
        assert!(sessions[0].thought.is_none());
        assert_eq!(sessions[0].thought_state, ThoughtState::Holding);
        assert_eq!(sessions[0].rest_state, RestState::Drowsy);
        assert_eq!(sessions[0].last_activity_at, aged);

        let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
            .expect("timestamp")
            .with_timezone(&Utc);
        supervisor
            .persist_thought(
                "sess-startup",
                Some("Need your approval to continue."),
                12,
                192_000,
                ThoughtState::Sleeping,
                ThoughtSource::CarryForward,
                RestState::Sleeping,
                false,
                Vec::new(),
                updated_at,
                ThoughtDeliveryState::default(),
                None,
                None,
            )
            .await;

        let sessions = supervisor.list_sessions().await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].thought.as_deref(),
            Some("Need your approval to continue.")
        );
        assert_eq!(sessions[0].thought_state, ThoughtState::Sleeping);
        assert_eq!(sessions[0].thought_source, ThoughtSource::CarryForward);
        assert_eq!(sessions[0].rest_state, RestState::Sleeping);
        assert_eq!(sessions[0].thought_updated_at, Some(updated_at));
        assert_eq!(sessions[0].last_activity_at, aged);
    }

    #[tokio::test]
    async fn list_sessions_merges_thought_snapshot_from_active_tmux_pane_batch_lookup() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, original_path) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sep=$(printf '\037')
    name=$(printf 'work\tspace')
    printf '%s%s0%s1%s1.0:%%1\n' "$name" "$sep" "$sep" "$sep"
    printf '%s%s1%s1%s1.1:%%2\n' "$name" "$sep" "$sep" "$sep"
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
        );

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let mut summary = test_summary("sess-live", SessionState::Idle);
        summary.tmux_name = "work\tspace".to_string();
        supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;
        supervisor.thought_snapshots.write().await.insert(
            "tmux:work\tspace:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 77,
                context_limit: 200_000,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        );

        let sessions = supervisor.list_sessions().await;

        restore_test_path(original_path);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].thought.as_deref(), Some("pane two"));
        assert_eq!(sessions[0].thought_state, ThoughtState::Active);
        assert_eq!(sessions[0].rest_state, RestState::Active);
        assert_eq!(sessions[0].token_count, 77);
    }

    #[tokio::test]
    async fn list_sessions_keeps_summary_when_active_tmux_pane_batch_lookup_fails() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, original_path) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
printf 'boom\n' >&2
exit 1
"#,
        );

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let mut summary = test_summary("sess-live", SessionState::Idle);
        summary.tmux_name = "work".to_string();
        supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;
        supervisor.thought_snapshots.write().await.insert(
            "tmux:work:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 77,
                context_limit: 200_000,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        );

        let sessions = supervisor.list_sessions().await;

        restore_test_path(original_path);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-live");
        assert_eq!(sessions[0].thought.as_deref(), None);
        assert_eq!(sessions[0].thought_state, ThoughtState::Holding);
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
    async fn list_sessions_keeps_cached_summary_when_live_reply_drops() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let mut summary = test_summary("sess-live", SessionState::Idle);
        summary.tmux_name = "tmux-live".to_string();
        supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;

        let initial = supervisor.list_sessions().await;
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].transport_health, TransportHealth::Healthy);

        supervisor
            .insert_test_handle(
                spawn_dropped_summary_handle("sess-live", "tmux-live", SessionState::Idle).await,
            )
            .await;

        let sessions = supervisor.list_sessions().await;

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-live");
        assert_eq!(sessions[0].tmux_name, "tmux-live");
        assert_eq!(sessions[0].transport_health, TransportHealth::Degraded);
        assert_eq!(
            sessions[0].state_evidence.cause,
            SummaryFallbackReason::Dropped
                .cached_fallback()
                .expect("dropped fallback cause")
                .0
        );
        assert!(sessions[0].state_evidence.observed_at.is_none());
        assert!(!sessions[0].is_stale);
    }

    #[tokio::test]
    async fn collect_live_summaries_keeps_cached_summary_when_live_reply_times_out() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        supervisor
            .insert_test_handle(
                spawn_summary_handle(test_summary("sess-timeout", SessionState::Busy)).await,
            )
            .await;

        let initial = supervisor
            .collect_live_summaries(Duration::from_millis(10))
            .await;
        assert_eq!(initial.len(), 1);

        supervisor
            .insert_test_handle(
                spawn_hung_summary_handle("sess-timeout", "tmux-sess-timeout").await,
            )
            .await;

        let sessions = supervisor
            .collect_live_summaries(Duration::from_millis(10))
            .await;

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sess-timeout");
        assert_eq!(sessions[0].transport_health, TransportHealth::Overloaded);
        assert_eq!(
            sessions[0].state_evidence.cause,
            SummaryFallbackReason::Timeout
                .cached_fallback()
                .expect("timeout fallback cause")
                .0
        );
        assert!(sessions[0].state_evidence.observed_at.is_none());
        assert!(!sessions[0].is_stale);
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
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
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
    async fn collect_session_snapshots_merges_thought_snapshot_from_active_tmux_pane_batch_lookup()
    {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (_dir, original_path) = install_fake_tmux(
            r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
    sep=$(printf '\037')
    name=$(printf 'work\tspace')
    printf '%s%s0%s1%s1.0:%%1\n' "$name" "$sep" "$sep" "$sep"
    printf '%s%s1%s1%s1.1:%%2\n' "$name" "$sep" "$sep" "$sep"
    ;;
  *)
    printf 'unexpected tmux command: %s\n' "${1-}" >&2
    exit 1
    ;;
esac
"#,
        );

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let mut summary = test_summary("sess-live", SessionState::Busy);
        summary.tmux_name = "work\tspace".to_string();
        supervisor
            .insert_test_handle(spawn_summary_handle(summary).await)
            .await;
        supervisor.thought_snapshots.write().await.insert(
            "tmux:work\tspace:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: Some("obj-pane".to_string()),
                token_count: 88,
                context_limit: 199_000,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        );

        let infos = supervisor.collect_session_snapshots().await;

        restore_test_path(original_path);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].session_id, "sess-live");
        assert_eq!(infos[0].thought.as_deref(), Some("pane two"));
        assert_eq!(infos[0].thought_state, ThoughtState::Active);
        assert_eq!(infos[0].rest_state, RestState::Active);
        assert_eq!(infos[0].objective_fingerprint.as_deref(), Some("obj-pane"));
        assert_eq!(infos[0].token_count, 88);
    }

    #[tokio::test]
    async fn collect_session_snapshots_fans_out_actor_requests_before_timeouts() {
        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();

        for session_id in ["sess-a", "sess-b", "sess-c"] {
            supervisor
                .insert_test_handle(
                    spawn_observed_hung_summary_handle(session_id, "", observed_tx.clone()).await,
                )
                .await;
        }
        drop(observed_tx);

        let collect = supervisor.collect_session_snapshots_with_timeout(Duration::from_secs(10));
        tokio::pin!(collect);
        let observations = async {
            let mut observed = Vec::new();
            for _ in 0..3 {
                observed.push(observed_rx.recv().await.expect("observed summary request"));
            }
            observed
        };
        tokio::pin!(observations);

        let observed = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                _ = &mut collect => panic!("hung actors should keep collection pending"),
                observed = &mut observations => observed,
            }
        })
        .await
        .expect("snapshot collection should request every actor before the first timeout");

        let observed: HashSet<_> = observed.into_iter().collect();
        let expected = HashSet::from_iter([
            "sess-a".to_string(),
            "sess-b".to_string(),
            "sess-c".to_string(),
        ]);
        assert_eq!(observed, expected);
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
    if [ "$cmd" = "new-session" ] && [ -n "${SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG:-}" ]; then
      printf '%s\n' "$@" > "${SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG}"
    fi
    while IFS= read -r line; do
      printf '%s\r\n' "$line"
    done
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_path}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_CWD:-/tmp/project}" ;;
      "#{pane_current_command}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_COMMAND:-codex}" ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
  send-keys)
    printf 'unexpected send-keys during spawn\n' >&2
    exit 9
    ;;
  kill-session)
    exit 0
    ;;
  capture-pane)
    printf 'captured pane\n'
    ;;
  list-sessions)
    if [ -f "${SWIMMERS_FAKE_TMUX_SESSIONS:-}" ]; then
      while IFS= read -r line || [ -n "$line" ]; do
        printf '%s\n' "$line"
      done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
    fi
    ;;
esac
"##,
        );

        let original_path = std::env::var_os("PATH");
        let original_cwd = std::env::var_os("SWIMMERS_FAKE_TMUX_CWD");
        let original_cmd = std::env::var_os("SWIMMERS_FAKE_TMUX_COMMAND");
        let original_new_session_log = std::env::var_os("SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG");
        let new_session_log = dir.path().join("new-session.log");
        prepend_test_path(&bin_dir, original_path.as_deref());
        std::env::set_var("SWIMMERS_FAKE_TMUX_CWD", dir.path());
        std::env::set_var("SWIMMERS_FAKE_TMUX_COMMAND", "codex");
        std::env::set_var("SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG", &new_session_log);

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

        assert_eq!(created.0.session_id, "sess_0");
        assert_eq!(created.0.tmux_name, "0");
        assert_eq!(created.0.tool.as_deref(), Some("Codex"));
        assert_eq!(created.0.cwd, dir.path().to_string_lossy());
        for _ in 0..20 {
            if new_session_log.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let new_session_log = std::fs::read_to_string(new_session_log).expect("new-session log");
        assert!(new_session_log.contains("new-session\n-s\n0\n-c\n"));
        assert!(new_session_log.contains("{ prompt_file="));
        assert!(new_session_log.contains("caam run codex -- \"$prompt\""));
        assert!(new_session_log.contains("falling back to raw codex"));
        assert!(new_session_log.contains("exec \"${SHELL:-/bin/sh}\""));
        assert!(!new_session_log.contains("investigate startup"));
        assert!(
            new_session_log
                .find("caam run codex -- \"$prompt\"")
                .expect("caam command")
                < new_session_log.find("codex-raw").expect("raw fallback"),
            "caam must be attempted before raw fallback"
        );
        supervisor
            .delete_session(
                &created.0.session_id,
                crate::config::SessionDeleteMode::DetachBridge,
            )
            .await
            .expect("cleanup session");

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_cwd {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_CWD", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_CWD"),
        }
        match original_cmd {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_COMMAND", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_COMMAND"),
        }
        match original_new_session_log {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG"),
        }
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
    done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
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
        let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
        prepend_test_path(&bin_dir, original_path.as_deref());
        std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

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
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
        }

        let sessions = supervisor.sessions.read().await;
        assert_eq!(sessions.len(), 2);
        assert!(sessions.values().any(|handle| handle.tmux_name == "11"));
        assert!(sessions
            .values()
            .any(|handle| handle.tmux_name == "workspace"));
    }

    #[tokio::test]
    async fn discover_tmux_sessions_reconciles_external_create_remove_and_restart() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let sessions_file;
        let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
        let (_dir, original_path) = install_fake_tmux(
            r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
    while IFS= read -r line || [ -n "$line" ]; do
      printf '%s\n' "$line"
    done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
    ;;
  list-panes)
    exit 0
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
        sessions_file = _dir.path().join("sessions.txt");
        std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        std::fs::write(&sessions_file, "alpha\nbeta\n").expect("initial sessions");
        supervisor
            .discover_tmux_sessions_with_reason("test_discovery")
            .await
            .expect("initial discover");
        let first_ids = {
            let sessions = supervisor.sessions.read().await;
            sessions
                .values()
                .map(|handle| (handle.tmux_name.clone(), handle.session_id.clone()))
                .collect::<HashMap<_, _>>()
        };
        assert_eq!(first_ids.len(), 2);
        let alpha_id = first_ids.get("alpha").expect("alpha id").clone();
        let beta_id = first_ids.get("beta").expect("beta id").clone();

        std::fs::write(&sessions_file, "beta\ngamma\n").expect("updated sessions");
        supervisor
            .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
            .await
            .expect("rediscover after remove/create");
        let after_remove = supervisor.list_sessions().await;
        assert_eq!(after_remove.len(), 2);
        assert!(after_remove
            .iter()
            .any(|summary| { summary.tmux_name == "beta" && summary.session_id == beta_id }));
        assert!(after_remove
            .iter()
            .any(|summary| summary.tmux_name == "gamma"));
        assert!(!after_remove
            .iter()
            .any(|summary| summary.tmux_name == "alpha"));
        {
            let stale = supervisor.stale_sessions.read().await;
            let alpha = stale
                .iter()
                .find(|summary| summary.tmux_name == "alpha")
                .expect("removed alpha should become stale");
            assert_eq!(alpha.session_id, alpha_id);
            assert_eq!(alpha.state, SessionState::Exited);
            assert!(alpha.is_stale);
            assert_eq!(alpha.transport_health, TransportHealth::Disconnected);
        }

        std::fs::write(&sessions_file, "alpha\nbeta\ngamma\n").expect("restarted sessions");
        supervisor
            .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
            .await
            .expect("rediscover after restart");
        let after_restart = supervisor.list_sessions().await;
        assert_eq!(after_restart.len(), 3);
        assert!(after_restart
            .iter()
            .any(|summary| { summary.tmux_name == "alpha" && summary.session_id == alpha_id }));

        supervisor
            .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
            .await
            .expect("dedup rediscover");
        let final_ids = {
            let sessions = supervisor.sessions.read().await;
            sessions
                .values()
                .map(|handle| (handle.tmux_name.clone(), handle.session_id.clone()))
                .collect::<HashMap<_, _>>()
        };
        assert_eq!(final_ids.len(), 3);
        assert_eq!(final_ids.get("alpha"), Some(&alpha_id));
        assert_eq!(final_ids.get("beta"), Some(&beta_id));

        restore_test_path(original_path);
        match original_sessions {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
        }
    }

    #[tokio::test]
    async fn adopt_tmux_session_reuses_stale_identity_and_rejects_duplicates() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
        let (_dir, original_path) = install_fake_tmux(
            r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
    while IFS= read -r line || [ -n "$line" ]; do
      printf '%s\n' "$line"
    done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
    ;;
  list-panes)
    exit 0
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
        let sessions_file = _dir.path().join("sessions.txt");
        std::fs::write(&sessions_file, "alpha\n").expect("sessions");
        std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        let mut stale = supervisor.build_placeholder_summary("sess_42", "alpha");
        stale.state = SessionState::Exited;
        stale.is_stale = true;
        stale.transport_health = TransportHealth::Disconnected;
        stale.cwd = "/tmp/project".to_string();
        supervisor.stale_sessions.write().await.push(stale);

        let adopted = supervisor
            .adopt_tmux_session("alpha".to_string(), None)
            .await
            .expect("adopt stale tmux session");
        assert!(adopted.reused_session_id);
        assert_eq!(adopted.session.session_id, "sess_42");
        assert_eq!(adopted.session.tmux_name, "alpha");
        assert!(!adopted.session.is_stale);

        let active = supervisor.list_sessions().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, "sess_42");
        assert!(supervisor.stale_sessions.read().await.is_empty());

        let duplicate = supervisor
            .adopt_tmux_session("alpha".to_string(), None)
            .await
            .expect_err("already tracked tmux should be rejected");
        assert_eq!(
            duplicate,
            TmuxAdoptError::AlreadyTracked {
                tmux_name: "alpha".to_string(),
                session_id: "sess_42".to_string()
            }
        );

        restore_test_path(original_path);
        match original_sessions {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
        }
    }

    #[tokio::test]
    async fn adopt_tmux_session_preserves_exact_whitespace_padded_name() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
        let original_attach_log = std::env::var_os("SWIMMERS_FAKE_TMUX_ATTACH_LOG");
        let (_dir, original_path) = install_fake_tmux(
            r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
    while IFS= read -r line || [ -n "$line" ]; do
      printf '%s\n' "$line"
    done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
    ;;
  attach-session)
    if [ -n "${SWIMMERS_FAKE_TMUX_ATTACH_LOG:-}" ]; then
      printf '%s\n' "$@" > "${SWIMMERS_FAKE_TMUX_ATTACH_LOG}"
    fi
    exit 0
    ;;
  list-panes|send-keys|kill-session|capture-pane)
    exit 0
    ;;
  display-message)
    case "${5-}" in
      "#{pane_current_command}") printf 'codex\n' ;;
      "#{pane_current_path}") printf '/tmp/project\n' ;;
      "#{pane_pid}") printf '101\n' ;;
      "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
    esac
    ;;
esac
"##,
        );
        let sessions_file = _dir.path().join("sessions.txt");
        let attach_log = _dir.path().join("attach.log");
        std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);
        std::env::set_var("SWIMMERS_FAKE_TMUX_ATTACH_LOG", &attach_log);

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        assert_eq!(
            supervisor
                .adopt_tmux_session(String::new(), None)
                .await
                .expect_err("empty tmux target should still be rejected"),
            TmuxAdoptError::EmptyTmuxName
        );

        std::fs::write(&sessions_file, "  padded  \n").expect("sessions");
        let stale = supervisor.build_placeholder_summary("sess_7", "  padded  ");
        supervisor.stale_sessions.write().await.push(stale);

        let adopted = supervisor
            .adopt_tmux_session("  padded  ".to_string(), Some("sess_7".to_string()))
            .await
            .expect("exact whitespace-padded tmux target should be adopted");
        assert!(adopted.reused_session_id);
        assert_eq!(adopted.session.session_id, "sess_7");
        assert_eq!(adopted.session.tmux_name, "  padded  ");
        let attach_args = (0..20)
            .find_map(|_| {
                std::fs::read_to_string(&attach_log).ok().or_else(|| {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                })
            })
            .expect("attach log");
        assert_eq!(attach_args, "attach-session\n-t\n=  padded  \n");

        let missing_trimmed = supervisor
            .adopt_tmux_session("padded".to_string(), None)
            .await
            .expect_err("trimmed spelling should not match exact tmux target");
        assert_eq!(
            missing_trimmed,
            TmuxAdoptError::TargetNotFound {
                tmux_name: "padded".to_string()
            }
        );

        restore_test_path(original_path);
        match original_sessions {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
        }
        match original_attach_log {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_ATTACH_LOG", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_ATTACH_LOG"),
        }
    }

    #[tokio::test]
    async fn adopt_tmux_session_rejects_missing_ambiguous_and_conflicting_targets() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
        let (_dir, original_path) = install_fake_tmux(
            r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
    while IFS= read -r line || [ -n "$line" ]; do
      printf '%s\n' "$line"
    done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
    ;;
  list-panes|send-keys|kill-session|capture-pane)
    exit 0
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
esac
"##,
        );
        let sessions_file = _dir.path().join("sessions.txt");
        std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

        let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
        std::fs::write(&sessions_file, "alpha\n").expect("sessions");
        assert_eq!(
            supervisor
                .adopt_tmux_session("beta".to_string(), None)
                .await
                .expect_err("missing tmux target should be rejected"),
            TmuxAdoptError::TargetNotFound {
                tmux_name: "beta".to_string()
            }
        );

        std::fs::write(&sessions_file, "alpha\nalpha\n").expect("duplicate sessions");
        assert_eq!(
            supervisor
                .adopt_tmux_session("alpha".to_string(), None)
                .await
                .expect_err("ambiguous tmux target should be rejected"),
            TmuxAdoptError::AmbiguousTarget {
                tmux_name: "alpha".to_string(),
                matches: 2
            }
        );

        std::fs::write(&sessions_file, "alpha\n").expect("sessions");
        let stale = supervisor.build_placeholder_summary("sess_7", "beta");
        supervisor.stale_sessions.write().await.push(stale);
        assert_eq!(
            supervisor
                .adopt_tmux_session("alpha".to_string(), Some("sess_7".to_string()))
                .await
                .expect_err("conflicting stale identity should be rejected"),
            TmuxAdoptError::StaleSessionConflict {
                session_id: "sess_7".to_string(),
                stale_tmux_name: "beta".to_string(),
                requested_tmux_name: "alpha".to_string()
            }
        );

        restore_test_path(original_path);
        match original_sessions {
            Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
            None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
        }
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

    #[test]
    fn plan_tmux_discovery_skips_empty_names() {
        let listed = vec!["".to_string(), "  ".to_string(), "".to_string()];
        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &HashSet::new(), &HashMap::new());
        // Empty strings are not valid discovery candidates; whitespace names
        // are preserved because tmux session names are exact targets.
        assert_eq!(highest_numeric, 0);
        assert_eq!(candidates.len(), 1); // "  " is non-empty, not tracked
        assert_eq!(candidates[0].tmux_name, "  ");
    }

    #[test]
    fn parse_tmux_session_names_preserves_exact_names() {
        let names = parse_tmux_session_names(b"alpha\n  padded  \n\tindented\n\nbeta\n");

        assert_eq!(
            names,
            vec![
                "alpha".to_string(),
                "  padded  ".to_string(),
                "\tindented".to_string(),
                "beta".to_string(),
            ]
        );
    }

    #[test]
    fn plan_tmux_discovery_all_tracked_returns_empty_candidates() {
        let listed = vec!["alpha".to_string(), "beta".to_string()];
        let tracked = HashSet::from_iter(["alpha".to_string(), "beta".to_string()]);
        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&listed, &tracked, &HashMap::new());
        assert_eq!(highest_numeric, 0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn plan_tmux_discovery_empty_list_returns_empty() {
        let (candidates, highest_numeric) =
            plan_tmux_discovery_candidates(&[], &HashSet::new(), &HashMap::new());
        assert_eq!(highest_numeric, 0);
        assert!(candidates.is_empty());
    }
}
