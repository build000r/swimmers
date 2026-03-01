import { describe, expect, it } from "vitest";
import {
  beginTouchScroll,
  createTouchScrollState,
  endTouchScroll,
  nextTouchScrollTop,
} from "@/lib/touch-scroll";

describe("touch scroll helper", () => {
  it("returns null when no gesture is active", () => {
    const state = createTouchScrollState();
    expect(nextTouchScrollTop(state, 120)).toBeNull();
  });

  it("computes upward swipe as positive scroll delta", () => {
    const state = createTouchScrollState();
    beginTouchScroll(state, 700, 100);
    expect(nextTouchScrollTop(state, 520)).toBe(280);
  });

  it("computes downward swipe as negative scroll delta", () => {
    const state = createTouchScrollState();
    beginTouchScroll(state, 500, 220);
    expect(nextTouchScrollTop(state, 640)).toBe(80);
  });

  it("stops computing after gesture ends", () => {
    const state = createTouchScrollState();
    beginTouchScroll(state, 500, 220);
    endTouchScroll(state);
    expect(nextTouchScrollTop(state, 460)).toBeNull();
  });
});
