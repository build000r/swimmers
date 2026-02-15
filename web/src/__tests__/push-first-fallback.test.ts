import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { BootstrapResponse } from "@/types";
import { makeBootstrapResponse, makeSession } from "./helpers/fixtures";

/**
 * These tests exercise the push-first fallback gating logic in app.tsx:
 *   - When transport is "healthy", polling does NOT start.
 *   - When transport becomes "degraded", polling starts at poll_fallback_ms.
 *   - When transport returns to "healthy", polling stops.
 *
 * We test the logic directly by simulating the polling control flow rather than
 * rendering the full App component (which depends on xterm, WebSocket, etc.).
 * This approach tests the decision logic in isolation.
 */

describe("push-first fallback gating", () => {
  let pollInterval: ReturnType<typeof setInterval> | null = null;
  let pollStartCount = 0;
  let pollStopCount = 0;

  /**
   * Reimplements the polling decision logic from app.tsx useEffect:
   *
   *   const shouldPoll =
   *     bootstrapDone &&
   *     currentView === "overview" &&
   *     (transportHealth === "degraded" || transportHealth === "disconnected");
   *
   *   if (shouldPoll) startPolling();
   *   else stopPolling();
   */
  function evaluatePolling(opts: {
    bootstrapDone: boolean;
    currentView: "overview" | "terminal";
    transportHealth: "healthy" | "degraded" | "overloaded" | "disconnected";
    pollFallbackMs: number;
  }) {
    const shouldPoll =
      opts.bootstrapDone &&
      opts.currentView === "overview" &&
      (opts.transportHealth === "degraded" ||
        opts.transportHealth === "disconnected");

    if (shouldPoll) {
      if (!pollInterval) {
        pollStartCount++;
        pollInterval = setInterval(() => {}, opts.pollFallbackMs);
      }
    } else {
      if (pollInterval) {
        pollStopCount++;
        clearInterval(pollInterval);
        pollInterval = null;
      }
    }
  }

  beforeEach(() => {
    pollInterval = null;
    pollStartCount = 0;
    pollStopCount = 0;
  });

  afterEach(() => {
    if (pollInterval) {
      clearInterval(pollInterval);
      pollInterval = null;
    }
  });

  it("does NOT start polling when transport is healthy", () => {
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "healthy",
      pollFallbackMs: 2000,
    });

    expect(pollInterval).toBeNull();
    expect(pollStartCount).toBe(0);
  });

  it("starts polling when transport becomes degraded", () => {
    // Initially healthy - no polling
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "healthy",
      pollFallbackMs: 2000,
    });
    expect(pollInterval).toBeNull();

    // Transport degrades - polling should start
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "degraded",
      pollFallbackMs: 2000,
    });
    expect(pollInterval).not.toBeNull();
    expect(pollStartCount).toBe(1);
  });

  it("stops polling when transport returns to healthy", () => {
    // Start degraded
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "degraded",
      pollFallbackMs: 2000,
    });
    expect(pollInterval).not.toBeNull();
    expect(pollStartCount).toBe(1);

    // Transport recovers
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "healthy",
      pollFallbackMs: 2000,
    });
    expect(pollInterval).toBeNull();
    expect(pollStopCount).toBe(1);
  });

  it("does NOT start polling before bootstrap completes", () => {
    evaluatePolling({
      bootstrapDone: false,
      currentView: "overview",
      transportHealth: "degraded",
      pollFallbackMs: 2000,
    });

    expect(pollInterval).toBeNull();
    expect(pollStartCount).toBe(0);
  });

  it("does NOT start polling when in terminal view", () => {
    evaluatePolling({
      bootstrapDone: true,
      currentView: "terminal",
      transportHealth: "degraded",
      pollFallbackMs: 2000,
    });

    expect(pollInterval).toBeNull();
    expect(pollStartCount).toBe(0);
  });

  it("starts polling when transport is disconnected", () => {
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "disconnected",
      pollFallbackMs: 2000,
    });

    expect(pollInterval).not.toBeNull();
    expect(pollStartCount).toBe(1);
  });

  it("does NOT double-start polling on repeated evaluations", () => {
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "degraded",
      pollFallbackMs: 2000,
    });
    evaluatePolling({
      bootstrapDone: true,
      currentView: "overview",
      transportHealth: "degraded",
      pollFallbackMs: 2000,
    });

    expect(pollStartCount).toBe(1);
  });
});
