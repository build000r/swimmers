#![cfg_attr(not(feature = "personal-workflows"), allow(dead_code))]

use crate::api::service::{
    list_dirs as list_dirs_service, restart_dir_services as restart_dir_services_service,
    start_dir_repo_action as start_dir_repo_action_service, ApiServiceError,
};
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{DirRepoActionRequest, DirRestartRequest, ErrorResponse};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use std::sync::Arc;

#[derive(serde::Deserialize)]
struct DirQuery {
    path: Option<String>,
    managed_only: Option<bool>,
    group: Option<String>,
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

fn service_error_response(error: ApiServiceError) -> Response {
    error_response(error.status(), error.code(), error.message())
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

    match list_dirs_service(
        &state,
        query.path.as_deref(),
        query.managed_only.unwrap_or(false),
        query.group.as_deref(),
    )
    .await
    {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::to_value(response).unwrap()),
        )
            .into_response(),
        Err(error) => service_error_response(error),
    }
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

    match restart_dir_services_service(&body.path).await {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::to_value(response).unwrap()),
        )
            .into_response(),
        Err(error) => service_error_response(error),
    }
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

    match start_dir_repo_action_service(state, &body.path, body.kind).await {
        Ok(response) => (
            StatusCode::ACCEPTED,
            Json(serde_json::to_value(response).unwrap()),
        )
            .into_response(),
        Err(error) => service_error_response(error),
    }
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
    use crate::api::service::{
        dirs_base_path, list_group_entries_sync, managed_base_child_names,
        overlay_service_health_map, restart_services, services_for_directory,
        start_repo_action_with_executor, OverlayServiceContext,
    };
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::host_actions::RepoActionExecutor;
    use crate::session::overlay::{OverlayDirConfig, OverlayDirGroup, OverlayServiceEntry};
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
    use std::path::Path;
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
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
                std::time::Duration::from_secs(15),
            )),
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
    fn list_group_entries_sync_merges_sources_and_preserves_full_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_a = dir.path().join("skills-a");
        let src_b = dir.path().join("skills-b");
        fs::create_dir_all(src_a.join("alpha").join("nested")).expect("alpha nested");
        fs::create_dir_all(src_b.join("beta")).expect("beta");
        fs::create_dir_all(src_b.join("alpha")).expect("duplicate alpha");
        fs::create_dir_all(src_b.join(".hidden")).expect("hidden");

        let group = OverlayDirGroup {
            name: "skills".into(),
            dirs: vec![src_a.clone(), src_b.clone()],
        };

        let entries = list_group_entries_sync(&group);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);

        let alpha = entries
            .iter()
            .find(|entry| entry.name == "alpha")
            .expect("alpha entry");
        let alpha_path = src_a
            .join("alpha")
            .canonicalize()
            .expect("canonical alpha path")
            .to_string_lossy()
            .into_owned();
        assert!(alpha.has_children);
        assert_eq!(alpha.full_path.as_deref(), Some(alpha_path.as_str()));
    }

    #[tokio::test]
    async fn restart_services_runs_only_requested_commands() {
        let dir = tempfile::tempdir().expect("tempdir");
        let requested_marker = dir.path().join("requested.txt");
        let skipped_marker = dir.path().join("skipped.txt");
        let services = vec![
            OverlayServiceEntry {
                name: "svc-requested".into(),
                dir: "alpha".into(),
                health_url: None,
                restart: Some(format!(
                    "printf requested > '{}'",
                    requested_marker.display()
                )),
                open_url: None,
            },
            OverlayServiceEntry {
                name: "svc-skipped".into(),
                dir: "beta".into(),
                health_url: None,
                restart: Some(format!("printf skipped > '{}'", skipped_marker.display())),
                open_url: None,
            },
        ];

        restart_services(&services, &["svc-requested".to_string()])
            .await
            .expect("requested service restart should succeed");

        assert_eq!(
            fs::read_to_string(&requested_marker).expect("requested marker"),
            "requested"
        );
        assert!(
            !skipped_marker.exists(),
            "unrequested service should not run"
        );
    }

    #[tokio::test]
    async fn restart_services_rejects_empty_requested_list() {
        let err = restart_services(&[], &[])
            .await
            .expect_err("empty requested list");
        assert_eq!(err, "no restartable services mapped for this path");
    }

    #[tokio::test]
    async fn restart_services_surfaces_command_failures() {
        let services = vec![OverlayServiceEntry {
            name: "svc-failing".into(),
            dir: "alpha".into(),
            health_url: None,
            restart: Some("printf boom >&2; exit 9".into()),
            open_url: None,
        }];

        let err = restart_services(&services, &["svc-failing".to_string()])
            .await
            .expect_err("restart command should fail");
        assert!(err.contains("svc-failing"));
        assert!(err.contains("boom"));
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
        let response = start_repo_action_with_executor(
            state.clone(),
            &repo.to_string_lossy(),
            RepoActionKind::Commit,
            Arc::new(FakeRepoActionExecutor::sleeping_ok(200)),
        )
        .await
        .expect("repo action should start");

        assert!(response.ok);
        assert_eq!(
            response.status.state,
            crate::types::RepoActionState::Running
        );
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
        let mut perms = std::fs::metadata(&git_path)
            .expect("metadata")
            .permissions();
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
