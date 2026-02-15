const ScrollGuard = require('../server/scroll-guard');

describe('ScrollGuard', () => {
  let guard;
  let emitted;
  let emitCallback;

  beforeEach(() => {
    vi.useFakeTimers();
    emitted = [];
    emitCallback = (data) => emitted.push(data);
    guard = new ScrollGuard(emitCallback);
  });

  afterEach(() => {
    guard.destroy();
    vi.useRealTimers();
  });

  describe('constructor', () => {
    it('takes an onOutput callback', () => {
      const cb = vi.fn();
      const g = new ScrollGuard(cb);
      g.process('hello');
      expect(cb).toHaveBeenCalledWith('hello');
      g.destroy();
    });
  });

  describe('normal output passes through', () => {
    it('forwards data immediately when it has few cursor sequences', () => {
      guard.process('hello world\r\n');
      expect(emitted).toEqual(['hello world\r\n']);
    });

    it('forwards data with fewer than 10 cursor sequences immediately', () => {
      // 9 cursor-positioning sequences — below threshold
      let data = '';
      for (let i = 0; i < 9; i++) {
        data += `\x1b[${i + 1};1H line ${i}`;
      }
      guard.process(data);
      expect(emitted).toHaveLength(1);
      expect(emitted[0]).toBe(data);
    });

    it('forwards multiple normal chunks in order', () => {
      guard.process('chunk1');
      guard.process('chunk2');
      guard.process('chunk3');
      expect(emitted).toEqual(['chunk1', 'chunk2', 'chunk3']);
    });
  });

  describe('scroll detection', () => {
    function makeScrollData(count = 15) {
      let data = '';
      for (let i = 0; i < count; i++) {
        data += `\x1b[${i + 1};1H content line ${i}`;
      }
      return data;
    }

    it('detects data with >= 10 cursor-positioning sequences as a scroll redraw', () => {
      const scrollData = makeScrollData(10);
      guard.process(scrollData);

      // Should not emit immediately — it's buffered
      expect(emitted).toHaveLength(0);
    });

    it('emits buffered scroll data after 32ms coalesce delay', () => {
      const scrollData = makeScrollData(12);
      guard.process(scrollData);

      expect(emitted).toHaveLength(0);

      vi.advanceTimersByTime(32);
      expect(emitted).toHaveLength(1);
      expect(emitted[0]).toBe(scrollData);
    });

    it('recognizes \\x1b[H (cursor home) as a cursor-positioning sequence', () => {
      let data = '';
      for (let i = 0; i < 10; i++) {
        data += `\x1b[${i + 1}H`;
      }
      guard.process(data);
      // Should be buffered (detected as scroll)
      expect(emitted).toHaveLength(0);
      vi.advanceTimersByTime(32);
      expect(emitted).toHaveLength(1);
    });

    it('recognizes \\x1b[row;colH sequences', () => {
      let data = '';
      for (let i = 0; i < 12; i++) {
        data += `\x1b[${i + 1};${5}H`;
      }
      guard.process(data);
      expect(emitted).toHaveLength(0);
    });
  });

  describe('coalescing', () => {
    function makeScrollData(id) {
      let data = '';
      for (let i = 0; i < 15; i++) {
        data += `\x1b[${i + 1};1H frame-${id} line ${i}`;
      }
      return data;
    }

    it('coalesces multiple rapid scroll redraws, emitting only the last', () => {
      const frame1 = makeScrollData(1);
      const frame2 = makeScrollData(2);
      const frame3 = makeScrollData(3);

      guard.process(frame1);
      vi.advanceTimersByTime(10);
      guard.process(frame2);
      vi.advanceTimersByTime(10);
      guard.process(frame3);

      // Nothing emitted yet
      expect(emitted).toHaveLength(0);

      // Wait for the coalesce timer to fire
      vi.advanceTimersByTime(32);

      // Only the last frame should be emitted
      expect(emitted).toHaveLength(1);
      expect(emitted[0]).toBe(frame3);
    });

    it('flushes buffered scroll data when normal output arrives', () => {
      const scrollData = makeScrollData('scroll');
      guard.process(scrollData);

      expect(emitted).toHaveLength(0);

      // Normal output arrives — should flush the buffer first, then emit normal output
      guard.process('normal output');

      expect(emitted).toHaveLength(2);
      expect(emitted[0]).toBe(scrollData);
      expect(emitted[1]).toBe('normal output');
    });
  });

  describe('input grace period', () => {
    function makeScrollData() {
      let data = '';
      for (let i = 0; i < 20; i++) {
        data += `\x1b[${i + 1};1H line ${i}`;
      }
      return data;
    }

    it('passes through scroll-like output immediately after notifyInput()', () => {
      guard.notifyInput();
      const scrollData = makeScrollData();
      guard.process(scrollData);

      // Should emit immediately — within the 200ms input grace period
      expect(emitted).toHaveLength(1);
      expect(emitted[0]).toBe(scrollData);
    });

    it('grace period lasts 200ms', () => {
      guard.notifyInput();

      vi.advanceTimersByTime(199);
      const scrollData = makeScrollData();
      guard.process(scrollData);
      // Still within grace period
      expect(emitted).toHaveLength(1);
    });

    it('resumes coalescing after grace period expires', () => {
      guard.notifyInput();

      vi.advanceTimersByTime(200);
      const scrollData = makeScrollData();
      guard.process(scrollData);
      // Grace period expired — should be buffered
      expect(emitted).toHaveLength(0);

      vi.advanceTimersByTime(32);
      expect(emitted).toHaveLength(1);
    });

    it('flushes any pending buffer when input grace triggers pass-through', () => {
      // First, get some data buffered
      const frame1 = makeScrollData();
      guard.process(frame1);
      expect(emitted).toHaveLength(0);

      // Now notify input and send more scroll data
      guard.notifyInput();
      const frame2 = makeScrollData();
      guard.process(frame2);

      // Should flush the old buffer and emit the new data immediately
      expect(emitted).toHaveLength(2);
      expect(emitted[0]).toBe(frame1);
      expect(emitted[1]).toBe(frame2);
    });
  });

  describe('destroy()', () => {
    it('clears pending timers and buffer', () => {
      let data = '';
      for (let i = 0; i < 15; i++) {
        data += `\x1b[${i + 1};1H line`;
      }
      guard.process(data);
      expect(emitted).toHaveLength(0);

      guard.destroy();

      // Timer should have been cleared — advancing time should not emit
      vi.advanceTimersByTime(100);
      expect(emitted).toHaveLength(0);
    });
  });
});
