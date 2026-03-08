export interface TerminalHostSize {
  width: number;
  height: number;
}

export interface TerminalGridSize {
  cols: number;
  rows: number;
}

export type TerminalHostResizeSource = "viewport" | "container";

/**
 * Returns true when the terminal host geometry has actually changed.
 */
export function hasDistinctHostSize(
  previous: TerminalHostSize | null,
  next: TerminalHostSize,
): boolean {
  if (!previous) return true;
  return previous.width !== next.width || previous.height !== next.height;
}

/**
 * Returns true when only host height changed while width stayed constant.
 */
export function isHeightOnlyHostResize(
  previous: TerminalHostSize | null,
  next: TerminalHostSize,
): boolean {
  if (!previous) return false;
  return previous.width === next.width && previous.height !== next.height;
}

/**
 * Returns true when the host should be refit for the given resize source.
 *
 * Viewport-driven height-only changes can be ignored on mobile/iOS to avoid
 * transient browser chrome churn. Container-driven changes still fit because
 * they reflect real layout changes inside the workspace.
 */
export function shouldFitTerminalHostResize(
  previous: TerminalHostSize | null,
  next: TerminalHostSize,
  options?: {
    ignoreHeightOnly?: boolean;
    source?: TerminalHostResizeSource;
  },
): boolean {
  if (!hasDistinctHostSize(previous, next)) return false;
  if (
    options?.ignoreHeightOnly &&
    options.source !== "container" &&
    isHeightOnlyHostResize(previous, next)
  ) {
    return false;
  }
  return true;
}

/**
 * Returns true when the PTY grid dimensions differ from the previous emit.
 */
export function hasDistinctTerminalGridSize(
  previous: TerminalGridSize | null,
  next: TerminalGridSize,
): boolean {
  if (!previous) return true;
  return previous.cols !== next.cols || previous.rows !== next.rows;
}
