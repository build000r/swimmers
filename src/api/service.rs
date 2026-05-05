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
use crate::persistence::file_store::FileStore;
use crate::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayServiceEntry,
};
use crate::types::{
    DirEntry, DirGroupMembershipUpdateRequest, DirGroupMembershipUpdateResponse,
    DirGroupMemberships, DirListResponse, DirRepoActionResponse, DirRestartResponse,
    LaunchTargetSummary, NativeDesktopApp, NativeDesktopOpenResponse, NativeDesktopStatusResponse,
    RepoActionKind, RepoActionState, RepoActionStatus, SessionState,
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
        let raw_path = base_path.join(&service.dir);
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
    let read_dir = std::fs::read_dir(&canonical).map_err(|error| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::health::BridgeHealthState;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
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
