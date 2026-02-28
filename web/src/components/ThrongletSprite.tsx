import { useEffect, useMemo, useState } from "preact/hooks";
import type { SessionState, SpritePack } from "@/types";
import { ACTIVE, DROWSY, SLEEPING, DEEP_SLEEP } from "@/lib/thronglet-svgs";
import {
  DEEP_SLEEP_AFTER_MS,
  DROWSY_AFTER_MS,
  SLEEPING_AFTER_MS,
} from "@/lib/thronglet-motion";

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

const IDLE_SPRITE_TICK_MS = 5_000;

function svgForState(
  state: SessionState,
  lastActivityAt?: string,
  spritePack?: SpritePack | null,
): string {
  const active = spritePack?.active ?? ACTIVE;
  const drowsy = spritePack?.drowsy ?? DROWSY;
  const sleeping = spritePack?.sleeping ?? SLEEPING;
  const deepSleep = spritePack?.deep_sleep ?? DEEP_SLEEP;

  if (state === "idle" || state === "attention") {
    if (!lastActivityAt) return drowsy;
    const lastMs = new Date(lastActivityAt).getTime();
    if (!Number.isFinite(lastMs)) return drowsy;
    const idleMs = Date.now() - lastMs;
    if (idleMs >= DEEP_SLEEP_AFTER_MS) return deepSleep;
    if (idleMs >= SLEEPING_AFTER_MS) return sleeping;
    if (idleMs >= DROWSY_AFTER_MS) return drowsy;
    return active;
  }
  if (state === "exited") return deepSleep;
  return active; // busy, error, attention
}

// ---- Tool badge labels ----

const TOOL_BADGE: Record<string, string> = {
  "Claude Code": "C",
  Codex: "X",
};

// ---- Component ----

interface ThrongletSpriteProps {
  state: SessionState;
  tool?: string | null;
  lastActivityAt?: string;
  spritePack?: SpritePack | null;
  class?: string;
}

export function ThrongletSprite({
  state,
  tool,
  lastActivityAt,
  spritePack,
  class: className,
}: ThrongletSpriteProps) {
  const [idleTick, setIdleTick] = useState(0);

  // Keep rest-state sprites transitioning (active -> drowsy -> sleeping)
  // even when no other props change.
  useEffect(() => {
    if (state !== "idle" && state !== "attention") return;
    const timer = setInterval(() => {
      setIdleTick((value) => value + 1);
    }, IDLE_SPRITE_TICK_MS);
    return () => clearInterval(timer);
  }, [state, lastActivityAt]);

  const svg = useMemo(
    () => svgForState(state, lastActivityAt, spritePack),
    [state, lastActivityAt, spritePack, idleTick],
  );
  const toolName = canonicalToolName(tool);

  // When a sprite pack is present, let its baked-in brand colors show through.
  // Only apply tool color CSS vars for default (non-branded) sprites.
  const hasBrandSprites = !!spritePack;
  const colors = hasBrandSprites
    ? null
    : (toolName && TOOL_COLORS[toolName]) ?? DEFAULT_COLORS;

  const style = useMemo(
    () => ({
      ...(colors ?? {}),
      display: "inline-block",
      width: "100%",
      height: "100%",
      position: "relative" as const,
    }),
    [colors],
  );

  const htmlObj = useMemo(() => ({ __html: svg }), [svg]);
  const badgeLabel = hasBrandSprites && toolName ? TOOL_BADGE[toolName] : null;

  return (
    <div class={className} style={style}>
      <div dangerouslySetInnerHTML={htmlObj} style={{ width: "100%", height: "100%" }} />
      {badgeLabel && (
        <span class="thronglet-tool-badge" data-tool={toolName}>{badgeLabel}</span>
      )}
    </div>
  );
}
