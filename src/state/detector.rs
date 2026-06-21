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
    #[cfg(test)]
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
        } else if self.state == SessionState::Busy && self.output_idle_deadline.is_none() {
            // The mode can flip on AFTER a session was already classified Busy
            // (e.g. a periodic liveness tool-refresh). Without arming the silence
            // timer here, that session has no path back to Idle — no further
            // output/input would call refresh_tui_output_idle_deadline, and
            // liveness can't correct a Busy-with-children session — so it would
            // stay Busy forever (wrong attention/cadence tiering).
            self.output_idle_deadline =
                Some(Instant::now() + Duration::from_millis(OUTPUT_IDLE_MS));
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
                // Mirror the fallback path: a busy classification must arm the
                // output-silence deadline so a later quiet period still
                // transitions a TUI tool to idle (the function self-guards on
                // tui_tool_mode + Busy).
                self.refresh_tui_output_idle_deadline(now);
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
        self.check_error_deadline(now);
        self.check_output_idle_deadline(now);
        self.check_attention_deadline(now);
    }

    fn check_error_deadline(&mut self, now: Instant) {
        if Self::take_expired_deadline(&mut self.error_deadline, now)
            && self.state == SessionState::Error
        {
            self.set_state(SessionState::Idle, Some(None), now, "error_timer_expired");
        }
    }

    fn check_output_idle_deadline(&mut self, now: Instant) {
        if Self::take_expired_deadline(&mut self.output_idle_deadline, now)
            && self.state == SessionState::Busy
        {
            debug!("TUI tool output silence expired, transitioning to idle");
            self.set_state(
                SessionState::Idle,
                Some(None),
                now,
                "output_silence_expired",
            );
        }
    }

    fn check_attention_deadline(&mut self, now: Instant) {
        if Self::take_expired_deadline(&mut self.attention_deadline, now)
            && self.state == SessionState::Idle
        {
            self.set_state(
                SessionState::Attention,
                Some(None),
                now,
                "attention_timer_expired",
            );
        }
    }

    fn take_expired_deadline(deadline: &mut Option<Instant>, now: Instant) -> bool {
        if deadline.is_some_and(|deadline| now >= deadline) {
            *deadline = None;
            true
        } else {
            false
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
        if self.correct_busy_without_children(has_children, now) {
            return;
        }
        if self.confirm_busy_with_children(has_children, now) {
            return;
        }
        self.correct_idle_or_attention_with_children(has_children, now);
    }

    fn correct_busy_without_children(&mut self, has_children: bool, now: Instant) -> bool {
        if self.state != SessionState::Busy || has_children {
            return false;
        }
        debug!("process liveness: no children but state is busy, correcting to idle");
        self.set_state(SessionState::Idle, Some(None), now, "liveness_no_children");
        true
    }

    fn confirm_busy_with_children(&mut self, has_children: bool, now: Instant) -> bool {
        if self.state != SessionState::Busy
            || !has_children
            || self.tui_tool_mode
            || self.state_evidence.confidence == StateConfidence::High
        {
            return false;
        }
        debug!(
            cause = %self.state_evidence.cause,
            "process liveness confirmed busy state"
        );
        self.set_state(SessionState::Busy, None, now, "liveness_has_children");
        true
    }

    fn correct_idle_or_attention_with_children(
        &mut self,
        has_children: bool,
        now: Instant,
    ) -> bool {
        if !matches!(self.state, SessionState::Idle | SessionState::Attention) || !has_children {
            return false;
        }
        if self.tui_tool_mode {
            debug!(
                state = ?self.state,
                "process liveness: ignoring child-only busy override in TUI tool mode"
            );
            return true;
        }
        debug!(
            state = ?self.state,
            "process liveness: children found but state is idle/attention, correcting to busy"
        );
        self.set_state(SessionState::Busy, Some(None), now, "liveness_has_children");
        true
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
        if matches!(self.escape_state, EscapeState::Normal) {
            self.consume_normal_byte(b, visible);
        } else if self.consume_control_escape_state_byte(b) {
            // Handled by ESC/CSI dispatch.
        } else if self.consume_osc_state_byte(b, visible.len(), markers) {
            // Handled by OSC dispatch.
        } else {
            self.consume_private_string_state_byte(b);
        }
    }

    fn consume_control_escape_state_byte(&mut self, b: u8) -> bool {
        match &mut self.escape_state {
            EscapeState::Esc => {
                self.consume_escape_byte(b);
                true
            }
            EscapeState::EscIntermediate { consumed } => {
                if let Some(next_state) = Self::consume_escape_intermediate_byte(b, consumed) {
                    self.escape_state = next_state;
                }
                true
            }
            EscapeState::Csi { consumed } => {
                if let Some(next_state) = Self::consume_csi_byte(b, consumed) {
                    self.escape_state = next_state;
                }
                true
            }
            _ => false,
        }
    }

    fn consume_osc_state_byte(
        &mut self,
        b: u8,
        visible_offset: usize,
        markers: &mut Vec<PositionedOsc133Marker>,
    ) -> bool {
        let EscapeState::Osc {
            buf,
            esc_pending,
            consumed,
        } = &mut self.escape_state
        else {
            return false;
        };

        if let Some(next_state) =
            Self::consume_osc_byte(b, visible_offset, buf, esc_pending, consumed, markers)
        {
            self.escape_state = next_state;
        }
        true
    }

    fn consume_private_string_state_byte(&mut self, b: u8) -> bool {
        match &mut self.escape_state {
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
                true
            }
            _ => false,
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
        // A re-detection that changes neither the state, the cause, nor the
        // command is a no-op: rebuilding StateEvidence would only re-stamp a
        // fresh observed_at, which then compares unequal and spuriously
        // re-broadcasts session_state -- e.g. the ~2s liveness re-confirmation
        // of a quiet busy session (swimmers-lzll). Preserve the existing
        // evidence in that case so no event is emitted. A real state change, a
        // different cause, or a new command still rebuilds and re-stamps it.
        let command_unchanged = match &command_update {
            None => true,
            Some(cmd) => cmd == &self.current_command,
        };
        let is_noop_redetection =
            new_state == prev && self.state_evidence.cause == cause && command_unchanged;
        self.state = new_state;
        if !is_noop_redetection {
            self.state_evidence = StateEvidence::new(cause);
        }
        self.apply_command_update(command_update);
        self.update_attention_timer_for_transition(prev, new_state);
        self.emit_state_transition(prev, new_state, cause);
    }

    fn apply_command_update(&mut self, command_update: Option<Option<String>>) {
        if let Some(cmd) = command_update {
            self.current_command = cmd;
        }
    }

    fn update_attention_timer_for_transition(
        &mut self,
        prev: SessionState,
        new_state: SessionState,
    ) {
        if Self::should_schedule_attention(prev, new_state) {
            self.schedule_attention();
        }

        // Cancel attention timer if leaving idle for anything other than attention.
        // idle -> idle re-detections must NOT cancel the timer.
        if Self::should_clear_attention_timer(prev, new_state) {
            self.clear_attention_timer();
        }
    }

    fn should_schedule_attention(prev: SessionState, new_state: SessionState) -> bool {
        new_state == SessionState::Idle && prev == SessionState::Busy
    }

    fn should_clear_attention_timer(prev: SessionState, new_state: SessionState) -> bool {
        prev == SessionState::Idle
            && new_state != SessionState::Attention
            && new_state != SessionState::Idle
    }

    fn emit_state_transition(
        &self,
        prev: SessionState,
        new_state: SessionState,
        cause: &'static str,
    ) {
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
mod tests;
