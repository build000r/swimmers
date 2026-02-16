// State detector -- classifies terminal state as idle, busy, error, or attention.
// Uses OSC 133 shell integration sequences when available, falls back to regex.


use std::time::{Duration, Instant};

use regex::Regex;

use crate::types::SessionState;

const ERROR_LINGER_MS: u64 = 4000;
const ATTENTION_DELAY_MS: u64 = 10000;

/// Callback signature for state change notifications.
/// Arguments: new_state, previous_state, current_command.
pub type StateChangeCallback =
    Box<dyn Fn(SessionState, SessionState, Option<String>) + Send + 'static>;

pub struct StateDetector {
    state: SessionState,
    current_command: Option<String>,
    prompt_pattern: Regex,
    error_patterns: Vec<Regex>,
    /// Deadline at which an active error state should auto-clear to idle.
    error_deadline: Option<Instant>,
    /// Deadline at which idle should transition to attention.
    attention_deadline: Option<Instant>,
    on_state_change: Option<StateChangeCallback>,
}

impl StateDetector {
    pub fn new() -> Self {
        Self {
            state: SessionState::Idle,
            current_command: None,
            prompt_pattern: Regex::new(r"[$%>#]\s*$").expect("prompt_pattern is valid"),
            error_patterns: vec![
                Regex::new(r"(?i)command not found").expect("error pattern is valid"),
                Regex::new(r"(?i)Permission denied").expect("error pattern is valid"),
                Regex::new(r"(?i)segmentation fault").expect("error pattern is valid"),
                Regex::new(r"panic:").expect("error pattern is valid"),
            ],
            error_deadline: None,
            attention_deadline: None,
            on_state_change: None,
        }
    }

    /// Register a callback invoked on every state transition.
    pub fn on_state_change<F>(&mut self, cb: F)
    where
        F: Fn(SessionState, SessionState, Option<String>) + Send + 'static,
    {
        self.on_state_change = Some(Box::new(cb));
    }

    /// Shell integration init script injected into zsh on session start.
    /// Returns OSC 133 sequences for prompt/command boundaries.
    pub fn shell_integration_script() -> &'static str {
        "precmd() { printf '\\e]133;A\\a' }; preexec() { printf '\\e]133;C;cmd=%s\\a' \"$1\" }"
    }

    /// Strip ANSI/CSI/OSC escape sequences to get visible text for pattern matching.
    /// OSC 133 sequences are checked separately against raw output first.
    pub fn strip_ansi(s: &str) -> String {
        // Lazily compiled regexes -- these are only constructed once per call site
        // thanks to regex crate's internal optimizations. For a truly hot path you
        // could hoist them into OnceLock, but clarity wins here.
        let csi = Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap();
        let osc = Regex::new(r"\x1b\][^\x07]*\x07").unwrap();
        let charset = Regex::new(r"\x1b[()][0-9A-B]").unwrap();
        let mode = Regex::new(r"\x1b[>=<]").unwrap();
        let ctrl = Regex::new(r"[\x00-\x08\x0e-\x1f]").unwrap();

        let s = csi.replace_all(s, "");
        let s = osc.replace_all(&s, "");
        let s = charset.replace_all(&s, "");
        let s = mode.replace_all(&s, "");
        let s = ctrl.replace_all(&s, "");
        s.into_owned()
    }

    /// Process raw PTY output bytes, detecting state transitions.
    ///
    /// This is the main entry point called from the session actor's output loop.
    /// Timer-based transitions (error auto-clear, idle->attention) are checked
    /// at the start of each call using Instant deadlines.
    pub fn process_output(&mut self, data: &[u8]) {
        let now = Instant::now();

        // Check timer deadlines before processing new output.
        self.check_timers(now);

        let text = String::from_utf8_lossy(data);

        // Check for OSC 133 sequences in raw output.
        // Prompt shown (A) -> idle
        if text.contains("\x1b]133;A") {
            self.clear_error_timer();
            self.set_state(SessionState::Idle, Some(None), now);
            return;
        }

        // Command starting (C) with command name.
        if let Some(cmd) = Self::extract_osc133_command(&text) {
            self.clear_error_timer();
            self.set_state(SessionState::Busy, Some(Some(cmd)), now);
            return;
        }

        // Strip escape sequences for heuristic pattern matching.
        let visible = Self::strip_ansi(&text);

        // Check for error patterns against visible text.
        let mut found_error = false;
        for pattern in &self.error_patterns {
            if pattern.is_match(&visible) {
                self.set_state(SessionState::Error, None, now);
                self.schedule_error_clear(now);
                found_error = true;
                break;
            }
        }

        // Fallback: regex prompt detection (for shells without OSC 133).
        // Recovers from busy AND error states when a new prompt appears.
        // Checked against visible text so tmux escape sequences don't mask the prompt.
        if self.state != SessionState::Idle
            && !found_error
            && self.prompt_pattern.is_match(&visible)
        {
            self.clear_error_timer();
            self.set_state(SessionState::Idle, Some(None), now);
        }
    }

    /// Check and fire any expired timer deadlines. Called at the start of
    /// each `process_output` and can also be called independently by the
    /// actor's timer loop.
    pub fn check_timers(&mut self, now: Instant) {
        // Error auto-clear
        if let Some(deadline) = self.error_deadline {
            if now >= deadline {
                self.error_deadline = None;
                if self.state == SessionState::Error {
                    self.set_state(SessionState::Idle, Some(None), now);
                }
            }
        }

        // Attention promotion
        if let Some(deadline) = self.attention_deadline {
            if now >= deadline {
                self.attention_deadline = None;
                if self.state == SessionState::Idle {
                    self.set_state(SessionState::Attention, Some(None), now);
                }
            }
        }
    }

    /// Returns the next Instant at which a timer will fire, if any.
    /// Useful for the actor to know when to schedule a wake-up.
    pub fn next_deadline(&self) -> Option<Instant> {
        match (self.error_deadline, self.attention_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Dismiss the attention state, returning to idle.
    pub fn dismiss_attention(&mut self) {
        self.clear_attention_timer();
        if self.state == SessionState::Attention {
            let now = Instant::now();
            self.set_state(SessionState::Idle, Some(None), now);
        }
    }

    /// Get the current state and command as a tuple.
    pub fn get_state(&self) -> (SessionState, Option<String>) {
        (self.state, self.current_command.clone())
    }

    /// Return the current session state.
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// Return the current command, if any.
    pub fn current_command(&self) -> Option<String> {
        self.current_command.clone()
    }

    /// Alias for `process_output` -- accepts a byte slice from the scroll guard
    /// output pipeline. This is the name used by the session actor.
    pub fn feed(&mut self, data: &[u8]) {
        self.process_output(data);
    }

    // --- Private helpers ---

    /// Extract the command name from an OSC 133;C sequence.
    fn extract_osc133_command(text: &str) -> Option<String> {
        let re = Regex::new(r"\x1b\]133;C;cmd=([^\x07]*)\x07").unwrap();
        re.captures(text).map(|caps| caps[1].trim().to_string())
    }

    /// Core state transition logic. Mirrors the JS `_setState` method.
    ///
    /// `command_update`:
    ///  - `None` -- do not touch current_command (used for error transitions)
    ///  - `Some(None)` -- clear current_command to None
    ///  - `Some(Some(cmd))` -- set current_command to cmd
    fn set_state(
        &mut self,
        new_state: SessionState,
        command_update: Option<Option<String>>,
        _now: Instant,
    ) {
        let prev = self.state;
        self.state = new_state;

        if let Some(cmd) = command_update {
            self.current_command = cmd;
        }

        // Start attention timer when transitioning from busy -> idle
        if new_state == SessionState::Idle && prev == SessionState::Busy {
            self.schedule_attention();
        }

        // Cancel attention timer if leaving idle for anything other than attention.
        // idle -> idle re-detections must NOT cancel the timer.
        if prev == SessionState::Idle
            && new_state != SessionState::Attention
            && new_state != SessionState::Idle
        {
            self.clear_attention_timer();
        }

        if new_state != prev {
            if let Some(ref cb) = self.on_state_change {
                cb(new_state, prev, self.current_command.clone());
            }
        }
    }

    fn schedule_error_clear(&mut self, now: Instant) {
        self.error_deadline = Some(now + Duration::from_millis(ERROR_LINGER_MS));
    }

    fn clear_error_timer(&mut self) {
        self.error_deadline = None;
    }

    fn schedule_attention(&mut self) {
        self.attention_deadline = Some(Instant::now() + Duration::from_millis(ATTENTION_DELAY_MS));
    }

    fn clear_attention_timer(&mut self) {
        self.attention_deadline = None;
    }
}

impl Default for StateDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Helper: create a detector with a recording callback.
    fn detector_with_log() -> (StateDetector, Arc<Mutex<Vec<(SessionState, SessionState)>>>) {
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
    }

    #[test]
    fn osc133_prompt_sets_idle() {
        let (mut d, _log) = detector_with_log();
        // Force busy first so the idle transition fires.
        d.process_output(b"\x1b]133;C;cmd=ls\x07");
        assert_eq!(d.get_state().0, SessionState::Busy);

        d.process_output(b"\x1b]133;A");
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
    fn busy_to_idle_schedules_attention() {
        let mut d = StateDetector::new();
        d.process_output(b"\x1b]133;C;cmd=sleep 1\x07");
        assert_eq!(d.get_state().0, SessionState::Busy);
        assert!(d.attention_deadline.is_none());

        d.process_output(b"\x1b]133;A");
        assert_eq!(d.get_state().0, SessionState::Idle);
        assert!(d.attention_deadline.is_some());
    }

    #[test]
    fn attention_fires_after_deadline() {
        let (mut d, log) = detector_with_log();
        d.process_output(b"\x1b]133;C;cmd=make\x07");
        d.process_output(b"\x1b]133;A");
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
        d.process_output(b"\x1b]133;A");
        assert!(d.attention_deadline.is_some());

        // Another idle detection must NOT clear the timer.
        d.process_output(b"\x1b]133;A");
        assert!(d.attention_deadline.is_some());
    }

    #[test]
    fn idle_to_busy_cancels_attention_timer() {
        let mut d = StateDetector::new();
        d.process_output(b"\x1b]133;C;cmd=ls\x07");
        d.process_output(b"\x1b]133;A");
        assert!(d.attention_deadline.is_some());

        d.process_output(b"\x1b]133;C;cmd=pwd\x07");
        assert!(d.attention_deadline.is_none());
    }

    #[test]
    fn dismiss_attention_returns_to_idle() {
        let mut d = StateDetector::new();
        d.process_output(b"\x1b]133;C;cmd=make\x07");
        d.process_output(b"\x1b]133;A");
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
        let input = "\x1b[32mgreen\x1b[0m \x1b]0;title\x07 \x1b(B \x1b> \x01\x02hello";
        let result = StateDetector::strip_ansi(input);
        assert_eq!(result, "green    hello");
    }

    #[test]
    fn callback_fires_on_state_change_not_on_same_state() {
        let (mut d, log) = detector_with_log();
        // idle -> idle: no callback
        d.process_output(b"\x1b]133;A");
        let transitions = log.lock().unwrap();
        assert!(transitions.is_empty());
    }
}
