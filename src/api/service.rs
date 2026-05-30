use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

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
use crate::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayServiceEntry,
};
use crate::thought::probe::{run_thought_config_probe, ThoughtConfigProbeResult};
use crate::thought::runtime_config::ThoughtConfig;
use crate::thought_ui::thought_config_ui_metadata;
use crate::types::{
    CreateSessionsBatchResponse, CreateSessionsBatchResult, DirEntry,
    DirGroupMembershipUpdateRequest, DirGroupMembershipUpdateResponse, DirGroupMemberships,
    DirListResponse, DirRepoActionResponse, DirRepoSearchResponse, DirRestartResponse,
    ErrorResponse, LaunchTargetSummary, NativeAttentionGroupOpenRequest,
    NativeAttentionGroupOpenResponse, NativeDesktopApp, NativeDesktopOpenResponse,
    NativeDesktopStatusResponse, PlanFileResponse, RepoActionKind, RepoActionState,
    RepoActionStatus, RepoTheme, SessionBatchMembership, SessionState, SessionSummary, SpawnTool,
    ThoughtConfigResponse,
};

/// Max concurrent git probes per `list_dirs` call. Keeps a single listing from
/// fork-bombing the system when a repos directory has many git subdirs, while
/// still parallelizing enough to hide per-call git latency.
const GIT_PROBE_CONCURRENCY: usize = 16;
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

pub struct OverlayServiceContext {
    pub base_path: PathBuf,
    pub services: Vec<OverlayServiceEntry>,
}

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

fn scan_repo_search_roots_sync(roots: &[PathBuf], max_depth: usize) -> Vec<DirEntry> {
    let mut queue = VecDeque::new();
    for root in roots {
        queue.push_back((root.clone(), 0usize));
    }

    let mut seen = BTreeSet::new();
    let mut repos = Vec::new();
    while let Some((path, depth)) = queue.pop_front() {
        let canonical = path.canonicalize().unwrap_or(path);
        if !seen.insert(canonical.clone()) {
            continue;
        }

        if canonical.join(".git").exists() {
            repos.push(repo_search_entry(&canonical));
            continue;
        }

        if depth >= max_depth {
            continue;
        }

        let Ok(read_dir) = std::fs::read_dir(&canonical) else {
            continue;
        };
        for child in read_dir.flatten() {
            let Ok(file_type) = child.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = child.file_name().to_string_lossy().into_owned();
            if !should_descend_for_repo_search(&name) {
                continue;
            }
            queue.push_back((child.path(), depth + 1));
        }
    }

    repos.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.full_path.cmp(&right.full_path))
    });
    repos
}

pub async fn list_repo_search_entries() -> Result<DirRepoSearchResponse, ApiServiceError> {
    let roots = repo_search_roots();
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

    let max_depth = repo_search_max_depth();
    if let Ok(cache) = repo_search_cache().lock() {
        if let Some(cache) = cache.as_ref() {
            if cache.roots == roots
                && cache.max_depth == max_depth
                && cache.generated_at.elapsed() < REPO_SEARCH_CACHE_TTL
            {
                return Ok(DirRepoSearchResponse {
                    roots: root_labels,
                    entries: cache.entries.clone(),
                });
            }
        }
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

    if let Ok(mut cache) = repo_search_cache().lock() {
        *cache = Some(RepoSearchCacheEntry {
            roots: roots.clone(),
            max_depth,
            generated_at: Instant::now(),
            entries: entries.clone(),
        });
    }

    Ok(DirRepoSearchResponse {
        roots: root_labels,
        entries,
    })
}

/// Resolve the overlay dir config for the given path.
pub fn resolve_dir_config(path: &Path) -> Option<&'static OverlayDirConfig> {
    let overlay = default_overlay()?;
    overlay.find_dir_config(&path.to_string_lossy())
}

/// Compute which top-level children of `base` are "managed" by the overlay.
pub fn managed_base_child_names(
    config: &OverlayDirConfig,
    base: &Path,
) -> Option<BTreeSet<String>> {
    if config.services.is_empty() {
        return None;
    }

    let resolved_base = config
        .base_path
        .canonicalize()
        .unwrap_or(config.base_path.clone());
    let canonical_base = base.canonicalize().unwrap_or(base.to_path_buf());

    let mut children = BTreeSet::new();
    for service in &config.services {
        let service_path = service_dir_path(&resolved_base, &service.dir);
        let Ok(canonical) = service_path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&canonical_base) {
            continue;
        }
        let Ok(relative) = canonical.strip_prefix(&canonical_base) else {
            continue;
        };
        let Some(Component::Normal(name)) = relative.components().next() else {
            continue;
        };
        children.insert(name.to_string_lossy().into_owned());
    }

    if children.is_empty() {
        None
    } else {
        Some(children)
    }
}

fn service_dir_path(base: &Path, dir: &str) -> PathBuf {
    let path = PathBuf::from(dir);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

pub fn services_for_directory(path: &Path, context: &OverlayServiceContext) -> Vec<String> {
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let canonical_base = context
        .base_path
        .canonicalize()
        .unwrap_or_else(|_| context.base_path.clone());
    if target == canonical_base {
        return Vec::new();
    };

    let mut services = BTreeSet::new();
    for service in &context.services {
        let service_path = service_dir_path(&canonical_base, &service.dir);
        let canonical_service = service_path
            .canonicalize()
            .unwrap_or_else(|_| service_path.clone());
        if canonical_service == target
            || canonical_service.starts_with(&target)
            || target.starts_with(&canonical_service)
        {
            services.insert(service.name.clone());
        }
    }

    services.into_iter().collect()
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
            let ok = client.get(&url).send().await.is_ok();
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

    let mut ran_command = false;
    for service in services {
        if !requested.contains(&service.name) {
            continue;
        }
        let Some(cmd) = &service.restart else {
            continue;
        };
        ran_command = true;
        let output = tokio::time::timeout(
            Duration::from_secs(240),
            // `kill_on_drop` ensures the timeout actually reaps the child: when
            // the timeout fires the `output()` future is dropped, and without
            // this the spawned `sh` (and its descendants' controlling process)
            // would be orphaned and keep running past the deadline.
            Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| format!("restart of {} timed out after 240s", service.name))?
        .map_err(|error| error.to_string())?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail: String = if stderr.is_empty() {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .rev()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("restart failed")
                    .trim()
                    .chars()
                    .take(600)
                    .collect()
            } else {
                stderr.chars().take(600).collect()
            };
            return Err(format!("{}: {}", service.name, detail));
        }
    }

    if !ran_command {
        return Err("matched services have no restart command configured".to_string());
    }

    Ok(())
}

pub fn resolve_target_path(
    base: PathBuf,
    target: PathBuf,
) -> Result<(PathBuf, PathBuf), ApiServiceError> {
    let canonical = target.canonicalize().map_err(|_| {
        ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            format!("directory not found: {}", target.display()),
        )
    })?;

    let canonical_base = base.canonicalize().unwrap_or(base);
    if !canonical.starts_with(&canonical_base) {
        return Err(ApiServiceError::new(
            StatusCode::FORBIDDEN,
            "DIR_OUTSIDE_BASE",
            "path is outside the allowed base directory",
        ));
    }

    Ok((canonical_base, canonical))
}

/// List entries from a virtual directory group, combining children from all
/// source directories. Each entry carries its full absolute path since entries
/// come from multiple distinct parents.
pub async fn list_group_entries(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let group = group.clone();
    tokio::task::spawn_blocking(move || list_group_entries_sync(&group))
        .await
        .unwrap_or_default()
}

pub fn list_group_entries_sync(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let mut seen = BTreeSet::new();
    let mut entries: Vec<(DirEntry, u64)> = Vec::new();

    for entry_path in &group.paths {
        let Some(name) = entry_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name.starts_with('.') || !seen.insert(name.clone()) {
            continue;
        }

        let full_path = entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.clone())
            .to_string_lossy()
            .into_owned();

        entries.push((
            DirEntry {
                name,
                has_children: false,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: Some(full_path),
                has_restart: None,
                open_url: None,
            },
            modified_secs(entry_path),
        ));
    }

    for source_dir in &group.dirs {
        let Ok(read_dir) = std::fs::read_dir(source_dir) else {
            continue;
        };
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
            if !seen.insert(name.clone()) {
                continue;
            }

            let entry_path = entry.path();
            let has_children = std::fs::read_dir(&entry_path)
                .map(|rd| {
                    rd.flatten().any(|child| {
                        child.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                            && !child.file_name().to_string_lossy().starts_with('.')
                    })
                })
                .unwrap_or(false);

            let modified_at = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);

            let full_path = entry_path
                .canonicalize()
                .unwrap_or(entry_path)
                .to_string_lossy()
                .into_owned();

            entries.push((
                DirEntry {
                    name,
                    has_children,
                    is_running: None,
                    repo_dirty: None,
                    repo_action: None,
                    group: None,
                    groups: Vec::new(),
                    full_path: Some(full_path),
                    has_restart: None,
                    open_url: None,
                },
                modified_at,
            ));
        }
    }

    entries.sort_by(|(a, _), (b, _)| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.into_iter().map(|(entry, _)| entry).collect()
}

pub fn list_effective_group_entries_sync(
    group: &OverlayDirGroup,
    memberships: &DirGroupMemberships,
) -> Vec<DirEntry> {
    let Some(delta) = memberships.groups.get(&group.name) else {
        return list_group_entries_sync(group);
    };

    let mut seen_names = BTreeSet::new();
    let mut entries: Vec<(DirEntry, u64)> = list_group_entries_sync(group)
        .into_iter()
        .filter(|entry| {
            entry
                .full_path
                .as_deref()
                .map(|path| !delta.exclude_paths.contains(path))
                .unwrap_or(true)
        })
        .map(|entry| {
            seen_names.insert(entry.name.clone());
            let modified_at = entry
                .full_path
                .as_deref()
                .map(|path| modified_secs(Path::new(path)))
                .unwrap_or(0);
            (entry, modified_at)
        })
        .collect();

    for raw_path in &delta.include_paths {
        if delta.exclude_paths.contains(raw_path) {
            continue;
        }
        let path = PathBuf::from(raw_path);
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name.starts_with('.') || !seen_names.insert(name.clone()) {
            continue;
        }

        entries.push((
            DirEntry {
                name,
                has_children: false,
                is_running: None,
                repo_dirty: None,
                repo_action: None,
                group: None,
                groups: Vec::new(),
                full_path: Some(canonical_path_string(&path)),
                has_restart: None,
                open_url: None,
            },
            modified_secs(&path),
        ));
    }

    entries.sort_by(|(a, _), (b, _)| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.into_iter().map(|(entry, _)| entry).collect()
}

async fn list_effective_group_entries(
    group: &OverlayDirGroup,
    memberships: &DirGroupMemberships,
) -> Vec<DirEntry> {
    let group = group.clone();
    let memberships = memberships.clone();
    tokio::task::spawn_blocking(move || list_effective_group_entries_sync(&group, &memberships))
        .await
        .unwrap_or_default()
}

fn canonical_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn overlay_group_contains_path(group: &OverlayDirGroup, canonical_path: &Path) -> bool {
    if group
        .paths
        .iter()
        .any(|path| path.canonicalize().unwrap_or_else(|_| path.clone()) == canonical_path)
    {
        return true;
    }

    let Some(parent) = canonical_path.parent() else {
        return false;
    };
    group
        .dirs
        .iter()
        .any(|dir| dir.canonicalize().unwrap_or_else(|_| dir.clone()) == parent)
}

fn effective_groups_for_path(
    config: &OverlayDirConfig,
    memberships: &DirGroupMemberships,
    path: &Path,
) -> Vec<String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let normalized = canonical.to_string_lossy();
    let mut groups = Vec::new();
    for group in &config.groups {
        let delta = memberships.groups.get(&group.name);
        if delta
            .map(|delta| delta.exclude_paths.contains(normalized.as_ref()))
            .unwrap_or(false)
        {
            continue;
        }
        let overlay_member = overlay_group_contains_path(group, &canonical);
        let user_member = delta
            .map(|delta| delta.include_paths.contains(normalized.as_ref()))
            .unwrap_or(false);
        if overlay_member || user_member {
            groups.push(group.name.clone());
        }
    }
    groups
}

fn annotate_dir_entry_groups(
    entries: &mut [DirEntry],
    parent: &Path,
    config: &OverlayDirConfig,
    memberships: &DirGroupMemberships,
) {
    for entry in entries {
        let path = entry
            .full_path
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| parent.join(&entry.name));
        entry.groups = effective_groups_for_path(config, memberships, &path);
    }
}

fn has_visible_child_dirs(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|read_dir| {
            read_dir.flatten().any(|child| {
                child.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                    && !child.file_name().to_string_lossy().starts_with('.')
            })
        })
        .unwrap_or(false)
}

fn modified_secs(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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
        for service in &services {
            unique_services.insert(service.clone());
        }

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

pub async fn list_managed_service_entries(
    state: &Arc<AppState>,
    config: &OverlayDirConfig,
) -> Vec<DirEntry> {
    let base_path = config
        .base_path
        .canonicalize()
        .unwrap_or(config.base_path.clone());
    let context = OverlayServiceContext {
        base_path: base_path.clone(),
        services: config.services.clone(),
    };
    let mut seen_paths = BTreeSet::new();
    let mut unique_services = BTreeSet::new();
    let mut pending = Vec::new();

    for service in &config.services {
        let raw_path = service_dir_path(&base_path, &service.dir);
        let Ok(entry_path) = raw_path.canonicalize() else {
            continue;
        };
        if !entry_path.is_dir() || !seen_paths.insert(entry_path.clone()) {
            continue;
        }

        let name = entry_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| service.name.clone());
        let services = services_for_directory(&entry_path, &context);
        for service_name in &services {
            unique_services.insert(service_name.clone());
        }

        pending.push(PendingEntry {
            name,
            has_children: false,
            modified_at: modified_secs(&entry_path),
            services,
            entry_path,
        });
    }

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
    let dir_config = resolve_dir_config(&canonical_base);
    if let Some(response) = list_managed_root_response(
        state,
        canonical_base.as_path(),
        canonical.as_path(),
        dir_config,
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
        dir_config,
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
    let store = state.current_file_store().ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PERSISTENCE_UNAVAILABLE",
            "directory group edits require file persistence",
        )
    })?;

    let base = dirs_base_path();
    let canonical_base = base.canonicalize().unwrap_or(base.clone());
    let dir_config = resolve_dir_config(&canonical_base).ok_or_else(|| {
        ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OVERLAY_UNAVAILABLE",
            "directory group edits require an overlay with directory groups",
        )
    })?;
    if dir_config.groups.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "GROUPS_UNAVAILABLE",
            "no directory groups are configured for this overlay",
        ));
    }

    update_dir_group_memberships_with_config(store, &canonical_base, dir_config, body).await
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
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "GROUP_PATH_REQUIRED",
            "path is required",
        ));
    }
    let path = PathBuf::from(trimmed);
    let canonical = path.canonicalize().map_err(|_| {
        ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            format!("directory not found: {trimmed}"),
        )
    })?;
    if !canonical.is_dir() {
        return Err(ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            format!("directory not found: {trimmed}"),
        ));
    }
    if canonical.starts_with(canonical_base)
        || config
            .groups
            .iter()
            .any(|group| overlay_group_contains_path(group, &canonical))
    {
        return Ok(canonical);
    }

    Err(ApiServiceError::new(
        StatusCode::FORBIDDEN,
        "DIR_OUTSIDE_BASE",
        "path is outside the allowed directory group roots",
    ))
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
    for group in remove {
        let delta = memberships.groups.entry(group).or_default();
        delta.include_paths.remove(path);
        delta.exclude_paths.insert(path.to_string());
    }
    for group in add {
        let delta = memberships.groups.entry(group).or_default();
        delta.exclude_paths.remove(path);
        delta.include_paths.insert(path.to_string());
    }
    prune_empty_group_deltas(memberships);
}

async fn list_group_dir_response(
    base: PathBuf,
    group_name: &str,
    memberships: &DirGroupMemberships,
) -> Result<DirListResponse, ApiServiceError> {
    let canonical_base = base.canonicalize().unwrap_or(base.clone());
    let dir_config = resolve_dir_config(&canonical_base);
    let group = dir_config.and_then(|config| config.groups.iter().find(|g| g.name == group_name));
    let Some(group) = group else {
        return Err(ApiServiceError::new(
            StatusCode::NOT_FOUND,
            "GROUP_NOT_FOUND",
            format!("no group named '{group_name}' in overlay"),
        ));
    };
    let mut entries = list_effective_group_entries(group, memberships).await;
    if let Some(config) = dir_config {
        annotate_dir_entry_groups(&mut entries, &canonical_base, config, memberships);
    }
    Ok(DirListResponse {
        path: canonical_base.to_string_lossy().into_owned(),
        entries,
        overlay_label: dir_config.map(|c| c.label.clone()),
        groups: dir_groups(dir_config),
        launch_targets: launch_targets_for(dir_config),
        default_launch_target: default_launch_target_for(dir_config, Some(group_name)),
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

pub async fn start_restart_action(
    state: Arc<AppState>,
    path: &str,
    kind: RepoActionKind,
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
    let (canonical_base, canonical) = resolve_target_path(base, target)?;

    let Some(config) = resolve_dir_config(&canonical_base) else {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_OVERLAY",
            "no overlay configuration found for this path",
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

    let commands: Vec<(String, String)> = config
        .services
        .iter()
        .filter(|service| matched_services.contains(&service.name))
        .filter_map(|service| {
            service
                .restart
                .as_ref()
                .map(|command| (service.name.clone(), command.clone()))
        })
        .collect();

    if commands.is_empty() {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "NO_RESTART_COMMAND",
            "matched services have no restart command configured",
        ));
    }

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
    if !status.supported {
        return Err(NativeOpenServiceError::Unsupported {
            reason: status.reason,
        });
    }

    let plan = plan_attention_group_sessions(
        state.supervisor.list_sessions().await,
        request.max_sessions.unwrap_or(6),
        &request.current_session_ids,
        request.include_unnumbered_sessions,
    );
    if plan.visible.is_empty() {
        if !request.focus && !request.current_session_ids.is_empty() {
            return native::clear_native_attention_group()
                .await
                .map_err(|error| NativeOpenServiceError::Internal(error.to_string()));
        }
        return Err(NativeOpenServiceError::NoAttentionSessions);
    }

    let mut response = native::open_native_attention_group(
        app,
        ghostty_mode,
        &plan.visible,
        request.focus,
        request.layout.unwrap_or_default(),
    )
    .await
    .map_err(|error| NativeOpenServiceError::Internal(error.to_string()))?;
    response.backlog_session_ids = plan
        .backlog
        .iter()
        .map(|session| session.session_id.clone())
        .collect();
    Ok(response)
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
    let mut candidates = sessions
        .into_iter()
        .filter(attention_group_session_is_eligible)
        .filter(|session| include_unnumbered_sessions || tmux_name_is_numbered(&session.tmux_name))
        .map(AttentionCandidate::from)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return AttentionGroupPlan {
            visible: Vec::new(),
            backlog: Vec::new(),
        };
    }

    let current_ids = current_session_ids.iter().collect::<HashSet<_>>();
    let mut visible = Vec::<AttentionCandidate>::new();
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

    if visible.is_empty() {
        let anchor_index = best_attention_anchor_index(&candidates);
        visible.push(candidates.remove(anchor_index));
    }

    while visible.len() < limit && !candidates.is_empty() {
        let next_index = best_attention_fill_index(&visible, &candidates);
        visible.push(candidates.remove(next_index));
    }

    candidates.sort_by(|a, b| {
        best_adjacency_to_group(b, &visible)
            .cmp(&best_adjacency_to_group(a, &visible))
            .then_with(|| b.session.last_activity_at.cmp(&a.session.last_activity_at))
            .then_with(|| a.session.session_id.cmp(&b.session.session_id))
    });

    let visible_sessions = visible
        .into_iter()
        .map(|candidate| candidate.session)
        .collect::<Vec<_>>();
    let backlog_sessions = candidates
        .into_iter()
        .filter(|candidate| !current_ids.contains(&candidate.session.session_id))
        .map(|candidate| candidate.session)
        .collect::<Vec<_>>();

    AttentionGroupPlan {
        visible: visible_sessions,
        backlog: backlog_sessions,
    }
}

fn attention_group_session_is_eligible(session: &SessionSummary) -> bool {
    session.session_id != NATIVE_ATTENTION_GROUP_SESSION_ID
        && session.tmux_name != NATIVE_ATTENTION_GROUP_TMUX_NAME
        && remote_sessions::split_remote_session_id(&session.session_id).is_none()
        && session_ready_for_operator_group_input(session)
}

fn tmux_name_is_numbered(tmux_name: &str) -> bool {
    let trimmed = tmux_name.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch.is_ascii_digit())
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
    candidates
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            best_adjacency_to_group(a, visible)
                .cmp(&best_adjacency_to_group(b, visible))
                .then_with(|| a.session.last_activity_at.cmp(&b.session.last_activity_at))
                .then_with(|| b.session.session_id.cmp(&a.session.session_id))
        })
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn best_adjacency_to_group(candidate: &AttentionCandidate, visible: &[AttentionCandidate]) -> i32 {
    visible
        .iter()
        .map(|visible| attention_adjacency_score(candidate, visible))
        .max()
        .unwrap_or(0)
}

fn attention_adjacency_score(a: &AttentionCandidate, b: &AttentionCandidate) -> i32 {
    if a.session.session_id == b.session.session_id {
        return 0;
    }
    let mut score = 0;
    if !a.repo.is_empty() && a.repo == b.repo {
        score += 100;
    }
    if !a.family.is_empty() && a.family == b.family {
        score += 70;
    }
    if a.batch.is_some() && a.batch == b.batch {
        score += 50;
    }
    if a.session.tool.is_some() && a.session.tool == b.session.tool {
        score += 5;
    }
    score
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
    fn effective_groups_merge_overlay_includes_and_excludes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let frontend = dir.path().join("frontend-app");
        let backend = dir.path().join("backend-app");
        let skills_root = dir.path().join("skills");
        let skill = skills_root.join("alpha-skill");
        std::fs::create_dir_all(&frontend).expect("frontend");
        std::fs::create_dir_all(&backend).expect("backend");
        std::fs::create_dir_all(&skill).expect("skill");

        let config = OverlayDirConfig {
            label: "test".into(),
            base_path: dir.path().to_path_buf(),
            services: Vec::new(),
            groups: vec![
                OverlayDirGroup {
                    name: "frontend".into(),
                    paths: vec![frontend.clone()],
                    dirs: Vec::new(),
                },
                OverlayDirGroup {
                    name: "backend".into(),
                    paths: vec![backend.clone()],
                    dirs: Vec::new(),
                },
                OverlayDirGroup {
                    name: "skills".into(),
                    paths: Vec::new(),
                    dirs: vec![skills_root],
                },
            ],
            launch: crate::session::overlay::OverlayLaunchConfig::local_only(),
        };

        let mut memberships = DirGroupMemberships::default();
        memberships
            .groups
            .entry("frontend".into())
            .or_default()
            .include_paths
            .insert(canonical_path_string(&backend));
        memberships
            .groups
            .entry("backend".into())
            .or_default()
            .exclude_paths
            .insert(canonical_path_string(&backend));
        memberships
            .groups
            .entry("skills".into())
            .or_default()
            .exclude_paths
            .insert(canonical_path_string(&skill));

        assert_eq!(
            effective_groups_for_path(&config, &memberships, &frontend),
            vec!["frontend".to_string()]
        );
        assert_eq!(
            effective_groups_for_path(&config, &memberships, &backend),
            vec!["frontend".to_string()]
        );
        assert!(effective_groups_for_path(&config, &memberships, &skill).is_empty());
    }

    #[test]
    fn list_effective_group_entries_applies_user_include_and_exclude_deltas() {
        let dir = tempfile::tempdir().expect("tempdir");
        let overlay_repo = dir.path().join("overlay-repo");
        let user_repo = dir.path().join("user-repo");
        let source = dir.path().join("source");
        let source_child = source.join("source-child");
        std::fs::create_dir_all(&overlay_repo).expect("overlay repo");
        std::fs::create_dir_all(&user_repo).expect("user repo");
        std::fs::create_dir_all(&source_child).expect("source child");

        let group = OverlayDirGroup {
            name: "frontend".into(),
            paths: vec![overlay_repo.clone()],
            dirs: vec![source],
        };
        let mut memberships = DirGroupMemberships::default();
        let delta = memberships.groups.entry("frontend".into()).or_default();
        delta
            .exclude_paths
            .insert(canonical_path_string(&overlay_repo));
        delta
            .include_paths
            .insert(canonical_path_string(&user_repo));

        let entries = list_effective_group_entries_sync(&group, &memberships);
        let names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["source-child", "user-repo"]);
        assert!(entries.iter().all(|entry| entry.full_path.is_some()));
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
