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
