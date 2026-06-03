use super::*;

#[test]
fn mermaid_semantic_labels_track_zoom_and_pan() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    let content_rect = mermaid_content_rect(layout.overview_field);

    app.render(&mut renderer, layout);
    let (alpha_before, beta_before) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node before"),
            find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node before"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    app.zoom_mermaid_viewer(MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
    app.pan_mermaid_viewer(24.0, 18.0);
    app.render(&mut renderer, layout);

    let (alpha_after, beta_after, prepare_count) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node after"),
            find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node after"),
            viewer.source_prepare_count,
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_ne!(alpha_after, alpha_before);
    assert_ne!(beta_after, beta_before);
    assert_eq!(prepare_count, 1);
}

#[test]
fn mermaid_zoom_status_clamps_to_fit_and_uses_round_percentages() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Producer] --> B[Consumer]\n", 120, 32);

    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("fit 100%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    press_mermaid_key(&mut app, layout, '-');
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("fit 100%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("zoom 150%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
    assert!(
        !row_text(&renderer, layout.overview_field.y).contains("179%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_sequence_diagram_falls_back_to_connector_only_background() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    app.render(&mut renderer, layout);

    let (render_error, cached_lines_empty, cached_semantic_lines_empty, background_chars) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                viewer.render_error.clone(),
                viewer.cached_lines.is_empty(),
                viewer.cached_semantic_lines.is_empty(),
                mermaid_background_charset(viewer),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };
    assert_eq!(render_error, None);
    assert!(!cached_lines_empty);
    assert!(cached_semantic_lines_empty);
    assert!(find_text_position(&renderer, "hello").is_none());
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '\\' | '>' | '<')),
        "{background_chars:?}"
    );
}

#[test]
fn mermaid_tab_reports_no_semantic_targets_for_sequence_diagrams() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    let (focused_source_index, focus_status) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.focused_source_index, viewer.focus_status.clone()),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_source_index, None);
    assert_eq!(focus_status.as_deref(), Some("no semantic targets"));
    assert!(row_text(&renderer, layout.overview_field.y).contains("no semantic targets"));
}

#[test]
fn mermaid_semantic_labels_clip_to_viewport_bounds() {
    let content_rect = Rect {
        x: 42,
        y: 10,
        width: 20,
        height: 5,
    };
    let projected = project_mermaid_semantic_lines(
        &[MermaidSemanticLine {
            text: "Alpha Node".to_string(),
            diagram_x: 0.0,
            diagram_y: 4.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::NodeSummary,
            owner_key: "node:A".to_string(),
            outline_eligible: true,
            owner_width: 20.0,
            owner_height: 8.0,
        }],
        MermaidViewportTransform {
            scale: 1.0,
            tx: -4.0,
            ty: 0.0,
        },
        content_rect,
        MermaidViewState::L1,
    );

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].x, content_rect.x);
    assert_eq!(projected[0].y, content_rect.y + 1);
    assert_eq!(projected[0].text, "Alpha Node");
}

#[test]
fn mermaid_compacts_multiline_node_text_to_consecutive_rows() {
    let content_rect = Rect {
        x: 10,
        y: 10,
        width: 30,
        height: 8,
    };
    let projected = project_mermaid_semantic_lines(
        &[
            MermaidSemanticLine {
                text: "first line".to_string(),
                diagram_x: 0.0,
                diagram_y: 4.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 20.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "second line".to_string(),
                diagram_x: 0.0,
                diagram_y: 12.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 20.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "third line".to_string(),
                diagram_x: 0.0,
                diagram_y: 20.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 20.0,
                owner_height: 20.0,
            },
        ],
        MermaidViewportTransform {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        content_rect,
        MermaidViewState::L2,
    );

    assert_eq!(projected.len(), 3);
    assert_eq!(
        projected.iter().map(|line| line.y).collect::<Vec<_>>(),
        vec![content_rect.y + 1, content_rect.y + 2, content_rect.y + 3]
    );
}

#[test]
fn mermaid_detail_projection_hides_owner_summary_when_detail_lines_exist() {
    let content_rect = Rect {
        x: 10,
        y: 10,
        width: 40,
        height: 10,
    };
    let projected = project_mermaid_semantic_lines(
        &[
            MermaidSemanticLine {
                text: "Alpha compact".to_string(),
                diagram_x: 0.0,
                diagram_y: 12.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeSummary,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 24.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "Alpha Full".to_string(),
                diagram_x: 0.0,
                diagram_y: 4.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 24.0,
                owner_height: 20.0,
            },
            MermaidSemanticLine {
                text: "Second Line".to_string(),
                diagram_x: 0.0,
                diagram_y: 8.0,
                anchor: MermaidTextAnchor::Start,
                kind: MermaidSemanticKind::NodeTitle,
                owner_key: "node:A".to_string(),
                outline_eligible: false,
                owner_width: 24.0,
                owner_height: 20.0,
            },
        ],
        MermaidViewportTransform {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        content_rect,
        MermaidViewState::L2,
    );

    assert_eq!(
        projected
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>(),
        vec!["Alpha Full".to_string(), "Second Line".to_string()]
    );
}

#[test]
fn mermaid_detail_box_rects_wrap_visible_lines_tightly() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 20,
    };
    let source_lines = vec![
        MermaidSemanticLine {
            text: "USER".to_string(),
            diagram_x: 0.0,
            diagram_y: 0.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::NodeTitle,
            owner_key: "node:USER".to_string(),
            outline_eligible: false,
            owner_width: 20.0,
            owner_height: 20.0,
        },
        MermaidSemanticLine {
            text: "id".to_string(),
            diagram_x: 0.0,
            diagram_y: 0.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::ErAttributeName,
            owner_key: "node:USER".to_string(),
            outline_eligible: false,
            owner_width: 20.0,
            owner_height: 20.0,
        },
        MermaidSemanticLine {
            text: "email".to_string(),
            diagram_x: 0.0,
            diagram_y: 0.0,
            anchor: MermaidTextAnchor::Start,
            kind: MermaidSemanticKind::ErAttributeName,
            owner_key: "node:USER".to_string(),
            outline_eligible: false,
            owner_width: 20.0,
            owner_height: 20.0,
        },
    ];
    let projected = vec![
        MermaidProjectedLine {
            source_index: 0,
            x: 20,
            y: 11,
            text: "USER".to_string(),
            color: MERMAID_BODY_COLOR,
        },
        MermaidProjectedLine {
            source_index: 1,
            x: 18,
            y: 12,
            text: "id".to_string(),
            color: MERMAID_BODY_COLOR,
        },
        MermaidProjectedLine {
            source_index: 2,
            x: 18,
            y: 13,
            text: "email".to_string(),
            color: MERMAID_BODY_COLOR,
        },
    ];

    let rects = mermaid_detail_box_rects(&source_lines, &projected, content_rect);
    assert_eq!(
        rects.get("node:USER").copied(),
        Some(MermaidOutlineLabelRect {
            left: 17,
            right: 24,
            top: 10,
            bottom: 14,
        })
    );
}

#[test]
fn mermaid_packed_detail_rects_center_cluster_within_viewport() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 60,
        height: 20,
    };
    let owners = vec![
        MermaidPackedDetailOwner {
            owner_key: "node:a".to_string(),
            sort_x: 48,
            sort_y: 1,
            lines: vec![MermaidPackedDetailLine {
                source_index: 0,
                text: "SSH as sandbox user".to_string(),
                color: MERMAID_BODY_COLOR,
                kind: MermaidSemanticKind::NodeTitle,
            }],
        },
        MermaidPackedDetailOwner {
            owner_key: "node:b".to_string(),
            sort_x: 50,
            sort_y: 5,
            lines: vec![
                MermaidPackedDetailLine {
                    source_index: 1,
                    text: "skillbox-login.sh".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 2,
                    text: "ForceCommand".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
            ],
        },
        MermaidPackedDetailOwner {
            owner_key: "node:c".to_string(),
            sort_x: 48,
            sort_y: 10,
            lines: vec![
                MermaidPackedDetailLine {
                    source_index: 3,
                    text: "tailscale whois".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 4,
                    text: "identity resolution".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
            ],
        },
        MermaidPackedDetailOwner {
            owner_key: "node:d".to_string(),
            sort_x: 50,
            sort_y: 15,
            lines: vec![
                MermaidPackedDetailLine {
                    source_index: 5,
                    text: "SKILLBOX_DEV".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 6,
                    text: "GIT_AUTHOR_NAME".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
                MermaidPackedDetailLine {
                    source_index: 7,
                    text: "GIT_AUTHOR_EMAIL".to_string(),
                    color: MERMAID_BODY_COLOR,
                    kind: MermaidSemanticKind::NodeTitle,
                },
            ],
        },
    ];

    let rects = mermaid_pack_detail_box_rects(content_rect, &owners);
    assert_eq!(rects.len(), owners.len());

    let left = rects.values().map(|rect| rect.left).min().expect("left");
    let right = rects.values().map(|rect| rect.right).max().expect("right");
    let top = rects.values().map(|rect| rect.top).min().expect("top");
    let bottom = rects
        .values()
        .map(|rect| rect.bottom)
        .max()
        .expect("bottom");
    let center_x = (left + right) / 2;
    let center_y = (top + bottom) / 2;
    let expected_x = i32::from(content_rect.x + content_rect.width / 2);
    let expected_y = i32::from(content_rect.y + content_rect.height / 2);
    assert!((center_x - expected_x).abs() <= 2);
    assert!((center_y - expected_y).abs() <= 2);
    assert!(right - left >= i32::from(content_rect.width / 3));
}

#[test]
fn mermaid_er_detail_view_draws_compact_box_around_visible_lines() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..2 {
        press_mermaid_key(&mut app, layout, '+');
    }
    app.render(&mut renderer, layout);

    let user = find_text_position(&renderer, "USER").expect("USER label");
    let id = find_text_position(&renderer, "id PK").expect("id label");
    let email = find_text_position(&renderer, "email").expect("email label");

    assert_eq!(id.1, user.1 + 1);
    assert_eq!(email.1, id.1 + 1);

    let left = user.0.min(id.0).min(email.0).saturating_sub(1);
    let right = (user.0 + display_width("USER") - 1)
        .max(id.0 + display_width("id PK") - 1)
        .max(email.0 + display_width("email") - 1)
        .saturating_add(1);

    assert_eq!(cell_at(&renderer, left, user.1).ch, '|');
    assert_eq!(cell_at(&renderer, left, id.1).ch, '|');
    assert_eq!(cell_at(&renderer, left, email.1).ch, '|');
    assert_eq!(cell_at(&renderer, right, user.1).ch, '|');
    assert_eq!(cell_at(&renderer, right, id.1).ch, '|');
    assert_eq!(cell_at(&renderer, right, email.1).ch, '|');
    assert_eq!(
        cell_at(&renderer, left + 1, user.1.saturating_sub(1)).ch,
        '_'
    );
    assert_eq!(
        cell_at(&renderer, left + 1, email.1.saturating_add(1)).ch,
        '_'
    );
}

#[test]
fn mermaid_flowchart_detail_l2_packs_boxes_to_use_viewport() {
    let source = "graph TD\nA[SSH as sandbox user] -->|triggers| B[skillbox-login.sh]\nB -->|runs| C[tailscale whois]\nC -->|sets| D[SKILLBOX_DEV]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);

    let (bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    let center_x = (bounds.0 + bounds.1) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 2);
}

#[test]
fn mermaid_er_semantic_columns_cap_type_to_name_gap_at_three_spaces() {
    let node = mermaid_rs_renderer::NodeLayout {
        id: "ITEM".to_string(),
        x: 10.0,
        y: 10.0,
        width: 140.0,
        height: 80.0,
        label: mermaid_rs_renderer::layout::TextBlock {
            lines: vec![
                "ITEM".to_string(),
                "---".to_string(),
                "uuid id PK".to_string(),
                "decimal total".to_string(),
                "bool open".to_string(),
            ],
            width: 0.0,
            height: 0.0,
        },
        shape: mermaid_rs_renderer::ir::NodeShape::Rectangle,
        style: mermaid_rs_renderer::ir::NodeStyle::default(),
        link: None,
        anchor_subgraph: None,
        hidden: false,
        icon: None,
    };
    let mut semantic_lines = Vec::new();
    extend_mermaid_er_semantic_lines(
        &mut semantic_lines,
        &node,
        10.0,
        14.0,
        10.0,
        "node:ITEM",
        true,
    );

    let projected = project_mermaid_semantic_lines(
        &semantic_lines,
        MermaidViewportTransform {
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        Rect {
            x: 0,
            y: 0,
            width: 120,
            height: 32,
        },
        MermaidViewState::L3,
    );

    let x_for = |needle: &str| -> u16 {
        projected
            .iter()
            .find(|line| line.text == needle)
            .map(|line| line.x)
            .unwrap_or_else(|| panic!("{needle}"))
    };

    let uuid = x_for("uuid");
    let id = x_for("id PK");
    let decimal = x_for("decimal");
    let total = x_for("total");
    let bool_pos = x_for("bool");
    let open = x_for("open");

    assert_eq!(id, uuid + display_width("uuid") + 3);
    assert_eq!(total, decimal + display_width("decimal") + 3);
    assert_eq!(open, bool_pos + display_width("bool") + 3);
}

#[test]
fn mermaid_resize_reprojects_semantic_labels() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);
    let group_before = find_text_position(&renderer, "Group One").expect("Group One before");

    let resized_layout = test_layout(160, 48);
    let mut resized_renderer = test_renderer(160, 48);
    app.render(&mut resized_renderer, resized_layout);

    let group_after = find_text_position(&resized_renderer, "Group One").expect("Group One after");
    assert_ne!(group_after, group_before);
    assert!(find_text_position(&resized_renderer, "Producer").is_none());
}

#[test]
fn mermaid_resize_preserves_focused_semantic_target() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);
    let (focused_before, focus_status_before) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.focused_source_index, viewer.focus_status.clone()),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    let resized_layout = test_layout(160, 48);
    let mut resized_renderer = test_renderer(160, 48);
    app.render(&mut resized_renderer, resized_layout);

    let (focused_after, focus_status, highlighted_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focused_source_index,
            viewer.focus_status.clone(),
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| Some(line.source_index) == viewer.focused_source_index)
                .map(|line| (line.x, line.y))
                .expect("focused semantic line after resize"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_after, focused_before);
    assert_eq!(focus_status, focus_status_before);
    assert_eq!(
        cell_at(
            &resized_renderer,
            highlighted_position.0,
            highlighted_position.1
        )
        .fg,
        MERMAID_FOCUS_COLOR
    );
}

#[test]
fn mermaid_pan_and_zoom_preserve_focused_target() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    let focused_before = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.focused_source_index,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    press_mermaid_key(&mut app, layout, '+');
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);

    let focused_after = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.focused_source_index,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_after, focused_before);
    assert!(row_text(&renderer, layout.overview_field.y).contains("zoom 150%"));
    assert!(row_text(&renderer, layout.overview_field.y).contains("focus Alpha Node"));
}

#[test]
fn mermaid_open_shortcut_uses_artifact_path_and_stays_in_viewer() {
    let api = MockApi::new();
    let opener = Arc::new(MockArtifactOpener::default());
    let layout = test_layout(120, 32);
    let mut app = make_app_with_artifact_opener(api, opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));

    assert_eq!(
        opener.calls(),
        vec!["/tmp/repos/swimmers/flow.mmd".to_string()]
    );
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Mermaid(_)));
    assert_eq!(app.visible_message(), Some("open artifact -> flow.mmd"));
}

#[test]
fn mermaid_open_shortcut_reports_failures_and_missing_paths() {
    let api = MockApi::new();
    let opener = Arc::new(MockArtifactOpener::default());
    opener.fail_with("boom");
    let layout = test_layout(120, 32);
    let mut app = make_app_with_artifact_opener(api, opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));
    assert_eq!(app.visible_message(), Some("failed to open artifact: boom"));
    assert_eq!(
        opener.calls(),
        vec!["/tmp/repos/swimmers/flow.mmd".to_string()]
    );

    let opener = Arc::new(MockArtifactOpener::default());
    let mut app = make_app_with_artifact_opener(MockApi::new(), opener.clone());
    app.merge_sessions(
        vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)],
        layout.overview_field,
    );
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );
    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.path = None;

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));
    assert_eq!(opener.calls(), Vec::<String>::new());
    assert_eq!(app.visible_message(), Some("artifact path unavailable"));
}

#[test]
fn mermaid_open_shortcut_resolves_readme_from_repo_root() {
    let repo = tempdir().expect("tempdir");
    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("write cargo");
    fs::write(repo.path().join("README.md"), "# Demo\n").expect("write readme");
    fs::create_dir_all(repo.path().join("plans").join("draft").join("slice"))
        .expect("create plan dir");
    let schema_path = repo
        .path()
        .join("plans")
        .join("draft")
        .join("slice")
        .join("schema.mmd");
    fs::write(&schema_path, "graph TD\nA-->B\n").expect("write schema");

    let api = MockApi::new();
    let opener = Arc::new(MockArtifactOpener::default());
    let layout = test_layout(120, 32);
    let mut app = make_app_with_artifact_opener(api, opener.clone());
    app.merge_sessions(
        vec![session_summary(
            "sess-1",
            "7",
            repo.path().join("src").to_string_lossy().as_ref(),
        )],
        layout.overview_field,
    );
    let mut artifact = mermaid_artifact(
        "sess-1",
        &schema_path.to_string_lossy(),
        "2026-03-23T10:05:00Z",
        "graph TD\nA-->B\n",
    );
    artifact.plan_files = Some(vec!["README.md".to_string()]);
    app.mermaid_artifacts.insert("sess-1".to_string(), artifact);

    app.open_mermaid_viewer("sess-1".to_string());
    app.switch_plan_tab(DomainPlanTab::Readme);
    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    ));

    assert_eq!(
        opener.calls(),
        vec![repo.path().join("README.md").to_string_lossy().into_owned()]
    );
    assert_eq!(app.visible_message(), Some("open artifact -> README.md"));
}
