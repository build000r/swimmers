import { useEffect, useRef, useState, useCallback } from "preact/hooks";
import type { SessionSummary } from "@/types";
import { TerminalWorkspace } from "@/components/TerminalWorkspace";
import { useTerminalCache } from "@/hooks/useTerminalCache";
import { useSwipeBack } from "@/hooks/useGestures";
import { terminalCacheTtlMs } from "@/app";
import type { CachedTerminal } from "@/hooks/useTerminalCache";
import type { ReplayTruncatedPayload, SessionOverloadedPayload } from "@/types";
import { realtime } from "@/app";

// ---- Helpers ----

function isDesktop(): boolean {
  return window.innerWidth > 768;
}

interface ZoneState {
  sessionId: string;
  age: number; // timestamp for "replace oldest"
}

interface ZoneManagerProps {
  sessions: SessionSummary[];
  activeSessionId: string | null;
  preferZone: "main" | "bottom" | null;
  observer?: boolean;
  onShowOverview: () => void;
  onStartPolling: () => void;
  onStopPolling: () => void;
}

export function ZoneManager({
  sessions,
  activeSessionId,
  preferZone,
  observer = false,
  onShowOverview,
  onStartPolling,
  onStopPolling,
}: ZoneManagerProps) {
  const [mainZone, setMainZone] = useState<ZoneState | null>(null);
  const [bottomZone, setBottomZone] = useState<ZoneState | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.6);
  const [recoveryBanners, setRecoveryBanners] = useState<
    Record<string, string | null>
  >({});

  const dividerRef = useRef<HTMLDivElement>(null);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const cache = useTerminalCache(terminalCacheTtlMs.value);

  // ---- Swipe back gesture ----
  useSwipeBack(wrapperRef, () => {
    closeAllZones();
    onShowOverview();
  });

  // ---- Realtime event wiring for recovery banners ----

  useEffect(() => {
    realtime.on({
      onReplayTruncated(
        sessionId: string,
        payload: ReplayTruncatedPayload,
      ) {
        setRecoveryBanners((prev) => ({
          ...prev,
          [sessionId]: `Replay gap: seq ${payload.requested_resume_from_seq}-${payload.replay_window_start_seq} lost. Refresh recommended.`,
        }));
      },
      onSessionOverloaded(
        sessionId: string,
        payload: SessionOverloadedPayload,
      ) {
        setRecoveryBanners((prev) => ({
          ...prev,
          [sessionId]: `Session overloaded (queue: ${payload.queue_depth}). Retry in ${payload.retry_after_ms}ms.`,
        }));
      },
    });
  }, []);

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

    // Cache current occupant if the target is occupied
    // (The TerminalWorkspace component handles caching on unmount via onCache)

    // On mobile, clear bottom zone
    if (!isDesktop() && bottomZone) {
      setBottomZone(null);
    }

    if (target === "main") {
      setMainZone({ sessionId: activeSessionId, age: Date.now() });
    } else {
      setBottomZone({ sessionId: activeSessionId, age: Date.now() });
    }
  }, [activeSessionId, preferZone]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---- Close zones ----

  const closeZone = useCallback(
    (zone: "main" | "bottom") => {
      if (zone === "main") setMainZone(null);
      else setBottomZone(null);

      // If no zones remain, go to overview
      const otherZone = zone === "main" ? bottomZone : mainZone;
      if (!otherZone) {
        onShowOverview();
      } else {
        onStartPolling();
      }
    },
    [mainZone, bottomZone, onShowOverview, onStartPolling],
  );

  const closeAllZones = useCallback(() => {
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
      cache.evict(sessionId);
      if (mainZone?.sessionId === sessionId) setMainZone(null);
      if (bottomZone?.sessionId === sessionId) setBottomZone(null);
      // Check if any zones remain
      const mainAfter =
        mainZone?.sessionId === sessionId ? null : mainZone;
      const bottomAfter =
        bottomZone?.sessionId === sessionId ? null : bottomZone;
      if (!mainAfter && !bottomAfter) onShowOverview();
    },
    [mainZone, bottomZone, cache, onShowOverview],
  );

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
    if (dualZone) {
      onStopPolling();
    } else if (singleZone) {
      onStartPolling();
    }
  }, [mainZone, bottomZone, onStartPolling, onStopPolling]);

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
            onCache={handleCache}
            onSessionExit={handleSessionExit}
            onClose={() => closeZone("main")}
            recoveryBanner={recoveryBanners[mainZone.sessionId] ?? null}
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
            onCache={handleCache}
            onSessionExit={handleSessionExit}
            onClose={() => closeZone("bottom")}
            recoveryBanner={recoveryBanners[bottomZone.sessionId] ?? null}
          />
        </div>
      )}
    </div>
  );
}
