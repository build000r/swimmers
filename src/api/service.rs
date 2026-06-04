use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use tokio::process::Command;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::{fetch_live_summary, remote_sessions, AppState};
use crate::host_actions::{
    inspect_git_repo, RepoActionExecutor, RestartExecutor, SystemRepoActionExecutor,
};
use crate::native;
use crate::openrouter_models::cached_or_default_openrouter_candidates;
use crate::operator_pressure::session_ready_for_operator_group_input;
use crate::persistence::file_store::FileStore;
use crate::session::actor::{ActorHandle, SessionCommand};
#[cfg(test)]
use crate::session::overlay::OverlayDirGroup;
use crate::session::overlay::{OverlayDirConfig, OverlayServiceEntry};
use crate::thought::probe::{run_thought_config_probe, ThoughtConfigProbeResult};
use crate::thought::runtime_config::ThoughtConfig;
use crate::thought_ui::thought_config_ui_metadata;
use crate::types::{
    CreateSessionsBatchResponse, CreateSessionsBatchResult, DirEntry,
    DirGroupMembershipUpdateRequest, DirGroupMembershipUpdateResponse, DirGroupMemberships,
    DirListResponse, DirRepoActionResponse, DirRepoSearchResponse, DirRestartResponse,
    ErrorResponse, GhosttyOpenMode, LaunchTargetSummary, NativeAttentionGroupOpenRequest,
    NativeAttentionGroupOpenResponse, NativeDesktopApp, NativeDesktopOpenResponse,
    NativeDesktopStatusResponse, PlanFileResponse, RepoActionKind, RepoActionState,
    RepoActionStatus, RepoTheme, SessionBatchMembership, SessionState, SessionSummary, SpawnTool,
    ThoughtConfigResponse,
};

#[path = "service_directory.rs"]
mod service_directory;

use service_directory::{
    annotate_dir_entry_groups, canonical_path_string, effective_dir_config_for_base,
    effective_groups_for_path, has_visible_child_dirs, list_effective_group_entries, modified_secs,
    overlay_group_contains_path, service_dir_path,
};
pub use service_directory::{
    list_effective_group_entries_sync, list_group_entries, list_group_entries_sync,
    managed_base_child_names, resolve_dir_config, resolve_target_path, services_for_directory,
    OverlayServiceContext,
};

/// Max concurrent git probes per `list_dirs` call. Keeps a single listing from
/// fork-bombing the system when a repos directory has many git subdirs, while
/// still parallelizing enough to hide per-call git latency.
const GIT_PROBE_CONCURRENCY: usize = 32;
const BATCH_PROMPT_EXCERPT_MAX_CHARS: usize = 72;
const BATCH_LABEL_MAX_CHARS: usize = 28;
pub const BATCH_CREATE_MAX_DIRS: usize = 32;
pub const BATCH_CREATE_CONCURRENCY: usize = 4;
pub const PLAN_FILE_TIMEOUT: Duration = Duration::from_secs(5);
const REPO_SEARCH_ROOTS_ENV: &str = "SWIMMERS_REPO_SEARCH_ROOTS";
const REPO_SEARCH_MAX_DEPTH_ENV: &str = "SWIMMERS_REPO_SEARCH_MAX_DEPTH";
const REPO_SEARCH_DEFAULT_MAX_DEPTH: usize = 8;
const REPO_SEARCH_CACHE_TTL: Duration = Duration::from_secs(60);
const NATIVE_ATTENTION_GROUP_SESSION_ID: &str = "attention-group";
const NATIVE_ATTENTION_GROUP_TMUX_NAME: &str = "swimmers-attention";

/// Check service health by sending HTTP GET requests to overlay-defined URLs.
///
/// Local-dev overlays routinely declare health URLs for services that are not
/// currently running (`http://localhost:PORT/...`). Without these tight
/// budgets, reqwest's defaults make every `/v1/dirs` call take 5 s whenever
/// any one service is down or hung.
///
/// `connect_timeout` catches services with no listener (port closed -> fails
/// fast at 250 ms). `timeout` caps the worst case for services that accept the
/// TCP connection but never write a response (a stuck process holding the
/// port). For local picker-decoration UX, 500 ms is generous.
const HEALTH_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const HEALTH_PROBE_TOTAL_TIMEOUT: Duration = Duration::from_millis(500);

struct ListCandidate {
    name: String,
    has_children: bool,
    modified_at: u64,
    services: Vec<String>,
    repo_dirty: Option<bool>,
    repo_action: Option<RepoActionStatus>,
    full_path: Option<String>,
}

struct PendingEntry {
    name: String,
    entry_path: PathBuf,
    has_children: bool,
    modified_at: u64,
    services: Vec<String>,
}

type RepoProbe = (Option<bool>, Option<RepoActionStatus>);

struct RestartActionPlan {
    canonical: PathBuf,
    commands: Vec<(String, String)>,
}

enum RepoSearchVisit {
    Repository,
    Descend,
    Skip,
}

#[derive(Clone)]
struct RepoSearchCacheEntry {
    roots: Vec<PathBuf>,
    max_depth: usize,
    generated_at: Instant,
    entries: Vec<DirEntry>,
}

static REPO_SEARCH_CACHE: OnceLock<Mutex<Option<RepoSearchCacheEntry>>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct ApiServiceError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiServiceError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ApiServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiServiceError {}

#[derive(Debug)]
pub enum NativeOpenServiceError {
    Unsupported { reason: Option<String> },
    NoAttentionSessions,
    SessionNotFound,
    SessionExited,
    Internal(String),
}

impl std::fmt::Display for NativeOpenServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { reason } => {
                f.write_str(reason.as_deref().unwrap_or("native desktop unavailable"))
            }
            Self::NoAttentionSessions => f.write_str("no sessions are waiting for operator input"),
            Self::SessionNotFound => f.write_str("session not found"),
            Self::SessionExited => f.write_str("session has already exited"),
            Self::Internal(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for NativeOpenServiceError {}

/// Base path for directory browsing. Prefers `DIRS_BASE_PATH` env var, then
/// the overlay's configured base path, then the server's cwd.
pub fn dirs_base_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("DIRS_BASE_PATH") {
        return PathBuf::from(explicit);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    if let Some(config) = resolve_dir_config(&cwd) {
        return config.base_path.clone();
    }

    cwd
}

pub async fn list_sessions_for_client(
    state: &Arc<AppState>,
    include_remote: bool,
) -> Vec<SessionSummary> {
    let mut sessions = state.supervisor.list_sessions().await;
    if include_remote {
        sessions.extend(remote_sessions::list_remote_sessions().await);
    }
    sessions
}

pub async fn thought_config_response(state: &Arc<AppState>) -> ThoughtConfigResponse {
    let config = state.thought_config.read().await.clone();
    ThoughtConfigResponse {
        config,
        daemon_defaults: state.current_daemon_defaults(),
        ui: thought_config_ui_metadata(&cached_or_default_openrouter_candidates()),
    }
}

pub fn validate_thought_config(config: ThoughtConfig) -> Result<ThoughtConfig, ApiServiceError> {
    config.normalize_and_validate().map_err(|err| {
        ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            err.to_string(),
        )
    })
}

pub async fn persist_validated_thought_config(
    store: &Arc<FileStore>,
    runtime_config: &mut ThoughtConfig,
    config: ThoughtConfig,
) -> Result<(), ApiServiceError> {
    if let Err(err) = store.save_thought_config(&config).await {
        tracing::error!(error = %err, "failed to persist thought runtime config");
        return Err(ApiServiceError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            "failed to persist thought config",
        ));
    }

    *runtime_config = config;
    Ok(())
}

pub async fn update_thought_config(
    state: &Arc<AppState>,
    config: ThoughtConfig,
) -> Result<ThoughtConfig, ApiServiceError> {
    let config = validate_thought_config(config)?;
    let store = state.current_file_store().ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PERSISTENCE_UNAVAILABLE",
            "thought config persistence is unavailable",
        )
    })?;

    let mut runtime_config = state.thought_config.write().await;
    persist_validated_thought_config(&store, &mut runtime_config, config.clone()).await?;
    Ok(config)
}

pub async fn test_thought_config(
    config: ThoughtConfig,
) -> Result<ThoughtConfigProbeResult, ApiServiceError> {
    let config = validate_thought_config(config)?;
    Ok(run_thought_config_probe(&config).await)
}

pub fn validate_sessions_batch_dirs(dirs: &[String]) -> Result<(), ApiServiceError> {
    if dirs.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "dirs must not be empty",
        ));
    }
    if dirs.len() > BATCH_CREATE_MAX_DIRS {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            format!("dirs must include at most {BATCH_CREATE_MAX_DIRS} entries"),
        ));
    }
    Ok(())
}

pub async fn create_local_sessions_batch(
    state: Arc<AppState>,
    dirs: Vec<String>,
    spawn_tool: Option<SpawnTool>,
    initial_request: Option<String>,
) -> Result<CreateSessionsBatchResponse, ApiServiceError> {
    validate_sessions_batch_dirs(&dirs)?;
    let total = dirs.len();
    let (batch_id, batch_label, batch_created_at, prompt_excerpt) =
        new_batch_context(total, initial_request.as_deref());
    let tasks = dirs.into_iter().enumerate().map(|(index, cwd)| {
        let supervisor = state.supervisor.clone();
        let initial_request = initial_request.clone();
        let batch = session_batch_membership(
            batch_id.clone(),
            batch_label.clone(),
            index,
            total,
            batch_created_at,
            prompt_excerpt.clone(),
        );
        async move {
            let created = supervisor
                .create_session_with_batch(
                    None,
                    Some(cwd.clone()),
                    spawn_tool,
                    initial_request,
                    Some(batch),
                )
                .await;
            create_sessions_batch_result(index, cwd, created)
        }
    });

    let mut results: Vec<_> = stream::iter(tasks)
        .buffer_unordered(BATCH_CREATE_CONCURRENCY)
        .collect()
        .await;
    results.sort_by_key(|result| result.index);
    Ok(CreateSessionsBatchResponse { results })
}

pub fn session_batch_membership(
    id: String,
    label: String,
    index: usize,
    total: usize,
    created_at: DateTime<Utc>,
    prompt_excerpt: Option<String>,
) -> SessionBatchMembership {
    SessionBatchMembership {
        id,
        label,
        index,
        total,
        created_at,
        prompt_excerpt,
    }
}

pub fn new_batch_context(
    total: usize,
    initial_request: Option<&str>,
) -> (String, String, DateTime<Utc>, Option<String>) {
    let batch_id = format!("batch-{}", Uuid::new_v4().simple());
    let created_at = Utc::now();
    let prompt_excerpt = prompt_excerpt(initial_request);
    let label = batch_label(prompt_excerpt.as_deref(), &batch_id);
    debug_assert!(total > 0);
    (batch_id, label, created_at, prompt_excerpt)
}

fn prompt_excerpt(prompt: Option<&str>) -> Option<String> {
    let normalized = prompt?.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(truncate_chars(normalized, BATCH_PROMPT_EXCERPT_MAX_CHARS))
}

fn batch_label(prompt_excerpt: Option<&str>, batch_id: &str) -> String {
    prompt_excerpt
        .map(|excerpt| truncate_chars(excerpt, BATCH_LABEL_MAX_CHARS))
        .unwrap_or_else(|| {
            let suffix = batch_id
                .strip_prefix("batch-")
                .unwrap_or(batch_id)
                .chars()
                .take(8)
                .collect::<String>();
            format!("batch {suffix}")
        })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}~")
    } else {
        truncated
    }
}

pub fn create_sessions_batch_result(
    index: usize,
    cwd: String,
    created: anyhow::Result<(SessionSummary, Option<RepoTheme>)>,
) -> CreateSessionsBatchResult {
    match created {
        Ok((session, repo_theme)) => CreateSessionsBatchResult {
            index,
            cwd,
            ok: true,
            session: Some(session),
            repo_theme,
            error: None,
        },
        Err(err) => {
            let msg = err.to_string();
            CreateSessionsBatchResult {
                index,
                cwd,
                ok: false,
                session: None,
                repo_theme: None,
                error: Some(create_session_error(&msg)),
            }
        }
    }
}

fn create_session_error(msg: &str) -> ErrorResponse {
    let code = if msg.contains("already exists") || msg.contains("duplicate session") {
        "SESSION_ALREADY_EXISTS"
    } else if msg.contains("cwd does not exist") {
        "VALIDATION_FAILED"
    } else {
        "INTERNAL_ERROR"
    };

    ErrorResponse::with_message(code, msg)
}

#[derive(Debug)]
pub enum PlanFileServiceError {
    Remote(remote_sessions::RemoteSessionError),
    SessionNotFound,
    ActorUnavailable,
    ReplyDropped,
    TimedOut,
}

impl PlanFileServiceError {
    pub fn message(&self) -> String {
        match self {
            Self::Remote(err) => err.message().to_string(),
            Self::SessionNotFound => "session not found".to_string(),
            Self::ActorUnavailable => "session actor unavailable".to_string(),
            Self::ReplyDropped => "actor dropped plan file reply".to_string(),
            Self::TimedOut => "plan file request timed out".to_string(),
        }
    }
}

impl std::fmt::Display for PlanFileServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for PlanFileServiceError {}

pub async fn request_plan_file(
    state: &Arc<AppState>,
    session_id: &str,
    name: &str,
) -> Result<PlanFileResponse, PlanFileServiceError> {
    request_plan_file_with_timeout(state, session_id, name, PLAN_FILE_TIMEOUT).await
}

pub async fn request_plan_file_with_timeout(
    state: &Arc<AppState>,
    session_id: &str,
    name: &str,
    timeout: Duration,
) -> Result<PlanFileResponse, PlanFileServiceError> {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return remote_sessions::fetch_remote_plan_file(&target, remote_session_id, name)
                .await
                .map_err(PlanFileServiceError::Remote);
        }
        Ok(None) => {}
        Err(err) => return Err(PlanFileServiceError::Remote(err)),
    }

    let handle = state
        .supervisor
        .get_session(session_id)
        .await
        .ok_or(PlanFileServiceError::SessionNotFound)?;
    request_plan_file_from_actor(&handle, name.to_string(), timeout).await
}

async fn request_plan_file_from_actor(
    handle: &ActorHandle,
    name: String,
    timeout: Duration,
) -> Result<PlanFileResponse, PlanFileServiceError> {
    let (tx, rx) = oneshot::channel::<PlanFileResponse>();
    if handle
        .send(SessionCommand::GetPlanFile { name, reply: tx })
        .await
        .is_err()
    {
        return Err(PlanFileServiceError::ActorUnavailable);
    }

    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(_)) => Err(PlanFileServiceError::ReplyDropped),
        Err(_) => Err(PlanFileServiceError::TimedOut),
    }
}

fn repo_search_cache() -> &'static Mutex<Option<RepoSearchCacheEntry>> {
    REPO_SEARCH_CACHE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
pub fn clear_repo_search_cache_for_tests() {
    if let Ok(mut cache) = repo_search_cache().lock() {
        *cache = None;
    }
}

fn repo_search_roots() -> Vec<PathBuf> {
    let configured = std::env::var_os(REPO_SEARCH_ROOTS_ENV)
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|home| vec![home.join("repos"), home.join("hard")])
                .unwrap_or_default()
        });

    let mut seen = BTreeSet::new();
    configured
        .into_iter()
        .map(expand_repo_search_root)
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let canonical = path.canonicalize().unwrap_or(path);
            seen.insert(canonical.clone()).then_some(canonical)
        })
        .collect()
}

fn expand_repo_search_root(path: PathBuf) -> PathBuf {
    let Some(raw) = path.to_str().map(|value| value.to_string()) else {
        return path;
    };
    let Some(home) = dirs::home_dir() else {
        return path;
    };
    if raw == "~" {
        return home;
    }
    raw.strip_prefix("~/")
        .map(|suffix| home.join(suffix))
        .unwrap_or(path)
}

fn repo_search_max_depth() -> usize {
    std::env::var(REPO_SEARCH_MAX_DEPTH_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|depth| *depth > 0)
        .unwrap_or(REPO_SEARCH_DEFAULT_MAX_DEPTH)
}

fn should_descend_for_repo_search(name: &str) -> bool {
    if name.starts_with('.') {
        return false;
    }

    !matches!(
        name,
        "node_modules"
            | "target"
            | "dist"
            | "build"
            | "DerivedData"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
    )
}

fn compact_repo_search_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(suffix) = path.strip_prefix(&home) {
            let suffix = suffix.to_string_lossy();
            if suffix.is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", suffix.trim_start_matches('/'));
        }
    }
    path.to_string_lossy().into_owned()
}

fn repo_search_entry(path: &Path) -> DirEntry {
    let basename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let compact = compact_repo_search_path(path);
    let name = if compact.ends_with(&basename) {
        format!("{basename}  {compact}")
    } else {
        basename
    };
    DirEntry {
        name,
        has_children: false,
        is_running: None,
        repo_dirty: None,
        repo_action: None,
        group: None,
        groups: Vec::new(),
        full_path: Some(path.to_string_lossy().into_owned()),
        has_restart: None,
        open_url: None,
    }
}

fn repo_search_queue(roots: &[PathBuf]) -> VecDeque<(PathBuf, usize)> {
    roots.iter().cloned().map(|root| (root, 0usize)).collect()
}

fn unseen_repo_search_path(path: PathBuf, seen: &mut BTreeSet<PathBuf>) -> Option<PathBuf> {
    let canonical = path.canonicalize().unwrap_or(path);
    seen.insert(canonical.clone()).then_some(canonical)
}

fn repo_search_visit(path: &Path, depth: usize, max_depth: usize) -> RepoSearchVisit {
    if path.join(".git").exists() {
        return RepoSearchVisit::Repository;
    }
    if depth >= max_depth {
        return RepoSearchVisit::Skip;
    }
    RepoSearchVisit::Descend
}

fn repo_search_child_dir_path(child: std::fs::DirEntry) -> Option<PathBuf> {
    let file_type = child.file_type().ok()?;
    file_type.is_dir().then(|| ())?;
    let name = child.file_name().to_string_lossy().into_owned();
    should_descend_for_repo_search(&name).then(|| child.path())
}

fn repo_search_child_dirs(path: &Path) -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    read_dir
        .flatten()
        .filter_map(repo_search_child_dir_path)
        .collect()
}

fn enqueue_repo_search_children(
    queue: &mut VecDeque<(PathBuf, usize)>,
    parent: &Path,
    next_depth: usize,
) {
    queue.extend(
        repo_search_child_dirs(parent)
            .into_iter()
            .map(|child| (child, next_depth)),
    );
}

fn sort_repo_search_entries(repos: &mut [DirEntry]) {
    repos.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.full_path.cmp(&right.full_path))
    });
}

fn scan_repo_search_roots_sync(roots: &[PathBuf], max_depth: usize) -> Vec<DirEntry> {
    let mut queue = repo_search_queue(roots);
    let mut seen = BTreeSet::new();
    let mut repos = Vec::new();
    while let Some((path, depth)) = queue.pop_front() {
        let Some(canonical) = unseen_repo_search_path(path, &mut seen) else {
            continue;
        };

        match repo_search_visit(&canonical, depth, max_depth) {
            RepoSearchVisit::Repository => repos.push(repo_search_entry(&canonical)),
            RepoSearchVisit::Descend => {
                enqueue_repo_search_children(&mut queue, &canonical, depth + 1);
            }
            RepoSearchVisit::Skip => {}
        }
    }

    sort_repo_search_entries(&mut repos);
    repos
}

fn cached_repo_search_entries(roots: &[PathBuf], max_depth: usize) -> Option<Vec<DirEntry>> {
    let cache = repo_search_cache().lock().ok()?;
    let cache = cache.as_ref()?;
    (cache.roots == roots
        && cache.max_depth == max_depth
        && cache.generated_at.elapsed() < REPO_SEARCH_CACHE_TTL)
        .then(|| cache.entries.clone())
}

fn write_repo_search_cache(roots: &[PathBuf], max_depth: usize, entries: &[DirEntry]) {
    if let Ok(mut cache) = repo_search_cache().lock() {
        *cache = Some(RepoSearchCacheEntry {
            roots: roots.to_vec(),
            max_depth,
            generated_at: Instant::now(),
            entries: entries.to_vec(),
        });
    }
}

pub async fn list_repo_search_entries() -> Result<DirRepoSearchResponse, ApiServiceError> {
    let roots = repo_search_roots();
    let max_depth = repo_search_max_depth();
    list_repo_search_entries_inner(roots, max_depth).await
}

async fn list_repo_search_entries_inner(
    roots: Vec<PathBuf>,
    max_depth: usize,
) -> Result<DirRepoSearchResponse, ApiServiceError> {
    let root_labels = roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return Ok(DirRepoSearchResponse {
            roots: root_labels,
            entries: Vec::new(),
        });
    }

    if let Some(entries) = cached_repo_search_entries(&roots, max_depth) {
        return Ok(DirRepoSearchResponse {
            roots: root_labels,
            entries,
        });
    }

    let scan_roots = roots.clone();
    let entries =
        tokio::task::spawn_blocking(move || scan_repo_search_roots_sync(&scan_roots, max_depth))
            .await
            .map_err(|err| {
                ApiServiceError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "REPO_SEARCH_FAILED",
                    format!("repository search task failed: {err}"),
                )
            })?;
    write_repo_search_cache(&roots, max_depth, &entries);

    Ok(DirRepoSearchResponse {
        roots: root_labels,
        entries,
    })
}
pub async fn overlay_service_health_map(
    services: &[OverlayServiceEntry],
    requested: &[String],
) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    if requested.is_empty() {
        return map;
    }

    let client = reqwest::Client::builder()
        .connect_timeout(HEALTH_PROBE_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(HEALTH_PROBE_TOTAL_TIMEOUT)
        .build()
        .unwrap_or_default();

    let mut probes = Vec::new();
    for service in services {
        if !requested.contains(&service.name) {
            continue;
        }
        let Some(url) = &service.health_url else {
            map.insert(service.name.clone(), true);
            continue;
        };
        let name = service.name.clone();
        let url = url.clone();
        let client = client.clone();
        probes.push(async move {
            let ok = client
                .get(&url)
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false);
            (name, ok)
        });
    }

    // Bound concurrency like the git-probe fan-out so a large mapped-service
    // set cannot launch an unbounded burst of in-flight HTTP requests.
    let results: Vec<(String, bool)> = stream::iter(probes)
        .buffer_unordered(GIT_PROBE_CONCURRENCY)
        .collect()
        .await;
    for (name, ok) in results {
        map.insert(name, ok);
    }

    map
}

pub async fn restart_services(
    services: &[OverlayServiceEntry],
    requested: &[String],
) -> Result<(), String> {
    if requested.is_empty() {
        return Err("no restartable services mapped for this path".to_string());
    }

    let commands = restart_commands_for_requested_services(services, requested);
    if commands.is_empty() {
        return Err("matched services have no restart command configured".to_string());
    }

    run_restart_commands(&commands).await
}

fn restart_commands_for_requested_services<'a>(
    services: &'a [OverlayServiceEntry],
    requested: &[String],
) -> Vec<RestartCommandRef<'a>> {
    services
        .iter()
        .filter(|service| requested.contains(&service.name))
        .filter_map(restart_command_for_service)
        .collect()
}

fn restart_command_for_service(service: &OverlayServiceEntry) -> Option<RestartCommandRef<'_>> {
    Some(RestartCommandRef {
        service_name: &service.name,
        command: service.restart.as_deref()?,
    })
}

async fn run_restart_commands(commands: &[RestartCommandRef<'_>]) -> Result<(), String> {
    for command in commands {
        run_restart_command(command).await?;
    }
    Ok(())
}

async fn run_restart_command(command: &RestartCommandRef<'_>) -> Result<(), String> {
    let output = restart_command_output(command).await?;
    if output.status.success() {
        return Ok(());
    }

    Err(restart_failure_message(
        command.service_name,
        &output.stdout,
        &output.stderr,
    ))
}

struct RestartCommandRef<'a> {
    service_name: &'a str,
    command: &'a str,
}

async fn probe_pending_entries(state: &Arc<AppState>, pending: &[PendingEntry]) -> Vec<RepoProbe> {
    let repo_actions = state.repo_actions.clone();
    let entry_paths = pending
        .iter()
        .map(|pending_entry| pending_entry.entry_path.clone())
        .collect::<Vec<_>>();
    stream::iter(entry_paths.into_iter().map(|entry_path| {
        let repo_actions = repo_actions.clone();
        async move {
            let repo_summary =
                inspect_git_repo(&entry_path)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|summary| {
                        let canonical_entry =
                            entry_path.canonicalize().unwrap_or(entry_path.clone());
                        (summary.repo_root == canonical_entry).then_some(summary)
                    });
            let repo_dirty = repo_summary.as_ref().map(|summary| summary.dirty);
            let repo_action = match repo_summary.as_ref() {
                Some(summary) => repo_actions.status_for(&summary.repo_root).await,
                None => None,
            };
            (repo_dirty, repo_action)
        }
    }))
    .buffered(GIT_PROBE_CONCURRENCY)
    .collect()
    .await
}

fn service_metadata_map(services: &[OverlayServiceEntry]) -> HashMap<&str, &OverlayServiceEntry> {
    services
        .iter()
        .map(|service| (service.name.as_str(), service))
        .collect()
}

fn service_entry_metadata(
    services: &[String],
    health_map: &HashMap<String, bool>,
    svc_meta: &HashMap<&str, &OverlayServiceEntry>,
) -> (Option<bool>, Option<bool>, Option<String>) {
    let is_running = if services.is_empty() {
        None
    } else {
        Some(
            services
                .iter()
                .any(|service| health_map.get(service).copied().unwrap_or(false)),
        )
    };
    let has_restart = services
        .iter()
        .any(|service| {
            svc_meta
                .get(service.as_str())
                .and_then(|entry| entry.restart.as_ref())
                .is_some()
        })
        .then_some(true);
    let open_url = services.iter().find_map(|service| {
        svc_meta
            .get(service.as_str())
            .and_then(|entry| entry.open_url.clone().or_else(|| entry.health_url.clone()))
    });

    (is_running, has_restart, open_url)
}

fn pending_to_candidates(
    pending: Vec<PendingEntry>,
    probes: Vec<RepoProbe>,
    include_full_path: bool,
) -> Vec<ListCandidate> {
    pending
        .into_iter()
        .zip(probes)
        .map(|(pending_entry, (repo_dirty, repo_action))| {
            let full_path =
                include_full_path.then(|| pending_entry.entry_path.to_string_lossy().into_owned());
            ListCandidate {
                name: pending_entry.name,
                has_children: pending_entry.has_children,
                modified_at: pending_entry.modified_at,
                services: pending_entry.services,
                repo_dirty,
                repo_action,
                full_path,
            }
        })
        .collect()
}

fn build_dir_entries(
    candidates: Vec<ListCandidate>,
    health_map: &HashMap<String, bool>,
    svc_meta: &HashMap<&str, &OverlayServiceEntry>,
) -> Vec<DirEntry> {
    let mut entries: Vec<(DirEntry, u64)> = candidates
        .into_iter()
        .map(|candidate| {
            let (is_running, has_restart, open_url) =
                service_entry_metadata(&candidate.services, health_map, svc_meta);
            (
                DirEntry {
                    name: candidate.name,
                    has_children: candidate.has_children,
                    is_running,
                    repo_dirty: candidate.repo_dirty,
                    repo_action: candidate.repo_action,
                    group: None,
                    groups: Vec::new(),
                    full_path: candidate.full_path,
                    has_restart,
                    open_url,
                },
                candidate.modified_at,
            )
        })
        .collect();

    entries.sort_by(|(a, a_modified), (b, b_modified)| {
        b_modified
            .cmp(a_modified)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries.into_iter().map(|(entry, _)| entry).collect()
}

fn collect_visible_pending_entries(
    read_dir: std::fs::ReadDir,
    service_context: Option<&OverlayServiceContext>,
) -> (Vec<PendingEntry>, BTreeSet<String>) {
    let mut pending = Vec::new();
    let mut unique_services = BTreeSet::new();
    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }

        let entry_path = entry.path();
        let has_children = has_visible_child_dirs(&entry_path);
        let modified_at = modified_secs(&entry_path);

        let services = service_context
            .map(|context| services_for_directory(&entry_path, context))
            .unwrap_or_default();
        extend_unique_services(&mut unique_services, &services);

        pending.push(PendingEntry {
            name,
            entry_path,
            has_children,
            modified_at,
            services,
        });
    }

    (pending, unique_services)
}

fn extend_unique_services(unique_services: &mut BTreeSet<String>, services: &[String]) {
    unique_services.extend(services.iter().cloned());
}

fn managed_service_context(config: &OverlayDirConfig) -> OverlayServiceContext {
    OverlayServiceContext {
        base_path: config
            .base_path
            .canonicalize()
            .unwrap_or(config.base_path.clone()),
        services: config.services.clone(),
    }
}

fn collect_managed_service_pending_entries(
    context: &OverlayServiceContext,
) -> (Vec<PendingEntry>, BTreeSet<String>) {
    let mut seen_paths = BTreeSet::new();
    let mut unique_services = BTreeSet::new();
    let mut pending = Vec::new();

    for service in &context.services {
        let Some(entry) = managed_service_pending_entry(service, context, &mut seen_paths) else {
            continue;
        };
        extend_unique_services(&mut unique_services, &entry.services);
        pending.push(entry);
    }

    (pending, unique_services)
}

fn managed_service_pending_entry(
    service: &OverlayServiceEntry,
    context: &OverlayServiceContext,
    seen_paths: &mut BTreeSet<PathBuf>,
) -> Option<PendingEntry> {
    let entry_path = managed_service_entry_path(&context.base_path, service, seen_paths)?;
    let services = services_for_directory(&entry_path, context);

    Some(PendingEntry {
        name: managed_service_entry_name(&entry_path, service),
        has_children: false,
        modified_at: modified_secs(&entry_path),
        services,
        entry_path,
    })
}

fn managed_service_entry_path(
    base_path: &Path,
    service: &OverlayServiceEntry,
    seen_paths: &mut BTreeSet<PathBuf>,
) -> Option<PathBuf> {
    let raw_path = service_dir_path(base_path, &service.dir);
    let entry_path = raw_path.canonicalize().ok()?;
    (entry_path.is_dir() && seen_paths.insert(entry_path.clone())).then_some(entry_path)
}

fn managed_service_entry_name(entry_path: &Path, service: &OverlayServiceEntry) -> String {
    entry_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| service.name.clone())
}

pub async fn list_managed_service_entries(
    state: &Arc<AppState>,
    config: &OverlayDirConfig,
) -> Vec<DirEntry> {
    let context = managed_service_context(config);
    let (pending, unique_services) = collect_managed_service_pending_entries(&context);

    let probes = probe_pending_entries(state, &pending).await;
    let services: Vec<String> = unique_services.into_iter().collect();
    let health_map = overlay_service_health_map(&config.services, &services).await;
    let svc_meta = service_metadata_map(&config.services);
    let candidates = pending_to_candidates(pending, probes, true);
    build_dir_entries(candidates, &health_map, &svc_meta)
}

async fn load_dir_group_memberships(state: &Arc<AppState>) -> DirGroupMemberships {
    match state.current_file_store() {
        Some(store) => store.load_dir_group_memberships().await,
        None => DirGroupMemberships::default(),
    }
}

pub async fn list_dirs(
    state: &Arc<AppState>,
    path: Option<&str>,
    managed_only: bool,
    group: Option<&str>,
) -> Result<DirListResponse, ApiServiceError> {
    let base = dirs_base_path();
    let memberships = load_dir_group_memberships(state).await;

    if let Some(group_name) = group {
        return list_group_dir_response(base, group_name, &memberships).await;
    }

    let request_started = Instant::now();
    let target = path
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| base.clone());

    let (canonical_base, canonical) = resolve_target_path(base, target)?;
    let dir_config = effective_dir_config_for_base(&canonical_base);
    if let Some(response) = list_managed_root_response(
        state,
        canonical_base.as_path(),
        canonical.as_path(),
        dir_config.as_ref(),
        &memberships,
        managed_only,
        request_started,
    )
    .await
    {
        return Ok(response);
    }

    list_regular_dir_response(
        state,
        canonical.as_path(),
        dir_config.as_ref(),
        &memberships,
        managed_only,
        request_started,
    )
    .await
}

pub async fn update_dir_group_memberships(
    state: Arc<AppState>,
    body: DirGroupMembershipUpdateRequest,
) -> Result<DirGroupMembershipUpdateResponse, ApiServiceError> {
    let preflight = update_dir_group_memberships_preflight(
        state.current_file_store(),
        dirs_base_path(),
        effective_dir_config_for_base,
    )?;

    update_dir_group_memberships_with_config(
        preflight.store,
        &preflight.canonical_base,
        &preflight.dir_config,
        body,
    )
    .await
}

struct DirGroupMembershipUpdatePreflight {
    store: Arc<FileStore>,
    canonical_base: PathBuf,
    dir_config: OverlayDirConfig,
}

fn update_dir_group_memberships_preflight(
    store: Option<Arc<FileStore>>,
    base: PathBuf,
    dir_config_for_base: impl FnOnce(&Path) -> Option<OverlayDirConfig>,
) -> Result<DirGroupMembershipUpdatePreflight, ApiServiceError> {
    let store = store.ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PERSISTENCE_UNAVAILABLE",
            "directory group edits require file persistence",
        )
    })?;

    let canonical_base = base.canonicalize().unwrap_or(base);
    let dir_config = dir_config_for_base(&canonical_base).ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OVERLAY_UNAVAILABLE",
            "directory group edits require a configured directory group source",
        )
    })?;
    if dir_config.groups.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "GROUPS_UNAVAILABLE",
            "no directory groups are configured",
        ));
    }

    Ok(DirGroupMembershipUpdatePreflight {
        store,
        canonical_base,
        dir_config,
    })
}

async fn update_dir_group_memberships_with_config(
    store: Arc<FileStore>,
    canonical_base: &Path,
    dir_config: &OverlayDirConfig,
    body: DirGroupMembershipUpdateRequest,
) -> Result<DirGroupMembershipUpdateResponse, ApiServiceError> {
    let canonical_path = resolve_group_membership_path(canonical_base, &body.path, dir_config)?;
    let available_groups = dir_groups(Some(dir_config));
    let valid_groups = available_groups.iter().cloned().collect::<BTreeSet<_>>();
    let add = normalize_group_update_names(&body.add, &valid_groups)?;
    let remove = normalize_group_update_names(&body.remove, &valid_groups)?;
    if add.is_empty() && remove.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "GROUP_UPDATE_EMPTY",
            "at least one group must be added or removed",
        ));
    }

    let path = canonical_path_string(&canonical_path);
    let update_path = path.clone();
    let memberships = store
        .update_dir_group_memberships(move |memberships| {
            apply_group_membership_update(memberships, &update_path, add, remove);
        })
        .await
        .map_err(|error| {
            ApiServiceError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "GROUP_UPDATE_FAILED",
                format!("failed to persist directory group edits: {error}"),
            )
        })?;

    Ok(DirGroupMembershipUpdateResponse {
        groups: effective_groups_for_path(dir_config, &memberships, &canonical_path),
        available_groups,
        path,
    })
}

fn resolve_group_membership_path(
    canonical_base: &Path,
    raw_path: &str,
    config: &OverlayDirConfig,
) -> Result<PathBuf, ApiServiceError> {
    let trimmed = require_group_membership_path(raw_path)?;
    let canonical = canonical_group_membership_dir(trimmed)?;
    if group_membership_path_allowed(canonical_base, &canonical, config) {
        return Ok(canonical);
    }

    Err(group_membership_outside_roots_error())
}

fn require_group_membership_path(raw_path: &str) -> Result<&str, ApiServiceError> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "GROUP_PATH_REQUIRED",
            "path is required",
        ));
    }
    Ok(trimmed)
}

fn canonical_group_membership_dir(path: &str) -> Result<PathBuf, ApiServiceError> {
    let canonical = PathBuf::from(path)
        .canonicalize()
        .map_err(|_| group_membership_dir_not_found_error(path))?;
    if !canonical.is_dir() {
        return Err(group_membership_dir_not_found_error(path));
    }
    Ok(canonical)
}

fn group_membership_dir_not_found_error(path: &str) -> ApiServiceError {
    ApiServiceError::new(
        StatusCode::NOT_FOUND,
        "DIR_NOT_FOUND",
        format!("directory not found: {path}"),
    )
}

fn group_membership_path_allowed(
    canonical_base: &Path,
    canonical: &Path,
    config: &OverlayDirConfig,
) -> bool {
    canonical.starts_with(canonical_base)
        || config
            .groups
            .iter()
            .any(|group| overlay_group_contains_path(group, canonical))
}

fn group_membership_outside_roots_error() -> ApiServiceError {
    ApiServiceError::new(
        StatusCode::FORBIDDEN,
        "DIR_OUTSIDE_BASE",
        "path is outside the allowed directory group roots",
    )
}

fn normalize_group_update_names(
    groups: &[String],
    valid_groups: &BTreeSet<String>,
) -> Result<Vec<String>, ApiServiceError> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for raw in groups {
        let name = raw.trim();
        if name.is_empty() {
            return Err(ApiServiceError::new(
                StatusCode::BAD_REQUEST,
                "GROUP_NAME_REQUIRED",
                "group names must not be empty",
            ));
        }
        if !valid_groups.contains(name) {
            return Err(ApiServiceError::new(
                StatusCode::NOT_FOUND,
                "GROUP_NOT_FOUND",
                format!("no group named '{name}' in overlay"),
            ));
        }
        if seen.insert(name.to_string()) {
            normalized.push(name.to_string());
        }
    }
    Ok(normalized)
}

fn prune_empty_group_deltas(memberships: &mut DirGroupMemberships) {
    memberships
        .groups
        .retain(|_, delta| !delta.include_paths.is_empty() || !delta.exclude_paths.is_empty());
}

fn apply_group_membership_update(
    memberships: &mut DirGroupMemberships,
    path: &str,
    add: Vec<String>,
    remove: Vec<String>,
) {
    apply_group_membership_removes(memberships, path, remove);
    apply_group_membership_adds(memberships, path, add);
    prune_empty_group_deltas(memberships);
}

fn apply_group_membership_removes(
    memberships: &mut DirGroupMemberships,
    path: &str,
    groups: Vec<String>,
) {
    for group in groups {
        apply_group_membership_remove(memberships, path, group);
    }
}

fn apply_group_membership_adds(
    memberships: &mut DirGroupMemberships,
    path: &str,
    groups: Vec<String>,
) {
    for group in groups {
        apply_group_membership_add(memberships, path, group);
    }
}

fn apply_group_membership_remove(memberships: &mut DirGroupMemberships, path: &str, group: String) {
    let delta = memberships.groups.entry(group).or_default();
    delta.include_paths.remove(path);
    delta.exclude_paths.insert(path.to_string());
}

fn apply_group_membership_add(memberships: &mut DirGroupMemberships, path: &str, group: String) {
    let delta = memberships.groups.entry(group).or_default();
    delta.exclude_paths.remove(path);
    delta.include_paths.insert(path.to_string());
}

async fn list_group_dir_response(
    base: PathBuf,
    group_name: &str,
    memberships: &DirGroupMemberships,
) -> Result<DirListResponse, ApiServiceError> {
    let canonical_base = base.canonicalize().unwrap_or(base.clone());
    let dir_config = effective_dir_config_for_base(&canonical_base);
    let group = dir_config
        .as_ref()
        .and_then(|config| config.groups.iter().find(|g| g.name == group_name));
    let Some(group) = group else {
        return Err(ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "GROUP_NOT_FOUND",
            format!("no group named '{group_name}' in overlay"),
        ));
    };
    let mut entries = list_effective_group_entries(group, memberships).await;
    if let Some(config) = dir_config.as_ref() {
        annotate_dir_entry_groups(&mut entries, &canonical_base, config, memberships);
    }
    Ok(DirListResponse {
        path: canonical_base.to_string_lossy().into_owned(),
        entries,
        overlay_label: dir_config.as_ref().map(|c| c.label.clone()),
        groups: dir_groups(dir_config.as_ref()),
        launch_targets: launch_targets_for(dir_config.as_ref()),
        default_launch_target: default_launch_target_for(dir_config.as_ref(), Some(group_name)),
    })
}

async fn list_managed_root_response(
    state: &Arc<AppState>,
    canonical_base: &Path,
    canonical: &Path,
    dir_config: Option<&OverlayDirConfig>,
    memberships: &DirGroupMemberships,
    managed_only: bool,
    request_started: Instant,
) -> Option<DirListResponse> {
    if !managed_only || canonical != canonical_base {
        return None;
    }

    let config = dir_config.filter(|config| !config.services.is_empty())?;
    let pending_started = Instant::now();
    let mut entries = list_managed_service_entries(state, config).await;
    annotate_dir_entry_groups(&mut entries, canonical_base, config, memberships);
    let total_ms = request_started.elapsed().as_millis() as u64;
    tracing::info!(
        target: "swimmers::api::dirs::timing",
        managed_only,
        pending_count = entries.len(),
        pending_phase_ms = pending_started.elapsed().as_millis() as u64,
        probe_phase_ms = 0_u64,
        health_phase_ms = 0_u64,
        total_ms,
        "list_dirs managed exact entries timing"
    );
    Some(DirListResponse {
        path: canonical.to_string_lossy().into_owned(),
        entries,
        overlay_label: Some(config.label.clone()),
        groups: dir_groups(Some(config)),
        launch_targets: launch_targets_for(Some(config)),
        default_launch_target: default_launch_target_for(Some(config), None),
    })
}

async fn list_regular_dir_response(
    state: &Arc<AppState>,
    canonical: &Path,
    dir_config: Option<&OverlayDirConfig>,
    memberships: &DirGroupMemberships,
    managed_only: bool,
    request_started: Instant,
) -> Result<DirListResponse, ApiServiceError> {
    let read_dir = std::fs::read_dir(canonical).map_err(|error| {
        ApiServiceError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DIR_READ_ERROR",
            error.to_string(),
        )
    })?;

    let service_context = dir_config.map(|config| OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    });

    let (pending, unique_services) =
        collect_visible_pending_entries(read_dir, service_context.as_ref());
    let pending_phase_ms = request_started.elapsed().as_millis() as u64;
    let pending_count = pending.len();
    let probe_started = Instant::now();
    let probes = probe_pending_entries(state, &pending).await;
    let probe_phase_ms = probe_started.elapsed().as_millis() as u64;
    let candidates = pending_to_candidates(pending, probes, false);

    let health_started = Instant::now();
    let health_map = if let Some(config) = dir_config {
        let services: Vec<String> = unique_services.into_iter().collect();
        overlay_service_health_map(&config.services, &services).await
    } else {
        HashMap::new()
    };
    let health_phase_ms = health_started.elapsed().as_millis() as u64;
    let total_ms = request_started.elapsed().as_millis() as u64;
    tracing::info!(
        target: "swimmers::api::dirs::timing",
        managed_only,
        pending_count,
        pending_phase_ms,
        probe_phase_ms,
        health_phase_ms,
        total_ms,
        "list_dirs phase timing"
    );

    let svc_meta = dir_config
        .map(|config| service_metadata_map(&config.services))
        .unwrap_or_default();
    let mut entries = build_dir_entries(candidates, &health_map, &svc_meta);
    if let Some(config) = dir_config {
        annotate_dir_entry_groups(&mut entries, canonical, config, memberships);
    }
    let groups = dir_config
        .map(|config| {
            config
                .groups
                .iter()
                .map(|group| group.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(DirListResponse {
        path: canonical.to_string_lossy().into_owned(),
        entries,
        overlay_label: dir_config.map(|c| c.label.clone()),
        groups,
        launch_targets: launch_targets_for(dir_config),
        default_launch_target: default_launch_target_for(dir_config, None),
    })
}

fn dir_groups(config: Option<&OverlayDirConfig>) -> Vec<String> {
    config
        .map(|config| {
            config
                .groups
                .iter()
                .map(|group| group.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn launch_targets_for(config: Option<&OverlayDirConfig>) -> Vec<LaunchTargetSummary> {
    config
        .map(|config| config.launch.targets.clone())
        .filter(|targets| !targets.is_empty())
        .unwrap_or_else(|| vec![LaunchTargetSummary::local()])
}

fn default_launch_target_for(
    config: Option<&OverlayDirConfig>,
    group: Option<&str>,
) -> Option<String> {
    Some(
        config
            .map(|config| config.launch.default_for_group(group))
            .unwrap_or_else(|| "local".to_string()),
    )
}

fn repo_action_error(error: &io::Error) -> ApiServiceError {
    match error.kind() {
        io::ErrorKind::AlreadyExists => ApiServiceError::new(
            StatusCode::CONFLICT,
            "ACTION_ALREADY_RUNNING",
            error.to_string(),
        ),
        _ => ApiServiceError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ACTION_START_FAILED",
            error.to_string(),
        ),
    }
}

fn require_repo_action_path(path: &str) -> Result<&str, ApiServiceError> {
    let requested_path = path.trim();
    if requested_path.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "path is required",
        ));
    }
    Ok(requested_path)
}

fn restart_action_config(
    canonical_base: &Path,
) -> Result<&'static OverlayDirConfig, ApiServiceError> {
    resolve_dir_config(canonical_base).ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_OVERLAY",
            "no overlay configuration found for this path",
        )
    })
}

fn collect_restart_commands(
    services: &[OverlayServiceEntry],
    matched_services: &[String],
) -> Vec<(String, String)> {
    services
        .iter()
        .filter(|service| matched_services.contains(&service.name))
        .filter_map(|service| {
            service
                .restart
                .as_ref()
                .map(|command| (service.name.clone(), command.clone()))
        })
        .collect()
}

fn restart_commands_for_matched_services(
    services: &[OverlayServiceEntry],
    matched_services: &[String],
) -> Result<Vec<(String, String)>, ApiServiceError> {
    if matched_services.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_SERVICE_FOR_PATH",
            "no overlay service is mapped to this folder",
        ));
    }

    let commands = collect_restart_commands(services, matched_services);
    if commands.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_RESTART_COMMAND",
            "matched services have no restart command configured",
        ));
    }
    Ok(commands)
}

fn restart_commands_for_path(
    canonical: &Path,
    config: &OverlayDirConfig,
) -> Result<Vec<(String, String)>, ApiServiceError> {
    let context = OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    };
    let matched_services = services_for_directory(canonical, &context);
    restart_commands_for_matched_services(&config.services, &matched_services)
}

fn plan_restart_action(path: &str) -> Result<RestartActionPlan, ApiServiceError> {
    let requested_path = require_repo_action_path(path)?;
    let target = PathBuf::from(requested_path);
    let (canonical_base, canonical) = resolve_target_path(dirs_base_path(), target)?;
    let config = restart_action_config(&canonical_base)?;
    let commands = restart_commands_for_path(&canonical, &config)?;

    Ok(RestartActionPlan {
        canonical,
        commands,
    })
}

pub async fn start_restart_action(
    state: Arc<AppState>,
    path: &str,
    kind: RepoActionKind,
) -> Result<DirRepoActionResponse, ApiServiceError> {
    let RestartActionPlan {
        canonical,
        commands,
    } = plan_restart_action(path)?;

    let executor: Arc<dyn RepoActionExecutor> = Arc::new(RestartExecutor { commands });
    state
        .repo_actions
        .start(canonical.clone(), kind, executor)
        .await
        .map_err(|error| repo_action_error(&error))?;

    let status = state
        .repo_actions
        .status_for(&canonical)
        .await
        .unwrap_or(RepoActionStatus {
            kind,
            state: RepoActionState::Running,
            detail: None,
        });

    Ok(DirRepoActionResponse {
        ok: true,
        path: canonical.to_string_lossy().into_owned(),
        status,
    })
}

pub async fn start_repo_action_with_executor(
    state: Arc<AppState>,
    path: &str,
    kind: RepoActionKind,
    executor: Arc<dyn RepoActionExecutor>,
) -> Result<DirRepoActionResponse, ApiServiceError> {
    let requested_path = path.trim();
    if requested_path.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "path is required",
        ));
    }

    let base = dirs_base_path();
    let target = PathBuf::from(requested_path);
    let (_canonical_base, canonical) = resolve_target_path(base, target)?;

    let Some(repo_summary) = inspect_git_repo(&canonical).await.ok().flatten() else {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_GIT_REPO",
            "path is not inside a git repository",
        ));
    };

    if !repo_summary.dirty {
        return Err(ApiServiceError::new(
            StatusCode::CONFLICT,
            "REPO_CLEAN",
            "repo has no pending changes to commit",
        ));
    }

    state
        .repo_actions
        .start(repo_summary.repo_root.clone(), kind, executor)
        .await
        .map_err(|error| repo_action_error(&error))?;

    // The action may flip the repo's dirty state. Drop the cached probe so
    // the next `inspect_git_repo` call re-runs git instead of returning the
    // pre-action snapshot.
    crate::host_actions::invalidate_inspect_git_repo(&repo_summary.repo_root);
    if canonical != repo_summary.repo_root {
        crate::host_actions::invalidate_inspect_git_repo(&canonical);
    }

    let status = state
        .repo_actions
        .status_for(&repo_summary.repo_root)
        .await
        .unwrap_or(RepoActionStatus {
            kind,
            state: RepoActionState::Running,
            detail: None,
        });

    Ok(DirRepoActionResponse {
        ok: true,
        path: repo_summary.repo_root.to_string_lossy().into_owned(),
        status,
    })
}

pub async fn start_dir_repo_action(
    state: Arc<AppState>,
    path: &str,
    kind: RepoActionKind,
) -> Result<DirRepoActionResponse, ApiServiceError> {
    match kind {
        RepoActionKind::Restart => start_restart_action(state, path, kind).await,
        RepoActionKind::Open => Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "CLIENT_ONLY",
            "open actions are handled client-side",
        )),
        _ => {
            start_repo_action_with_executor(state, path, kind, Arc::new(SystemRepoActionExecutor))
                .await
        }
    }
}

pub async fn restart_dir_services(path: &str) -> Result<DirRestartResponse, ApiServiceError> {
    let requested_path = path.trim();
    if requested_path.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "path is required",
        ));
    }

    let base = dirs_base_path();
    let target = PathBuf::from(requested_path);
    let (canonical_base, canonical) = resolve_target_path(base, target)?;

    let Some(config) = resolve_dir_config(&canonical_base) else {
        return Err(ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OVERLAY_UNAVAILABLE",
            "no skillbox-config overlay found with service definitions",
        ));
    };

    let context = OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    };
    let matched_services = services_for_directory(&canonical, &context);
    if matched_services.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_SERVICE_FOR_PATH",
            "no overlay service is mapped to this folder",
        ));
    }

    restart_services(&config.services, &matched_services)
        .await
        .map_err(|message| {
            ApiServiceError::new(StatusCode::INTERNAL_SERVER_ERROR, "RESTART_FAILED", message)
        })?;

    Ok(DirRestartResponse {
        ok: true,
        path: canonical.to_string_lossy().into_owned(),
        services: matched_services,
    })
}

pub async fn native_status_for_host(
    state: &Arc<AppState>,
    host: &str,
) -> NativeDesktopStatusResponse {
    let app = *state.native_desktop_app.read().await;
    let ghostty_mode = *state.ghostty_open_mode.read().await;
    let mut status = native::support_for_host(host, app);
    if app == NativeDesktopApp::Ghostty {
        status.ghostty_mode = Some(ghostty_mode);
    }
    status
}

pub async fn open_native_session_for_host(
    state: &Arc<AppState>,
    host: &str,
    session_id: &str,
) -> Result<NativeDesktopOpenResponse, NativeOpenServiceError> {
    let app = *state.native_desktop_app.read().await;
    let ghostty_mode = *state.ghostty_open_mode.read().await;
    let status = native::support_for_host(host, app);
    if !status.supported {
        return Err(NativeOpenServiceError::Unsupported {
            reason: status.reason,
        });
    }

    let summary = fetch_live_summary(state, session_id)
        .await
        .map_err(|error| NativeOpenServiceError::Internal(error.to_string()))?
        .ok_or(NativeOpenServiceError::SessionNotFound)?;

    if summary.state == SessionState::Exited {
        return Err(NativeOpenServiceError::SessionExited);
    }

    native::open_native_session(
        app,
        ghostty_mode,
        &summary.session_id,
        &summary.tmux_name,
        &summary.cwd,
    )
    .await
    .map_err(|error| NativeOpenServiceError::Internal(error.to_string()))
}

pub async fn open_native_attention_group_for_host(
    state: &Arc<AppState>,
    host: &str,
    request: NativeAttentionGroupOpenRequest,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    let app = *state.native_desktop_app.read().await;
    let ghostty_mode = *state.ghostty_open_mode.read().await;
    let status = native::support_for_host(host, app);
    let plan = native_attention_group_plan(state, &request).await;

    open_native_attention_group_plan(app, ghostty_mode, request, status.supported, plan).await
}

async fn native_attention_group_plan(
    state: &Arc<AppState>,
    request: &NativeAttentionGroupOpenRequest,
) -> AttentionGroupPlan {
    plan_attention_group_sessions(
        state.supervisor.list_sessions().await,
        request.max_sessions.unwrap_or(6),
        &request.current_session_ids,
        request.include_unnumbered_sessions,
    )
}

async fn open_native_attention_group_plan(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    request: NativeAttentionGroupOpenRequest,
    native_supported: bool,
    plan: AttentionGroupPlan,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    if plan.visible.is_empty() {
        return handle_empty_attention_group_plan(&request).await;
    }

    open_visible_native_attention_group(app, ghostty_mode, &request, native_supported, plan).await
}

async fn handle_empty_attention_group_plan(
    request: &NativeAttentionGroupOpenRequest,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    match empty_attention_group_plan_outcome(request) {
        EmptyAttentionGroupPlanOutcome::ClearNative => native::clear_native_attention_group()
            .await
            .map_err(|error| NativeOpenServiceError::Internal(error.to_string())),
        EmptyAttentionGroupPlanOutcome::NoAttentionSessions => {
            Err(NativeOpenServiceError::NoAttentionSessions)
        }
    }
}

fn empty_attention_group_plan_outcome(
    request: &NativeAttentionGroupOpenRequest,
) -> EmptyAttentionGroupPlanOutcome {
    if !request.focus && !request.current_session_ids.is_empty() {
        EmptyAttentionGroupPlanOutcome::ClearNative
    } else {
        EmptyAttentionGroupPlanOutcome::NoAttentionSessions
    }
}

async fn open_visible_native_attention_group(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    request: &NativeAttentionGroupOpenRequest,
    native_supported: bool,
    plan: AttentionGroupPlan,
) -> Result<NativeAttentionGroupOpenResponse, NativeOpenServiceError> {
    let response = native::open_native_attention_group(
        app,
        ghostty_mode,
        &plan.visible,
        request.focus && native_supported,
        request.layout.unwrap_or_default(),
    )
    .await
    .map_err(|error| NativeOpenServiceError::Internal(error.to_string()))?;

    Ok(response_with_attention_backlog(response, &plan))
}

fn response_with_attention_backlog(
    mut response: NativeAttentionGroupOpenResponse,
    plan: &AttentionGroupPlan,
) -> NativeAttentionGroupOpenResponse {
    response.backlog_session_ids = plan
        .backlog
        .iter()
        .map(|session| session.session_id.clone())
        .collect();
    response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyAttentionGroupPlanOutcome {
    ClearNative,
    NoAttentionSessions,
}

#[derive(Debug, Clone)]
struct AttentionGroupPlan {
    visible: Vec<SessionSummary>,
    backlog: Vec<SessionSummary>,
}

#[derive(Debug, Clone)]
struct AttentionCandidate {
    session: SessionSummary,
    repo: String,
    family: String,
    batch: Option<String>,
}

fn plan_attention_group_sessions(
    sessions: Vec<SessionSummary>,
    max_sessions: usize,
    current_session_ids: &[String],
    include_unnumbered_sessions: bool,
) -> AttentionGroupPlan {
    let limit = max_sessions.clamp(1, 6);
    let mut candidates = attention_group_candidates(sessions, include_unnumbered_sessions);
    if candidates.is_empty() {
        return AttentionGroupPlan {
            visible: Vec::new(),
            backlog: Vec::new(),
        };
    }

    let current_ids = current_session_ids.iter().collect::<HashSet<_>>();
    let mut visible =
        retain_current_attention_group_candidates(&mut candidates, current_session_ids, limit);
    fill_attention_group_candidates(&mut visible, &mut candidates, limit);
    sort_attention_backlog_candidates(&mut candidates, &visible);

    AttentionGroupPlan {
        visible: attention_sessions_from_candidates(visible),
        backlog: attention_backlog_sessions(candidates, &current_ids),
    }
}

fn attention_group_candidates(
    sessions: Vec<SessionSummary>,
    include_unnumbered_sessions: bool,
) -> Vec<AttentionCandidate> {
    sessions
        .into_iter()
        .filter(attention_group_session_is_eligible)
        .filter(|session| include_unnumbered_sessions || tmux_name_is_numbered(&session.tmux_name))
        .map(AttentionCandidate::from)
        .collect()
}

fn retain_current_attention_group_candidates(
    candidates: &mut Vec<AttentionCandidate>,
    current_session_ids: &[String],
    limit: usize,
) -> Vec<AttentionCandidate> {
    let mut visible = Vec::new();
    for session_id in current_session_ids {
        if visible.len() >= limit {
            break;
        }
        if let Some(index) = candidates
            .iter()
            .position(|candidate| candidate.session.session_id == *session_id)
        {
            visible.push(candidates.remove(index));
        }
    }
    visible
}

fn fill_attention_group_candidates(
    visible: &mut Vec<AttentionCandidate>,
    candidates: &mut Vec<AttentionCandidate>,
    limit: usize,
) {
    if visible.is_empty() {
        let anchor_index = best_attention_anchor_index(&candidates);
        visible.push(candidates.remove(anchor_index));
    }

    while visible.len() < limit && !candidates.is_empty() {
        let next_index = best_attention_fill_index(&visible, &candidates);
        visible.push(candidates.remove(next_index));
    }
}

fn sort_attention_backlog_candidates(
    candidates: &mut [AttentionCandidate],
    visible: &[AttentionCandidate],
) {
    candidates.sort_by(|a, b| {
        best_adjacency_to_group(b, visible)
            .cmp(&best_adjacency_to_group(a, visible))
            .then_with(|| b.session.last_activity_at.cmp(&a.session.last_activity_at))
            .then_with(|| a.session.session_id.cmp(&b.session.session_id))
    });
}

fn attention_sessions_from_candidates(candidates: Vec<AttentionCandidate>) -> Vec<SessionSummary> {
    candidates
        .into_iter()
        .map(|candidate| candidate.session)
        .collect()
}

fn attention_backlog_sessions(
    candidates: Vec<AttentionCandidate>,
    current_ids: &HashSet<&String>,
) -> Vec<SessionSummary> {
    candidates
        .into_iter()
        .filter(|candidate| !current_ids.contains(&candidate.session.session_id))
        .map(|candidate| candidate.session)
        .collect()
}

fn attention_group_session_is_eligible(session: &SessionSummary) -> bool {
    session.session_id != NATIVE_ATTENTION_GROUP_SESSION_ID
        && session.tmux_name != NATIVE_ATTENTION_GROUP_TMUX_NAME
        && remote_sessions::split_remote_session_id(&session.session_id).is_none()
        && session_ready_for_operator_group_input(session)
}

fn tmux_name_is_numbered(tmux_name: &str) -> bool {
    !tmux_name.is_empty() && tmux_name.chars().all(|ch| ch.is_ascii_digit())
}

impl From<SessionSummary> for AttentionCandidate {
    fn from(session: SessionSummary) -> Self {
        let repo = attention_repo_key(&session.cwd);
        let family = attention_project_family(&repo);
        let batch = session.batch.as_ref().map(|batch| batch.id.clone());
        Self {
            session,
            repo,
            family,
            batch,
        }
    }
}

fn best_attention_anchor_index(candidates: &[AttentionCandidate]) -> usize {
    candidates
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            attention_anchor_score(a, candidates)
                .cmp(&attention_anchor_score(b, candidates))
                .then_with(|| a.session.last_activity_at.cmp(&b.session.last_activity_at))
                .then_with(|| b.session.session_id.cmp(&a.session.session_id))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn attention_anchor_score(
    candidate: &AttentionCandidate,
    candidates: &[AttentionCandidate],
) -> i32 {
    candidates
        .iter()
        .filter(|other| other.session.session_id != candidate.session.session_id)
        .map(|other| attention_adjacency_score(candidate, other))
        .sum()
}

fn best_attention_fill_index(
    visible: &[AttentionCandidate],
    candidates: &[AttentionCandidate],
) -> usize {
    best_attention_fill_choice(visible, candidates).unwrap_or(0)
}

fn best_attention_fill_choice(
    visible: &[AttentionCandidate],
    candidates: &[AttentionCandidate],
) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .max_by_key(|(_, candidate)| attention_fill_rank(candidate, visible))
        .map(|(index, _)| index)
}

fn attention_fill_rank(
    candidate: &AttentionCandidate,
    visible: &[AttentionCandidate],
) -> (i32, DateTime<Utc>, Reverse<String>) {
    (
        best_adjacency_to_group(candidate, visible),
        candidate.session.last_activity_at,
        Reverse(candidate.session.session_id.clone()),
    )
}

fn best_adjacency_to_group(candidate: &AttentionCandidate, visible: &[AttentionCandidate]) -> i32 {
    visible
        .iter()
        .map(|visible| attention_adjacency_score(candidate, visible))
        .max()
        .unwrap_or(0)
}

fn attention_adjacency_score(a: &AttentionCandidate, b: &AttentionCandidate) -> i32 {
    if attention_candidates_are_same_session(a, b) {
        return attention_self_adjacency_score();
    }
    attention_relationship_score(a, b)
}

fn attention_candidates_are_same_session(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    a.session.session_id == b.session.session_id
}

fn attention_self_adjacency_score() -> i32 {
    0
}

fn attention_relationship_score(a: &AttentionCandidate, b: &AttentionCandidate) -> i32 {
    attention_weight_if(attention_repos_match(a, b), 100)
        + attention_weight_if(attention_families_match(a, b), 70)
        + attention_weight_if(attention_batches_match(a, b), 50)
        + attention_weight_if(attention_tools_match(a, b), 5)
}

fn attention_weight_if(matched: bool, weight: i32) -> i32 {
    if matched {
        weight
    } else {
        0
    }
}

fn attention_repos_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    !a.repo.is_empty() && a.repo == b.repo
}

fn attention_families_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    !a.family.is_empty() && a.family == b.family
}

fn attention_batches_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    a.batch.is_some() && a.batch == b.batch
}

fn attention_tools_match(a: &AttentionCandidate, b: &AttentionCandidate) -> bool {
    a.session.tool.is_some() && a.session.tool == b.session.tool
}

fn attention_repo_key(cwd: &str) -> String {
    let parts = cwd_path_parts(cwd);
    if parts.is_empty() {
        return String::new();
    }
    if let Some(index) = parts.iter().position(|part| part == "repos") {
        for part in parts.iter().skip(index + 1) {
            if !matches!(
                part.as_str(),
                "opensource" | "clients" | "personal" | "work" | "projects"
            ) {
                return part.clone();
            }
        }
    }
    parts.last().cloned().unwrap_or_default()
}

fn cwd_path_parts(cwd: &str) -> Vec<String> {
    Path::new(cwd)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(|value| value.trim().to_ascii_lowercase()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .collect()
}

fn attention_project_family(repo: &str) -> String {
    let mut family = repo.trim().to_ascii_lowercase();
    for suffix in [
        "_server",
        "-server",
        "_backend",
        "-backend",
        "_frontend",
        "-frontend",
        "_client",
        "-client",
        "_web",
        "-web",
        "_api",
        "-api",
        "_core",
        "-core",
    ] {
        if family.len() > suffix.len() && family.ends_with(suffix) {
            family.truncate(family.len() - suffix.len());
            break;
        }
    }
    family
}

async fn restart_command_output(
    command: &RestartCommandRef<'_>,
) -> Result<std::process::Output, String> {
    tokio::time::timeout(
        Duration::from_secs(240),
        // `kill_on_drop` ensures the timeout actually reaps the child: when
        // the timeout fires the `output()` future is dropped, and without
        // this the spawned `sh` (and its descendants' controlling process)
        // would be orphaned and keep running past the deadline.
        Command::new("sh")
            .arg("-c")
            .arg(command.command)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("restart of {} timed out after 240s", command.service_name))?
    .map_err(|error| error.to_string())
}

fn restart_failure_message(service_name: &str, stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "{}: {}",
        service_name,
        restart_failure_detail(stdout, stderr)
    )
}

fn restart_failure_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if stderr.is_empty() {
        restart_stdout_failure_detail(stdout)
    } else {
        truncate_restart_detail(&stderr)
    }
}

fn restart_stdout_failure_detail(stdout: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let detail = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("restart failed")
        .trim();
    truncate_restart_detail(detail)
}

fn truncate_restart_detail(detail: &str) -> String {
    detail.chars().take(600).collect()
}

#[cfg(test)]
fn select_attention_group_sessions(
    sessions: Vec<SessionSummary>,
    max_sessions: usize,
) -> Vec<SessionSummary> {
    plan_attention_group_sessions(sessions, max_sessions, &[], false).visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::health::BridgeHealthState;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{
        RestState, SessionBatchMembership, StateEvidence, ThoughtSource, ThoughtState,
        TransportHealth,
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: Arc::new(BridgeHealthState::new_with_tick(Duration::from_secs(15))),
            published_selection: Arc::new(RwLock::new(
                crate::api::PublishedSelectionState::default(),
            )),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    fn overlay_service(name: &str, dir: &str, restart: Option<&str>) -> OverlayServiceEntry {
        OverlayServiceEntry {
            name: name.to_string(),
            dir: dir.to_string(),
            health_url: None,
            restart: restart.map(str::to_string),
            open_url: None,
        }
    }

    fn managed_service_config(base: &Path, services: Vec<OverlayServiceEntry>) -> OverlayDirConfig {
        OverlayDirConfig {
            label: "managed".into(),
            base_path: base.to_path_buf(),
            services,
            groups: Vec::new(),
            launch: crate::session::overlay::OverlayLaunchConfig::local_only(),
        }
    }

    #[tokio::test]
    async fn list_managed_service_entries_dedupes_dirs_skips_missing_and_keeps_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let alpha = base.join("alpha");
        std::fs::create_dir_all(&alpha).expect("alpha");
        let alpha_absolute = alpha.to_string_lossy().into_owned();
        let config = managed_service_config(
            &base,
            vec![
                overlay_service("alpha-api", "alpha", None),
                overlay_service("alpha-worker", &alpha_absolute, Some("make restart")),
                overlay_service("missing", "missing", Some("make missing")),
            ],
        );

        let entries = list_managed_service_entries(&test_state(), &config).await;

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        let expected_full_path = alpha
            .canonicalize()
            .expect("canonical alpha")
            .to_string_lossy()
            .into_owned();
        assert_eq!(entry.name, "alpha");
        assert_eq!(
            entry.full_path.as_deref(),
            Some(expected_full_path.as_str())
        );
        assert_eq!(entry.has_restart, Some(true));
        assert_eq!(entry.is_running, Some(true));
        assert_eq!(entry.repo_dirty, None);
        assert_eq!(entry.group, None);
        assert!(entry.groups.is_empty());
    }

    fn repo_search_response_paths(response: &DirRepoSearchResponse) -> Vec<String> {
        response
            .entries
            .iter()
            .filter_map(|entry| entry.full_path.clone())
            .collect()
    }

    async fn repo_search_cache_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

    #[test]
    fn scan_repo_search_roots_finds_git_repositories_under_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repos = dir.path().join("repos");
        let hard = dir.path().join("hard");
        let swimmers = repos.join("opensource").join("swimmers");
        let pcbcd = hard.join("pcbcd");
        let not_repo = repos.join("notes");
        std::fs::create_dir_all(swimmers.join(".git")).expect("create swimmers git marker");
        std::fs::create_dir_all(pcbcd.join(".git")).expect("create pcbcd git marker");
        std::fs::create_dir_all(&not_repo).expect("create non repo");

        let entries = scan_repo_search_roots_sync(&[repos, hard], REPO_SEARCH_DEFAULT_MAX_DEPTH);
        let paths = entries
            .iter()
            .filter_map(|entry| entry.full_path.clone())
            .collect::<BTreeSet<_>>();
        let swimmers = swimmers.canonicalize().expect("canonical swimmers");
        let pcbcd = pcbcd.canonicalize().expect("canonical pcbcd");
        let not_repo = not_repo.canonicalize().expect("canonical non repo");

        assert!(paths.contains(&swimmers.to_string_lossy().into_owned()));
        assert!(paths.contains(&pcbcd.to_string_lossy().into_owned()));
        assert!(!paths.contains(&not_repo.to_string_lossy().into_owned()));
    }

    #[test]
    fn scan_repo_search_roots_prunes_inside_found_repositories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let parent = dir.path().join("repos").join("parent");
        let nested = parent.join("nested");
        std::fs::create_dir_all(parent.join(".git")).expect("create parent git marker");
        std::fs::create_dir_all(nested.join(".git")).expect("create nested git marker");

        let entries =
            scan_repo_search_roots_sync(&[dir.path().join("repos")], REPO_SEARCH_DEFAULT_MAX_DEPTH);
        let paths = entries
            .iter()
            .filter_map(|entry| entry.full_path.clone())
            .collect::<Vec<_>>();
        let parent = parent.canonicalize().expect("canonical parent");

        assert_eq!(paths, vec![parent.to_string_lossy().into_owned()]);
    }

    #[test]
    fn repo_search_visit_treats_repositories_as_terminal_at_max_depth() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).expect("create git marker");

        assert!(matches!(
            repo_search_visit(
                &repo,
                REPO_SEARCH_DEFAULT_MAX_DEPTH,
                REPO_SEARCH_DEFAULT_MAX_DEPTH
            ),
            RepoSearchVisit::Repository
        ));
    }

    #[test]
    fn repo_search_visit_skips_non_repositories_at_max_depth() {
        let dir = tempfile::tempdir().expect("tempdir");

        assert!(matches!(
            repo_search_visit(
                dir.path(),
                REPO_SEARCH_DEFAULT_MAX_DEPTH,
                REPO_SEARCH_DEFAULT_MAX_DEPTH
            ),
            RepoSearchVisit::Skip
        ));
    }

    #[test]
    fn repo_search_child_dirs_filters_non_dirs_and_blocked_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("src");
        std::fs::create_dir_all(dir.path().join("target")).expect("target");
        std::fs::create_dir_all(dir.path().join(".hidden")).expect("hidden");
        std::fs::write(dir.path().join("README.md"), "notes").expect("file");

        let child_names = repo_search_child_dirs(dir.path())
            .into_iter()
            .map(|path| {
                path.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(child_names, BTreeSet::from(["src".to_string()]));
    }

    #[test]
    fn scan_repo_search_roots_respects_max_depth_for_non_repo_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repos");
        let direct = root.join("direct");
        let nested = root.join("container").join("nested");
        std::fs::create_dir_all(direct.join(".git")).expect("direct repo");
        std::fs::create_dir_all(nested.join(".git")).expect("nested repo");

        let entries = scan_repo_search_roots_sync(&[root], 1);
        let paths = entries
            .iter()
            .filter_map(|entry| entry.full_path.clone())
            .collect::<BTreeSet<_>>();
        let direct = direct.canonicalize().expect("canonical direct");
        let nested = nested.canonicalize().expect("canonical nested");

        assert!(paths.contains(&direct.to_string_lossy().into_owned()));
        assert!(!paths.contains(&nested.to_string_lossy().into_owned()));
    }

    #[test]
    fn scan_repo_search_roots_skips_duplicate_canonical_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repos");
        let repo = root.join("swimmers");
        std::fs::create_dir_all(repo.join(".git")).expect("repo");

        let entries =
            scan_repo_search_roots_sync(&[root.clone(), root], REPO_SEARCH_DEFAULT_MAX_DEPTH);
        let paths = entries
            .iter()
            .filter_map(|entry| entry.full_path.clone())
            .collect::<Vec<_>>();
        let repo = repo.canonicalize().expect("canonical repo");

        assert_eq!(paths, vec![repo.to_string_lossy().into_owned()]);
    }

    #[test]
    fn restart_commands_for_matched_services_collects_only_matched_restart_commands() {
        let services = vec![
            overlay_service("web", "web", Some("restart web")),
            overlay_service("api", "api", Some("restart api")),
            overlay_service("worker", "worker", None),
            overlay_service("db", "db", Some("restart db")),
        ];
        let matched = vec!["api".to_string(), "worker".to_string(), "web".to_string()];

        let commands =
            restart_commands_for_matched_services(&services, &matched).expect("commands");

        assert_eq!(
            commands,
            vec![
                ("web".to_string(), "restart web".to_string()),
                ("api".to_string(), "restart api".to_string())
            ]
        );
    }

    #[test]
    fn restart_commands_for_matched_services_rejects_no_matched_services() {
        let services = vec![overlay_service("web", "web", Some("restart web"))];

        let err = restart_commands_for_matched_services(&services, &[])
            .expect_err("empty matched services should fail");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "NO_SERVICE_FOR_PATH");
    }

    #[test]
    fn restart_commands_for_matched_services_rejects_no_restart_commands() {
        let services = vec![
            overlay_service("web", "web", None),
            overlay_service("api", "api", Some("restart api")),
        ];
        let matched = vec!["web".to_string()];

        let err = restart_commands_for_matched_services(&services, &matched)
            .expect_err("matched service without restart command should fail");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "NO_RESTART_COMMAND");
    }

    #[test]
    fn restart_services_failure_detail_prefers_stderr_and_truncates() {
        let stdout = b"stdout detail";
        let stderr = format!("{}{}", "x".repeat(610), "tail");

        let detail = restart_failure_detail(stdout, stderr.as_bytes());

        assert_eq!(detail.chars().count(), 600);
        assert_eq!(detail, "x".repeat(600));
    }

    #[test]
    fn restart_services_failure_detail_uses_last_nonempty_stdout_line() {
        let stdout = b"first\n\n second detail \n";

        let detail = restart_failure_detail(stdout, b"  \n");

        assert_eq!(detail, "second detail");
    }

    #[test]
    fn restart_services_failure_detail_defaults_without_output() {
        let detail = restart_failure_detail(b"\n  \n", b"");

        assert_eq!(detail, "restart failed");
    }

    // Empty roots short-circuit before any cache access, so this test never
    // touches the global repo-search cache — keeping it race-free alongside the
    // scan/cache test below, which is the sole cache mutator.
    #[tokio::test]
    async fn list_repo_search_entries_inner_returns_empty_for_no_roots() {
        let response = list_repo_search_entries_inner(Vec::new(), REPO_SEARCH_DEFAULT_MAX_DEPTH)
            .await
            .expect("empty roots should not error");
        assert!(response.roots.is_empty(), "no roots should yield no labels");
        assert!(
            response.entries.is_empty(),
            "no roots should yield no entries"
        );
    }

    // Exercises the rescan-and-populate branch followed by the cache-hit branch.
    // Removing the repo from disk between calls proves the second call serves the
    // cached entries rather than rescanning.
    #[tokio::test]
    async fn list_repo_search_entries_inner_scans_then_serves_cache() {
        let _cache_guard = repo_search_cache_test_guard().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repos");
        let repo = root.join("alpha");
        std::fs::create_dir_all(repo.join(".git")).expect("create git marker");

        clear_repo_search_cache_for_tests();
        let mut scanned =
            list_repo_search_entries_inner(vec![root.clone()], REPO_SEARCH_DEFAULT_MAX_DEPTH)
                .await
                .expect("fresh scan should succeed");
        let repo_canon = repo.canonicalize().expect("canonical repo");
        assert!(
            scanned.entries.iter().any(|entry| {
                entry.full_path.as_deref() == Some(repo_canon.to_string_lossy().as_ref())
            }),
            "fresh scan should find the alpha repo, got {:?}",
            scanned.entries
        );
        assert_eq!(
            scanned.roots,
            vec![root.to_string_lossy().into_owned()],
            "roots label should echo the requested root"
        );
        let scanned_paths = repo_search_response_paths(&scanned);
        scanned.entries.clear();

        // Delete the repo from disk; a cache hit within the TTL must still
        // return the previously scanned entries instead of rescanning. A rescan
        // now would find nothing, so finding alpha proves the cache served it.
        std::fs::remove_dir_all(&repo).expect("remove repo");
        let cached =
            list_repo_search_entries_inner(vec![root.clone()], REPO_SEARCH_DEFAULT_MAX_DEPTH)
                .await
                .expect("cache hit should succeed");
        let cached_paths = repo_search_response_paths(&cached);
        assert_eq!(
            cached_paths, scanned_paths,
            "second call within TTL should serve cached entries, not rescan"
        );
        assert!(
            cached_paths.contains(&repo_canon.to_string_lossy().into_owned()),
            "cache hit should still surface the now-deleted repo (a rescan would not)"
        );

        clear_repo_search_cache_for_tests();
    }

    #[tokio::test]
    async fn list_repo_search_entries_inner_cache_key_includes_roots_and_max_depth() {
        let _cache_guard = repo_search_cache_test_guard().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("repos");
        let nested = root.join("container").join("nested");
        std::fs::create_dir_all(nested.join(".git")).expect("create nested repo");

        clear_repo_search_cache_for_tests();
        let shallow = list_repo_search_entries_inner(vec![root.clone()], 1)
            .await
            .expect("shallow scan should succeed");
        let nested_canon = nested.canonicalize().expect("canonical nested repo");
        let nested_path = nested_canon.to_string_lossy().into_owned();
        assert!(
            !repo_search_response_paths(&shallow).contains(&nested_path),
            "shallow scan should not reach nested repo"
        );

        let deep = list_repo_search_entries_inner(vec![root.clone()], 2)
            .await
            .expect("deeper scan should succeed");
        assert!(
            repo_search_response_paths(&deep).contains(&nested_path),
            "changed max_depth should miss the shallow cache and rescan"
        );

        let alpha_root = dir.path().join("alpha-root");
        let beta_root = dir.path().join("beta-root");
        let alpha = alpha_root.join("alpha");
        let beta = beta_root.join("beta");
        std::fs::create_dir_all(alpha.join(".git")).expect("create alpha repo");
        std::fs::create_dir_all(beta.join(".git")).expect("create beta repo");

        clear_repo_search_cache_for_tests();
        let alpha_response =
            list_repo_search_entries_inner(vec![alpha_root.clone()], REPO_SEARCH_DEFAULT_MAX_DEPTH)
                .await
                .expect("alpha scan should succeed");
        let alpha_path = alpha
            .canonicalize()
            .expect("canonical alpha repo")
            .to_string_lossy()
            .into_owned();
        assert!(
            repo_search_response_paths(&alpha_response).contains(&alpha_path),
            "alpha scan should find alpha"
        );

        let beta_response =
            list_repo_search_entries_inner(vec![beta_root.clone()], REPO_SEARCH_DEFAULT_MAX_DEPTH)
                .await
                .expect("beta scan should succeed");
        let beta_path = beta
            .canonicalize()
            .expect("canonical beta repo")
            .to_string_lossy()
            .into_owned();
        let beta_paths = repo_search_response_paths(&beta_response);
        assert!(
            beta_paths.contains(&beta_path),
            "changed roots should miss the alpha cache and rescan"
        );
        assert!(
            !beta_paths.contains(&alpha_path),
            "changed roots should not serve entries from the alpha cache"
        );
        assert_eq!(
            beta_response.roots,
            vec![beta_root.to_string_lossy().into_owned()],
            "roots label should echo the requested beta root"
        );

        clear_repo_search_cache_for_tests();
    }

    fn summary(session_id: &str, tmux_name: &str, state: SessionState) -> SessionSummary {
        SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: tmux_name.to_string(),
            state,
            current_command: None,
            state_evidence: StateEvidence::new("test"),
            cwd: "/tmp/repos/swimmers".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Active,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
        }
    }

    fn waiting_session(session_id: &str, cwd: &str, seconds_ago: i64) -> SessionSummary {
        let mut session = summary(session_id, session_id, SessionState::Idle);
        session.cwd = cwd.to_string();
        session.last_activity_at = Utc::now() - ChronoDuration::seconds(seconds_ago);
        session
    }

    fn numbered_waiting_session(
        session_id: &str,
        tmux_name: &str,
        cwd: &str,
        seconds_ago: i64,
    ) -> SessionSummary {
        let mut session = waiting_session(session_id, cwd, seconds_ago);
        session.tmux_name = tmux_name.to_string();
        session
    }

    fn batch_session(mut session: SessionSummary, batch_id: &str) -> SessionSummary {
        session.batch = Some(SessionBatchMembership {
            id: batch_id.to_string(),
            label: batch_id.to_string(),
            index: 0,
            total: 2,
            created_at: Utc::now(),
            prompt_excerpt: None,
        });
        session
    }

    fn plan_ids(
        sessions: Vec<SessionSummary>,
        max_sessions: usize,
        current_session_ids: &[&str],
    ) -> Vec<String> {
        plan_ids_with_unnumbered(sessions, max_sessions, current_session_ids, false)
    }

    fn plan_ids_with_unnumbered(
        sessions: Vec<SessionSummary>,
        max_sessions: usize,
        current_session_ids: &[&str],
        include_unnumbered_sessions: bool,
    ) -> Vec<String> {
        let current = current_session_ids
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>();
        plan_attention_group_sessions(
            sessions,
            max_sessions,
            &current,
            include_unnumbered_sessions,
        )
        .visible
        .into_iter()
        .map(|session| session.session_id)
        .collect()
    }

    fn attention_group_request(
        focus: bool,
        current_session_ids: &[&str],
    ) -> NativeAttentionGroupOpenRequest {
        NativeAttentionGroupOpenRequest {
            max_sessions: None,
            current_session_ids: current_session_ids
                .iter()
                .map(|session_id| (*session_id).to_string())
                .collect(),
            include_unnumbered_sessions: false,
            layout: None,
            focus,
        }
    }

    #[test]
    fn attention_group_empty_focus_plan_reports_no_sessions() {
        let request = attention_group_request(true, &["visible-a"]);

        assert_eq!(
            empty_attention_group_plan_outcome(&request),
            EmptyAttentionGroupPlanOutcome::NoAttentionSessions
        );
    }

    #[test]
    fn attention_group_empty_non_focus_current_group_requests_native_clear() {
        let request = attention_group_request(false, &["visible-a"]);

        assert_eq!(
            empty_attention_group_plan_outcome(&request),
            EmptyAttentionGroupPlanOutcome::ClearNative
        );
    }

    #[test]
    fn attention_group_empty_non_focus_without_current_group_reports_no_sessions() {
        let request = attention_group_request(false, &[]);

        assert_eq!(
            empty_attention_group_plan_outcome(&request),
            EmptyAttentionGroupPlanOutcome::NoAttentionSessions
        );
    }

    #[test]
    fn attention_group_response_populates_backlog_session_ids() {
        let visible = numbered_waiting_session("visible-a", "69", "/Users/b/repos/swimmers", 10);
        let backlog_a = numbered_waiting_session("backlog-a", "70", "/Users/b/repos/swimmers", 20);
        let backlog_b = numbered_waiting_session("backlog-b", "71", "/Users/b/repos/swimmers", 30);
        let plan = AttentionGroupPlan {
            visible: vec![visible],
            backlog: vec![backlog_a, backlog_b],
        };
        let response = NativeAttentionGroupOpenResponse {
            session_id: NATIVE_ATTENTION_GROUP_SESSION_ID.to_string(),
            tmux_name: NATIVE_ATTENTION_GROUP_TMUX_NAME.to_string(),
            session_count: 1,
            session_ids: vec!["visible-a".to_string()],
            backlog_session_ids: vec!["stale".to_string()],
            status: "refreshed".to_string(),
            focused: false,
            pane_id: None,
            attach_command: None,
        };

        let response = response_with_attention_backlog(response, &plan);

        assert_eq!(
            response.backlog_session_ids,
            vec!["backlog-a".to_string(), "backlog-b".to_string()]
        );
    }

    #[test]
    fn attention_group_selection_includes_idle_agent_sessions_without_sleep_snapshot() {
        let idle_agent = summary("sess-11", "11", SessionState::Idle);
        let managed_group = summary(
            NATIVE_ATTENTION_GROUP_SESSION_ID,
            NATIVE_ATTENTION_GROUP_TMUX_NAME,
            SessionState::Attention,
        );
        let mut shell = summary("shell", "shell", SessionState::Idle);
        shell.tool = None;
        let busy_agent = summary("busy", "busy", SessionState::Busy);

        let selected =
            select_attention_group_sessions(vec![shell, idle_agent, managed_group, busy_agent], 6);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].tmux_name, "11");
    }

    #[test]
    fn attention_group_selection_excludes_unnumbered_tmux_names_by_default() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let wave = summary("sess-wave-01", "dac-cyclechef-wave-01", SessionState::Idle);
        let named = summary("sess-named", "buildooor", SessionState::Attention);

        let selected = plan_ids(vec![wave, named, numbered], 6, &[]);

        assert_eq!(selected, vec!["sess-8"]);
    }

    #[test]
    fn attention_group_selection_requires_exact_numeric_tmux_names() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let padded_numeric = summary("sess-padded-9", " 9 ", SessionState::Attention);

        let selected = plan_ids(vec![padded_numeric, numbered], 6, &[]);

        assert_eq!(selected, vec!["sess-8"]);
    }

    #[test]
    fn attention_group_refresh_drops_current_unnumbered_tmux_names_by_default() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let current_wave = summary("sess-wave-01", "dac-cyclechef-wave-01", SessionState::Idle);
        let next_numbered = summary("sess-9", "9", SessionState::Attention);

        let selected = plan_ids(
            vec![current_wave, numbered, next_numbered],
            2,
            &["sess-wave-01", "sess-8"],
        );

        assert_eq!(selected, vec!["sess-8", "sess-9"]);
    }

    #[test]
    fn attention_group_can_include_unnumbered_tmux_names_when_opted_in() {
        let numbered = summary("sess-8", "8", SessionState::Idle);
        let wave = summary("sess-wave-01", "dac-cyclechef-wave-01", SessionState::Idle);

        let selected = plan_ids_with_unnumbered(vec![numbered, wave], 2, &["sess-wave-01"], true);

        assert_eq!(selected, vec!["sess-wave-01", "sess-8"]);
    }

    #[test]
    fn attention_queue_prefers_same_sweet_potato_project_over_newer_unrelated_sessions() {
        let sweet_a = numbered_waiting_session("sweet-a", "21", "/Users/b/repos/sweet-potato", 120);
        let sweet_b = numbered_waiting_session(
            "sweet-b",
            "22",
            "/Users/b/repos/sweet-potato/packages/api",
            90,
        );
        let newer_unrelated =
            numbered_waiting_session("newer", "23", "/Users/b/repos/buildooor", 1);

        let selected = plan_ids(vec![newer_unrelated, sweet_a, sweet_b], 2, &[]);

        assert_eq!(selected, vec!["sweet-b", "sweet-a"]);
    }

    #[test]
    fn attention_queue_treats_htma_and_htma_server_as_adjacent_siblings() {
        let htma = numbered_waiting_session("htma-ui", "31", "/Users/b/repos/htma", 80);
        let htma_server =
            numbered_waiting_session("htma-api", "32", "/Users/b/repos/htma_server", 70);
        let unrelated =
            numbered_waiting_session("newer-unrelated", "33", "/Users/b/repos/finalreceipts", 1);

        let selected = plan_ids(vec![unrelated, htma, htma_server], 2, &[]);

        assert_eq!(selected, vec!["htma-api", "htma-ui"]);
    }

    #[test]
    fn attention_queue_uses_batch_before_recency_tie_break() {
        let batch_a = batch_session(
            numbered_waiting_session("batch-a", "41", "/Users/b/repos/alpha", 90),
            "b1",
        );
        let batch_b = batch_session(
            numbered_waiting_session("batch-b", "42", "/Users/b/repos/beta", 80),
            "b1",
        );
        let newer_unrelated = numbered_waiting_session("newer", "43", "/Users/b/repos/gamma", 1);

        let selected = plan_ids(vec![newer_unrelated, batch_a, batch_b], 2, &[]);

        assert_eq!(selected, vec!["batch-b", "batch-a"]);
    }

    #[test]
    fn attention_adjacency_score_weights_only_present_matching_relationships() {
        let alpha_a = AttentionCandidate::from(batch_session(
            numbered_waiting_session("alpha-a", "44", "/Users/b/repos/alpha", 20),
            "b1",
        ));
        let alpha_b = AttentionCandidate::from(batch_session(
            numbered_waiting_session("alpha-b", "45", "/Users/b/repos/alpha", 10),
            "b1",
        ));

        assert_eq!(attention_adjacency_score(&alpha_a, &alpha_b), 225);
        assert_eq!(attention_adjacency_score(&alpha_a, &alpha_a), 0);

        let mut missing_a = numbered_waiting_session("missing-a", "46", "", 20);
        missing_a.tool = None;
        let mut missing_b = numbered_waiting_session("missing-b", "47", "", 10);
        missing_b.tool = None;

        assert_eq!(
            attention_adjacency_score(
                &AttentionCandidate::from(missing_a),
                &AttentionCandidate::from(missing_b)
            ),
            0
        );
    }

    #[test]
    fn attention_fill_prefers_best_adjacency_to_visible_group() {
        let visible = vec![AttentionCandidate::from(numbered_waiting_session(
            "visible-alpha",
            "48",
            "/Users/b/repos/alpha",
            60,
        ))];
        let unrelated_newer = AttentionCandidate::from(numbered_waiting_session(
            "unrelated-newer",
            "49",
            "/Users/b/repos/gamma",
            1,
        ));
        let same_repo_older = AttentionCandidate::from(numbered_waiting_session(
            "same-repo-older",
            "50",
            "/Users/b/repos/alpha",
            120,
        ));

        assert_eq!(
            best_attention_fill_index(&visible, &[unrelated_newer, same_repo_older]),
            1
        );
    }

    #[test]
    fn attention_fill_uses_recency_then_session_id_for_equal_scores() {
        let visible = vec![AttentionCandidate::from(numbered_waiting_session(
            "visible-alpha",
            "56",
            "/Users/b/repos/alpha",
            60,
        ))];
        let older = AttentionCandidate::from(numbered_waiting_session(
            "older-alpha",
            "57",
            "/Users/b/repos/alpha",
            120,
        ));
        let newer = AttentionCandidate::from(numbered_waiting_session(
            "newer-alpha",
            "58",
            "/Users/b/repos/alpha",
            30,
        ));

        assert_eq!(best_attention_fill_index(&visible, &[older, newer]), 1);

        let tied_at = Utc::now();
        let mut later_id = numbered_waiting_session("tie-b", "59", "/Users/b/repos/alpha", 30);
        later_id.last_activity_at = tied_at;
        let mut earlier_id = numbered_waiting_session("tie-a", "60", "/Users/b/repos/alpha", 30);
        earlier_id.last_activity_at = tied_at;

        assert_eq!(
            best_attention_fill_index(
                &visible,
                &[
                    AttentionCandidate::from(later_id),
                    AttentionCandidate::from(earlier_id)
                ]
            ),
            1
        );
    }

    #[test]
    fn attention_fill_returns_zero_when_candidates_are_empty() {
        let visible = vec![AttentionCandidate::from(numbered_waiting_session(
            "visible-alpha",
            "68",
            "/Users/b/repos/alpha",
            60,
        ))];

        assert_eq!(best_attention_fill_index(&visible, &[]), 0);
    }

    #[test]
    fn attention_queue_rotates_one_in_one_out_from_current_visible_set() {
        let visible_a =
            numbered_waiting_session("visible-a", "51", "/Users/b/repos/sweet-potato", 120);
        let mut resolved_b =
            numbered_waiting_session("visible-b", "52", "/Users/b/repos/sweet-potato", 110);
        resolved_b.thought_state = ThoughtState::Active;
        let visible_c =
            numbered_waiting_session("visible-c", "53", "/Users/b/repos/sweet-potato", 100);
        let next_d = numbered_waiting_session(
            "next-d",
            "54",
            "/Users/b/repos/sweet-potato/packages/api",
            90,
        );
        let unrelated_newer =
            numbered_waiting_session("unrelated", "55", "/Users/b/repos/buildooor", 1);

        let selected = plan_ids(
            vec![visible_a, resolved_b, visible_c, next_d, unrelated_newer],
            3,
            &["visible-a", "visible-b", "visible-c"],
        );

        assert_eq!(selected, vec!["visible-a", "visible-c", "next-d"]);
    }

    #[test]
    fn attention_queue_excludes_unsafe_sessions() {
        let ready = numbered_waiting_session("ready", "61", "/Users/b/repos/swimmers", 1);
        let mut stale = numbered_waiting_session("stale", "62", "/Users/b/repos/swimmers", 1);
        stale.is_stale = true;
        let mut unhealthy =
            numbered_waiting_session("unhealthy", "63", "/Users/b/repos/swimmers", 1);
        unhealthy.transport_health = TransportHealth::Disconnected;
        let mut unobserved =
            numbered_waiting_session("unobserved", "64", "/Users/b/repos/swimmers", 1);
        unobserved.state_evidence = StateEvidence::unobserved("test");
        let exited = summary("exited", "65", SessionState::Exited);
        let mut deep_sleep = numbered_waiting_session("deep", "66", "/Users/b/repos/swimmers", 1);
        deep_sleep.rest_state = RestState::DeepSleep;
        let remote = numbered_waiting_session("remote::sess", "67", "/Users/b/repos/swimmers", 1);
        let managed = summary(
            NATIVE_ATTENTION_GROUP_SESSION_ID,
            NATIVE_ATTENTION_GROUP_TMUX_NAME,
            SessionState::Attention,
        );

        let selected = plan_ids(
            vec![
                ready, stale, unhealthy, unobserved, exited, deep_sleep, remote, managed,
            ],
            6,
            &[],
        );

        assert_eq!(selected, vec!["ready"]);
    }

    #[test]
    fn normalize_group_update_names_deduplicates_and_rejects_unknown_groups() {
        let valid = ["frontend".to_string(), "backend".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let names = normalize_group_update_names(
            &[
                "frontend".to_string(),
                " backend ".to_string(),
                "frontend".to_string(),
            ],
            &valid,
        )
        .expect("valid names");
        assert_eq!(names, vec!["frontend".to_string(), "backend".to_string()]);

        let err = normalize_group_update_names(&["skills".to_string()], &valid)
            .expect_err("unknown group");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.code, "GROUP_NOT_FOUND");
    }

    #[test]
    fn apply_group_membership_update_makes_add_win_over_remove_for_same_group() {
        let path = "/tmp/repo";
        let mut memberships = DirGroupMemberships::default();
        apply_group_membership_update(
            &mut memberships,
            path,
            vec!["frontend".to_string()],
            vec!["backend".to_string(), "frontend".to_string()],
        );

        let frontend = memberships.groups.get("frontend").expect("frontend delta");
        assert!(frontend.include_paths.contains(path));
        assert!(!frontend.exclude_paths.contains(path));

        let backend = memberships.groups.get("backend").expect("backend delta");
        assert!(backend.exclude_paths.contains(path));
        assert!(!backend.include_paths.contains(path));
    }

    #[test]
    fn apply_group_membership_update_records_remove_as_exclusion() {
        let path = "/tmp/repo";
        let mut memberships = DirGroupMemberships::default();
        memberships
            .groups
            .entry("backend".to_string())
            .or_default()
            .include_paths
            .insert(path.to_string());

        apply_group_membership_update(
            &mut memberships,
            path,
            Vec::new(),
            vec!["backend".to_string()],
        );

        let backend = memberships.groups.get("backend").expect("backend delta");
        assert!(!backend.include_paths.contains(path));
        assert!(backend.exclude_paths.contains(path));
    }

    #[test]
    fn apply_group_membership_update_records_add_as_inclusion() {
        let path = "/tmp/repo";
        let mut memberships = DirGroupMemberships::default();
        memberships
            .groups
            .entry("frontend".to_string())
            .or_default()
            .exclude_paths
            .insert(path.to_string());

        apply_group_membership_update(
            &mut memberships,
            path,
            vec!["frontend".to_string()],
            Vec::new(),
        );

        let frontend = memberships.groups.get("frontend").expect("frontend delta");
        assert!(frontend.include_paths.contains(path));
        assert!(!frontend.exclude_paths.contains(path));
    }

    #[test]
    fn apply_group_membership_update_prunes_empty_stale_deltas() {
        let path = "/tmp/repo";
        let mut memberships = DirGroupMemberships::default();
        memberships
            .groups
            .insert("stale".to_string(), Default::default());

        apply_group_membership_update(
            &mut memberships,
            path,
            vec!["frontend".to_string()],
            Vec::new(),
        );

        assert!(!memberships.groups.contains_key("stale"));
        assert!(memberships
            .groups
            .get("frontend")
            .expect("frontend delta")
            .include_paths
            .contains(path));
    }

    fn test_group_config(
        base: &Path,
        frontend: PathBuf,
        backend: PathBuf,
        wildcard_root: PathBuf,
    ) -> OverlayDirConfig {
        OverlayDirConfig {
            label: "test".into(),
            base_path: base.to_path_buf(),
            services: Vec::new(),
            groups: vec![
                OverlayDirGroup {
                    name: "frontend".into(),
                    paths: vec![frontend],
                    dirs: Vec::new(),
                },
                OverlayDirGroup {
                    name: "backend".into(),
                    paths: vec![backend],
                    dirs: Vec::new(),
                },
                OverlayDirGroup {
                    name: "skills".into(),
                    paths: Vec::new(),
                    dirs: vec![wildcard_root],
                },
            ],
            launch: crate::session::overlay::OverlayLaunchConfig::local_only(),
        }
    }

    fn empty_group_config(base: &Path) -> OverlayDirConfig {
        OverlayDirConfig {
            label: "empty".into(),
            base_path: base.to_path_buf(),
            services: Vec::new(),
            groups: Vec::new(),
            launch: crate::session::overlay::OverlayLaunchConfig::local_only(),
        }
    }

    fn assert_api_service_error(
        err: ApiServiceError,
        status: StatusCode,
        code: &str,
        message: &str,
    ) {
        assert_eq!(err.status, status);
        assert_eq!(err.code, code);
        assert_eq!(err.message, message);
    }

    #[test]
    fn resolve_group_membership_path_rejects_empty_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        std::fs::create_dir_all(&base).expect("base");
        let config = empty_group_config(&base);

        let err = resolve_group_membership_path(
            &base.canonicalize().expect("canonical base"),
            " \t\n ",
            &config,
        )
        .expect_err("empty path");

        assert_api_service_error(
            err,
            StatusCode::BAD_REQUEST,
            "GROUP_PATH_REQUIRED",
            "path is required",
        );
    }

    #[test]
    fn resolve_group_membership_path_rejects_non_directory_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let file = base.join("README.md");
        std::fs::create_dir_all(&base).expect("base");
        std::fs::write(&file, "not a directory").expect("file");
        let config = empty_group_config(&base);
        let raw_path = file.to_string_lossy().into_owned();

        let err = resolve_group_membership_path(
            &base.canonicalize().expect("canonical base"),
            &raw_path,
            &config,
        )
        .expect_err("file path");

        assert_api_service_error(
            err,
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            &format!("directory not found: {raw_path}"),
        );
    }

    #[test]
    fn resolve_group_membership_path_rejects_paths_outside_base_and_overlay_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let frontend = base.join("frontend-app");
        let backend = base.join("backend-app");
        let wildcard_root = dir.path().join("skills");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&wildcard_root).expect("wildcard");
        std::fs::create_dir_all(&outside).expect("outside");
        let config = test_group_config(&base, frontend, backend, wildcard_root);
        let raw_path = outside.to_string_lossy().into_owned();

        let err = resolve_group_membership_path(
            &base.canonicalize().expect("canonical base"),
            &raw_path,
            &config,
        )
        .expect_err("outside path");

        assert_api_service_error(
            err,
            StatusCode::FORBIDDEN,
            "DIR_OUTSIDE_BASE",
            "path is outside the allowed directory group roots",
        );
    }

    #[test]
    fn resolve_group_membership_path_allows_overlay_group_paths_outside_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let frontend = base.join("frontend-app");
        let backend = base.join("backend-app");
        let wildcard_root = dir.path().join("skills");
        let skill = wildcard_root.join("alpha-skill");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&skill).expect("skill");
        let config = test_group_config(&base, frontend, backend, wildcard_root);

        let resolved = resolve_group_membership_path(
            &base.canonicalize().expect("canonical base"),
            &skill.to_string_lossy(),
            &config,
        )
        .expect("overlay group path");

        assert_eq!(resolved, skill.canonicalize().expect("canonical skill"));
    }

    #[test]
    fn update_dir_group_memberships_preflight_rejects_missing_store() {
        let result = update_dir_group_memberships_preflight(None, PathBuf::from("/tmp"), |_| {
            panic!("dir config lookup should not run without persistence")
        });
        let err = match result {
            Ok(_) => panic!("missing store should fail"),
            Err(err) => err,
        };

        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code, "PERSISTENCE_UNAVAILABLE");
        assert_eq!(
            err.message,
            "directory group edits require file persistence"
        );
    }

    #[tokio::test]
    async fn update_dir_group_memberships_preflight_rejects_missing_overlay() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");

        let result =
            update_dir_group_memberships_preflight(Some(store), dir.path().to_path_buf(), |_| None);
        let err = match result {
            Ok(_) => panic!("missing overlay should fail"),
            Err(err) => err,
        };

        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code, "OVERLAY_UNAVAILABLE");
        assert_eq!(
            err.message,
            "directory group edits require a configured directory group source"
        );
    }

    #[tokio::test]
    async fn update_dir_group_memberships_preflight_rejects_empty_groups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");

        let result = update_dir_group_memberships_preflight(
            Some(store),
            dir.path().to_path_buf(),
            |canonical_base| Some(empty_group_config(canonical_base)),
        );
        let err = match result {
            Ok(_) => panic!("empty groups should fail"),
            Err(err) => err,
        };

        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code, "GROUPS_UNAVAILABLE");
        assert_eq!(err.message, "no directory groups are configured");
    }

    #[tokio::test]
    async fn update_dir_group_memberships_preflight_returns_store_canonical_base_and_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("missing-base");
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");
        let expected_store = store.clone();
        let expected_base = base.clone();

        let preflight =
            update_dir_group_memberships_preflight(Some(store), base, |canonical_base| {
                assert_eq!(canonical_base, expected_base.as_path());
                let frontend = canonical_base.join("frontend-app");
                let backend = canonical_base.join("backend-app");
                let wildcard_root = canonical_base.join("skills");
                Some(test_group_config(
                    canonical_base,
                    frontend,
                    backend,
                    wildcard_root,
                ))
            })
            .expect("preflight");

        assert!(Arc::ptr_eq(&preflight.store, &expected_store));
        assert_eq!(preflight.canonical_base, expected_base);
        assert_eq!(
            dir_groups(Some(&preflight.dir_config)),
            vec![
                "frontend".to_string(),
                "backend".to_string(),
                "skills".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn update_dir_group_memberships_persists_delta_and_returns_effective_groups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let frontend = base.join("frontend-app");
        let backend = base.join("backend-app");
        let wildcard_root = dir.path().join("skills");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&wildcard_root).expect("wildcard");
        let config = test_group_config(&base, frontend.clone(), backend.clone(), wildcard_root);
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");

        let response = update_dir_group_memberships_with_config(
            store.clone(),
            &base.canonicalize().expect("base"),
            &config,
            DirGroupMembershipUpdateRequest {
                path: backend.to_string_lossy().into_owned(),
                add: vec!["frontend".into()],
                remove: vec!["backend".into()],
            },
        )
        .await
        .expect("update groups");

        let backend_path = canonical_path_string(&backend);
        assert_eq!(response.path, backend_path);
        assert_eq!(response.groups, vec!["frontend".to_string()]);
        assert_eq!(
            response.available_groups,
            vec![
                "frontend".to_string(),
                "backend".to_string(),
                "skills".to_string()
            ]
        );

        let memberships = store.load_dir_group_memberships().await;
        assert!(memberships
            .groups
            .get("frontend")
            .expect("frontend delta")
            .include_paths
            .contains(&backend_path));
        assert!(memberships
            .groups
            .get("backend")
            .expect("backend delta")
            .exclude_paths
            .contains(&backend_path));
    }

    #[tokio::test]
    async fn update_dir_group_memberships_rejects_unknown_and_empty_updates_before_persisting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let frontend = base.join("frontend-app");
        let backend = base.join("backend-app");
        let wildcard_root = dir.path().join("skills");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&wildcard_root).expect("wildcard");
        let config = test_group_config(&base, frontend.clone(), backend, wildcard_root);
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");

        let unknown = update_dir_group_memberships_with_config(
            store.clone(),
            &base.canonicalize().expect("base"),
            &config,
            DirGroupMembershipUpdateRequest {
                path: frontend.to_string_lossy().into_owned(),
                add: vec!["missing".into()],
                remove: Vec::new(),
            },
        )
        .await
        .expect_err("unknown group");
        assert_eq!(unknown.status, StatusCode::NOT_FOUND);
        assert_eq!(unknown.code, "GROUP_NOT_FOUND");

        let empty = update_dir_group_memberships_with_config(
            store.clone(),
            &base.canonicalize().expect("base"),
            &config,
            DirGroupMembershipUpdateRequest {
                path: frontend.to_string_lossy().into_owned(),
                add: Vec::new(),
                remove: Vec::new(),
            },
        )
        .await
        .expect_err("empty update");
        assert_eq!(empty.status, StatusCode::BAD_REQUEST);
        assert_eq!(empty.code, "GROUP_UPDATE_EMPTY");

        assert!(store.load_dir_group_memberships().await.groups.is_empty());
    }

    #[tokio::test]
    async fn update_dir_group_memberships_forbids_paths_outside_base_and_overlay_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let frontend = base.join("frontend-app");
        let backend = base.join("backend-app");
        let wildcard_root = dir.path().join("skills");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&wildcard_root).expect("wildcard");
        std::fs::create_dir_all(&outside).expect("outside");
        let config = test_group_config(&base, frontend, backend, wildcard_root);
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");

        let err = update_dir_group_memberships_with_config(
            store,
            &base.canonicalize().expect("base"),
            &config,
            DirGroupMembershipUpdateRequest {
                path: outside.to_string_lossy().into_owned(),
                add: vec!["frontend".into()],
                remove: Vec::new(),
            },
        )
        .await
        .expect_err("outside path");

        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(err.code, "DIR_OUTSIDE_BASE");
    }

    #[tokio::test]
    async fn update_dir_group_memberships_allows_overlay_group_roots_outside_base() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let frontend = base.join("frontend-app");
        let backend = base.join("backend-app");
        let wildcard_root = dir.path().join("skills");
        let skill = wildcard_root.join("alpha-skill");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&skill).expect("skill");
        let config = test_group_config(&base, frontend, backend, wildcard_root);
        let store = FileStore::new(dir.path().join("store"))
            .await
            .expect("store");

        let response = update_dir_group_memberships_with_config(
            store,
            &base.canonicalize().expect("base"),
            &config,
            DirGroupMembershipUpdateRequest {
                path: skill.to_string_lossy().into_owned(),
                add: vec!["frontend".into()],
                remove: vec!["skills".into()],
            },
        )
        .await
        .expect("outside overlay path");

        assert_eq!(response.groups, vec!["frontend".to_string()]);
    }

    #[tokio::test]
    async fn start_restart_action_rejects_empty_path() {
        let err = start_restart_action(test_state(), "", RepoActionKind::Restart)
            .await
            .expect_err("empty path must error");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_FAILED");
    }

    #[tokio::test]
    async fn start_restart_action_rejects_whitespace_path() {
        let err = start_restart_action(test_state(), "   \t\n", RepoActionKind::Restart)
            .await
            .expect_err("whitespace-only path must error");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "VALIDATION_FAILED");
    }
}
