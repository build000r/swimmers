const StateDetector = require('../server/state-detector');

describe('StateDetector', () => {
  let detector;

  beforeEach(() => {
    detector = new StateDetector();
  });

  afterEach(() => {
    // Clean up any pending timers the detector may have scheduled
    detector._clearErrorTimer();
    detector._clearAttentionTimer();
  });

  describe('initial state', () => {
    it('starts as idle', () => {
      expect(detector.state).toBe('idle');
    });

    it('starts with no current command', () => {
      expect(detector.currentCommand).toBeNull();
    });
  });

  describe('getState()', () => {
    it('returns an object with state and currentCommand', () => {
      const result = detector.getState();
      expect(result).toEqual({ state: 'idle', currentCommand: null });
    });

    it('reflects the current state after transitions', () => {
      detector.processOutput('\x1b]133;C;cmd=npm test\x07');
      const result = detector.getState();
      expect(result).toEqual({ state: 'busy', currentCommand: 'npm test' });
    });
  });

  describe('OSC 133 sequences', () => {
    it('detects prompt start (133;A) as idle', () => {
      // First move to busy so idle transition is meaningful
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      expect(detector.state).toBe('busy');

      detector.processOutput('\x1b]133;A\x07');
      expect(detector.state).toBe('idle');
    });

    it('detects command start (133;C) as busy with command name', () => {
      detector.processOutput('\x1b]133;C;cmd=make build\x07');
      expect(detector.state).toBe('busy');
      expect(detector.currentCommand).toBe('make build');
    });

    it('trims whitespace from command name', () => {
      detector.processOutput('\x1b]133;C;cmd=  ls -la  \x07');
      expect(detector.currentCommand).toBe('ls -la');
    });

    it('clears current command on prompt start', () => {
      detector.processOutput('\x1b]133;C;cmd=make\x07');
      expect(detector.currentCommand).toBe('make');

      detector.processOutput('\x1b]133;A\x07');
      expect(detector.currentCommand).toBeNull();
    });

    it('handles OSC 133;A embedded in other output', () => {
      detector.processOutput('\x1b]133;C;cmd=echo hello\x07');
      detector.processOutput('some output\x1b]133;A\x07');
      expect(detector.state).toBe('idle');
    });
  });

  describe('prompt detection via regex fallback', () => {
    it('detects output ending with "$ " as idle when busy', () => {
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      expect(detector.state).toBe('busy');

      detector.processOutput('user@host:~$ ');
      expect(detector.state).toBe('idle');
    });

    it('detects output ending with "% " as idle when busy', () => {
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      detector.processOutput('user@host% ');
      expect(detector.state).toBe('idle');
    });

    it('detects output ending with "# " as idle when busy', () => {
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      detector.processOutput('root@host:~# ');
      expect(detector.state).toBe('idle');
    });

    it('does not transition from idle to idle via regex (no redundant transition)', () => {
      const changes = [];
      detector.onStateChange((info) => changes.push(info));

      // Already idle, regex prompt should not fire a state change
      detector.processOutput('user@host:~$ ');
      expect(changes).toHaveLength(0);
    });

    it('detects prompt even when mixed with ANSI color codes', () => {
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      // Prompt with color codes that should be stripped
      detector.processOutput('\x1b[32muser@host\x1b[0m:\x1b[34m~\x1b[0m$ ');
      expect(detector.state).toBe('idle');
    });
  });

  describe('error detection', () => {
    it('detects "command not found"', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      detector.processOutput('bash: foo: command not found');
      expect(detector.state).toBe('error');
    });

    it('detects "Permission denied"', () => {
      detector.processOutput('\x1b]133;C;cmd=cat /etc/shadow\x07');
      detector.processOutput('cat: /etc/shadow: Permission denied');
      expect(detector.state).toBe('error');
    });

    it('detects "segmentation fault"', () => {
      detector.processOutput('\x1b]133;C;cmd=./broken\x07');
      detector.processOutput('Segmentation fault (core dumped)');
      expect(detector.state).toBe('error');
    });

    it('detects "panic:"', () => {
      detector.processOutput('\x1b]133;C;cmd=go run main.go\x07');
      detector.processOutput('goroutine 1 [running]:\npanic: runtime error');
      expect(detector.state).toBe('error');
    });

    it('is case-insensitive for "command not found"', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      detector.processOutput('COMMAND NOT FOUND');
      expect(detector.state).toBe('error');
    });

    it('preserves current command when entering error state', () => {
      detector.processOutput('\x1b]133;C;cmd=badcmd\x07');
      detector.processOutput('badcmd: command not found');
      expect(detector.state).toBe('error');
      expect(detector.currentCommand).toBe('badcmd');
    });
  });

  describe('error auto-clear', () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('auto-clears error state after 4000ms', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      detector.processOutput('command not found');
      expect(detector.state).toBe('error');

      vi.advanceTimersByTime(3999);
      expect(detector.state).toBe('error');

      vi.advanceTimersByTime(1);
      expect(detector.state).toBe('idle');
    });

    it('resets error timer if a new error arrives', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      detector.processOutput('command not found');
      expect(detector.state).toBe('error');

      vi.advanceTimersByTime(3000);
      // Another error resets the timer
      detector.processOutput('Permission denied');

      vi.advanceTimersByTime(3000);
      // Should still be in error (timer was reset)
      expect(detector.state).toBe('error');

      vi.advanceTimersByTime(1000);
      expect(detector.state).toBe('idle');
    });

    it('cancels error timer when prompt is detected', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      detector.processOutput('command not found');
      expect(detector.state).toBe('error');

      // Prompt arrives, clearing the error
      detector.processOutput('user@host:~$ ');
      expect(detector.state).toBe('idle');

      // Advancing past the original timer should not cause any issues
      vi.advanceTimersByTime(5000);
      expect(detector.state).toBe('idle');
    });
  });

  describe('state transitions', () => {
    it('transitions idle -> busy on command start', () => {
      expect(detector.state).toBe('idle');
      detector.processOutput('\x1b]133;C;cmd=npm install\x07');
      expect(detector.state).toBe('busy');
    });

    it('transitions busy -> idle on prompt', () => {
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      expect(detector.state).toBe('busy');

      detector.processOutput('\x1b]133;A\x07');
      expect(detector.state).toBe('idle');
    });

    it('transitions busy -> error on error pattern', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      expect(detector.state).toBe('busy');

      detector.processOutput('bash: foo: command not found');
      expect(detector.state).toBe('error');
    });

    it('transitions error -> idle on prompt', () => {
      detector.processOutput('\x1b]133;C;cmd=foo\x07');
      detector.processOutput('command not found');
      expect(detector.state).toBe('error');

      detector.processOutput('user@host:~$ ');
      expect(detector.state).toBe('idle');
    });

    it('fires onStateChange callback with correct info', () => {
      const changes = [];
      detector.onStateChange((info) => changes.push({ ...info }));

      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      expect(changes).toHaveLength(1);
      expect(changes[0]).toEqual({
        state: 'busy',
        previousState: 'idle',
        currentCommand: 'ls',
      });

      detector.processOutput('\x1b]133;A\x07');
      expect(changes).toHaveLength(2);
      expect(changes[1]).toEqual({
        state: 'idle',
        previousState: 'busy',
        currentCommand: null,
      });
    });

    it('does not fire onStateChange when state does not change', () => {
      const changes = [];
      detector.onStateChange((info) => changes.push(info));

      // idle -> idle should not fire
      detector.processOutput('\x1b]133;A\x07');
      expect(changes).toHaveLength(0);
    });
  });

  describe('attention state', () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('transitions to attention after 10s of idle following busy', () => {
      detector.processOutput('\x1b]133;C;cmd=make\x07');
      expect(detector.state).toBe('busy');

      detector.processOutput('\x1b]133;A\x07');
      expect(detector.state).toBe('idle');

      vi.advanceTimersByTime(9999);
      expect(detector.state).toBe('idle');

      vi.advanceTimersByTime(1);
      expect(detector.state).toBe('attention');
    });

    it('does not trigger attention if never busy first', () => {
      // Starting idle, wait 10s — should stay idle, no attention
      vi.advanceTimersByTime(15000);
      expect(detector.state).toBe('idle');
    });

    it('cancels attention timer if a new command starts', () => {
      // busy -> idle (starts attention timer)
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      detector.processOutput('\x1b]133;A\x07');
      expect(detector.state).toBe('idle');

      vi.advanceTimersByTime(5000);
      // New command starts, should cancel the attention timer
      detector.processOutput('\x1b]133;C;cmd=make\x07');
      expect(detector.state).toBe('busy');

      // Go back to idle
      detector.processOutput('\x1b]133;A\x07');

      // Advance past original 10s mark — should not be attention yet
      // because the timer was reset on the second busy->idle transition
      vi.advanceTimersByTime(5000);
      expect(detector.state).toBe('idle');

      vi.advanceTimersByTime(5000);
      expect(detector.state).toBe('attention');
    });

    it('does not cancel attention timer on idle re-detection', () => {
      // busy -> idle (starts attention timer)
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      detector.processOutput('\x1b]133;A\x07');
      expect(detector.state).toBe('idle');

      // Simulate prompt re-detection while already idle (tmux redraw)
      vi.advanceTimersByTime(5000);
      detector.processOutput('\x1b]133;A\x07');
      expect(detector.state).toBe('idle');

      // Attention should still fire at the original 10s mark
      vi.advanceTimersByTime(5000);
      expect(detector.state).toBe('attention');
    });

    it('can be dismissed with dismissAttention()', () => {
      detector.processOutput('\x1b]133;C;cmd=ls\x07');
      detector.processOutput('\x1b]133;A\x07');

      vi.advanceTimersByTime(10000);
      expect(detector.state).toBe('attention');

      detector.dismissAttention();
      expect(detector.state).toBe('idle');
    });
  });

  describe('_stripAnsi()', () => {
    it('strips CSI color sequences', () => {
      const result = detector._stripAnsi('\x1b[32mgreen\x1b[0m');
      expect(result).toBe('green');
    });

    it('strips OSC sequences', () => {
      const result = detector._stripAnsi('\x1b]0;title\x07');
      expect(result).toBe('');
    });

    it('strips control characters but keeps tab, newline, carriage return', () => {
      const result = detector._stripAnsi('hello\tworld\nfoo\rbar\x01baz');
      expect(result).toBe('hello\tworld\nfoo\rbarbaz');
    });
  });
});
