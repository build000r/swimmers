import { describe, it, expect, beforeEach, vi } from "vitest";
import type { ReplayTruncatedPayload, TerminalSnapshot } from "@/types";

/**
 * Tests snapshot recovery behavior triggered by replay_truncated events.
 *
 * In TerminalWorkspace.tsx, when a replay_truncated event fires:
 *   1. fetchSnapshot(sessionId) is called
 *   2. terminal is cleared
 *   3. snapshot screen_text is written to terminal
 *   4. seqRef is updated to snapshot.latest_seq
 *
 * We test this logic by simulating the callback chain directly, avoiding
 * xterm.js DOM dependency in unit tests.
 */

describe("snapshot recovery", () => {
  it("triggers snapshot fetch on replay_truncated event", async () => {
    let fetchCalled = false;
    let fetchedSessionId: string | null = null;

    const mockFetchSnapshot = vi.fn(
      async (sessionId: string): Promise<TerminalSnapshot> => {
        fetchCalled = true;
        fetchedSessionId = sessionId;
        return {
          session_id: sessionId,
          latest_seq: 500,
          truncated: false,
          screen_text: "$ restored-content\n",
        };
      },
    );

    // Simulate the replay_truncated callback from TerminalWorkspace.tsx
    const replayTruncatedHandler = async (
      sessionId: string,
      _payload: ReplayTruncatedPayload,
    ) => {
      const targetSessionId = "sess-001";
      if (sessionId !== targetSessionId) return;
      const snapshot = await mockFetchSnapshot(sessionId);
      // In real code: term.clear(); term.write(snapshot.screen_text);
      return snapshot;
    };

    const payload: ReplayTruncatedPayload = {
      code: "replay_truncated",
      requested_resume_from_seq: 5,
      replay_window_start_seq: 100,
      latest_seq: 500,
    };

    const result = await replayTruncatedHandler("sess-001", payload);

    expect(fetchCalled).toBe(true);
    expect(fetchedSessionId).toBe("sess-001");
    expect(result).toBeDefined();
    expect(result!.latest_seq).toBe(500);
    expect(result!.screen_text).toBe("$ restored-content\n");
  });

  it("does NOT fetch snapshot for non-matching session", async () => {
    const mockFetchSnapshot = vi.fn();

    const replayTruncatedHandler = async (
      sessionId: string,
      _payload: ReplayTruncatedPayload,
    ) => {
      const targetSessionId = "sess-001";
      if (sessionId !== targetSessionId) return;
      await mockFetchSnapshot(sessionId);
    };

    const payload: ReplayTruncatedPayload = {
      code: "replay_truncated",
      requested_resume_from_seq: 5,
      replay_window_start_seq: 100,
      latest_seq: 500,
    };

    await replayTruncatedHandler("sess-999", payload);

    expect(mockFetchSnapshot).not.toHaveBeenCalled();
  });

  it("updates sequence number from snapshot response", async () => {
    let currentSeq = 5; // Simulates seqRef.current

    const mockFetchSnapshot = vi.fn(
      async (sessionId: string): Promise<TerminalSnapshot> => ({
        session_id: sessionId,
        latest_seq: 750,
        truncated: false,
        screen_text: "$ current-state\n",
      }),
    );

    const replayTruncatedHandler = async (
      sessionId: string,
      _payload: ReplayTruncatedPayload,
    ) => {
      const snapshot = await mockFetchSnapshot(sessionId);
      currentSeq = snapshot.latest_seq;
    };

    const payload: ReplayTruncatedPayload = {
      code: "replay_truncated",
      requested_resume_from_seq: 5,
      replay_window_start_seq: 100,
      latest_seq: 750,
    };

    await replayTruncatedHandler("sess-001", payload);

    expect(currentSeq).toBe(750);
  });

  it("de-duplicates recovery by incident key (guard regression)", async () => {
    let fetchCount = 0;

    const mockFetchSnapshot = vi.fn(
      async (sessionId: string): Promise<TerminalSnapshot> => {
        fetchCount++;
        return {
          session_id: sessionId,
          latest_seq: 500,
          truncated: false,
          screen_text: "$ recovered\n",
        };
      },
    );

    const seenKeys = new Set<string>();

    const replayTruncatedHandler = async (
      sessionId: string,
      payload: ReplayTruncatedPayload,
    ) => {
      const targetSessionId = "sess-001";
      if (sessionId !== targetSessionId) return;
      const incidentKey = `${payload.requested_resume_from_seq}:${payload.replay_window_start_seq}:${payload.latest_seq}`;
      if (seenKeys.has(incidentKey)) return;
      seenKeys.add(incidentKey);
      await mockFetchSnapshot(sessionId);
    };

    const payload: ReplayTruncatedPayload = {
      code: "replay_truncated",
      requested_resume_from_seq: 5,
      replay_window_start_seq: 100,
      latest_seq: 500,
    };

    // First call triggers recovery
    await replayTruncatedHandler("sess-001", payload);
    expect(fetchCount).toBe(1);

    // Same incident key: should be de-duplicated
    await replayTruncatedHandler("sess-001", payload);
    expect(fetchCount).toBe(1);

    // Different incident key: should trigger a new recovery
    const payload2: ReplayTruncatedPayload = {
      code: "replay_truncated",
      requested_resume_from_seq: 501,
      replay_window_start_seq: 600,
      latest_seq: 800,
    };
    await replayTruncatedHandler("sess-001", payload2);
    expect(fetchCount).toBe(2);
  });

  it("handles fetch failure gracefully (keeps current state)", async () => {
    let currentSeq = 42;
    let terminalCleared = false;

    const mockFetchSnapshot = vi.fn(
      async (_sessionId: string): Promise<TerminalSnapshot> => {
        throw new Error("Network error");
      },
    );

    const replayTruncatedHandler = async (
      sessionId: string,
      _payload: ReplayTruncatedPayload,
    ) => {
      try {
        const snapshot = await mockFetchSnapshot(sessionId);
        currentSeq = snapshot.latest_seq;
        terminalCleared = true;
      } catch {
        // Keep current terminal state; matches TerminalWorkspace behavior
      }
    };

    const payload: ReplayTruncatedPayload = {
      code: "replay_truncated",
      requested_resume_from_seq: 5,
      replay_window_start_seq: 100,
      latest_seq: 200,
    };

    await replayTruncatedHandler("sess-001", payload);

    // Seq should be unchanged, terminal not cleared
    expect(currentSeq).toBe(42);
    expect(terminalCleared).toBe(false);
    expect(mockFetchSnapshot).toHaveBeenCalledTimes(1);
  });
});
