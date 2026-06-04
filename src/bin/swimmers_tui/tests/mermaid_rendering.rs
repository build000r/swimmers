use super::*;

#[test]
fn mermaid_clicking_sequence_diagram_does_not_create_focus() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);
    app.render(&mut renderer, layout);
    let content_rect = mermaid_content_rect(layout.overview_field);
    let (column, row) = find_blank_position(&renderer, content_rect).expect("blank sequence cell");

    assert!(app.handle_mermaid_mouse_down(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
    ));

    let (focused_source_index, focus_status, render_error) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focused_source_index,
            viewer.focus_status.clone(),
            viewer.render_error.clone(),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_source_index, None);
    assert_eq!(focus_status, None);
    assert_eq!(render_error, None);
}

#[test]
fn mermaid_render_reuses_prepared_source_state_across_zoom_and_pan() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api);
    let mut renderer = test_renderer(120, 32);
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
    viewer.unsupported_reason = None;

    app.render(&mut renderer, layout);
    let (prepare_after_first, viewport_after_first, first_lines_empty) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.source_prepare_count,
            viewer.viewport_render_count,
            viewer.cached_lines.is_empty(),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_first, 1);
    assert_eq!(viewport_after_first, 1);
    assert!(!first_lines_empty);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);
    let (prepare_after_zoom, viewport_after_zoom) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => {
            (viewer.source_prepare_count, viewer.viewport_render_count)
        }
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_zoom, 1);
    assert_eq!(viewport_after_zoom, 2);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);
    let (prepare_after_pan, viewport_after_pan) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => {
            (viewer.source_prepare_count, viewer.viewport_render_count)
        }
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_pan, 1);
    assert_eq!(viewport_after_pan, 3);
}

#[test]
fn mermaid_refresh_invalidates_prepared_source_state_when_artifact_changes() {
    let api = MockApi::new();
    let layout = test_layout(120, 32);
    let mut app = make_app(api.clone());
    let mut renderer = test_renderer(120, 32);
    let sessions = vec![session_summary("sess-1", "7", TEST_REPO_SWIMMERS)];
    app.merge_sessions(sessions.clone(), layout.overview_field);
    app.mermaid_artifacts.insert(
        "sess-1".to_string(),
        mermaid_artifact(
            "sess-1",
            "/tmp/repos/swimmers/flow-a.mmd",
            "2026-03-23T10:05:00Z",
            "graph TD\nA-->B\n",
        ),
    );

    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = None;

    app.render(&mut renderer, layout);
    let prepare_after_first = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.source_prepare_count,
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_first, 1);

    api.push_mermaid_artifact(Ok(mermaid_artifact(
        "sess-1",
        "/tmp/repos/swimmers/flow-b.mmd",
        "2026-03-23T10:06:00Z",
        "graph TD\nA-->C\n",
    )));
    app.refresh_mermaid_artifacts(&sessions);
    app.render(&mut renderer, layout);

    let (prepare_after_refresh, refreshed_path) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.source_prepare_count,
            viewer.path.as_deref().map(str::to_string),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(prepare_after_refresh, 2);
    assert_eq!(
        refreshed_path.as_deref(),
        Some("/tmp/repos/swimmers/flow-b.mmd")
    );
}

#[test]
fn mermaid_build_semantic_lines_early_returns_for_unsupported_kind() {
    // Sequence diagrams are not in the supported list → exercises the
    // mermaid_kind_supports_semantic_overlay early-return branch.
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 30,
    };
    let options = mermaid_render_options(content_rect);
    let source = "sequenceDiagram\n  Alice ->> Bob: Hello\n  Bob -->> Alice: Hi\n";
    let parsed = parse_mermaid(source).expect("parse");
    let layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
    let lines = build_mermaid_semantic_lines(&layout, &options);
    assert!(
        lines.is_empty(),
        "sequence diagram should yield no semantic lines"
    );
}

#[test]
fn mermaid_state_diagram_renders_without_panic() {
    // Exercises DiagramKind::State paths in build_mermaid_semantic_lines:
    // State subgraph label (header_height / label_x), State node font/line_height,
    // and __start_/__end_ node hiding.
    let source = "stateDiagram-v2\n  [*] --> Still\n  Still --> Moving\n  Moving --> Crash\n  Crash --> [*]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    app.render(&mut renderer, layout);
    // Diagram rendered successfully; state labels appear as overlay text
    assert!(matches!(&app.fish_bowl_mode, FishBowlMode::Mermaid(v) if v.prepared_render.is_some()));
}

#[test]
fn mermaid_state_diagram_with_edge_labels_exercises_state_edge_font_path() {
    // State diagram with labeled transitions → exercises the DiagramKind::State
    // branch inside the edge loop (state_font_size / state_line_height selection).
    let source = "stateDiagram-v2\n  [*] --> Active : start\n  Active --> Inactive : stop\n  Inactive --> [*]\n";
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 30,
    };
    let options = mermaid_render_options(content_rect);
    let parsed = parse_mermaid(source).expect("parse");
    let layout = compute_layout(&parsed.graph, &options.theme, &options.layout);
    let lines = build_mermaid_semantic_lines(&layout, &options);
    // State transitions with labels produce EdgeLabel semantic lines
    assert!(lines
        .iter()
        .any(|l| matches!(l.kind, MermaidSemanticKind::EdgeLabel)));
}

#[test]
fn mermaid_state_diagram_with_compound_state_renders_subgraph_label() {
    // Compound states produce subgraphs in the layout → exercises the
    // DiagramKind::State subgraph branch (header_height, label_x).
    let source = "stateDiagram-v2\n  state \"Running\" as running {\n    [*] --> Start\n    Start --> End\n  }\n  [*] --> running\n  running --> [*]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    app.render(&mut renderer, layout);
    assert!(matches!(&app.fish_bowl_mode, FishBowlMode::Mermaid(v) if v.prepared_render.is_some()));
}

#[test]
fn mermaid_class_diagram_with_methods_renders_divider_lines() {
    // Class diagrams with methods produce divider lines in node labels →
    // exercises the extend_mermaid_class_semantic_lines branch.
    let source = "classDiagram\n  class Animal {\n    +String name\n    +makeSound() void\n  }\n  class Dog {\n    +fetch() void\n  }\n  Animal <|-- Dog\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    app.render(&mut renderer, layout);
    assert!(matches!(&app.fish_bowl_mode, FishBowlMode::Mermaid(v) if v.prepared_render.is_some()));
}

#[test]
fn mermaid_flowchart_with_subgraph_renders_subgraph_label() {
    // Subgraph with a label exercises the non-State subgraph code path
    // (label_x = subgraph.x + subgraph.width / 2, push_mermaid_summary_line).
    let source =
        "graph TD\n  subgraph cluster[\"My Cluster\"]\n    A --> B\n  end\n  C --> cluster\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    app.render(&mut renderer, layout);
    assert!(matches!(&app.fish_bowl_mode, FishBowlMode::Mermaid(v) if v.prepared_render.is_some()));
}

#[test]
fn mermaid_flowchart_edge_with_label_exercises_edge_label_path() {
    // An edge with a label exercises the edge label anchor branch
    // (push_mermaid_text_block_semantic_lines for EdgeLabel kind).
    let source = "graph LR\n  A -->|transfer data| B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    app.render(&mut renderer, layout);
    assert!(matches!(&app.fish_bowl_mode, FishBowlMode::Mermaid(v) if v.prepared_render.is_some()));
}

#[test]
fn mermaid_graph_node_labels_render_as_terminal_text() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    app.render(&mut renderer, layout);

    let alpha = find_text_position(&renderer, "Alpha Node").expect("Alpha Node overlay");
    let beta = find_text_position(&renderer, "Beta Node").expect("Beta Node overlay");
    assert_eq!(cell_at(&renderer, alpha.0, alpha.1).ch, 'A');
    assert_eq!(cell_at(&renderer, beta.0, beta.1).ch, 'B');
    assert!(row_text(&renderer, layout.overview_field.y).contains("outline"));
}

#[test]
fn mermaid_outline_background_stays_sparse_for_simple_flowchart() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    app.render(&mut renderer, layout);

    let background_chars = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => mermaid_background_charset(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        !background_chars.is_empty(),
        "outline should draw connectors"
    );
    assert!(
        background_chars.len() < 40,
        "outline background should stay sparse: {background_chars:?}"
    );
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '>' | '<')),
        "{background_chars:?}"
    );
}

#[test]
fn mermaid_merge_outline_segments_coalesces_overlapping_ranges() {
    let merged = mermaid_merge_outline_segments(&[
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Horizontal,
            fixed: 8,
            start: 10,
            end: 16,
        },
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Horizontal,
            fixed: 8,
            start: 14,
            end: 22,
        },
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Vertical,
            fixed: 30,
            start: 4,
            end: 7,
        },
        MermaidOutlineSegment {
            axis: MermaidOutlineAxis::Vertical,
            fixed: 30,
            start: 8,
            end: 11,
        },
    ]);

    assert_eq!(
        merged,
        vec![
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Horizontal,
                fixed: 8,
                start: 10,
                end: 22,
            },
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Vertical,
                fixed: 30,
                start: 4,
                end: 11,
            },
        ]
    );
}

#[test]
fn mermaid_outline_route_keeps_segment_order_arrow_and_label_avoidance() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 12,
    };
    let from = MermaidOutlineNode {
        key: "node:left".to_string(),
        source_index: 0,
        x: 2,
        y: 2,
        text_width: 4,
    };
    let to = MermaidOutlineNode {
        key: "node:right".to_string(),
        source_index: 1,
        x: 26,
        y: 8,
        text_width: 5,
    };
    let edge = MermaidOutlineEdge {
        from_key: from.key.clone(),
        to_key: to.key.clone(),
        directed: true,
    };
    let mut reserved_segments = Vec::new();
    let mut lane_cache_vertical = HashMap::new();
    let mut lane_cache_horizontal = HashMap::new();
    let mut label_rects = mermaid_outline_label_rects(&[from.clone(), to.clone()]);
    label_rects.insert(
        "node:blocker".to_string(),
        MermaidOutlineLabelRect {
            left: 15,
            right: 15,
            top: 2,
            bottom: 8,
        },
    );

    let (segments, arrow) = mermaid_plan_outline_route(
        content_rect,
        &edge,
        &from,
        &to,
        &mut reserved_segments,
        &mut lane_cache_vertical,
        &mut lane_cache_horizontal,
        &label_rects,
    );

    assert_eq!(
        segments,
        vec![
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Horizontal,
                fixed: 2,
                start: 6,
                end: 13,
            },
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Vertical,
                fixed: 13,
                start: 2,
                end: 8,
            },
            MermaidOutlineSegment {
                axis: MermaidOutlineAxis::Horizontal,
                fixed: 8,
                start: 13,
                end: 25,
            },
        ]
    );
    assert_eq!(
        arrow,
        Some(MermaidOutlineArrow {
            x: 25,
            y: 8,
            ch: '>',
        })
    );
    assert_eq!(reserved_segments, segments);
}

#[test]
fn mermaid_outline_background_coalesces_duplicate_edges() {
    let content_rect = Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 12,
    };
    let nodes = vec![
        MermaidOutlineNode {
            key: "node:left".to_string(),
            source_index: 0,
            x: 2,
            y: 2,
            text_width: 4,
        },
        MermaidOutlineNode {
            key: "node:right".to_string(),
            source_index: 1,
            x: 26,
            y: 8,
            text_width: 5,
        },
    ];
    let single = mermaid_render_outline_background(
        content_rect,
        &nodes,
        [MermaidOutlineEdge {
            from_key: "node:left".to_string(),
            to_key: "node:right".to_string(),
            directed: true,
        }],
    );
    let duplicated = mermaid_render_outline_background(
        content_rect,
        &nodes,
        [
            MermaidOutlineEdge {
                from_key: "node:left".to_string(),
                to_key: "node:right".to_string(),
                directed: true,
            },
            MermaidOutlineEdge {
                from_key: "node:left".to_string(),
                to_key: "node:right".to_string(),
                directed: true,
            },
        ],
    );

    assert_eq!(duplicated, single);
}

#[test]
fn mermaid_tab_focuses_first_visible_semantic_target_and_highlights_it() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    let (focus_status, focused_source_index, alpha_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            viewer.focused_source_index,
            find_cached_semantic_line(viewer, "Alpha Node").expect("Alpha Node overlay"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Alpha Node"));
    assert!(focused_source_index.is_some());
    assert_eq!(
        cell_at(&renderer, alpha_position.0, alpha_position.1).fg,
        MERMAID_FOCUS_COLOR
    );
}

#[test]
fn mermaid_tab_cycles_forward_and_back_between_visible_targets() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);

    press_mermaid_tab(&mut app, layout);
    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);
    let (focus_status, beta_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            find_cached_semantic_line(viewer, "Beta Node").expect("Beta Node overlay"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Beta Node"));
    assert_eq!(
        cell_at(&renderer, beta_position.0, beta_position.1).fg,
        MERMAID_FOCUS_COLOR
    );

    press_mermaid_backtab(&mut app, layout);
    app.render(&mut renderer, layout);
    let focus_status = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.focus_status.clone(),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focus_status.as_deref(), Some("focus Alpha Node"));
}

#[test]
fn mermaid_er_entities_state_shows_only_entity_names_and_is_centered() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let (semantic_texts, bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(semantic_texts.contains(&"ORDER".to_string()));
    assert!(!semantic_texts.contains(&"email".to_string()));
    assert!(!semantic_texts.contains(&"user_id".to_string()));
    assert!(!semantic_texts.contains(&"uuid".to_string()));
    assert!(!semantic_texts.contains(&"places".to_string()));
    let center_x = (bounds.0 + bounds.1) / 2;
    let center_y = (bounds.2 + bounds.3) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    let expected_y = content_rect.y + content_rect.height / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 2);
    assert!((center_y as i32 - expected_y as i32).abs() <= 1);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
}

#[test]
fn mermaid_flowchart_overview_hides_edge_labels_until_zoomed() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    assert!(find_text_position(&renderer, "Group One").is_some());
    assert!(find_text_position(&renderer, "Producer").is_none());
    assert!(find_text_position(&renderer, "Consumer").is_none());
    assert!(find_text_position(&renderer, "ships data").is_none());
    assert!(row_text(&renderer, layout.overview_field.y).contains("outline"));
}

#[test]
fn mermaid_outline_collapses_subgraph_edges_to_top_level_groups() {
    let source = "graph LR\nsubgraph Left Side\nA[Alpha]\nB[Beta]\nend\nsubgraph Right Side\nC[Gamma]\nD[Delta]\nend\nA --> C\nB --> D\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 140, 36);

    app.render(&mut renderer, layout);

    let (semantic_texts, background_chars) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_background_charset(viewer),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(
        semantic_texts,
        vec!["Left Side".to_string(), "Right Side".to_string()]
    );
    assert!(find_text_position(&renderer, "Alpha").is_none());
    assert!(find_text_position(&renderer, "Beta").is_none());
    assert!(find_text_position(&renderer, "Gamma").is_none());
    assert!(find_text_position(&renderer, "Delta").is_none());
    assert!(
        background_chars
            .iter()
            .any(|ch| matches!(ch, '_' | '>' | '<')),
        "{background_chars:?}"
    );
}

#[test]
fn mermaid_flowchart_overview_compacts_long_node_labels() {
    let source = "graph TD\nA[1. Verified Identity And api cfo admin hierarchy role restricted]\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let (semantic_texts, background_chars) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_background_charset(viewer),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.iter().any(|text| text.starts_with("1. Ver")),
        "{semantic_texts:?}"
    );
    assert!(!semantic_texts.iter().any(|text| text.contains("hierarchy")));
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '\\' | '>' | '<')),
        "{background_chars:?}"
    );
    assert!(row_text(&renderer, layout.overview_field.y).contains("outline"));
}

#[test]
fn mermaid_er_overview_shows_compact_entity_words_without_svg_text_noise() {
    let source = "erDiagram\ngoverned_revision_artifacts {\n  uuid id PK\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);

    let (semantic_texts, background_chars) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            cached_semantic_texts(viewer),
            mermaid_background_charset(viewer),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts
            .iter()
            .any(|text| text == "governed revision"),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts
            .iter()
            .any(|text| text.contains("governed_revision_artifacts")),
        "{semantic_texts:?}"
    );
    assert!(
        background_chars
            .iter()
            .all(|ch| matches!(ch, '|' | '_' | '\\' | '>' | '<')),
        "{background_chars:?}"
    );
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
}

#[test]
fn mermaid_detail_projection_suppresses_edge_labels_in_compact_views() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        find_text_position(&renderer, "ships data").is_none(),
        "status row: {}; semantic_texts: {:?}",
        row_text(&renderer, layout.overview_field.y),
        semantic_texts
    );
    assert!(find_text_position(&renderer, "Producer").is_some());
    assert!(find_text_position(&renderer, "Consumer").is_some());
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("detail L2"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("zoom 150%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_tab_focuses_visible_owner_labels_in_detail_l2() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    let (focus_status, producer_position) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.focus_status.clone(),
            find_cached_semantic_line(viewer, "Producer").expect("Producer overlay"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L2"));
    assert_eq!(focus_status.as_deref(), Some("focus Producer"));
    assert_eq!(
        cell_at(&renderer, producer_position.0, producer_position.1).fg,
        MERMAID_FOCUS_COLOR
    );
    assert!(find_text_position(&renderer, "ships data").is_none());
}

#[test]
fn mermaid_escape_clears_focus_before_closing() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    press_mermaid_tab(&mut app, layout);
    app.render(&mut renderer, layout);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    let (focused_source_index, focus_status) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (viewer.focused_source_index, viewer.focus_status.clone()),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(focused_source_index, None);
    assert_eq!(focus_status, None);

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    ));
    assert!(matches!(app.fish_bowl_mode, FishBowlMode::Aquarium));
}

#[test]
fn mermaid_er_schema_uses_smart_colors_for_titles_types_and_connectors() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..3 {
        scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    }
    app.render(&mut renderer, layout);

    let (background_colors, user_owner_key, order_owner_key, owner_colors) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                mermaid_background_colors_set(viewer),
                mermaid_owner_key_for_text(viewer, "USER"),
                mermaid_owner_key_for_text(viewer, "ORDER"),
                mermaid_owner_accent_map(
                    &viewer
                        .prepared_render
                        .as_ref()
                        .expect("prepared render")
                        .semantic_lines,
                ),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

    let user_accent = mermaid_owner_accent_color(&user_owner_key, &owner_colors);
    let order_accent = mermaid_owner_accent_color(&order_owner_key, &owner_colors);
    assert_eq!(mermaid_text_color(&renderer, "USER"), user_accent);
    assert_eq!(mermaid_text_color(&renderer, "ORDER"), order_accent);
    assert_ne!(user_accent, order_accent);
    assert_eq!(mermaid_border_color(&renderer, "USER"), user_accent);
    assert_eq!(mermaid_border_color(&renderer, "ORDER"), order_accent);
    assert_eq!(mermaid_text_color(&renderer, "uuid"), MERMAID_TYPE_COLOR);
    assert_eq!(mermaid_text_color(&renderer, "email"), MERMAID_BODY_COLOR);
    assert_eq!(
        mermaid_text_color(&renderer, "user_id FK"),
        MERMAID_BODY_COLOR
    );
    assert!(background_colors.contains(&format!("{MERMAID_CONNECTOR_COLOR:?}")));
}

#[test]
fn mermaid_flowchart_detail_uses_smart_colors_for_titles_labels_and_connectors() {
    let source =
        "graph TD\nsubgraph Group One\nA[Producer]\nB[Consumer]\nend\nA -- ships data --> B\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    press_mermaid_key(&mut app, layout, '+');
    app.render(&mut renderer, layout);

    let (producer_owner_key, consumer_owner_key, background_colors, owner_colors) =
        match &app.fish_bowl_mode {
            FishBowlMode::Mermaid(viewer) => (
                mermaid_owner_key_for_text(viewer, "Producer"),
                mermaid_owner_key_for_text(viewer, "Consumer"),
                mermaid_background_colors_set(viewer),
                mermaid_owner_accent_map(
                    &viewer
                        .prepared_render
                        .as_ref()
                        .expect("prepared render")
                        .semantic_lines,
                ),
            ),
            FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
        };

    assert_eq!(
        mermaid_text_color(&renderer, "Producer"),
        mermaid_owner_accent_color(&producer_owner_key, &owner_colors)
    );
    assert_eq!(
        mermaid_text_color(&renderer, "Consumer"),
        mermaid_owner_accent_color(&consumer_owner_key, &owner_colors)
    );
    assert_eq!(
        mermaid_border_color(&renderer, "Producer"),
        mermaid_owner_accent_color(&producer_owner_key, &owner_colors)
    );
    assert_eq!(find_text_position(&renderer, "ships data"), None);
    assert!(!background_colors.is_empty());
    assert!(row_text(&renderer, layout.overview_field.y).contains("detail L2"));
}

#[test]
fn mermaid_sequence_diagram_connector_fallback_uses_dark_grey_cells() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("sequenceDiagram\nAlice->>Bob: hello\n", 120, 32);

    app.render(&mut renderer, layout);

    let background_colors = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => mermaid_background_colors(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(!background_colors.is_empty());
    assert!(
        background_colors
            .iter()
            .all(|color| *color == MERMAID_CONNECTOR_COLOR),
        "{background_colors:?}"
    );
}

#[test]
fn mermaid_error_and_unsupported_states_keep_existing_colors() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.unsupported_reason =
            Some("inline Mermaid rendering is unsupported for TERM=dumb".to_string());
    }
    app.render(&mut renderer, layout);
    let unsupported = find_text_position(
        &renderer,
        "inline Mermaid rendering is unsupported for TERM=dumb",
    )
    .expect("unsupported text");
    assert_eq!(
        cell_at(&renderer, unsupported.0, unsupported.1).fg,
        Color::DarkGrey
    );
    assert_eq!(
        cell_at(&renderer, layout.overview_field.x, layout.overview_field.y).fg,
        Color::Cyan
    );

    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.artifact_error = Some("failed to parse mermaid artifact: bad source".to_string());
    }
    app.render(&mut renderer, layout);
    let artifact_error = find_text_position(&renderer, "failed to parse mermaid artifact")
        .expect("artifact error text");
    assert_eq!(
        cell_at(&renderer, artifact_error.0, artifact_error.1).fg,
        Color::Red
    );
}

#[test]
fn mermaid_viewport_cache_errors_render_text_and_set_render_error() {
    let (mut app, mut renderer, layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.source = None;
    }

    app.render(&mut renderer, layout);

    let render_error = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.render_error.clone(),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(render_error.as_deref(), Some("Mermaid source unavailable"));

    let error_text =
        find_text_position(&renderer, "Mermaid source unavailable").expect("viewport error text");
    assert_eq!(
        cell_at(&renderer, error_text.0, error_text.1).fg,
        Color::Red
    );
}

#[test]
fn mermaid_owner_accents_stay_stable_across_pan_and_zoom() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);
    let content_rect = mermaid_content_rect(layout.overview_field);

    app.render(&mut renderer, layout);
    let (user_before, order_before) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "USER")
                .map(|line| line.color)
                .expect("USER before"),
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "ORDER")
                .map(|line| line.color)
                .expect("ORDER before"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    app.zoom_mermaid_viewer(MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
    app.pan_mermaid_viewer(18.0, 12.0);
    app.render(&mut renderer, layout);

    let (user_after, order_after, prepare_count) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "USER")
                .map(|line| line.color)
                .expect("USER after"),
            viewer
                .cached_semantic_lines
                .iter()
                .find(|line| line.text == "ORDER")
                .map(|line| line.color)
                .expect("ORDER after"),
            viewer.source_prepare_count,
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };

    assert_eq!(user_after, user_before);
    assert_eq!(order_after, order_before);
    assert_eq!(prepare_count, 1);
}
