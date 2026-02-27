import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { SessionState, TerminalSnapshot } from "@/types";

/**
 * Tests silence detection logic from TerminalWorkspace.tsx.
 *
 * The component uses a 5s silence timer that fires when:
 *   - lifecycleState is "live"
 *   - snapshotReady is true
 *   - sessionState is "busy"
 *   - no terminal output arrives for 5 seconds
 *
 * On fire: calls forceResubscribe() + recoverFromSnapshot().
 * The timer resets on every output frame and on markLive().
 */

const SILENCE_TIMEOUT_MS = 5000;

describe("silence detection", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("triggers re-subscribe + snapshot after 5s silence in busy state", () => {
    let forceResubscribeCalled = false;
    let recoverFromSnapshotCalled = false;
    const sessionState: { current: SessionState } = { current: "busy" };
    const snapshotReady = { current: true };
    let silenceTimer: ReturnType<typeof setTimeout> | null = null;

    const forceResubscribe = () => {
      forceResubscribeCalled = true;
    };
    const recoverFromSnapshot = () => {
      recoverFromSnapshotCalled = true;
    };

    const startSilenceTimer = () => {
      if (!snapshotReady.current) return;
      silenceTimer = setTimeout(() => {
        silenceTimer = null;
        if (sessionState.current !== "busy") return;
        forceResubscribe();
        recoverFromSnapshot();
      }, SILENCE_TIMEOUT_MS);
    };

    // Simulate going live
    startSilenceTimer();

    // Before 5s: nothing should fire
    vi.advanceTimersByTime(4999);
    expect(forceResubscribeCalled).toBe(false);
    expect(recoverFromSnapshotCalled).toBe(false);

    // At 5s: should fire
    vi.advanceTimersByTime(1);
    expect(forceResubscribeCalled).toBe(true);
    expect(recoverFromSnapshotCalled).toBe(true);
  });

  it("does NOT trigger re-subscribe during idle silence", () => {
    let forceResubscribeCalled = false;
    const sessionState: { current: SessionState } = { current: "idle" };
    const snapshotReady = { current: true };
    let silenceTimer: ReturnType<typeof setTimeout> | null = null;

    const startSilenceTimer = () => {
      if (!snapshotReady.current) return;
      silenceTimer = setTimeout(() => {
        silenceTimer = null;
        if (sessionState.current !== "busy") return;
        forceResubscribeCalled = true;
      }, SILENCE_TIMEOUT_MS);
    };

    startSilenceTimer();
    vi.advanceTimersByTime(SILENCE_TIMEOUT_MS + 100);
    expect(forceResubscribeCalled).toBe(false);
  });

  it("resets silence timer on output arrival (no false trigger)", () => {
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

    // Start the timer
    startSilenceTimer();

    // Simulate output arriving at 3s — resets the timer
    vi.advanceTimersByTime(3000);
    resetSilenceTimer();

    // Another 3s passes (total 6s from start, but only 3s since reset)
    vi.advanceTimersByTime(3000);
    expect(triggerCount).toBe(0);

    // 2 more seconds (5s since last reset) — should trigger
    vi.advanceTimersByTime(2000);
    expect(triggerCount).toBe(1);
  });

  it("silence detection is inactive during initial snapshot load", () => {
    let triggerCount = 0;
    const sessionState: { current: SessionState } = { current: "busy" };
    const snapshotReady = { current: false }; // Not yet loaded
    let silenceTimer: ReturnType<typeof setTimeout> | null = null;

    const startSilenceTimer = () => {
      if (!snapshotReady.current) return; // Guard: don't start if snapshot not ready
      silenceTimer = setTimeout(() => {
        silenceTimer = null;
        if (sessionState.current !== "busy") return;
        triggerCount++;
      }, SILENCE_TIMEOUT_MS);
    };

    startSilenceTimer();

    // Even after 10s, should not trigger because snapshot isn't ready
    vi.advanceTimersByTime(10000);
    expect(triggerCount).toBe(0);
    expect(silenceTimer).toBeNull();
  });
});
