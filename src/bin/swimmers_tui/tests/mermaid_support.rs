fn open_mermaid_test_viewer(
    source: &str,
    width: u16,
    height: u16,
) -> (App<MockApi>, Renderer, WorkspaceLayout) {
    let api = MockApi::new();
    let layout = test_layout(width, height);
    let mut app = make_app(api);
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
            source,
        ),
    );
    app.open_mermaid_viewer("sess-1".to_string());
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    viewer.unsupported_reason = None;
    (app, test_renderer(width, height), layout)
}

#[derive(Clone, Copy, Debug)]
enum MermaidMetamorphicOp {
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MermaidSemanticSnapshot {
    source_index: usize,
    text: String,
    rel_x: u16,
    rel_y: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MermaidMetamorphicSnapshot {
    view_state: MermaidViewState,
    focused_source_index: Option<usize>,
    cached_lines: Vec<String>,
    semantic_lines: Vec<MermaidSemanticSnapshot>,
}

fn mermaid_flowchart_source_strategy() -> impl Strategy<Value = String> {
    let words = prop::sample::select(vec![
        "Alpha", "Beta", "Gamma", "Delta", "Producer", "Consumer", "Queue", "Worker", "Client",
        "Server", "Stream", "Buffer",
    ]);
    let edges = prop::sample::select(vec!["ships", "queues", "sends", "loads", "syncs", "pushes"]);

    (
        0u8..3,
        words.clone(),
        words.clone(),
        words.clone(),
        words.clone(),
        edges,
    )
        .prop_map(
            |(template, left, right, extra, group, edge)| match template {
                0 => format!("graph TD\nA[{left}] -->|{edge}| B[{right}]\n"),
                1 => format!(
                    "graph TD\nsubgraph {group}\nA[{left}]\nB[{right}]\nend\nA -->|{edge}| B\n"
                ),
                _ => format!("graph TD\nA[{left}] -->|{edge}| B[{right}]\nA --> C[{extra}]\n"),
            },
        )
}

fn mermaid_anchorable_source_strategy() -> impl Strategy<Value = String> {
    let words = prop::sample::select(vec![
        "Alpha", "Beta", "Gamma", "Delta", "Producer", "Consumer", "Stream", "Buffer",
    ]);
    let edges = prop::sample::select(vec!["ships", "queues", "sends", "syncs"]);

    (words.clone(), words, edges).prop_map(|(left, right, edge)| {
        format!("graph TD\nA[{left} Node] -->|{edge}| B[{right} Node]\n")
    })
}

fn mermaid_metamorphic_ops_strategy() -> impl Strategy<Value = Vec<MermaidMetamorphicOp>> {
    proptest::collection::vec(0u8..6, 0..8).prop_map(|ops| {
        ops.into_iter()
            .map(|op| match op {
                0 => MermaidMetamorphicOp::ZoomIn,
                1 => MermaidMetamorphicOp::ZoomOut,
                2 => MermaidMetamorphicOp::PanLeft,
                3 => MermaidMetamorphicOp::PanRight,
                4 => MermaidMetamorphicOp::PanUp,
                _ => MermaidMetamorphicOp::PanDown,
            })
            .collect()
    })
}

fn mermaid_snapshot(viewer: &MermaidViewerState) -> MermaidMetamorphicSnapshot {
    let content_rect = viewer.content_rect.expect("content rect");
    let mut semantic_lines = viewer
        .cached_semantic_lines
        .iter()
        .map(|line| MermaidSemanticSnapshot {
            source_index: line.source_index,
            text: line.text.clone(),
            rel_x: line.x.saturating_sub(content_rect.x),
            rel_y: line.y.saturating_sub(content_rect.y),
        })
        .collect::<Vec<_>>();
    semantic_lines.sort();

    MermaidMetamorphicSnapshot {
        view_state: mermaid_view_state_for_view(viewer, content_rect),
        focused_source_index: viewer.focused_source_index,
        cached_lines: viewer.cached_lines.clone(),
        semantic_lines,
    }
}

fn render_mermaid_snapshot(
    app: &mut App<MockApi>,
    renderer: &mut Renderer,
    layout: WorkspaceLayout,
) -> MermaidMetamorphicSnapshot {
    app.render(renderer, layout);
    let FishBowlMode::Mermaid(viewer) = &app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    mermaid_snapshot(viewer)
}

fn mermaid_content_rect_for_layout(layout: WorkspaceLayout) -> Rect {
    mermaid_content_rect(layout.overview_field)
}

fn mermaid_pan_headroom(viewer: &MermaidViewerState, content_rect: Rect) -> (f32, f32, f32, f32) {
    let (sample_width, sample_height) = mermaid_sample_dimensions(content_rect);
    let base_scale = mermaid_fit_scale(
        viewer.diagram_width,
        viewer.diagram_height,
        sample_width as f32,
        sample_height as f32,
    );
    let scale = (base_scale * viewer.zoom.clamp(MERMAID_MIN_ZOOM, MERMAID_MAX_ZOOM)).max(0.000_1);
    let visible_width = sample_width as f32 / scale;
    let visible_height = sample_height as f32 / scale;

    let min_center_x = if visible_width >= viewer.diagram_width {
        viewer.diagram_width / 2.0
    } else {
        visible_width / 2.0
    };
    let max_center_x = if visible_width >= viewer.diagram_width {
        viewer.diagram_width / 2.0
    } else {
        viewer.diagram_width - visible_width / 2.0
    };
    let min_center_y = if visible_height >= viewer.diagram_height {
        viewer.diagram_height / 2.0
    } else {
        visible_height / 2.0
    };
    let max_center_y = if visible_height >= viewer.diagram_height {
        viewer.diagram_height / 2.0
    } else {
        viewer.diagram_height - visible_height / 2.0
    };

    (
        (viewer.center_x - min_center_x).max(0.0),
        (max_center_x - viewer.center_x).max(0.0),
        (viewer.center_y - min_center_y).max(0.0),
        (max_center_y - viewer.center_y).max(0.0),
    )
}

fn mermaid_safe_pan_distance(
    ratio_percent: i16,
    negative_headroom: f32,
    positive_headroom: f32,
) -> f32 {
    if ratio_percent < 0 {
        -negative_headroom * f32::from(-ratio_percent) / 100.0
    } else {
        positive_headroom * f32::from(ratio_percent) / 100.0
    }
}

fn apply_mermaid_metamorphic_ops(
    app: &mut App<MockApi>,
    layout: WorkspaceLayout,
    ops: &[MermaidMetamorphicOp],
) {
    let content_rect = mermaid_content_rect_for_layout(layout);
    for op in ops {
        match op {
            MermaidMetamorphicOp::ZoomIn => {
                app.zoom_mermaid_viewer(MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
            }
            MermaidMetamorphicOp::ZoomOut => {
                app.zoom_mermaid_viewer(-MERMAID_SCROLL_ZOOM_STEP_PERCENT, None, content_rect);
            }
            MermaidMetamorphicOp::PanLeft => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).0,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(-step, 0.0);
            }
            MermaidMetamorphicOp::PanRight => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).0,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(step, 0.0);
            }
            MermaidMetamorphicOp::PanUp => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).1,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(0.0, -step);
            }
            MermaidMetamorphicOp::PanDown => {
                let step = match &app.fish_bowl_mode {
                    FishBowlMode::Mermaid(viewer) => mermaid_pan_step(viewer, content_rect).1,
                    FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
                };
                app.pan_mermaid_viewer(0.0, step);
            }
        }
    }
}

fn find_cached_semantic_line(viewer: &MermaidViewerState, needle: &str) -> Option<(u16, u16)> {
    viewer
        .cached_semantic_lines
        .iter()
        .find(|line| line.text == needle)
        .map(|line| (line.x, line.y))
}

fn cached_semantic_texts(viewer: &MermaidViewerState) -> Vec<String> {
    viewer
        .cached_semantic_lines
        .iter()
        .map(|line| line.text.clone())
        .collect()
}

fn mermaid_background_charset(viewer: &MermaidViewerState) -> Vec<char> {
    viewer
        .cached_lines
        .iter()
        .flat_map(|line| line.chars())
        .filter(|ch| *ch != ' ')
        .collect()
}

fn mermaid_background_colors(viewer: &MermaidViewerState) -> Vec<Color> {
    viewer
        .cached_background_cells
        .iter()
        .flat_map(|row| row.iter())
        .filter(|cell| cell.ch != ' ')
        .map(|cell| cell.fg)
        .collect()
}

fn mermaid_background_colors_set(
    viewer: &MermaidViewerState,
) -> std::collections::BTreeSet<String> {
    mermaid_background_colors(viewer)
        .into_iter()
        .map(|color| format!("{color:?}"))
        .collect()
}

fn mermaid_text_color(renderer: &Renderer, needle: &str) -> Color {
    let (x, y) = find_text_position(renderer, needle).unwrap_or_else(|| panic!("{needle}"));
    cell_at(renderer, x, y).fg
}

fn mermaid_border_color(renderer: &Renderer, needle: &str) -> Color {
    let (x, y) = find_text_position(renderer, needle).unwrap_or_else(|| panic!("{needle}"));
    let width = display_width(needle);
    let candidates = [
        (x.saturating_sub(1), y),
        (x.saturating_add(width), y),
        (x, y.saturating_sub(1)),
        (x, y.saturating_add(1)),
    ];
    candidates
        .into_iter()
        .map(|(cx, cy)| cell_at(renderer, cx, cy))
        .find(|cell| matches!(cell.ch, '|' | '_'))
        .map(|cell| cell.fg)
        .unwrap_or_else(|| panic!("missing border for {needle}"))
}

fn mermaid_owner_key_for_text(viewer: &MermaidViewerState, needle: &str) -> String {
    let line = viewer
        .cached_semantic_lines
        .iter()
        .find(|line| line.text == needle)
        .unwrap_or_else(|| panic!("{needle}"));
    viewer
        .prepared_render
        .as_ref()
        .and_then(|prepared| prepared.semantic_lines.get(line.source_index))
        .map(|line| line.owner_key.clone())
        .unwrap_or_else(|| panic!("missing owner key for {needle}"))
}

fn mermaid_render_bounds(
    viewer: &MermaidViewerState,
    content_rect: Rect,
) -> Option<(u16, u16, u16, u16)> {
    let mut left = u16::MAX;
    let mut right = 0u16;
    let mut top = u16::MAX;
    let mut bottom = 0u16;
    let mut saw_any = false;

    for (row_offset, line) in viewer.cached_lines.iter().enumerate() {
        let y = content_rect.y + row_offset as u16;
        for (column_offset, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let x = content_rect.x + column_offset as u16;
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            saw_any = true;
        }
    }

    for line in &viewer.cached_semantic_lines {
        let text_right = line
            .x
            .saturating_add(display_width(&line.text).saturating_sub(1));
        left = left.min(line.x);
        right = right.max(text_right);
        top = top.min(line.y);
        bottom = bottom.max(line.y);
        saw_any = true;
    }

    saw_any.then_some((left, right, top, bottom))
}

fn er_order_node(owner_key: &str, x: f32, y: f32, neighbors: &[&str]) -> MermaidErOrderNode {
    MermaidErOrderNode {
        owner_key: owner_key.to_string(),
        x,
        y,
        neighbors: neighbors
            .iter()
            .map(|neighbor| (*neighbor).to_string())
            .collect(),
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press_mermaid_key(app: &mut App<MockApi>, layout: WorkspaceLayout, key: char) {
    assert!(handle_key_event(
        app,
        layout,
        KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE),
    ));
}

fn press_mermaid_tab(app: &mut App<MockApi>, layout: WorkspaceLayout) {
    assert!(handle_key_event(
        app,
        layout,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    ));
}

fn press_mermaid_backtab(app: &mut App<MockApi>, layout: WorkspaceLayout) {
    assert!(handle_key_event(
        app,
        layout,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    ));
}

fn scroll_mermaid(
    app: &mut App<MockApi>,
    layout: WorkspaceLayout,
    direction: MermaidZoomDirection,
) {
    let column = layout.overview_field.x + layout.overview_field.width / 2;
    let row = layout.overview_field.y + layout.overview_field.height / 2;
    assert!(app.handle_mermaid_scroll(
        layout.overview_field,
        crossterm::event::MouseEvent {
            kind: match direction {
                MermaidZoomDirection::In => MouseEventKind::ScrollUp,
                MermaidZoomDirection::Out => MouseEventKind::ScrollDown,
            },
            column,
            row,
            modifiers: KeyModifiers::NONE,
        },
        direction,
    ));
}
