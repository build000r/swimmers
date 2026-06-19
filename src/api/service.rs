use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::OnceLock;
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
use crate::persistence::file_store::FileStore;
use crate::session::actor::{ActorHandle, SessionCommand};
#[cfg(test)]
use crate::session::overlay::OverlayDirGroup;
use crate::session::overlay::{OverlayDirConfig, OverlayServiceEntry};
use crate::thought::probe::{run_thought_config_probe, ThoughtConfigProbeResult};
use crate::thought::runtime_config::ThoughtConfig;
use crate::thought_ui::thought_config_ui_metadata;
use crate::types::{
    CreateSessionsBatchResponse, CreateSessionsBatchResult, DirEntry, DirGroupMemberships,
    DirListResponse, DirRepoActionResponse, DirRestartResponse, ErrorResponse, LaunchTargetSummary,
    NativeDesktopApp, NativeDesktopOpenResponse, NativeDesktopStatusResponse, PlanFileResponse,
    RepoActionKind, RepoActionState, RepoActionStatus, RepoTheme, SessionBatchMembership,
    SessionState, SessionSummary, SpawnTool, ThoughtConfigResponse,
};

#[path = "service/attention_group.rs"]
mod attention_group;
#[path = "service/group_membership.rs"]
mod group_membership;
#[path = "service/repo_search.rs"]
mod repo_search;
#[path = "service_directory.rs"]
mod service_directory;

pub use attention_group::open_native_attention_group_for_host;
use group_membership::load_dir_group_memberships;
pub use group_membership::update_dir_group_memberships;
#[cfg(test)]
use group_membership::{
    apply_group_membership_update, normalize_group_update_names, resolve_group_membership_path,
    update_dir_group_memberships_preflight, update_dir_group_memberships_with_config,
};
#[cfg(test)]
pub use repo_search::clear_repo_search_cache_for_tests;
pub use repo_search::list_repo_search_entries;
#[cfg(test)]
use repo_search::{
    list_repo_search_entries_inner, repo_search_child_dirs, repo_search_visit,
    scan_repo_search_roots_sync, RepoSearchVisit, REPO_SEARCH_DEFAULT_MAX_DEPTH,
};
#[cfg(test)]
use service_directory::canonical_path_string;
use service_directory::{
    annotate_dir_entry_groups, effective_dir_config_for_base, has_visible_child_dirs,
    list_effective_group_entries, modified_secs, service_dir_path,
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
    crate::advisory::attach_advisories_to_sessions(&mut sessions);
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
    if dirs.iter().any(|dir| dir.trim().is_empty()) {
        return Err(ApiServiceError::new(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "dirs must not include blank entries",
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

pub async fn list_dirs(
    state: &Arc<AppState>,
    path: Option<&str>,
    managed_only: bool,
    group: Option<&str>,
) -> Result<DirListResponse, ApiServiceError> {
    let base = dirs_base_path();
    let memberships = load_dir_group_memberships(state).await;

    if let Some(group_name) = group {
        return list_group_dir_response(state, base, group_name, &memberships).await;
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

async fn list_group_dir_response(
    state: &Arc<AppState>,
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
        decorate_group_entries(state, &mut entries, config).await;
        annotate_dir_entry_groups(&mut entries, &canonical_base, config, memberships);
    }
    Ok(DirListResponse {
        path: canonical_base.to_string_lossy().into_owned(),
        entries,
        overlay_label: dir_config.as_ref().map(|c| c.label.clone()),
        groups: dir_groups(dir_config.as_ref()),
        launch_targets: launch_targets_for(dir_config.as_ref()),
        default_launch_target: default_launch_target_for(
            dir_config.as_ref(),
            Some(group_name),
            canonical_base.as_path(),
        ),
    })
}

async fn decorate_group_entries(
    state: &Arc<AppState>,
    entries: &mut [DirEntry],
    config: &OverlayDirConfig,
) {
    let service_context = OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    };
    let mut pending_indices = Vec::new();
    let mut pending = Vec::new();
    let mut unique_services = BTreeSet::new();

    for (index, entry) in entries.iter().enumerate() {
        let Some(entry_path) = entry.full_path.as_deref().map(PathBuf::from) else {
            continue;
        };
        let services = services_for_directory(&entry_path, &service_context);
        extend_unique_services(&mut unique_services, &services);
        pending_indices.push(index);
        pending.push(PendingEntry {
            name: entry.name.clone(),
            entry_path: entry_path.clone(),
            has_children: entry.has_children,
            modified_at: modified_secs(&entry_path),
            services,
        });
    }

    let probes = probe_pending_entries(state, &pending).await;
    let services: Vec<String> = unique_services.into_iter().collect();
    let health_map = overlay_service_health_map(&config.services, &services).await;
    let svc_meta = service_metadata_map(&config.services);

    for ((index, pending_entry), (repo_dirty, repo_action)) in pending_indices
        .into_iter()
        .zip(pending.into_iter())
        .zip(probes.into_iter())
    {
        let (is_running, has_restart, open_url) =
            service_entry_metadata(&pending_entry.services, &health_map, &svc_meta);
        let entry = &mut entries[index];
        entry.is_running = is_running;
        entry.repo_dirty = repo_dirty;
        entry.repo_action = repo_action;
        entry.has_restart = has_restart;
        entry.open_url = open_url;
    }
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
        default_launch_target: default_launch_target_for(Some(config), None, canonical),
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
        default_launch_target: default_launch_target_for(dir_config, None, canonical),
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
    path: &Path,
) -> Option<String> {
    Some(
        config
            .map(|config| config.launch.default_for_group_or_path(group, path))
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
    let commands = restart_commands_for_path(&canonical, config)?;

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
mod tests;
