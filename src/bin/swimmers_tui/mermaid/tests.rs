use super::*;

fn test_renderer(width: u16, height: u16) -> Renderer {
    let buffer_size = (width as usize) * (height as usize);
    Renderer {
        stdout: BufWriter::new(io::stdout()),
        width,
        height,
        buffer: vec![Cell::default(); buffer_size],
        last_buffer: vec![Cell::default(); buffer_size],
        terminal_state: TerminalState::default(),
    }
}

fn test_viewer(
    cached_lines: Vec<&str>,
    cached_background_cells: Vec<Vec<Cell>>,
) -> MermaidViewerState {
    MermaidViewerState {
        session_id: "test-session".to_string(),
        tmux_name: "test-tmux".to_string(),
        tmux_target: swimmers::tmux_target::TmuxTarget::Default,
        cwd: ".".to_string(),
        path: None,
        source: None,
        artifact_error: None,
        render_error: None,
        unsupported_reason: None,
        zoom: 1.0,
        center_x: 0.5,
        center_y: 0.5,
        diagram_width: 0.0,
        diagram_height: 0.0,
        back_rect: None,
        content_rect: None,
        cached_rect: None,
        cached_zoom: 1.0,
        cached_center_x: 0.5,
        cached_center_y: 0.5,
        cached_lines: cached_lines.into_iter().map(str::to_string).collect(),
        cached_background_cells,
        cached_semantic_lines: Vec::new(),
        focused_source_index: None,
        focus_status: None,
        prepared_render: None,
        source_prepare_count: 0,
        viewport_render_count: 0,
        plan_tabs: None,
        active_tab: DomainPlanTab::Schema,
        inline_plan_files: BTreeMap::new(),
        plan_text_content: None,
        plan_text_lines: Vec::new(),
        plan_text_scroll: 0,
        plan_text_cached_width: 0,
        tab_rects: Vec::new(),
        disk_only: false,
    }
}

fn cell(ch: char, fg: Color) -> Cell {
    Cell { ch, fg }
}

fn rendered_cell(renderer: &Renderer, x: u16, y: u16) -> Cell {
    renderer.buffer[(y as usize) * (renderer.width as usize) + (x as usize)]
}

fn roomy_content_rect() -> Rect {
    Rect {
        x: 2,
        y: 3,
        width: MERMAID_VIEW_MIN_WIDTH + 20,
        height: MERMAID_VIEW_MIN_HEIGHT + 10,
    }
}

fn focus_line(y: u16, x: u16, source_index: usize) -> MermaidProjectedLine {
    MermaidProjectedLine {
        source_index,
        x,
        y,
        text: "candidate".to_string(),
        color: Color::Reset,
    }
}

fn focus_target(
    sort_y: u16,
    sort_x: u16,
    source_index: usize,
    priority: u8,
) -> MermaidFocusAccumulator {
    MermaidFocusAccumulator {
        source_index,
        text: "target".to_string(),
        diagram_x: 0.0,
        diagram_y: 0.0,
        priority,
        sort_y,
        sort_x,
        left: sort_x,
        right: sort_x,
        top: sort_y,
        bottom: sort_y,
    }
}

fn detail_semantic_line(owner_key: &str, kind: MermaidSemanticKind) -> MermaidSemanticLine {
    MermaidSemanticLine {
        text: "detail".to_string(),
        diagram_x: 10.0,
        diagram_y: 10.0,
        anchor: MermaidTextAnchor::Start,
        kind,
        owner_key: owner_key.to_string(),
        outline_eligible: true,
        owner_width: 40.0,
        owner_height: 20.0,
    }
}

fn detail_projected_line(source_index: usize) -> MermaidProjectedLine {
    MermaidProjectedLine {
        source_index,
        x: 6,
        y: 7,
        text: "detail".to_string(),
        color: Color::Reset,
    }
}

fn detail_label_rect(left: i32, right: i32, top: i32, bottom: i32) -> MermaidOutlineLabelRect {
    MermaidOutlineLabelRect {
        left,
        right,
        top,
        bottom,
    }
}

fn identity_transform() -> MermaidViewportTransform {
    MermaidViewportTransform {
        scale: 1.0,
        tx: 0.0,
        ty: 0.0,
    }
}

#[test]
fn mermaid_semantic_visibility_thresholds_are_inclusive() {
    let threshold_cases = [
        (MermaidSemanticKind::SubgraphSummary, 10.0, 1.0),
        (MermaidSemanticKind::NodeSummary, 8.0, 1.0),
        (MermaidSemanticKind::ClassMember, 10.0, 2.5),
        (MermaidSemanticKind::ErAttributeName, 8.0, 2.5),
        (MermaidSemanticKind::ErAttributeType, 12.0, 3.0),
    ];

    for (kind, min_cols, min_rows) in threshold_cases {
        assert!(
            kind.is_visible_for_owner(min_cols, min_rows),
            "{kind:?} should be visible at its threshold"
        );
        assert!(
            !kind.is_visible_for_owner(min_cols - 0.25, min_rows),
            "{kind:?} should hide below its column threshold"
        );
        assert!(
            !kind.is_visible_for_owner(min_cols, min_rows - 0.25),
            "{kind:?} should hide below its row threshold"
        );
    }
}

#[test]
fn mermaid_semantic_always_visible_kinds_ignore_owner_size() {
    for kind in [
        MermaidSemanticKind::SubgraphTitle,
        MermaidSemanticKind::NodeTitle,
        MermaidSemanticKind::EdgeLabel,
    ] {
        assert!(kind.is_visible_for_owner(-1.0, -1.0), "{kind:?}");
    }
}

#[test]
fn render_mermaid_detail_lines_errors_without_prepared_render() {
    let mut viewer = test_viewer(vec!["stale"], Vec::new());

    let err = render_mermaid_detail_lines(
        &mut viewer,
        roomy_content_rect(),
        identity_transform(),
        MermaidViewState::L1,
    )
    .unwrap_err();

    assert_eq!(err, "Mermaid source unavailable");
    assert_eq!(viewer.cached_lines, vec!["stale".to_string()]);
}

#[test]
fn mermaid_detail_parts_return_none_for_empty_projection() {
    let semantic_lines = vec![detail_semantic_line(
        "node:alpha",
        MermaidSemanticKind::NodeSummary,
    )];

    assert!(mermaid_detail_render_parts_from_projected(
        &semantic_lines,
        Vec::new(),
        roomy_content_rect(),
        MermaidViewState::L1,
    )
    .is_none());
}

#[test]
fn mermaid_detail_parts_require_packed_l2_l3_owners() {
    let semantic_lines = vec![detail_semantic_line(
        "edge:alpha:beta",
        MermaidSemanticKind::EdgeLabel,
    )];
    let projected = vec![detail_projected_line(0)];

    assert!(mermaid_detail_render_parts_from_projected(
        &semantic_lines,
        projected.clone(),
        roomy_content_rect(),
        MermaidViewState::L2,
    )
    .is_none());
    assert!(mermaid_detail_render_parts_from_projected(
        &semantic_lines,
        projected,
        roomy_content_rect(),
        MermaidViewState::L3,
    )
    .is_none());
}

#[test]
fn mermaid_detail_parts_return_none_for_empty_label_rects() {
    assert!(mermaid_detail_render_parts_from_label_rects(
        vec![detail_projected_line(0)],
        HashMap::new(),
    )
    .is_none());
}

#[test]
fn mermaid_detail_parts_pack_compact_l2_owner_lines() {
    let semantic_lines = vec![detail_semantic_line(
        "node:alpha",
        MermaidSemanticKind::NodeSummary,
    )];

    let parts = mermaid_detail_render_parts_from_projected(
        &semantic_lines,
        vec![detail_projected_line(0)],
        roomy_content_rect(),
        MermaidViewState::L2,
    )
    .expect("compact L2 owner should produce detail parts");

    assert!(parts.label_rects.contains_key("node:alpha"));
    assert_eq!(parts.projected.len(), 1);
    assert_eq!(parts.projected[0].source_index, 0);
}

#[test]
fn mermaid_filter_visible_outline_edges_requires_both_endpoint_keys() {
    let mut label_rects = HashMap::new();
    label_rects.insert("node:a".to_string(), detail_label_rect(0, 4, 0, 2));
    label_rects.insert("node:b".to_string(), detail_label_rect(6, 10, 0, 2));

    let visible_edges = mermaid_filter_visible_outline_edges(
        [
            MermaidOutlineEdge {
                from_key: "node:a".to_string(),
                to_key: "node:b".to_string(),
                directed: true,
            },
            MermaidOutlineEdge {
                from_key: "node:a".to_string(),
                to_key: "node:hidden".to_string(),
                directed: true,
            },
            MermaidOutlineEdge {
                from_key: "node:hidden".to_string(),
                to_key: "node:b".to_string(),
                directed: false,
            },
        ],
        &label_rects,
    );

    assert_eq!(visible_edges.len(), 1);
    assert_eq!(visible_edges[0].from_key, "node:a");
    assert_eq!(visible_edges[0].to_key, "node:b");
}

#[test]
fn mermaid_best_packed_detail_layout_prefers_highest_scoring_fit() {
    let specs = vec![
        MermaidPackedDetailBoxSize {
            outer_width: 4,
            outer_height: 3,
        },
        MermaidPackedDetailBoxSize {
            outer_width: 4,
            outer_height: 3,
        },
        MermaidPackedDetailBoxSize {
            outer_width: 4,
            outer_height: 3,
        },
        MermaidPackedDetailBoxSize {
            outer_width: 4,
            outer_height: 3,
        },
    ];
    let viewport = MermaidPackedDetailViewport {
        width: 20,
        height: 10,
        gap_x: 1,
        gap_y: 1,
    };

    let layout = mermaid_best_packed_detail_layout(&specs, viewport).expect("layout");

    assert_eq!(layout.column_count, 3);
    assert_eq!(layout.row_widths, vec![14, 4]);
    assert_eq!(layout.row_heights, vec![3, 3]);
    assert_eq!(layout.cluster_height, 7);
}

#[test]
fn mermaid_best_packed_detail_layout_returns_none_when_no_columns_fit() {
    let specs = vec![
        MermaidPackedDetailBoxSize {
            outer_width: 12,
            outer_height: 3,
        },
        MermaidPackedDetailBoxSize {
            outer_width: 12,
            outer_height: 3,
        },
    ];
    let viewport = MermaidPackedDetailViewport {
        width: 10,
        height: 10,
        gap_x: 1,
        gap_y: 1,
    };

    assert!(mermaid_best_packed_detail_layout(&specs, viewport).is_none());
}

#[test]
fn mermaid_special_renderer_maps_er_outline_and_detail_states() {
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::ErEntities),
        Some(MermaidSpecialRenderer::ErPacked)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::ErKeys),
        Some(MermaidSpecialRenderer::ErPacked)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::ErColumns),
        Some(MermaidSpecialRenderer::ErPacked)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::ErSchema),
        Some(MermaidSpecialRenderer::ErPacked)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::Outline),
        Some(MermaidSpecialRenderer::Outline)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::L1),
        Some(MermaidSpecialRenderer::Detail)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::L2),
        Some(MermaidSpecialRenderer::Detail)
    );
    assert_eq!(
        mermaid_special_renderer_for_view(MermaidViewState::L3),
        Some(MermaidSpecialRenderer::Detail)
    );
}

#[test]
fn mermaid_mark_viewport_cache_rendered_updates_metadata_and_saturates_count() {
    let mut viewer = test_viewer(Vec::new(), Vec::new());
    let content_rect = roomy_content_rect();
    viewer.zoom = 2.5;
    viewer.center_x = 25.0;
    viewer.center_y = 50.0;
    viewer.viewport_render_count = 41;

    mermaid_mark_viewport_cache_rendered(&mut viewer, content_rect);

    assert_eq!(viewer.cached_rect, Some(content_rect));
    assert_eq!(viewer.cached_zoom, 2.5);
    assert_eq!(viewer.cached_center_x, 25.0);
    assert_eq!(viewer.cached_center_y, 50.0);
    assert_eq!(viewer.viewport_render_count, 42);

    viewer.viewport_render_count = u64::MAX;
    mermaid_mark_viewport_cache_rendered(&mut viewer, content_rect);
    assert_eq!(viewer.viewport_render_count, u64::MAX);
}

#[test]
fn mermaid_viewer_body_state_preserves_branch_precedence() {
    let content_rect = roomy_content_rect();
    let mut viewer = test_viewer(Vec::new(), Vec::new());
    viewer.active_tab = DomainPlanTab::Plan;
    viewer.unsupported_reason = Some("unsupported".to_string());
    viewer.artifact_error = Some("artifact".to_string());

    assert_eq!(
        mermaid_viewer_body_state(&viewer, content_rect),
        MermaidViewerBodyState::PlanText
    );

    viewer.active_tab = DomainPlanTab::Schema;
    assert_eq!(
        mermaid_viewer_body_state(
            &viewer,
            Rect {
                width: MERMAID_VIEW_MIN_WIDTH - 1,
                ..content_rect
            },
        ),
        MermaidViewerBodyState::TooSmall
    );
    assert_eq!(
        mermaid_viewer_body_state(&viewer, content_rect),
        MermaidViewerBodyState::Unsupported("unsupported".to_string())
    );

    viewer.unsupported_reason = None;
    assert_eq!(
        mermaid_viewer_body_state(&viewer, content_rect),
        MermaidViewerBodyState::ArtifactError("artifact".to_string())
    );

    viewer.artifact_error = None;
    assert_eq!(
        mermaid_viewer_body_state(&viewer, content_rect),
        MermaidViewerBodyState::Diagram
    );
}

#[test]
fn mermaid_truncate_leaves_lines_unchanged_when_within_limit() {
    let mut lines = vec!["first".to_string(), "second".to_string()];

    assert!(!mermaid_truncate_lines_with_marker(&mut lines, 2));
    assert_eq!(lines, vec!["first".to_string(), "second".to_string()]);
}

#[test]
fn mermaid_truncate_zero_rows_clears_over_limit_lines() {
    let mut lines = vec!["first".to_string()];

    assert!(mermaid_truncate_lines_with_marker(&mut lines, 0));
    assert!(lines.is_empty());
}

#[test]
fn mermaid_truncate_replaces_final_retained_line_with_marker() {
    let mut lines = vec![
        "first".to_string(),
        "second".to_string(),
        "third".to_string(),
    ];

    assert!(mermaid_truncate_lines_with_marker(&mut lines, 2));
    assert_eq!(
        lines,
        vec!["first".to_string(), MERMAID_TRUNCATED_MARKER.to_string()]
    );
}

#[test]
fn mermaid_focus_lower_priority_label_wins() {
    let line = focus_line(20, 20, 20);
    let target = focus_target(1, 1, 1, 3);

    assert!(mermaid_focus_line_has_better_label(&line, &target, 2));
    assert!(!mermaid_focus_line_has_better_label(&line, &target, 4));
}

#[test]
fn mermaid_focus_tied_priority_uses_position_then_source_index() {
    let target = focus_target(5, 5, 5, 2);

    assert!(mermaid_focus_line_has_better_label(
        &focus_line(4, 99, 99),
        &target,
        2
    ));
    assert!(mermaid_focus_line_has_better_label(
        &focus_line(5, 4, 99),
        &target,
        2
    ));
    assert!(mermaid_focus_line_has_better_label(
        &focus_line(5, 5, 4),
        &target,
        2
    ));
}

#[test]
fn mermaid_focus_tied_priority_rejects_same_or_later_label() {
    let target = focus_target(5, 5, 5, 2);

    assert!(!mermaid_focus_line_has_better_label(
        &focus_line(5, 5, 5),
        &target,
        2
    ));
    assert!(!mermaid_focus_line_has_better_label(
        &focus_line(5, 5, 6),
        &target,
        2
    ));
    assert!(!mermaid_focus_line_has_better_label(
        &focus_line(5, 6, 1),
        &target,
        2
    ));
    assert!(!mermaid_focus_line_has_better_label(
        &focus_line(6, 1, 1),
        &target,
        2
    ));
}

#[test]
fn mermaid_set_background_cell_color_recolors_in_bounds_non_space_cell() {
    let mut cells = vec![vec![cell('-', MERMAID_CONNECTOR_COLOR)]];

    mermaid_set_background_cell_color(
        &mut cells,
        Rect {
            x: 10,
            y: 20,
            width: 1,
            height: 1,
        },
        10,
        20,
        Color::Yellow,
    );

    assert_eq!(cells[0][0], cell('-', Color::Yellow));
}

#[test]
fn mermaid_set_background_cell_color_preserves_space_cell_color() {
    let mut cells = vec![vec![cell(' ', Color::Reset)]];

    mermaid_set_background_cell_color(
        &mut cells,
        Rect {
            x: 10,
            y: 20,
            width: 1,
            height: 1,
        },
        10,
        20,
        Color::Yellow,
    );

    assert_eq!(cells[0][0], cell(' ', Color::Reset));
}

#[test]
fn mermaid_set_background_cell_color_ignores_coordinates_outside_content_rect() {
    let content_rect = Rect {
        x: 10,
        y: 20,
        width: 2,
        height: 2,
    };

    for (x, y) in [(9, 20), (12, 20), (10, 19), (10, 22)] {
        let mut cells = vec![vec![cell('-', MERMAID_CONNECTOR_COLOR)]];

        mermaid_set_background_cell_color(&mut cells, content_rect, x, y, Color::Yellow);

        assert_eq!(cells[0][0], cell('-', MERMAID_CONNECTOR_COLOR), "{x},{y}");
    }
}

#[test]
fn mermaid_set_background_cell_color_ignores_missing_cached_cell() {
    let content_rect = Rect {
        x: 10,
        y: 20,
        width: 2,
        height: 2,
    };
    let mut cells = vec![vec![cell('-', MERMAID_CONNECTOR_COLOR)]];

    mermaid_set_background_cell_color(&mut cells, content_rect, 11, 20, Color::Yellow);
    mermaid_set_background_cell_color(&mut cells, content_rect, 10, 21, Color::Yellow);

    assert_eq!(cells[0][0], cell('-', MERMAID_CONNECTOR_COLOR));
}

#[test]
fn render_mermaid_cached_background_draws_matching_cell_rows() {
    let mut renderer = test_renderer(8, 4);
    let viewer = test_viewer(
        vec!["fallback-1", "fallback-2"],
        vec![
            vec![cell('A', Color::Red), cell('B', Color::Green)],
            vec![cell('C', Color::Yellow)],
        ],
    );

    render_mermaid_cached_background(
        &mut renderer,
        Rect {
            x: 2,
            y: 1,
            width: 4,
            height: 2,
        },
        &viewer,
    );

    assert_eq!(rendered_cell(&renderer, 2, 1), cell('A', Color::Red));
    assert_eq!(rendered_cell(&renderer, 3, 1), cell('B', Color::Green));
    assert_eq!(rendered_cell(&renderer, 2, 2), cell('C', Color::Yellow));
}

#[test]
fn render_mermaid_cached_background_skips_space_cells() {
    let mut renderer = test_renderer(6, 3);
    renderer.draw_char(1, 1, '.', Color::Blue);
    let viewer = test_viewer(
        vec!["zz"],
        vec![vec![cell(' ', Color::Red), cell('Z', Color::Green)]],
    );

    render_mermaid_cached_background(
        &mut renderer,
        Rect {
            x: 1,
            y: 1,
            width: 3,
            height: 1,
        },
        &viewer,
    );

    assert_eq!(rendered_cell(&renderer, 1, 1), cell('.', Color::Blue));
    assert_eq!(rendered_cell(&renderer, 2, 1), cell('Z', Color::Green));
}

#[test]
fn render_mermaid_cached_background_clips_rows_at_bottom() {
    let mut renderer = test_renderer(6, 4);
    let viewer = test_viewer(
        vec!["first", "second"],
        vec![vec![cell('A', Color::Red)], vec![cell('B', Color::Green)]],
    );

    render_mermaid_cached_background(
        &mut renderer,
        Rect {
            x: 1,
            y: 2,
            width: 3,
            height: 1,
        },
        &viewer,
    );

    assert_eq!(rendered_cell(&renderer, 1, 2), cell('A', Color::Red));
    assert_eq!(rendered_cell(&renderer, 1, 3), Cell::default());
}

#[test]
fn render_mermaid_cached_background_falls_back_to_cached_lines() {
    let mut renderer = test_renderer(8, 4);
    let viewer = test_viewer(vec!["ab", "cd"], Vec::new());

    render_mermaid_cached_background(
        &mut renderer,
        Rect {
            x: 2,
            y: 1,
            width: 4,
            height: 1,
        },
        &viewer,
    );

    assert_eq!(
        rendered_cell(&renderer, 2, 1),
        cell('a', MERMAID_CONNECTOR_COLOR)
    );
    assert_eq!(
        rendered_cell(&renderer, 3, 1),
        cell('b', MERMAID_CONNECTOR_COLOR)
    );
    assert_eq!(rendered_cell(&renderer, 2, 2), Cell::default());
}
