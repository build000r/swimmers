// State detector — classifies terminal state as idle, busy, error, or attention
// Uses OSC 133 shell integration sequences when available, falls back to regex

const ERROR_LINGER_MS = 4000; // error state auto-clears after 4s
const ATTENTION_DELAY_MS = 10000; // idle → attention after 10s of waiting

class StateDetector {
  constructor() {
    this.state = 'idle';
    this.currentCommand = null;
    this._promptPattern = /[$%>#]\s*$/;
    this._errorTimer = null;
    this._attentionTimer = null;
    // Only match patterns that unambiguously indicate a real shell error,
    // not strings that might appear in file contents or tool output
    this._errorPatterns = [
      /command not found/i,
      /Permission denied/i,
      /segmentation fault/i,
      /panic:/i,
    ];
    this._onStateChange = null;
  }

  onStateChange(cb) {
    this._onStateChange = cb;
  }

  // Shell integration init script injected into zsh on session start
  static shellIntegrationScript() {
    // OSC 133 sequences for prompt/command boundaries
    return [
      // Mark prompt start (A) and command start (C)
      `precmd() { printf '\\e]133;A\\a' }`,
      `preexec() { printf '\\e]133;C;cmd=%s\\a' "$1" }`,
    ].join('; ');
  }

  // Strip ANSI/CSI/OSC escape sequences to get visible text for pattern matching.
  // OSC 133 sequences are checked separately against raw output first.
  _stripAnsi(str) {
    return str
      .replace(/\x1b\[[0-9;]*[A-Za-z]/g, '')   // CSI sequences (colors, cursor)
      .replace(/\x1b\][^\x07]*\x07/g, '')       // OSC sequences
      .replace(/\x1b[()][0-9A-B]/g, '')          // charset switches
      .replace(/\x1b[>=<]/g, '')                 // mode sets
      .replace(/[\x00-\x08\x0e-\x1f]/g, '');    // control chars (keep \t \n \r)
  }

  // Process raw PTY output, detect state transitions
  processOutput(data) {
    const str = typeof data === 'string' ? data : data.toString('utf-8');

    // Check for OSC 133 sequences in raw output
    // Prompt shown (A) → idle
    const promptMatch = str.match(/\x1b\]133;A/);
    if (promptMatch) {
      this._clearErrorTimer();
      this._setState('idle', null);
      return;
    }

    // Command starting (C) with command name
    const cmdMatch = str.match(/\x1b\]133;C;cmd=([^\x07]*)\x07/);
    if (cmdMatch) {
      this._clearErrorTimer();
      this._setState('busy', cmdMatch[1].trim());
      return;
    }

    // Strip escape sequences for heuristic pattern matching
    const visible = this._stripAnsi(str);

    // Check for error patterns against visible text
    for (const pattern of this._errorPatterns) {
      if (pattern.test(visible)) {
        this._setState('error', this.currentCommand);
        this._scheduleErrorClear();
        break;
      }
    }

    // Fallback: regex prompt detection (for shells without OSC 133)
    // Recovers from busy AND error states when a new prompt appears.
    // Checked against visible text so tmux escape sequences don't mask the prompt.
    if (this.state !== 'idle' && this._promptPattern.test(visible)) {
      this._clearErrorTimer();
      this._setState('idle', null);
    }
  }

  _scheduleErrorClear() {
    this._clearErrorTimer();
    this._errorTimer = setTimeout(() => {
      this._errorTimer = null;
      if (this.state === 'error') {
        this._setState('idle', null);
      }
    }, ERROR_LINGER_MS);
  }

  _clearErrorTimer() {
    if (this._errorTimer) {
      clearTimeout(this._errorTimer);
      this._errorTimer = null;
    }
  }

  _scheduleAttention(fromCommand) {
    this._clearAttentionTimer();
    this._attentionTimer = setTimeout(() => {
      this._attentionTimer = null;
      if (this.state === 'idle') {
        this._setState('attention', null);
        this._lastAttentionCommand = fromCommand;
      }
    }, ATTENTION_DELAY_MS);
  }

  _clearAttentionTimer() {
    if (this._attentionTimer) {
      clearTimeout(this._attentionTimer);
      this._attentionTimer = null;
    }
  }

  dismissAttention() {
    this._clearAttentionTimer();
    if (this.state === 'attention') {
      this._setState('idle', null);
    }
  }

  _setState(newState, command) {
    const prev = this.state;
    this.state = newState;
    if (command !== undefined) {
      this.currentCommand = command;
    }

    // Start attention timer when transitioning from busy → idle
    if (newState === 'idle' && prev === 'busy') {
      this._scheduleAttention(this.currentCommand);
    }
    // Cancel attention timer if leaving idle for anything other than attention
    // (idle → idle re-detections must NOT cancel the timer)
    if (prev === 'idle' && newState !== 'attention' && newState !== 'idle') {
      this._clearAttentionTimer();
    }

    if (newState !== prev && this._onStateChange) {
      this._onStateChange({ state: newState, previousState: prev, currentCommand: this.currentCommand });
    }
  }

  getState() {
    return { state: this.state, currentCommand: this.currentCommand };
  }
}

module.exports = StateDetector;
