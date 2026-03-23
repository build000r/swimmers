// ScrollGuard -- coalesces rapid full-screen redraws from tmux to prevent
// visual garbage when another tmux client scrolls.
//
// When two tmux clients are attached to the same session, scroll events in one
// client trigger full-screen redraws that reach the other client's PTY. These
// arrive as bursts of cursor-positioning sequences that cause flickering and
// partial-render artifacts in the terminal client.
//
// Strategy:
//  1. If ThrongTerm recently sent input, pass everything through immediately
//     (the redraw is in response to our own activity).
//  2. If output has many cursor-positioning sequences and no recent input,
//     it's likely a scroll-triggered redraw from the other client --
//     buffer it and only forward the final frame after a short delay.
//  3. Normal output (command output, prompts) passes through immediately.

use std::time::{Duration, Instant};

use regex::Regex;

const COALESCE_MS: u64 = 32; // ~2 frames at 60fps
const CURSOR_POS_THRESHOLD: usize = 10; // min cursor-position seqs to trigger coalescing
const INPUT_GRACE_MS: u64 = 200; // pass-through window after ThrongTerm input

pub struct ScrollGuard {
    cursor_pos_re: Regex,
    last_input_time: Option<Instant>,
    buffer: Option<Vec<u8>>,
    flush_deadline: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollOutputChunk {
    pub data: Vec<u8>,
    pub coalesced_redraw: bool,
}

impl ScrollOutputChunk {
    fn new(data: Vec<u8>, coalesced_redraw: bool) -> Self {
        Self {
            data,
            coalesced_redraw,
        }
    }
}

impl ScrollGuard {
    pub fn new() -> Self {
        Self {
            cursor_pos_re: Regex::new(r"\x1b\[\d+(?:;\d+)?H").expect("cursor_pos_re is valid"),
            last_input_time: None,
            buffer: None,
            flush_deadline: None,
        }
    }

    /// Record that ThrongTerm sent keystrokes to the PTY.
    /// Output arriving within INPUT_GRACE_MS of this call is assumed to be
    /// in response to our own activity and is passed through without coalescing.
    pub fn notify_input(&mut self) {
        self.last_input_time = Some(Instant::now());
    }

    /// Alias for `notify_input` -- the name used by the session actor.
    #[allow(dead_code)]
    pub fn note_input(&mut self) {
        self.notify_input();
    }

    /// Process a chunk of PTY output.
    ///
    /// Returns a vec of data chunks to emit immediately. The vec may be:
    /// - Empty: data was buffered for coalescing, nothing to emit yet.
    /// - One element: either normal pass-through or flushed buffer.
    /// - Two elements: flushed buffer followed by new pass-through data.
    ///
    /// The caller should forward each returned chunk to the replay buffer
    /// and any downstream consumers in order.
    pub fn process(&mut self, data: &[u8]) -> Vec<ScrollOutputChunk> {
        let now = Instant::now();
        let mut output = Vec::new();

        // Recent input from ThrongTerm -> this redraw is expected, pass through.
        if let Some(last_input) = self.last_input_time {
            if now.duration_since(last_input) < Duration::from_millis(INPUT_GRACE_MS) {
                if let Some(buffered) = self.force_flush() {
                    output.push(buffered);
                }
                output.push(ScrollOutputChunk::new(data.to_vec(), false));
                return output;
            }
        }

        // If the coalescing window expired while output keeps streaming, flush
        // on the next chunk so rendering keeps progressing even without timer
        // wakeups winning the select race.
        if self.buffer.is_some()
            && self
                .flush_deadline
                .map(|deadline| now >= deadline)
                .unwrap_or(true)
        {
            if let Some(buffered) = self.force_flush() {
                output.push(buffered);
            }
        }

        // Count cursor-positioning sequences as a proxy for "full-screen redraw".
        let text = String::from_utf8_lossy(data);
        let pos_count = self.cursor_pos_re.find_iter(&text).count();

        if pos_count >= CURSOR_POS_THRESHOLD {
            // Likely a scroll-triggered redraw from the other client -- coalesce.
            // Keep a full byte stream inside the coalescing window so split
            // escape sequences are not corrupted.
            if let Some(buffered) = self.buffer.as_mut() {
                buffered.extend_from_slice(data);
            } else {
                self.buffer = Some(data.to_vec());
                self.flush_deadline = Some(now + Duration::from_millis(COALESCE_MS));
            }
            // Nothing to emit yet.
        } else {
            // Normal output -- flush pending buffer, then emit immediately.
            if let Some(buffered) = self.force_flush() {
                output.push(buffered);
            }
            output.push(ScrollOutputChunk::new(data.to_vec(), false));
        }

        output
    }

    /// Force-flush any buffered data, returning it if present.
    /// Clears the flush deadline.
    pub fn flush(&mut self) -> Option<ScrollOutputChunk> {
        self.force_flush()
    }

    /// Returns the Instant at which buffered data should be flushed.
    /// Returns None if there is no pending buffer.
    ///
    /// The session actor should use this to set a timer. When the timer fires,
    /// call `flush()` and forward the result.
    pub fn check_flush_deadline(&self) -> Option<Instant> {
        if self.buffer.is_some() {
            self.flush_deadline
        } else {
            None
        }
    }

    // --- Private helpers ---

    /// Internal flush that clears both buffer and deadline.
    fn force_flush(&mut self) -> Option<ScrollOutputChunk> {
        self.flush_deadline = None;
        self.buffer
            .take()
            .map(|data| ScrollOutputChunk::new(data, true))
    }
}

impl Default for ScrollGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a chunk of data with N cursor-position sequences.
    fn make_cursor_data(count: usize) -> Vec<u8> {
        let mut s = String::new();
        for i in 0..count {
            s.push_str(&format!("\x1b[{};{}H", i + 1, 1));
            s.push_str("line content\r\n");
        }
        s.into_bytes()
    }

    #[test]
    fn normal_output_passes_through() {
        let mut guard = ScrollGuard::new();
        let data = b"hello world\r\n";
        let result = guard.process(data);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].data, data);
        assert!(!result[0].coalesced_redraw);
    }

    #[test]
    fn high_cursor_count_gets_buffered() {
        let mut guard = ScrollGuard::new();
        let data = make_cursor_data(15);
        let result = guard.process(&data);
        assert!(result.is_empty(), "should buffer high-cursor output");
        assert!(guard.check_flush_deadline().is_some());
    }

    #[test]
    fn buffered_data_returned_on_flush() {
        let mut guard = ScrollGuard::new();
        let data = make_cursor_data(15);
        guard.process(&data);

        let flushed = guard.flush();
        assert!(flushed.is_some());
        let flushed = flushed.unwrap();
        assert_eq!(flushed.data, data);
        assert!(flushed.coalesced_redraw);
        assert!(guard.check_flush_deadline().is_none());
    }

    #[test]
    fn normal_output_flushes_pending_buffer() {
        let mut guard = ScrollGuard::new();
        let redraw = make_cursor_data(15);
        guard.process(&redraw);

        let normal = b"prompt$ ";
        let result = guard.process(normal);
        // Should get the flushed buffer + normal data.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].data, redraw);
        assert!(result[0].coalesced_redraw);
        assert_eq!(result[1].data, normal.to_vec());
        assert!(!result[1].coalesced_redraw);
    }

    #[test]
    fn input_grace_bypasses_coalescing() {
        let mut guard = ScrollGuard::new();
        guard.notify_input();

        let data = make_cursor_data(20);
        let result = guard.process(&data);
        // Should pass through because of recent input.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].data, data);
        assert!(!result[0].coalesced_redraw);
    }

    #[test]
    fn input_grace_expires() {
        let mut guard = ScrollGuard::new();
        guard.last_input_time = Some(Instant::now() - Duration::from_millis(INPUT_GRACE_MS + 50));

        let data = make_cursor_data(20);
        let result = guard.process(&data);
        // Grace period expired, should buffer.
        assert!(result.is_empty());
    }

    #[test]
    fn below_threshold_passes_through() {
        let mut guard = ScrollGuard::new();
        // Just under the threshold.
        let data = make_cursor_data(CURSOR_POS_THRESHOLD - 1);
        let result = guard.process(&data);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn successive_redraws_append_within_coalesce_window() {
        let mut guard = ScrollGuard::new();
        let first = make_cursor_data(15);
        let second = make_cursor_data(20);

        guard.process(&first);
        guard.process(&second);

        // Both chunks should remain in-order inside the coalesced stream.
        let flushed = guard.flush().unwrap();
        let mut expected = first.clone();
        expected.extend_from_slice(&second);
        assert_eq!(flushed.data, expected);
        assert!(flushed.coalesced_redraw);
    }

    #[test]
    fn no_deadline_when_no_buffer() {
        let guard = ScrollGuard::new();
        assert!(guard.check_flush_deadline().is_none());
    }

    #[test]
    fn flush_on_empty_returns_none() {
        let mut guard = ScrollGuard::new();
        assert!(guard.flush().is_none());
    }

    #[test]
    fn input_grace_flushes_existing_buffer() {
        let mut guard = ScrollGuard::new();
        let redraw = make_cursor_data(15);
        guard.process(&redraw);

        // Now user types something.
        guard.notify_input();
        let more_redraw = make_cursor_data(20);
        let result = guard.process(&more_redraw);

        // Should flush old buffer + pass through new data.
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].data, redraw);
        assert!(result[0].coalesced_redraw);
        assert_eq!(result[1].data, more_redraw);
        assert!(!result[1].coalesced_redraw);
    }

    #[test]
    fn expired_deadline_flushes_before_buffering_next_redraw() {
        let mut guard = ScrollGuard::new();
        let first = make_cursor_data(15);
        let second = make_cursor_data(15);

        let result1 = guard.process(&first);
        assert!(result1.is_empty());

        // Simulate timer starvation where PTY keeps winning the actor select.
        guard.flush_deadline = Some(Instant::now() - Duration::from_millis(1));

        let result2 = guard.process(&second);
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].data, first);
        assert!(result2[0].coalesced_redraw);

        let flushed = guard.flush().unwrap();
        assert_eq!(flushed.data, second);
        assert!(flushed.coalesced_redraw);
    }

    #[test]
    fn split_escape_sequence_across_redraw_chunks_is_preserved() {
        let mut guard = ScrollGuard::new();
        let mut prefix = make_cursor_data(12);
        prefix.extend_from_slice(b"\x1b[31");
        let mut suffix = make_cursor_data(12);
        suffix.extend_from_slice(b"mHELLO\x1b[0m");

        assert!(guard.process(&prefix).is_empty());
        assert!(guard.process(&suffix).is_empty());

        let flushed = guard.flush().unwrap();
        let mut expected = prefix.clone();
        expected.extend_from_slice(&suffix);
        assert_eq!(flushed.data, expected);
        assert!(flushed.coalesced_redraw);
    }
}
