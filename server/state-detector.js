// State detector — classifies terminal state as idle, busy, or error
// Uses OSC 133 shell integration sequences when available, falls back to regex

class StateDetector {
  constructor() {
    this.state = 'idle';
    this.currentCommand = null;
    this._promptPattern = /[$%>#]\s*$/;
    this._errorPatterns = [
      /command not found/i,
      /^Error:/m,
      /Permission denied/i,
      /No such file or directory/i,
      /fatal:/i,
      /ENOENT/,
      /EACCES/,
      /segmentation fault/i,
      /killed/i,
      /abort/i,
      /panic:/i,
      /Traceback \(most recent call last\)/i,
      /SyntaxError:/,
      /TypeError:/,
      /ReferenceError:/,
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

  // Process raw PTY output, detect state transitions
  processOutput(data) {
    const str = typeof data === 'string' ? data : data.toString('utf-8');

    // Check for OSC 133 sequences
    // Prompt shown (A) → idle
    const promptMatch = str.match(/\x1b\]133;A/);
    if (promptMatch) {
      this._setState('idle', null);
      return;
    }

    // Command starting (C) with command name
    const cmdMatch = str.match(/\x1b\]133;C;cmd=([^\x07]*)\x07/);
    if (cmdMatch) {
      this._setState('busy', cmdMatch[1].trim());
      return;
    }

    // Check for error patterns in output
    for (const pattern of this._errorPatterns) {
      if (pattern.test(str)) {
        this._setState('error', this.currentCommand);
        return;
      }
    }

    // Fallback: regex prompt detection (for shells without OSC 133)
    if (this.state === 'busy' && this._promptPattern.test(str)) {
      this._setState('idle', null);
    }
  }

  _setState(newState, command) {
    const prev = this.state;
    this.state = newState;
    if (command !== undefined) {
      this.currentCommand = command;
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
