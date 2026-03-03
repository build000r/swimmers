use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tokio::process::Command;
use tracing::warn;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    DirEntry, DirListResponse, DirRestartRequest, DirRestartResponse, ErrorResponse,
};

#[derive(serde::Deserialize)]
struct DirQuery {
    path: Option<String>,
    managed_only: Option<bool>,
}

struct EnvServiceContext {
    env_manager_root: PathBuf,
    repo_root: PathBuf,
    service_repo_map: Vec<(String, String)>,
}

struct ListCandidate {
    name: String,
    has_children: bool,
    modified_at: u64,
    services: Vec<String>,
}

/// Base path for directory browsing. Falls back to the server's cwd.
fn dirs_base_path() -> PathBuf {
    std::env::var("DIRS_BASE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

fn env_manager_root(base: &Path) -> Option<PathBuf> {
    let mut candidates = vec![base.join(".env-manager")];
    if let Some(parent) = base.parent() {
        candidates.push(parent.join(".env-manager"));
    }
    candidates.into_iter().find(|candidate| candidate.is_dir())
}

fn parse_shell_array_items(line: &str) -> Vec<String> {
    let Some(start) = line.find('(') else {
        return Vec::new();
    };
    let Some(end) = line.rfind(')') else {
        return Vec::new();
    };
    if end <= start + 1 {
        return Vec::new();
    }

    line[start + 1..end]
        .split_whitespace()
        .map(|raw| {
            raw.trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_string()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

fn env_manager_targets(env_manager_root: &Path) -> Vec<String> {
    let mut targets = BTreeSet::new();

    let sync_path = env_manager_root.join("sync.sh");
    if let Ok(contents) = std::fs::read_to_string(&sync_path) {
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("BACKEND_TARGETS=") || trimmed.starts_with("FRONTEND_TARGETS=") {
                for target in parse_shell_array_items(trimmed) {
                    targets.insert(target);
                }
            }
        }
    }

    // Fallback for environments where sync.sh isn't available.
    if targets.is_empty() {
        let out_dir = env_manager_root.join("out");
        if let Ok(entries) = std::fs::read_dir(out_dir) {
            for entry in entries.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                if !ft.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "local" {
                    continue;
                }
                targets.insert(name);
            }
        }
    }

    targets.into_iter().collect()
}

fn target_code_dir(base: &Path, target: &str) -> PathBuf {
    match target {
        "cfo-discord-bot" => base.join("cfo").join("discord_bot"),
        "unclawg-approval-feedback-api" | "openclawth-approval-feedback-api" => base
            .join("unclawg")
            .join("services")
            .join("approval_feedback_api"),
        _ => base.join(target),
    }
}

fn managed_base_child_names(base: &Path) -> Option<BTreeSet<String>> {
    let manager_root = env_manager_root(base)?;
    let targets = env_manager_targets(&manager_root);
    if targets.is_empty() {
        return None;
    }

    let mut children = BTreeSet::new();
    for target in targets {
        let project_dir = target_code_dir(base, &target);
        let Ok(canonical_project) = project_dir.canonicalize() else {
            continue;
        };
        if !canonical_project.starts_with(base) {
            continue;
        }
        let Ok(relative) = canonical_project.strip_prefix(base) else {
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

fn normalize_repo_rel_path(raw: &str) -> String {
    raw.trim().trim_matches('/').replace('\\', "/")
}

fn parse_assoc_array_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let key_end = trimmed.find(']')?;
    let key = trimmed.get(1..key_end)?.trim();
    if key.is_empty() {
        return None;
    }

    let remainder = trimmed.get(key_end + 1..)?.trim_start();
    let remainder = remainder.strip_prefix('=')?.trim_start();
    let quote = remainder.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_portion = remainder.get(1..)?;
    let value_end = value_portion.find(quote)?;
    let value = value_portion.get(..value_end)?.trim();
    if value.is_empty() {
        return None;
    }
    Some((key.to_string(), normalize_repo_rel_path(value)))
}

fn env_manager_service_repo_map(env_manager_root: &Path) -> Vec<(String, String)> {
    let script_path = env_manager_root
        .join("scripts")
        .join("project")
        .join("project.sh");
    let Ok(contents) = std::fs::read_to_string(script_path) else {
        return Vec::new();
    };

    let mut in_block = false;
    let mut seen = BTreeSet::new();
    let mut parsed = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if !in_block {
            if trimmed.starts_with("declare -A SERVICE_REPO=(") {
                in_block = true;
            }
            continue;
        }
        if trimmed == ")" {
            break;
        }
        let Some((service, repo_rel)) = parse_assoc_array_line(trimmed) else {
            continue;
        };
        if seen.insert((service.clone(), repo_rel.clone())) {
            parsed.push((service, repo_rel));
        }
    }
    parsed
}

fn env_service_context(base: &Path) -> Option<EnvServiceContext> {
    let env_manager_root = env_manager_root(base)?;
    let repo_root = env_manager_root.parent()?.to_path_buf();
    let repo_root = repo_root.canonicalize().unwrap_or(repo_root);
    let service_repo_map = env_manager_service_repo_map(&env_manager_root);
    if service_repo_map.is_empty() {
        return None;
    }
    Some(EnvServiceContext {
        env_manager_root,
        repo_root,
        service_repo_map,
    })
}

fn relative_repo_path(repo_root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(repo_root).ok()?;
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    Some(components.join("/"))
}

fn services_for_directory(path: &Path, context: &EnvServiceContext) -> Vec<String> {
    let Some(relative_path) = relative_repo_path(&context.repo_root, path) else {
        return Vec::new();
    };
    if relative_path.is_empty() {
        return Vec::new();
    }

    let mut services = BTreeSet::new();
    for (service, service_repo_path) in &context.service_repo_map {
        if service_repo_path == &relative_path
            || service_repo_path.starts_with(&format!("{relative_path}/"))
            || relative_path.starts_with(&format!("{service_repo_path}/"))
        {
            services.insert(service.clone());
        }
    }

    services.into_iter().collect()
}

async fn env_service_health_map(
    env_manager_root: &Path,
    services: &[String],
) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    if services.is_empty() {
        return map;
    }

    let services_arg = format!("services={}", services.join(" "));
    let output = match Command::new("make")
        .current_dir(env_manager_root)
        .arg("--no-print-directory")
        .arg("project")
        .arg("action=status")
        .arg(services_arg)
        .env_remove("PROJECT_ACTION")
        .env_remove("PROJECT_SERVICES")
        .env_remove("PROJECT_PROFILE")
        .env_remove("PROJECT_MOBILE")
        .env_remove("PROJECT_WATCH")
        .output()
        .await
    {
        Ok(output) => output,
        Err(error) => {
            warn!(
                error = %error,
                root = %env_manager_root.display(),
                "failed to run env-manager status"
            );
            return map;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("SERVICE")
            || trimmed.starts_with("-------")
            || trimmed.starts_with('[')
        {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(service) = parts.next() else {
            continue;
        };
        let Some(health) = parts.next() else {
            continue;
        };
        let Some(run_handle) = parts.next() else {
            continue;
        };
        // "running" should match operator intent (process exists), not only health.
        // env-manager status can report HEALTH=down with a live RUN HANDLE pid.
        if health == "up" || health == "down" {
            let running = health == "up" || run_handle != "-";
            map.insert(service.to_string(), running);
        }
    }

    if !output.status.success() {
        warn!(
            status = %output.status,
            root = %env_manager_root.display(),
            "env-manager status exited non-zero"
        );
    }

    map
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

fn trim_failure_details(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr.chars().take(600).collect();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let tail = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("restart failed");
    tail.trim().chars().take(600).collect()
}

async fn restart_services(env_manager_root: &Path, services: &[String]) -> Result<(), String> {
    if services.is_empty() {
        return Err("no restartable services mapped for this path".to_string());
    }

    let services_arg = format!("services={}", services.join(" "));
    let output = tokio::time::timeout(
        Duration::from_secs(240),
        Command::new("make")
            .current_dir(env_manager_root)
            .arg("--no-print-directory")
            .arg("project")
            .arg("action=restart")
            .arg(services_arg)
            .arg("mobile=1")
            .arg("watch=0")
            .env_remove("PROJECT_ACTION")
            .env_remove("PROJECT_SERVICES")
            .env_remove("PROJECT_PROFILE")
            .env_remove("PROJECT_MOBILE")
            .env_remove("PROJECT_WATCH")
            .output(),
    )
    .await
    .map_err(|_| "restart timed out after 240s".to_string())?
    .map_err(|error| error.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(trim_failure_details(&output))
    }
}

// GET /v1/dirs?path=...
async fn list_dirs(
    Extension(auth): Extension<AuthInfo>,
    State(_state): State<Arc<AppState>>,
    Query(query): Query<DirQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    let base = dirs_base_path();
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
    let managed_children = if managed_only && canonical == canonical_base {
        managed_base_child_names(&canonical_base)
    } else {
        None
    };

    let env_context = env_service_context(&canonical_base);

    let mut candidates: Vec<ListCandidate> = Vec::new();
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

        let services = env_context
            .as_ref()
            .map(|context| services_for_directory(&entry_path, context))
            .unwrap_or_default();
        for service in &services {
            unique_services.insert(service.clone());
        }

        candidates.push(ListCandidate {
            name,
            has_children,
            modified_at,
            services,
        });
    }

    let health_map = if let Some(context) = &env_context {
        let services: Vec<String> = unique_services.into_iter().collect();
        env_service_health_map(&context.env_manager_root, &services).await
    } else {
        HashMap::new()
    };

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
            (
                DirEntry {
                    name: candidate.name,
                    has_children: candidate.has_children,
                    is_running,
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

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(DirListResponse {
                path: canonical.to_string_lossy().into_owned(),
                entries,
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

    let Some(context) = env_service_context(&canonical_base) else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ENV_MANAGER_UNAVAILABLE",
            "could not locate .env-manager service metadata",
        );
    };

    let services = services_for_directory(&canonical, &context);
    if services.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "NO_SERVICE_FOR_PATH",
            "no .env-manager service is mapped to this folder",
        );
    }

    if let Err(message) = restart_services(&context.env_manager_root, &services).await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "RESTART_FAILED", message);
    }

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(DirRestartResponse {
                ok: true,
                path: canonical.to_string_lossy().into_owned(),
                services,
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
}
