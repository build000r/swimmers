import { describe, expect, it } from "vitest";

import { shouldOpenNativeDesktopByDefault } from "@/lib/native-desktop";

describe("native desktop routing", () => {
  it("prefers native open when desktop support is enabled", () => {
    expect(
      shouldOpenNativeDesktopByDefault(
        { supported: true, app: "iTerm" },
        false,
      ),
    ).toBe(true);
  });

  it("falls back to inline when native desktop support is unavailable", () => {
    expect(
      shouldOpenNativeDesktopByDefault(
        { supported: false, reason: "localhost only" },
        false,
      ),
    ).toBe(false);
  });

  it("falls back to inline for explicit pane placement", () => {
    expect(
      shouldOpenNativeDesktopByDefault(
        { supported: true, app: "iTerm" },
        false,
        "bottom",
      ),
    ).toBe(false);
  });

  it("never uses native open in observer mode", () => {
    expect(
      shouldOpenNativeDesktopByDefault(
        { supported: true, app: "iTerm" },
        true,
      ),
    ).toBe(false);
  });
});
