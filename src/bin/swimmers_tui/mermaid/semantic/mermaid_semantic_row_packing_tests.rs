use super::*;

fn projected_candidate(
    owner_key: &str,
    kind: MermaidSemanticKind,
    x: u16,
    y: u16,
    text: &str,
    compact_rows: bool,
) -> MermaidProjectedCandidate {
    MermaidProjectedCandidate {
        priority: kind.priority(),
        area_rank: 0,
        kind,
        owner_key: owner_key.to_string(),
        compact_rows,
        source_index: 0,
        x,
        y,
        text: text.to_string(),
        color: Color::White,
    }
}

fn projection_bounds(top: i32, bottom: i32) -> MermaidProjectionBounds {
    MermaidProjectionBounds {
        left: 0,
        right: 80,
        top,
        bottom,
    }
}

#[test]
fn mermaid_numeric_prefix_accepts_digits_with_single_optional_marker() {
    for token in ["1", "01", "2.", "3)", "4:", "  42.  "] {
        assert!(mermaid_is_numeric_prefix(token), "{token:?}");
    }
}

#[test]
fn mermaid_numeric_prefix_rejects_missing_digits_or_extra_suffix() {
    for token in ["", ".", "a1", "1..", "1) item", "1-", "one:"] {
        assert!(!mermaid_is_numeric_prefix(token), "{token:?}");
    }
}

#[test]
fn mermaid_first_available_candidate_row_uses_nudged_rows_after_collision() {
    let candidate = projected_candidate(
        "node:a",
        MermaidSemanticKind::ClassMember,
        10,
        5,
        "value",
        false,
    );
    let occupied_rows = HashMap::from([(5, vec![(9, 16)])]);

    assert_eq!(
        mermaid_first_available_candidate_row(
            &candidate,
            &occupied_rows,
            1,
            projection_bounds(0, 10)
        ),
        Some((6, 9, 16))
    );
}

#[test]
fn mermaid_first_available_candidate_row_allows_touching_ranges() {
    let candidate = projected_candidate(
        "node:a",
        MermaidSemanticKind::ClassMember,
        10,
        5,
        "value",
        false,
    );
    let occupied_rows = HashMap::from([(5, vec![(0, 9), (16, 20)])]);

    assert_eq!(
        mermaid_first_available_candidate_row(
            &candidate,
            &occupied_rows,
            1,
            projection_bounds(0, 10)
        ),
        Some((5, 9, 16))
    );
}

#[test]
fn mermaid_first_available_candidate_row_skips_out_of_bounds_rows() {
    let candidate = projected_candidate(
        "node:a",
        MermaidSemanticKind::ClassMember,
        10,
        1,
        "value",
        false,
    );

    assert_eq!(
        mermaid_first_available_candidate_row(
            &candidate,
            &HashMap::new(),
            0,
            projection_bounds(2, 4)
        ),
        Some((2, 10, 15))
    );
}

#[test]
fn pack_compact_rows_remaps_unique_owner_rows_to_consecutive_rows() {
    let mut candidates = vec![
        projected_candidate(
            "node:a",
            MermaidSemanticKind::NodeTitle,
            1,
            10,
            "title",
            true,
        ),
        projected_candidate(
            "node:a",
            MermaidSemanticKind::ClassMember,
            1,
            12,
            "first",
            true,
        ),
        projected_candidate(
            "node:a",
            MermaidSemanticKind::ClassMember,
            1,
            12,
            "second",
            true,
        ),
        projected_candidate(
            "node:a",
            MermaidSemanticKind::ClassMember,
            1,
            30,
            "third",
            true,
        ),
        projected_candidate(
            "node:b",
            MermaidSemanticKind::NodeTitle,
            1,
            7,
            "other",
            true,
        ),
        projected_candidate(
            "edge:a",
            MermaidSemanticKind::EdgeLabel,
            1,
            20,
            "edge",
            false,
        ),
    ];

    pack_compact_rows(&mut candidates, 12);

    let rows = candidates
        .iter()
        .map(|candidate| candidate.y)
        .collect::<Vec<_>>();
    assert_eq!(rows, vec![10, 11, 11, 12, 7, 20]);
}
