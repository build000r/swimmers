// ScrollGuard -- coalesces rapid full-screen redraws from tmux to prevent
// visual garbage when another tmux client scrolls.
//
// When two tmux clients are attached to the same session, scroll events in one
// client trigger full-screen redraws that reach the other client's PTY. These
// arrive as bursts of cursor-positioning sequences that cause flickering and
// partial-render artifacts in the terminal client.
//
// Strategy:
//  1. If swimmers recently sent input, pass everything through immediately
//     (the redraw is in response to our own activity).
//  2. If output has many cursor-positioning sequences and no recent input,
//     it's likely a scroll-triggered redraw from the other client --
//     buffer it and only forward the final frame after a short delay.
//  3. Normal output (command output, prompts) passes through immediately.

use std::time::{Duration, Instant};

use regex::Regex;

const COALESCE_MS: u64 = 32; // ~2 frames at 60fps
const CURSOR_POS_THRESHOLD: usize = 10; // min cursor-position seqs to trigger coalescing
const INPUT_GRACE_MS: u64 = 200; // pass-through window after swimmers input
// Hard cap on the coalesce buffer. The 32ms window bounds typical bursts, but a
// sustained redraw storm (e.g. a peer tmux client scrolling continuously) could
// otherwise grow the buffer without bound; force-flush past this so memory stays
// bounded while the latest frame is still delivered.
const MAX_COALESCE_BYTES: usize = 512 * 1024;
// Max bytes of a trailing, not-yet-terminated CSI escape carried across reads so
// a cursor-position sequence split on a read boundary is still counted.
const MAX_ESCAPE_CARRY: usize = 16;

pub struct ScrollGuard {
    cursor_pos_re: Regex,
    last_input_time: Option<Instant>,
    buffer: Option<Vec<u8>>,
    flush_deadline: Option<Instant>,
    /// Trailing bytes of the previous chunk that form an incomplete CSI escape,
    /// carried so a cursor-position sequence split on a read boundary is counted
    /// once it completes in the next chunk.
    carry: Vec<u8>,
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
            carry: Vec::new(),
        }
    }

    /// Record that swimmers sent keystrokes to the PTY.
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

        if self.is_inside_input_grace(now) {
            // Passing through on our own activity; a carried partial escape from
            // before the grace window is no longer a reliable redraw signal.
            self.carry.clear();
            self.flush_into(&mut output);
            output.push(ScrollOutputChunk::new(data.to_vec(), false));
            return output;
        }

        self.flush_if_deadline_expired(now, &mut output);

        if self.is_redraw(data) {
            self.buffer_redraw(data, now);
            // Bound memory under a sustained redraw burst: once the coalesced
            // buffer grows past the cap, force-flush now instead of waiting for
            // the 32ms deadline.
            if self.buffered_len() >= MAX_COALESCE_BYTES {
                self.flush_into(&mut output);
            }
        } else {
            self.flush_into(&mut output);
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

    fn is_inside_input_grace(&self, now: Instant) -> bool {
        self.last_input_time
            .map(|last_input| {
                // `checked_duration_since` keeps us safe if the monotonic clock
                // ever appears to move backwards: treat that as still inside
                // the grace window rather than panicking on Instant subtraction.
                now.checked_duration_since(last_input)
                    .unwrap_or(Duration::ZERO)
                    < Duration::from_millis(INPUT_GRACE_MS)
            })
            .unwrap_or(false)
    }

    fn flush_if_deadline_expired(&mut self, now: Instant, output: &mut Vec<ScrollOutputChunk>) {
        if self.buffer.is_some()
            && self
                .flush_deadline
                .map(|deadline| now >= deadline)
                .unwrap_or(true)
        {
            self.flush_into(output);
        }
    }

    fn is_redraw(&mut self, data: &[u8]) -> bool {
        // Count cursor-positioning sequences as a proxy for "full-screen
        // redraw". Bridge any cursor-position escape split across the previous
        // read boundary by prepending the carried tail before counting, so a
        // real redraw is not undercounted and waved through uncoalesced; then
        // re-derive the carry from this chunk's trailing incomplete escape.
        let combined: Vec<u8> = if self.carry.is_empty() {
            data.to_vec()
        } else {
            let mut c = std::mem::take(&mut self.carry);
            c.extend_from_slice(data);
            c
        };
        let text = String::from_utf8_lossy(&combined);
        let count = self.cursor_pos_re.find_iter(&text).count();
        self.carry = trailing_incomplete_csi(&combined);
        count >= CURSOR_POS_THRESHOLD
    }

    fn buffered_len(&self) -> usize {
        self.buffer.as_ref().map_or(0, Vec::len)
    }

    fn buffer_redraw(&mut self, data: &[u8], now: Instant) {
        // Keep a full byte stream inside the coalescing window so split escape
        // sequences are not corrupted.
        if let Some(buffered) = self.buffer.as_mut() {
            buffered.extend_from_slice(data);
        } else {
            self.buffer = Some(data.to_vec());
            self.flush_deadline = Some(now + Duration::from_millis(COALESCE_MS));
        }
    }

    fn flush_into(&mut self, output: &mut Vec<ScrollOutputChunk>) {
        if let Some(buffered) = self.force_flush() {
            output.push(buffered);
        }
    }

    /// Internal flush that clears both buffer and deadline.
    fn force_flush(&mut self) -> Option<ScrollOutputChunk> {
        self.flush_deadline = None;
        self.buffer
            .take()
            .map(|data| ScrollOutputChunk::new(data, true))
    }
}

/// Return the trailing bytes of `data` that form a not-yet-terminated CSI
/// escape (`ESC`, `ESC[`, or `ESC[` + parameter bytes), capped to
/// `MAX_ESCAPE_CARRY`, so the next chunk can complete and count a
/// cursor-position sequence split on a read boundary. Returns empty when the
/// tail ends on a complete or irrelevant sequence.
fn trailing_incomplete_csi(data: &[u8]) -> Vec<u8> {
    let start = data.len().saturating_sub(MAX_ESCAPE_CARRY);
    let tail = &data[start..];
    let Some(esc) = tail.iter().rposition(|&b| b == 0x1b) else {
        return Vec::new();
    };
    let seq = &tail[esc..];
    if is_incomplete_csi(seq) {
        seq.to_vec()
    } else {
        Vec::new()
    }
}

/// True if `seq` (which must start with ESC) is an incomplete CSI sequence: a
/// lone ESC, `ESC[`, or `ESC[` followed only by parameter bytes (ASCII digits
/// or `;`) with no final byte yet. A present final byte means the sequence is
/// already complete (and was counted), so there is nothing to carry.
fn is_incomplete_csi(seq: &[u8]) -> bool {
    let mut iter = seq.iter();
    if iter.next() != Some(&0x1b) {
        return false;
    }
    match iter.next() {
        None => return true, // lone ESC
        Some(b'[') => {}
        Some(_) => return false, // not a CSI introducer we track
    }
    iter.all(|&b| b.is_ascii_digit() || b == b';')
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
    fn threshold_cursor_count_gets_buffered() {
        let mut guard = ScrollGuard::new();
        let data = make_cursor_data(CURSOR_POS_THRESHOLD);
        let result = guard.process(&data);
        assert!(result.is_empty(), "threshold cursor count should buffer");
        assert!(guard.check_flush_deadline().is_some());
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
    fn last_input_in_the_future_does_not_panic() {
        // Regression: `now.duration_since(last_input)` panics if last_input >
        // now, which can happen on NTP steps or platform-specific monotonic
        // clock quirks. We must treat that case as "still in the grace window"
        // and pass output through without crashing.
        let mut guard = ScrollGuard::new();
        guard.last_input_time = Some(Instant::now() + Duration::from_secs(60));
        let data = make_cursor_data(20);
        let result = guard.process(&data);
        assert_eq!(result.len(), 1, "future last_input should treat as grace");
        assert_eq!(result[0].data, data);
        assert!(!result[0].coalesced_redraw);
    }

    #[test]
    fn cursor_sequence_split_across_read_boundary_is_still_detected_as_redraw() {
        let mut guard = ScrollGuard::new();
        // 9 complete cursor seqs + the start of a 10th, split before its final
        // 'H'. Below threshold on its own, so it passes through...
        let mut first = make_cursor_data(CURSOR_POS_THRESHOLD - 1);
        first.extend_from_slice(b"\x1b[10;1");
        let passed = guard.process(&first);
        assert_eq!(passed.len(), 1, "sub-threshold chunk passes through");
        assert!(!passed[0].coalesced_redraw);

        // ...the next chunk completes the split sequence; together with its own
        // 9 cursor seqs that reaches the threshold, so it must be detected as a
        // redraw and buffered. Regression: without cross-chunk carry the lone
        // 'H' + 9 seqs counts only 9 and the redraw leaks through uncoalesced.
        let mut second = Vec::from(&b"H"[..]);
        second.extend_from_slice(&make_cursor_data(CURSOR_POS_THRESHOLD - 1));
        let result = guard.process(&second);
        assert!(
            result.is_empty(),
            "completed split sequence should tip the chunk into a buffered redraw"
        );
        assert!(guard.check_flush_deadline().is_some());
    }

    #[test]
    fn complete_trailing_sequence_is_not_double_counted_across_chunks() {
        let mut guard = ScrollGuard::new();
        // A chunk ending on a *complete* cursor sequence must not leave a carry
        // that double-counts into the next chunk.
        let first = make_cursor_data(CURSOR_POS_THRESHOLD - 1); // ends with \r\n
        assert_eq!(guard.process(&first).len(), 1, "below threshold passes");
        // A following sub-threshold chunk must still pass through (no phantom
        // carry pushing it over the line).
        let second = make_cursor_data(CURSOR_POS_THRESHOLD - 1);
        assert_eq!(
            guard.process(&second).len(),
            1,
            "no phantom carry should tip a sub-threshold chunk into a redraw"
        );
    }

    #[test]
    fn coalesce_buffer_is_capped_under_a_sustained_redraw_burst() {
        let mut guard = ScrollGuard::new();
        let chunk = make_cursor_data(15); // each chunk is itself a redraw
        let chunk_len = chunk.len();
        assert!(chunk_len > 0);
        let needed = MAX_COALESCE_BYTES / chunk_len + 2;
        let mut forced_flush = false;
        for _ in 0..needed {
            for out in guard.process(&chunk) {
                // A mid-burst emitted chunk is the capped force-flush.
                if out.coalesced_redraw {
                    forced_flush = true;
                }
            }
            if forced_flush {
                break;
            }
        }
        assert!(
            forced_flush,
            "buffer should force-flush once it exceeds MAX_COALESCE_BYTES"
        );
        // And the live buffer must never exceed the cap by more than one chunk.
        assert!(guard.buffered_len() < MAX_COALESCE_BYTES + chunk_len);
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
