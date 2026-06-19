use super::*;

#[cfg(test)]
fn set_mtime(path: &Path, when: SystemTime) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for mtime");
    file.set_modified(when).expect("set_modified");
}

fn create_git_repo(path: &Path) {
    std::fs::create_dir_all(path.join(".git")).expect("git repo");
}

fn test_launch_target(id: &str, label: &str, kind: &str) -> LaunchTargetSummary {
    LaunchTargetSummary {
        id: id.to_string(),
        label: label.to_string(),
        kind: kind.to_string(),
        base_url: None,
        auth_token_env: None,
        bootstrap_hint: None,
        path_mappings: Vec::new(),
    }
}

fn mapped_launch_target(id: &str, local_prefix: &str, remote_prefix: &str) -> LaunchTargetSummary {
    LaunchTargetSummary {
        id: id.to_string(),
        label: id.to_string(),
        kind: "swimmers_api".to_string(),
        base_url: Some("http://127.0.0.1:3210".to_string()),
        auth_token_env: None,
        bootstrap_hint: None,
        path_mappings: vec![crate::types::LaunchPathMapping {
            local_prefix: local_prefix.to_string(),
            remote_prefix: remote_prefix.to_string(),
        }],
    }
}

#[test]
fn implicit_launch_default_uses_longest_matching_path_mapping() {
    let config = OverlayLaunchConfig {
        default_target: "local".to_string(),
        default_target_explicit: false,
        targets: vec![
            LaunchTargetSummary::local(),
            mapped_launch_target("broad", "/tmp/repos", "/srv/repos"),
            mapped_launch_target("specific", "/tmp/repos/swimmers", "/srv/swimmers"),
        ],
        group_defaults: BTreeMap::new(),
    };

    assert_eq!(
        config.default_for_group_or_path(None, Path::new("/tmp/repos/swimmers/src")),
        "specific"
    );
}

#[test]
fn implicit_launch_default_keeps_first_equal_specificity_mapping() {
    let config = OverlayLaunchConfig {
        default_target: "local".to_string(),
        default_target_explicit: false,
        targets: vec![
            LaunchTargetSummary::local(),
            mapped_launch_target("primary", "/tmp/repos", "/srv/primary"),
            mapped_launch_target("duplicate", "/tmp/./repos", "/srv/duplicate"),
        ],
        group_defaults: BTreeMap::new(),
    };

    assert_eq!(
        config.default_for_group_or_path(None, Path::new("/tmp/repos/swimmers")),
        "primary"
    );
}

#[test]
fn explicit_launch_default_wins_over_path_mapping() {
    let config = OverlayLaunchConfig {
        default_target: "local".to_string(),
        default_target_explicit: true,
        targets: vec![
            LaunchTargetSummary::local(),
            mapped_launch_target("devbox", "/tmp/repos/swimmers", "/srv/swimmers"),
        ],
        group_defaults: BTreeMap::new(),
    };

    assert_eq!(
        config.default_for_group_or_path(None, Path::new("/tmp/repos/swimmers")),
        "local"
    );
}

#[test]
fn implicit_launch_default_ignores_empty_path_mapping_prefixes() {
    let mut empty_local = mapped_launch_target("empty-local", "", "/srv/all");
    let mut empty_remote = mapped_launch_target("empty-remote", "/tmp/repos", "");
    empty_local.path_mappings[0].local_prefix.clear();
    empty_remote.path_mappings[0].remote_prefix.clear();
    let config = OverlayLaunchConfig {
        default_target: "local".to_string(),
        default_target_explicit: false,
        targets: vec![LaunchTargetSummary::local(), empty_local, empty_remote],
        group_defaults: BTreeMap::new(),
    };

    assert_eq!(
        config.default_for_group_or_path(None, Path::new("/tmp/repos/swimmers")),
        "local"
    );
}

#[test]
fn group_launch_default_wins_over_implicit_path_mapping() {
    let mut group_defaults = BTreeMap::new();
    group_defaults.insert("backend".to_string(), "backend-box".to_string());
    let config = OverlayLaunchConfig {
        default_target: "local".to_string(),
        default_target_explicit: false,
        targets: vec![
            LaunchTargetSummary::local(),
            mapped_launch_target("devbox", "/tmp/repos/swimmers", "/srv/swimmers"),
            mapped_launch_target("backend-box", "/tmp/repos", "/srv/backend"),
        ],
        group_defaults,
    };

    assert_eq!(
        config.default_for_group_or_path(Some("backend"), Path::new("/tmp/repos/swimmers")),
        "backend-box"
    );
}

fn test_launch_client(label: &str, targets: Vec<LaunchTargetSummary>) -> ClientOverlay {
    ClientOverlay {
        label: label.to_string(),
        cwd_patterns: Vec::new(),
        cwd_match_count: 0,
        plan_root: None,
        plan_draft: None,
        dir_config: Some(OverlayDirConfig {
            label: label.to_string(),
            base_path: PathBuf::from("/tmp"),
            services: Vec::new(),
            groups: Vec::new(),
            launch: OverlayLaunchConfig {
                default_target: "local".to_string(),
                default_target_explicit: true,
                targets,
                group_defaults: BTreeMap::new(),
            },
        }),
    }
}

fn test_dir_client(
    label: &str,
    base_path: PathBuf,
    cwd_patterns: Vec<String>,
    has_dir_config: bool,
) -> ClientOverlay {
    ClientOverlay {
        label: label.to_string(),
        cwd_patterns,
        cwd_match_count: 0,
        plan_root: None,
        plan_draft: None,
        dir_config: has_dir_config.then(|| OverlayDirConfig {
            label: label.to_string(),
            base_path,
            services: Vec::new(),
            groups: Vec::new(),
            launch: OverlayLaunchConfig::local_only(),
        }),
    }
}

#[test]
fn cwd_starts_with_exact_match() {
    assert!(cwd_starts_with("/tmp/repos/example", "/tmp/repos/example"));
}

#[test]
fn cwd_starts_with_child_dir() {
    assert!(cwd_starts_with(
        "/tmp/repos/example/src/data",
        "/tmp/repos/example"
    ));
}

#[test]
fn cwd_starts_with_rejects_partial_name() {
    assert!(!cwd_starts_with(
        "/tmp/repos/example_server",
        "/tmp/repos/example"
    ));
}

#[test]
fn expand_tilde() {
    let expanded = expand_path("~/repos/foo");
    assert!(!expanded.starts_with('~'));
    assert!(expanded.ends_with("/repos/foo"));
}

#[test]
fn expand_path_terminates_when_env_var_resolves_to_self_referential_text() {
    // Regression: the previous implementation re-scanned from offset 0
    // after each substitution, so an env var that expanded to text
    // containing the same `${VAR}` reference would loop forever.
    let key = "SWIMMERS_EXPAND_PATH_RECURSIVE_TEST";
    let prior = std::env::var(key).ok();
    std::env::set_var(key, format!("${{{key}}}/x"));

    let expanded = expand_path(&format!("${{{key}}}/y"));

    match prior {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }

    // The first expansion is the only one performed; the inserted
    // `${VAR}` is treated as literal text, not re-resolved.
    assert_eq!(expanded, format!("${{{key}}}/x/y"));
}

#[test]
fn expand_repo_path_falls_back_to_base_for_unset_monoserver_root() {
    let key = "SKILLBOX_MONOSERVER_ROOT";
    let prior = std::env::var(key).ok();
    std::env::remove_var(key);

    let base = PathBuf::from("/tmp/repos");
    let expanded = expand_repo_path("${SKILLBOX_MONOSERVER_ROOT}/voice-to-text", &base);

    match prior {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }

    assert_eq!(expanded, base.join("voice-to-text"));
}

#[test]
fn expand_group_dir_literal_passthrough() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let literal = tmp.path().join("alpha");
    std::fs::create_dir_all(&literal).expect("alpha");
    let results = expand_group_dir(literal.to_str().unwrap());
    assert_eq!(results, vec![literal]);
}

#[test]
fn expand_group_dir_literal_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("does-not-exist");
    let results = expand_group_dir(missing.to_str().unwrap());
    assert!(results.is_empty());
}

#[test]
fn expand_group_dir_single_star_with_suffix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("repo-a").join(".claude").join("skills"))
        .expect("repo-a skills");
    std::fs::create_dir_all(tmp.path().join("repo-b").join(".claude").join("skills"))
        .expect("repo-b skills");
    // A sibling without the suffix should be ignored.
    std::fs::create_dir_all(tmp.path().join("repo-c")).expect("repo-c");
    // A file (not a dir) at the wildcard level should be ignored.
    std::fs::write(tmp.path().join("not-a-dir"), "x").expect("file");

    let pattern = format!("{}/*/.claude/skills", tmp.path().display());
    let results = expand_group_dir(&pattern);

    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|p| p.ends_with("repo-a/.claude/skills")));
    assert!(results.iter().any(|p| p.ends_with("repo-b/.claude/skills")));
}

#[test]
fn expand_group_dir_single_star_projects_skills() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("alpha").join("skills")).expect("alpha skills");
    std::fs::create_dir_all(tmp.path().join("beta").join("skills")).expect("beta skills");
    std::fs::create_dir_all(tmp.path().join("gamma")).expect("gamma no-skills");

    let pattern = format!("{}/*/skills", tmp.path().display());
    let results = expand_group_dir(&pattern);

    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|p| p.ends_with("alpha/skills")));
    assert!(results.iter().any(|p| p.ends_with("beta/skills")));
}

#[test]
fn load_client_overlays_returns_none_when_clients_dir_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(load_client_overlays(tmp.path()).is_none());
}

#[test]
fn load_client_overlays_returns_empty_when_clients_dir_has_no_overlay_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let clients_dir = tmp.path().join("clients");
    std::fs::create_dir_all(clients_dir.join("empty-client")).expect("client dir");
    std::fs::write(clients_dir.join("not-a-client"), "x").expect("file");

    let clients = load_client_overlays(tmp.path()).expect("scan clients");

    assert!(clients.is_empty());
}

#[test]
fn client_overlay_paths_are_sorted_by_client_dir() {
    let root = PathBuf::from("/tmp/swimmers-overlay-sort");
    let mut paths = vec![
        (root.join("zeta"), root.join("zeta").join("overlay.yaml")),
        (root.join("alpha"), root.join("alpha").join("overlay.yaml")),
        (
            root.join("middle"),
            root.join("middle").join("overlay.yaml"),
        ),
    ];

    sort_client_overlay_paths(&mut paths);

    assert_eq!(
        paths
            .iter()
            .map(|(client_dir, _)| {
                client_dir
                    .file_name()
                    .expect("client dir name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>(),
        vec!["alpha", "middle", "zeta"]
    );
}

#[test]
fn parse_agent_launch_injects_local_and_filters_unknown_defaults() {
    let mut group_defaults = BTreeMap::new();
    group_defaults.insert("known".to_string(), "remote".to_string());
    group_defaults.insert("unknown".to_string(), "missing".to_string());

    let launch = parse_agent_launch(Some(DevSanityAgentLaunch {
        default_target: Some("missing".to_string()),
        targets: vec![DevSanityLaunchTarget {
            id: Some("remote".to_string()),
            label: None,
            kind: Some("swimmers_api".to_string()),
            base_url: Some("http://remote.test:3210".to_string()),
            auth_token_env: Some("REMOTE_TOKEN".to_string()),
            bootstrap_hint: None,
            path_mappings: vec![DevSanityLaunchPathMapping {
                local_prefix: Some("/local".to_string()),
                remote_prefix: Some("/remote".to_string()),
            }],
        }],
        group_defaults,
    }));

    assert_eq!(launch.default_target, "local");
    assert_eq!(
        launch
            .targets
            .iter()
            .map(|target| target.id.as_str())
            .collect::<Vec<_>>(),
        vec!["local", "remote"]
    );
    assert_eq!(launch.default_for_group(Some("known")), "remote");
    assert_eq!(launch.default_for_group(Some("unknown")), "local");

    let remote = launch
        .targets
        .iter()
        .find(|target| target.id == "remote")
        .expect("remote target");
    assert_eq!(remote.label, "remote");
    assert_eq!(remote.kind, "swimmers_api");
    assert_eq!(remote.path_mappings[0].local_prefix, "/local");
    assert_eq!(remote.path_mappings[0].remote_prefix, "/remote");
}

#[test]
fn parse_agent_launch_trims_target_identity_and_filters_blank_ids() {
    let mut group_defaults = BTreeMap::new();
    group_defaults.insert("known".to_string(), " remote ".to_string());
    group_defaults.insert("blank".to_string(), "   ".to_string());

    let launch = parse_agent_launch(Some(DevSanityAgentLaunch {
        default_target: Some(" remote ".to_string()),
        targets: vec![
            DevSanityLaunchTarget {
                id: Some(" remote ".to_string()),
                label: Some(" Remote Box ".to_string()),
                kind: Some(" swimmers_api ".to_string()),
                base_url: Some("http://remote.test:3210".to_string()),
                auth_token_env: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            DevSanityLaunchTarget {
                id: Some("   ".to_string()),
                label: Some("Blank".to_string()),
                kind: Some("swimmers_api".to_string()),
                base_url: Some("http://blank.test:3210".to_string()),
                auth_token_env: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
        ],
        group_defaults,
    }));

    assert_eq!(launch.default_target, "remote");
    assert_eq!(launch.default_for_group(Some("known")), "remote");
    assert!(!launch.group_defaults.contains_key("blank"));
    assert_eq!(
        launch
            .targets
            .iter()
            .map(|target| (
                target.id.as_str(),
                target.label.as_str(),
                target.kind.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("local", "Local machine", "local"),
            ("remote", "Remote Box", "swimmers_api")
        ]
    );
}

#[test]
fn parse_agent_launch_ignores_blank_or_empty_expanded_path_mappings() {
    let key = "SWIMMERS_OVERLAY_EMPTY_MAPPING_TEST";
    let prior = std::env::var(key).ok();
    std::env::remove_var(key);

    let launch = parse_agent_launch(Some(DevSanityAgentLaunch {
        default_target: Some("remote".to_string()),
        targets: vec![DevSanityLaunchTarget {
            id: Some("remote".to_string()),
            label: None,
            kind: Some("swimmers_api".to_string()),
            base_url: Some("http://remote.test:3210".to_string()),
            auth_token_env: None,
            bootstrap_hint: None,
            path_mappings: vec![
                DevSanityLaunchPathMapping {
                    local_prefix: Some("".to_string()),
                    remote_prefix: Some("/remote".to_string()),
                },
                DevSanityLaunchPathMapping {
                    local_prefix: Some(format!("${{{key}}}")),
                    remote_prefix: Some("/remote-env".to_string()),
                },
                DevSanityLaunchPathMapping {
                    local_prefix: Some("/local".to_string()),
                    remote_prefix: Some("   ".to_string()),
                },
                DevSanityLaunchPathMapping {
                    local_prefix: Some("/local".to_string()),
                    remote_prefix: Some("/remote".to_string()),
                },
            ],
        }],
        group_defaults: BTreeMap::new(),
    }));

    match prior {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }

    let remote = launch
        .targets
        .iter()
        .find(|target| target.id == "remote")
        .expect("remote target");

    assert_eq!(remote.path_mappings.len(), 1);
    assert_eq!(remote.path_mappings[0].local_prefix, "/local");
    assert_eq!(remote.path_mappings[0].remote_prefix, "/remote");
}

#[test]
fn all_launch_targets_preserves_first_client_order_and_first_duplicate() {
    let first_shared = test_launch_target("shared", "First shared", "swimmers_api");
    let second_shared = test_launch_target("shared", "Second shared", "swimmers_api");
    let overlay = SkillboxOverlay {
        clients: vec![
            test_launch_client(
                "one",
                vec![
                    LaunchTargetSummary::local(),
                    test_launch_target("remote-a", "Remote A", "swimmers_api"),
                    first_shared.clone(),
                ],
            ),
            ClientOverlay {
                dir_config: None,
                ..test_launch_client("no-config", Vec::new())
            },
            test_launch_client(
                "two",
                vec![
                    LaunchTargetSummary::local(),
                    second_shared,
                    test_launch_target("remote-b", "Remote B", "swimmers_api"),
                ],
            ),
        ],
        loaded_at: Utc::now(),
    };

    let targets = overlay.all_launch_targets();

    assert_eq!(
        targets
            .iter()
            .map(|target| target.id.as_str())
            .collect::<Vec<_>>(),
        vec!["local", "remote-a", "shared", "remote-b"]
    );
    assert_eq!(
        targets
            .iter()
            .find(|target| target.id == "shared")
            .expect("shared target")
            .label,
        first_shared.label
    );
}

#[test]
fn all_launch_targets_returns_empty_when_no_clients_have_dir_config() {
    let overlay = SkillboxOverlay {
        clients: vec![ClientOverlay {
            label: "no-config".to_string(),
            cwd_patterns: Vec::new(),
            cwd_match_count: 0,
            plan_root: None,
            plan_draft: None,
            dir_config: None,
        }],
        loaded_at: Utc::now(),
    };

    assert!(overlay.all_launch_targets().is_empty());
}

#[test]
fn find_dir_config_prefers_base_path_over_earlier_cwd_match() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let owned_base = tmp.path().join("owned");
    let other_base = tmp.path().join("other");
    let cwd = owned_base.join("repo").join("src");
    std::fs::create_dir_all(&cwd).expect("cwd");
    std::fs::create_dir_all(&other_base).expect("other base");

    let overlay = SkillboxOverlay {
        clients: vec![
            test_dir_client(
                "pattern-first",
                other_base,
                vec![owned_base.to_string_lossy().into_owned()],
                true,
            ),
            test_dir_client("base-owner", owned_base, Vec::new(), true),
        ],
        loaded_at: Utc::now(),
    };

    let config = overlay
        .find_dir_config(&cwd.to_string_lossy())
        .expect("dir config");

    assert_eq!(config.label, "base-owner");
}

#[test]
fn find_dir_config_falls_back_to_cwd_match() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let service_base = tmp.path().join("services");
    let repo_base = tmp.path().join("repo");
    let cwd = repo_base.join("nested");
    std::fs::create_dir_all(&service_base).expect("service base");
    std::fs::create_dir_all(&cwd).expect("cwd");

    let overlay = SkillboxOverlay {
        clients: vec![test_dir_client(
            "fallback",
            service_base,
            vec![repo_base.to_string_lossy().into_owned()],
            true,
        )],
        loaded_at: Utc::now(),
    };

    let config = overlay
        .find_dir_config(&cwd.to_string_lossy())
        .expect("dir config");

    assert_eq!(config.label, "fallback");
}

#[test]
fn find_dir_config_clients_without_dir_config_cannot_produce_fallback_config() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo_base = tmp.path().join("repo");
    let cwd = repo_base.join("nested");
    let later_service_base = tmp.path().join("later-services");
    std::fs::create_dir_all(&cwd).expect("cwd");
    std::fs::create_dir_all(&later_service_base).expect("later service base");

    let overlay = SkillboxOverlay {
        clients: vec![
            test_dir_client(
                "no-config",
                tmp.path().join("ignored"),
                vec![repo_base.to_string_lossy().into_owned()],
                false,
            ),
            test_dir_client(
                "later-config",
                later_service_base,
                vec![repo_base.to_string_lossy().into_owned()],
                true,
            ),
        ],
        loaded_at: Utc::now(),
    };

    assert!(overlay.find_dir_config(&cwd.to_string_lossy()).is_none());
}

#[test]
fn append_scan_root_services_appends_sorted_git_repos_after_existing_entries() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("base");
    let external = tmp.path().join("external");
    std::fs::create_dir_all(&base).expect("base");
    std::fs::create_dir_all(external.join("zeta").join(".git")).expect("zeta repo");
    std::fs::create_dir_all(external.join("alpha").join(".git")).expect("alpha repo");
    std::fs::create_dir_all(external.join("no-git")).expect("no-git dir");
    std::fs::create_dir_all(external.join(".hidden").join(".git")).expect("hidden repo");
    std::fs::write(external.join("not-a-dir"), "x").expect("file");

    let mut services = vec![OverlayServiceEntry {
        name: "manual".to_string(),
        dir: "manual".to_string(),
        health_url: Some("http://localhost:3000".to_string()),
        restart: Some("restart manual".to_string()),
        open_url: Some("http://localhost:3000".to_string()),
    }];
    let mut seen_dirs = services
        .iter()
        .map(|service| service.dir.clone())
        .collect::<BTreeSet<_>>();

    append_scan_root_services(
        &mut services,
        &mut seen_dirs,
        &[base.clone(), external.clone()],
        &base,
    );

    assert_eq!(
        services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>(),
        vec!["manual", "alpha", "zeta"]
    );
    assert_eq!(
        services[0].health_url.as_deref(),
        Some("http://localhost:3000")
    );
    assert_eq!(services[1].dir, external.join("alpha").to_string_lossy());
    assert_eq!(services[2].dir, external.join("zeta").to_string_lossy());
}

#[test]
fn scan_root_is_outside_base_uses_canonical_root_and_base() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("base");
    let nested = base.join("nested");
    let sibling = tmp.path().join("sibling");
    std::fs::create_dir_all(&nested).expect("nested");
    std::fs::create_dir_all(&sibling).expect("sibling");

    let same_base = canonical_scan_root_paths(&base.join("..").join("base"), &base);
    assert!(!scan_root_is_outside_base(&same_base.root, &same_base.base));

    let nested_root = canonical_scan_root_paths(&nested, &base);
    assert!(!scan_root_is_outside_base(
        &nested_root.root,
        &nested_root.base
    ));

    let sibling_root = canonical_scan_root_paths(&sibling, &base);
    assert!(scan_root_is_outside_base(
        &sibling_root.root,
        &sibling_root.base
    ));
}

#[test]
fn service_entries_from_scan_root_excludes_roots_equal_to_or_inside_base() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("base");
    let nested = base.join("nested");
    create_git_repo(&base.join("alpha"));
    create_git_repo(&nested.join("beta"));

    assert!(service_entries_from_scan_root(&base, &base).is_empty());
    assert!(service_entries_from_scan_root(&nested, &base).is_empty());
}

#[test]
fn service_entries_from_scan_root_returns_empty_when_root_cannot_be_read() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("base");
    let missing_root = tmp.path().join("missing-scan-root");
    std::fs::create_dir_all(&base).expect("base");

    assert!(service_entries_from_scan_root(&missing_root, &base).is_empty());
}

#[test]
fn repo_dirs_in_scan_root_keeps_only_visible_git_repo_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    create_git_repo(&root.join("alpha"));
    create_git_repo(&root.join(".hidden"));
    std::fs::create_dir_all(root.join("no-git")).expect("no-git");
    std::fs::write(root.join("not-a-dir"), "x").expect("file");

    let names = repo_dirs_in_scan_root(&root)
        .into_iter()
        .map(|path| {
            path.file_name()
                .expect("repo dir name")
                .to_string_lossy()
                .into_owned()
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(names, BTreeSet::from(["alpha".to_string()]));
}

#[test]
fn collect_sorted_service_entries_sorts_by_service_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("base");
    let external = tmp.path().join("external");
    let zeta = external.join("zeta");
    let alpha = external.join("alpha");
    std::fs::create_dir_all(&base).expect("base");
    std::fs::create_dir_all(&zeta).expect("zeta");
    std::fs::create_dir_all(&alpha).expect("alpha");

    let services = collect_sorted_service_entries(vec![zeta, alpha], &base);

    assert_eq!(
        services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    assert_eq!(services[0].dir, external.join("alpha").to_string_lossy());
    assert_eq!(services[1].dir, external.join("zeta").to_string_lossy());
}

#[test]
fn append_scan_root_services_skips_dirs_already_seen_by_absolute_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().join("base");
    let external = tmp.path().join("external");
    let alpha = external.join("alpha");
    let beta = external.join("beta");
    std::fs::create_dir_all(&base).expect("base");
    std::fs::create_dir_all(alpha.join(".git")).expect("alpha repo");
    std::fs::create_dir_all(beta.join(".git")).expect("beta repo");
    let alpha_dir = alpha
        .canonicalize()
        .unwrap_or_else(|_| alpha.clone())
        .to_string_lossy()
        .into_owned();

    let mut services = vec![OverlayServiceEntry {
        name: "manual-alpha".to_string(),
        dir: alpha_dir.clone(),
        health_url: None,
        restart: None,
        open_url: None,
    }];
    let mut seen_dirs = BTreeSet::from([alpha_dir]);

    append_scan_root_services(&mut services, &mut seen_dirs, &[external.clone()], &base);

    assert_eq!(
        services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>(),
        vec!["manual-alpha", "beta"]
    );
}

#[test]
fn parse_client_overlay_adds_client_repos_to_dir_config_services() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let client_dir = tmp.path().join("clients").join("personal");
    std::fs::create_dir_all(&client_dir).expect("client dir");
    let repo_base = tmp.path().join("repos");
    std::fs::create_dir_all(&repo_base).expect("repo base");
    std::fs::create_dir_all(repo_base.join("finalreceipts")).expect("finalreceipts repo");
    std::fs::create_dir_all(repo_base.join("sweet-potato")).expect("sweet-potato repo");
    let hard_root = tmp.path().join("hard");
    let hard_repo = hard_root.join("mmd-pcb");
    std::fs::create_dir_all(&hard_repo).expect("hard repo");
    let scanned_hard_repo = hard_root.join("pcbcd");
    std::fs::create_dir_all(scanned_hard_repo.join(".git")).expect("scanned hard repo");
    let overlay_path = client_dir.join("overlay.yaml");
    std::fs::write(
        &overlay_path,
        format!(
            r#"
version: 1
client:
  id: personal
  context:
    cwd_match:
      - {repo_base}
    repo_landscape:
      scan_roots:
        - {hard_root}
  repos:
    - id: finalreceipts
      kind: repo
      repo_path: {repo_base}/finalreceipts
    - id: sweet-potato-dupe
      kind: repo
      repo_path: {repo_base}/sweet-potato
    - id: mmd-pcb
      kind: repo
      repo_path: {hard_repo}
dev_sanity:
  services:
    base_path: {repo_base}
    entries:
      - name: spaps
        dir: sweet-potato
        health_url: http://localhost:3301
  groups:
    - name: frontend
      paths:
        - {repo_base}/finalreceipts
"#,
            hard_repo = hard_repo.display(),
            hard_root = hard_root.display(),
            repo_base = repo_base.display()
        ),
    )
    .expect("write overlay");

    let client = parse_client_overlay(&client_dir, &overlay_path).expect("parse overlay");
    let config = client.dir_config.expect("dir config");
    let service_dirs: Vec<&str> = config
        .services
        .iter()
        .map(|service| service.dir.as_str())
        .collect();

    assert_eq!(
        service_dirs,
        vec![
            "sweet-potato",
            "finalreceipts",
            hard_repo.to_str().expect("hard path"),
            scanned_hard_repo.to_str().expect("scanned hard path")
        ]
    );
    assert!(config
        .services
        .iter()
        .any(|service| service.name == "finalreceipts"));
    assert_eq!(config.groups[0].name, "frontend");
    assert!(config.groups[0]
        .paths
        .iter()
        .any(|path| path.ends_with("finalreceipts")));
}

#[test]
fn list_all_plans_sorts_by_mtime_desc() {
    use std::time::Duration;
    let tmp = tempfile::tempdir().expect("tempdir");
    let client_dir = tmp.path().join("clients").join("personal");
    let released = client_dir.join("plans").join("released");
    let draft = client_dir.join("plans").join("draft");
    std::fs::create_dir_all(released.join("older_plan")).unwrap();
    std::fs::create_dir_all(released.join("newest_plan")).unwrap();
    std::fs::create_dir_all(draft.join("draft_plan")).unwrap();
    let older_schema = released.join("older_plan").join("schema.mmd");
    let newest_schema = released.join("newest_plan").join("schema.mmd");
    let draft_schema = draft.join("draft_plan").join("schema.mmd");
    std::fs::write(&older_schema, "older").unwrap();
    std::fs::write(&newest_schema, "newest").unwrap();
    std::fs::write(&draft_schema, "draft").unwrap();
    // Stamp mtimes so the sort order is deterministic without relying on
    // fs precision or write-order side-effects.
    let now = SystemTime::now();
    let earlier = now - Duration::from_secs(3600);
    let oldest = earlier - Duration::from_secs(3600);
    set_mtime(&older_schema, oldest);
    set_mtime(&newest_schema, now);
    set_mtime(&draft_schema, earlier);

    let client = ClientOverlay {
        label: "personal".to_string(),
        cwd_patterns: Vec::new(),
        cwd_match_count: 0,
        plan_root: Some(released),
        plan_draft: Some(draft),
        dir_config: None,
    };
    let overlay = SkillboxOverlay {
        clients: vec![client],
        loaded_at: Utc::now(),
    };
    let plans = overlay.list_all_plans();
    assert_eq!(
        plans.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(),
        vec!["newest_plan", "draft_plan", "older_plan"]
    );
    assert_eq!(plans[0].kind, "released");
    assert_eq!(plans[1].kind, "draft");
    assert!(plans.iter().all(|p| p.client_label == "personal"));
}

#[test]
fn list_all_plans_breaks_equal_mtime_ties_deterministically() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let alpha_root = tmp.path().join("alpha").join("plans").join("released");
    let beta_root = tmp.path().join("beta").join("plans").join("released");
    std::fs::create_dir_all(alpha_root.join("zeta_plan")).unwrap();
    std::fs::create_dir_all(beta_root.join("alpha_plan")).unwrap();
    let alpha_schema = alpha_root.join("zeta_plan").join("schema.mmd");
    let beta_schema = beta_root.join("alpha_plan").join("schema.mmd");
    std::fs::write(&alpha_schema, "alpha").unwrap();
    std::fs::write(&beta_schema, "beta").unwrap();
    let same_time = SystemTime::now();
    set_mtime(&alpha_schema, same_time);
    set_mtime(&beta_schema, same_time);

    let overlay = SkillboxOverlay {
        clients: vec![
            ClientOverlay {
                label: "beta".to_string(),
                cwd_patterns: Vec::new(),
                cwd_match_count: 0,
                plan_root: Some(beta_root),
                plan_draft: None,
                dir_config: None,
            },
            ClientOverlay {
                label: "alpha".to_string(),
                cwd_patterns: Vec::new(),
                cwd_match_count: 0,
                plan_root: Some(alpha_root),
                plan_draft: None,
                dir_config: None,
            },
        ],
        loaded_at: Utc::now(),
    };

    let plans = overlay.list_all_plans();

    assert_eq!(
        plans
            .iter()
            .map(|plan| (plan.client_label.as_str(), plan.slug.as_str()))
            .collect::<Vec<_>>(),
        vec![("alpha", "zeta_plan"), ("beta", "alpha_plan")]
    );
}

#[test]
fn list_all_plans_skips_archived_and_missing_schema() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let client_dir = tmp.path().join("clients").join("personal");
    let released = client_dir.join("plans").join("released");
    std::fs::create_dir_all(released.join("live_plan")).unwrap();
    std::fs::write(released.join("live_plan").join("schema.mmd"), "ok").unwrap();
    // No schema.mmd → skipped.
    std::fs::create_dir_all(released.join("no_schema")).unwrap();
    // "archived" in path → skipped even with schema.mmd.
    let archived = client_dir.join("plans").join("archived").join("stale_plan");
    std::fs::create_dir_all(&archived).unwrap();
    std::fs::write(archived.join("schema.mmd"), "stale").unwrap();

    let client = ClientOverlay {
        label: "personal".to_string(),
        cwd_patterns: Vec::new(),
        cwd_match_count: 0,
        plan_root: Some(released),
        plan_draft: Some(client_dir.join("plans").join("archived")),
        dir_config: None,
    };
    let overlay = SkillboxOverlay {
        clients: vec![client],
        loaded_at: Utc::now(),
    };
    let plans = overlay.list_all_plans();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].slug, "live_plan");
}

#[test]
fn overlay_health_reports_load_age_and_remote_target_count_without_probe() {
    let remote = LaunchTargetSummary {
        id: "remote-skillbox".to_string(),
        label: "Remote".to_string(),
        kind: " swimmers_api ".to_string(),
        base_url: Some("http://example.test:3210".to_string()),
        auth_token_env: Some("REMOTE_TOKEN".to_string()),
        bootstrap_hint: None,
        path_mappings: Vec::new(),
    };
    let client = ClientOverlay {
        label: "health".to_string(),
        cwd_patterns: Vec::new(),
        cwd_match_count: 0,
        plan_root: None,
        plan_draft: None,
        dir_config: Some(OverlayDirConfig {
            label: "health".to_string(),
            base_path: PathBuf::from("/tmp"),
            services: Vec::new(),
            groups: Vec::new(),
            launch: OverlayLaunchConfig {
                default_target: "local".to_string(),
                default_target_explicit: true,
                targets: vec![LaunchTargetSummary::local(), remote],
                group_defaults: BTreeMap::new(),
            },
        }),
    };
    let overlay = SkillboxOverlay {
        clients: vec![client],
        loaded_at: Utc::now() - chrono::Duration::seconds(1),
    };

    let health = overlay.health_snapshot();
    assert_eq!(health.status, crate::types::DependencyHealthStatus::Healthy);
    assert_eq!(health.details["client_count"], "1");
    assert!(health.freshness_ms.is_some());

    let remote = overlay.remote_targets_health_snapshot();
    assert_eq!(remote.status, crate::types::DependencyHealthStatus::Unknown);
    assert_eq!(remote.details["configured_targets"], "1");
    assert_eq!(remote.details["probe"], "not_run_by_health");
    assert!(
        !remote
            .details
            .values()
            .any(|value| value.contains("REMOTE_TOKEN")),
        "health details must not leak token env names or values"
    );
}

#[test]
fn expand_group_dir_rejects_partial_component_wildcard() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("alpha-one")).expect("alpha-one");
    let pattern = format!("{}/alpha-*", tmp.path().display());
    let results = expand_group_dir(&pattern);
    assert!(
        results.is_empty(),
        "partial-component wildcards are not supported: {:?}",
        results
    );
}

#[test]
fn expand_group_dir_rejects_multi_star() {
    let pattern = "/tmp/*/*/skills";
    let results = expand_group_dir(pattern);
    assert!(results.is_empty());
}

fn find_plan_dirs_overlay(client: ClientOverlay) -> SkillboxOverlay {
    find_plan_dirs_overlay_with_clients(vec![client])
}

fn find_plan_dirs_overlay_with_clients(clients: Vec<ClientOverlay>) -> SkillboxOverlay {
    SkillboxOverlay {
        clients,
        loaded_at: Utc::now(),
    }
}

fn make_plan_client(
    cwd_patterns: Vec<String>,
    cwd_match_count: usize,
    plan_root: Option<PathBuf>,
    plan_draft: Option<PathBuf>,
) -> ClientOverlay {
    ClientOverlay {
        label: "test".to_string(),
        cwd_patterns,
        cwd_match_count,
        plan_root,
        plan_draft,
        dir_config: None,
    }
}

#[test]
fn find_plan_dirs_returns_none_when_no_client_matches_cwd() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plan_root = tmp.path().join("plans").join("released");
    std::fs::create_dir_all(&plan_root).unwrap();
    let client = make_plan_client(
        vec!["/some/other/repo".to_string()],
        1,
        Some(plan_root),
        None,
    );
    let overlay = find_plan_dirs_overlay(client);
    assert!(overlay.find_plan_dirs("/unrelated/path").is_none());
}

#[test]
fn find_plan_dirs_skips_multi_repo_clients() {
    // Multi-repo clients (cwd_match_count > 1) span multiple repos so the
    // overlay can't pick a single plan dir set; caller falls back to the
    // in-repo scan.
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path().to_string_lossy().to_string();
    let plan_root = tmp.path().join("plans").join("released");
    std::fs::create_dir_all(&plan_root).unwrap();
    let client = make_plan_client(vec![cwd.clone()], 2, Some(plan_root), None);
    let overlay = find_plan_dirs_overlay(client);
    assert!(overlay.find_plan_dirs(&cwd).is_none());
}

#[test]
fn find_plan_dirs_rejects_first_multi_repo_match_without_falling_through() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path().join("repo");
    let cwd = cwd.to_string_lossy().to_string();
    let first_root = tmp.path().join("first").join("plans").join("released");
    let second_root = tmp.path().join("second").join("plans").join("released");
    std::fs::create_dir_all(&first_root).unwrap();
    std::fs::create_dir_all(&second_root).unwrap();
    let first = make_plan_client(vec![cwd.clone()], 2, Some(first_root), None);
    let second = make_plan_client(vec![cwd.clone()], 1, Some(second_root), None);
    let overlay = find_plan_dirs_overlay_with_clients(vec![first, second]);
    assert!(overlay.find_plan_dirs(&cwd).is_none());
}

#[test]
fn find_plan_dirs_returns_both_root_and_draft_when_present() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path().to_string_lossy().to_string();
    let plan_root = tmp.path().join("plans").join("released");
    let plan_draft = tmp.path().join("plans").join("draft");
    std::fs::create_dir_all(&plan_root).unwrap();
    std::fs::create_dir_all(&plan_draft).unwrap();
    let client = make_plan_client(
        vec![cwd.clone()],
        1,
        Some(plan_root.clone()),
        Some(plan_draft.clone()),
    );
    let overlay = find_plan_dirs_overlay(client);
    let dirs = overlay.find_plan_dirs(&cwd).expect("dirs");
    assert_eq!(dirs, vec![plan_root, plan_draft]);
}

#[test]
fn find_plan_dirs_skips_directories_that_do_not_exist_on_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path().to_string_lossy().to_string();
    let real_root = tmp.path().join("plans").join("released");
    std::fs::create_dir_all(&real_root).unwrap();
    // plan_draft points to a path that was never created.
    let missing_draft = tmp.path().join("plans").join("draft");
    let client = make_plan_client(
        vec![cwd.clone()],
        1,
        Some(real_root.clone()),
        Some(missing_draft),
    );
    let overlay = find_plan_dirs_overlay(client);
    let dirs = overlay.find_plan_dirs(&cwd).expect("dirs");
    assert_eq!(dirs, vec![real_root]);
}

#[test]
fn find_plan_dirs_returns_none_when_neither_dir_exists_on_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path().to_string_lossy().to_string();
    let missing_root = tmp.path().join("plans").join("released");
    let missing_draft = tmp.path().join("plans").join("draft");
    let client = make_plan_client(
        vec![cwd.clone()],
        1,
        Some(missing_root),
        Some(missing_draft),
    );
    let overlay = find_plan_dirs_overlay(client);
    assert!(overlay.find_plan_dirs(&cwd).is_none());
}

#[test]
fn find_plan_dirs_matches_cwd_inside_pattern_dir() {
    // cwd_starts_with allows nested directories under the pattern.
    let tmp = tempfile::tempdir().expect("tempdir");
    let pattern = tmp.path().to_string_lossy().to_string();
    let nested = tmp.path().join("nested").join("crate");
    std::fs::create_dir_all(&nested).unwrap();
    let plan_root = tmp.path().join("plans").join("released");
    std::fs::create_dir_all(&plan_root).unwrap();
    let client = make_plan_client(vec![pattern], 1, Some(plan_root.clone()), None);
    let overlay = find_plan_dirs_overlay(client);
    let dirs = overlay
        .find_plan_dirs(&nested.to_string_lossy())
        .expect("dirs");
    assert_eq!(dirs, vec![plan_root]);
}

#[test]
fn find_plan_dirs_returns_none_when_no_plan_paths_configured() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path().to_string_lossy().to_string();
    let client = make_plan_client(vec![cwd.clone()], 1, None, None);
    let overlay = find_plan_dirs_overlay(client);
    assert!(overlay.find_plan_dirs(&cwd).is_none());
}
