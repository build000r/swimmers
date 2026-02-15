// ScrollGuard — coalesces rapid full-screen redraws from tmux
// to prevent visual garbage when another tmux client scrolls.
//
// When two tmux clients are attached to the same session, scroll events
// in one client trigger full-screen redraws that reach the other client's
// PTY. These arrive as bursts of cursor-positioning sequences that cause
// flickering and partial-render artifacts in xterm.js.
//
// Strategy:
//  1. If ThrongTerm recently sent input, pass everything through immediately
//     (the redraw is in response to our own activity)
//  2. If output has many cursor-positioning sequences and no recent input,
//     it's likely a scroll-triggered redraw from the other client —
//     buffer it and only forward the final frame after a short delay
//  3. Normal output (command output, prompts) passes through immediately

const COALESCE_MS = 32;          // ~2 frames at 60fps
const CURSOR_POS_THRESHOLD = 10; // min cursor-position seqs to trigger coalescing
const INPUT_GRACE_MS = 200;      // pass-through window after ThrongTerm input

// CSI row;col H  or  CSI row H
const CURSOR_POS_RE = /\x1b\[\d+(?:;\d+)?H/g;

class ScrollGuard {
  constructor(emit) {
    this._emit = emit; // callback(data) — forward output to replay buffer + WebSocket
    this._lastInputTime = 0;
    this._buffer = null;
    this._flushTimer = null;
  }

  // Call when ThrongTerm sends keystrokes to the PTY
  notifyInput() {
    this._lastInputTime = Date.now();
  }

  // Process a chunk of PTY output
  process(data) {
    const now = Date.now();

    // Recent input from ThrongTerm → this redraw is expected, pass through
    if (now - this._lastInputTime < INPUT_GRACE_MS) {
      this._flush();
      this._emit(data);
      return;
    }

    // Count cursor-positioning sequences as a proxy for "full-screen redraw"
    const str = typeof data === 'string' ? data : data.toString('utf-8');
    const matches = str.match(CURSOR_POS_RE);
    const posCount = matches ? matches.length : 0;

    if (posCount >= CURSOR_POS_THRESHOLD) {
      // Likely a scroll-triggered redraw from the other client — coalesce.
      // Replace any previously buffered frame (we only care about the last one).
      this._buffer = data;
      if (this._flushTimer) clearTimeout(this._flushTimer);
      this._flushTimer = setTimeout(() => {
        this._flushTimer = null;
        this._flush();
      }, COALESCE_MS);
    } else {
      // Normal output — flush pending buffer, then emit immediately
      this._flush();
      this._emit(data);
    }
  }

  _flush() {
    if (this._flushTimer) {
      clearTimeout(this._flushTimer);
      this._flushTimer = null;
    }
    if (this._buffer) {
      const buf = this._buffer;
      this._buffer = null;
      this._emit(buf);
    }
  }

  destroy() {
    if (this._flushTimer) {
      clearTimeout(this._flushTimer);
      this._flushTimer = null;
    }
    this._buffer = null;
  }
}

module.exports = ScrollGuard;
