use super::*;

#[test]
fn mermaid_compact_overview_text_prefers_numeric_prefix_and_keywords() {
    let compact = mermaid_compact_overview_text([
        "1. Verified Identity And",
        "/api/cfo/admin/* calls are not outside the hierarchy",
    ])
    .expect("compact overview text");

    assert_eq!(compact, "1. Verified Identity");
}

#[test]
fn mermaid_compact_overview_text_splits_snake_case_into_words() {
    let compact = mermaid_compact_overview_text(["governed_revision_artifacts"])
        .expect("compact snake_case overview text");

    assert_eq!(compact, "governed revision");
}
