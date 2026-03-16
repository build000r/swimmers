// State detector -- classifies terminal state as idle, busy, error, or attention.
// Uses OSC 133 shell integration sequences when available, falls back to regex.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use regex::Regex;
use tracing::debug;

use crate::types::SessionState;

const ERROR_LINGER_MS: u64 = 4000;
const ATTENTION_DELAY_MS: u64 = 300000;
const OUTPUT_IDLE_MS: u64 = 5000;

/// Callback signature for state change notifications.
/// Arguments: new_state, previous_state, current_command.
pub type StateChangeCallback =
    Box<dyn Fn(SessionState, SessionState, Option<String>) + Send + 'static>;

pub struct StateDetector {
    state: SessionState,
    current_command: Option<String>,
    error_patterns: Vec<Regex>,
    escape_state: EscapeState,
    /// Deadline at which an active error state should auto-clear to idle.
    error_deadline: Option<Instant>,
    /// Deadline at which idle should transition to attention.
    attention_deadline: Option<Instant>,
    /// When true, use PTY output silence to detect idle instead of prompt detection.
    /// Enabled when a TUI tool (Claude Code, Codex, etc.) is running.
    tui_tool_mode: bool,
    /// Deadline at which a busy TUI tool session should transition to idle
    /// due to output silence.
    output_idle_deadline: Option<Instant>,
    on_state_change: Option<StateChangeCallback>,
}

impl StateDetector {
    pub fn new() -> Self {
        Self {
            state: SessionState::Idle,
            current_command: None,
            error_patterns: vec![
                Regex::new(r"(?i)command not found").expect("error pattern is valid"),
                Regex::new(r"(?i)Permission denied").expect("error pattern is valid"),
                Regex::new(r"(?i)segmentation fault").expect("error pattern is valid"),
                Regex::new(r"panic:").expect("error pattern is valid"),
            ],
            escape_state: EscapeState::Normal,
            error_deadline: None,
            attention_deadline: None,
            tui_tool_mode: false,
            output_idle_deadline: None,
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

    /// Enable or disable TUI tool mode. When enabled, PTY output silence
    /// is used to detect idle instead of prompt regex detection.
    pub fn set_tui_tool_mode(&mut self, enabled: bool) {
        self.tui_tool_mode = enabled;
        if !enabled {
            self.output_idle_deadline = None;
        }
    }

    /// Shell integration init script injected into zsh on session start.
    /// Returns OSC 133 sequences for prompt/command boundaries.
    #[allow(dead_code)]
    pub fn shell_integration_script() -> &'static str {
        "precmd() { printf '\\e]133;A\\a' }; preexec() { printf '\\e]133;C;cmd=%s\\a' \"$1\" }"
    }

    /// Strip ANSI/CSI/OSC escape sequences to get visible text for pattern matching.
    /// OSC 133 sequences are checked separately against raw output first.
    pub fn strip_ansi(s: &str) -> String {
        // Strip CSI (including private-mode params like ?25h), OSC (BEL or ST),
        // other 2-byte ESC sequences, then remaining control chars.
        static CSI_RE: OnceLock<Regex> = OnceLock::new();
        static OSC_RE: OnceLock<Regex> = OnceLock::new();
        static ESC_RE: OnceLock<Regex> = OnceLock::new();
        static CTRL_RE: OnceLock<Regex> = OnceLock::new();

        let csi = CSI_RE
            .get_or_init(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").expect("csi regex is valid"));
        let osc = OSC_RE.get_or_init(|| {
            Regex::new(r"(?s)\x1b\].*?(?:\x07|\x1b\\)").expect("osc regex is valid")
        });
        let esc = ESC_RE.get_or_init(|| Regex::new(r"\x1b[@-Z\\-_]").expect("esc regex is valid"));
        let ctrl = CTRL_RE
            .get_or_init(|| Regex::new(r"[\x00-\x08\x0B-\x1F\x7F]").expect("ctrl regex is valid"));

        let s = csi.replace_all(s, "");
        let s = osc.replace_all(&s, "");
        let s = esc.replace_all(&s, "");
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

        let ParsedChunk { visible, markers } = self.parse_chunk(data);

        // Check for OSC 133 prompt/command markers.
        // If both appear in one logical sequence window, honor whichever occurs last.
        if !markers.is_empty() {
            let prompt_idx = markers
                .iter()
                .rposition(|m| matches!(m, Osc133Marker::Prompt));
            let command = markers
                .iter()
                .enumerate()
                .rev()
                .find_map(|(idx, marker)| match marker {
                    Osc133Marker::Command(cmd) => Some((idx, cmd.clone())),
                    Osc133Marker::Prompt => None,
                });
            let command_wins = match (prompt_idx, command.as_ref()) {
                (Some(prompt_marker_idx), Some((cmd_marker_idx, _))) => {
                    *cmd_marker_idx >= prompt_marker_idx
                }
                (None, Some(_)) => true,
                _ => false,
            };
            self.clear_error_timer();
            if command_wins {
                let (command_idx, cmd) =
                    command.expect("command_wins implies command marker exists");
                debug!(
                    prompt_idx = prompt_idx,
                    command_idx,
                    command = %cmd,
                    "OSC 133 classified output as busy"
                );
                self.set_state(SessionState::Busy, Some(Some(cmd)), now, "osc133_command");
            } else {
                debug!(
                    prompt_idx = prompt_idx,
                    command_idx = command.as_ref().map(|(idx, _)| *idx),
                    "OSC 133 classified output as idle"
                );
                self.set_state(SessionState::Idle, Some(None), now, "osc133_prompt");
            }
            return;
        }

        // Check for error patterns against visible text.
        let mut found_error = false;
        for pattern in &self.error_patterns {
            if pattern.is_match(&visible) {
                debug!(
                    pattern = %pattern.as_str(),
                    sample = %Self::log_excerpt(&visible),
                    "error pattern matched visible output"
                );
                self.set_state(SessionState::Error, None, now, "error_pattern");
                self.schedule_error_clear(now);
                found_error = true;
                break;
            }
        }

        // Heuristic fallback for shells without OSC 133:
        // if we're idle/attention and visible output is not a prompt,
        // treat this as command activity and mark busy.
        if matches!(self.state, SessionState::Idle | SessionState::Attention) && !found_error {
            let has_visible_text = visible.chars().any(|c| !c.is_whitespace());
            let looks_like_prompt = Self::looks_like_prompt(&visible);
            if has_visible_text && !looks_like_prompt {
                self.clear_error_timer();
                debug!(
                    sample = %Self::log_excerpt(&visible),
                    "fallback classified output as busy"
                );
                self.set_state(
                    SessionState::Busy,
                    Some(None),
                    now,
                    "fallback_non_prompt_output",
                );
            }
        }

        // Fallback: regex prompt detection (for shells without OSC 133).
        // Recovers from busy AND error states when a new prompt appears.
        // Checked against visible text so tmux escape sequences don't mask the prompt.
        if self.state != SessionState::Idle && !found_error && Self::looks_like_prompt(&visible) {
            self.clear_error_timer();
            debug!(
                sample = %Self::log_excerpt(&visible),
                "fallback classified output as idle prompt"
            );
            self.set_state(
                SessionState::Idle,
                Some(None),
                now,
                "fallback_prompt_detected",
            );
        }

        // TUI tool mode: reset the output idle deadline on every chunk of output.
        // When the tool stops producing output (silence), the deadline expires
        // and check_timers will transition busy -> idle.
        if self.tui_tool_mode && self.state == SessionState::Busy {
            self.output_idle_deadline = Some(now + Duration::from_millis(OUTPUT_IDLE_MS));
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
                    self.set_state(SessionState::Idle, Some(None), now, "error_timer_expired");
                }
            }
        }

        // TUI tool output silence -> idle
        if let Some(deadline) = self.output_idle_deadline {
            if now >= deadline {
                self.output_idle_deadline = None;
                if self.state == SessionState::Busy {
                    debug!("TUI tool output silence expired, transitioning to idle");
                    self.set_state(
                        SessionState::Idle,
                        Some(None),
                        now,
                        "output_silence_expired",
                    );
                }
            }
        }

        // Attention promotion
        if let Some(deadline) = self.attention_deadline {
            if now >= deadline {
                self.attention_deadline = None;
                if self.state == SessionState::Idle {
                    self.set_state(
                        SessionState::Attention,
                        Some(None),
                        now,
                        "attention_timer_expired",
                    );
                }
            }
        }
    }

    /// Returns the next Instant at which a timer will fire, if any.
    /// Useful for the actor to know when to schedule a wake-up.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.error_deadline,
            self.attention_deadline,
            self.output_idle_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Dismiss the attention state, returning to idle.
    pub fn dismiss_attention(&mut self) {
        self.clear_attention_timer();
        if self.state == SessionState::Attention {
            let now = Instant::now();
            self.set_state(SessionState::Idle, Some(None), now, "dismiss_attention");
        }
    }

    /// Record local user input as command activity.
    ///
    /// This is a fallback for shells that don't emit OSC 133 command markers.
    /// It lets the state machine enter `Busy` immediately on typed input so a
    /// later prompt can produce a reliable busy -> idle transition.
    pub fn note_input(&mut self) {
        if self.state == SessionState::Busy || self.state == SessionState::Exited {
            return;
        }
        let now = Instant::now();
        self.clear_error_timer();
        debug!(state = ?self.state, "local input activity marks session busy");
        self.set_state(SessionState::Busy, Some(None), now, "local_input");
        // In TUI tool mode, start the silence timer from the input event
        // so idle fires if the tool produces no output after user input.
        if self.tui_tool_mode {
            self.output_idle_deadline = Some(now + Duration::from_millis(OUTPUT_IDLE_MS));
        }
    }

    /// Force terminal state to exited after PTY/process shutdown.
    /// This persists exited state for session summaries even if no realtime
    /// client was subscribed when the exit event happened.
    pub fn mark_exited(&mut self) {
        if self.state == SessionState::Exited {
            return;
        }
        self.clear_error_timer();
        self.clear_attention_timer();
        let now = Instant::now();
        self.set_state(SessionState::Exited, Some(None), now, "process_exit");
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
    #[allow(dead_code)]
    pub fn feed(&mut self, data: &[u8]) {
        self.process_output(data);
    }

    // --- Private helpers ---

    fn parse_chunk(&mut self, data: &[u8]) -> ParsedChunk {
        let mut visible: Vec<u8> = Vec::with_capacity(data.len());
        let mut markers: Vec<Osc133Marker> = Vec::new();

        for &b in data {
            self.consume_chunk_byte(b, &mut visible, &mut markers);
        }

        ParsedChunk {
            visible: String::from_utf8_lossy(&visible).to_string(),
            markers,
        }
    }

    fn consume_chunk_byte(
        &mut self,
        b: u8,
        visible: &mut Vec<u8>,
        markers: &mut Vec<Osc133Marker>,
    ) {
        match &mut self.escape_state {
            EscapeState::Normal => self.consume_normal_byte(b, visible),
            EscapeState::Esc => self.consume_escape_byte(b),
            EscapeState::EscIntermediate => self.consume_escape_intermediate_byte(b),
            EscapeState::Csi => self.consume_csi_byte(b),
            EscapeState::Osc { buf, esc_pending } => {
                if let Some(next_state) = Self::consume_osc_byte(b, buf, esc_pending, markers) {
                    self.escape_state = next_state;
                }
            }
            EscapeState::Dcs { esc_pending }
            | EscapeState::Pm { esc_pending }
            | EscapeState::Apc { esc_pending } => {
                if let Some(next_state) = Self::consume_private_string_byte(b, esc_pending) {
                    self.escape_state = next_state;
                }
            }
        }
    }

    fn consume_normal_byte(&mut self, b: u8, visible: &mut Vec<u8>) {
        match b {
            0x1b => self.escape_state = EscapeState::Esc,
            0x9b => self.escape_state = EscapeState::Csi,
            0x9d => self.escape_state = Self::osc_state(),
            0x90 => self.escape_state = Self::private_string_state(EscapeState::Dcs {
                esc_pending: false,
            }),
            0x9e => self.escape_state = Self::private_string_state(EscapeState::Pm {
                esc_pending: false,
            }),
            0x9f => self.escape_state = Self::private_string_state(EscapeState::Apc {
                esc_pending: false,
            }),
            b'\n' | b'\r' | b'\t' => visible.push(b),
            _ if (0x20..=0x7e).contains(&b) => visible.push(b),
            _ => {}
        }
    }

    fn consume_escape_byte(&mut self, b: u8) {
        self.escape_state = match b {
            b'[' => EscapeState::Csi,
            b']' => Self::osc_state(),
            b'P' => Self::private_string_state(EscapeState::Dcs { esc_pending: false }),
            b'^' => Self::private_string_state(EscapeState::Pm { esc_pending: false }),
            b'_' => Self::private_string_state(EscapeState::Apc { esc_pending: false }),
            0x20..=0x2f => EscapeState::EscIntermediate,
            _ => EscapeState::Normal,
        };
    }

    fn consume_escape_intermediate_byte(&mut self, b: u8) {
        if (0x30..=0x7e).contains(&b) || !(0x20..=0x2f).contains(&b) {
            self.escape_state = EscapeState::Normal;
        }
    }

    fn consume_csi_byte(&mut self, b: u8) {
        if (0x40..=0x7e).contains(&b) {
            self.escape_state = EscapeState::Normal;
        }
    }

    fn consume_osc_byte(
        b: u8,
        buf: &mut Vec<u8>,
        esc_pending: &mut bool,
        markers: &mut Vec<Osc133Marker>,
    ) -> Option<EscapeState> {
        if *esc_pending {
            return Self::consume_pending_osc_escape(b, buf, esc_pending, markers);
        }

        match b {
            0x07 | 0x9c => {
                Self::push_osc_marker(buf, markers);
                return Some(EscapeState::Normal);
            }
            0x1b => {
                *esc_pending = true;
                return None;
            }
            _ if buf.len() < 8192 => buf.push(b),
            _ => {}
        }
        None
    }

    fn consume_pending_osc_escape(
        b: u8,
        buf: &mut Vec<u8>,
        esc_pending: &mut bool,
        markers: &mut Vec<Osc133Marker>,
    ) -> Option<EscapeState> {
        if b == b'\\' {
            Self::push_osc_marker(buf, markers);
            return Some(EscapeState::Normal);
        }

        *esc_pending = false;
        if b != 0x1b {
            buf.push(b);
        }
        None
    }

    fn consume_private_string_byte(
        b: u8,
        esc_pending: &mut bool,
    ) -> Option<EscapeState> {
        if *esc_pending {
            if b == b'\\' {
                return Some(EscapeState::Normal);
            } else if b != 0x1b {
                *esc_pending = false;
            }
            return None;
        }

        match b {
            0x9c => Some(EscapeState::Normal),
            0x1b => {
                *esc_pending = true;
                None
            }
            _ => None,
        }
    }

    fn osc_state() -> EscapeState {
        EscapeState::Osc {
            buf: Vec::new(),
            esc_pending: false,
        }
    }

    fn private_string_state(state: EscapeState) -> EscapeState {
        state
    }

    fn push_osc_marker(buf: &[u8], markers: &mut Vec<Osc133Marker>) {
        if let Some(marker) = Self::parse_osc133(buf) {
            markers.push(marker);
        }
    }

    fn parse_osc133(buf: &[u8]) -> Option<Osc133Marker> {
        let payload = String::from_utf8_lossy(buf);
        if !payload.starts_with("133;") {
            return None;
        }

        let mut parts = payload.split(';');
        let _ = parts.next(); // 133
        let kind = parts.next()?;

        match kind {
            "A" => Some(Osc133Marker::Prompt),
            "C" => {
                let command = parts
                    .find_map(|part| part.strip_prefix("cmd=").map(str::to_string))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                Some(Osc133Marker::Command(command))
            }
            _ => None,
        }
    }

    /// Heuristic prompt detection for shells that do not emit OSC 133.
    ///
    /// We intentionally avoid classifying generic values like "42%" as prompts
    /// to prevent busy/idle oscillation during progress output.
    fn looks_like_prompt(visible: &str) -> bool {
        let line = visible
            .split(['\n', '\r'])
            .rev()
            .map(str::trim_end)
            .find(|line| !line.is_empty());
        let Some(line) = line else {
            return false;
        };

        let mut chars = line.chars();
        let Some(marker @ ('$' | '%' | '#' | '>')) = chars.next_back() else {
            return false;
        };
        let prefix = chars.as_str().trim_end();
        if prefix.is_empty() {
            return true;
        }

        if Self::has_prompt_context(prefix) {
            if marker == '%' {
                let compact = prefix.replace(',', "");
                if compact
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_whitespace())
                {
                    return false;
                }
            }
            return true;
        }

        // Minimal prompts like "project$", "host%", etc.
        // Keep this strict so generic output lines do not masquerade as prompts.
        if prefix.len() > 32 {
            return false;
        }
        if prefix.chars().any(|c| c.is_whitespace()) {
            return false;
        }
        if prefix
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        {
            return false;
        }
        if !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return false;
        }

        matches!(marker, '$' | '#' | '%')
    }

    fn has_prompt_context(prefix: &str) -> bool {
        prefix.contains('@')
            || prefix.contains(':')
            || prefix.contains('/')
            || prefix.contains('~')
            || prefix.contains('\\')
            || prefix.ends_with(')')
            || prefix.ends_with(']')
    }

    fn log_excerpt(visible: &str) -> String {
        let mut flat = visible
            .replace('\r', "\\r")
            .replace('\n', "\\n")
            .trim()
            .to_string();
        if flat.is_empty() {
            return "<empty>".to_string();
        }
        if flat.len() > 140 {
            flat.truncate(140);
            flat.push('…');
        }
        flat
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
        cause: &'static str,
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
            debug!(
                from = ?prev,
                to = ?new_state,
                cause,
                current_command = ?self.current_command,
                "state transition"
            );
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

#[derive(Debug)]
struct ParsedChunk {
    visible: String,
    markers: Vec<Osc133Marker>,
}

#[derive(Debug, Clone)]
enum Osc133Marker {
    Prompt,
    Command(String),
}

#[derive(Debug)]
enum EscapeState {
    Normal,
    Esc,
    EscIntermediate,
    Csi,
    Osc { buf: Vec<u8>, esc_pending: bool },
    Dcs { esc_pending: bool },
    Pm { esc_pending: bool },
    Apc { esc_pending: bool },
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
}
