import { describe, expect, it } from "vitest";
import {
  computeVisualViewportBottomInsetPx,
  hasMeaningfulDelta,
  isLikelyIOSDevice,
  shouldEnableTerminalWebgl,
} from "@/lib/mobile-perf";

describe("mobile performance helpers", () => {
  it("detects iPhone and iPadOS touch signatures", () => {
    expect(
      isLikelyIOSDevice({
        userAgent:
          "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
        platform: "iPhone",
        maxTouchPoints: 5,
      }),
    ).toBe(true);

    expect(
      isLikelyIOSDevice({
        userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        platform: "MacIntel",
        maxTouchPoints: 5,
      }),
    ).toBe(true);
  });

  it("keeps desktop signatures out of iOS path", () => {
    expect(
      isLikelyIOSDevice({
        userAgent: "Mozilla/5.0 (X11; Linux x86_64)",
        platform: "Linux x86_64",
        maxTouchPoints: 0,
      }),
    ).toBe(false);
  });

  it("disables WebGL on iOS by default but supports explicit overrides", () => {
    const ios = {
      userAgent:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
      platform: "iPhone",
      maxTouchPoints: 5,
    };
    const desktop = {
      userAgent: "Mozilla/5.0 (X11; Linux x86_64)",
      platform: "Linux x86_64",
      maxTouchPoints: 0,
    };

    expect(shouldEnableTerminalWebgl(ios, null)).toBe(false);
    expect(shouldEnableTerminalWebgl(desktop, null)).toBe(true);
    expect(shouldEnableTerminalWebgl(ios, "on")).toBe(true);
    expect(shouldEnableTerminalWebgl(desktop, "off")).toBe(false);
  });

  it("computes bottom inset from visual viewport geometry", () => {
    expect(computeVisualViewportBottomInsetPx(844, 560, 0)).toBe(284);
    expect(computeVisualViewportBottomInsetPx(844, 844, 0)).toBe(0);
    expect(computeVisualViewportBottomInsetPx(844, 500.5, 60.2)).toBe(283);
  });

  it("treats tiny pixel drift as non-meaningful when thresholded", () => {
    expect(hasMeaningfulDelta(100, 100, 2)).toBe(false);
    expect(hasMeaningfulDelta(100, 101, 2)).toBe(false);
    expect(hasMeaningfulDelta(100, 102, 2)).toBe(true);
  });
});
