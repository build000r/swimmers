import { describe, it, expect } from "vitest";
import { render } from "@testing-library/preact";
import { h } from "preact";
import { ThrongletSprite } from "@/components/ThrongletSprite";
import type { SessionState, SpritePack } from "@/types";

function isoAgo(ms: number): string {
  return new Date(Date.now() - ms).toISOString();
}

function renderedTitle(state: SessionState, lastActivityAt?: string): string | null {
  const { container } = render(
    <ThrongletSprite state={state} lastActivityAt={lastActivityAt} />,
  );
  return container.querySelector("title")?.textContent ?? null;
}

function renderedStyleForTool(tool: string, spritePack?: SpritePack | null): string {
  const { container } = render(
    <ThrongletSprite state="busy" tool={tool} spritePack={spritePack} />,
  );
  return container.firstElementChild?.getAttribute("style") ?? "";
}

const FAKE_SPRITE_PACK: SpritePack = {
  active: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Active</title><rect x="0" y="0" width="16" height="16" class="b"/></svg>',
  drowsy: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Drowsy</title></svg>',
  sleeping: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Sleeping</title></svg>',
  deep_sleep: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Deep Sleep</title></svg>',
};

describe("ThrongletSprite idle-depth selection", () => {
  it("uses active for recently idle sessions", () => {
    expect(renderedTitle("idle", isoAgo(5_000))).toBe("Thronglet - Active");
  });

  it("uses idle-depth sprites while in attention", () => {
    expect(renderedTitle("attention", isoAgo(30_000))).toBe("Thronglet - Drowsy");
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
    expect(renderedStyleForTool("Codex CLI")).toContain("--thr-body: #F4C542");
  });
});

describe("ThrongletSprite brand preservation with sprite pack", () => {
  it("suppresses tool color CSS vars when sprite pack is present", () => {
    const style = renderedStyleForTool("Claude Code", FAKE_SPRITE_PACK);
    expect(style).not.toContain("--thr-body");
    expect(style).not.toContain("--thr-outline");
    expect(style).not.toContain("--thr-accent");
    expect(style).not.toContain("--thr-shirt");
  });

  it("renders tool badge when sprite pack + tool are present", () => {
    const { container } = render(
      <ThrongletSprite state="busy" tool="Claude Code" spritePack={FAKE_SPRITE_PACK} />,
    );
    const badge = container.querySelector(".thronglet-tool-badge");
    expect(badge).not.toBeNull();
    expect(badge!.textContent).toBe("C");
    expect(badge!.getAttribute("data-tool")).toBe("Claude Code");
  });

  it("renders Codex badge distinct from Claude", () => {
    const { container } = render(
      <ThrongletSprite state="busy" tool="Codex" spritePack={FAKE_SPRITE_PACK} />,
    );
    const badge = container.querySelector(".thronglet-tool-badge");
    expect(badge).not.toBeNull();
    expect(badge!.textContent).toBe("X");
    expect(badge!.getAttribute("data-tool")).toBe("Codex");
  });

  it("still applies tool colors when no sprite pack", () => {
    const style = renderedStyleForTool("Claude Code", null);
    expect(style).toContain("--thr-body: #E07B39");
    expect(style).toContain("--thr-outline: #8B3D1F");
  });

  it("renders no badge when sprite pack present but no tool", () => {
    const { container } = render(
      <ThrongletSprite state="busy" spritePack={FAKE_SPRITE_PACK} />,
    );
    expect(container.querySelector(".thronglet-tool-badge")).toBeNull();
  });
});
