use super::*;

#[test]
fn fit_whole_words_keeps_text_at_exact_budget() {
    assert_eq!(mermaid_fit_whole_words("Alpha Node", 10), "Alpha Node");
}

#[test]
fn fit_whole_words_stops_before_word_that_exceeds_budget() {
    assert_eq!(
        mermaid_fit_whole_words("Alpha Node Details", 10),
        "Alpha Node"
    );
}

#[test]
fn fit_whole_words_preserves_empty_result_for_first_word_over_budget() {
    assert_eq!(mermaid_fit_whole_words("AlphaNode", 5), "");
}

#[test]
fn fit_whole_words_zero_budget_returns_empty() {
    assert_eq!(mermaid_fit_whole_words("Alpha", 0), "");
}

#[test]
fn fit_whole_words_collapses_source_whitespace_between_kept_words() {
    assert_eq!(
        mermaid_fit_whole_words("Alpha   Node\tReady", 12),
        "Alpha Node"
    );
}

#[test]
fn fit_whole_words_counts_unicode_chars_not_bytes() {
    assert_eq!(mermaid_fit_whole_words("café café", 4), "café");
    assert_eq!(mermaid_fit_whole_words("café café", 9), "café café");
}
