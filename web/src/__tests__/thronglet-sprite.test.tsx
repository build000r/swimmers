import { describe, it, expect } from "vitest";
import { render } from "@testing-library/preact";
import { h } from "preact";
import { ThrongletSprite } from "@/components/ThrongletSprite";
import type { SessionState } from "@/types";

function isoAgo(ms: number): string {
  return new Date(Date.now() - ms).toISOString();
}

function renderedTitle(state: SessionState, lastActivityAt?: string): string | null {
  const { container } = render(
    <ThrongletSprite state={state} lastActivityAt={lastActivityAt} />,
  );
  return container.querySelector("title")?.textContent ?? null;
}

function renderedStyleForTool(tool: string): string {
  const { container } = render(<ThrongletSprite state="busy" tool={tool} />);
  return container.firstElementChild?.getAttribute("style") ?? "";
}

describe("ThrongletSprite idle-depth selection", () => {
  it("uses active for recently idle sessions", () => {
    expect(renderedTitle("idle", isoAgo(5_000))).toBe("Thronglet - Active");
  });

  it("uses drowsy after 20s idle", () => {
    expect(renderedTitle("idle", isoAgo(30_000))).toBe("Thronglet - Drowsy");
  });

  it("uses sleeping after 60s idle", () => {
    expect(renderedTitle("idle", isoAgo(90_000))).toBe("Thronglet - Sleeping");
  });

  it("uses deep sleep after 120s idle", () => {
    expect(renderedTitle("idle", isoAgo(180_000))).toBe("Thronglet - Deep Sleep");
  });
});

describe("ThrongletSprite tool color mapping", () => {
  it("uses Claude palette for claude aliases", () => {
    expect(renderedStyleForTool("claude")).toContain("--thr-body: #E07B39");
    expect(renderedStyleForTool("Claude Code")).toContain("--thr-body: #E07B39");
  });

  it("uses Codex palette for codex aliases", () => {
    expect(renderedStyleForTool("codex")).toContain("--thr-body: #F4C542");
    expect(renderedStyleForTool("Codex")).toContain("--thr-body: #F4C542");
  });
});
