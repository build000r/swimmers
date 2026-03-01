import { describe, expect, it } from "vitest";
import {
  computeFastScrollThumbHeight,
  computeFastScrollThumbTop,
  computeScrollTopFromThumbOffset,
  hasFastScrollOverflow,
  isNearScrollBottom,
} from "@/lib/fast-scroll";

describe("fast scroll helpers", () => {
  it("detects overflow only when content exceeds viewport", () => {
    expect(hasFastScrollOverflow(500, 300)).toBe(true);
    expect(hasFastScrollOverflow(300, 300)).toBe(false);
  });

  it("computes thumb height proportionally with minimum size", () => {
    const thumb = computeFastScrollThumbHeight(400, 2000, 400, 28);
    expect(thumb).toBe(80);
    const small = computeFastScrollThumbHeight(100, 9000, 100, 28);
    expect(small).toBe(28);
  });

  it("maps scroll position to thumb top and back", () => {
    const scrollTop = 720;
    const scrollHeight = 2400;
    const clientHeight = 400;
    const trackHeight = 400;
    const thumbHeight = computeFastScrollThumbHeight(
      clientHeight,
      scrollHeight,
      trackHeight,
      28,
    );
    const thumbTop = computeFastScrollThumbTop(
      scrollTop,
      scrollHeight,
      clientHeight,
      trackHeight,
      thumbHeight,
    );
    const mapped = computeScrollTopFromThumbOffset(
      thumbTop,
      scrollHeight,
      clientHeight,
      trackHeight,
      thumbHeight,
    );
    expect(Math.abs(mapped - scrollTop)).toBeLessThanOrEqual(12);
  });

  it("detects near-bottom within tolerance", () => {
    expect(isNearScrollBottom(595, 1000, 400, 6)).toBe(true);
    expect(isNearScrollBottom(560, 1000, 400, 6)).toBe(false);
  });
});
