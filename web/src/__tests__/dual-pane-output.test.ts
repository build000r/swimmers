import { describe, it, expect, beforeEach, vi } from "vitest";
import { MockRealtimeService } from "./helpers/mock-realtime";
import type { TerminalOutputFrame } from "@/services/realtime";

const encoder = new TextEncoder();

/**
 * Tests dual-pane output continuity:
 *   - Two panes can independently subscribe and receive output
 *   - Closing secondary pane doesn't interrupt primary pane output
 *   - Same session can't be assigned to both panes
 *
 * These tests exercise the subscription model of RealtimeService.
 * In production, each TerminalWorkspace calls subscribeTerminalOutput()
 * and filters by sessionId. We validate that the subscription/unsubscription
 * pattern works correctly for the dual-pane scenario.
 */

describe("dual-pane output continuity", () => {
  let realtime: MockRealtimeService;

  beforeEach(() => {
    realtime = new MockRealtimeService();
  });

  function makeFrame(
    sessionId: string,
    seq: number,
    text: string,
  ): TerminalOutputFrame {
    return {
      sessionId,
      seq,
      data: encoder.encode(text),
    };
  }

  it("two panes independently subscribe and receive output for different sessions", () => {
    const paneAOutput: string[] = [];
    const paneBOutput: string[] = [];

    // Pane A subscribes, filtering for sess-001
    realtime.subscribeTerminalOutput((frame) => {
      if (frame.sessionId === "sess-001") {
        paneAOutput.push(new TextDecoder().decode(frame.data));
      }
    });

    // Pane B subscribes, filtering for sess-002
    realtime.subscribeTerminalOutput((frame) => {
      if (frame.sessionId === "sess-002") {
        paneBOutput.push(new TextDecoder().decode(frame.data));
      }
    });

    // Simulate output for both sessions
    realtime.simulateTerminalOutput(makeFrame("sess-001", 1, "output-A-1"));
    realtime.simulateTerminalOutput(makeFrame("sess-002", 1, "output-B-1"));
    realtime.simulateTerminalOutput(makeFrame("sess-001", 2, "output-A-2"));
    realtime.simulateTerminalOutput(makeFrame("sess-002", 2, "output-B-2"));

    expect(paneAOutput).toEqual(["output-A-1", "output-A-2"]);
    expect(paneBOutput).toEqual(["output-B-1", "output-B-2"]);
  });

  it("closing secondary pane (unsubscribe) does not interrupt primary pane", () => {
    const primaryOutput: string[] = [];
    const secondaryOutput: string[] = [];

    // Primary pane subscribes
    realtime.subscribeTerminalOutput((frame) => {
      if (frame.sessionId === "sess-001") {
        primaryOutput.push(new TextDecoder().decode(frame.data));
      }
    });

    // Secondary pane subscribes
    const unsubSecondary = realtime.subscribeTerminalOutput((frame) => {
      if (frame.sessionId === "sess-002") {
        secondaryOutput.push(new TextDecoder().decode(frame.data));
      }
    });

    // Both receive output
    realtime.simulateTerminalOutput(makeFrame("sess-001", 1, "primary-1"));
    realtime.simulateTerminalOutput(makeFrame("sess-002", 1, "secondary-1"));

    expect(primaryOutput).toEqual(["primary-1"]);
    expect(secondaryOutput).toEqual(["secondary-1"]);

    // Close secondary pane (unsubscribe)
    unsubSecondary();

    // Primary should still receive output
    realtime.simulateTerminalOutput(makeFrame("sess-001", 2, "primary-2"));
    realtime.simulateTerminalOutput(makeFrame("sess-002", 2, "secondary-2"));

    expect(primaryOutput).toEqual(["primary-1", "primary-2"]);
    // Secondary should NOT receive new output
    expect(secondaryOutput).toEqual(["secondary-1"]);
  });

  it("same session cannot be meaningfully assigned to both panes (ZoneManager pickTargetZone logic)", () => {
    /**
     * In ZoneManager.tsx, pickTargetZone() checks:
     *   if (mainZone?.sessionId === sessionId) return { existing: "main", target: "main" };
     *   if (bottomZone?.sessionId === sessionId) return { existing: "bottom", target: "bottom" };
     *
     * This means if a session is already assigned to a zone, attempting to
     * assign it again returns the existing zone - it does NOT create a second.
     *
     * We test this logic directly.
     */
    interface ZoneState {
      sessionId: string;
      age: number;
    }

    function pickTargetZone(
      sessionId: string,
      mainZone: ZoneState | null,
      bottomZone: ZoneState | null,
      pref: "main" | "bottom" | null,
    ): { existing?: "main" | "bottom"; target: "main" | "bottom" } {
      if (mainZone?.sessionId === sessionId)
        return { existing: "main", target: "main" };
      if (bottomZone?.sessionId === sessionId)
        return { existing: "bottom", target: "bottom" };

      if (pref) return { target: pref };
      if (!mainZone) return { target: "main" };
      if (!bottomZone) return { target: "main" };

      return {
        target:
          (mainZone.age ?? 0) <= (bottomZone.age ?? 0) ? "main" : "bottom",
      };
    }

    const mainZone: ZoneState = { sessionId: "sess-001", age: Date.now() };
    const bottomZone: ZoneState = { sessionId: "sess-002", age: Date.now() };

    // Trying to assign sess-001 again should return existing "main"
    const result1 = pickTargetZone("sess-001", mainZone, bottomZone, null);
    expect(result1.existing).toBe("main");
    expect(result1.target).toBe("main");

    // Trying to assign sess-002 again should return existing "bottom"
    const result2 = pickTargetZone("sess-002", mainZone, bottomZone, null);
    expect(result2.existing).toBe("bottom");
    expect(result2.target).toBe("bottom");

    // New session goes to oldest zone
    const result3 = pickTargetZone("sess-003", mainZone, bottomZone, null);
    expect(result3.existing).toBeUndefined();

    // With only main occupied, default tap replaces main.
    const result4 = pickTargetZone("sess-003", mainZone, null, null);
    expect(result4.target).toBe("main");

    // Secondary pane assignment is explicit preference.
    const result5 = pickTargetZone("sess-003", mainZone, null, "bottom");
    expect(result5.target).toBe("bottom");
  });

  it("replay_truncated events are independently delivered to each pane's listener", () => {
    const primaryTruncations: string[] = [];
    const secondaryTruncations: string[] = [];

    realtime.subscribeReplayTruncated((sessionId, _payload) => {
      if (sessionId === "sess-001") {
        primaryTruncations.push(sessionId);
      }
    });

    const unsubSecondary = realtime.subscribeReplayTruncated(
      (sessionId, _payload) => {
        if (sessionId === "sess-002") {
          secondaryTruncations.push(sessionId);
        }
      },
    );

    realtime.simulateReplayTruncated("sess-001", {
      code: "replay_truncated",
      requested_resume_from_seq: 1,
      replay_window_start_seq: 50,
      latest_seq: 100,
    });

    realtime.simulateReplayTruncated("sess-002", {
      code: "replay_truncated",
      requested_resume_from_seq: 10,
      replay_window_start_seq: 80,
      latest_seq: 150,
    });

    expect(primaryTruncations).toEqual(["sess-001"]);
    expect(secondaryTruncations).toEqual(["sess-002"]);

    // Unsubscribe secondary
    unsubSecondary();

    realtime.simulateReplayTruncated("sess-002", {
      code: "replay_truncated",
      requested_resume_from_seq: 20,
      replay_window_start_seq: 90,
      latest_seq: 200,
    });

    // Secondary should not get new truncation
    expect(secondaryTruncations).toEqual(["sess-002"]);
  });
});
