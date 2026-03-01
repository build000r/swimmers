export interface TouchScrollState {
  active: boolean;
  startY: number;
  startScrollTop: number;
}

export function createTouchScrollState(): TouchScrollState {
  return {
    active: false,
    startY: 0,
    startScrollTop: 0,
  };
}

export function beginTouchScroll(
  state: TouchScrollState,
  startY: number,
  startScrollTop: number,
): void {
  state.active = true;
  state.startY = startY;
  state.startScrollTop = startScrollTop;
}

export function endTouchScroll(state: TouchScrollState): void {
  state.active = false;
}

/**
 * Returns the next scrollTop for the active gesture, or null when inactive.
 */
export function nextTouchScrollTop(
  state: TouchScrollState,
  currentY: number,
): number | null {
  if (!state.active) return null;
  const delta = state.startY - currentY;
  return state.startScrollTop + delta;
}
