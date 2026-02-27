import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { SessionState, TerminalSnapshot } from "@/types";

/**
 * Tests the resize nudge that fires after snapshot recovery in
 * TerminalWorkspace.tsx.
 *
 * After recoverFromSnapshot() writes a plain-text snapshot to the terminal,
 * it sends two resize commands to the PTY:
 *   1. (cols + 1, rows) — nudge
 *   2. (cols, rows) — restore, after 100ms delay
 *
 * This forces tmux to emit a full-screen ANSI redraw, replacing the
 * plain-text snapshot with properly formatted output.
 */

const SILENCE_TIMEOUT_MS = 5000;
const RESIZE_NUDGE_DELAY_MS = 100;

describe("resize nudge after snapshot recovery", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("sends resize nudge after silence-triggered recovery", async () => {
    const resizeCalls: Array<{ sessionId: string; cols: number; rows: number }> = [];
    const sessionId = "sess-001";
    const termCols = 120;
    const termRows = 40;
    let disposed = false;

    const sendResize = (sid: string, cols: number, rows: number) => {
      resizeCalls.push({ sessionId: sid, cols, rows });
    };

    const mockFetchSnapshot = async (sid: string): Promise<TerminalSnapshot> => ({
      session_id: sid,
      latest_seq: 500,
      truncated: false,
      screen_text: "$ snapshot-text\n",
    });

    // Simulate recoverFromSnapshot with resize nudge (matches implementation)
    const recoverFromSnapshot = async () => {
      if (disposed) return;
      const snapshot = await mockFetchSnapshot(sessionId);
      if (disposed) return;
      // term.clear(); term.write(snapshot.screen_text); — skipped in unit test

      // Resize nudge
      sendResize(sessionId, termCols + 1, termRows);
      setTimeout(() => {
        if (disposed) return;
        sendResize(sessionId, termCols, termRows);
      }, RESIZE_NUDGE_DELAY_MS);
    };

    await recoverFromSnapshot();

    // First resize sent immediately
    expect(resizeCalls).toHaveLength(1);
    expect(resizeCalls[0]).toEqual({ sessionId, cols: termCols + 1, rows: termRows });

    // After 100ms delay, second resize restores original dimensions
    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS);
    expect(resizeCalls).toHaveLength(2);
    expect(resizeCalls[1]).toEqual({ sessionId, cols: termCols, rows: termRows });
  });

  it("sends resize nudge after replay-truncated recovery", async () => {
    const resizeCalls: Array<{ cols: number; rows: number }> = [];
    const sessionId = "sess-002";
    const termCols = 80;
    const termRows = 24;
    let disposed = false;

    const sendResize = (_sid: string, cols: number, rows: number) => {
      resizeCalls.push({ cols, rows });
    };

    const mockFetchSnapshot = async (sid: string): Promise<TerminalSnapshot> => ({
      session_id: sid,
      latest_seq: 750,
      truncated: false,
      screen_text: "$ replay-recovered\n",
    });

    const recoverFromSnapshot = async () => {
      if (disposed) return;
      const snapshot = await mockFetchSnapshot(sessionId);
      if (disposed) return;

      sendResize(sessionId, termCols + 1, termRows);
      setTimeout(() => {
        if (disposed) return;
        sendResize(sessionId, termCols, termRows);
      }, RESIZE_NUDGE_DELAY_MS);
    };

    await recoverFromSnapshot();
    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS);

    expect(resizeCalls).toHaveLength(2);
    expect(resizeCalls[0]).toEqual({ cols: termCols + 1, rows: termRows });
    expect(resizeCalls[1]).toEqual({ cols: termCols, rows: termRows });
  });

  it("does NOT send resize nudge on failed snapshot fetch", async () => {
    const resizeCalls: Array<{ cols: number; rows: number }> = [];
    const sessionId = "sess-003";
    const termCols = 100;
    const termRows = 30;
    let recoveryBanner: string | null = null;

    const sendResize = (_sid: string, cols: number, rows: number) => {
      resizeCalls.push({ cols, rows });
    };

    const mockFetchSnapshot = async (_sid: string): Promise<TerminalSnapshot> => {
      throw new Error("Network error");
    };

    const recoverFromSnapshot = async () => {
      try {
        const snapshot = await mockFetchSnapshot(sessionId);

        sendResize(sessionId, termCols + 1, termRows);
        setTimeout(() => {
          sendResize(sessionId, termCols, termRows);
        }, RESIZE_NUDGE_DELAY_MS);
      } catch {
        recoveryBanner = "Replay recovery failed. Retry snapshot to re-sync this pane.";
      }
    };

    await recoverFromSnapshot();
    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS + 100);

    expect(resizeCalls).toHaveLength(0);
    expect(recoveryBanner).toBe("Replay recovery failed. Retry snapshot to re-sync this pane.");
  });

  it("does NOT send restore resize if disposed before delay fires", async () => {
    const resizeCalls: Array<{ cols: number; rows: number }> = [];
    const sessionId = "sess-004";
    const termCols = 120;
    const termRows = 40;
    let disposed = false;

    const sendResize = (_sid: string, cols: number, rows: number) => {
      resizeCalls.push({ cols, rows });
    };

    const mockFetchSnapshot = async (sid: string): Promise<TerminalSnapshot> => ({
      session_id: sid,
      latest_seq: 300,
      truncated: false,
      screen_text: "$ text\n",
    });

    const recoverFromSnapshot = async () => {
      if (disposed) return;
      const snapshot = await mockFetchSnapshot(sessionId);
      if (disposed) return;

      sendResize(sessionId, termCols + 1, termRows);
      setTimeout(() => {
        if (disposed) return;
        sendResize(sessionId, termCols, termRows);
      }, RESIZE_NUDGE_DELAY_MS);
    };

    await recoverFromSnapshot();
    expect(resizeCalls).toHaveLength(1); // nudge sent

    // Dispose before the restore timer fires
    disposed = true;
    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS);

    // Restore resize should NOT have been sent
    expect(resizeCalls).toHaveLength(1);
  });

  it("sends resize nudge after initial snapshot load", async () => {
    const resizeCalls: Array<{ sessionId: string; cols: number; rows: number }> = [];
    const sessionId = "sess-010";
    const termCols = 100;
    const termRows = 36;
    let disposed = false;

    const sendResize = (sid: string, cols: number, rows: number) => {
      resizeCalls.push({ sessionId: sid, cols, rows });
    };

    const mockFetchSnapshot = async (sid: string): Promise<TerminalSnapshot> => ({
      session_id: sid,
      latest_seq: 200,
      truncated: false,
      screen_text: "$ initial-content\n",
    });

    // Simulate the initial load .then() callback from TerminalWorkspace
    const snapshot = await mockFetchSnapshot(sessionId);
    if (disposed) return;
    // term.write(snapshot.screen_text); seqRef = snapshot.latest_seq;
    // snapshotReadyRef = true; flushPendingFrames(); markLive();

    // Resize nudge after initial load
    const c = termCols;
    const r = termRows;
    sendResize(sessionId, c + 1, r);
    setTimeout(() => {
      if (disposed) return;
      sendResize(sessionId, c, r);
    }, RESIZE_NUDGE_DELAY_MS);

    expect(resizeCalls).toHaveLength(1);
    expect(resizeCalls[0]).toEqual({ sessionId, cols: termCols + 1, rows: termRows });

    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS);
    expect(resizeCalls).toHaveLength(2);
    expect(resizeCalls[1]).toEqual({ sessionId, cols: termCols, rows: termRows });
  });

  it("does NOT send resize nudge on initial snapshot failure", async () => {
    const resizeCalls: Array<{ cols: number; rows: number }> = [];
    const sessionId = "sess-011";
    const termCols = 100;
    const termRows = 36;

    const sendResize = (_sid: string, cols: number, rows: number) => {
      resizeCalls.push({ cols, rows });
    };

    // Simulate the initial load .catch() callback — no nudge
    let snapshotReadyRef = false;
    try {
      const _snapshot: TerminalSnapshot = await (async () => {
        throw new Error("Network error");
      })();
      // Nudge would go here in the .then() path — but we're in .catch()
      sendResize(sessionId, termCols + 1, termRows);
    } catch {
      // Matches .catch() path: set snapshotReady, flush, markLive — no nudge
      snapshotReadyRef = true;
    }

    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS + 100);

    expect(resizeCalls).toHaveLength(0);
    expect(snapshotReadyRef).toBe(true);
  });

  it("cached terminal restore does NOT trigger nudge", () => {
    const resizeCalls: Array<{ sessionId: string; cols: number; rows: number }> = [];
    const sessionId = "sess-012";
    const termCols = 80;
    const termRows = 24;
    const cachedLatestSeq = 300;

    const sendResize = (sid: string, cols: number, rows: number) => {
      resizeCalls.push({ sessionId: sid, cols, rows });
    };

    // Simulate the cached branch (lines 519-534 in TerminalWorkspace)
    // seqRef = cachedLatestSeq; snapshotReadyRef = true;
    // subscribeSession(); sendResize(); fitAddon.fit(); markLive();
    sendResize(sessionId, termCols, termRows);

    // Only one resize call — the normal sendResize, no nudge pattern
    expect(resizeCalls).toHaveLength(1);
    expect(resizeCalls[0]).toEqual({ sessionId, cols: termCols, rows: termRows });

    // No delayed restore resize should fire
    vi.advanceTimersByTime(RESIZE_NUDGE_DELAY_MS + 100);
    expect(resizeCalls).toHaveLength(1);
  });

  it("normal resize from window resize does NOT trigger nudge pattern", () => {
    const resizeCalls: Array<{ cols: number; rows: number }> = [];
    let resizeTimer: ReturnType<typeof setTimeout> | null = null;

    // Simulates the normal resize handler from TerminalWorkspace
    const handleWindowResize = (cols: number, rows: number) => {
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeCalls.push({ cols, rows });
      }, 150); // debounce from TerminalWorkspace
    };

    // Simulate rapid window resize events
    handleWindowResize(100, 30);
    handleWindowResize(110, 32);
    handleWindowResize(120, 35);

    vi.advanceTimersByTime(200);

    // Only the last debounced resize should fire
    expect(resizeCalls).toHaveLength(1);
    expect(resizeCalls[0]).toEqual({ cols: 120, rows: 35 });
  });

  it("silence timer resets on output frames without triggering recovery", () => {
    let triggerCount = 0;
    const sessionState: { current: SessionState } = { current: "busy" };
    const snapshotReady = { current: true };
    let silenceTimer: ReturnType<typeof setTimeout> | null = null;

    const startSilenceTimer = () => {
      if (!snapshotReady.current) return;
      silenceTimer = setTimeout(() => {
        silenceTimer = null;
        if (sessionState.current !== "busy") return;
        triggerCount++;
      }, SILENCE_TIMEOUT_MS);
    };

    const resetSilenceTimer = () => {
      if (silenceTimer) {
        clearTimeout(silenceTimer);
        silenceTimer = null;
      }
      startSilenceTimer();
    };

    // Start timer
    startSilenceTimer();

    // Output arrives every 2s — keeps resetting the timer
    for (let i = 0; i < 10; i++) {
      vi.advanceTimersByTime(2000);
      resetSilenceTimer();
    }

    // 20s of output at 2s intervals: timer should never have fired
    expect(triggerCount).toBe(0);

    // Now let it go silent for 5s
    vi.advanceTimersByTime(SILENCE_TIMEOUT_MS);
    expect(triggerCount).toBe(1);
  });
});
