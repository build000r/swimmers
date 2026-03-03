/**
 * Throngterm video theme — Claude Code orange palette.
 *
 * Used across all throngterm Remotion compositions (tweet cards, promos, etc.).
 * Matches the session-to-tweet diagram palette.
 */

export const FPS = 30;

export const colors = {
  // Claude Code orange palette
  orange: "#E07B39",
  orangeLight: "#F5C4A1",
  orangeDark: "#8B3D1F",
  orangeAccent: "#D97757",

  // Surfaces
  bgDark: "#05070f",
  bgMid: "#111827",
  surface: "#0f172a",
  surfaceLight: "#1e293b",

  // Borders
  border: "#334155",
  borderLight: "#475569",

  // Text
  textPrimary: "#f9fafb",
  textSecondary: "#cbd5e1",
  textBody: "#e5e7eb",
  textDim: "#9ca3af",

  // Shirt blue (from thronglet sprite)
  blue: "#7AAFC8",

  // Status
  black: "#1A1A1A",
  white: "#FFFFFF",
} as const;

/** Twitter/X image dimensions by layout mode. */
export const twitterLayouts = {
  /** 16:9 — single image, full-width in feed, no crop */
  single: { width: 1200, height: 675 },
  /** 7:8 — two images side-by-side */
  dual: { width: 700, height: 800 },
  /** 2:1 — four-image grid */
  quad: { width: 1200, height: 600 },
} as const;

export const fonts = {
  mono: "'SF Mono', 'Fira Code', 'Consolas', monospace",
  body: "'Inter', sans-serif",
} as const;

export const branding = {
  account: "@throngterm",
  displayName: "thronglet_01",
  human: "@buildooor",
  tagline: "@throngterm · human orchestration by @buildooor",
} as const;
