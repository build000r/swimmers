import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { signal, batch } from "@preact/signals";
import type { SessionSummary, TransportHealth, BootstrapResponse } from "@/types";
import type {
  SessionStatePayload,
  ThoughtUpdatePayload,
  SessionCreatedPayload,
  SessionDeletedPayload,
} from "@/types";
import { bootstrap as apiFetch } from "@/services/api";
import { RealtimeService } from "@/services/realtime";
import type { WorkspaceLayoutState } from "@/services/workspace-history";
import {
  applyWorkspaceLayoutToUrl,
  normalizeWorkspaceLayout,
  parseWorkspaceLayoutFromUrl,
} from "@/services/workspace-history";
import { OverviewField } from "@/components/OverviewField";
import { ZoneManager } from "@/components/ZoneManager";
import { useObserverMode } from "@/hooks/useObserverMode";

// ---- Global signals ----
export const sessions = signal<SessionSummary[]>([]);
export const transportHealth = signal<TransportHealth>("disconnected");
export const currentView = signal<"overview" | "terminal">("overview");
export const activeSessionId = signal<string | null>(null);
export const activeZonePreference = signal<"main" | "bottom" | null>(null);
export const terminalCacheTtlMs = signal<number>(300_000);
export const zoneLayout = signal<"single" | "dual">("single");

// Shared realtime service singleton
export const realtime = new RealtimeService();

interface RestoreLayoutRequest extends WorkspaceLayoutState {
  requestId: number;
}

export function App() {
  const [bootstrapDone, setBootstrapDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authMode, setAuthMode] = useState<string | undefined>(undefined);
  const [restoreRequest, setRestoreRequest] =
    useState<RestoreLayoutRequest | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const bootstrapDataRef = useRef<BootstrapResponse | null>(null);
  const workspaceHistoryModeRef = useRef("url_state_v1");
  const restoreRequestIdRef = useRef(0);
  const lastLayoutRef = useRef<WorkspaceLayoutState>({
    view: "overview",
    mainSessionId: null,
    bottomSessionId: null,
    splitRatio: 0.6,
  });
  const { isObserver } = useObserverMode(authMode);

  // ---- Session helpers ----

  const updateSession = useCallback(
    (sessionId: string, updater: (s: SessionSummary) => SessionSummary) => {
      sessions.value = sessions.value.map((s) =>
        s.session_id === sessionId ? updater(s) : s,
      );
    },
    [],
  );

  const writeWorkspaceLayout = useCallback(
    (layout: WorkspaceLayoutState, mode: "push" | "replace" = "push") => {
      if (workspaceHistoryModeRef.current !== "url_state_v1") return;

      const nextUrl = applyWorkspaceLayoutToUrl(
        new URL(window.location.href),
        layout,
      );

      if (
        nextUrl.pathname === window.location.pathname &&
        nextUrl.search === window.location.search &&
        nextUrl.hash === window.location.hash
      ) {
        lastLayoutRef.current = layout;
        return;
      }

      const nextHref = `${nextUrl.pathname}${nextUrl.search}${nextUrl.hash}`;
      if (mode === "replace") {
        window.history.replaceState({ workspace: layout }, "", nextHref);
      } else {
        window.history.pushState({ workspace: layout }, "", nextHref);
      }

      lastLayoutRef.current = layout;
    },
    [],
  );

  const applyWorkspaceLayout = useCallback(
    (candidate: WorkspaceLayoutState, availableSessions: SessionSummary[]) => {
      const normalized = normalizeWorkspaceLayout(
        candidate,
        new Set(availableSessions.map((s) => s.session_id)),
      );

      batch(() => {
        currentView.value = normalized.view;
        activeSessionId.value = null;
        activeZonePreference.value = null;
      });

      if (normalized.view === "terminal") {
        restoreRequestIdRef.current += 1;
        setRestoreRequest({
          ...normalized,
          requestId: restoreRequestIdRef.current,
        });
      } else {
        setRestoreRequest(null);
      }

      lastLayoutRef.current = normalized;
      return normalized;
    },
    [],
  );

  // ---- Realtime event wiring ----

  useEffect(() => {
    realtime.on({
      onHealthChange(health) {
        transportHealth.value = health;
      },

      onSessionState(sessionId: string, payload: SessionStatePayload) {
        updateSession(sessionId, (s) => ({
          ...s,
          state: payload.state,
          current_command: payload.current_command,
          transport_health: payload.transport_health,
        }));
        // After exit, let the thronglet walk off screen then remove it
        if (payload.state === "exited") {
          setTimeout(() => {
            sessions.value = sessions.value.filter(
              (s) => s.session_id !== sessionId,
            );
          }, 2500);
        }
      },

      onSessionTitle(sessionId: string, title: string) {
        updateSession(sessionId, (s) => {
          // Extract cwd from title. Common formats:
          // "user@host: /path", "user@host:/path", "/path/to/dir"
          let cwd = s.cwd;
          const colonSlash = title.indexOf(": /");
          if (colonSlash !== -1) {
            cwd = title.slice(colonSlash + 2).trim();
          } else if (title.startsWith("/")) {
            cwd = title.trim();
          }
          return { ...s, cwd };
        });
      },

      onThoughtUpdate(sessionId: string, payload: ThoughtUpdatePayload) {
        updateSession(sessionId, (s) => ({
          ...s,
          thought: payload.thought,
          token_count: payload.token_count,
          context_limit: payload.context_limit,
        }));
      },

      onSessionCreated(payload: SessionCreatedPayload) {
        // Guard against duplicates — the REST create response may have
        // already added this session before the WebSocket event arrives.
        if (!sessions.value.some(s => s.session_id === payload.session.session_id)) {
          sessions.value = [...sessions.value, payload.session];
        }
      },

      onSessionDeleted(sessionId: string, _payload: SessionDeletedPayload) {
        sessions.value = sessions.value.filter(
          (s) => s.session_id !== sessionId,
        );
      },

      onControlError(payload) {
        console.error("[realtime] control error:", payload.code, payload.message);
      },
    });
  }, [updateSession]);

  // ---- Bootstrap ----

  useEffect(() => {
    let cancelled = false;

    async function init() {
      try {
        const data = await apiFetch();
        if (cancelled) return;
        bootstrapDataRef.current = data;

        batch(() => {
          sessions.value = data.sessions;
          terminalCacheTtlMs.value = data.terminal_cache_ttl_ms;
        });

        setAuthMode(data.auth_mode);
        workspaceHistoryModeRef.current = data.workspace_history_mode;

        const initialLayout = applyWorkspaceLayout(
          parseWorkspaceLayoutFromUrl(new URL(window.location.href)),
          data.sessions,
        );
        writeWorkspaceLayout(initialLayout, "replace");

        // Connect realtime WebSocket
        // Derive ws URL from the page origin if the bootstrap URL is absolute
        const wsProto =
          window.location.protocol === "https:" ? "wss:" : "ws:";
        const wsUrl = `${wsProto}//${window.location.host}/v1/realtime`;
        realtime.connect(wsUrl);

        setBootstrapDone(true);
      } catch (err) {
        if (!cancelled) {
          setError(
            err instanceof Error ? err.message : "Failed to connect",
          );
        }
      }
    }

    init();
    return () => {
      cancelled = true;
    };
  }, [applyWorkspaceLayout, writeWorkspaceLayout]);

  useEffect(() => {
    if (!bootstrapDone || workspaceHistoryModeRef.current !== "url_state_v1") {
      return;
    }

    const onPopState = () => {
      applyWorkspaceLayout(
        parseWorkspaceLayoutFromUrl(new URL(window.location.href)),
        sessions.value,
      );
    };

    window.addEventListener("popstate", onPopState);
    return () => {
      window.removeEventListener("popstate", onPopState);
    };
  }, [bootstrapDone, applyWorkspaceLayout]);

  // ---- Polling fallback (when on overview) ----

  const startPolling = useCallback(() => {
    if (currentView.value !== "overview") return;
    if (
      transportHealth.value !== "degraded" &&
      transportHealth.value !== "disconnected"
    ) {
      return;
    }
    if (pollRef.current) return;
    const ms = bootstrapDataRef.current?.poll_fallback_ms ?? 2000;
    pollRef.current = setInterval(async () => {
      try {
        const { fetchSessions } = await import("@/services/api");
        const resp = await fetchSessions();
        sessions.value = resp.sessions;
      } catch {
        // Keep stale data on network error
      }
    }, ms);
  }, []);

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  // Poll only in degraded/disconnected transport while on overview.
  useEffect(() => {
    const shouldPoll =
      bootstrapDone &&
      currentView.value === "overview" &&
      (transportHealth.value === "degraded" ||
        transportHealth.value === "disconnected");

    if (shouldPoll) {
      startPolling();
    } else {
      stopPolling();
    }
    return stopPolling;
  }, [
    bootstrapDone,
    startPolling,
    stopPolling,
    currentView.value,
    transportHealth.value,
  ]);

  // ---- Navigation ----

  const openTerminal = useCallback(
    (sessionId: string, preferZone?: "main" | "bottom") => {
      activeSessionId.value = sessionId;
      activeZonePreference.value = preferZone ?? null;
      currentView.value = "terminal";
      stopPolling();
    },
    [stopPolling],
  );

  const showOverview = useCallback(() => {
    batch(() => {
      currentView.value = "overview";
      activeSessionId.value = null;
      activeZonePreference.value = null;
    });
    setRestoreRequest(null);
    writeWorkspaceLayout(
      {
        view: "overview",
        mainSessionId: null,
        bottomSessionId: null,
        splitRatio: lastLayoutRef.current.splitRatio,
      },
      "push",
    );
  }, [writeWorkspaceLayout]);

  const handleLayoutChange = useCallback(
    (layout: WorkspaceLayoutState) => {
      const previous = lastLayoutRef.current;
      const mode =
        previous.view === layout.view &&
        previous.mainSessionId === layout.mainSessionId &&
        previous.bottomSessionId === layout.bottomSessionId
          ? "replace"
          : "push";
      writeWorkspaceLayout(layout, mode);
    },
    [writeWorkspaceLayout],
  );

  // ---- Render ----

  if (error) {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          height: "100vh",
          color: "#e74c3c",
          fontFamily: "monospace",
          padding: "2rem",
          textAlign: "center",
        }}
      >
        <div>
          <div style={{ fontSize: "1.5rem", marginBottom: "0.5rem" }}>
            Connection failed
          </div>
          <div style={{ opacity: 0.7 }}>{error}</div>
        </div>
      </div>
    );
  }

  if (!bootstrapDone) {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          height: "100vh",
          color: "#888",
          fontFamily: "monospace",
        }}
      >
        Connecting...
      </div>
    );
  }

  const isOverview = currentView.value === "overview";
  const isTerminal = currentView.value === "terminal";
  const splitMode = isTerminal && zoneLayout.value === "single";

  // In split mode: terminal left 50%, field right 50%
  // In dual-zone: terminal full screen, field hidden
  const overviewTransform = isOverview
    ? "translateX(0)"
    : splitMode
      ? "translateX(0)"
      : "translateX(-100%)";

  return (
    <div style={{ position: "absolute", inset: 0, overflow: "hidden" }}>
      {/* Transport health banner */}
      {transportHealth.value !== "healthy" &&
        transportHealth.value !== "disconnected" && (
          <div
            style={{
              position: "fixed",
              top: 0,
              left: 0,
              right: 0,
              zIndex: 1000,
              background:
                transportHealth.value === "degraded" ? "#F5A623" : "#E74C3C",
              color: "#fff",
              textAlign: "center",
              padding: "4px 0",
              fontSize: "12px",
              fontWeight: 600,
            }}
          >
            Transport {transportHealth.value}
          </div>
        )}

      {/* Overview field */}
      <div
        class={`view ${isOverview || splitMode ? "active" : ""}`}
        style={{
          position: "absolute",
          top: 0,
          left: splitMode ? "50%" : 0,
          right: 0,
          bottom: 0,
          transform: overviewTransform,
          transition: "transform 0.25s ease, left 0.25s ease",
        }}
      >
        <OverviewField
          sessions={sessions.value}
          observer={isObserver}
          onTapSession={openTerminal}
          onDragToBottom={(id) => openTerminal(id, "bottom")}
          onCreateSession={async () => {
            if (isObserver) return;
            try {
              const { createSession } = await import("@/services/api");
              const resp = await createSession();
              // Add only if the WebSocket session_created event hasn't already.
              if (!sessions.value.some(s => s.session_id === resp.session.session_id)) {
                sessions.value = [...sessions.value, resp.session];
              }
              openTerminal(resp.session.session_id);
            } catch (err) {
              console.error("Failed to create session:", err);
            }
          }}
        />
      </div>

      {/* Terminal workspace */}
      <div
        class={`view ${isTerminal ? "active" : ""}`}
        style={{
          position: "absolute",
          top: 0,
          left: 0,
          right: splitMode ? "50%" : 0,
          bottom: 0,
          transform: isTerminal ? "translateX(0)" : "translateX(100%)",
          transition: "transform 0.25s ease, right 0.25s ease",
          display: "flex",
          flexDirection: "row",
        }}
      >
        {isTerminal && (
          <ZoneManager
            sessions={sessions.value}
            activeSessionId={activeSessionId.value}
            preferZone={activeZonePreference.value}
            restoreRequest={restoreRequest}
            observer={isObserver}
            onShowOverview={showOverview}
            onStartPolling={startPolling}
            onStopPolling={stopPolling}
            onLayoutChange={handleLayoutChange}
          />
        )}
      </div>
    </div>
  );
}
