#![cfg_attr(not(feature = "personal-workflows"), allow(dead_code))]

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::host_actions::{
    inspect_git_repo, RepoActionExecutor, RepoActionTracker, RestartExecutor,
    SystemRepoActionExecutor,
};
use crate::session::overlay::{
    default_overlay, OverlayDirConfig, OverlayDirGroup, OverlayServiceEntry,
};
use crate::types::{
    DirEntry, DirListResponse, DirRepoActionRequest, DirRepoActionResponse, DirRestartRequest,
    DirRestartResponse, ErrorResponse, RepoActionKind, RepoActionStatus,
};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use futures::stream::{self, StreamExt};
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::process::Command;

/// Max concurrent git probes per `list_dirs` call. Keeps a single listing from
/// fork-bombing the system when a repos directory has many git subdirs, while
/// still parallelizing enough to hide per-call git latency.
const GIT_PROBE_CONCURRENCY: usize = 16;

#[derive(serde::Deserialize)]
struct DirQuery {
    path: Option<String>,
    managed_only: Option<bool>,
    group: Option<String>,
}

struct OverlayServiceContext {
    base_path: PathBuf,
    services: Vec<OverlayServiceEntry>,
}

struct ListCandidate {
    name: String,
    has_children: bool,
    modified_at: u64,
    services: Vec<String>,
    repo_dirty: Option<bool>,
    repo_action: Option<RepoActionStatus>,
}

/// Base path for directory browsing. Prefers `DIRS_BASE_PATH` env var, then
/// the overlay's `dev_sanity.services.base_path`, then the server's cwd.
fn dirs_base_path() -> PathBuf {
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
fn resolve_dir_config(path: &Path) -> Option<&'static OverlayDirConfig> {
    let overlay = default_overlay()?;
    overlay.find_dir_config(&path.to_string_lossy())
}

/// Compute which top-level children of `base` are "managed" by the overlay.
///
/// Derives the set from the `dir` fields of overlay service entries: each
/// entry's first path component is a managed child of the base directory.
fn managed_base_child_names(config: &OverlayDirConfig, base: &Path) -> Option<BTreeSet<String>> {
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

fn services_for_directory(path: &Path, context: &OverlayServiceContext) -> Vec<String> {
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

/// Check service health by sending HTTP GET requests to overlay-defined URLs.
async fn overlay_service_health_map(
    services: &[OverlayServiceEntry],
    requested: &[String],
) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    if requested.is_empty() {
        return map;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let mut handles = Vec::new();
    for service in services {
        if !requested.contains(&service.name) {
            continue;
        }
        let Some(url) = &service.health_url else {
            // No health URL — assume running if the service is declared.
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

/// Restart services by running overlay-defined shell commands.
async fn restart_services(
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

fn repo_action_error_code(error: &io::Error) -> (&'static str, StatusCode) {
    match error.kind() {
        io::ErrorKind::AlreadyExists => ("ACTION_ALREADY_RUNNING", StatusCode::CONFLICT),
        _ => ("ACTION_START_FAILED", StatusCode::INTERNAL_SERVER_ERROR),
    }
}

fn error_response(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(
            serde_json::to_value(ErrorResponse {
                code: code.to_string(),
                message: Some(message.into()),
            })
            .unwrap(),
        ),
    )
        .into_response()
}

fn resolve_target_path(base: PathBuf, target: PathBuf) -> Result<(PathBuf, PathBuf), Response> {
    let canonical = target.canonicalize().map_err(|_| {
        error_response(
            StatusCode::NOT_FOUND,
            "DIR_NOT_FOUND",
            format!("directory not found: {}", target.display()),
        )
    })?;

    let canonical_base = base.canonicalize().unwrap_or(base);
    if !canonical.starts_with(&canonical_base) {
        return Err(error_response(
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
///
/// Runs on Tokio's blocking pool via `spawn_blocking` — the nested
/// `std::fs::read_dir` walks would otherwise stall an async handler worker
/// the same way `inspect_git_repo` did before it was offloaded. Group configs
/// can aggregate many sources with many entries each, so this is a
/// latent hot-path stall waiting to happen.
async fn list_group_entries(group: &OverlayDirGroup) -> Vec<DirEntry> {
    let group = group.clone();
    tokio::task::spawn_blocking(move || list_group_entries_sync(&group))
        .await
        .unwrap_or_default()
}

fn list_group_entries_sync(group: &OverlayDirGroup) -> Vec<DirEntry> {
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

// GET /v1/dirs?path=...
async fn list_dirs(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Query(query): Query<DirQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    let base = dirs_base_path();

    // Handle group listing: return combined entries from the group's source dirs.
    if let Some(group_name) = &query.group {
        let canonical_base = base.canonicalize().unwrap_or(base.clone());
        let dir_config = resolve_dir_config(&canonical_base);
        let group = dir_config
            .and_then(|config| config.groups.iter().find(|g| &g.name == group_name));
        let Some(group) = group else {
            return error_response(
                StatusCode::NOT_FOUND,
                "GROUP_NOT_FOUND",
                format!("no group named '{group_name}' in overlay"),
            );
        };
        let entries = list_group_entries(group).await;
        return (
            StatusCode::OK,
            Json(
                serde_json::to_value(DirListResponse {
                    path: canonical_base.to_string_lossy().into_owned(),
                    entries,
                    overlay_label: dir_config.map(|c| c.label.clone()),
                    groups: dir_config
                        .map(|c| c.groups.iter().map(|g| g.name.clone()).collect())
                        .unwrap_or_default(),
                })
                .unwrap(),
            ),
        )
            .into_response();
    }

    let target = match &query.path {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => base.clone(),
    };

    let (canonical_base, canonical) = match resolve_target_path(base, target) {
        Ok(paths) => paths,
        Err(response) => return response,
    };

    let read_dir = match std::fs::read_dir(&canonical) {
        Ok(read_dir) => read_dir,
        Err(error) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "DIR_READ_ERROR",
                error.to_string(),
            );
        }
    };

    let managed_only = query.managed_only.unwrap_or(false);
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

    struct PendingEntry {
        name: String,
        entry_path: PathBuf,
        has_children: bool,
        modified_at: u64,
        services: Vec<String>,
    }

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

    // Probe git state for every entry concurrently. `inspect_git_repo` uses
    // `tokio::process::Command` with a per-call timeout so a slow repo can't
    // stall the Tokio worker, and `buffered(N)` bounds concurrent git forks.
    // Inputs are materialized as owned tuples up-front so the stream closure
    // has no borrows into `pending` (rustc can't infer HRTBs through it).
    let probe_inputs: Vec<(PathBuf, RepoActionTracker)> = pending
        .iter()
        .map(|pending_entry| (pending_entry.entry_path.clone(), state.repo_actions.clone()))
        .collect();

    let probes: Vec<(Option<bool>, Option<RepoActionStatus>)> = stream::iter(probe_inputs)
        .map(|(entry_path, repo_actions)| async move {
            let repo_summary = inspect_git_repo(&entry_path)
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

    let candidates: Vec<ListCandidate> = pending
        .into_iter()
        .zip(probes.into_iter())
        .map(
            |(pending_entry, (repo_dirty, repo_action))| ListCandidate {
                name: pending_entry.name,
                has_children: pending_entry.has_children,
                modified_at: pending_entry.modified_at,
                services: pending_entry.services,
                repo_dirty,
                repo_action,
            },
        )
        .collect();

    let health_map = if let Some(config) = dir_config {
        let services: Vec<String> = unique_services.into_iter().collect();
        overlay_service_health_map(&config.services, &services).await
    } else {
        HashMap::new()
    };

    // Build lookup from service name → overlay metadata for restart/open_url.
    let svc_meta: HashMap<&str, &OverlayServiceEntry> = dir_config
        .map(|config| {
            config
                .services
                .iter()
                .map(|s| (s.name.as_str(), s))
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
                .any(|svc| {
                    svc_meta
                        .get(svc.as_str())
                        .and_then(|e| e.restart.as_ref())
                        .is_some()
                })
                .then_some(true);
            let open_url = candidate
                .services
                .iter()
                .find_map(|svc| {
                    svc_meta.get(svc.as_str()).and_then(|e| {
                        e.open_url.clone().or_else(|| e.health_url.clone())
                    })
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
    let entries: Vec<DirEntry> = entries.into_iter().map(|(entry, _)| entry).collect();

    let groups = dir_config
        .map(|config| {
            config
                .groups
                .iter()
                .map(|g| g.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(DirListResponse {
                path: canonical.to_string_lossy().into_owned(),
                entries,
                overlay_label: dir_config.map(|c| c.label.clone()),
                groups,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

// POST /v1/dirs/restart
async fn restart_dir_services(
    Extension(auth): Extension<AuthInfo>,
    State(_state): State<Arc<AppState>>,
    Json(body): Json<DirRestartRequest>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    let requested_path = body.path.trim();
    if requested_path.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "path is required",
        );
    }

    let base = dirs_base_path();
    let target = PathBuf::from(requested_path);
    let (canonical_base, canonical) = match resolve_target_path(base, target) {
        Ok(paths) => paths,
        Err(response) => return response,
    };

    let Some(config) = resolve_dir_config(&canonical_base) else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "OVERLAY_UNAVAILABLE",
            "no skillbox-config overlay found with service definitions",
        );
    };

    let context = OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    };
    let matched_services = services_for_directory(&canonical, &context);
    if matched_services.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "NO_SERVICE_FOR_PATH",
            "no overlay service is mapped to this folder",
        );
    }

    if let Err(message) = restart_services(&config.services, &matched_services).await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "RESTART_FAILED", message);
    }

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(DirRestartResponse {
                ok: true,
                path: canonical.to_string_lossy().into_owned(),
                services: matched_services,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

// POST /v1/dirs/actions
async fn start_dir_repo_action(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<DirRepoActionRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match body.kind {
        RepoActionKind::Restart => start_restart_action(state, body).await,
        RepoActionKind::Open => error_response(
            StatusCode::BAD_REQUEST,
            "CLIENT_ONLY",
            "open actions are handled client-side",
        ),
        _ => {
            start_dir_repo_action_with_executor(state, body, Arc::new(SystemRepoActionExecutor))
                .await
        }
    }
}

/// Start a restart action by looking up overlay service commands and tracking
/// execution through the standard `RepoActionTracker`.
async fn start_restart_action(state: Arc<AppState>, body: DirRepoActionRequest) -> Response {
    let requested_path = body.path.trim();
    if requested_path.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "path is required",
        );
    }

    let base = dirs_base_path();
    let target = PathBuf::from(requested_path);
    let (canonical_base, canonical) = match resolve_target_path(base, target) {
        Ok(paths) => paths,
        Err(response) => return response,
    };

    let Some(config) = resolve_dir_config(&canonical_base) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "NO_OVERLAY",
            "no overlay configuration found for this path",
        );
    };

    let context = OverlayServiceContext {
        base_path: config.base_path.clone(),
        services: config.services.clone(),
    };
    let matched_services = services_for_directory(&canonical, &context);
    if matched_services.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "NO_SERVICE_FOR_PATH",
            "no overlay service is mapped to this folder",
        );
    }

    let commands: Vec<(String, String)> = config
        .services
        .iter()
        .filter(|s| matched_services.contains(&s.name))
        .filter_map(|s| s.restart.as_ref().map(|cmd| (s.name.clone(), cmd.clone())))
        .collect();

    if commands.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "NO_RESTART_COMMAND",
            "matched services have no restart command configured",
        );
    }

    let executor: Arc<dyn RepoActionExecutor> = Arc::new(RestartExecutor { commands });

    if let Err(error) = state
        .repo_actions
        .start(canonical.clone(), body.kind, executor)
        .await
    {
        let (code, status) = repo_action_error_code(&error);
        return error_response(status, code, error.to_string());
    }

    let status = state
        .repo_actions
        .status_for(&canonical)
        .await
        .unwrap_or(RepoActionStatus {
            kind: body.kind,
            state: crate::types::RepoActionState::Running,
            detail: None,
        });

    (
        StatusCode::ACCEPTED,
        Json(
            serde_json::to_value(DirRepoActionResponse {
                ok: true,
                path: canonical.to_string_lossy().into_owned(),
                status,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

async fn start_dir_repo_action_with_executor(
    state: Arc<AppState>,
    body: DirRepoActionRequest,
    executor: Arc<dyn RepoActionExecutor>,
) -> Response {
    let requested_path = body.path.trim();
    if requested_path.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "VALIDATION_FAILED",
            "path is required",
        );
    }

    let base = dirs_base_path();
    let target = PathBuf::from(requested_path);
    let (_canonical_base, canonical) = match resolve_target_path(base, target) {
        Ok(paths) => paths,
        Err(response) => return response,
    };

    let Some(repo_summary) = inspect_git_repo(&canonical).await.ok().flatten() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "NO_GIT_REPO",
            "path is not inside a git repository",
        );
    };

    if !repo_summary.dirty {
        return error_response(
            StatusCode::CONFLICT,
            "REPO_CLEAN",
            "repo has no pending changes to commit",
        );
    }

    if let Err(error) = state
        .repo_actions
        .start(repo_summary.repo_root.clone(), body.kind, executor)
        .await
    {
        let (code, status) = repo_action_error_code(&error);
        return error_response(status, code, error.to_string());
    }

    let status = state
        .repo_actions
        .status_for(&repo_summary.repo_root)
        .await
        .unwrap_or(RepoActionStatus {
            kind: body.kind,
            state: crate::types::RepoActionState::Running,
            detail: None,
        });

    (
        StatusCode::ACCEPTED,
        Json(
            serde_json::to_value(DirRepoActionResponse {
                ok: true,
                path: repo_summary.repo_root.to_string_lossy().into_owned(),
                status,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/dirs", get(list_dirs))
        .route("/v1/dirs/restart", post(restart_dir_services))
        .route("/v1/dirs/actions", post(start_dir_repo_action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::RepoActionKind;
    use axum::body::to_bytes;
    use axum::extract::{Json, Query, State};
    use axum::response::IntoResponse;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::process::Command as ProcessCommand;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: impl Into<OsString>) -> EnvGuard {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value.into());
        EnvGuard { key, previous }
    }

    struct FakeRepoActionExecutor {
        sleep_ms: u64,
        detail: Option<String>,
    }

    impl FakeRepoActionExecutor {
        fn sleeping_ok(sleep_ms: u64) -> Self {
            Self {
                sleep_ms,
                detail: None,
            }
        }
    }

    impl RepoActionExecutor for FakeRepoActionExecutor {
        fn execute(
            &self,
            _repo_root: std::path::PathBuf,
            _kind: RepoActionKind,
        ) -> io::Result<Option<String>> {
            if self.sleep_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.sleep_ms));
            }
            Ok(self.detail.clone())
        }
    }

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: None,
            file_store: None,
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    fn init_dirty_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("repo dir");
        let status = ProcessCommand::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");
        fs::write(path.join("README.md"), "dirty\n").expect("write readme");
    }

    #[test]
    fn managed_base_child_names_derives_from_service_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        fs::create_dir_all(base.join("alpha")).expect("alpha");
        fs::create_dir_all(base.join("services").join("nested-app")).expect("nested");
        fs::create_dir_all(base.join("zeta")).expect("zeta");

        let config = OverlayDirConfig {
            label: "test".into(),
            base_path: base.to_path_buf(),
            services: vec![
                OverlayServiceEntry {
                    name: "svc-alpha".into(),
                    dir: "alpha".into(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
                OverlayServiceEntry {
                    name: "svc-nested".into(),
                    dir: "services/nested-app".into(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
            ],
            groups: Vec::new(),
        };

        let children = managed_base_child_names(&config, base).expect("should have children");
        assert!(children.contains("alpha"));
        assert!(children.contains("services"));
        assert!(!children.contains("zeta"));
    }

    #[test]
    fn services_for_directory_matches_overlay_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        fs::create_dir_all(base.join("alpha")).expect("alpha");
        fs::create_dir_all(base.join("services").join("nested-app")).expect("nested");

        let context = OverlayServiceContext {
            base_path: base.to_path_buf(),
            services: vec![
                OverlayServiceEntry {
                    name: "svc-alpha".into(),
                    dir: "alpha".into(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
                OverlayServiceEntry {
                    name: "svc-nested".into(),
                    dir: "services/nested-app".into(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
            ],
        };

        let svcs = services_for_directory(&base.join("alpha"), &context);
        assert_eq!(svcs, vec!["svc-alpha"]);

        let svcs = services_for_directory(&base.join("services").join("nested-app"), &context);
        assert_eq!(svcs, vec!["svc-nested"]);

        let svcs = services_for_directory(&base.join("zeta"), &context);
        assert!(svcs.is_empty());
    }

    #[tokio::test]
    async fn overlay_health_map_reports_services_without_urls_as_running() {
        let services = vec![
            OverlayServiceEntry {
                name: "svc-no-url".into(),
                dir: "x".into(),
                health_url: None,
                restart: None,
                open_url: None,
            },
            OverlayServiceEntry {
                name: "svc-bad-url".into(),
                dir: "y".into(),
                health_url: Some("http://127.0.0.1:1/__nonexistent".into()),
                restart: None,
                open_url: None,
            },
        ];
        let requested = vec!["svc-no-url".to_string(), "svc-bad-url".to_string()];
        let map = overlay_service_health_map(&services, &requested).await;
        assert_eq!(map.get("svc-no-url"), Some(&true));
        assert_eq!(map.get("svc-bad-url"), Some(&false));
    }

    #[test]
    fn dirs_base_path_honors_explicit_override() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let explicit_root = dir.path().join("custom-root");
        fs::create_dir_all(&explicit_root).expect("custom root");
        let _base = set_env_var("DIRS_BASE_PATH", explicit_root.as_os_str().to_os_string());

        assert_eq!(dirs_base_path(), explicit_root);
    }

    #[tokio::test]
    async fn list_dirs_filters_managed_entries_from_overlay() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repo");
        fs::create_dir_all(base.join("alpha")).expect("alpha");
        fs::create_dir_all(base.join("services").join("nested-app")).expect("nested");
        fs::create_dir_all(base.join("zeta")).expect("zeta");
        fs::create_dir_all(base.join(".hidden")).expect("hidden");

        let _base_env = set_env_var("DIRS_BASE_PATH", base.as_os_str().to_os_string());

        // Without overlay, managed_only still returns all non-hidden dirs.
        let response = list_dirs(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Query(DirQuery {
                path: None,
                managed_only: Some(false),
                group: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let entries = json["entries"].as_array().expect("entries");
        assert!(entries.len() >= 3); // alpha, services, zeta
    }

    #[tokio::test]
    async fn restart_without_overlay_returns_unavailable() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repo");
        fs::create_dir_all(base.join("alpha")).expect("alpha");

        let _base_env = set_env_var("DIRS_BASE_PATH", base.as_os_str().to_os_string());

        let response = restart_dir_services(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(DirRestartRequest {
                path: base.join("alpha").to_string_lossy().into_owned(),
            }),
        )
        .await
        .into_response();

        // Without overlay, restart is unavailable.
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn list_dirs_reports_repo_dirty_and_running_action() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = base.join("swimmers");
        init_dirty_git_repo(&repo);
        let _base_env = set_env_var("DIRS_BASE_PATH", base.as_os_str().to_os_string());

        let state = test_state();
        state
            .repo_actions
            .start(
                repo.clone(),
                RepoActionKind::Commit,
                Arc::new(FakeRepoActionExecutor::sleeping_ok(200)),
            )
            .await
            .expect("start repo action");

        let response = list_dirs(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            Query(DirQuery {
                path: None,
                managed_only: Some(false),
                group: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let entry = json["entries"]
            .as_array()
            .and_then(|entries| entries.first())
            .expect("entry");
        assert_eq!(entry["name"], "swimmers");
        assert_eq!(entry["repo_dirty"], true);
        assert_eq!(entry["repo_action"]["kind"], "commit");
        assert_eq!(entry["repo_action"]["state"], "running");
    }

    #[tokio::test]
    async fn start_dir_repo_action_accepts_dirty_repo() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("repos");
        let repo = base.join("swimmers");
        init_dirty_git_repo(&repo);
        let _base_env = set_env_var("DIRS_BASE_PATH", base.as_os_str().to_os_string());

        let state = test_state();
        let response = start_dir_repo_action_with_executor(
            state.clone(),
            DirRepoActionRequest {
                path: repo.to_string_lossy().into_owned(),
                kind: RepoActionKind::Commit,
            },
            Arc::new(FakeRepoActionExecutor::sleeping_ok(200)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let json = response_json(response).await;
        assert_eq!(json["ok"], true);
        assert_eq!(json["status"]["state"], "running");
        assert_eq!(
            state
                .repo_actions
                .status_for(&repo)
                .await
                .map(|status| status.state),
            Some(crate::types::RepoActionState::Running)
        );
    }

    struct PathGuard(Option<OsString>);

    impl Drop for PathGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    fn install_fake_slow_git(sleep_ms: u64) -> (tempfile::TempDir, PathGuard) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("fake git tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        // Sleep per git invocation, then return the `-C PATH` value for
        // `rev-parse --show-toplevel` and empty output for `status --short`
        // (clean repo). Any other subcommand exits silently.
        let sleep_seconds = format!("{:.3}", sleep_ms as f64 / 1000.0);
        let script = format!(
            r#"#!/bin/sh
set -eu
sleep {sleep_seconds}
repo_root=""
while [ $# -gt 0 ]; do
  case "$1" in
    -C)
      shift
      repo_root="$1"
      shift
      ;;
    *)
      break
      ;;
  esac
done

case "${{1-}}" in
  rev-parse)
    printf '%s\n' "$repo_root"
    ;;
  status)
    ;;
  *)
    ;;
esac
"#
        );

        let git_path = bin_dir.join("git");
        std::fs::write(&git_path, script).expect("write fake git script");
        let mut perms = std::fs::metadata(&git_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&git_path, perms).expect("chmod fake git");

        let original_path = std::env::var_os("PATH");
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = original_path.as_ref() {
            entries.extend(std::env::split_paths(existing).map(|p| p.into_os_string()));
        }
        std::env::set_var(
            "PATH",
            std::env::join_paths(entries).expect("join fake git path"),
        );

        (dir, PathGuard(original_path))
    }

    /// Regression guard for the "swimmers API unavailable (timed out while
    /// trying to create a session)" class of bug: `list_dirs` used to run
    /// synchronous `git` subprocesses per entry on the async handler worker,
    /// which stalled the Tokio runtime long enough for `POST /v1/sessions`
    /// to hit its 10s client timeout. The fix runs each probe via
    /// `spawn_blocking` + a bounded `buffered(16)` stream.
    ///
    /// With 24 fake repos × 200ms per probe, a serial blocking path would
    /// take ≥4.8s; the parallel path must land well under 2s. Bound is loose
    /// enough to absorb CI noise.
    #[tokio::test]
    async fn list_dirs_parallelizes_git_probes_under_slow_git() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let (_fake_git_dir, _path_guard) = install_fake_slow_git(200);

        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().join("repos");
        std::fs::create_dir_all(&base).expect("repos base");
        for i in 0..24 {
            std::fs::create_dir_all(base.join(format!("repo-{i:02}"))).expect("repo subdir");
        }
        let _base_env = set_env_var("DIRS_BASE_PATH", base.as_os_str().to_os_string());

        let started = std::time::Instant::now();
        let response = list_dirs(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Query(DirQuery {
                path: None,
                managed_only: Some(false),
                group: None,
            }),
        )
        .await
        .into_response();
        let elapsed = started.elapsed();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let entries = json["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 24, "all repo entries should be present");
        assert!(
            elapsed < Duration::from_secs(2),
            "list_dirs must parallelize git probes; serial would be ~4.8s, got {elapsed:?}"
        );
    }
}
