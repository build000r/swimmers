import { describe, expect, it } from "vitest";
import {
  applyWorkspaceLayoutToUrl,
  normalizeWorkspaceLayout,
  parseWorkspaceLayoutFromUrl,
} from "@/services/workspace-history";

describe("workspace history layout", () => {
  it("parses terminal layout from URL params", () => {
    const url = new URL(
      "https://example.com/app?tt_view=terminal&tt_main=sess-1&tt_bottom=sess-2&tt_split=0.72",
    );

    const layout = parseWorkspaceLayoutFromUrl(url);

    expect(layout.view).toBe("terminal");
    expect(layout.mainSessionId).toBe("sess-1");
    expect(layout.bottomSessionId).toBe("sess-2");
    expect(layout.splitRatio).toBe(0.72);
  });

  it("normalizes duplicate session assignments to a single pane", () => {
    const url = new URL(
      "https://example.com/app?tt_view=terminal&tt_main=sess-1&tt_bottom=sess-1",
    );

    const layout = parseWorkspaceLayoutFromUrl(url);

    expect(layout.view).toBe("terminal");
    expect(layout.mainSessionId).toBe("sess-1");
    expect(layout.bottomSessionId).toBeNull();
  });

  it("drops missing sessions while preserving remaining valid layout", () => {
    const normalized = normalizeWorkspaceLayout(
      {
        view: "terminal",
        mainSessionId: "sess-1",
        bottomSessionId: "sess-2",
        splitRatio: 0.8,
      },
      new Set(["sess-2"]),
    );

    expect(normalized.view).toBe("terminal");
    expect(normalized.mainSessionId).toBe("sess-2");
    expect(normalized.bottomSessionId).toBeNull();
    expect(normalized.splitRatio).toBe(0.8);
  });

  it("writes URL params for terminal view and preserves unrelated params", () => {
    const initial = new URL("https://example.com/app?mode=observer");
    const updated = applyWorkspaceLayoutToUrl(initial, {
      view: "terminal",
      mainSessionId: "sess-7",
      bottomSessionId: null,
      splitRatio: 0.6,
    });

    expect(updated.searchParams.get("mode")).toBe("observer");
    expect(updated.searchParams.get("tt_view")).toBe("terminal");
    expect(updated.searchParams.get("tt_main")).toBe("sess-7");
    expect(updated.searchParams.get("tt_bottom")).toBeNull();
    expect(updated.searchParams.get("tt_split")).toBe("0.60");
  });
});
