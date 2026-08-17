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
        ssh_alias: None,
        remote_attach_command_template: None,
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
        ssh_alias: None,
        remote_attach_command_template: None,
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
        fleet_presets: Vec::new(),
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
        fleet_presets: Vec::new(),
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
fn expand_bare_tilde() {
    // Regression: a bare `~` (no trailing slash) must expand to home too, else
    // the overlay path is left literal and silently dropped downstream.
    let expanded = expand_path("~");
    assert_ne!(expanded, "~");
    if let Some(home) = dirs::home_dir() {
        assert_eq!(expanded, home.display().to_string());
    }
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

// ---------------------------------------------------------------------------
// Skillbox environment-inventory contract
// ---------------------------------------------------------------------------

fn json_string(raw: &str) -> String {
    serde_json::to_string(raw).expect("json string")
}

/// Every field the contract must still promise, optionally dropping one to
/// prove the loader notices.
fn superseded_fields_json(dropped: Option<&str>) -> String {
    let entries: Vec<String> = REQUIRED_SUPERSEDED_FIELDS
        .iter()
        .filter(|field| Some(**field) != dropped)
        .map(|field| format!("{}:[\"contract.path\"]", json_string(field)))
        .collect();
    format!("{{{}}}", entries.join(","))
}

fn contract_document(clients: &str, repos: &str, sources: &str) -> String {
    contract_document_with(r#"{"declared":{}}"#, clients, repos, sources, None)
}

fn contract_document_with(
    machine: &str,
    clients: &str,
    repos: &str,
    sources: &str,
    dropped_field: Option<&str>,
) -> String {
    let fields = superseded_fields_json(dropped_field);
    format!(
        r#"{{"contract":"{INVENTORY_CONTRACT}","schema_version":"{INVENTORY_SCHEMA_VERSION}",
           "machine":{machine},"clients":{clients},"repos":{repos},"sources":{sources},
           "readiness":{{"status":"ready"}},"freshness":{{"observed":false}},"recovery":[],
           "supersedes":{{"consumer":"swimmers","fields":{fields}}}}}"#
    )
}

fn load_contract(document: &str) -> Result<LoadedInventory, ContractUnavailable> {
    parse_inventory_json(document, "test-contract")?.into_payload()
}

/// Full happy path: parse, verify the contract still promises what we deleted
/// our parser for, then project clients.
fn contract_clients(document: &str) -> Vec<ClientOverlay> {
    let payload = load_contract(document).expect("payload");
    payload.verify_contract_version().expect("schema version");
    payload.verify_supersedes().expect("supersedes");
    payload.build_client_overlays()
}

#[test]
fn contract_reports_not_built_instead_of_returning_none() {
    let envelope = r#"{"ok":false,"action":"show","source":"cache","stale":false,
        "cache_path":"/box/.skillbox-state/inventory/environment_inventory.json",
        "inventory":null,"reason":"no cache written yet",
        "next_actions":["python3 .env-manager/manage.py env-inventory refresh --format json"]}"#;

    let err = load_contract(envelope).expect_err("cache miss must be typed");

    assert_eq!(err.code(), "CONTRACT_NOT_BUILT");
    assert_eq!(
        err.remedy().as_deref(),
        Some("python3 .env-manager/manage.py env-inventory refresh --format json")
    );
    assert!(err.to_string().contains("no cache written yet"));
}

#[test]
fn contract_rejects_an_unknown_schema_version() {
    let document =
        contract_document("[]", "[]", "[]").replace(INVENTORY_SCHEMA_VERSION, "1999-01-01+v0");

    let err = load_contract(&document)
        .expect("payload")
        .verify_contract_version()
        .expect_err("schema mismatch must be typed");

    assert_eq!(err.code(), "CONTRACT_SCHEMA_UNSUPPORTED");
    assert!(err.to_string().contains("1999-01-01+v0"));
}

#[test]
fn contract_rejects_a_supersedes_block_that_dropped_a_field_we_stopped_parsing() {
    let document = contract_document_with(
        r#"{"declared":{}}"#,
        "[]",
        "[]",
        "[]",
        Some("context.cwd_match"),
    );

    let err = load_contract(&document)
        .expect("payload")
        .verify_supersedes()
        .expect_err("missing promise must be typed");

    assert_eq!(err.code(), "CONTRACT_SUPERSEDES_INCOMPLETE");
    assert!(err.to_string().contains("context.cwd_match"));
}

#[test]
fn contract_treats_an_empty_supersedes_promise_as_missing() {
    let document = contract_document("[]", "[]", "[]").replace(
        &format!("{}:[\"contract.path\"]", json_string("client.label")),
        &format!("{}:[]", json_string("client.label")),
    );

    let err = load_contract(&document)
        .expect("payload")
        .verify_supersedes()
        .expect_err("empty promise must be typed");

    assert!(err.to_string().contains("client.label"));
}

#[test]
fn contract_unavailable_codes_are_distinct() {
    let codes = [
        ContractUnavailable::SkillboxRepoNotFound {
            searched: Vec::new(),
        },
        ContractUnavailable::CommandUnavailable {
            command: String::new(),
            detail: String::new(),
        },
        ContractUnavailable::CommandFailed {
            command: String::new(),
            code: None,
            detail: String::new(),
        },
        ContractUnavailable::MalformedPayload {
            source: String::new(),
            detail: String::new(),
        },
        ContractUnavailable::NotBuilt {
            reason: String::new(),
            next_action: None,
        },
        ContractUnavailable::SchemaUnsupported {
            contract: String::new(),
            schema_version: String::new(),
        },
        ContractUnavailable::SupersedesIncomplete {
            consumer: String::new(),
            missing: Vec::new(),
        },
        ContractUnavailable::NoClients {
            schema_version: String::new(),
        },
    ]
    .iter()
    .map(ContractUnavailable::code)
    .collect::<BTreeSet<_>>();

    assert_eq!(codes.len(), 8, "every reason needs its own operator action");
}

#[test]
fn contract_client_overlay_paths_skip_absent_and_pathless_sources() {
    let sources = vec![
        InventorySource {
            kind: "client_overlay".to_string(),
            client_id: Some("present".to_string()),
            path: Some("/config/clients/present/overlay.yaml".to_string()),
            present: true,
        },
        InventorySource {
            kind: "client_overlay".to_string(),
            client_id: Some("absent".to_string()),
            path: Some("/config/clients/absent/overlay.yaml".to_string()),
            present: false,
        },
        InventorySource {
            kind: "client_overlay".to_string(),
            client_id: Some("pathless".to_string()),
            path: None,
            present: true,
        },
        InventorySource {
            kind: "machines".to_string(),
            client_id: None,
            path: Some("/config/machines.yaml".to_string()),
            present: true,
        },
    ];

    let paths = client_overlay_paths(&sources);

    assert_eq!(paths.keys().copied().collect::<Vec<_>>(), vec!["present"]);
    assert_eq!(
        paths["present"],
        PathBuf::from("/config/clients/present/overlay.yaml")
    );
}

#[test]
fn contract_repo_paths_translate_declared_roots_onto_this_machine() {
    let repo = InventoryRepo {
        repo_id: "sha256:deadbeef".to_string(),
        declared: InventoryRepoDeclared {
            path_declared: Some("/elsewhere/repos/widget".to_string()),
            path_relative: Some("widget".to_string()),
            root_category: Some("repos".to_string()),
            ..InventoryRepoDeclared::default()
        },
        observed: None,
    };
    let roots = MachineRoots {
        repos: vec!["/box/repos".to_string(), "/box/repos-alias".to_string()],
        projects: vec!["/box/projects".to_string()],
    };

    assert_eq!(
        repo_match_paths(&repo, &roots),
        vec![
            "/box/repos/widget".to_string(),
            "/box/repos-alias/widget".to_string(),
            "/elsewhere/repos/widget".to_string(),
        ],
        "root translation is Skillbox's job, and we must consume it"
    );
}

#[test]
fn contract_repo_paths_prefer_the_observed_path_when_present() {
    let repo = InventoryRepo {
        repo_id: "sha256:deadbeef".to_string(),
        declared: InventoryRepoDeclared {
            path_declared: Some("~/repos/widget".to_string()),
            path_relative: Some("widget".to_string()),
            root_category: Some("repos".to_string()),
            ..InventoryRepoDeclared::default()
        },
        observed: Some(InventoryRepoObserved {
            path: Some("/box/repos/widget".to_string()),
            present: true,
        }),
    };
    let roots = MachineRoots {
        repos: vec!["/box/repos".to_string()],
        projects: Vec::new(),
    };

    assert_eq!(repo_match_paths(&repo, &roots)[0], "/box/repos/widget");
    assert_eq!(
        observed_repo_path(&repo).as_deref(),
        Some("/box/repos/widget")
    );
}

#[test]
fn contract_repo_paths_ignore_an_observed_repo_that_is_not_present() {
    let repo = InventoryRepo {
        repo_id: "sha256:deadbeef".to_string(),
        declared: InventoryRepoDeclared {
            path_declared: Some("/elsewhere/widget".to_string()),
            ..InventoryRepoDeclared::default()
        },
        observed: Some(InventoryRepoObserved {
            path: Some("/box/repos/widget".to_string()),
            present: false,
        }),
    };

    assert_eq!(observed_repo_path(&repo), None);
    assert_eq!(
        repo_match_paths(&repo, &MachineRoots::default()),
        vec!["/elsewhere/widget".to_string()]
    );
}

#[test]
fn contract_client_cwd_patterns_follow_match_then_scan_roots_then_landscape_repos() {
    let clients = contract_clients(&contract_document(
        r#"[{"client_id":"personal","declared":{"client_id":"personal","label":"Personal",
             "cwd_match":["/box/repos/one"],"scan_roots":["/box/scan"],
             "repo_ids":["sha256:aaa","sha256:bbb"],"plan_root":null,"plan_draft":null}}]"#,
        r#"[{"repo_id":"sha256:aaa","declared":{"path_declared":"/box/landscape/alpha",
             "declared_by":["client:personal.repo_landscape"]}},
            {"repo_id":"sha256:bbb","declared":{"path_declared":"/box/repos/beta",
             "declared_by":["client:personal.repos"]}}]"#,
        "[]",
    ));

    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].label, "Personal");
    assert_eq!(clients[0].cwd_match_count, 1);
    assert_eq!(
        clients[0].cwd_patterns,
        vec![
            "/box/repos/one".to_string(),
            "/box/scan".to_string(),
            "/box/landscape/alpha".to_string(),
        ],
        "repos declared under client.repos[] are not cwd patterns"
    );
}

#[test]
fn contract_client_label_falls_back_to_the_declared_client_id() {
    let clients = contract_clients(&contract_document(
        r#"[{"client_id":"dir-name","declared":{"client_id":"declared-id","label":"  ",
             "cwd_match":[],"scan_roots":[],"repo_ids":[]}}]"#,
        "[]",
        "[]",
    ));

    assert_eq!(clients[0].label, "declared-id");
}

#[test]
fn contract_plan_dirs_resolve_against_the_client_overlay_directory() {
    let clients = contract_clients(&contract_document(
        r#"[{"client_id":"personal","declared":{"client_id":"personal","label":"Personal",
             "cwd_match":[],"scan_roots":[],"repo_ids":[],
             "plan_root":"plans/released","plan_draft":"plans/draft"}}]"#,
        "[]",
        r#"[{"kind":"client_overlay","client_id":"personal","present":true,
             "path":"/config/clients/personal/overlay.yaml"}]"#,
    ));

    assert_eq!(
        clients[0].plan_root,
        Some(PathBuf::from("/config/clients/personal/plans/released"))
    );
    assert_eq!(
        clients[0].plan_draft,
        Some(PathBuf::from("/config/clients/personal/plans/draft"))
    );
}

#[test]
fn contract_plan_dirs_are_none_without_a_readable_client_overlay_source() {
    let clients = contract_clients(&contract_document(
        r#"[{"client_id":"personal","declared":{"client_id":"personal","label":"Personal",
             "cwd_match":[],"scan_roots":[],"repo_ids":[],
             "plan_root":"plans/released","plan_draft":"plans/draft"}}]"#,
        "[]",
        r#"[{"kind":"client_overlay","client_id":"personal","present":false,
             "path":"/config/clients/personal/overlay.yaml"}]"#,
    ));

    assert_eq!(clients[0].plan_root, None);
    assert_eq!(clients[0].plan_draft, None);
    assert!(clients[0].dir_config.is_none());
}

#[test]
fn contract_accepts_a_bare_inventory_document_as_well_as_the_cli_envelope() {
    let payload = load_contract(&contract_document("[]", "[]", "[]")).expect("bare document");

    assert_eq!(payload.source, "file");
    assert!(!payload.stale);
    assert_eq!(payload.facts().schema_version, INVENTORY_SCHEMA_VERSION);
}

#[test]
fn contract_facts_carry_machine_identity_and_staleness_for_health() {
    let document = format!(
        r#"{{"ok":true,"source":"cache","stale":true,"inventory":{}}}"#,
        contract_document_with(
            r#"{"declared":{"machine_id":"mac-laptop","repo_roots":[],"projects_roots":[]},
                "detection_source":"hostname"}"#,
            "[]",
            "[]",
            "[]",
            None,
        )
    );

    let facts = load_contract(&document).expect("payload").facts();

    assert_eq!(facts.source, "cache");
    assert!(facts.stale, "staleness is reported, never blocking");
    assert_eq!(facts.machine_id.as_deref(), Some("mac-laptop"));
    assert_eq!(facts.detection_source.as_deref(), Some("hostname"));
    assert_eq!(facts.readiness.as_deref(), Some("ready"));
}

#[test]
fn contract_malformed_stdout_is_typed_rather_than_silent() {
    let err = load_contract("not json at all").expect_err("garbage must be typed");

    assert_eq!(err.code(), "CONTRACT_MALFORMED");
    assert!(err.to_string().contains("test-contract"));
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
            ssh_alias: Some("remote-ssh".to_string()),
            remote_attach_command_template: None,
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
    assert_eq!(remote.ssh_alias.as_deref(), Some("remote-ssh"));
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
                ssh_alias: None,
                remote_attach_command_template: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            DevSanityLaunchTarget {
                id: Some("   ".to_string()),
                label: Some("Blank".to_string()),
                kind: Some("swimmers_api".to_string()),
                base_url: Some("http://blank.test:3210".to_string()),
                auth_token_env: None,
                ssh_alias: None,
                remote_attach_command_template: None,
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
            ssh_alias: None,
            remote_attach_command_template: None,
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
        contract: ContractFacts::default(),
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
            fleet_presets: Vec::new(),
        }],
        loaded_at: Utc::now(),
        contract: ContractFacts::default(),
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
        contract: ContractFacts::default(),
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
        contract: ContractFacts::default(),
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
        contract: ContractFacts::default(),
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

    append_scan_root_services(
        &mut services,
        &mut seen_dirs,
        std::slice::from_ref(&external),
        &base,
    );

    assert_eq!(
        services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>(),
        vec!["manual-alpha", "beta"]
    );
}

#[test]
fn contract_client_repos_become_dir_config_services() {
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

    // Only `dev_sanity` survives on disk: the client/context sections that used
    // to live alongside it are now read from the contract.
    std::fs::write(
        &overlay_path,
        format!(
            r#"
version: 1
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
            repo_base = repo_base.display()
        ),
    )
    .expect("write overlay");

    let clients = contract_clients(&contract_document(
        &format!(
            r#"[{{"client_id":"personal","declared":{{"client_id":"personal","label":"personal",
                 "cwd_match":[{repo_base}],"scan_roots":[{hard_root}],
                 "repo_ids":["sha256:aaa","sha256:bbb","sha256:ccc"]}}}}]"#,
            repo_base = json_string(&repo_base.display().to_string()),
            hard_root = json_string(&hard_root.display().to_string()),
        ),
        &format!(
            r#"[{{"repo_id":"sha256:aaa","declared":{{"registry_id":"finalreceipts","kind":"repo",
                 "path_declared":{finalreceipts},"declared_by":["client:personal.repos"]}}}},
                {{"repo_id":"sha256:bbb","declared":{{"registry_id":"sweet-potato-dupe","kind":"repo",
                 "path_declared":{sweet_potato},"declared_by":["client:personal.repos"]}}}},
                {{"repo_id":"sha256:ccc","declared":{{"registry_id":"mmd-pcb","kind":"repo",
                 "path_declared":{hard_repo},"declared_by":["client:personal.repos"]}}}}]"#,
            finalreceipts = json_string(&repo_base.join("finalreceipts").display().to_string()),
            sweet_potato = json_string(&repo_base.join("sweet-potato").display().to_string()),
            hard_repo = json_string(&hard_repo.display().to_string()),
        ),
        &format!(
            r#"[{{"kind":"client_overlay","client_id":"personal","present":true,"path":{path}}}]"#,
            path = json_string(&overlay_path.display().to_string()),
        ),
    ));

    let config = clients
        .into_iter()
        .next()
        .expect("client")
        .dir_config
        .expect("dir config");
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
fn contract_client_repos_skip_non_repo_kinds() {
    let entry = service_entry_from_client_repo(
        &InventoryRepo {
            repo_id: "sha256:aaa".to_string(),
            declared: InventoryRepoDeclared {
                registry_id: Some("docs".to_string()),
                kind: Some("notes".to_string()),
                path_declared: Some("/box/repos/docs".to_string()),
                ..InventoryRepoDeclared::default()
            },
            observed: None,
        },
        Path::new("/box/repos"),
    );

    assert!(entry.is_none());
}

#[test]
fn contract_client_keeps_harmless_fleet_lens_presets() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let client_dir = tmp.path().join("clients").join("personal");
    std::fs::create_dir_all(&client_dir).expect("client dir");
    let repo_base = tmp.path().join("repos");
    std::fs::create_dir_all(&repo_base).expect("repo base");
    let overlay_path = client_dir.join("overlay.yaml");
    std::fs::write(
        &overlay_path,
        format!(
            r#"
version: 1
dev_sanity:
  fleet_lenses:
    - id: swimmers-on-devbox
      label: Swimmers on devbox
      matchers:
        - type: target_kind
          kind: swimmers_api
        - type: repo
          key: {repo_base}/opensource/swimmers
  services:
    base_path: {repo_base}
    entries: []
"#,
            repo_base = repo_base.display()
        ),
    )
    .expect("write overlay");

    let clients = contract_clients(&contract_document(
        r#"[{"client_id":"personal","declared":{"client_id":"personal","label":"personal",
             "cwd_match":[],"scan_roots":[],"repo_ids":[]}}]"#,
        "[]",
        &format!(
            r#"[{{"kind":"client_overlay","client_id":"personal","present":true,"path":{path}}}]"#,
            path = json_string(&overlay_path.display().to_string()),
        ),
    ));

    let client = clients.into_iter().next().expect("client");
    assert_eq!(client.fleet_presets.len(), 1);
    assert_eq!(client.fleet_presets[0].id, "swimmers-on-devbox");
    assert_eq!(client.fleet_presets[0].label, "Swimmers on devbox");
    assert_eq!(client.fleet_presets[0].source, "overlay");
    assert!(matches!(
        &client.fleet_presets[0].matchers[0],
        FleetLensPresetMatcher::TargetKind { kind } if kind == "swimmers_api"
    ));
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
        fleet_presets: Vec::new(),
    };
    let overlay = SkillboxOverlay {
        clients: vec![client],
        loaded_at: Utc::now(),
        contract: ContractFacts::default(),
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
                fleet_presets: Vec::new(),
            },
            ClientOverlay {
                label: "alpha".to_string(),
                cwd_patterns: Vec::new(),
                cwd_match_count: 0,
                plan_root: Some(alpha_root),
                plan_draft: None,
                dir_config: None,
                fleet_presets: Vec::new(),
            },
        ],
        loaded_at: Utc::now(),
        contract: ContractFacts::default(),
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
        fleet_presets: Vec::new(),
    };
    let overlay = SkillboxOverlay {
        clients: vec![client],
        loaded_at: Utc::now(),
        contract: ContractFacts::default(),
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
        kind: " Swimmers_API ".to_string(),
        base_url: Some("http://example.test:3210".to_string()),
        auth_token_env: Some("REMOTE_TOKEN".to_string()),
        ssh_alias: None,
        remote_attach_command_template: None,
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
        fleet_presets: Vec::new(),
    };
    let overlay = SkillboxOverlay {
        clients: vec![client],
        loaded_at: Utc::now() - chrono::Duration::seconds(1),
        contract: ContractFacts::default(),
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
        contract: ContractFacts::default(),
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
        fleet_presets: Vec::new(),
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

/// End-to-end check against the real Skillbox checkout on this box.
///
/// Ignored by default: it shells out to `manage.py env-inventory show --cached`,
/// so it only means anything where Skillbox is actually installed and its cache
/// has been built. Run with
/// `cargo test session::overlay -- --ignored --nocapture`.
#[test]
#[ignore = "requires a local skillbox checkout with a built inventory cache"]
fn live_contract_loads_clients_from_the_skillbox_cli() {
    let overlay = match default_overlay_result() {
        Ok(overlay) => overlay,
        Err(unavailable) => panic!("[{}] {unavailable}", unavailable.code()),
    };
    let facts = overlay.contract();

    assert_eq!(facts.schema_version, INVENTORY_SCHEMA_VERSION);
    assert!(
        !overlay.all_launch_targets().is_empty(),
        "a live contract should still yield launch targets from dev_sanity"
    );
    println!(
        "contract source={} stale={} observed={} machine={:?} detection={:?} readiness={:?} \
         launch_targets={} plans={}",
        facts.source,
        facts.stale,
        facts.observed,
        facts.machine_id,
        facts.detection_source,
        facts.readiness,
        overlay.all_launch_targets().len(),
        overlay.list_all_plans().len(),
    );
}
