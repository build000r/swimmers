use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use axum::http::StatusCode;
use futures::stream::{self, StreamExt};
use tokio::process::Command;

use super::{fetch_live_summary, AppState};
use crate::host_actions::{
    inspect_git_repo, RepoActionExecutor, RestartExecutor, SystemRepoActionExecutor,
};
use crate::native;
use crate::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayServiceEntry,
};
use crate::types::{
    DirEntry, DirListResponse, DirRepoActionResponse, DirRestartResponse, NativeDesktopApp,
    NativeDesktopOpenResponse, NativeDesktopStatusResponse, RepoActionKind, RepoActionState,
    RepoActionStatus, SessionState,
};

/// Max concurrent git probes per `list_dirs` call. Keeps a single listing from
/// fork-bombing the system when a repos directory has many git subdirs, while
/// still parallelizing enough to hide per-call git latency.
const GIT_PROBE_CONCURRENCY: usize = 16;

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
}

struct PendingEntry {
    name: String,
    entry_path: PathBuf,
    has_children: bool,
    modified_at: u64,
    services: Vec<String>,
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
        let service_path = resolved_base.join(&service.dir);
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

fn relative_repo_path(base: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(base).ok()?;
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    Some(components.join("/"))
}

pub fn services_for_directory(path: &Path, context: &OverlayServiceContext) -> Vec<String> {
    let Some(relative_path) = relative_repo_path(&context.base_path, path) else {
        return Vec::new();
    };
    if relative_path.is_empty() {
        return Vec::new();
    }

    let mut services = BTreeSet::new();
    for service in &context.services {
        if service.dir == relative_path
            || service.dir.starts_with(&format!("{relative_path}/"))
            || relative_path.starts_with(&format!("{}/", service.dir))
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

    let mut handles = Vec::new();
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
        handles.push(tokio::spawn(async move {
            let ok = client.get(&url).send().await.is_ok();
            (name, ok)
        }));
    }

    for handle in handles {
        if let Ok((name, ok)) = handle.await {
            map.insert(name, ok);
        }
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

    for service in services {
        if !requested.contains(&service.name) {
            continue;
        }
        let Some(cmd) = &service.restart else {
            continue;
        };
        let output = tokio::time::timeout(
            Duration::from_secs(240),
            Command::new("sh").arg("-c").arg(cmd).output(),
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

pub async fn list_dirs(
    state: &Arc<AppState>,
    path: Option<&str>,
    managed_only: bool,
    group: Option<&str>,
) -> Result<DirListResponse, ApiServiceError> {
    let base = dirs_base_path();

    if let Some(group_name) = group {
        let canonical_base = base.canonicalize().unwrap_or(base.clone());
        let dir_config = resolve_dir_config(&canonical_base);
        let group =
            dir_config.and_then(|config| config.groups.iter().find(|g| g.name == group_name));
        let Some(group) = group else {
            return Err(ApiServiceError::new(
                StatusCode::NOT_FOUND,
                "GROUP_NOT_FOUND",
                format!("no group named '{group_name}' in overlay"),
            ));
        };
        let entries = list_group_entries(group).await;
        return Ok(DirListResponse {
            path: canonical_base.to_string_lossy().into_owned(),
            entries,
            overlay_label: dir_config.map(|c| c.label.clone()),
            groups: dir_config
                .map(|c| c.groups.iter().map(|g| g.name.clone()).collect())
                .unwrap_or_default(),
        });
    }

    let request_started = Instant::now();
    let target = match path {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => base.clone(),
    };

    let (canonical_base, canonical) = resolve_target_path(base, target)?;
    let read_dir = std::fs::read_dir(&canonical).map_err(|error| {
        ApiServiceError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "DIR_READ_ERROR",
            error.to_string(),
        )
    })?;

    let dir_config = resolve_dir_config(&canonical_base);
    let managed_children = if managed_only && canonical == canonical_base {
        dir_config.and_then(|config| managed_base_child_names(config, &canonical_base))
    } else {
        None
    };

    let service_context = dir_config.map(|config| OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    });

    let mut pending: Vec<PendingEntry> = Vec::new();
    let mut unique_services: BTreeSet<String> = BTreeSet::new();
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
        if let Some(allowed) = &managed_children {
            if !allowed.contains(&name) {
                continue;
            }
        }

        let entry_path = entry.path();
        let has_children = std::fs::read_dir(&entry_path)
            .map(|read_dir| {
                read_dir.flatten().any(|child| {
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

        let services = service_context
            .as_ref()
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

    let probe_inputs: Vec<(PathBuf, _)> = pending
        .iter()
        .map(|pending_entry| (pending_entry.entry_path.clone(), state.repo_actions.clone()))
        .collect();

    let pending_phase_ms = request_started.elapsed().as_millis() as u64;
    let pending_count = pending.len();
    let probe_started = Instant::now();
    let probes: Vec<(Option<bool>, Option<RepoActionStatus>)> = stream::iter(probe_inputs)
        .map(|(entry_path, repo_actions)| async move {
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
        })
        .buffered(GIT_PROBE_CONCURRENCY)
        .collect()
        .await;
    let probe_phase_ms = probe_started.elapsed().as_millis() as u64;

    let candidates: Vec<ListCandidate> = pending
        .into_iter()
        .zip(probes)
        .map(|(pending_entry, (repo_dirty, repo_action))| ListCandidate {
            name: pending_entry.name,
            has_children: pending_entry.has_children,
            modified_at: pending_entry.modified_at,
            services: pending_entry.services,
            repo_dirty,
            repo_action,
        })
        .collect();

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

    let svc_meta: HashMap<&str, &OverlayServiceEntry> = dir_config
        .map(|config| {
            config
                .services
                .iter()
                .map(|service| (service.name.as_str(), service))
                .collect()
        })
        .unwrap_or_default();

    let mut entries: Vec<(DirEntry, u64)> = candidates
        .into_iter()
        .map(|candidate| {
            let is_running = if candidate.services.is_empty() {
                None
            } else {
                Some(
                    candidate
                        .services
                        .iter()
                        .any(|service| health_map.get(service).copied().unwrap_or(false)),
                )
            };
            let has_restart = candidate
                .services
                .iter()
                .any(|service| {
                    svc_meta
                        .get(service.as_str())
                        .and_then(|entry| entry.restart.as_ref())
                        .is_some()
                })
                .then_some(true);
            let open_url = candidate.services.iter().find_map(|service| {
                svc_meta
                    .get(service.as_str())
                    .and_then(|entry| entry.open_url.clone().or_else(|| entry.health_url.clone()))
            });
            (
                DirEntry {
                    name: candidate.name,
                    has_children: candidate.has_children,
                    is_running,
                    repo_dirty: candidate.repo_dirty,
                    repo_action: candidate.repo_action,
                    group: None,
                    full_path: None,
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
    let entries = entries.into_iter().map(|(entry, _)| entry).collect();
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
    })
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
