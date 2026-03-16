import { useEffect, useRef, useState, useCallback } from "preact/hooks";
import type { SessionSummary } from "@/types";
import type { WorkspaceLayoutState } from "@/services/workspace-history";
import { TerminalWorkspace } from "@/components/TerminalWorkspace";
import { useTerminalCache } from "@/hooks/useTerminalCache";
import { useSwipeBack } from "@/hooks/useGestures";
import { terminalCacheTtlMs, zoneLayout } from "@/app";
import type { CachedTerminal } from "@/hooks/useTerminalCache";
import { realtime } from "@/app";

// ---- Helpers ----

function isDesktop(): boolean {
  return window.innerWidth > 768;
}

interface ZoneState {
  sessionId: string;
  age: number; // timestamp for "replace oldest"
}

interface RestoreLayoutRequest extends WorkspaceLayoutState {
  requestId: number;
}

interface ZoneManagerProps {
  sessions: SessionSummary[];
  activeSessionId: string | null;
  preferZone: "main" | "bottom" | null;
  restoreRequest: RestoreLayoutRequest | null;
  observer?: boolean;
  benchedIds?: Set<string>;
  onBenchToggle?: (sessionId: string) => void;
  onShowOverview: () => void;
  onStartPolling: () => void;
  onStopPolling: () => void;
  onLayoutChange: (layout: WorkspaceLayoutState) => void;
}

export const ZoneManager = function ({
  sessions,
  activeSessionId,
  preferZone,
  restoreRequest,
  observer = false,
  benchedIds,
  onBenchToggle,
  onShowOverview,
  onStartPolling,
  onStopPolling,
  onLayoutChange,
}: ZoneManagerProps) {
  const [mainZone, setMainZone] = useState<ZoneState | null>(null);
  const [bottomZone, setBottomZone] = useState<ZoneState | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.6);
  const openSequenceRef = useRef({ main: 0, bottom: 0 });
  const mainZoneRef = useRef<ZoneState | null>(null);
  const bottomZoneRef = useRef<ZoneState | null>(null);

  const dividerRef = useRef<HTMLDivElement>(null);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const cache = useTerminalCache(terminalCacheTtlMs.value);

  useEffect(() => {
    mainZoneRef.current = mainZone;
    bottomZoneRef.current = bottomZone;
  }, [mainZone, bottomZone]);

  // ---- Swipe back gesture ----
  useSwipeBack(wrapperRef, () => {
    closeAllZones();
    onShowOverview();
  });

  // ---- Zone target selection ----

  const pickTargetZone = useCallback(
    (
      sessionId: string,
      pref: "main" | "bottom" | null,
    ): { existing?: "main" | "bottom"; target: "main" | "bottom" } => {
      if (mainZone?.sessionId === sessionId) return { existing: "main", target: "main" };
      if (bottomZone?.sessionId === sessionId)
        return { existing: "bottom", target: "bottom" };

      if (!isDesktop()) {
        return { target: "main" };
      }

      if (pref) return { target: pref };
      if (!mainZone) return { target: "main" };
      if (!bottomZone) return { target: "bottom" };

      // Both occupied: replace oldest
      return {
        target:
          (mainZone.age ?? 0) <= (bottomZone.age ?? 0) ? "main" : "bottom",
      };
    },
    [mainZone, bottomZone],
  );

  // ---- Assign active session to a zone ----

  useEffect(() => {
    if (!activeSessionId) return;

    const { existing, target } = pickTargetZone(activeSessionId, preferZone);
    if (existing) return; // Already visible

    const sequence = ++openSequenceRef.current[target];

    queueMicrotask(() => {
      if (openSequenceRef.current[target] !== sequence) return;
      const now = Date.now();

      // On mobile, enforce single-pane behavior and detach any secondary stream.
      if (!isDesktop()) {
        setBottomZone((prev) => {
          if (prev) {
            realtime.unsubscribeSession(prev.sessionId);
          }
          return null;
        });
      }

      if (target === "main") {
        setMainZone((prev) => {
          if (prev && prev.sessionId !== activeSessionId) {
            realtime.unsubscribeSession(prev.sessionId);
          }
          return { sessionId: activeSessionId, age: now };
        });
      } else {
        setBottomZone((prev) => {
          if (prev && prev.sessionId !== activeSessionId) {
            realtime.unsubscribeSession(prev.sessionId);
          }
          return { sessionId: activeSessionId, age: now };
        });
      }
    });
  }, [activeSessionId, preferZone, pickTargetZone]);

  useEffect(() => {
    if (!restoreRequest) return;

    const availableIds = new Set(sessions.map((s) => s.session_id));
    let mainSessionId =
      restoreRequest.mainSessionId &&
      availableIds.has(restoreRequest.mainSessionId)
        ? restoreRequest.mainSessionId
        : null;
    let bottomSessionId =
      restoreRequest.bottomSessionId &&
      availableIds.has(restoreRequest.bottomSessionId)
        ? restoreRequest.bottomSessionId
        : null;

    if (mainSessionId && bottomSessionId && mainSessionId === bottomSessionId) {
      bottomSessionId = null;
    }
    if (!mainSessionId && bottomSessionId) {
      mainSessionId = bottomSessionId;
      bottomSessionId = null;
    }

    const previousAssignments = [
      mainZoneRef.current?.sessionId,
      bottomZoneRef.current?.sessionId,
    ].filter((value): value is string => !!value);
    const nextAssignments = [mainSessionId, bottomSessionId].filter(
      (value): value is string => !!value,
    );

    for (const sessionId of previousAssignments) {
      if (!nextAssignments.includes(sessionId)) {
        realtime.unsubscribeSession(sessionId);
      }
    }

    if (!mainSessionId && !bottomSessionId) {
      setMainZone(null);
      setBottomZone(null);
      onShowOverview();
      return;
    }

    const now = Date.now();
    const clampedSplitRatio = Math.max(0.2, Math.min(0.8, restoreRequest.splitRatio));

    setMainZone((prev) => {
      const prevSessionId = prev?.sessionId ?? null;
      const nextSessionId = mainSessionId ?? null;
      if (prevSessionId === nextSessionId) return prev;
      return nextSessionId ? { sessionId: nextSessionId, age: now } : null;
    });

    setBottomZone((prev) => {
      const prevSessionId = prev?.sessionId ?? null;
      const nextSessionId = bottomSessionId ?? null;
      if (prevSessionId === nextSessionId) return prev;
      return nextSessionId ? { sessionId: nextSessionId, age: now + 1 } : null;
    });

    setSplitRatio((prev) =>
      Math.abs(prev - clampedSplitRatio) < 0.0001 ? prev : clampedSplitRatio,
    );
  }, [restoreRequest?.requestId, sessions, onShowOverview]);

  // ---- Close zones ----

  const removeSessionFromZones = useCallback(
    (sessionId: string) => {
      let changed = false;

      if (mainZoneRef.current?.sessionId === sessionId) {
        mainZoneRef.current = null;
        setMainZone(null);
        changed = true;
      }
      if (bottomZoneRef.current?.sessionId === sessionId) {
        bottomZoneRef.current = null;
        setBottomZone(null);
        changed = true;
      }
      if (!changed) return;

      cache.evict(sessionId);
      realtime.unsubscribeSession(sessionId);

      if (!mainZoneRef.current && !bottomZoneRef.current) {
        onShowOverview();
      } else {
        onStartPolling();
      }
    },
    [cache, onShowOverview, onStartPolling],
  );

  const closeZone = useCallback(
    (zone: "main" | "bottom") => {
      // Read refs for fresh state — avoids stale closures when rapidly closing
      // both panes before React re-renders.
      const currentMain = mainZoneRef.current;
      const currentBottom = bottomZoneRef.current;

      const closingSessionId =
        zone === "main" ? currentMain?.sessionId : currentBottom?.sessionId;
      if (closingSessionId) {
        realtime.unsubscribeSession(closingSessionId);
      }

      // Update ref synchronously so the next closeZone call sees fresh state.
      if (zone === "main") {
        mainZoneRef.current = null;
        setMainZone(null);
      } else {
        bottomZoneRef.current = null;
        setBottomZone(null);
      }

      // If no zones remain, go to overview
      const otherZone = zone === "main" ? bottomZoneRef.current : mainZoneRef.current;
      if (!otherZone) {
        onShowOverview();
      } else {
        onStartPolling();
      }
    },
    [onShowOverview, onStartPolling],
  );

  const closeAllZones = useCallback(() => {
    if (mainZoneRef.current?.sessionId) {
      realtime.unsubscribeSession(mainZoneRef.current.sessionId);
    }
    if (bottomZoneRef.current?.sessionId) {
      realtime.unsubscribeSession(bottomZoneRef.current.sessionId);
    }
    mainZoneRef.current = null;
    bottomZoneRef.current = null;
    setMainZone(null);
    setBottomZone(null);
  }, []);

  // ---- Handle terminal caching from workspace ----

  const handleCache = useCallback(
    (cached: CachedTerminal) => {
      cache.put(cached);
    },
    [cache],
  );

  // ---- Handle session exit ----

  const handleSessionExit = useCallback(
    (sessionId: string) => {
      removeSessionFromZones(sessionId);
    },
    [removeSessionFromZones],
  );

  useEffect(() => {
    const liveIds = new Set(sessions.map((session) => session.session_id));
    const missing = [
      mainZoneRef.current?.sessionId,
      bottomZoneRef.current?.sessionId,
    ].filter((sessionId): sessionId is string => !!sessionId && !liveIds.has(sessionId));

    if (missing.length === 0) return;

    for (const sessionId of new Set(missing)) {
      removeSessionFromZones(sessionId);
    }
  }, [sessions, removeSessionFromZones]);

  // ---- Divider drag ----

  const handleDividerMouseDown = useCallback(
    (e: MouseEvent) => {
      e.preventDefault();
      const wrapper = wrapperRef.current;
      if (!wrapper) return;

      const onMouseMove = (me: MouseEvent) => {
        const rect = wrapper.getBoundingClientRect();
        const relX = me.clientX - rect.left;
        setSplitRatio(Math.max(0.2, Math.min(0.8, relX / rect.width)));
      };

      const onMouseUp = () => {
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };

      document.body.style.cursor = "ew-resize";
      document.body.style.userSelect = "none";
      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [],
  );

  // ---- Manage polling based on zone layout ----

  useEffect(() => {
    const dualZone = mainZone && bottomZone;
    const singleZone = (mainZone || bottomZone) && !dualZone;
    zoneLayout.value = dualZone ? "dual" : "single";
    if (dualZone) {
      onStopPolling();
    } else if (singleZone) {
      onStartPolling();
    }

    if (mainZone || bottomZone) {
      onLayoutChange({
        view: "terminal",
        mainSessionId: mainZone?.sessionId ?? null,
        bottomSessionId: bottomZone?.sessionId ?? null,
        splitRatio,
      });
    } else {
      onLayoutChange({
        view: "overview",
        mainSessionId: null,
        bottomSessionId: null,
        splitRatio,
      });
    }
  }, [
    mainZone,
    bottomZone,
    splitRatio,
    onStartPolling,
    onStopPolling,
    onLayoutChange,
  ]);

  // ---- Lookup session data ----

  const mainSession = mainZone
    ? sessions.find((s) => s.session_id === mainZone.sessionId)
    : null;
  const bottomSession = bottomZone
    ? sessions.find((s) => s.session_id === bottomZone.sessionId)
    : null;

  const dualZone = !!mainZone && !!bottomZone;
  const dividerWidth = 4;

  return (
    <div
      ref={wrapperRef}
      style={{
        display: "flex",
        flexDirection: "row",
        width: "100%",
        height: "100%",
        overflow: "hidden",
      }}
    >
      {/* Main zone */}
      {mainSession && mainZone && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            width: dualZone
              ? `calc(${splitRatio * 100}% - ${dividerWidth / 2}px)`
              : "100%",
            height: "100%",
            overflow: "hidden",
            minWidth: "120px",
          }}
        >
          <TerminalWorkspace
            key={mainZone.sessionId}
            session={mainSession}
            cached={cache.get(mainZone.sessionId)}
            observer={observer}
            isBenched={benchedIds?.has(mainZone.sessionId) ?? false}
            onBenchToggle={onBenchToggle}
            onCache={handleCache}
            onSessionExit={handleSessionExit}
            onClose={() => closeZone("main")}
          />
        </div>
      )}

      {/* Divider */}
      {dualZone && (
        <div
          ref={dividerRef}
          id="zone-divider"
          style={{
            width: dividerWidth + "px",
            background: "#0d1117",
            cursor: "ew-resize",
            flexShrink: 0,
            zIndex: 10,
          }}
          onMouseDown={handleDividerMouseDown}
        />
      )}

      {/* Bottom zone */}
      {bottomSession && bottomZone && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            width: dualZone
              ? `calc(${(1 - splitRatio) * 100}% - ${dividerWidth / 2}px)`
              : "100%",
            height: "100%",
            overflow: "hidden",
            minWidth: "120px",
          }}
        >
          <TerminalWorkspace
            key={bottomZone.sessionId}
            session={bottomSession}
            cached={cache.get(bottomZone.sessionId)}
            observer={observer}
            isBenched={benchedIds?.has(bottomZone.sessionId) ?? false}
            onBenchToggle={onBenchToggle}
            onCache={handleCache}
            onSessionExit={handleSessionExit}
            onClose={() => closeZone("bottom")}
          />
        </div>
      )}
    </div>
  );
};
