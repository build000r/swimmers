export interface TerminalHostSize {
  width: number;
  height: number;
}

export interface TerminalGridSize {
  cols: number;
  rows: number;
}

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
 * Returns true when the PTY grid dimensions differ from the previous emit.
 */
export function hasDistinctTerminalGridSize(
  previous: TerminalGridSize | null,
  next: TerminalGridSize,
): boolean {
  if (!previous) return true;
  return previous.cols !== next.cols || previous.rows !== next.rows;
}
