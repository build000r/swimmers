use super::*;

#[test]
fn maybe_refresh_plans_uses_tui_api_source() {
    let api = MockApi::new();
    api.push_overlay_plans(Ok(vec![PlanPanelEntry {
        slug: "alpha".to_string(),
        client_label: "remote".to_string(),
        kind: "released".to_string(),
        schema_path: "/tmp/plans/alpha/schema.mmd".to_string(),
    }]));
    let mut app = make_app(api);
    app.tick = 1;

    app.maybe_refresh_plans();

    assert_eq!(app.cached_plans.len(), 1);
    assert_eq!(app.cached_plans[0].slug, "alpha");
    assert_eq!(app.cached_plans[0].client_label, "remote");
}

#[test]
fn open_plan_viewer_reads_schema_source_from_disk() {
    let temp = tempdir().expect("tempdir");
    let plan_dir = temp.path().join("plans").join("released").join("alpha");
    fs::create_dir_all(&plan_dir).expect("plan dir");
    let schema = plan_dir.join("schema.mmd");
    fs::write(&schema, "graph TD\nA-->B\n").expect("schema");
    fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("plan doc");

    let mut app = make_app(MockApi::new());
    app.open_plan_viewer(schema.to_string_lossy().into_owned(), "alpha".to_string());

    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected mermaid viewer");
    };
    assert!(viewer.disk_only);
    assert_eq!(viewer.source.as_deref(), Some("graph TD\nA-->B\n"));
    assert!(viewer.artifact_error.is_none());
    assert!(viewer
        .plan_tabs
        .as_ref()
        .is_some_and(|tabs| tabs.contains(&DomainPlanTab::Plan)));
}

#[test]
fn read_plan_file_from_disk_rejects_path_traversal_names() {
    let temp = tempdir().expect("tempdir");
    let plan_dir = temp.path().join("plans").join("released").join("alpha");
    fs::create_dir_all(&plan_dir).expect("plan dir");
    let schema = plan_dir.join("schema.mmd");
    fs::write(&schema, "graph TD\nA-->B\n").expect("schema");
    fs::write(plan_dir.join("plan.md"), "# Plan\n").expect("plan doc");
    fs::write(plan_dir.parent().unwrap().join("secret.txt"), "secret\n").expect("secret");

    let schema_path = schema.to_string_lossy().into_owned();
    let valid = read_plan_file_from_disk(Some(&schema_path), "plan.md").expect("valid response");
    assert_eq!(valid.content.as_deref(), Some("# Plan\n"));

    let rejected =
        read_plan_file_from_disk(Some(&schema_path), "../secret.txt").expect("rejection response");
    assert!(rejected.content.is_none());
    assert_eq!(
        rejected.error.as_deref(),
        Some("artifact file name not allowed: ../secret.txt")
    );
}

#[test]
fn resolve_tui_log_path_honors_explicit_dir_override() {
    // SWIMMERS_TUI_LOG_DIR takes precedence over TMPDIR. The filename always
    // embeds the calling pid so concurrent TUIs don't clobber each other.
    let _lock = lock_test_env();
    let tmp = tempdir().expect("tempdir");
    let prior_dir = env::var_os("SWIMMERS_TUI_LOG_DIR");
    let prior_tmp = env::var_os("TMPDIR");
    env::set_var("SWIMMERS_TUI_LOG_DIR", tmp.path());
    env::set_var("TMPDIR", "/should-not-be-used");

    let (dir, path) = super::resolve_tui_log_path();
    assert_eq!(dir, tmp.path());
    let expected_name = format!("swimmers-tui-client-{}.log", std::process::id());
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some(expected_name.as_str())
    );
    assert_eq!(path.parent(), Some(tmp.path()));

    match prior_dir {
        Some(value) => env::set_var("SWIMMERS_TUI_LOG_DIR", value),
        None => env::remove_var("SWIMMERS_TUI_LOG_DIR"),
    }
    match prior_tmp {
        Some(value) => env::set_var("TMPDIR", value),
        None => env::remove_var("TMPDIR"),
    }
}
