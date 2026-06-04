use super::*;

#[test]
fn mermaid_er_scroll_enters_keys_then_columns_then_schema_states() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(
        semantic_texts.contains(&"id PK".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"email".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"uuid".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"string".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER keys"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.contains(&"email".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        semantic_texts.contains(&"id PK".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.contains(&"uuid".to_string()),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER columns"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(
        semantic_texts.iter().any(|text| text.contains("uuid")),
        "{semantic_texts:?}"
    );
    assert!(
        semantic_texts.iter().any(|text| text.contains("string")),
        "{semantic_texts:?}"
    );
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER schema"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_er_reset_fit_returns_to_entities_state() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    for _ in 0..3 {
        scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    }
    app.render(&mut renderer, layout);
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("ER schema"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );

    assert!(handle_key_event(
        &mut app,
        layout,
        KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE),
    ));
    app.render(&mut renderer, layout);

    let semantic_texts = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => cached_semantic_texts(viewer),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert!(semantic_texts.contains(&"USER".to_string()));
    assert!(!semantic_texts.contains(&"id PK".to_string()));
    assert!(!semantic_texts.contains(&"email".to_string()));
    assert!(!semantic_texts.contains(&"uuid".to_string()));
    assert!(!semantic_texts.contains(&"string".to_string()));
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
    assert!(
        row_text(&renderer, layout.overview_field.y).contains("fit 100%"),
        "status row: {}",
        row_text(&renderer, layout.overview_field.y)
    );
}

#[test]
fn mermaid_er_dense_schema_fit_is_centered_and_uses_the_viewport() {
    let source = r#"erDiagram
applications {
  uuid id PK
}
conversation_anchor_types {
  uuid id PK
  uuid application_id FK
  string anchor_type
}
conversation_anchors {
  uuid id PK
  uuid application_id FK
  uuid anchor_type_id FK
  string anchor_key
}
conversations {
  uuid id PK
  uuid application_id FK
  uuid anchor_id FK
  string conversation_type
}
conversation_policy_bindings {
  uuid id PK
  uuid conversation_id FK
  string policy_template_key
}
conversation_named_participants {
  uuid id PK
  uuid conversation_id FK
  string actor_type
}
conversation_effective_participants {
  uuid id PK
  uuid conversation_id FK
  boolean can_read
}
conversation_messages {
  uuid id PK
  uuid conversation_id FK
  string kind
}
conversation_events {
  uuid id PK
  uuid conversation_id FK
  uuid message_id FK
}
conversation_reads {
  uuid id PK
  uuid conversation_id FK
  uuid last_event_id FK
}
applications ||--o{ conversation_anchor_types : owns
applications ||--o{ conversation_anchors : scopes
applications ||--o{ conversations : scopes
conversation_anchor_types ||--o{ conversation_anchors : categorizes
conversation_anchors ||--o{ conversations : roots
conversations ||--o{ conversation_policy_bindings : uses
conversations ||--o{ conversation_named_participants : includes
conversations ||--o{ conversation_effective_participants : materializes
conversations ||--o{ conversation_messages : stores
conversations ||--o{ conversation_events : records
conversations ||--o{ conversation_reads : tracks
"#;
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 160, 48);

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
    assert!(semantic_texts.len() >= 6, "{semantic_texts:?}");
    assert!(
        !semantic_texts.iter().any(|text| text.contains(" PK")),
        "{semantic_texts:?}"
    );
    assert!(
        !semantic_texts.iter().any(|text| text.contains(" FK")),
        "{semantic_texts:?}"
    );
    let center_x = (bounds.0 + bounds.1) / 2;
    let center_y = (bounds.2 + bounds.3) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    let expected_y = content_rect.y + content_rect.height / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 3);
    assert!((center_y as i32 - expected_y as i32).abs() <= 2);
    let width_occupancy = f32::from(bounds.1.saturating_sub(bounds.0).saturating_add(1))
        / f32::from(content_rect.width);
    let height_occupancy = f32::from(bounds.3.saturating_sub(bounds.2).saturating_add(1))
        / f32::from(content_rect.height);
    assert!(width_occupancy >= 0.40, "{width_occupancy}");
    assert!(height_occupancy >= 0.30, "{height_occupancy}");
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
}

#[test]
fn mermaid_er_scroll_states_are_discrete_and_reversible() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    app.render(&mut renderer, layout);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER entities"));
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER keys"));
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);
    assert!(row_text(&renderer, layout.overview_field.y).contains("ER columns"));
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::Out);
    app.render(&mut renderer, layout);
    let status = row_text(&renderer, layout.overview_field.y);
    assert!(status.contains("ER keys"), "{status}");
    assert!(!status.contains("detail L"), "{status}");
}

#[test]
fn mermaid_er_zoom_resets_pan_and_recenters_packed_layout() {
    let source = "erDiagram\nUSER {\n  uuid id PK\n  string email\n}\nORDER {\n  uuid id PK\n  uuid user_id FK\n}\nUSER ||--o{ ORDER : places\n";
    let (mut app, mut renderer, layout) = open_mermaid_test_viewer(source, 120, 32);

    if let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode {
        viewer.center_x = 500.0;
        viewer.center_y = 400.0;
        viewer.invalidate_viewport_cache();
    }
    scroll_mermaid(&mut app, layout, MermaidZoomDirection::In);
    app.render(&mut renderer, layout);

    let (center_x, center_y, bounds, content_rect) = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => (
            viewer.center_x,
            viewer.center_y,
            mermaid_render_bounds(viewer, viewer.content_rect.expect("content rect"))
                .expect("render bounds"),
            viewer.content_rect.expect("content rect"),
        ),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_ne!(center_x, 500.0);
    assert_ne!(center_y, 400.0);
    let center_x = (bounds.0 + bounds.1) / 2;
    let center_y = (bounds.2 + bounds.3) / 2;
    let expected_x = content_rect.x + content_rect.width / 2;
    let expected_y = content_rect.y + content_rect.height / 2;
    assert!((center_x as i32 - expected_x as i32).abs() <= 2);
    assert!((center_y as i32 - expected_y as i32).abs() <= 1);
}

#[test]
fn mermaid_er_order_clusters_connected_nodes_before_isolated_scanline_nodes() {
    let order = mermaid_order_er_nodes(&[
        er_order_node("node:a_leaf", 0.0, 0.0, &["node:a_hub"]),
        er_order_node("node:b_isolated", 10.0, 0.0, &[]),
        er_order_node("node:a_hub", 0.0, 10.0, &["node:a_leaf", "node:a_tail"]),
        er_order_node("node:a_tail", 0.0, 20.0, &["node:a_hub"]),
    ]);

    assert_eq!(
        order,
        vec![
            "node:a_hub".to_string(),
            "node:a_leaf".to_string(),
            "node:a_tail".to_string(),
            "node:b_isolated".to_string(),
        ]
    );
}

#[test]
fn mermaid_er_order_keeps_components_contiguous_when_xy_positions_interleave() {
    let order = mermaid_order_er_nodes(&[
        er_order_node("node:north_a", 0.0, 0.0, &["node:north_b"]),
        er_order_node("node:south_a", 20.0, 0.0, &["node:south_b"]),
        er_order_node("node:north_b", 0.0, 10.0, &["node:north_a"]),
        er_order_node("node:south_b", 20.0, 10.0, &["node:south_a"]),
    ]);

    assert_eq!(
        order,
        vec![
            "node:north_a".to_string(),
            "node:north_b".to_string(),
            "node:south_a".to_string(),
            "node:south_b".to_string(),
        ]
    );
}

#[test]
fn mermaid_er_order_ignores_self_and_unknown_neighbors_for_tie_breaks() {
    let order = mermaid_order_er_nodes(&[
        er_order_node("node:b", 10.0, 10.0, &[]),
        er_order_node("node:a", 10.0, 10.0, &["node:a", "node:missing"]),
    ]);

    assert_eq!(order, vec!["node:a".to_string(), "node:b".to_string()]);
}

#[test]
fn mermaid_er_order_graph_builds_symmetric_known_adjacency() {
    let (positions, adjacency) = mermaid_er_order_graph(&[
        er_order_node("node:a", 1.0, 2.0, &["node:b", "node:a", "node:missing"]),
        er_order_node("node:b", 3.0, 4.0, &[]),
        er_order_node("node:c", 5.0, 6.0, &["node:b"]),
        er_order_node("node:isolated", 7.0, 8.0, &[]),
    ]);

    assert_eq!(positions.len(), 4);
    assert_eq!(positions.get("node:a"), Some(&(1.0, 2.0)));
    assert_eq!(positions.get("node:b"), Some(&(3.0, 4.0)));
    assert_eq!(positions.get("node:c"), Some(&(5.0, 6.0)));
    assert_eq!(positions.get("node:isolated"), Some(&(7.0, 8.0)));

    let expected_keys = BTreeSet::from([
        "node:a".to_string(),
        "node:b".to_string(),
        "node:c".to_string(),
        "node:isolated".to_string(),
    ]);
    assert_eq!(
        adjacency.keys().cloned().collect::<BTreeSet<_>>(),
        expected_keys
    );

    let expected_a = BTreeSet::from(["node:b".to_string()]);
    let expected_b = BTreeSet::from(["node:a".to_string(), "node:c".to_string()]);
    let expected_c = BTreeSet::from(["node:b".to_string()]);
    let expected_isolated = BTreeSet::<String>::new();
    assert_eq!(adjacency.get("node:a"), Some(&expected_a));
    assert_eq!(adjacency.get("node:b"), Some(&expected_b));
    assert_eq!(adjacency.get("node:c"), Some(&expected_c));
    assert_eq!(adjacency.get("node:isolated"), Some(&expected_isolated));
}

#[test]
fn mermaid_er_candidate_ordering_preserves_tie_break_precedence() {
    assert_er_order_prefix(
        &[
            er_order_node(
                "node:seed",
                0.0,
                0.0,
                &["node:first", "node:two_adjacent", "node:one_adjacent"],
            ),
            er_order_node("node:first", 1.0, 0.0, &["node:two_adjacent"]),
            er_order_node("node:two_adjacent", 10.0, 0.0, &[]),
            er_order_node("node:one_adjacent", 5.0, 0.0, &[]),
        ],
        &["node:seed", "node:first", "node:two_adjacent"],
    );
    assert_er_order_prefix(
        &[
            er_order_node("node:seed", 0.0, 0.0, &["node:near", "node:far"]),
            er_order_node("node:near", 2.0, 0.0, &[]),
            er_order_node("node:far", 5.0, 0.0, &[]),
        ],
        &["node:seed", "node:near"],
    );
    assert_er_order_prefix(
        &[
            er_order_node("node:seed", 0.0, 0.0, &["node:a", "node:b", "node:f"]),
            er_order_node("node:a", 1.0, 0.0, &[]),
            er_order_node("node:b", 1.0, 0.0, &["node:c"]),
            er_order_node("node:c", 10.0, 0.0, &[]),
            er_order_node("node:f", 20.0, 0.0, &[]),
        ],
        &["node:seed", "node:b"],
    );
    assert_er_order_prefix(
        &[
            er_order_node(
                "node:seed",
                0.0,
                0.0,
                &["node:right", "node:left", "node:far_left"],
            ),
            er_order_node("node:right", 1.0, 0.0, &[]),
            er_order_node("node:left", -1.0, 0.0, &[]),
            er_order_node("node:far_left", -10.0, 0.0, &[]),
        ],
        &["node:seed", "node:left"],
    );
    assert_er_order_prefix(
        &[
            er_order_node(
                "node:seed",
                0.0,
                0.0,
                &["node:y_high", "node:y_low", "node:centering_tail"],
            ),
            er_order_node("node:y_high", 1.0, 1.0, &[]),
            er_order_node("node:y_low", 1.0, -1.0, &[]),
            er_order_node("node:centering_tail", -2.0, 0.0, &[]),
        ],
        &["node:seed", "node:y_low"],
    );
    assert_er_order_prefix(
        &[
            er_order_node("node:seed", 0.0, 0.0, &["node:right", "node:left"]),
            er_order_node("node:right", 1.0, 0.0, &[]),
            er_order_node("node:left", -1.0, 0.0, &[]),
        ],
        &["node:seed", "node:left"],
    );
    assert_er_order_prefix(
        &[
            er_order_node("node:seed", 0.0, 0.0, &["node:b", "node:a"]),
            er_order_node("node:b", 1.0, 0.0, &[]),
            er_order_node("node:a", 1.0, 0.0, &[]),
        ],
        &["node:seed", "node:a"],
    );
}

fn assert_er_order_prefix(nodes: &[MermaidErOrderNode], expected_prefix: &[&str]) {
    let order = mermaid_order_er_nodes(nodes);
    let actual_prefix = order
        .iter()
        .take(expected_prefix.len())
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(actual_prefix, expected_prefix, "full order: {order:?}");
}

#[test]
fn mermaid_er_box_content_filters_title_and_attrs_by_view_state() {
    let lines = vec![
        er_semantic_line("ACCOUNT", 1.0, MermaidSemanticKind::NodeSummary),
        er_semantic_line("ACCOUNT", 1.0, MermaidSemanticKind::NodeTitle),
        er_semantic_line("uuid", 2.0, MermaidSemanticKind::ErAttributeType),
        er_semantic_line("id PK", 2.0, MermaidSemanticKind::ErAttributeName),
        er_semantic_line("uuid", 3.0, MermaidSemanticKind::ErAttributeType),
        er_semantic_line("user_id FK", 3.0, MermaidSemanticKind::ErAttributeName),
        er_semantic_line("string", 4.0, MermaidSemanticKind::ErAttributeType),
        er_semantic_line("display_name", 4.0, MermaidSemanticKind::ErAttributeName),
    ];
    let source_indices = (0..lines.len()).collect::<Vec<_>>();

    let (entity_titles, entity_attrs) =
        mermaid_build_er_box_content(&lines, &source_indices, MermaidViewState::ErEntities);
    assert_eq!(entity_titles, vec![(0, "ACCOUNT".to_string())]);
    assert!(entity_attrs.is_empty());

    let (key_titles, key_attrs) =
        mermaid_build_er_box_content(&lines, &source_indices, MermaidViewState::ErKeys);
    assert_eq!(key_titles, vec![(1, "ACCOUNT".to_string())]);
    assert_eq!(
        key_attrs
            .iter()
            .map(|row| row.name_text.as_str())
            .collect::<Vec<_>>(),
        vec!["id PK", "user_id FK"]
    );

    let (_, column_attrs) =
        mermaid_build_er_box_content(&lines, &source_indices, MermaidViewState::ErColumns);
    assert_eq!(column_attrs.len(), 3);
    assert!(column_attrs
        .iter()
        .all(|row| row.type_text.as_deref().is_some()));
}

#[test]
fn mermaid_er_pack_plan_prefers_viewport_fit_and_keeps_fallback_shape() {
    let specs = vec![
        MermaidErBoxSize {
            outer_width: 10,
            outer_height: 4,
            type_col_width: 0,
        },
        MermaidErBoxSize {
            outer_width: 10,
            outer_height: 4,
            type_col_width: 0,
        },
        MermaidErBoxSize {
            outer_width: 10,
            outer_height: 4,
            type_col_width: 0,
        },
        MermaidErBoxSize {
            outer_width: 10,
            outer_height: 4,
            type_col_width: 0,
        },
    ];

    let plan = mermaid_plan_er_box_packing(&specs, 24, 10, 2, 1);
    assert_eq!(plan.column_count, 2);
    assert_eq!(plan.row_widths, vec![22, 22]);
    assert_eq!(plan.row_heights, vec![4, 4]);
    assert_eq!(plan.cluster_height, 9);

    let fallback = mermaid_plan_er_box_packing(
        &[MermaidErBoxSize {
            outer_width: 50,
            outer_height: 20,
            type_col_width: 0,
        }],
        10,
        5,
        2,
        1,
    );
    assert_eq!(fallback.column_count, 1);
    assert_eq!(fallback.row_widths, vec![50]);
    assert_eq!(fallback.row_heights, vec![20]);
    assert_eq!(fallback.cluster_height, 5);
}

#[test]
fn mermaid_er_connected_owner_pair_filters_missing_and_same_owner_edges() {
    let owners = HashMap::from([
        ("node-a".to_string(), "owner-a".to_string()),
        ("node-a-label".to_string(), "owner-a".to_string()),
        ("node-b".to_string(), "owner-b".to_string()),
    ]);

    assert!(mermaid_er_connected_owner_pair("node-a", "missing", &owners).is_none());
    assert!(mermaid_er_connected_owner_pair("node-a", "node-a-label", &owners).is_none());

    let Some((from, to)) = mermaid_er_connected_owner_pair("node-a", "node-b", &owners) else {
        panic!("expected connected owner pair");
    };
    assert_eq!(from, "owner-a");
    assert_eq!(to, "owner-b");
}

#[test]
fn mermaid_too_small_view_keeps_existing_guard() {
    let (mut app, mut renderer, _layout) =
        open_mermaid_test_viewer("graph TD\nA[Alpha Node] --> B[Beta Node]\n", 120, 32);
    let small_field = Rect {
        x: 0,
        y: 0,
        width: 15,
        height: 7,
    };
    let FishBowlMode::Mermaid(viewer) = &mut app.fish_bowl_mode else {
        panic!("expected Mermaid viewer mode");
    };
    render_mermaid_viewer(&mut renderer, small_field, viewer);

    assert!(find_text_position(&renderer, "Mermaid view").is_some());
    assert!(find_text_position(&renderer, "too small").is_some());
    let semantic_count = match &app.fish_bowl_mode {
        FishBowlMode::Mermaid(viewer) => viewer.cached_semantic_lines.len(),
        FishBowlMode::Aquarium => panic!("expected Mermaid viewer mode"),
    };
    assert_eq!(semantic_count, 0);
}

fn er_semantic_line(text: &str, diagram_y: f32, kind: MermaidSemanticKind) -> MermaidSemanticLine {
    MermaidSemanticLine {
        text: text.to_string(),
        diagram_x: 0.0,
        diagram_y,
        anchor: MermaidTextAnchor::Start,
        kind,
        owner_key: "node:account".to_string(),
        outline_eligible: true,
        owner_width: 100.0,
        owner_height: 60.0,
    }
}
