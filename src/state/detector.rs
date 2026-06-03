// State detector -- classifies terminal state as idle, busy, error, or attention.
// Uses OSC 133 shell integration sequences when available, falls back to regex.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use regex::Regex;
use tracing::debug;

use crate::types::{SessionState, StateConfidence, StateEvidence};

const ERROR_LINGER_MS: u64 = 4000;
const ATTENTION_DELAY_MS: u64 = 300000;
const OUTPUT_IDLE_MS: u64 = 5000;
const TERMINAL_STRING_RECOVERY_BYTES: usize = 8192;

/// Callback signature for state change notifications.
/// Arguments: new_state, previous_state, current_command.
pub type StateChangeCallback =
    Box<dyn Fn(SessionState, SessionState, Option<String>) + Send + 'static>;

pub struct StateDetector {
    state: SessionState,
    current_command: Option<String>,
    state_evidence: StateEvidence,
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
            state_evidence: StateEvidence::unobserved("initial_state"),
            error_patterns: vec![
                Regex::new(r"(?i)command not found").expect("error pattern is valid"),
                Regex::new(r"(?i)Permission denied").expect("error pattern is valid"),
                Regex::new(r"(?i)segmentation fault").expect("error pattern is valid"),
                Regex::new(r"(?i)\bpanic:|\bpanicked at\b").expect("error pattern is valid"),
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
    // FIXME(2026-04-21): Runtime wiring still polls state directly; callback registration is currently test-only.
    #[allow(dead_code)]
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

        self.check_timers(now);

        let ParsedChunk { visible, markers } = self.parse_chunk(data);
        if self.apply_osc133_markers(&visible, &markers, now) {
            return;
        }

        self.apply_fallback_detection(&visible, now);
    }

    fn apply_osc133_markers(
        &mut self,
        visible: &str,
        markers: &[PositionedOsc133Marker],
        now: Instant,
    ) -> bool {
        let Some(decision) = Osc133Decision::from_markers(markers) else {
            return false;
        };

        self.clear_error_timer();
        match decision {
            Osc133Decision::Command {
                prompt_idx,
                command_idx,
                visible_offset,
                command,
            } => {
                debug!(
                    prompt_idx = prompt_idx,
                    command_idx,
                    command = %command,
                    "OSC 133 classified output as busy"
                );
                self.set_state(
                    SessionState::Busy,
                    Some(Some(command)),
                    now,
                    "osc133_command",
                );
                self.apply_visible_error_pattern(&visible[visible_offset..], now);
            }
            Osc133Decision::Prompt {
                prompt_idx,
                command_idx,
            } => {
                debug!(
                    prompt_idx = prompt_idx,
                    command_idx = command_idx,
                    "OSC 133 classified output as idle"
                );
                self.set_state(SessionState::Idle, Some(None), now, "osc133_prompt");
                self.apply_visible_error_pattern(visible, now);
            }
        }
        true
    }

    fn apply_fallback_detection(&mut self, visible: &str, now: Instant) {
        let looks_like_prompt = Self::looks_like_prompt(visible);
        let found_error = self.apply_visible_error_pattern(visible, now);
        self.apply_fallback_busy_detection(visible, looks_like_prompt, found_error, now);
        self.apply_fallback_prompt_detection(visible, looks_like_prompt, now);
        self.refresh_tui_output_idle_deadline(now);
    }

    fn apply_fallback_busy_detection(
        &mut self,
        visible: &str,
        looks_like_prompt: bool,
        found_error: bool,
        now: Instant,
    ) {
        if found_error
            || !matches!(self.state, SessionState::Idle | SessionState::Attention)
            || !Self::has_visible_text(visible)
            || looks_like_prompt
        {
            return;
        }

        self.clear_error_timer();
        debug!(
            sample = %Self::log_excerpt(visible),
            "fallback classified output as busy"
        );
        self.set_state(
            SessionState::Busy,
            Some(None),
            now,
            "fallback_non_prompt_output",
        );
    }

    fn apply_fallback_prompt_detection(
        &mut self,
        visible: &str,
        looks_like_prompt: bool,
        now: Instant,
    ) {
        if self.state == SessionState::Idle || !looks_like_prompt {
            return;
        }

        self.clear_error_timer();
        debug!(
            sample = %Self::log_excerpt(visible),
            "fallback classified output as idle prompt"
        );
        self.set_state(
            SessionState::Idle,
            Some(None),
            now,
            "fallback_prompt_detected",
        );
    }

    fn refresh_tui_output_idle_deadline(&mut self, now: Instant) {
        if self.tui_tool_mode && self.state == SessionState::Busy {
            self.output_idle_deadline = Some(now + Duration::from_millis(OUTPUT_IDLE_MS));
        }
    }

    fn has_visible_text(visible: &str) -> bool {
        visible.chars().any(|c| !c.is_whitespace())
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

    fn apply_visible_error_pattern(&mut self, visible: &str, now: Instant) -> bool {
        for pattern in &self.error_patterns {
            if pattern.is_match(visible) {
                debug!(
                    pattern = %pattern.as_str(),
                    sample = %Self::log_excerpt(visible),
                    "error pattern matched visible output"
                );
                self.set_state(SessionState::Error, None, now, "error_pattern");
                self.schedule_error_clear(now);
                return true;
            }
        }
        false
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

    /// Reconcile detector state with process-tree ground truth.
    ///
    /// Called periodically (~2s) by the actor after querying the pane's process
    /// tree. When the process tree disagrees with the output-based heuristics,
    /// the process tree wins:
    ///
    /// - Detector says `Busy` but shell has no children → override to `Idle`
    ///   (the prompt was missed by output heuristics).
    /// - Detector says `Idle`/`Attention` but shell has children → override to
    ///   `Busy` (a quiet command is running that produced no recognizable output).
    ///
    /// `Exited` and `Error` states are never overridden — they come from
    /// higher-confidence signals (PTY close, error pattern match).
    ///
    /// TUI agent tools are the exception to the idle/attention -> busy rule.
    /// Those processes stay alive while waiting on user input, so child
    /// existence alone is not evidence of active work once silence has already
    /// settled the pane into `Idle` or `Attention`.
    pub fn apply_process_liveness(&mut self, has_children: bool) {
        let now = Instant::now();
        match self.state {
            SessionState::Busy if !has_children => {
                debug!("process liveness: no children but state is busy, correcting to idle");
                self.set_state(SessionState::Idle, Some(None), now, "liveness_no_children");
            }
            SessionState::Busy
                if has_children
                    && !self.tui_tool_mode
                    && self.state_evidence.confidence != StateConfidence::High =>
            {
                debug!(
                    cause = %self.state_evidence.cause,
                    "process liveness confirmed busy state"
                );
                self.set_state(SessionState::Busy, None, now, "liveness_has_children");
            }
            SessionState::Idle | SessionState::Attention if has_children => {
                if self.tui_tool_mode {
                    debug!(
                        state = ?self.state,
                        "process liveness: ignoring child-only busy override in TUI tool mode"
                    );
                    return;
                }
                debug!(
                    state = ?self.state,
                    "process liveness: children found but state is idle/attention, correcting to busy"
                );
                self.set_state(SessionState::Busy, Some(None), now, "liveness_has_children");
            }
            _ => {}
        }
    }

    /// Get the current state and command as a tuple.
    pub fn get_state(&self) -> (SessionState, Option<String>) {
        (self.state, self.current_command.clone())
    }

    /// Return evidence for the current state classification.
    pub fn state_evidence(&self) -> StateEvidence {
        self.state_evidence.clone()
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
        let mut markers: Vec<PositionedOsc133Marker> = Vec::new();

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
        markers: &mut Vec<PositionedOsc133Marker>,
    ) {
        match &mut self.escape_state {
            EscapeState::Normal => self.consume_normal_byte(b, visible),
            EscapeState::Esc => self.consume_escape_byte(b),
            EscapeState::EscIntermediate { consumed } => {
                if let Some(next_state) = Self::consume_escape_intermediate_byte(b, consumed) {
                    self.escape_state = next_state;
                }
            }
            EscapeState::Csi { consumed } => {
                if let Some(next_state) = Self::consume_csi_byte(b, consumed) {
                    self.escape_state = next_state;
                }
            }
            EscapeState::Osc {
                buf,
                esc_pending,
                consumed,
            } => {
                if let Some(next_state) =
                    Self::consume_osc_byte(b, visible.len(), buf, esc_pending, consumed, markers)
                {
                    self.escape_state = next_state;
                }
            }
            EscapeState::Dcs {
                esc_pending,
                consumed,
            }
            | EscapeState::Pm {
                esc_pending,
                consumed,
            }
            | EscapeState::Apc {
                esc_pending,
                consumed,
            } => {
                if let Some(next_state) =
                    Self::consume_private_string_byte(b, esc_pending, consumed)
                {
                    self.escape_state = next_state;
                }
            }
        }
    }

    fn consume_normal_byte(&mut self, b: u8, visible: &mut Vec<u8>) {
        match b {
            0x1b => self.escape_state = EscapeState::Esc,
            0x9b => self.escape_state = EscapeState::Csi { consumed: 0 },
            0x9d => self.escape_state = Self::osc_state(),
            0x90 => {
                self.escape_state = Self::private_string_state(EscapeState::Dcs {
                    esc_pending: false,
                    consumed: 0,
                })
            }
            0x9e => {
                self.escape_state = Self::private_string_state(EscapeState::Pm {
                    esc_pending: false,
                    consumed: 0,
                })
            }
            0x9f => {
                self.escape_state = Self::private_string_state(EscapeState::Apc {
                    esc_pending: false,
                    consumed: 0,
                })
            }
            b'\n' | b'\r' | b'\t' => visible.push(b),
            _ if (0x20..=0x7e).contains(&b) => visible.push(b),
            _ => {}
        }
    }

    fn consume_escape_byte(&mut self, b: u8) {
        self.escape_state = match b {
            b'[' => EscapeState::Csi { consumed: 0 },
            b']' => Self::osc_state(),
            b'P' => Self::private_string_state(EscapeState::Dcs {
                esc_pending: false,
                consumed: 0,
            }),
            b'^' => Self::private_string_state(EscapeState::Pm {
                esc_pending: false,
                consumed: 0,
            }),
            b'_' => Self::private_string_state(EscapeState::Apc {
                esc_pending: false,
                consumed: 0,
            }),
            0x20..=0x2f => EscapeState::EscIntermediate { consumed: 0 },
            _ => EscapeState::Normal,
        };
    }

    fn consume_escape_intermediate_byte(b: u8, consumed: &mut usize) -> Option<EscapeState> {
        *consumed += 1;
        if *consumed > TERMINAL_STRING_RECOVERY_BYTES {
            return Some(EscapeState::Normal);
        }
        if (0x30..=0x7e).contains(&b) || !(0x20..=0x2f).contains(&b) {
            return Some(EscapeState::Normal);
        }
        None
    }

    fn consume_csi_byte(b: u8, consumed: &mut usize) -> Option<EscapeState> {
        *consumed += 1;
        if *consumed > TERMINAL_STRING_RECOVERY_BYTES {
            return Some(EscapeState::Normal);
        }
        if (0x40..=0x7e).contains(&b) {
            return Some(EscapeState::Normal);
        }
        None
    }

    fn consume_osc_byte(
        b: u8,
        visible_offset: usize,
        buf: &mut Vec<u8>,
        esc_pending: &mut bool,
        consumed: &mut usize,
        markers: &mut Vec<PositionedOsc133Marker>,
    ) -> Option<EscapeState> {
        *consumed += 1;
        if *consumed > TERMINAL_STRING_RECOVERY_BYTES {
            return Some(EscapeState::Normal);
        }

        if *esc_pending {
            return Self::consume_pending_osc_escape(b, visible_offset, buf, esc_pending, markers);
        }

        match b {
            0x07 | 0x9c => {
                Self::push_osc_marker(buf, visible_offset, markers);
                return Some(EscapeState::Normal);
            }
            0x1b => {
                *esc_pending = true;
                return None;
            }
            _ if buf.len() < TERMINAL_STRING_RECOVERY_BYTES => buf.push(b),
            _ => {}
        }
        None
    }

    fn consume_pending_osc_escape(
        b: u8,
        visible_offset: usize,
        buf: &mut Vec<u8>,
        esc_pending: &mut bool,
        markers: &mut Vec<PositionedOsc133Marker>,
    ) -> Option<EscapeState> {
        if b == b'\\' {
            Self::push_osc_marker(buf, visible_offset, markers);
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
        consumed: &mut usize,
    ) -> Option<EscapeState> {
        *consumed += 1;
        if *consumed > TERMINAL_STRING_RECOVERY_BYTES {
            return Some(EscapeState::Normal);
        }

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
            consumed: 0,
        }
    }

    fn private_string_state(state: EscapeState) -> EscapeState {
        state
    }

    fn push_osc_marker(
        buf: &[u8],
        visible_offset: usize,
        markers: &mut Vec<PositionedOsc133Marker>,
    ) {
        if let Some(marker) = Self::parse_osc133(buf) {
            markers.push(PositionedOsc133Marker {
                marker,
                visible_offset,
            });
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
        let Some((prefix, marker)) = Self::prompt_candidate(visible) else {
            return false;
        };

        if prefix.is_empty() {
            return true;
        }

        if Self::has_prompt_context(prefix) {
            return Self::context_prompt_allowed(prefix, marker);
        }

        Self::minimal_prompt_allowed(prefix, marker)
    }

    fn prompt_candidate(visible: &str) -> Option<(&str, char)> {
        let line = visible
            .split(['\n', '\r'])
            .rev()
            .map(str::trim_end)
            .find(|line| !line.is_empty())?;

        let mut chars = line.chars();
        let marker = match chars.next_back()? {
            marker @ ('$' | '%' | '#' | '>') => marker,
            _ => return None,
        };
        let prefix = chars.as_str().trim_end();

        Some((prefix, marker))
    }

    fn context_prompt_allowed(prefix: &str, marker: char) -> bool {
        marker != '%' || !Self::is_numeric_percent_context(prefix)
    }

    fn is_numeric_percent_context(prefix: &str) -> bool {
        prefix
            .replace(',', "")
            .chars()
            .all(Self::is_digit_dot_or_whitespace)
    }

    fn is_digit_dot_or_whitespace(c: char) -> bool {
        c.is_ascii_digit() || c == '.' || c.is_ascii_whitespace()
    }

    fn minimal_prompt_allowed(prefix: &str, marker: char) -> bool {
        if !Self::minimal_prompt_prefix_allowed(prefix) {
            return false;
        }

        matches!(marker, '$' | '#' | '%')
    }

    fn minimal_prompt_prefix_allowed(prefix: &str) -> bool {
        // Minimal prompts like "project$", "host%", etc.
        // Keep this strict so generic output lines do not masquerade as prompts.
        Self::minimal_prompt_prefix_shape_allowed(prefix)
            && !Self::is_numeric_minimal_prompt_prefix(prefix)
            && Self::minimal_prompt_prefix_chars_allowed(prefix)
    }

    fn minimal_prompt_prefix_shape_allowed(prefix: &str) -> bool {
        prefix.len() <= 32 && !prefix.chars().any(|c| c.is_whitespace())
    }

    fn is_numeric_minimal_prompt_prefix(prefix: &str) -> bool {
        prefix
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    }

    fn minimal_prompt_prefix_chars_allowed(prefix: &str) -> bool {
        prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    }

    fn has_prompt_context(prefix: &str) -> bool {
        prefix.chars().any(Self::is_prompt_context_char)
            || prefix
                .chars()
                .last()
                .is_some_and(Self::is_prompt_context_suffix)
    }

    fn is_prompt_context_char(c: char) -> bool {
        matches!(c, '@' | ':' | '/' | '~' | '\\')
    }

    fn is_prompt_context_suffix(c: char) -> bool {
        matches!(c, ')' | ']')
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
        // Truncate by characters, not bytes: callers currently pass ASCII-only
        // `visible` text, but a byte `truncate(140)` would panic if that ever
        // changed and the 140th byte fell inside a multi-byte sequence. Char
        // truncation is identical for ASCII and robust regardless.
        if flat.chars().count() > 140 {
            flat = flat.chars().take(140).collect();
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
        self.state_evidence = StateEvidence::new(cause);

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
    markers: Vec<PositionedOsc133Marker>,
}

#[derive(Debug)]
struct PositionedOsc133Marker {
    marker: Osc133Marker,
    visible_offset: usize,
}

#[derive(Debug, Clone)]
enum Osc133Marker {
    Prompt,
    Command(String),
}

#[derive(Debug)]
enum Osc133Decision {
    Prompt {
        prompt_idx: Option<usize>,
        command_idx: Option<usize>,
    },
    Command {
        prompt_idx: Option<usize>,
        command_idx: usize,
        visible_offset: usize,
        command: String,
    },
}

impl Osc133Decision {
    fn from_markers(markers: &[PositionedOsc133Marker]) -> Option<Self> {
        let prompt_idx = Self::last_prompt_idx(markers);
        let command = Self::last_command(markers);

        if Self::command_wins(prompt_idx, command.as_ref()) {
            let (command_idx, visible_offset, command) =
                command.expect("command_wins implies command marker exists");
            return Some(Self::Command {
                prompt_idx,
                command_idx,
                visible_offset,
                command,
            });
        }

        prompt_idx.map(|prompt_idx| Self::Prompt {
            prompt_idx: Some(prompt_idx),
            command_idx: command.map(|(idx, _, _)| idx),
        })
    }

    fn last_prompt_idx(markers: &[PositionedOsc133Marker]) -> Option<usize> {
        markers
            .iter()
            .rposition(|m| matches!(m.marker, Osc133Marker::Prompt))
    }

    fn last_command(markers: &[PositionedOsc133Marker]) -> Option<(usize, usize, String)> {
        markers
            .iter()
            .enumerate()
            .rev()
            .find_map(|(idx, marker)| match &marker.marker {
                Osc133Marker::Command(command) => {
                    Some((idx, marker.visible_offset, command.clone()))
                }
                Osc133Marker::Prompt => None,
            })
    }

    fn command_wins(prompt_idx: Option<usize>, command: Option<&(usize, usize, String)>) -> bool {
        match (prompt_idx, command) {
            (Some(prompt_marker_idx), Some((command_idx, _, _))) => {
                *command_idx >= prompt_marker_idx
            }
            (None, Some(_)) => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
enum EscapeState {
    Normal,
    Esc,
    EscIntermediate {
        consumed: usize,
    },
    Csi {
        consumed: usize,
    },
    Osc {
        buf: Vec<u8>,
        esc_pending: bool,
        consumed: usize,
    },
    Dcs {
        esc_pending: bool,
        consumed: usize,
    },
    Pm {
        esc_pending: bool,
        consumed: usize,
    },
    Apc {
        esc_pending: bool,
        consumed: usize,
    },
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
            StateDetector::consume_private_string_byte(0x1b, &mut esc_pending, &mut consumed)
                .is_none()
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
            StateDetector::consume_private_string_byte(b'x', &mut esc_pending, &mut consumed)
                .is_none()
        );
        assert!(!esc_pending);
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
}
