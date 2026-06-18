use crate::api::envelope::{error_response, success_json};
use crate::api::remote_sessions;
use crate::api::service::{
    list_dirs as list_dirs_service, list_repo_search_entries as list_repo_search_entries_service,
    restart_dir_services as restart_dir_services_service,
    start_dir_repo_action as start_dir_repo_action_service,
    update_dir_group_memberships as update_dir_group_memberships_service, ApiServiceError,
};
use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{DirGroupMembershipUpdateRequest, DirRepoActionRequest, DirRestartRequest};
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
    target: Option<String>,
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

    if let Some(target) = query
        .target
        .as_deref()
        .map(str::trim)
        .filter(|target| !target.is_empty() && *target != "local")
    {
        return match remote_sessions::list_remote_dirs(
            target,
            query.path.as_deref(),
            query.managed_only.unwrap_or(false),
            query.group.as_deref(),
        )
        .await
        {
            Ok(response) => success_json(StatusCode::OK, &response),
            Err(error) => error.into_response(),
        };
    }

    match list_dirs_service(
        &state,
        query.path.as_deref(),
        query.managed_only.unwrap_or(false),
        query.group.as_deref(),
    )
    .await
    {
        Ok(response) => success_json(StatusCode::OK, &response),
        Err(error) => service_error_response(error),
    }
}

// GET /v1/dirs/repositories
async fn list_repo_search_entries(Extension(auth): Extension<AuthInfo>) -> impl IntoResponse {
    list_repo_search_entries_response(auth).await
}

async fn list_repo_search_entries_response(auth: AuthInfo) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    match list_repo_search_entries_service().await {
        Ok(response) => success_json(StatusCode::OK, &response),
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
        Ok(response) => success_json(StatusCode::OK, &response),
        Err(error) => service_error_response(error),
    }
}

// POST /v1/dirs/actions
async fn start_dir_repo_action(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<DirRepoActionRequest>,
) -> Response {
    start_dir_repo_action_response(auth, state, body).await
}

async fn start_dir_repo_action_response(
    auth: AuthInfo,
    state: Arc<AppState>,
    body: DirRepoActionRequest,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match start_dir_repo_action_service(state, &body.path, body.kind).await {
        Ok(response) => success_json(StatusCode::ACCEPTED, &response),
        Err(error) => service_error_response(error),
    }
}

// POST /v1/dirs/group-memberships
async fn update_dir_group_memberships(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<DirGroupMembershipUpdateRequest>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsWrite) {
        return resp;
    }

    match update_dir_group_memberships_service(state, body).await {
        Ok(response) => success_json(StatusCode::OK, &response),
        Err(error) => service_error_response(error),
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/dirs", get(list_dirs))
        .route("/v1/dirs/repositories", get(list_repo_search_entries))
        .route("/v1/dirs/restart", post(restart_dir_services))
        .route("/v1/dirs/actions", post(start_dir_repo_action))
        .route(
            "/v1/dirs/group-memberships",
            post(update_dir_group_memberships),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::service::{
        dirs_base_path, list_group_entries_sync, list_managed_service_entries,
        managed_base_child_names, overlay_service_health_map, restart_services,
        services_for_directory, start_repo_action_with_executor, OverlayServiceContext,
    };
    use crate::api::PublishedSelectionState;
    use crate::auth::{OBSERVER_SCOPES, OPERATOR_SCOPES};
    use crate::config::Config;
    use crate::host_actions::RepoActionExecutor;
    use crate::session::overlay::{
        OverlayDirConfig, OverlayDirGroup, OverlayLaunchConfig, OverlayServiceEntry,
    };
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{DirGroupMembershipUpdateRequest, RepoActionKind};
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

    fn p95_duration(mut samples: Vec<Duration>) -> Duration {
        assert!(!samples.is_empty(), "p95 requires at least one sample");
        samples.sort_unstable();
        let index = samples
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1);
        samples[index]
    }

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
            launch: OverlayLaunchConfig::local_only(),
        };

        let children = managed_base_child_names(&config, base).expect("should have children");
        assert!(children.contains("alpha"));
        assert!(children.contains("services"));
        assert!(!children.contains("zeta"));
    }

    #[tokio::test]
    async fn managed_service_entries_preserve_exact_repo_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let finalreceipts = base.join("finalreceipts");
        let swimmers = base.join("opensource").join("swimmers");
        let hard_repo = dir.path().join("hard").join("mmd-pcb");
        fs::create_dir_all(&finalreceipts).expect("finalreceipts");
        fs::create_dir_all(swimmers.join("src")).expect("swimmers");
        fs::create_dir_all(&hard_repo).expect("hard repo");
        fs::create_dir_all(base.join("zeta")).expect("zeta");

        let config = OverlayDirConfig {
            label: "test".into(),
            base_path: base.to_path_buf(),
            services: vec![
                OverlayServiceEntry {
                    name: "finalreceipts".into(),
                    dir: "finalreceipts".into(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
                OverlayServiceEntry {
                    name: "swimmers".into(),
                    dir: "opensource/swimmers".into(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
                OverlayServiceEntry {
                    name: "mmd-pcb".into(),
                    dir: hard_repo.to_string_lossy().into_owned(),
                    health_url: None,
                    restart: None,
                    open_url: None,
                },
            ],
            groups: Vec::new(),
            launch: OverlayLaunchConfig::local_only(),
        };

        let entries = list_managed_service_entries(&test_state(), &config).await;
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert!(names.contains(&"finalreceipts"));
        assert!(names.contains(&"swimmers"));
        assert!(names.contains(&"mmd-pcb"));
        assert!(!names.contains(&"opensource"));
        assert!(!names.contains(&"zeta"));
        assert!(entries.iter().all(|entry| !entry.has_children));
        let swimmers_path = swimmers
            .canonicalize()
            .expect("canonical swimmers")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "swimmers")
                .and_then(|entry| entry.full_path.as_deref()),
            Some(swimmers_path.as_str())
        );
        let hard_path = hard_repo
            .canonicalize()
            .expect("canonical hard repo")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "mmd-pcb")
                .and_then(|entry| entry.full_path.as_deref()),
            Some(hard_path.as_str())
        );
    }

    #[test]
    fn services_for_directory_matches_overlay_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        fs::create_dir_all(base.join("alpha")).expect("alpha");
        fs::create_dir_all(base.join("services").join("nested-app")).expect("nested");
        let hard_repo = dir.path().join("..").join("hard").join("mmd-pcb");
        fs::create_dir_all(&hard_repo).expect("hard repo");

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
                OverlayServiceEntry {
                    name: "mmd-pcb".into(),
                    dir: hard_repo.to_string_lossy().into_owned(),
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

        let svcs = services_for_directory(&hard_repo, &context);
        assert_eq!(svcs, vec!["mmd-pcb"]);
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

    #[tokio::test]
    async fn overlay_health_map_treats_failed_http_status_as_not_running() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test health server");
        let addr = listener.local_addr().expect("local addr");
        let app = axum::Router::new().route(
            "/health",
            axum::routing::get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
        );
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve test health server");
        });

        let services = vec![OverlayServiceEntry {
            name: "svc-failing".into(),
            dir: "x".into(),
            health_url: Some(format!("http://{addr}/health")),
            restart: None,
            open_url: None,
        }];
        let requested = vec!["svc-failing".to_string()];

        let map = overlay_service_health_map(&services, &requested).await;
        assert_eq!(map.get("svc-failing"), Some(&false));
    }

    #[tokio::test]
    async fn overlay_health_map_uses_configured_url_status_without_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test health server");
        let addr = listener.local_addr().expect("local addr");
        let app = axum::Router::new()
            .route(
                "/health/empty",
                axum::routing::get(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/health/auth",
                axum::routing::get(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route(
                "/health/redirect",
                axum::routing::get(|| async { axum::response::Redirect::temporary("/login") }),
            )
            .route("/login", axum::routing::get(|| async { StatusCode::OK }));
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve test health server");
        });

        let services = vec![
            OverlayServiceEntry {
                name: "svc-empty".into(),
                dir: "empty".into(),
                health_url: Some(format!("http://{addr}/health/empty")),
                restart: None,
                open_url: None,
            },
            OverlayServiceEntry {
                name: "svc-auth".into(),
                dir: "auth".into(),
                health_url: Some(format!("http://{addr}/health/auth")),
                restart: None,
                open_url: None,
            },
            OverlayServiceEntry {
                name: "svc-redirect".into(),
                dir: "redirect".into(),
                health_url: Some(format!("http://{addr}/health/redirect")),
                restart: None,
                open_url: None,
            },
        ];
        let requested = vec![
            "svc-empty".to_string(),
            "svc-auth".to_string(),
            "svc-redirect".to_string(),
        ];

        let map = overlay_service_health_map(&services, &requested).await;
        assert_eq!(map.get("svc-empty"), Some(&true));
        assert_eq!(map.get("svc-auth"), Some(&false));
        assert_eq!(map.get("svc-redirect"), Some(&false));
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
            paths: Vec::new(),
            dirs: vec![src_a.clone(), src_b],
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

    #[test]
    fn list_group_entries_sync_includes_exact_paths_as_launch_targets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("finalreceipts");
        let source = dir.path().join("source");
        fs::create_dir_all(repo.join("src")).expect("repo");
        fs::create_dir_all(source.join("backend")).expect("backend");

        let group = OverlayDirGroup {
            name: "frontend".into(),
            paths: vec![repo.clone()],
            dirs: vec![source],
        };

        let entries = list_group_entries_sync(&group);
        let exact = entries
            .iter()
            .find(|entry| entry.name == "finalreceipts")
            .expect("exact path entry");
        let child = entries
            .iter()
            .find(|entry| entry.name == "backend")
            .expect("source child entry");

        assert!(!exact.has_children, "exact group paths should launch");
        let repo_path = repo
            .canonicalize()
            .expect("canonical repo")
            .to_string_lossy()
            .into_owned();
        assert_eq!(exact.full_path.as_deref(), Some(repo_path.as_str()));
        assert_eq!(child.name, "backend");
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
    async fn restart_services_rejects_matched_services_without_commands() {
        let services = vec![OverlayServiceEntry {
            name: "svc-no-restart".into(),
            dir: "alpha".into(),
            health_url: None,
            restart: None,
            open_url: None,
        }];

        let err = restart_services(&services, &["svc-no-restart".to_string()])
            .await
            .expect_err("matched service has no restart command");
        assert_eq!(err, "matched services have no restart command configured");
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
                target: None,
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
    async fn update_group_memberships_requires_write_scope() {
        let response = update_dir_group_memberships(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(DirGroupMembershipUpdateRequest {
                path: "/tmp".to_string(),
                add: vec!["frontend".to_string()],
                remove: Vec::new(),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn list_repo_search_entries_requires_read_scope() {
        let response = list_repo_search_entries(Extension(AuthInfo::new(Vec::new())))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["code"], "NOT_AUTHORIZED");
    }

    #[tokio::test]
    async fn list_repo_search_entries_returns_empty_search_response() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        crate::api::service::clear_repo_search_cache_for_tests();
        let dir = tempfile::tempdir().expect("tempdir");
        let missing_root = dir.path().join("missing-root");
        let _roots_env = set_env_var(
            "SWIMMERS_REPO_SEARCH_ROOTS",
            missing_root.as_os_str().to_os_string(),
        );

        let response = list_repo_search_entries(Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["entries"].as_array().expect("entries").len(), 0);
        assert!(json
            .get("roots")
            .and_then(|roots| roots.as_array())
            .is_none_or(|roots| roots.is_empty()));
    }

    #[tokio::test]
    async fn start_dir_repo_action_requires_write_scope() {
        let response = start_dir_repo_action(
            Extension(AuthInfo::new(OBSERVER_SCOPES.to_vec())),
            State(test_state()),
            Json(DirRepoActionRequest {
                path: "/tmp".to_string(),
                kind: RepoActionKind::Commit,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json = response_json(response).await;
        assert_eq!(json["code"], "NOT_AUTHORIZED");
    }

    #[tokio::test]
    async fn start_dir_repo_action_returns_service_error_response() {
        let response = start_dir_repo_action(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Json(DirRepoActionRequest {
                path: "/tmp".to_string(),
                kind: RepoActionKind::Open,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "CLIENT_ONLY");
        assert_eq!(json["message"], "open actions are handled client-side");
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
                target: None,
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

    struct PathGuard {
        path: Option<OsString>,
        fake_git_log: Option<OsString>,
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            match self.path.take() {
                Some(value) => std::env::set_var("PATH", value),
                None => std::env::remove_var("PATH"),
            }
            match self.fake_git_log.take() {
                Some(value) => std::env::set_var("SWIMMERS_FAKE_GIT_LOG", value),
                None => std::env::remove_var("SWIMMERS_FAKE_GIT_LOG"),
            }
        }
    }

    fn install_fake_slow_git(sleep_ms: u64) -> (tempfile::TempDir, PathGuard) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("fake git tempdir");
        let bin_dir = dir.path().join("bin");
        let log_path = dir.path().join("git-invocations.log");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        // Sleep per git invocation, then return the `-C PATH` value for
        // `rev-parse --show-toplevel` and empty output for `status --short`
        // (clean repo). Any other subcommand exits silently.
        let sleep_seconds = format!("{:.3}", sleep_ms as f64 / 1000.0);
        let script = format!(
            r#"#!/bin/sh
set -eu
if [ -n "${{SWIMMERS_FAKE_GIT_LOG:-}}" ]; then
  printf '%s\n' "$*" >> "$SWIMMERS_FAKE_GIT_LOG"
fi
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
        let original_fake_git_log = std::env::var_os("SWIMMERS_FAKE_GIT_LOG");
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = original_path.as_ref() {
            entries.extend(std::env::split_paths(existing).map(|p| p.into_os_string()));
        }
        std::env::set_var(
            "PATH",
            std::env::join_paths(entries).expect("join fake git path"),
        );
        std::env::set_var("SWIMMERS_FAKE_GIT_LOG", log_path);

        (
            dir,
            PathGuard {
                path: original_path,
                fake_git_log: original_fake_git_log,
            },
        )
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

        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            let response = list_dirs(
                Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
                State(test_state()),
                Query(DirQuery {
                    path: None,
                    managed_only: Some(false),
                    group: None,
                    target: None,
                }),
            )
            .await
            .into_response();
            let elapsed = started.elapsed();
            samples.push(elapsed);

            assert_eq!(response.status(), StatusCode::OK);
            let json = response_json(response).await;
            let entries = json["entries"].as_array().expect("entries array");
            assert_eq!(entries.len(), 24, "all repo entries should be present");
        }
        let p95 = p95_duration(samples);
        eprintln!("/v1/dirs p95: {p95:?} (budget 2s)");
        assert!(
            p95 < Duration::from_secs(2),
            "list_dirs must parallelize git probes; serial p95 would be ~4.8s, got {p95:?}"
        );
    }

    /// Direct repo-root entries should skip the `rev-parse` probe and only ask
    /// git for dirty status. The fake git log proves the command shape directly
    /// so this regression guard does not depend on a tight scheduler margin.
    #[tokio::test]
    async fn list_dirs_skips_rev_parse_for_direct_git_roots() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());

        let (fake_git_dir, _path_guard) = install_fake_slow_git(200);

        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path().join("repos");
        std::fs::create_dir_all(&base).expect("repos base");
        for i in 0..24 {
            std::fs::create_dir_all(base.join(format!("repo-{i:02}")).join(".git"))
                .expect("direct git repo subdir");
        }
        let _base_env = set_env_var("DIRS_BASE_PATH", base.as_os_str().to_os_string());

        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = std::time::Instant::now();
            let response = list_dirs(
                Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
                State(test_state()),
                Query(DirQuery {
                    path: None,
                    managed_only: Some(false),
                    group: None,
                    target: None,
                }),
            )
            .await
            .into_response();
            let elapsed = started.elapsed();
            samples.push(elapsed);

            assert_eq!(response.status(), StatusCode::OK);
            let json = response_json(response).await;
            let entries = json["entries"].as_array().expect("entries array");
            assert_eq!(entries.len(), 24, "all repo entries should be present");
        }

        let invocations =
            fs::read_to_string(fake_git_dir.path().join("git-invocations.log")).unwrap_or_default();
        let rev_parse_count = invocations
            .lines()
            .filter(|line| line.contains(" rev-parse "))
            .count();
        let status_count = invocations
            .lines()
            .filter(|line| line.contains(" status "))
            .count();

        assert_eq!(
            rev_parse_count, 0,
            "direct repo roots should not call rev-parse; invocations:\n{invocations}"
        );
        assert_eq!(
            status_count, 24,
            "direct repo roots should only run one status probe per entry; invocations:\n{invocations}"
        );

        let p95 = p95_duration(samples);
        eprintln!("/v1/dirs direct repo p95: {p95:?} (budget 1s)");
        assert!(
            p95 < Duration::from_secs(1),
            "direct repo roots should stay comfortably parallelized, got {p95:?}"
        );
    }
}
