use super::*;

#[test]
fn skill_panel_groups_available_skills_by_source_and_highlights_sbp() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let field = Rect {
        x: 1,
        y: 3,
        width: 118,
        height: 22,
    };
    let swimmers = session_summary("sess-swimmers", "1", TEST_REPO_SWIMMERS);
    let skills_repo = session_summary("sess-skills", "2", TEST_REPO_SKILLS);
    app.merge_sessions(vec![swimmers.clone(), skills_repo.clone()], field);
    app.selected_id = Some("sess-swimmers".to_string());
    app.session_skill_cache.insert(
        swimmers.session_id.clone(),
        SkillCacheEntry {
            context: SkillCacheContext::from_session(&swimmers),
            response: Some(session_skill_response(
                &swimmers.session_id,
                &swimmers.cwd,
                vec![
                    session_skill(
                        "sbp",
                        "skillbox",
                        "/Users/tester/repos/skills-private/sbp",
                        "/repo/.codex/skills/sbp",
                    ),
                    session_skill(
                        "ui",
                        "skills-private",
                        "/Users/tester/repos/skills-private/ui",
                        "/repo/.codex/skills/ui",
                    ),
                ],
            )),
        },
    );
    app.session_skill_cache.insert(
        skills_repo.session_id.clone(),
        SkillCacheEntry {
            context: SkillCacheContext::from_session(&skills_repo),
            response: Some(session_skill_response(
                &skills_repo.session_id,
                &skills_repo.cwd,
                vec![session_skill(
                    "ui",
                    "skills-private",
                    "/Users/tester/repos/skills-private/ui",
                    "/repo/.codex/skills/ui",
                )],
            )),
        },
    );

    let layout = build_skill_panel(&app, field);

    assert!(layout.panel_rect.is_some());
    assert!(layout.tank_field.width < field.width);
    assert_eq!(layout.header, "skills via SBP");
    assert!(layout.context_line.contains("swimmers*"));
    assert!(layout.context_line.contains("skills"));
    assert!(layout.status_line.contains("SBP active 2/2 repos"));

    let sbp_row = layout
        .rows
        .iter()
        .find_map(|row| match &row.kind {
            SkillPanelRowKind::Skill(skill) if skill.name == "sbp" => Some((row, skill)),
            _ => None,
        })
        .expect("sbp skill row");
    assert_eq!(sbp_row.0.color, Color::Yellow);
    assert!(sbp_row.1.sbp_highlight);
    assert_eq!(sbp_row.1.contexts, vec!["swimmers"]);

    let ui_row = layout
        .rows
        .iter()
        .find_map(|row| match &row.kind {
            SkillPanelRowKind::Skill(skill) if skill.name == "ui" => Some(skill),
            _ => None,
        })
        .expect("ui skill row");
    assert_eq!(ui_row.contexts, vec!["swimmers", "skills"]);
    assert!(ui_row.source_label.ends_with("skills-private/ui"));
}

#[test]
fn clicking_skill_panel_row_opens_plan_style_skill_atlas() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let field = Rect {
        x: 1,
        y: 3,
        width: 118,
        height: 22,
    };
    let swimmers = session_summary("sess-swimmers", "1", TEST_REPO_SWIMMERS);
    app.merge_sessions(vec![swimmers.clone()], field);
    app.selected_id = Some(swimmers.session_id.clone());
    app.session_skill_cache.insert(
        swimmers.session_id.clone(),
        SkillCacheEntry {
            context: SkillCacheContext::from_session(&swimmers),
            response: Some(session_skill_response(
                &swimmers.session_id,
                &swimmers.cwd,
                vec![session_skill(
                    "ui",
                    "skills-private",
                    "/Users/tester/repos/skills-private/ui",
                    "/repo/.codex/skills/ui",
                )],
            )),
        },
    );
    let layout = build_skill_panel(&app, field);
    let ui_rect = layout
        .rows
        .iter()
        .find_map(|row| match &row.kind {
            SkillPanelRowKind::Skill(skill) if skill.name == "ui" => Some(row.rect),
            _ => None,
        })
        .expect("ui row rect");

    app.handle_field_click(ui_rect.x, ui_rect.y, field);

    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected skill atlas viewer");
    };
    assert_eq!(viewer.tmux_name, "skill atlas: skill ui");
    assert_eq!(viewer.zoom, 1.0);
    assert_eq!((viewer.center_x, viewer.center_y), (0.0, 0.0));
    assert_eq!(
        viewer.plan_tabs,
        Some(vec![DomainPlanTab::Schema, DomainPlanTab::Plan])
    );
    assert!(viewer.inline_plan_files.contains_key(&DomainPlanTab::Plan));
    let source = viewer
        .source
        .as_deref()
        .expect("skill atlas Mermaid source");
    assert!(source.starts_with("flowchart TB"));
    assert!(source.contains("Skill Atlas"));
    assert!(source.contains("FOCUS\\nskill ui"));
    assert!(source.contains("source directories"));
    assert!(source.contains("ui"));

    let mermaid_layout = WorkspaceLayout::for_terminal_without_thought_panel(120, 32);
    let mut renderer = test_renderer(120, 32);
    app.render(&mut renderer, mermaid_layout);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected skill atlas viewer");
    };
    assert!(viewer.render_error.is_none());

    app.switch_plan_tab(DomainPlanTab::Plan);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected skill atlas viewer");
    };
    assert_eq!(viewer.active_tab, DomainPlanTab::Plan);
    let plan = viewer
        .plan_text_content
        .as_deref()
        .expect("inline skill atlas plan tab");
    assert!(plan.contains("# Skill Atlas"));
    assert!(plan.contains("Focus: skill ui"));
    assert!(plan.contains("## Skills By Source Directory"));

    app.switch_plan_tab(DomainPlanTab::Schema);
    app.pan_mermaid_viewer(2.0, -1.0);
    let before_scroll = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.zoom, viewer.center_x, viewer.center_y),
        FishBowlMode::Aquarium => panic!("expected skill atlas viewer"),
    };
    scroll_mermaid(&mut app, mermaid_layout, MermaidZoomDirection::In);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected skill atlas viewer");
    };
    assert!(viewer.zoom > before_scroll.0);
    assert_ne!(
        (viewer.center_x, viewer.center_y),
        (before_scroll.1, before_scroll.2)
    );
    app.close_mermaid_viewer();
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn clicking_skill_panel_source_row_opens_source_focused_atlas() {
    let api = MockApi::new();
    let mut app = make_app(api);
    let field = Rect {
        x: 1,
        y: 3,
        width: 118,
        height: 22,
    };
    let swimmers = session_summary("sess-swimmers", "1", TEST_REPO_SWIMMERS);
    app.merge_sessions(vec![swimmers.clone()], field);
    app.selected_id = Some(swimmers.session_id.clone());
    app.session_skill_cache.insert(
        swimmers.session_id.clone(),
        SkillCacheEntry {
            context: SkillCacheContext::from_session(&swimmers),
            response: Some(session_skill_response(
                &swimmers.session_id,
                &swimmers.cwd,
                vec![
                    session_skill(
                        "sbp",
                        "skillbox",
                        "/Users/tester/repos/skills-private/sbp",
                        "/repo/.codex/skills/sbp",
                    ),
                    session_skill(
                        "ui",
                        "skills-private",
                        "/Users/tester/repos/skills-private/ui",
                        "/repo/.codex/skills/ui",
                    ),
                ],
            )),
        },
    );
    let layout = build_skill_panel(&app, field);
    let source_rect = layout
        .rows
        .iter()
        .find_map(|row| match &row.kind {
            SkillPanelRowKind::Source { label, .. } if label.ends_with("skills-private/sbp") => {
                Some(row.rect)
            }
            _ => None,
        })
        .expect("source row rect");

    app.handle_field_click(source_rect.x, source_rect.y, field);

    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected source atlas viewer");
    };
    assert!(viewer.tmux_name.starts_with("skill atlas: source "));
    let source = viewer.source.as_deref().expect("source atlas Mermaid");
    assert!(source.contains("FOCUS\\nsource "));
    assert!(source.contains("[SBP] sbp"));
    assert!(source.contains("active repo contexts"));
}
