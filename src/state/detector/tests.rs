use super::*;
use std::sync::{Arc, Mutex};

type StateChangeLog = Arc<Mutex<Vec<(SessionState, SessionState)>>>;

/// Helper: create a detector with a recording callback.
fn detector_with_log() -> (StateDetector, StateChangeLog) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
    let mut d = StateDetector::new();
    d.on_state_change(move |new, prev, _cmd| {
        log2.lock().unwrap().push((new, prev));
    });
    (d, log)
}

#[test]
fn starts_idle() {
    let d = StateDetector::new();
    assert_eq!(d.get_state().0, SessionState::Idle);
    assert!(d.get_state().1.is_none());
    assert_eq!(d.state_evidence().cause, "initial_state");
    assert!(d.state_evidence().observed_at.is_none());
}

#[test]
fn osc133_prompt_sets_idle() {
    let (mut d, _log) = detector_with_log();
    // Force busy first so the idle transition fires.
    d.process_output(b"\x1b]133;C;cmd=ls\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);

    d.process_output(b"\x1b]133;A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);
    assert!(d.get_state().1.is_none());
}

#[test]
fn osc133_command_sets_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=cargo build\x07");
    let (state, cmd) = d.get_state();
    assert_eq!(state, SessionState::Busy);
    assert_eq!(cmd.as_deref(), Some("cargo build"));
    let evidence = d.state_evidence();
    assert_eq!(evidence.cause, "osc133_command");
    assert_eq!(evidence.confidence, StateConfidence::High);
    assert!(evidence.observed_at.is_some());
}

#[test]
fn same_state_detection_refreshes_state_evidence() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);
    let evidence = d.state_evidence();
    assert_eq!(evidence.cause, "osc133_prompt");
    assert_eq!(evidence.confidence, StateConfidence::High);
    assert!(evidence.observed_at.is_some());
}

#[test]
fn liveness_confirmation_upgrades_locally_inferred_busy_evidence() {
    let mut d = StateDetector::new();
    d.note_input();
    assert_eq!(d.get_state().0, SessionState::Busy);
    assert_eq!(d.state_evidence().cause, "local_input");
    assert_eq!(d.state_evidence().confidence, StateConfidence::Medium);

    d.apply_process_liveness(true);

    assert_eq!(d.get_state().0, SessionState::Busy);
    let evidence = d.state_evidence();
    assert_eq!(evidence.cause, "liveness_has_children");
    assert_eq!(evidence.confidence, StateConfidence::High);
    assert!(evidence.observed_at.is_some());
}

#[test]
fn consume_private_string_byte_handles_escape_and_st_terminators() {
    let mut esc_pending = false;
    let mut consumed = 0;
    assert!(
        StateDetector::consume_private_string_byte(0x1b, &mut esc_pending, &mut consumed).is_none()
    );
    assert!(esc_pending);
    assert!(matches!(
        StateDetector::consume_private_string_byte(b'\\', &mut esc_pending, &mut consumed),
        Some(EscapeState::Normal)
    ));

    esc_pending = false;
    consumed = 0;
    assert!(matches!(
        StateDetector::consume_private_string_byte(0x9c, &mut esc_pending, &mut consumed),
        Some(EscapeState::Normal)
    ));

    esc_pending = true;
    consumed = 0;
    assert!(
        StateDetector::consume_private_string_byte(b'x', &mut esc_pending, &mut consumed).is_none()
    );
    assert!(!esc_pending);
}

#[test]
fn consume_chunk_byte_dispatches_visible_csi_and_private_strings() {
    let mut d = StateDetector::new();
    let parsed = d.parse_chunk(b"before\x1b[31mred\x1b[0m\x1bPignored\x1b\\after");

    assert_eq!(parsed.visible, "beforeredafter");
    assert!(parsed.markers.is_empty());
}

#[test]
fn consume_chunk_byte_records_osc_marker_at_visible_offset() {
    let mut d = StateDetector::new();
    let parsed = d.parse_chunk(b"pre\x1b]133;C;cmd=ls\x07post");

    assert_eq!(parsed.visible, "prepost");
    assert_eq!(parsed.markers.len(), 1);
    assert_eq!(parsed.markers[0].visible_offset, 3);
    assert!(matches!(
        parsed.markers[0].marker,
        Osc133Marker::Command(ref command) if command == "ls"
    ));
}

#[test]
fn error_pattern_detected() {
    let (mut d, log) = detector_with_log();
    // Force to busy first so error is a real transition.
    d.process_output(b"\x1b]133;C;cmd=foo\x07");
    d.process_output(b"bash: foo: command not found\n");
    assert_eq!(d.get_state().0, SessionState::Error);

    let transitions = log.lock().unwrap();
    assert!(transitions
        .iter()
        .any(|(new, _)| *new == SessionState::Error));
}

#[test]
fn rust_panic_output_enters_error_state() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=cargo run\x07");

    d.process_output(b"thread 'main' panicked at src/main.rs:2:5:\nboom\n");

    assert_eq!(d.get_state().0, SessionState::Error);
}

#[test]
fn osc_command_and_error_text_in_same_chunk_enters_error() {
    let mut d = StateDetector::new();

    d.process_output(b"\x1b]133;C;cmd=foo\x07bash: foo: command not found\n");

    assert_eq!(d.get_state().0, SessionState::Error);
}

#[test]
fn osc_command_error_and_prompt_in_same_chunk_enters_error() {
    let mut d = StateDetector::new();

    d.process_output(b"\x1b]133;C;cmd=foo\x07bash: foo: command not found\n\x1b]133;A\x07");

    assert_eq!(d.get_state().0, SessionState::Error);
}

#[test]
fn osc_new_command_after_prompt_ignores_prior_error_text_in_same_chunk() {
    let mut d = StateDetector::new();

    d.process_output(
        b"\x1b]133;C;cmd=foo\x07bash: foo: command not found\n\x1b]133;A\x07user@host:~$ foo\n\x1b]133;C;cmd=bar\x07",
    );

    let (state, cmd) = d.get_state();
    assert_eq!(state, SessionState::Busy);
    assert_eq!(cmd.as_deref(), Some("bar"));
}

#[test]
fn error_auto_clears_after_deadline() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=foo\x07");
    d.process_output(b"Permission denied\n");
    assert_eq!(d.get_state().0, SessionState::Error);

    // Simulate time passing beyond the error linger duration.
    d.error_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn check_timers_leaves_unexpired_deadlines_pending() {
    let mut d = StateDetector::new();
    let now = Instant::now();
    let future = now + Duration::from_millis(10);
    d.error_deadline = Some(future);
    d.output_idle_deadline = Some(future);
    d.attention_deadline = Some(future);

    d.check_timers(now);

    assert_eq!(d.error_deadline, Some(future));
    assert_eq!(d.output_idle_deadline, Some(future));
    assert_eq!(d.attention_deadline, Some(future));
    assert_eq!(d.state(), SessionState::Idle);
    assert_eq!(d.state_evidence().cause, "initial_state");
}

#[test]
fn check_timers_clears_expired_deadlines_without_non_matching_transitions() {
    let mut d = StateDetector::new();
    let now = Instant::now();
    let expired = now - Duration::from_millis(1);
    d.error_deadline = Some(expired);
    d.output_idle_deadline = Some(expired);

    d.check_timers(now);

    assert!(d.error_deadline.is_none());
    assert!(d.output_idle_deadline.is_none());
    assert_eq!(d.state(), SessionState::Idle);
    assert_eq!(d.state_evidence().cause, "initial_state");
}

#[test]
fn check_timers_preserves_error_output_attention_order() {
    let mut d = StateDetector::new();
    let now = Instant::now();
    let expired = now - Duration::from_millis(1);
    d.state = SessionState::Error;
    d.error_deadline = Some(expired);
    d.output_idle_deadline = Some(expired);
    d.attention_deadline = Some(expired);

    d.check_timers(now);

    assert!(d.error_deadline.is_none());
    assert!(d.output_idle_deadline.is_none());
    assert!(d.attention_deadline.is_none());
    assert_eq!(d.state(), SessionState::Attention);
    assert_eq!(d.state_evidence().cause, "attention_timer_expired");
}

#[test]
fn fallback_error_and_prompt_same_chunk_returns_idle() {
    let mut d = StateDetector::new();

    d.process_output(b"bash: foo: command not found\nuser@host:~$ ");

    assert_eq!(d.get_state().0, SessionState::Idle);
    assert!(d.error_deadline.is_none());
}

#[test]
fn busy_to_idle_schedules_attention() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=sleep 1\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);
    assert!(d.attention_deadline.is_none());

    d.process_output(b"\x1b]133;A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);
    assert!(d.attention_deadline.is_some());
}

#[test]
fn attention_fires_after_deadline() {
    let (mut d, log) = detector_with_log();
    d.process_output(b"\x1b]133;C;cmd=make\x07");
    d.process_output(b"\x1b]133;A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);

    // Expire the attention deadline.
    d.attention_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.get_state().0, SessionState::Attention);

    let transitions = log.lock().unwrap();
    assert!(transitions
        .iter()
        .any(|(new, _)| *new == SessionState::Attention));
}

#[test]
fn idle_to_idle_does_not_cancel_attention_timer() {
    let mut d = StateDetector::new();
    // busy -> idle starts attention timer
    d.process_output(b"\x1b]133;C;cmd=ls\x07");
    d.process_output(b"\x1b]133;A\x07");
    assert!(d.attention_deadline.is_some());

    // Another idle detection must NOT clear the timer.
    d.process_output(b"\x1b]133;A\x07");
    assert!(d.attention_deadline.is_some());
}

#[test]
fn idle_to_busy_cancels_attention_timer() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=ls\x07");
    d.process_output(b"\x1b]133;A\x07");
    assert!(d.attention_deadline.is_some());

    d.process_output(b"\x1b]133;C;cmd=pwd\x07");
    assert!(d.attention_deadline.is_none());
}

#[test]
fn dismiss_attention_returns_to_idle() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=make\x07");
    d.process_output(b"\x1b]133;A\x07");
    d.attention_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.get_state().0, SessionState::Attention);

    d.dismiss_attention();
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn fallback_prompt_detection() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=echo hi\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);

    d.process_output(b"hi\nuser@host:~$ ");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn strip_ansi_removes_sequences() {
    let input = "\x1b[32mgreen\x1b[0m \x1b]0;title\x07 \x1b]133;A\x1b\\ \x1b(B \x1b> \x1b[?25h \x01\x02hello";
    let result = StateDetector::strip_ansi(input);
    assert!(!result.contains('\x1b'));
    assert!(!result.contains("[?25h"));
    assert!(result.contains("green"));
    assert!(result.contains("hello"));
}

#[test]
fn private_mode_control_sequences_do_not_mark_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b[?12l\x1b[?25h");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn prompt_with_private_mode_sequences_returns_to_idle() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=ls\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);

    d.process_output(b"\x1b[?25huser@host:~$ ");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn callback_fires_on_state_change_not_on_same_state() {
    let (mut d, log) = detector_with_log();
    // idle -> idle: no callback
    d.process_output(b"\x1b]133;A\x07");
    let transitions = log.lock().unwrap();
    assert!(transitions.is_empty());
}

#[test]
fn non_prompt_output_sets_busy_without_osc() {
    let mut d = StateDetector::new();
    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.get_state().0, SessionState::Busy);
}

#[test]
fn prompt_only_output_stays_idle_without_osc() {
    let mut d = StateDetector::new();
    d.process_output(b"user@host:~$ ");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn note_input_sets_busy_and_clears_attention_timer() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=ls\x07");
    d.process_output(b"\x1b]133;A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);
    assert!(d.attention_deadline.is_some());

    d.note_input();
    assert_eq!(d.get_state().0, SessionState::Busy);
    assert!(d.attention_deadline.is_none());
}

#[test]
fn progress_percentage_does_not_look_like_prompt() {
    assert!(!StateDetector::looks_like_prompt("42%"));
    assert!(!StateDetector::looks_like_prompt("downloading 100%"));
    assert!(StateDetector::looks_like_prompt("user@host:~$ "));
}

#[test]
fn looks_like_prompt_empty_and_whitespace_only() {
    assert!(!StateDetector::looks_like_prompt(""));
    assert!(!StateDetector::looks_like_prompt("   "));
    assert!(!StateDetector::looks_like_prompt("\n\n\r\n"));
}

#[test]
fn looks_like_prompt_no_trailing_marker() {
    assert!(!StateDetector::looks_like_prompt("hello world"));
    assert!(!StateDetector::looks_like_prompt("user@host:~"));
    assert!(!StateDetector::looks_like_prompt("building..."));
}

#[test]
fn looks_like_prompt_minimal_prompts_accept_dollar_hash_percent() {
    // Bare marker only: prefix is empty → return true
    assert!(StateDetector::looks_like_prompt("$"));
    assert!(StateDetector::looks_like_prompt("#"));
    assert!(StateDetector::looks_like_prompt("%"));
    // Minimal identifier prefix
    assert!(StateDetector::looks_like_prompt("project$"));
    assert!(StateDetector::looks_like_prompt("myhost#"));
    assert!(StateDetector::looks_like_prompt("zsh%"));
}

#[test]
fn looks_like_prompt_gt_marker_rejected_for_minimal_prefix() {
    // `>` with non-empty minimal prefix is rejected (not in the minimal-prompt allow list)
    assert!(!StateDetector::looks_like_prompt("project>"));
    // `>` alone has empty prefix → returns true (same as any bare marker)
    assert!(StateDetector::looks_like_prompt(">"));
}

#[test]
fn looks_like_prompt_prefix_too_long() {
    let long = "a".repeat(33) + "$";
    assert!(!StateDetector::looks_like_prompt(&long));
    // 32 chars is the boundary: still passes
    let boundary = "a".repeat(32) + "$";
    assert!(StateDetector::looks_like_prompt(&boundary));
}

#[test]
fn looks_like_prompt_prefix_with_whitespace_rejected() {
    assert!(!StateDetector::looks_like_prompt("my host$"));
    assert!(!StateDetector::looks_like_prompt("a b$"));
}

#[test]
fn looks_like_prompt_prefix_all_digits_dots_commas_rejected() {
    assert!(!StateDetector::looks_like_prompt("1.2.3$"));
    assert!(!StateDetector::looks_like_prompt("100,000$"));
}

#[test]
fn looks_like_prompt_prefix_with_invalid_chars_rejected() {
    // `!` is not in the allowed minimal-prefix charset
    assert!(!StateDetector::looks_like_prompt("host!$"));
    // Pipe is not in the allowed charset
    assert!(!StateDetector::looks_like_prompt("foo|bar$"));
    // `@` triggers has_prompt_context → accepted as a real prompt
    assert!(StateDetector::looks_like_prompt("host@domain$"));
}

#[test]
fn looks_like_prompt_multiline_uses_last_nonempty_line() {
    // Only the last non-empty line is inspected
    assert!(StateDetector::looks_like_prompt(
        "some output\nuser@host:~$ "
    ));
    assert!(!StateDetector::looks_like_prompt(
        "user@host:~$ \nsome output\n"
    ));
    // Trailing blank lines are skipped to find the prompt line
    assert!(StateDetector::looks_like_prompt("user@host:~$ \n\n  \n"));
}

#[test]
fn looks_like_prompt_crlf_multiline() {
    assert!(StateDetector::looks_like_prompt("output\r\nuser@host:~$ "));
}

#[test]
fn looks_like_prompt_gt_with_context_is_accepted() {
    // `>` is accepted when prefix has prompt context (e.g. contains `@`)
    assert!(StateDetector::looks_like_prompt("user@host>"));
    assert!(StateDetector::looks_like_prompt("~/project>"));
}

#[test]
fn looks_like_prompt_percent_numeric_context_path() {
    // has_prompt_context + compact prefix is all digits/dots/spaces → rejected as progress
    // This requires a prefix that (a) contains a context char and
    // (b) after comma removal is purely digit/dot/whitespace.
    // The classic "% progress in shell" case uses no context chars so the
    // early whitespace/digit guards catch it; this test covers the context path.
    // "42%" — no context, caught by all-digits guard
    assert!(!StateDetector::looks_like_prompt("42%"));
    // "user@host: 99.5%" — has context but prefix has alpha chars → still a prompt
    assert!(StateDetector::looks_like_prompt("user@host: 99.5%"));
    // "user@host:~$ " — canonical prompt with context, dollar marker
    assert!(StateDetector::looks_like_prompt("user@host:~$ "));
}

#[test]
fn fallback_prompt_detection_ignores_percent_progress() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=curl\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);

    d.process_output(b" 42%");
    assert_eq!(d.get_state().0, SessionState::Busy);
}

#[test]
fn osc133_command_with_st_terminator_sets_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=git status\x1b\\");
    let (state, cmd) = d.get_state();
    assert_eq!(state, SessionState::Busy);
    assert_eq!(cmd.as_deref(), Some("git status"));
}

#[test]
fn osc133_uses_last_marker_when_chunk_contains_both() {
    let mut d = StateDetector::new();

    d.process_output(b"\x1b]133;A\x07\x1b]133;C;cmd=ls\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);

    d.process_output(b"\x1b]133;C;cmd=ls\x07\x1b]133;A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn esc_charset_sequence_does_not_mark_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b(B");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn split_esc_charset_sequence_does_not_mark_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b(");
    d.process_output(b"B");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn osc133_c1_st_terminator_sets_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x9d133;C;cmd=git status\x9c");
    let (state, cmd) = d.get_state();
    assert_eq!(state, SessionState::Busy);
    assert_eq!(cmd.as_deref(), Some("git status"));
}

#[test]
fn split_private_mode_sequence_does_not_mark_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b[");
    d.process_output(b"?2004h");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn split_osc133_command_across_chunks_sets_busy() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=git");
    d.process_output(b" status\x07");
    let (state, cmd) = d.get_state();
    assert_eq!(state, SessionState::Busy);
    assert_eq!(cmd.as_deref(), Some("git status"));
}

#[test]
fn split_osc133_prompt_across_chunks_sets_idle() {
    let mut d = StateDetector::new();
    d.process_output(b"\x1b]133;C;cmd=ls\x07");
    assert_eq!(d.get_state().0, SessionState::Busy);
    d.process_output(b"\x1b]133;");
    d.process_output(b"A\x07");
    assert_eq!(d.get_state().0, SessionState::Idle);
}

#[test]
fn unterminated_osc_recovers_after_limit() {
    let mut d = StateDetector::new();
    let mut chunk = Vec::from(&b"\x1b]"[..]);
    chunk.extend(std::iter::repeat_n(
        b'x',
        TERMINAL_STRING_RECOVERY_BYTES + 1,
    ));
    chunk.extend_from_slice(b"user@host:~$ ");

    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.state(), SessionState::Busy);

    d.process_output(&chunk);
    assert_eq!(d.state(), SessionState::Idle);
}

#[test]
fn unterminated_private_string_recovers_after_limit() {
    let mut d = StateDetector::new();
    let mut chunk = Vec::from(&b"\x1bP"[..]);
    chunk.extend(std::iter::repeat_n(
        b'x',
        TERMINAL_STRING_RECOVERY_BYTES + 1,
    ));
    chunk.extend_from_slice(b"user@host:~$ ");

    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.state(), SessionState::Busy);

    d.process_output(&chunk);
    assert_eq!(d.state(), SessionState::Idle);
}

#[test]
fn unterminated_csi_recovers_after_limit() {
    // Adversarial input: ESC `[` followed by bytes outside CSI's accepted
    // ranges (param 0x30-0x3f, intermediate 0x20-0x2f, final 0x40-0x7e).
    // Without the consumed cap the parser stays in CSI forever and drops
    // every byte, so the prompt that follows is never seen.
    let mut d = StateDetector::new();
    let mut chunk = Vec::from(&b"\x1b["[..]);
    chunk.extend(std::iter::repeat_n(
        0x00u8,
        TERMINAL_STRING_RECOVERY_BYTES + 1,
    ));
    chunk.extend_from_slice(b"user@host:~$ ");

    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.state(), SessionState::Busy);

    d.process_output(&chunk);
    assert_eq!(d.state(), SessionState::Idle);
}

#[test]
fn unterminated_escape_intermediate_recovers_after_limit() {
    // ESC followed by an unbounded run of intermediate bytes (0x20-0x2f)
    // would never reset under the previous code. The recovery cap forces
    // a return to Normal so subsequent prompt bytes are detected.
    let mut d = StateDetector::new();
    let mut chunk = Vec::from(&b"\x1b "[..]);
    chunk.extend(std::iter::repeat_n(
        0x24u8,
        TERMINAL_STRING_RECOVERY_BYTES + 1,
    ));
    chunk.extend_from_slice(b"user@host:~$ ");

    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.state(), SessionState::Busy);

    d.process_output(&chunk);
    assert_eq!(d.state(), SessionState::Idle);
}

// --- TUI tool mode tests ---

#[test]
fn tui_tool_mode_output_silence_sets_idle() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    // Feed visible output to go busy via fallback.
    d.process_output(b"Thinking...\r\n");
    assert_eq!(d.state(), SessionState::Busy);
    assert!(d.output_idle_deadline.is_some());

    // Expire the output idle deadline.
    d.output_idle_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Idle);
}

#[test]
fn tui_tool_mode_output_resets_deadline() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    d.process_output(b"Working...\r\n");
    assert_eq!(d.state(), SessionState::Busy);
    let first_deadline = d.output_idle_deadline.unwrap();

    // Advance time slightly and feed more output.
    std::thread::sleep(Duration::from_millis(10));
    d.process_output(b"Still working...\r\n");
    let second_deadline = d.output_idle_deadline.unwrap();

    // Deadline should have been pushed forward.
    assert!(second_deadline > first_deadline);
}

#[test]
fn tui_tool_mode_idle_to_busy_on_output() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    // Go busy then idle via silence.
    d.process_output(b"Thinking...\r\n");
    d.output_idle_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Idle);

    // New visible output should go back to busy.
    d.process_output(b"Agent response output\r\n");
    assert_eq!(d.state(), SessionState::Busy);
    assert!(d.output_idle_deadline.is_some());
}

#[test]
fn tui_tool_mode_attention_after_idle() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    // Go busy via output.
    d.process_output(b"Processing...\r\n");
    assert_eq!(d.state(), SessionState::Busy);

    // Silence -> idle (triggers attention timer via busy->idle in set_state).
    d.output_idle_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Idle);
    assert!(d.attention_deadline.is_some());

    // Expire attention deadline -> attention.
    d.attention_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Attention);
}

#[test]
fn tui_tool_mode_note_input_starts_deadline() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    // Start idle, note_input should go busy AND set output_idle_deadline.
    assert_eq!(d.state(), SessionState::Idle);
    d.note_input();
    assert_eq!(d.state(), SessionState::Busy);
    assert!(d.output_idle_deadline.is_some());
}

#[test]
fn tui_tool_mode_disabled_no_deadline() {
    let mut d = StateDetector::new();
    // TUI mode is off by default.
    assert!(!d.tui_tool_mode);
    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.state(), SessionState::Busy);
    // No output idle deadline should be set.
    assert!(d.output_idle_deadline.is_none());
}

#[test]
fn tui_tool_mode_liveness_keeps_idle_when_tool_is_waiting() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    assert_eq!(d.state(), SessionState::Idle);

    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Idle);
}

#[test]
fn tui_tool_mode_liveness_keeps_attention_when_tool_is_waiting() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);
    d.process_output(b"Processing...\r\n");
    d.output_idle_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Idle);

    d.attention_deadline = Some(Instant::now() - Duration::from_millis(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Attention);

    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Attention);
}

#[test]
fn tui_tool_mode_liveness_does_not_upgrade_busy_confidence_from_waiting_child() {
    let mut d = StateDetector::new();
    d.set_tui_tool_mode(true);

    d.note_input();
    assert_eq!(d.state(), SessionState::Busy);
    assert_eq!(d.state_evidence().cause, "local_input");
    assert_eq!(d.state_evidence().confidence, StateConfidence::Medium);

    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Busy);
    assert_eq!(d.state_evidence().cause, "local_input");
    assert_eq!(d.state_evidence().confidence, StateConfidence::Medium);
}

// --- Process liveness reconciliation tests ---

#[test]
fn liveness_corrects_busy_to_idle_when_no_children() {
    let (mut d, log) = detector_with_log();
    // Drive to Busy via output.
    d.process_output(b"Compiling...\r\n");
    assert_eq!(d.state(), SessionState::Busy);

    // Process tree says: no children.
    d.apply_process_liveness(false);
    assert_eq!(d.state(), SessionState::Idle);

    let transitions = log.lock().unwrap();
    let last = transitions.last().unwrap();
    assert_eq!(last.0, SessionState::Idle);
    assert_eq!(last.1, SessionState::Busy);
}

#[test]
fn liveness_corrects_idle_to_busy_when_has_children() {
    let (mut d, log) = detector_with_log();
    assert_eq!(d.state(), SessionState::Idle);

    // Process tree says: children running.
    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Busy);

    let transitions = log.lock().unwrap();
    let last = transitions.last().unwrap();
    assert_eq!(last.0, SessionState::Busy);
    assert_eq!(last.1, SessionState::Idle);
}

#[test]
fn liveness_corrects_attention_to_busy_when_has_children() {
    let mut d = StateDetector::new();
    // Drive to Busy then Idle then Attention.
    d.process_output(b"Compiling...\r\n");
    d.process_output(b"user@host:~$ ");
    assert_eq!(d.state(), SessionState::Idle);
    // Simulate attention deadline expiring.
    d.attention_deadline = Some(Instant::now() - Duration::from_secs(1));
    d.check_timers(Instant::now());
    assert_eq!(d.state(), SessionState::Attention);

    // Process tree says: children running.
    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Busy);
}

#[test]
fn liveness_does_not_override_exited() {
    let mut d = StateDetector::new();
    d.mark_exited();
    assert_eq!(d.state(), SessionState::Exited);

    d.apply_process_liveness(false);
    assert_eq!(d.state(), SessionState::Exited);

    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Exited);
}

#[test]
fn liveness_does_not_override_error() {
    let mut d = StateDetector::new();
    d.process_output(b"command not found\r\n");
    assert_eq!(d.state(), SessionState::Error);

    d.apply_process_liveness(false);
    assert_eq!(d.state(), SessionState::Error);
}

#[test]
fn liveness_noop_when_state_matches_tree() {
    let (mut d, log) = detector_with_log();
    // Idle with no children — no transition expected.
    d.apply_process_liveness(false);
    assert_eq!(d.state(), SessionState::Idle);

    // Busy with children — no transition expected.
    d.process_output(b"Compiling...\r\n");
    let count_before = log.lock().unwrap().len();
    d.apply_process_liveness(true);
    assert_eq!(d.state(), SessionState::Busy);
    assert_eq!(log.lock().unwrap().len(), count_before);
}

#[test]
fn log_excerpt_truncates_multibyte_without_panic() {
    // A byte-wise truncate(140) would panic if the 140th byte fell inside
    // a multi-byte sequence. Char-aware truncation must stay panic-free and
    // cap the result at 140 chars plus the ellipsis.
    let multibyte = "é".repeat(200);
    let excerpt = StateDetector::log_excerpt(&multibyte);
    assert_eq!(excerpt.chars().count(), 141); // 140 chars + '…'
    assert!(excerpt.ends_with('…'));

    // Short ASCII input is returned verbatim (no ellipsis).
    assert_eq!(StateDetector::log_excerpt("ready$"), "ready$");
}
