import { useEffect, useRef, useCallback } from "preact/hooks";
import type { RefObject } from "preact";

const SWIPE_BACK_EDGE_START_PX = 40;
const SWIPE_BACK_MIN_DX_PX = 80;
const HEADER_SWIPE_CLOSE_MIN_DX_PX = 120;
const HEADER_SWIPE_CLOSE_MAX_DY_PX = 30;
const HEADER_SWIPE_CLOSE_MAX_DURATION_MS = 350;

/**
 * Long-press gesture hook. Fires callback after `delay` ms of holding.
 * Movement or release before the timer cancels the gesture.
 */
export function useLongPress(
  callback: () => void,
  delay = 500,
): {
  onMouseDown: (e: MouseEvent) => void;
  onTouchStart: (e: TouchEvent) => void;
  onTouchMove: () => void;
  onTouchEnd: () => void;
  onMouseUp: () => void;
  onMouseLeave: () => void;
  onMouseMove: () => void;
  onContextMenu: (e: Event) => void;
} {
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const firedRef = useRef(false);

  const clear = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const start = useCallback(() => {
    firedRef.current = false;
    clear();
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      firedRef.current = true;
      if (navigator.vibrate) navigator.vibrate(50);
      callback();
    }, delay);
  }, [callback, delay, clear]);

  // Clean up on unmount
  useEffect(() => clear, [clear]);

  return {
    onMouseDown: (e: MouseEvent) => {
      // Only fire on left click, and not on thronglet children
      if (e.button !== 0) return;
      const target = e.target as HTMLElement;
      if (target.closest?.(".thronglet")) return;
      start();
    },
    onTouchStart: (e: TouchEvent) => {
      const target = e.target as HTMLElement;
      if (target.closest?.(".thronglet")) return;
      start();
    },
    onTouchMove: clear,
    onTouchEnd: clear,
    onMouseUp: clear,
    onMouseLeave: clear,
    onMouseMove: clear,
    onContextMenu: (e: Event) => e.preventDefault(),
  };
}

/**
 * Swipe-from-left-edge gesture hook for "back" navigation.
 * Fires callback when a swipe starting within 40px of the left edge
 * travels more than 80px to the right.
 */
export function isSwipeBackGesture(startX: number, endX: number): boolean {
  const dx = endX - startX;
  return startX < SWIPE_BACK_EDGE_START_PX && dx > SWIPE_BACK_MIN_DX_PX;
}

export interface HeaderSwipeCloseSample {
  startX: number;
  startY: number;
  endX: number;
  endY: number;
  durationMs: number;
}

export function isHeaderSwipeCloseGesture(
  sample: HeaderSwipeCloseSample,
): boolean {
  if (sample.startX < SWIPE_BACK_EDGE_START_PX) return false;
  if (sample.durationMs > HEADER_SWIPE_CLOSE_MAX_DURATION_MS) return false;
  const dx = sample.endX - sample.startX;
  if (Math.abs(sample.endY - sample.startY) >= HEADER_SWIPE_CLOSE_MAX_DY_PX) {
    return false;
  }
  return dx <= -HEADER_SWIPE_CLOSE_MIN_DX_PX;
}

export function useSwipeBack(
  ref: RefObject<HTMLElement>,
  callback: () => void,
): void {
  const startXRef = useRef(0);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const onTouchStart = (e: TouchEvent) => {
      startXRef.current = e.touches[0].clientX;
    };

    const onTouchEnd = (e: TouchEvent) => {
      const touch = e.changedTouches[0];
      if (!touch) return;
      if (isSwipeBackGesture(startXRef.current, touch.clientX)) {
        callback();
      }
    };

    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: true });

    return () => {
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchend", onTouchEnd);
    };
  }, [ref, callback]);
}

/**
 * Swipe-left-on-header gesture hook for fast close on mobile.
 * Keeps the first 40px reserved for global edge-swipe back.
 */
export function useHeaderSwipeClose(
  ref: RefObject<HTMLElement>,
  callback: () => void,
): void {
  const startRef = useRef<{
    x: number;
    y: number;
    at: number;
  } | null>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    const onTouchStart = (e: TouchEvent) => {
      if (e.touches.length !== 1) {
        startRef.current = null;
        return;
      }
      const touch = e.touches[0];
      startRef.current = {
        x: touch.clientX,
        y: touch.clientY,
        at: Date.now(),
      };
    };

    const onTouchEnd = (e: TouchEvent) => {
      const start = startRef.current;
      startRef.current = null;
      const touch = e.changedTouches[0];
      if (!start || !touch) return;

      const sample: HeaderSwipeCloseSample = {
        startX: start.x,
        startY: start.y,
        endX: touch.clientX,
        endY: touch.clientY,
        durationMs: Date.now() - start.at,
      };

      if (isHeaderSwipeCloseGesture(sample)) {
        callback();
      }
    };

    const clear = () => {
      startRef.current = null;
    };

    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: true });
    el.addEventListener("touchcancel", clear, { passive: true });

    return () => {
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchend", onTouchEnd);
      el.removeEventListener("touchcancel", clear);
    };
  }, [ref, callback]);
}
