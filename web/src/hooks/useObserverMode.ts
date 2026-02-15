import { useMemo } from "preact/hooks";

/**
 * Determines whether the current client is in observer (read-only) mode.
 *
 * Observer mode is activated by:
 *   1. URL parameter `?mode=observer`
 *   2. Bootstrap response `auth_mode === "observer"` (when backend auth lands)
 *
 * Observers can view terminal output but cannot:
 *   - Send terminal input
 *   - Create or delete sessions
 *   - Dismiss attention state
 */
export function useObserverMode(authMode?: string): { isObserver: boolean } {
  const isObserver = useMemo(() => {
    // Check URL parameter first (for testing / client-side override)
    const params = new URLSearchParams(window.location.search);
    if (params.get("mode") === "observer") return true;

    // Check bootstrap auth_mode when backend auth is available
    if (authMode === "observer") return true;

    return false;
  }, [authMode]);

  return { isObserver };
}
