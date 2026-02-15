import { useEffect, useRef, useCallback } from "preact/hooks";
import type { RefObject } from "preact";

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
      const dx = e.changedTouches[0].clientX - startXRef.current;
      if (startXRef.current < 40 && dx > 80) {
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
