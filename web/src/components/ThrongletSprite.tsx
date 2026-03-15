import { useMemo, useRef } from "preact/hooks";
import type { RepoTheme, RestState, SessionState, SpritePack } from "@/types";
import { ACTIVE, DROWSY, SLEEPING, DEEP_SLEEP } from "@/lib/thronglet-svgs";

// ---- Color presets per tool ----

const TOOL_COLORS: Record<string, Record<string, string>> = {
  "Claude Code": {
    "--thr-body": "#E07B39",
    "--thr-outline": "#8B3D1F",
    "--thr-accent": "#6B2A12",
    "--thr-shirt": "#7AAFC8",
  },
  Codex: {
    "--thr-body": "#F4C542",
    "--thr-outline": "#8B6B00",
    "--thr-accent": "#5E4600",
    "--thr-shirt": "#7AAFC8",
  },
};

const DEFAULT_COLORS = TOOL_COLORS["Claude Code"];

function cssVarsForRepoTheme(theme: RepoTheme): Record<string, string> {
  return {
    "--thr-body": theme.body,
    "--thr-outline": theme.outline,
    "--thr-accent": theme.accent,
    "--thr-shirt": theme.shirt,
  };
}

const SPRITE_SCOPE_ATTR = "data-thr-scope";
let spriteScopeSeq = 0;

function nextSpriteScopeId(): string {
  spriteScopeSeq += 1;
  return `thr-scope-${spriteScopeSeq}`;
}

export function scopeInlineSpriteCss(svg: string, scopeId: string): string {
  const scopeSelector = `[${SPRITE_SCOPE_ATTR}="${scopeId}"]`;
  const withScopeAttr = svg.replace(/<svg\b([^>]*)>/i, (match, attrs) => {
    if (attrs.includes(SPRITE_SCOPE_ATTR)) return match;
    return `<svg${attrs} ${SPRITE_SCOPE_ATTR}="${scopeId}">`;
  });

  return withScopeAttr.replace(/<style>([\s\S]*?)<\/style>/i, (_match, css) => {
    const scopedCss = css.replace(
      /(^|})\s*\.([A-Za-z_][\w-]*)\s*\{/g,
      (_rule: string, prefix: string, className: string) =>
        `${prefix} ${scopeSelector} .${className} {`,
    );
    return `<style>${scopedCss}</style>`;
  });
}

function canonicalToolName(tool?: string | null): keyof typeof TOOL_COLORS | null {
  if (!tool) return null;
  const normalized = tool.trim().toLowerCase();
  const normalizedSeparators = normalized.replace(/[-_]+/g, " ");
  const compact = normalizedSeparators.replace(/\s+/g, "");

  if (
    normalized === "claude" ||
    normalized === "claude code" ||
    normalized === "claude-code" ||
    normalized === "claude_code" ||
    compact === "claudecode" ||
    /\bclaude\b/.test(normalizedSeparators)
  ) {
    return "Claude Code";
  }

  if (
    normalized === "codex" ||
    normalized === "codex-cli" ||
    normalized === "codex_cli" ||
    compact === "codexcli" ||
    /\bcodex\b/.test(normalizedSeparators)
  ) {
    return "Codex";
  }

  return null;
}

// ---- Idle-depth SVG selection ----

function svgForState(
  state: SessionState,
  restState: RestState,
  spritePack?: SpritePack | null,
): string {
  const active = spritePack?.active ?? ACTIVE;
  const drowsy = spritePack?.drowsy ?? DROWSY;
  const sleeping = spritePack?.sleeping ?? SLEEPING;
  const deepSleep = spritePack?.deep_sleep ?? DEEP_SLEEP;

  if (state === "exited") return deepSleep;
  switch (restState) {
    case "deep_sleep":
      return deepSleep;
    case "sleeping":
      return sleeping;
    case "drowsy":
      return drowsy;
    default:
      return active;
  }
}

// ---- Component ----

interface ThrongletSpriteProps {
  state: SessionState;
  restState: RestState;
  tool?: string | null;
  spritePack?: SpritePack | null;
  repoTheme?: RepoTheme | null;
  class?: string;
}

export function ThrongletSprite({
  state,
  restState,
  tool,
  spritePack,
  repoTheme,
  class: className,
}: ThrongletSpriteProps) {
  const scopeIdRef = useRef<string>("");
  if (!scopeIdRef.current) scopeIdRef.current = nextSpriteScopeId();

  const svg = useMemo(
    () => svgForState(state, restState, spritePack),
    [state, restState, spritePack],
  );
  const toolName = canonicalToolName(tool);

  // Repo theme colors are the authoritative palette. Without a repo theme,
  // custom sprite packs keep their baked-in defaults and plain sprites fall
  // back to tool-based colors.
  const colors = repoTheme
    ? cssVarsForRepoTheme(repoTheme)
    : spritePack
      ? null
      : (toolName && TOOL_COLORS[toolName]) ?? DEFAULT_COLORS;

  const style = useMemo(
    () => ({
      ...(colors ?? {}),
      display: "inline-block",
      width: "100%",
      height: "100%",
    }),
    [colors],
  );

  const scopedSvg = useMemo(
    () => scopeInlineSpriteCss(svg, scopeIdRef.current),
    [svg],
  );
  const htmlObj = useMemo(() => ({ __html: scopedSvg }), [scopedSvg]);

  return (
    <div class={className} style={style}>
      <div dangerouslySetInnerHTML={htmlObj} style={{ width: "100%", height: "100%" }} />
    </div>
  );
}
