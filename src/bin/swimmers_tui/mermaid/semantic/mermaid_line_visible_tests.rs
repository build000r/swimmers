use super::*;

fn semantic_line(
    kind: MermaidSemanticKind,
    text: &str,
    outline_eligible: bool,
) -> MermaidSemanticLine {
    MermaidSemanticLine {
        text: text.to_string(),
        diagram_x: 0.0,
        diagram_y: 0.0,
        anchor: MermaidTextAnchor::Start,
        kind,
        owner_key: "node:ACCOUNT".to_string(),
        outline_eligible,
        owner_width: 100.0,
        owner_height: 40.0,
    }
}

#[test]
fn mermaid_line_visible_outline_uses_outline_eligibility() {
    let visible = semantic_line(MermaidSemanticKind::NodeSummary, "ACCOUNT", true);
    let hidden = semantic_line(MermaidSemanticKind::NodeSummary, "ACCOUNT", false);

    assert!(mermaid_line_visible_in_state(
        &visible,
        MermaidViewState::Outline
    ));
    assert!(!mermaid_line_visible_in_state(
        &hidden,
        MermaidViewState::Outline
    ));
}

#[test]
fn mermaid_line_visible_detail_levels_follow_kind_thresholds() {
    let summary = semantic_line(MermaidSemanticKind::NodeSummary, "ACCOUNT", false);
    let title = semantic_line(MermaidSemanticKind::NodeTitle, "ACCOUNT", false);
    let er_type = semantic_line(MermaidSemanticKind::ErAttributeType, "int", false);

    assert!(mermaid_line_visible_in_state(
        &summary,
        MermaidViewState::L1
    ));
    assert!(!mermaid_line_visible_in_state(&title, MermaidViewState::L1));
    assert!(mermaid_line_visible_in_state(&title, MermaidViewState::L2));
    assert!(!mermaid_line_visible_in_state(
        &er_type,
        MermaidViewState::L2
    ));
    assert!(mermaid_line_visible_in_state(
        &er_type,
        MermaidViewState::L3
    ));
}

#[test]
fn mermaid_line_visible_detail_state_without_detail_level_warns_and_hides() {
    let line = semantic_line(MermaidSemanticKind::NodeSummary, "ACCOUNT", true);

    assert!(!mermaid_line_visible_in_detail_state(
        &line,
        MermaidViewState::Outline
    ));
}

#[test]
fn mermaid_line_visible_er_states_filter_lines_by_semantic_role() {
    let entity = semantic_line(MermaidSemanticKind::NodeSummary, "ACCOUNT", true);
    let title = semantic_line(MermaidSemanticKind::NodeTitle, "ACCOUNT", false);
    let pk_name = semantic_line(MermaidSemanticKind::ErAttributeName, "id PK", false);
    let fk_name = semantic_line(MermaidSemanticKind::ErAttributeName, "account_id FK", false);
    let regular_name = semantic_line(MermaidSemanticKind::ErAttributeName, "status", false);
    let er_type = semantic_line(MermaidSemanticKind::ErAttributeType, "varchar", false);

    assert!(mermaid_line_visible_in_state(
        &entity,
        MermaidViewState::ErEntities
    ));
    assert!(!mermaid_line_visible_in_state(
        &title,
        MermaidViewState::ErEntities
    ));

    assert!(mermaid_line_visible_in_state(
        &title,
        MermaidViewState::ErKeys
    ));
    assert!(mermaid_line_visible_in_state(
        &pk_name,
        MermaidViewState::ErKeys
    ));
    assert!(mermaid_line_visible_in_state(
        &fk_name,
        MermaidViewState::ErKeys
    ));
    assert!(!mermaid_line_visible_in_state(
        &regular_name,
        MermaidViewState::ErKeys
    ));

    assert!(mermaid_line_visible_in_state(
        &title,
        MermaidViewState::ErColumns
    ));
    assert!(mermaid_line_visible_in_state(
        &regular_name,
        MermaidViewState::ErColumns
    ));
    assert!(!mermaid_line_visible_in_state(
        &er_type,
        MermaidViewState::ErColumns
    ));

    assert!(mermaid_line_visible_in_state(
        &title,
        MermaidViewState::ErSchema
    ));
    assert!(mermaid_line_visible_in_state(
        &regular_name,
        MermaidViewState::ErSchema
    ));
    assert!(mermaid_line_visible_in_state(
        &er_type,
        MermaidViewState::ErSchema
    ));
}
