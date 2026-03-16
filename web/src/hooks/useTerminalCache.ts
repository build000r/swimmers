import { useRef, useCallback, useEffect } from "preact/hooks";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";

export interface CachedTerminal {
  term: Terminal;
  fitAddon: FitAddon;
  hostEl: HTMLDivElement;
  sessionId: string;
  latestSeq: number;
}

interface CacheEntry {
  cached: CachedTerminal;
  timer: ReturnType<typeof setTimeout> | null;
}

/**
 * Terminal instance cache. Keeps xterm + addon alive across view navigations.
 * Evicts entries after `ttlMs` of inactivity.
 */
export const useTerminalCache = function (ttlMs: number) {
  const entriesRef = useRef<Map<string, CacheEntry>>(new Map());

  const evict = useCallback((sessionId: string) => {
    const entries = entriesRef.current;
    const entry = entries.get(sessionId);
    if (!entry) return;
    if (entry.timer !== null) clearTimeout(entry.timer);
    entry.cached.term.dispose();
    if (entry.cached.hostEl.parentNode) {
      entry.cached.hostEl.parentNode.removeChild(entry.cached.hostEl);
    }
    entries.delete(sessionId);
  }, []);

  const get = useCallback(
    (sessionId: string): CachedTerminal | null => {
      const entries = entriesRef.current;
      const entry = entries.get(sessionId);
      if (!entry) return null;
      // Cancel eviction timer since it's being restored
      if (entry.timer !== null) {
        clearTimeout(entry.timer);
        entry.timer = null;
      }
      return entry.cached;
    },
    [],
  );

  const put = useCallback(
    (cached: CachedTerminal) => {
      const entries = entriesRef.current;
      const existing = entries.get(cached.sessionId);
      if (existing && existing.timer !== null) {
        clearTimeout(existing.timer);
      }

      const timer = setTimeout(() => evict(cached.sessionId), ttlMs);
      entries.set(cached.sessionId, { cached, timer });
    },
    [ttlMs, evict],
  );

  const has = useCallback((sessionId: string): boolean => {
    return entriesRef.current.has(sessionId);
  }, []);

  // Clean up all entries on unmount
  useEffect(() => {
    return () => {
      const entries = entriesRef.current;
      for (const [id] of entries) {
        evict(id);
      }
    };
  }, [evict]);

  return { get, put, evict, has };
};
