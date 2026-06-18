use super::*;
use crate::config::Config;
use crate::session::supervisor::SessionSupervisor;
use crate::thought::health::BridgeHealthState;
use crate::thought::protocol::SyncRequestSequence;
use crate::thought::runtime_config::ThoughtConfig;
use crate::types::{
    DirGroupMembershipUpdateRequest, DirRepoSearchResponse, LaunchPathMapping, LaunchTargetSummary,
};
use std::collections::BTreeMap;
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
        published_selection: Arc::new(RwLock::new(crate::api::PublishedSelectionState::default())),
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

fn service_mapped_launch_target(
    id: &str,
    local_prefix: &Path,
    remote_prefix: &str,
) -> LaunchTargetSummary {
    LaunchTargetSummary {
        id: id.to_string(),
        label: id.to_string(),
        kind: "swimmers_api".to_string(),
        base_url: Some("http://127.0.0.1:3210".to_string()),
        auth_token_env: None,
        path_mappings: vec![LaunchPathMapping {
            local_prefix: local_prefix.to_string_lossy().into_owned(),
            remote_prefix: remote_prefix.to_string(),
        }],
    }
}

#[test]
fn default_launch_target_for_uses_mapping_only_without_explicit_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path().join("repos");
    let swimmers = base.join("opensource").join("swimmers");
    let config = OverlayDirConfig {
        label: "managed".into(),
        base_path: base.clone(),
        services: Vec::new(),
        groups: Vec::new(),
        launch: crate::session::overlay::OverlayLaunchConfig {
            default_target: "local".to_string(),
            default_target_explicit: false,
            targets: vec![
                LaunchTargetSummary::local(),
                service_mapped_launch_target("broad", &base, "/srv/repos"),
                service_mapped_launch_target("devbox", &swimmers, "/srv/swimmers"),
            ],
            group_defaults: BTreeMap::new(),
        },
    };

    assert_eq!(
        default_launch_target_for(Some(&config), None, swimmers.join("src").as_path()).as_deref(),
        Some("devbox")
    );

    let mut explicit = config;
    explicit.launch.default_target_explicit = true;
    assert_eq!(
        default_launch_target_for(Some(&explicit), None, swimmers.as_path()).as_deref(),
        Some("local")
    );
}

#[tokio::test]
async fn list_managed_service_entries_dedupes_dirs_skips_missing_and_keeps_metadata() {
    let _lock = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    crate::host_actions::clear_inspect_git_repo_cache_for_tests();

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

    let entries = scan_repo_search_roots_sync(&[root.clone(), root], REPO_SEARCH_DEFAULT_MAX_DEPTH);
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

    let commands = restart_commands_for_matched_services(&services, &matched).expect("commands");

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
    let cached = list_repo_search_entries_inner(vec![root.clone()], REPO_SEARCH_DEFAULT_MAX_DEPTH)
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

    let err =
        normalize_group_update_names(&["skills".to_string()], &valid).expect_err("unknown group");
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

fn assert_api_service_error(err: ApiServiceError, status: StatusCode, code: &str, message: &str) {
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

    let preflight = update_dir_group_memberships_preflight(Some(store), base, |canonical_base| {
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
            target: None,
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
            target: None,
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
            target: None,
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
            target: None,
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
            target: None,
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
