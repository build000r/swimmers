import { useMemo } from "preact/hooks";
import type { SessionState } from "@/types";
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

function canonicalToolName(tool?: string | null): keyof typeof TOOL_COLORS | null {
  if (!tool) return null;
  const normalized = tool.trim().toLowerCase();

  if (
    normalized === "claude" ||
    normalized === "claude code" ||
    normalized === "claude-code" ||
    normalized === "claude_code"
  ) {
    return "Claude Code";
  }

  if (
    normalized === "codex" ||
    normalized === "codex-cli" ||
    normalized === "codex_cli"
  ) {
    return "Codex";
  }

  return null;
}

// ---- Idle-depth SVG selection ----

const DROWSY_AFTER_MS = 20_000;
const SLEEPING_AFTER_MS = 60_000;
const DEEP_SLEEP_AFTER_MS = 120_000;

function svgForState(state: SessionState, lastActivityAt?: string): string {
  if (state === "idle") {
    if (!lastActivityAt) return DROWSY;
    const lastMs = new Date(lastActivityAt).getTime();
    if (!Number.isFinite(lastMs)) return DROWSY;
    const idleMs = Date.now() - lastMs;
    if (idleMs >= DEEP_SLEEP_AFTER_MS) return DEEP_SLEEP;
    if (idleMs >= SLEEPING_AFTER_MS) return SLEEPING;
    if (idleMs >= DROWSY_AFTER_MS) return DROWSY;
    return ACTIVE;
  }
  if (state === "exited") return DEEP_SLEEP;
  return ACTIVE; // busy, error, attention
}

// ---- Component ----

interface ThrongletSpriteProps {
  state: SessionState;
  tool?: string | null;
  lastActivityAt?: string;
  class?: string;
}

export function ThrongletSprite({
  state,
  tool,
  lastActivityAt,
  class: className,
}: ThrongletSpriteProps) {
  const svg = svgForState(state, lastActivityAt);
  const toolName = canonicalToolName(tool);
  const colors = (toolName && TOOL_COLORS[toolName]) ?? DEFAULT_COLORS;

  const style = useMemo(
    () => ({
      ...colors,
      display: "inline-block",
      width: "100%",
      height: "100%",
    }),
    [colors],
  );

  const htmlObj = useMemo(() => ({ __html: svg }), [svg]);

  return (
    <div
      class={className}
      style={style}
      dangerouslySetInnerHTML={htmlObj}
    />
  );
}
