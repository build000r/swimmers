use crate::scroll::guard::ScrollOutputChunk;
use crate::state::detector::StateDetector;
use crate::types::SessionState;

pub(super) fn output_counts_as_meaningful_activity(
    previous_state: SessionState,
    current_state: SessionState,
    chunk: &ScrollOutputChunk,
) -> bool {
    meaningful_output_activity_reason(previous_state, current_state, chunk).is_some()
}

enum MeaningfulOutputActivity {
    BusyBecameIdle,
    VisibleOutput,
}

fn meaningful_output_activity_reason(
    previous_state: SessionState,
    current_state: SessionState,
    chunk: &ScrollOutputChunk,
) -> Option<MeaningfulOutputActivity> {
    if chunk.coalesced_redraw {
        return None;
    }

    if output_transition_finished_busy_work(previous_state, current_state) {
        return Some(MeaningfulOutputActivity::BusyBecameIdle);
    }

    visible_output_is_meaningful(&chunk.data).then_some(MeaningfulOutputActivity::VisibleOutput)
}

fn output_transition_finished_busy_work(
    previous_state: SessionState,
    current_state: SessionState,
) -> bool {
    !matches!(previous_state, SessionState::Idle) && matches!(current_state, SessionState::Idle)
}

fn visible_output_is_meaningful(data: &[u8]) -> bool {
    let visible = StateDetector::strip_ansi(&String::from_utf8_lossy(data));

    visible
        .lines()
        .map(str::trim)
        .any(trimmed_line_counts_as_meaningful_output)
}

fn trimmed_line_counts_as_meaningful_output(line: &str) -> bool {
    if line_looks_prompt_like(line) {
        return false;
    }

    line_has_substantive_text(line)
}

fn line_has_substantive_text(line: &str) -> bool {
    line_has_enough_visible_chars(line) && line_has_alphanumeric_char(line)
}

fn line_has_enough_visible_chars(line: &str) -> bool {
    line.chars().filter(|c| !c.is_whitespace()).count() >= 3
}

fn line_has_alphanumeric_char(line: &str) -> bool {
    line.chars().any(|c| c.is_alphanumeric())
}

fn line_looks_prompt_like(line: &str) -> bool {
    prompt_candidate(line)
        .map(prompt_candidate_looks_prompt_like)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy)]
struct PromptCandidate<'a> {
    prefix: &'a str,
    marker: char,
}

fn prompt_candidate(line: &str) -> Option<PromptCandidate<'_>> {
    let line = line.trim_end();
    let mut chars = line.chars();
    let marker = chars.next_back()?;
    is_shell_prompt_marker(marker).then_some(PromptCandidate {
        prefix: chars.as_str().trim_end(),
        marker,
    })
}

fn is_shell_prompt_marker(marker: char) -> bool {
    matches!(marker, '$' | '%' | '#' | '>')
}

fn prompt_candidate_looks_prompt_like(candidate: PromptCandidate<'_>) -> bool {
    if candidate.prefix.is_empty() {
        return true;
    }

    match prompt_prefix_class(candidate.prefix) {
        PromptPrefixClass::PathOrUser => {
            path_prompt_marker_allowed(candidate.marker, candidate.prefix)
        }
        PromptPrefixClass::Plain => plain_prompt_marker_allowed(candidate.marker),
        PromptPrefixClass::Other => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptPrefixClass {
    PathOrUser,
    Plain,
    Other,
}

fn prompt_prefix_class(prefix: &str) -> PromptPrefixClass {
    path_prompt_prefix_class(prefix).unwrap_or_else(|| plain_prompt_prefix_class(prefix))
}

fn path_prompt_prefix_class(prefix: &str) -> Option<PromptPrefixClass> {
    prefix_has_path_or_user_marker(prefix).then_some(PromptPrefixClass::PathOrUser)
}

fn plain_prompt_prefix_class(prefix: &str) -> PromptPrefixClass {
    if plain_prefix_looks_prompt_like(prefix) {
        PromptPrefixClass::Plain
    } else {
        PromptPrefixClass::Other
    }
}

fn path_prompt_marker_allowed(marker: char, prefix: &str) -> bool {
    !path_prompt_is_zsh_jobs_summary(marker, prefix)
}

fn path_prompt_is_zsh_jobs_summary(marker: char, prefix: &str) -> bool {
    matches!(marker, '%') && prefix_is_zsh_jobs_summary(prefix)
}

fn plain_prompt_marker_allowed(marker: char) -> bool {
    matches!(marker, '$' | '#' | '%')
}

type PrefixRejector = fn(&str) -> bool;

const PLAIN_PROMPT_PREFIX_REJECTORS: [PrefixRejector; 4] = [
    plain_prefix_is_too_long,
    plain_prefix_has_whitespace,
    plain_prefix_is_numeric_progress,
    plain_prefix_has_invalid_chars,
];

fn plain_prefix_looks_prompt_like(prefix: &str) -> bool {
    !PLAIN_PROMPT_PREFIX_REJECTORS
        .iter()
        .any(|reject| reject(prefix))
}

fn plain_prefix_is_too_long(prefix: &str) -> bool {
    prefix.len() > 32
}

fn plain_prefix_has_whitespace(prefix: &str) -> bool {
    prefix.chars().any(|c| c.is_whitespace())
}

fn plain_prefix_is_numeric_progress(prefix: &str) -> bool {
    prefix.chars().all(is_numeric_progress_char)
}

fn is_numeric_progress_char(c: char) -> bool {
    matches!(c, '0'..='9' | '.' | ',')
}

fn plain_prefix_has_invalid_chars(prefix: &str) -> bool {
    !prefix.chars().all(is_plain_prompt_char)
}

fn is_plain_prompt_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn prefix_has_path_or_user_marker(prefix: &str) -> bool {
    prefix_contains_path_or_user_char(prefix) || prefix_has_prompt_wrapper_suffix(prefix)
}

fn prefix_contains_path_or_user_char(prefix: &str) -> bool {
    prefix.chars().any(is_path_or_user_char)
}

fn is_path_or_user_char(c: char) -> bool {
    matches!(c, '@' | ':' | '/' | '~' | '\\')
}

fn prefix_has_prompt_wrapper_suffix(prefix: &str) -> bool {
    matches!(prefix.chars().last(), Some(')' | ']'))
}

fn prefix_is_zsh_jobs_summary(prefix: &str) -> bool {
    // zsh's `%` jobs summary line ends in `... 12.34%`; reject those.
    let compact = prefix.replace(',', "");
    compact.chars().all(is_zsh_jobs_summary_char)
}

fn is_zsh_jobs_summary_char(c: char) -> bool {
    c.is_ascii_digit() || c == '.' || c.is_ascii_whitespace()
}

#[cfg(test)]
mod tests {
    use super::{
        line_looks_prompt_like, output_counts_as_meaningful_activity, visible_output_is_meaningful,
    };
    use crate::scroll::guard::ScrollOutputChunk;
    use crate::types::SessionState;

    #[test]
    fn line_looks_prompt_like_handles_common_prompt_shapes() {
        assert!(line_looks_prompt_like("$"));
        assert!(line_looks_prompt_like("user@host:/tmp/project$"));
        assert!(line_looks_prompt_like("~/repo %"));
        assert!(!line_looks_prompt_like("42%"));
        assert!(!line_looks_prompt_like("build finished successfully >"));
        assert!(!line_looks_prompt_like("123,456%"));
    }

    #[test]
    fn visible_output_ignores_prompt_only_lines() {
        assert!(!visible_output_is_meaningful(b"b@host swimmers % "));
        assert!(!visible_output_is_meaningful(b"$ "));
    }

    #[test]
    fn visible_output_detects_substantive_terminal_text() {
        assert!(visible_output_is_meaningful(
            b"checking auth middleware header parsing\n"
        ));
        assert!(visible_output_is_meaningful(
            b"test auth::login ... FAILED\n"
        ));
    }

    #[test]
    fn coalesced_redraw_does_not_count_as_meaningful_activity() {
        let chunk = ScrollOutputChunk {
            data: b"prompt repaint".to_vec(),
            coalesced_redraw: true,
        };
        assert!(!output_counts_as_meaningful_activity(
            SessionState::Idle,
            SessionState::Idle,
            &chunk,
        ));
    }

    #[test]
    fn prompt_that_finishes_busy_work_counts_as_activity() {
        let chunk = ScrollOutputChunk {
            data: b"b@host swimmers % ".to_vec(),
            coalesced_redraw: false,
        };
        assert!(output_counts_as_meaningful_activity(
            SessionState::Busy,
            SessionState::Idle,
            &chunk,
        ));
    }
}
