#![cfg_attr(not(feature = "personal-workflows"), allow(dead_code))]

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
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::session::overlay::{default_overlay, OverlayDirConfig, OverlayServiceEntry};
use crate::types::{
    DirEntry, DirListResponse, DirRestartRequest, DirRestartResponse, ErrorResponse,
};

#[derive(serde::Deserialize)]
struct DirQuery {
    path: Option<String>,
    managed_only: Option<bool>,
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

    let resolved_base = config.base_path.canonicalize().unwrap_or(config.base_path.clone());
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

fn services_for_directory(
    path: &Path,
    context: &OverlayServiceContext,
) -> Vec<String> {
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

        let services = service_context
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

    let health_map = if let Some(config) = dir_config {
        let services: Vec<String> = unique_services.into_iter().collect();
        overlay_service_health_map(&config.services, &services).await
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
                overlay_label: dir_config.map(|c| c.label.clone()),
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

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/dirs", get(list_dirs))
        .route("/v1/dirs/restart", post(restart_dir_services))
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
    use axum::body::to_bytes;
    use axum::extract::{Json, Query, State};
    use axum::response::IntoResponse;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::fs;
    use std::sync::Arc;
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
        })
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
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
                },
                OverlayServiceEntry {
                    name: "svc-nested".into(),
                    dir: "services/nested-app".into(),
                    health_url: None,
                    restart: None,
                },
            ],
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
                },
                OverlayServiceEntry {
                    name: "svc-nested".into(),
                    dir: "services/nested-app".into(),
                    health_url: None,
                    restart: None,
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
            },
            OverlayServiceEntry {
                name: "svc-bad-url".into(),
                dir: "y".into(),
                health_url: Some("http://127.0.0.1:1/__nonexistent".into()),
                restart: None,
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
}
