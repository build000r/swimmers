import { describe, expect, it } from "vitest";
import { makeSession } from "./helpers/fixtures";
import {
  deriveThrongletBubble,
  deriveThrongletGauge,
} from "@/components/OverviewField";

describe("deriveThrongletBubble", () => {
  it("uses idle preview text for idle sessions without a thought", () => {
    const session = makeSession({
      state: "idle",
      thought: null,
      session_id: "sess-idle",
    });

    expect(deriveThrongletBubble(session, "recent build output")).toEqual({
      activityText: "recent build output",
      bubbleText: "recent build output",
      bubbleIdlePreview: true,
      showBubble: true,
      showRenderedBubble: true,
    });
  });

  it("suppresses prompt-like idle preview text", () => {
    const session = makeSession({
      state: "idle",
      thought: null,
      session_id: "sess-prompt",
    });

    expect(deriveThrongletBubble(session, "$ npm run test")).toEqual({
      activityText: "",
      bubbleText: "",
      bubbleIdlePreview: false,
      showBubble: false,
      showRenderedBubble: false,
    });
  });

  it("prefers thought text when present", () => {
    const session = makeSession({
      state: "idle",
      thought: "thinking",
      session_id: "sess-thought",
    });

    expect(deriveThrongletBubble(session, "preview text")).toEqual({
      activityText: "thinking",
      bubbleText: "thinking",
      bubbleIdlePreview: false,
      showBubble: true,
      showRenderedBubble: true,
    });
  });

  it("uses status fallback labels for attention/error", () => {
    const errorSession = makeSession({ state: "error", session_id: "sess-err" });
    const attentionSession = makeSession({
      state: "attention",
      session_id: "sess-attn",
    });

    expect(deriveThrongletBubble(errorSession, "")).toEqual({
      activityText: "error!",
      bubbleText: "!!!",
      bubbleIdlePreview: false,
      showBubble: true,
      showRenderedBubble: true,
    });
    expect(deriveThrongletBubble(attentionSession, "")).toEqual({
      activityText: "ready",
      bubbleText: "ready",
      bubbleIdlePreview: false,
      showBubble: true,
      showRenderedBubble: true,
    });
  });
});

describe("deriveThrongletGauge", () => {
  it("builds remaining-context gauge values", () => {
    const session = makeSession({
      token_count: 50_000,
      context_limit: 200_000,
      session_id: "sess-gauge",
    });

    expect(deriveThrongletGauge(session)).toEqual({
      showGauge: true,
      gaugeRatio: 0.25,
      gaugeFillSegments: 6,
      gaugeFillWidth: "75%",
      gaugePercentLeft: 75,
      isCritical: false,
    });
  });

  it("clamps over-limit token usage to empty", () => {
    const session = makeSession({
      token_count: 999_999,
      context_limit: 200_000,
      session_id: "sess-over",
    });

    expect(deriveThrongletGauge(session)).toEqual({
      showGauge: true,
      gaugeRatio: 1,
      gaugeFillSegments: 0,
      gaugeFillWidth: "0%",
      gaugePercentLeft: 0,
      isCritical: true,
    });
  });

  it("hides gauge when usage is unavailable", () => {
    const session = makeSession({
      token_count: 0,
      context_limit: 0,
      session_id: "sess-none",
    });

    expect(deriveThrongletGauge(session)).toEqual({
      showGauge: false,
      gaugeRatio: 0,
      gaugeFillSegments: 8,
      gaugeFillWidth: "100%",
      gaugePercentLeft: 100,
      isCritical: false,
    });
  });
});
