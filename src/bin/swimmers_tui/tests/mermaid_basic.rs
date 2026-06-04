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

#[test]
fn mermaid_compact_overview_text_falls_back_to_stopwords_when_all_words_are_common() {
    let compact =
        mermaid_compact_overview_text(["the and of"]).expect("compact stopword overview text");

    assert_eq!(compact, "the and of");
}

#[test]
fn mermaid_compact_overview_text_returns_numeric_prefix_when_words_do_not_fit() {
    let compact = mermaid_compact_overview_text(["12. extraordinarilylongidentifier"])
        .expect("compact numeric-only overview text");

    assert_eq!(compact, "12.");
}

#[test]
fn mermaid_compact_overview_text_strips_punctuation_and_honors_three_word_limit() {
    let compact = mermaid_compact_overview_text(["(Alpha), beta/gamma-delta epsilon"])
        .expect("compact punctuated overview text");

    assert_eq!(compact, "Alpha beta gamma");
}
