use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot, Mutex, Notify, RwLock};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::Config;
#[cfg(test)]
use crate::launcher::SpawnToolLauncher;
use crate::launcher::{create_private_file, prepare_private_dir};
use crate::persistence::file_store::{FileStore, PersistedSession, ThoughtSnapshot};
use crate::repo_theme::discover_repo_theme;
use crate::session::actor::{run_bounded_tmux_command_for_target, ActorHandle, SessionCommand};
#[cfg(test)]
use crate::session::spawn_command::{
    build_initial_request_input, build_spawn_tool_command, build_spawn_tool_command_with_launcher,
    schedule_prelaunch_file_cleanup_after, shell_single_quote,
};
use crate::session::spawn_command::{
    cleanup_prelaunch_files_now, current_working_dir, enqueue_initial_request_input,
    initial_request_delay, initial_tool_name, normalize_initial_request,
    normalize_requested_tmux_name, prepare_spawn_tool_command, schedule_prelaunch_file_cleanup,
    spawn_tool_consumes_initial_request, wrap_spawn_tool_command_for_tmux,
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
    fallback_rest_state, CassAdmissionError, CassAdmissionReservationRef, CassAdmissionSubject,
    CassOrigin, ControlEvent, DependencyHealthSnapshot, RepoTheme, SessionBatchMembership,
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
    tmux_names_requiring_active_pane_lookup, ActivePaneSessionIdMap,
};
pub use self::thought_persistence::SupervisorProvider;
use self::thought_persistence::THOUGHT_PERSIST_QUEUE_CAP;
#[path = "providers/mod.rs"]
mod providers;
use self::providers::{
    cass_admission_command_configured, cass_provider_identity_from_resume, prepare_provider_launch,
    refine_cass_provider_identity, run_cass_admission_command, unknown_launch_receipt,
    ProviderReceiptStore,
};

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn cass_admission_command_is_configured() -> bool {
    cass_admission_command_configured()
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn run_configured_cass_admission_command(
    request: crate::types::CassAdmissionCommandRequest,
) -> Result<crate::types::CassAdmissionCommandResponse, crate::types::CassAdmissionError> {
    run_cass_admission_command(request).await
}

#[cfg(test)]
pub(crate) use self::providers::{
    cass_provider_identity_from_resume as emit_cass_provider_identity_from_resume,
    refine_cass_provider_identity as refine_cass_admission_subject,
    reset_cass_admission_test_hooks, take_cass_command_ops, take_codex_app_server_launch_calls,
    take_last_cass_provider_identity,
};

#[cfg(test)]
static TMUX_SPAWN_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static CASS_POST_REFINEMENT_FAILURE: StdMutex<Option<&'static str>> = StdMutex::new(None);

const PROCESS_EXIT_SUMMARY_TIMEOUT: Duration = Duration::from_millis(250);
const TMUX_REDISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
const TMUX_KILL_SESSION_TIMEOUT: Duration = Duration::from_millis(500);
const THOUGHT_SNAPSHOT_COLLECTION_CONCURRENCY: usize = 8;
const EXACT_CLEANUP_RECEIPT_DIR: &str = "exact_session_cleanup_receipts";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactSessionCleanupOutcome {
    Deleted,
    AlreadyGone,
}

#[derive(Debug, Clone)]
pub struct ExactSessionCleanupResult {
    pub outcome: ExactSessionCleanupOutcome,
    pub delete_mode: crate::config::SessionDeleteMode,
    pub tmux_session_alive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactSessionCleanupError {
    NotAuthorized,
    GenerationMismatch,
    TmuxIncarnationMismatch,
    CleanupInProgress,
    Internal(String),
}

impl fmt::Display for ExactSessionCleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAuthorized => f.write_str("session cleanup is not authorized"),
            Self::GenerationMismatch => f.write_str("session cleanup generation mismatch"),
            Self::TmuxIncarnationMismatch => {
                f.write_str("tmux session incarnation no longer matches cleanup authority")
            }
            Self::CleanupInProgress => f.write_str("session cleanup is already in progress"),
            Self::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ExactSessionCleanupError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ExactSessionCleanupState {
    Active(ExactSessionCleanupTarget),
    Cleaning(ExactSessionCleanupTarget),
    Cleaned {
        generation: String,
        delete_mode: crate::config::SessionDeleteMode,
        tmux_session_alive: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExactSessionCleanupTarget {
    generation: String,
    tmux_name: String,
    tmux_target: TmuxTarget,
    tmux_incarnation: TmuxSessionIncarnation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TmuxSessionIncarnation {
    server_pid: u32,
    session_id: String,
    session_created: u64,
}

impl ExactSessionCleanupState {
    fn generation(&self) -> &str {
        match self {
            Self::Active(target) | Self::Cleaning(target) => &target.generation,
            Self::Cleaned { generation, .. } => generation,
        }
    }
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

    /// Private, versioned provider resume receipts keyed by Swimmers session id.
    provider_launch_receipts:
        RwLock<HashMap<String, crate::types::AuthorizedProviderResumeLaunchReceipt>>,

    /// Exact cleanup authority for the current incarnation of each session.
    /// Cleaned tombstones stay resident so authorized retries remain idempotent.
    exact_cleanup_states: RwLock<HashMap<String, ExactSessionCleanupState>>,

    /// Private durable authority/tombstone directory. Successful launch does
    /// not expose a generation until its active record is atomically durable.
    exact_cleanup_store_dir: PathBuf,

    /// Durable private receipt store. Resumable success is not returned until
    /// this store has atomically committed the exact Swimmers/provider pair.
    provider_receipt_store: ProviderReceiptStore,
}

#[derive(Debug)]
struct CassAdmissionLaunch {
    reservation: CassAdmissionReservationRef,
    cass_swimmers_session_id: String,
    phase: CassAdmissionPhase,
}

pub(crate) struct CassSessionAdmission {
    membership: SessionBatchMembership,
    reservation: CassAdmissionReservationRef,
}

impl CassSessionAdmission {
    pub(crate) fn new(
        membership: SessionBatchMembership,
        reservation: CassAdmissionReservationRef,
    ) -> Self {
        Self {
            membership,
            reservation,
        }
    }
}

struct SessionBatchLaunch {
    membership: Option<SessionBatchMembership>,
    cass_reservation: Option<CassAdmissionReservationRef>,
}

#[derive(Default)]
struct PrelaunchFileCleanupGuard {
    paths: Vec<PathBuf>,
}

impl PrelaunchFileCleanupGuard {
    fn replace(&mut self, paths: Vec<PathBuf>) {
        cleanup_prelaunch_files_now(&self.paths);
        self.paths = paths;
    }

    fn launch_started(&mut self) {
        schedule_prelaunch_file_cleanup(std::mem::take(&mut self.paths));
    }
}

impl Drop for PrelaunchFileCleanupGuard {
    fn drop(&mut self) {
        cleanup_prelaunch_files_now(&self.paths);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CassAdmissionPhase {
    Reserved,
    Consuming,
    Consumed,
    Refining,
    Refined,
    Releasing,
    Released,
    Reconciling,
    Reconciled,
    DurableSessionSuccess,
}

impl CassAdmissionLaunch {
    fn new(
        reservation: CassAdmissionReservationRef,
        batch: Option<&SessionBatchMembership>,
        swimmers_session_id: String,
    ) -> Result<Self, CassAdmissionError> {
        reservation.validate()?;
        let batch = batch.ok_or_else(|| {
            CassAdmissionError::new(
                "reservation_mismatch",
                "enforce mode requires canonical batch membership",
            )
        })?;
        let reservation_index = reservation.index.unwrap_or(reservation.batch_index) as usize;
        if batch.id != reservation.batch_id || batch.index != reservation_index {
            return Err(CassAdmissionError::new(
                "reservation_mismatch",
                "Cass reservation does not match canonical batch membership",
            ));
        }
        if reservation.target_id.as_deref() != Some("local") {
            return Err(CassAdmissionError::new(
                "reservation_mismatch",
                "Cass reservation is not bound to the target-local admission authority",
            ));
        }
        Ok(Self {
            reservation,
            cass_swimmers_session_id: swimmers_session_id,
            phase: CassAdmissionPhase::Reserved,
        })
    }
}

impl Drop for CassAdmissionLaunch {
    fn drop(&mut self) {
        if matches!(
            self.phase,
            CassAdmissionPhase::Released
                | CassAdmissionPhase::Reconciled
                | CassAdmissionPhase::DurableSessionSuccess
        ) {
            return;
        }
        let reservation_id = self.reservation.reservation_id.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            warn!(
                reservation_id,
                "Cass launch dropped unresolved outside a Tokio runtime"
            );
            return;
        };
        runtime.spawn(async move {
            let result =
                run_cass_admission_command(crate::types::CassAdmissionCommandRequest::reconcile(
                    &reservation_id,
                    "provider_ambiguity",
                ))
                .await;
            match result {
                Ok(response)
                    if response.ok
                        && response.reservation_id.as_deref() == Some(reservation_id.as_str())
                        && response.state.as_deref() == Some("unresolved") => {}
                Ok(response) => warn!(
                    cass_error_code = response
                        .error
                        .as_ref()
                        .map(|error| error.code.as_str())
                        .unwrap_or("partial_result"),
                    reservation_id, "Cass cancellation reconciliation returned an unbound result"
                ),
                Err(error) => warn!(
                    cass_error_code = %error.code,
                    reservation_id,
                    "failed to reconcile a cancellation-dropped Cass launch"
                ),
            }
        });
    }
}

#[derive(Default)]
struct ActivePaneCache {
    by_target: HashMap<TmuxTarget, ActivePaneCacheEntry>,
}

struct ActivePaneCacheEntry {
    fetched_at: Instant,
    panes: HashMap<String, String>,
}

const ACTIVE_PANE_CACHE_TTL: Duration = Duration::from_millis(1000);

impl SessionSupervisor {
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        #[cfg(test)]
        {
            let data_dir =
                std::env::temp_dir().join(format!("swimmers-supervisor-test-{}", Uuid::new_v4()));
            Self::new_with_stores(
                config,
                ProviderReceiptStore::new(data_dir.clone()),
                data_dir.join(EXACT_CLEANUP_RECEIPT_DIR),
            )
        }
        #[cfg(not(test))]
        {
            let data_dir = crate::startup::resolve_data_dir();
            Self::new_with_stores(
                config,
                ProviderReceiptStore::for_default_data_dir(),
                data_dir.join(EXACT_CLEANUP_RECEIPT_DIR),
            )
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_provider_receipt_data_dir(
        config: Arc<Config>,
        data_dir: impl Into<PathBuf>,
    ) -> Arc<Self> {
        let data_dir = data_dir.into();
        Self::new_with_stores(
            config,
            ProviderReceiptStore::new(data_dir.clone()),
            data_dir.join(EXACT_CLEANUP_RECEIPT_DIR),
        )
    }

    fn new_with_stores(
        config: Arc<Config>,
        provider_receipt_store: ProviderReceiptStore,
        exact_cleanup_store_dir: PathBuf,
    ) -> Arc<Self> {
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
            provider_launch_receipts: RwLock::new(HashMap::new()),
            exact_cleanup_states: RwLock::new(HashMap::new()),
            exact_cleanup_store_dir,
            provider_receipt_store,
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
        tmux_target: &TmuxTarget,
        tmux_names: &HashSet<String>,
        reason: &'static str,
    ) -> HashMap<String, String> {
        if tmux_names.is_empty() {
            return HashMap::new();
        }

        {
            let cache = self.active_pane_cache.lock().await;
            if let Some(entry) = cache.by_target.get(tmux_target) {
                if entry.fetched_at.elapsed() < ACTIVE_PANE_CACHE_TTL {
                    return filter_active_panes_to_requested(&entry.panes, tmux_names);
                }
            }
        }

        let fresh = match query_all_active_pane_session_ids(tmux_target).await {
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
            cache.by_target.insert(
                tmux_target.clone(),
                ActivePaneCacheEntry {
                    fetched_at: Instant::now(),
                    panes: fresh,
                },
            );
        }

        filtered
    }

    async fn active_pane_session_ids_for_summaries<'a, I>(
        &self,
        summaries: I,
        thought_snapshots: &HashMap<String, ThoughtSnapshot>,
        reason: &'static str,
    ) -> ActivePaneSessionIdMap
    where
        I: IntoIterator<Item = &'a SessionSummary>,
    {
        let tmux_names_by_target =
            tmux_names_requiring_active_pane_lookup(summaries, thought_snapshots);
        let mut active_panes = ActivePaneSessionIdMap::new();
        for (tmux_target, tmux_names) in tmux_names_by_target {
            let target_panes = self
                .active_pane_session_ids_cached(&tmux_target, &tmux_names, reason)
                .await;
            active_panes.extend(
                target_panes
                    .into_iter()
                    .map(|(tmux_name, pane_id)| ((tmux_target.clone(), tmux_name), pane_id)),
            );
        }
        active_panes
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
        match load_exact_cleanup_states(self.exact_cleanup_store_dir.clone()).await {
            Ok(states) => {
                for session_id in states.keys() {
                    self.bump_id_counter_from_session_id(session_id);
                }
                let mut cleanup_states = self.exact_cleanup_states.write().await;
                *cleanup_states = states
                    .into_iter()
                    .map(|(session_id, state)| {
                        let state = match state {
                            ExactSessionCleanupState::Cleaning(target) => {
                                ExactSessionCleanupState::Active(target)
                            }
                            state => state,
                        };
                        (session_id, state)
                    })
                    .collect();
            }
            Err(error) => {
                warn!(%error, "failed to load durable exact cleanup receipts");
            }
        }
        match self.provider_receipt_store.load().await {
            Ok(receipts) => {
                let mut provider_receipts = self.provider_launch_receipts.write().await;
                *provider_receipts = receipts
                    .into_iter()
                    .filter_map(|receipt| {
                        receipt
                            .launch()
                            .session_id
                            .clone()
                            .map(|session_id| (session_id, receipt))
                    })
                    .collect();
            }
            Err(error) => {
                warn!(%error, "failed to load durable provider resume receipts");
            }
        }

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
        self.create_session_with_target_batch_and_optional_cass(
            name,
            cwd,
            spawn_tool,
            initial_request,
            tmux_target,
            SessionBatchLaunch {
                membership: batch,
                cass_reservation: None,
            },
        )
        .await
    }

    pub(crate) async fn create_session_with_target_batch_and_cass(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: Option<String>,
        spawn_tool: Option<crate::types::SpawnTool>,
        initial_request: Option<String>,
        tmux_target: Option<TmuxTarget>,
        admission: CassSessionAdmission,
    ) -> anyhow::Result<(SessionSummary, Option<RepoTheme>)> {
        self.create_session_with_target_batch_and_optional_cass(
            name,
            cwd,
            spawn_tool,
            initial_request,
            tmux_target,
            SessionBatchLaunch {
                membership: Some(admission.membership),
                cass_reservation: Some(admission.reservation),
            },
        )
        .await
    }

    async fn create_session_with_target_batch_and_optional_cass(
        self: &Arc<Self>,
        name: Option<String>,
        cwd: Option<String>,
        spawn_tool: Option<crate::types::SpawnTool>,
        initial_request: Option<String>,
        tmux_target: Option<TmuxTarget>,
        batch_launch: SessionBatchLaunch,
    ) -> anyhow::Result<(SessionSummary, Option<RepoTheme>)> {
        let SessionBatchLaunch {
            membership: batch,
            cass_reservation,
        } = batch_launch;
        let start_cwd = cwd.or_else(current_working_dir);
        let mut initial_request = normalize_initial_request(initial_request);
        let tmux_target = tmux_target.unwrap_or_else(|| self.config.tmux_target.clone());
        let tmux_name = self.allocate_tmux_name(name);
        let (session_id, mut cass_launch) = match cass_reservation {
            Some(reservation) => {
                // Arm cancellation reconciliation before the first await. A
                // canceled request must not drop an admitted reservation while
                // UUID uniqueness checks are waiting on supervisor locks.
                let candidate = Uuid::new_v4().to_string();
                let mut launch = CassAdmissionLaunch::new(reservation, batch.as_ref(), candidate)?;
                let session_id = self.allocate_unique_cass_session_id(&mut launch).await;
                (session_id, Some(launch))
            }
            None => (self.allocate_unique_session_id().await, None),
        };

        if let Err(error) = tmux_target.validate() {
            self.release_cass_launch(&mut cass_launch).await?;
            return Err(error);
        }

        if let Some(dir) = start_cwd.as_deref() {
            if !Path::new(dir).is_dir() {
                self.release_cass_launch(&mut cass_launch).await?;
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
        let mut prelaunch_cleanup = PrelaunchFileCleanupGuard::default();
        if let Some(launch) = cass_launch.as_mut() {
            self.consume_cass_reservation_before_provider(launch)
                .await?;
        }
        let provider_launch_result =
            prepare_provider_launch(spawn_tool, start_cwd.as_deref(), initial_request.as_deref())
                .await;
        let mut provider_launch = match provider_launch_result {
            Ok(launch) => launch,
            Err(error) => {
                self.reconcile_cass_launch(&mut cass_launch, "provider_ambiguity")
                    .await?;
                return Err(error);
            }
        };
        if let Some(provider_launch) = provider_launch.as_mut() {
            prelaunch_cleanup.replace(provider_launch.take_cleanup_paths());
        }
        if let Some(launch) = cass_launch.as_mut() {
            self.refine_cass_after_provider_uuid(launch, provider_launch.as_ref())
                .await?;
        }
        let initial_command = if let Some(provider_launch) = provider_launch.as_mut() {
            initial_request = None;
            Some(wrap_spawn_tool_command_for_tmux(provider_launch.command()))
        } else {
            spawn_tool.map(|tool| {
                let command = prepare_spawn_tool_command(
                    tool,
                    start_cwd.as_deref(),
                    initial_request.as_deref(),
                );
                prelaunch_cleanup.replace(command.cleanup_paths);
                if spawn_tool_consumes_initial_request(tool) {
                    initial_request = None;
                }
                wrap_spawn_tool_command_for_tmux(&command.command)
            })
        };
        // Hold the discovery lock across spawn (which runs the real
        // `tmux new-session`) and the handle insert below so the periodic tmux
        // reconcile loop / adopt path cannot observe the freshly-created tmux
        // session in the window before its handle is registered and adopt it as
        // a *second* actor on the same pane (swimmers-ohfo). The discovery and
        // adopt paths both take `discovery_lock` first, and the lock order
        // `discovery_lock -> sessions.write` matches them, so no deadlock; the
        // only cost is serializing creates against discovery.
        let discovery_guard = self.discovery_lock.lock().await;
        let handle = match cass_post_refinement_fault("tmux_spawn_failure").and_then(|()| {
            crate::session::actor::SessionActor::spawn(
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
            )
        }) {
            Ok(handle) => {
                record_tmux_spawn_attempt();
                prelaunch_cleanup.launch_started();
                handle
            }
            Err(err) => {
                self.reconcile_cass_launch(&mut cass_launch, "tmux_spawn_failure")
                    .await?;
                return Err(err);
            }
        };
        let bootstrap_handle = handle.clone();

        let insert_result = match cass_post_refinement_fault("cleanup_authority_failure") {
            Ok(()) => self.insert_active_handle(session_id.clone(), handle).await,
            Err(error) => Err(error),
        };
        if let Err(error) = insert_result {
            if let Err(kill_error) =
                kill_tmux_session(&bootstrap_handle.tmux_name, &bootstrap_handle.tmux_target).await
            {
                warn!(
                    %kill_error,
                    session_id,
                    "failed to roll back tmux session after cleanup authority persistence failure"
                );
            }
            let _ = bootstrap_handle.cmd_tx.send(SessionCommand::Shutdown).await;
            self.reconcile_cass_launch(&mut cass_launch, "cleanup_authority_failure")
                .await?;
            return Err(error.context("exact cleanup authority persistence failed"));
        }
        // Release the discovery lock as soon as the handle is registered; the
        // remaining summary/emit/persist work no longer races discovery.
        drop(discovery_guard);
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
        if let Some(provider_launch) = provider_launch.as_mut() {
            let confirm_result = match cass_post_refinement_fault("provider_start_failure") {
                Ok(()) => provider_launch.confirm_started().await,
                Err(error) => Err(error),
            };
            if let Err(error) = confirm_result {
                self.rollback_provider_launch(&session_id, &bootstrap_handle)
                    .await;
                self.reconcile_cass_launch(&mut cass_launch, "provider_start_failure")
                    .await?;
                return Err(error);
            }
        }
        let launch_receipt =
            crate::types::LaunchReceipt::local(summary.cwd.clone(), session_id.clone(), false);
        let provider_receipt =
            cass_post_refinement_fault("provider_receipt_failure").and_then(|()| {
                match provider_launch {
                    Some(provider_launch) => provider_launch.finalize(launch_receipt),
                    None => Ok(unknown_launch_receipt(launch_receipt, spawn_tool)),
                }
            });
        let provider_receipt = match provider_receipt {
            Ok(receipt) => receipt,
            Err(error) => {
                self.rollback_provider_launch(&session_id, &bootstrap_handle)
                    .await;
                self.reconcile_cass_launch(&mut cass_launch, "provider_receipt_failure")
                    .await?;
                return Err(error);
            }
        };
        if provider_receipt.provider_resume().is_resumable() {
            let persist_result =
                match cass_post_refinement_fault("provider_receipt_persistence_failure") {
                    Ok(()) => self.provider_receipt_store.persist(&provider_receipt).await,
                    Err(error) => Err(error),
                };
            if let Err(error) = persist_result {
                self.rollback_provider_launch(&session_id, &bootstrap_handle)
                    .await;
                self.reconcile_cass_launch(
                    &mut cass_launch,
                    "provider_receipt_persistence_failure",
                )
                .await?;
                return Err(error.context("durable provider receipt persistence failed"));
            }
        }
        self.provider_launch_receipts
            .write()
            .await
            .insert(session_id.clone(), provider_receipt);
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
        if let Some(launch) = cass_launch.as_mut() {
            launch.phase = CassAdmissionPhase::DurableSessionSuccess;
        }

        Ok((summary, repo_theme))
    }

    async fn consume_cass_reservation_before_provider(
        &self,
        launch: &mut CassAdmissionLaunch,
    ) -> anyhow::Result<()> {
        if !cass_admission_command_configured() {
            return Err(anyhow::Error::new(CassAdmissionError::new(
                "admission_command_unavailable",
                "target admission command unavailable in enforce mode",
            )));
        }
        launch.phase = CassAdmissionPhase::Consuming;
        let response = match run_cass_admission_command(
            crate::types::CassAdmissionCommandRequest::consume(&launch.reservation),
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                self.reconcile_cass_launch_ref(launch, "transport_loss")
                    .await?;
                return Err(anyhow::Error::new(error));
            }
        };
        if !response.ok {
            let error = response
                .error
                .unwrap_or(crate::types::CassAdmissionCommandError {
                    code: "reservation_mismatch".to_string(),
                    message: "reservation consume refused".to_string(),
                });
            self.reconcile_cass_launch_ref(launch, "partial_result")
                .await?;
            return Err(anyhow::Error::new(CassAdmissionError::new(
                error.code,
                error.message,
            )));
        }
        if response.reservation_id.as_deref() != Some(launch.reservation.reservation_id.as_str())
            || response.batch_id.as_deref() != Some(launch.reservation.batch_id.as_str())
            || response.batch_index != Some(launch.reservation.batch_index)
            || !matches!(response.state.as_deref(), Some("reserved" | "consumed"))
        {
            self.reconcile_cass_launch_ref(launch, "partial_result")
                .await?;
            return Err(anyhow::Error::new(CassAdmissionError::new(
                "partial_result",
                "reservation consume returned an unbound result",
            )));
        }
        launch.phase = CassAdmissionPhase::Consumed;
        Ok(())
    }

    async fn refine_cass_after_provider_uuid(
        &self,
        launch: &mut CassAdmissionLaunch,
        provider_launch: Option<&self::providers::PreparedProviderLaunch>,
    ) -> anyhow::Result<()> {
        let Some(provider_launch) = provider_launch else {
            self.release_cass_launch_ref(launch).await?;
            return Err(anyhow::Error::new(CassAdmissionError::new(
                "provider_unavailable",
                "Cass enforce mode requires a provider launch with a captured session UUID",
            )));
        };
        let Some(resume) = provider_launch.captured_resume() else {
            self.release_cass_launch_ref(launch).await?;
            return Err(anyhow::Error::new(CassAdmissionError::new(
                "provider_unavailable",
                "Cass enforce mode provider cannot supply a pre-spawn session UUID",
            )));
        };
        let identity = match cass_provider_identity_from_resume(
            resume,
            &launch.cass_swimmers_session_id,
            &launch.reservation.batch_id,
            launch.reservation.batch_index,
            cass_origin_for_target(launch.reservation.target_id.as_deref()),
        ) {
            Ok(identity) => identity,
            Err(error) => {
                self.reconcile_cass_launch_ref(launch, "refinement_failure")
                    .await?;
                return Err(anyhow::Error::new(error));
            }
        };
        launch.phase = CassAdmissionPhase::Refining;
        match refine_cass_provider_identity(&launch.reservation, identity).await {
            Ok(subject) => {
                let _subject: CassAdmissionSubject = subject;
                launch.phase = CassAdmissionPhase::Refined;
                Ok(())
            }
            Err(error) => {
                self.reconcile_cass_launch_ref(launch, reconcile_cause_for(&error))
                    .await?;
                Err(anyhow::Error::new(error))
            }
        }
    }

    async fn reconcile_cass_launch(
        &self,
        launch: &mut Option<CassAdmissionLaunch>,
        cause: &str,
    ) -> Result<(), CassAdmissionError> {
        if let Some(launch) = launch.as_mut() {
            self.reconcile_cass_launch_ref(launch, cause).await?;
        }
        Ok(())
    }

    async fn reconcile_cass_launch_ref(
        &self,
        launch: &mut CassAdmissionLaunch,
        cause: &str,
    ) -> Result<(), CassAdmissionError> {
        if matches!(
            launch.phase,
            CassAdmissionPhase::Released
                | CassAdmissionPhase::Reconciled
                | CassAdmissionPhase::DurableSessionSuccess
        ) {
            return Ok(());
        }
        launch.phase = CassAdmissionPhase::Reconciling;
        let response =
            run_cass_admission_command(crate::types::CassAdmissionCommandRequest::reconcile(
                &launch.reservation.reservation_id,
                cause,
            ))
            .await?;
        validate_cass_settlement_response(&response, launch, "unresolved")?;
        launch.phase = CassAdmissionPhase::Reconciled;
        Ok(())
    }

    async fn release_cass_launch(
        &self,
        launch: &mut Option<CassAdmissionLaunch>,
    ) -> Result<(), CassAdmissionError> {
        if let Some(launch) = launch.as_mut() {
            self.release_cass_launch_ref(launch).await?;
        }
        Ok(())
    }

    async fn release_cass_launch_ref(
        &self,
        launch: &mut CassAdmissionLaunch,
    ) -> Result<(), CassAdmissionError> {
        if matches!(
            launch.phase,
            CassAdmissionPhase::Released
                | CassAdmissionPhase::Reconciled
                | CassAdmissionPhase::DurableSessionSuccess
        ) {
            return Ok(());
        }
        launch.phase = CassAdmissionPhase::Releasing;
        let response = run_cass_admission_command(
            crate::types::CassAdmissionCommandRequest::release(&launch.reservation.reservation_id),
        )
        .await?;
        validate_cass_settlement_response(&response, launch, "released")?;
        launch.phase = CassAdmissionPhase::Released;
        Ok(())
    }

    async fn rollback_provider_launch(&self, session_id: &str, handle: &ActorHandle) {
        self.sessions.write().await.remove(session_id);
        self.exact_cleanup_states.write().await.remove(session_id);
        if let Err(error) =
            remove_exact_cleanup_state(self.exact_cleanup_store_dir.clone(), session_id).await
        {
            warn!(
                %error,
                session_id,
                "failed to remove rolled-back exact cleanup authority"
            );
        }
        crate::metrics::set_active_sessions(self.sessions.read().await.len());
        if let Err(error) = kill_tmux_session(&handle.tmux_name, &handle.tmux_target).await {
            warn!(
                %error,
                session_id,
                "failed to roll back tmux session after provider receipt failure"
            );
        }
        let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
    }

    pub(crate) async fn provider_launch_receipt(
        &self,
        session_id: &str,
    ) -> Option<crate::types::AuthorizedProviderResumeLaunchReceipt> {
        self.provider_launch_receipts
            .read()
            .await
            .get(session_id)
            .cloned()
    }

    pub(crate) async fn exact_cleanup_generation(&self, session_id: &str) -> Option<String> {
        self.exact_cleanup_states
            .read()
            .await
            .get(session_id)
            .map(|state| state.generation().to_string())
    }

    fn allocate_tmux_name(&self, requested_name: Option<String>) -> String {
        normalize_requested_tmux_name(requested_name).unwrap_or_else(|| {
            let n = self.next_name_counter.fetch_add(1, Ordering::SeqCst);
            n.to_string()
        })
    }

    async fn insert_active_handle(
        &self,
        session_id: String,
        handle: ActorHandle,
    ) -> anyhow::Result<()> {
        let tmux_incarnation =
            match query_tmux_session_incarnation_retry(&handle.tmux_name, &handle.tmux_target)
                .await?
            {
                Some(incarnation) => incarnation,
                None => {
                    anyhow::bail!("new tmux session disappeared before cleanup authority binding")
                }
            };
        let state = ExactSessionCleanupState::Active(ExactSessionCleanupTarget {
            generation: Uuid::new_v4().to_string(),
            tmux_name: handle.tmux_name.clone(),
            tmux_target: handle.tmux_target.clone(),
            tmux_incarnation,
        });
        persist_exact_cleanup_state(
            self.exact_cleanup_store_dir.clone(),
            session_id.clone(),
            state.clone(),
        )
        .await?;
        let mut cleanup_states = self.exact_cleanup_states.write().await;
        let mut sessions = self.sessions.write().await;
        cleanup_states.insert(session_id.clone(), state);
        sessions.insert(session_id, handle);
        crate::metrics::set_active_sessions(sessions.len());
        Ok(())
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
                .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
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

    /// Delete only the exact authorized session incarnation.
    ///
    /// The generation check and transition to `Cleaning` happen under one
    /// lock, so stale or concurrent cleanup cannot remove a replacement actor.
    /// Provider launch receipts are intentionally outside this state machine
    /// and survive successful tmux cleanup.
    pub async fn delete_exact_session(
        self: &Arc<Self>,
        session_id: &str,
        generation: &str,
        delete_mode: crate::config::SessionDeleteMode,
    ) -> Result<ExactSessionCleanupResult, ExactSessionCleanupError> {
        let _discovery_guard = self.discovery_lock.lock().await;
        let (handle, target) = {
            let mut states = self.exact_cleanup_states.write().await;
            let state = states
                .get(session_id)
                .ok_or(ExactSessionCleanupError::NotAuthorized)?;
            if state.generation() != generation {
                return Err(ExactSessionCleanupError::GenerationMismatch);
            }
            match state {
                ExactSessionCleanupState::Cleaning(_) => {
                    return Err(ExactSessionCleanupError::CleanupInProgress);
                }
                ExactSessionCleanupState::Cleaned {
                    delete_mode,
                    tmux_session_alive,
                    ..
                } => {
                    return Ok(ExactSessionCleanupResult {
                        outcome: ExactSessionCleanupOutcome::AlreadyGone,
                        delete_mode: delete_mode.clone(),
                        tmux_session_alive: *tmux_session_alive,
                    });
                }
                ExactSessionCleanupState::Active(_) => {}
            }
            let target = match state {
                ExactSessionCleanupState::Active(target) => target.clone(),
                ExactSessionCleanupState::Cleaning(_)
                | ExactSessionCleanupState::Cleaned { .. } => unreachable!(),
            };
            let mut sessions = self.sessions.write().await;
            let handle = sessions.remove(session_id);
            crate::metrics::set_active_sessions(sessions.len());
            states.insert(
                session_id.to_string(),
                ExactSessionCleanupState::Cleaning(target.clone()),
            );
            (handle, target)
        };
        if let Err(error) = persist_exact_cleanup_state(
            self.exact_cleanup_store_dir.clone(),
            session_id.to_string(),
            ExactSessionCleanupState::Cleaning(target.clone()),
        )
        .await
        {
            self.restore_exact_cleanup(session_id, generation, handle)
                .await;
            return Err(ExactSessionCleanupError::Internal(format!(
                "failed to persist exact cleanup transition: {error}"
            )));
        }

        let mut tmux_session_alive = true;
        let outcome = if matches!(delete_mode, crate::config::SessionDeleteMode::KillTmux) {
            match query_tmux_session_incarnation(&target.tmux_name, &target.tmux_target).await {
                Ok(None) => {
                    tmux_session_alive = false;
                    ExactSessionCleanupOutcome::AlreadyGone
                }
                Ok(Some(incarnation)) if incarnation != target.tmux_incarnation => {
                    self.restore_exact_cleanup(session_id, generation, handle)
                        .await;
                    return Err(ExactSessionCleanupError::TmuxIncarnationMismatch);
                }
                Ok(Some(_)) => {
                    match kill_tmux_session_target_with_outcome(
                        &target.tmux_incarnation.session_id,
                        &target.tmux_target,
                    )
                    .await
                    {
                        Ok(was_alive) => {
                            tmux_session_alive = false;
                            if was_alive {
                                ExactSessionCleanupOutcome::Deleted
                            } else {
                                ExactSessionCleanupOutcome::AlreadyGone
                            }
                        }
                        Err(error) => {
                            self.restore_exact_cleanup(session_id, generation, handle)
                                .await;
                            return Err(ExactSessionCleanupError::Internal(error.to_string()));
                        }
                    }
                }
                Err(error) => {
                    self.restore_exact_cleanup(session_id, generation, handle)
                        .await;
                    return Err(ExactSessionCleanupError::Internal(error.to_string()));
                }
            }
        } else if handle.is_some() {
            ExactSessionCleanupOutcome::Deleted
        } else {
            ExactSessionCleanupOutcome::AlreadyGone
        };

        self.process_exit_seen_at.write().await.remove(session_id);
        info!(
            session_id,
            delete_mode = ?delete_mode,
            "deleting exact session generation"
        );
        if let Some(handle) = handle {
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown).await;
        }
        let _ = self.lifecycle_tx.send(LifecycleEvent::Deleted {
            session_id: session_id.to_string(),
            reason: "api_exact_delete".into(),
            delete_mode: delete_mode.clone(),
            tmux_session_alive,
        });
        self.persist_registry().await;

        let result = ExactSessionCleanupResult {
            outcome,
            delete_mode,
            tmux_session_alive,
        };
        self.complete_exact_cleanup(session_id, generation, &result)
            .await?;
        Ok(result)
    }

    async fn complete_exact_cleanup(
        &self,
        session_id: &str,
        generation: &str,
        result: &ExactSessionCleanupResult,
    ) -> Result<(), ExactSessionCleanupError> {
        {
            let states = self.exact_cleanup_states.read().await;
            let state = states
                .get(session_id)
                .ok_or(ExactSessionCleanupError::NotAuthorized)?;
            if state.generation() != generation {
                return Err(ExactSessionCleanupError::GenerationMismatch);
            }
            if !matches!(state, ExactSessionCleanupState::Cleaning(_)) {
                return Err(ExactSessionCleanupError::CleanupInProgress);
            }
        }
        let cleaned = ExactSessionCleanupState::Cleaned {
            generation: generation.to_string(),
            delete_mode: result.delete_mode.clone(),
            tmux_session_alive: result.tmux_session_alive,
        };
        persist_exact_cleanup_state(
            self.exact_cleanup_store_dir.clone(),
            session_id.to_string(),
            cleaned.clone(),
        )
        .await
        .map_err(|error| {
            ExactSessionCleanupError::Internal(format!(
                "failed to persist exact cleanup receipt: {error}"
            ))
        })?;
        self.exact_cleanup_states
            .write()
            .await
            .insert(session_id.to_string(), cleaned);
        Ok(())
    }

    async fn restore_exact_cleanup(
        &self,
        session_id: &str,
        generation: &str,
        handle: Option<ActorHandle>,
    ) {
        let active = {
            let mut states = self.exact_cleanup_states.write().await;
            let target = match states.get(session_id) {
                Some(ExactSessionCleanupState::Cleaning(target))
                    if target.generation == generation =>
                {
                    target.clone()
                }
                _ => return,
            };
            let mut sessions = self.sessions.write().await;
            if let Some(handle) = handle {
                sessions.insert(session_id.to_string(), handle);
                crate::metrics::set_active_sessions(sessions.len());
            }
            let active = ExactSessionCleanupState::Active(target);
            states.insert(session_id.to_string(), active.clone());
            active
        };
        if let Err(error) = persist_exact_cleanup_state(
            self.exact_cleanup_store_dir.clone(),
            session_id.to_string(),
            active,
        )
        .await
        {
            warn!(
                %error,
                session_id,
                "failed to persist restored exact cleanup authority"
            );
        }
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
        let mut cleanup_states = self.exact_cleanup_states.write().await;
        let mut sessions = self.sessions.write().await;
        cleanup_states.insert(
            handle.session_id.clone(),
            ExactSessionCleanupState::Active(ExactSessionCleanupTarget {
                generation: Uuid::new_v4().to_string(),
                tmux_name: handle.tmux_name.clone(),
                tmux_target: handle.tmux_target.clone(),
                tmux_incarnation: TmuxSessionIncarnation {
                    server_pid: 0,
                    session_id: format!("test:{}", handle.session_id),
                    session_created: 0,
                },
            }),
        );
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

        let summaries_with_replay: Vec<(SessionSummary, String)> = futures::stream::iter(handles)
            .map(|handle| Self::collect_summary_and_replay(handle, timeout))
            .buffer_unordered(THOUGHT_SNAPSHOT_COLLECTION_CONCURRENCY)
            .filter_map(futures::future::ready)
            .collect()
            .await;

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

    /// Load the persisted record for a session id with a single registry read,
    /// so a discovered/adopted actor can resume both its `last_activity_at`
    /// rest state and its batch membership without re-reading and
    /// re-deserializing the whole registry once per field.
    async fn persisted_session(&self, session_id: &str) -> Option<PersistedSession> {
        let store = {
            let guard = self.persistence.read().await;
            guard.as_ref().cloned()?
        };
        store
            .load_sessions()
            .await
            .into_iter()
            .find(|ps| ps.session_id == session_id)
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
        format!("sess_{n}")
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

            if self
                .exact_cleanup_states
                .read()
                .await
                .contains_key(&candidate)
            {
                continue;
            }

            return candidate;
        }
    }

    async fn allocate_unique_cass_session_id(&self, launch: &mut CassAdmissionLaunch) -> String {
        loop {
            let candidate = launch.cass_swimmers_session_id.clone();

            if self.sessions.read().await.contains_key(&candidate) {
                launch.cass_swimmers_session_id = Uuid::new_v4().to_string();
                continue;
            }
            if self
                .stale_sessions
                .read()
                .await
                .iter()
                .any(|session| session.session_id == candidate)
            {
                launch.cass_swimmers_session_id = Uuid::new_v4().to_string();
                continue;
            }
            if self
                .exact_cleanup_states
                .read()
                .await
                .contains_key(&candidate)
            {
                launch.cass_swimmers_session_id = Uuid::new_v4().to_string();
                continue;
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

const EXACT_CLEANUP_RECORD_VERSION: u16 = 1;

#[derive(Serialize, Deserialize)]
struct DurableExactCleanupRecord {
    version: u16,
    session_id: String,
    state: ExactSessionCleanupState,
}

async fn persist_exact_cleanup_state(
    dir: PathBuf,
    session_id: String,
    state: ExactSessionCleanupState,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        persist_exact_cleanup_state_blocking(&dir, &session_id, &state)
    })
    .await
    .map_err(|error| anyhow::anyhow!("exact cleanup persistence task failed: {error}"))?
}

fn persist_exact_cleanup_state_blocking(
    dir: &Path,
    session_id: &str,
    state: &ExactSessionCleanupState,
) -> anyhow::Result<()> {
    prepare_private_dir(dir)?;
    let stem = exact_cleanup_hex_name(session_id);
    let destination = dir.join(format!("{stem}.json"));
    let temporary = dir.join(format!(".{stem}.{}.tmp", Uuid::new_v4()));
    let record = DurableExactCleanupRecord {
        version: EXACT_CLEANUP_RECORD_VERSION,
        session_id: session_id.to_string(),
        state: state.clone(),
    };
    let encoded = serde_json::to_vec_pretty(&record)?;
    let write_result = (|| -> anyhow::Result<()> {
        let mut file = create_private_file(&temporary)?;
        file.write_all(&encoded)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)?;
        if let Err(error) = fs::File::open(dir).and_then(|directory| directory.sync_all()) {
            let removal = fs::remove_file(&destination);
            let _ = fs::File::open(dir).and_then(|directory| directory.sync_all());
            removal?;
            return Err(error.into());
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

async fn load_exact_cleanup_states(
    dir: PathBuf,
) -> anyhow::Result<HashMap<String, ExactSessionCleanupState>> {
    tokio::task::spawn_blocking(move || load_exact_cleanup_states_blocking(&dir))
        .await
        .map_err(|error| anyhow::anyhow!("exact cleanup load task failed: {error}"))?
}

fn load_exact_cleanup_states_blocking(
    dir: &Path,
) -> anyhow::Result<HashMap<String, ExactSessionCleanupState>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(error.into()),
    };
    let mut states = HashMap::new();
    for entry in entries {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json")
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let record: DurableExactCleanupRecord = serde_json::from_slice(&fs::read(entry.path())?)?;
        anyhow::ensure!(
            record.version == EXACT_CLEANUP_RECORD_VERSION,
            "unsupported exact cleanup receipt version {}",
            record.version
        );
        states.insert(record.session_id, record.state);
    }
    Ok(states)
}

async fn remove_exact_cleanup_state(dir: PathBuf, session_id: &str) -> anyhow::Result<()> {
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        let path = dir.join(format!("{}.json", exact_cleanup_hex_name(&session_id)));
        match fs::remove_file(path) {
            Ok(()) => {
                fs::File::open(dir)?.sync_all()?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    })
    .await
    .map_err(|error| anyhow::anyhow!("exact cleanup removal task failed: {error}"))?
}

fn exact_cleanup_hex_name(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn cass_origin_for_target(target_id: Option<&str>) -> CassOrigin {
    match target_id.map(str::trim) {
        Some("remote") => CassOrigin::Remote,
        Some(target) if !target.is_empty() && target != "local" => {
            crate::types::cass_publisher_locality_from_env()
        }
        _ => crate::types::cass_publisher_locality_from_env(),
    }
}

fn reconcile_cause_for(error: &CassAdmissionError) -> &'static str {
    match error.code.as_str() {
        "transport_loss" => "transport_loss",
        "partial_result" => "partial_result",
        "provider_ambiguity" => "provider_ambiguity",
        _ => "refinement_failure",
    }
}

fn validate_cass_settlement_response(
    response: &crate::types::CassAdmissionCommandResponse,
    launch: &CassAdmissionLaunch,
    expected_state: &str,
) -> Result<(), CassAdmissionError> {
    if !response.ok {
        let error = response
            .error
            .clone()
            .unwrap_or(crate::types::CassAdmissionCommandError {
                code: "partial_result".to_string(),
                message: "Cass settlement command was refused".to_string(),
            });
        return Err(CassAdmissionError::new(error.code, error.message));
    }
    if response.reservation_id.as_deref() != Some(launch.reservation.reservation_id.as_str())
        || response.state.as_deref() != Some(expected_state)
    {
        return Err(CassAdmissionError::new(
            "partial_result",
            "Cass settlement command returned an unbound result",
        ));
    }
    Ok(())
}

fn cass_post_refinement_fault(point: &'static str) -> anyhow::Result<()> {
    #[cfg(test)]
    {
        if CASS_POST_REFINEMENT_FAILURE
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .is_some_and(|configured| *configured == point)
        {
            anyhow::bail!("injected Cass post-refinement failure at {point}");
        }
    }
    let _ = point;
    Ok(())
}

#[cfg(test)]
pub(crate) fn set_cass_post_refinement_failure(point: Option<&'static str>) {
    *CASS_POST_REFINEMENT_FAILURE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = point;
}

fn record_tmux_spawn_attempt() {
    #[cfg(test)]
    TMUX_SPAWN_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn take_tmux_spawn_calls() -> usize {
    TMUX_SPAWN_CALLS.swap(0, Ordering::SeqCst)
}

async fn query_tmux_session_incarnation_retry(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
) -> anyhow::Result<Option<TmuxSessionIncarnation>> {
    const ATTEMPTS: usize = 8;
    for attempt in 0..ATTEMPTS {
        if let Some(incarnation) = query_tmux_session_incarnation(tmux_name, tmux_target).await? {
            return Ok(Some(incarnation));
        }
        if attempt + 1 < ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
    }
    Ok(None)
}

async fn query_tmux_session_incarnation(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
) -> anyhow::Result<Option<TmuxSessionIncarnation>> {
    let output = run_bounded_tmux_command_for_target(
        "tmux",
        tmux_target,
        &[
            "list-sessions",
            "-F",
            "#{pid}\t#{session_id}\t#{session_created}\t#{session_name}",
        ],
        TMUX_KILL_SESSION_TIMEOUT,
        "list-sessions",
    )
    .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if tmux_kill_reports_missing_session(&stderr) {
            return Ok(None);
        }
        return Err(anyhow::anyhow!(
            "tmux session incarnation query failed: {}",
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| anyhow::anyhow!("tmux session incarnation was not UTF-8: {error}"))?;
    let Some(line) = stdout.lines().find(|line| {
        line.rsplit_once('\t')
            .is_some_and(|(_, name)| name == tmux_name)
    }) else {
        return Ok(None);
    };
    let mut fields = line.split('\t');
    let server_pid = fields
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| anyhow::anyhow!("tmux session incarnation omitted server pid"))?;
    let session_id = fields
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("tmux session incarnation omitted session id"))?
        .to_string();
    let session_created = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| anyhow::anyhow!("tmux session incarnation omitted creation time"))?;
    let returned_name = fields
        .next()
        .ok_or_else(|| anyhow::anyhow!("tmux session incarnation omitted session name"))?;
    anyhow::ensure!(
        returned_name == tmux_name && fields.next().is_none(),
        "tmux session incarnation returned unexpected fields"
    );
    Ok(Some(TmuxSessionIncarnation {
        server_pid,
        session_id,
        session_created,
    }))
}

async fn kill_tmux_session(tmux_name: &str, tmux_target: &TmuxTarget) -> anyhow::Result<()> {
    kill_tmux_session_with_outcome(tmux_name, tmux_target)
        .await
        .map(|_| ())
}

/// Returns `true` when tmux reported a live session was killed and `false`
/// when the exact tmux target was already absent.
async fn kill_tmux_session_with_outcome(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
) -> anyhow::Result<bool> {
    let target = exact_session_target(tmux_name);
    kill_tmux_session_target_with_outcome(&target, tmux_target).await
}

async fn kill_tmux_session_target_with_outcome(
    target: &str,
    tmux_target: &TmuxTarget,
) -> anyhow::Result<bool> {
    let output = run_bounded_tmux_command_for_target(
        "tmux",
        tmux_target,
        &["kill-session", "-t", target],
        TMUX_KILL_SESSION_TIMEOUT,
        "kill-session",
    )
    .await?;

    if output.status.success() {
        Ok(true)
    } else if tmux_kill_reports_missing_session(&String::from_utf8_lossy(&output.stderr)) {
        Ok(false)
    } else {
        classify_failed_kill_tmux_session(&output.stderr).map(|()| false)
    }
}

#[cfg(test)]
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
