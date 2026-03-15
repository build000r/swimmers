import { describe, expect, it } from "vitest";
import {
  computeVisualViewportBottomInsetPx,
  hasMeaningfulDelta,
  isLikelyIOSDevice,
  shouldIgnoreHeightOnlyTerminalFit,
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
        userAgent:
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
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

    expect(
      isLikelyIOSDevice({
        userAgent:
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
        platform: "MacIntel",
        maxTouchPoints: 5,
      }),
    ).toBe(false);
  });

  it("enables WebGL on modern iOS but still supports explicit overrides", () => {
    const ios = {
      userAgent:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15",
      platform: "iPhone",
      maxTouchPoints: 5,
    };
    const legacyIos = {
      userAgent:
        "Mozilla/5.0 (iPhone; CPU iPhone OS 15_7 like Mac OS X) AppleWebKit/605.1.15",
      platform: "iPhone",
      maxTouchPoints: 5,
    };
    const desktop = {
      userAgent: "Mozilla/5.0 (X11; Linux x86_64)",
      platform: "Linux x86_64",
      maxTouchPoints: 0,
    };

    expect(shouldEnableTerminalWebgl(ios, null)).toBe(true);
    expect(shouldEnableTerminalWebgl(legacyIos, null)).toBe(false);
    expect(shouldEnableTerminalWebgl(desktop, null)).toBe(true);
    expect(shouldEnableTerminalWebgl(ios, "off")).toBe(false);
    expect(shouldEnableTerminalWebgl(desktop, "off")).toBe(false);
  });

  it("suppresses height-only terminal fits on mobile-sized viewports", () => {
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

    expect(shouldIgnoreHeightOnlyTerminalFit(ios, false)).toBe(true);
    expect(shouldIgnoreHeightOnlyTerminalFit(desktop, true)).toBe(true);
    expect(shouldIgnoreHeightOnlyTerminalFit(desktop, false)).toBe(false);
  });

  it("computes bottom inset from visual viewport geometry", () => {
    expect(computeVisualViewportBottomInsetPx(844, 560, 0)).toBe(284);
    expect(computeVisualViewportBottomInsetPx(844, 844, 0)).toBe(0);
    expect(computeVisualViewportBottomInsetPx(844, 500.5, 60.2)).toBe(283);
    expect(computeVisualViewportBottomInsetPx(844, 500.5, Number.NaN)).toBe(344);
  });

  it("treats tiny pixel drift as non-meaningful when thresholded", () => {
    expect(hasMeaningfulDelta(100, 100, 2)).toBe(false);
    expect(hasMeaningfulDelta(100, 101, 2)).toBe(false);
    expect(hasMeaningfulDelta(100, 102, 2)).toBe(true);
  });
});
