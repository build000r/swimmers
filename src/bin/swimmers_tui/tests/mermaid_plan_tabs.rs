use super::*;

pub(super) fn open_mermaid_on_plan_tab(
    content: Option<&str>,
    active_tab: DomainPlanTab,
) -> (App<MockApi>, Renderer, WorkspaceLayout) {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph LR\nA-->B",
    );
    artifact.plan_files = Some(vec![
        "schema.mmd".to_string(),
        "plan.md".to_string(),
        "backend.md".to_string(),
    ]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);
    app.open_mermaid_viewer("sess-1".to_string());
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.active_tab = active_tab;
        viewer.plan_text_content = content.map(str::to_string);
    }
    let renderer = test_renderer(120, 32);
    (app, renderer, layout)
}

#[test]
fn render_plan_text_content_loading_state_when_no_content() {
    let (mut app, mut renderer, layout) = open_mermaid_on_plan_tab(None, DomainPlanTab::Plan);
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_heading_and_list_lines() {
    let content = "# Heading\n- list item\n  - nested\nbody text\n| table |\n|-|-|";
    let (mut app, mut renderer, layout) =
        open_mermaid_on_plan_tab(Some(content), DomainPlanTab::Plan);
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_scroll_indicator_when_content_exceeds_height() {
    // 50 lines of content will overflow the viewport height (~28 usable rows)
    let content = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, mut renderer, layout) =
        open_mermaid_on_plan_tab(Some(&content), DomainPlanTab::Plan);
    // Set scroll to trigger the non-zero pct branch
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.plan_text_scroll = 5;
    }
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_scroll_indicator_at_top_pct_100() {
    // Short enough that scroll is 0 but total_lines > visible → pct = 100 when max_scroll == 0
    // Actually max_scroll == 0 when total_lines <= visible_height, so we need more lines but scroll stays 0
    let content = (0..50)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (mut app, mut renderer, layout) =
        open_mermaid_on_plan_tab(Some(&content), DomainPlanTab::Plan);
    // Leave scroll at 0; max_scroll > 0 so we get normal pct calculation
    app.render(&mut renderer, layout);
}

#[test]
fn render_plan_text_content_rewraps_on_second_render_same_width() {
    let content = "# Title\nbody";
    let (mut app, mut renderer, layout) =
        open_mermaid_on_plan_tab(Some(content), DomainPlanTab::Backend);
    // First render populates plan_text_lines
    app.render(&mut renderer, layout);
    // Second render should reuse cached lines (no re-wrap needed)
    app.render(&mut renderer, layout);
}

// ── switch_plan_tab tests ─────────────────────────────────────────────────────

pub(super) fn open_mermaid_with_plan_tabs(api: MockApi) -> App<MockApi> {
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph LR\nA-->B",
    );
    artifact.plan_files = Some(vec![
        "schema.mmd".to_string(),
        "plan.md".to_string(),
        "backend.md".to_string(),
    ]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);
    app.open_mermaid_viewer("sess-1".to_string());
    app
}

#[test]
fn switch_plan_tab_noop_in_aquarium_mode() {
    let api = MockApi::new();
    let mut app = make_app(api);
    // Default mode is Aquarium; switch_plan_tab must not panic or change state
    app.switch_plan_tab(DomainPlanTab::Plan);
}

#[test]
fn switch_plan_tab_noop_when_no_plan_tabs() {
    let (mut app, _, _) = open_mermaid_test_viewer("graph LR\nA-->B", 120, 32);
    // viewer has no plan_tabs (open_mermaid_test_viewer doesn't set plan_files)
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    // active_tab unchanged
    assert_eq!(viewer.active_tab, DomainPlanTab::Schema);
}

#[test]
fn switch_plan_tab_noop_when_already_on_tab() {
    let api = MockApi::new();
    let mut app = open_mermaid_with_plan_tabs(api);
    // active_tab starts at Schema; switching to Schema again is a no-op
    app.switch_plan_tab(DomainPlanTab::Schema);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Schema);
}

#[test]
fn switch_plan_tab_to_schema_updates_viewer_without_fetch() {
    let api = MockApi::new();
    let mut app = open_mermaid_with_plan_tabs(api.clone());
    // Set active_tab to Plan first so switching to Schema is valid
    {
        let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
            panic!()
        };
        viewer.active_tab = DomainPlanTab::Plan;
        viewer.plan_text_content = Some("old content".to_string());
    }
    app.switch_plan_tab(DomainPlanTab::Schema);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Schema);
    assert!(viewer.plan_text_content.is_none());
    // No plan file fetch should have been issued
    assert_eq!(api.native_status_calls(), 0);
}

#[test]
fn switch_plan_tab_to_non_schema_fetches_plan_file_ok() {
    let api = MockApi::new();
    api.push_plan_file(Ok(PlanFileResponse {
        session_id: "sess-1".to_string(),
        name: "plan.md".to_string(),
        content: Some("# Plan\n- slice one".to_string()),
        error: None,
    }));
    let mut app = open_mermaid_with_plan_tabs(api);
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    assert_eq!(
        viewer.plan_text_content.as_deref(),
        Some("# Plan\n- slice one")
    );
    assert_eq!(viewer.plan_text_scroll, 0);
}

#[test]
fn open_mermaid_viewer_includes_repo_doc_tabs_when_advertised() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph LR\nA-->B",
    );
    artifact.plan_files = Some(vec![
        "plan.md".to_string(),
        "README.md".to_string(),
        "VISION.md".to_string(),
    ]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);

    app.open_mermaid_viewer("sess-1".to_string());

    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(
        viewer.plan_tabs,
        Some(vec![
            DomainPlanTab::Schema,
            DomainPlanTab::Plan,
            DomainPlanTab::Readme,
            DomainPlanTab::Vision,
        ])
    );
}

#[test]
fn switch_plan_tab_to_readme_fetches_artifact_file_ok() {
    let api = MockApi::new();
    api.push_plan_file(Ok(PlanFileResponse {
        session_id: "sess-1".to_string(),
        name: "README.md".to_string(),
        content: Some("# swimmers\n\nrepo docs".to_string()),
        error: None,
    }));
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow.mmd",
        "2026-03-23T10:05:00Z",
        "graph LR\nA-->B",
    );
    artifact.plan_files = Some(vec!["README.md".to_string()]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);
    app.open_mermaid_viewer("sess-1".to_string());

    app.switch_plan_tab(DomainPlanTab::Readme);

    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Readme);
    assert_eq!(
        viewer.plan_text_content.as_deref(),
        Some("# swimmers\n\nrepo docs")
    );
}

#[test]
fn switch_plan_tab_to_non_schema_shows_error_from_response() {
    let api = MockApi::new();
    api.push_plan_file(Ok(PlanFileResponse {
        session_id: "sess-1".to_string(),
        name: "plan.md".to_string(),
        content: None,
        error: Some("file not found".to_string()),
    }));
    let mut app = open_mermaid_with_plan_tabs(api);
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    assert!(app
        .message
        .as_ref()
        .map(|(m, _)| m.contains("artifact file"))
        .unwrap_or(false));
}

#[test]
fn switch_plan_tab_to_non_schema_shows_error_on_fetch_failure() {
    let api = MockApi::new();
    api.push_plan_file(Err("network error".to_string()));
    let mut app = open_mermaid_with_plan_tabs(api);
    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!()
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    assert!(viewer.plan_text_content.is_none());
    assert!(app
        .message
        .as_ref()
        .map(|(m, _)| m.contains("artifact file fetch failed"))
        .unwrap_or(false));
}
