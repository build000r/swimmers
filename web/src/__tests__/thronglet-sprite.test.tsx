import { describe, it, expect } from "vitest";
import { render } from "@testing-library/preact";
import { h } from "preact";
import { ThrongletSprite, scopeInlineSpriteCss } from "@/components/ThrongletSprite";
import type { RepoTheme, RestState, SessionState, SpritePack } from "@/types";

function renderedTitle(restState: RestState, state: SessionState = "idle"): string | null {
  const { container } = render(<ThrongletSprite state={state} restState={restState} />);
  return container.querySelector("title")?.textContent ?? null;
}

function renderedStyleForTool(
  tool: string,
  spritePack?: SpritePack | null,
  repoTheme?: RepoTheme | null,
): string {
  const { container } = render(
    <ThrongletSprite
      state="busy"
      restState="active"
      tool={tool}
      spritePack={spritePack}
      repoTheme={repoTheme}
    />,
  );
  return container.firstElementChild?.getAttribute("style") ?? "";
}

const FAKE_SPRITE_PACK: SpritePack = {
  active: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Active</title><style>.b { fill: var(--thr-body, #AA5500); }</style><rect x="0" y="0" width="16" height="16" class="b"/></svg>',
  drowsy: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Drowsy</title><style>.b { fill: var(--thr-body, #AA5500); }</style></svg>',
  sleeping: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Sleeping</title><style>.b { fill: var(--thr-body, #AA5500); }</style></svg>',
  deep_sleep: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Brand - Deep Sleep</title><style>.b { fill: var(--thr-body, #AA5500); }</style></svg>',
};

const ALT_FAKE_SPRITE_PACK: SpritePack = {
  active: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Alt - Active</title><style>.b { fill: var(--thr-body, #00AA55); }</style><rect x="0" y="0" width="16" height="16" class="b"/></svg>',
  drowsy: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Alt - Drowsy</title><style>.b { fill: var(--thr-body, #00AA55); }</style></svg>',
  sleeping: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Alt - Sleeping</title><style>.b { fill: var(--thr-body, #00AA55); }</style></svg>',
  deep_sleep: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><title>Alt - Deep Sleep</title><style>.b { fill: var(--thr-body, #00AA55); }</style></svg>',
};

const BUILD_THEME: RepoTheme = {
  body: "#B89875",
  outline: "#3D2F24",
  accent: "#1D1914",
  shirt: "#AA9370",
};

describe("ThrongletSprite idle-depth selection", () => {
  it("uses active when rest state is active", () => {
    expect(renderedTitle("active")).toBe("Thronglet - Active");
  });

  it("uses daemon drowsy while in attention", () => {
    expect(renderedTitle("drowsy", "attention")).toBe("Thronglet - Drowsy");
  });

  it("uses drowsy from daemon rest state", () => {
    expect(renderedTitle("drowsy")).toBe("Thronglet - Drowsy");
  });

  it("uses sleeping from daemon rest state", () => {
    expect(renderedTitle("sleeping")).toBe("Thronglet - Sleeping");
  });

  it("uses deep sleep from daemon rest state", () => {
    expect(renderedTitle("deep_sleep")).toBe("Thronglet - Deep Sleep");
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

  it("does not render a tool badge when sprite pack + tool are present", () => {
    const { container } = render(
      <ThrongletSprite
        state="busy"
        restState="active"
        tool="Claude Code"
        spritePack={FAKE_SPRITE_PACK}
      />,
    );
    expect(container.querySelector(".thronglet-tool-badge")).toBeNull();
  });

  it("scopes inline SVG class selectors per sprite instance", () => {
    const { container } = render(
      <div>
        <ThrongletSprite
          state="busy"
          restState="active"
          tool="Claude Code"
          spritePack={FAKE_SPRITE_PACK}
        />
        <ThrongletSprite
          state="busy"
          restState="active"
          tool="Codex"
          spritePack={ALT_FAKE_SPRITE_PACK}
        />
      </div>,
    );
    const svgs = container.querySelectorAll("svg");
    expect(svgs).toHaveLength(2);

    const scopeA = svgs[0].getAttribute("data-thr-scope");
    const scopeB = svgs[1].getAttribute("data-thr-scope");
    expect(scopeA).toBeTruthy();
    expect(scopeB).toBeTruthy();
    expect(scopeA).not.toBe(scopeB);

    const scopedA = scopeInlineSpriteCss(FAKE_SPRITE_PACK.active, "scope-a");
    const scopedB = scopeInlineSpriteCss(ALT_FAKE_SPRITE_PACK.active, "scope-b");
    expect(scopedA).toContain('data-thr-scope="scope-a"');
    expect(scopedA).toContain('[data-thr-scope="scope-a"] .b');
    expect(scopedA).toContain("#AA5500");
    expect(scopedB).toContain('data-thr-scope="scope-b"');
    expect(scopedB).toContain('[data-thr-scope="scope-b"] .b');
    expect(scopedB).toContain("#00AA55");
  });

  it("still applies tool colors when no sprite pack", () => {
    const style = renderedStyleForTool("Claude Code", null);
    expect(style).toContain("--thr-body: #E07B39");
    expect(style).toContain("--thr-outline: #8B3D1F");
  });

  it("applies repo theme colors before tool fallback", () => {
    const style = renderedStyleForTool("Codex", null, BUILD_THEME);
    expect(style).toContain("--thr-body: #B89875");
    expect(style).toContain("--thr-outline: #3D2F24");
    expect(style).not.toContain("--thr-body: #F4C542");
  });

  it("applies repo theme colors even when sprite pack is present", () => {
    const style = renderedStyleForTool("Claude Code", FAKE_SPRITE_PACK, BUILD_THEME);
    expect(style).toContain("--thr-body: #B89875");
    expect(style).toContain("--thr-outline: #3D2F24");
    expect(style).toContain("--thr-accent: #1D1914");
    expect(style).toContain("--thr-shirt: #AA9370");
  });

  it("renders no badge when sprite pack present but no tool", () => {
    const { container } = render(
      <ThrongletSprite state="busy" restState="active" spritePack={FAKE_SPRITE_PACK} />,
    );
    expect(container.querySelector(".thronglet-tool-badge")).toBeNull();
  });
});
