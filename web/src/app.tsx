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
import { OverviewField } from "@/components/OverviewField";
import { ZoneManager } from "@/components/ZoneManager";

// ---- Global signals ----
export const sessions = signal<SessionSummary[]>([]);
export const transportHealth = signal<TransportHealth>("disconnected");
export const currentView = signal<"overview" | "terminal">("overview");
export const activeSessionId = signal<string | null>(null);
export const activeZonePreference = signal<"main" | "bottom" | null>(null);
export const terminalCacheTtlMs = signal<number>(300_000);

// Shared realtime service singleton
export const realtime = new RealtimeService();

export function App() {
  const [bootstrapDone, setBootstrapDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const bootstrapDataRef = useRef<BootstrapResponse | null>(null);

  // ---- Session helpers ----

  const updateSession = useCallback(
    (sessionId: string, updater: (s: SessionSummary) => SessionSummary) => {
      sessions.value = sessions.value.map((s) =>
        s.session_id === sessionId ? updater(s) : s,
      );
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
        sessions.value = [...sessions.value, payload.session];
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
  }, []);

  // ---- Polling fallback (when on overview) ----

  const startPolling = useCallback(() => {
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

  // Start polling when on overview
  useEffect(() => {
    if (bootstrapDone && currentView.value === "overview") {
      startPolling();
    }
    return stopPolling;
  }, [bootstrapDone, startPolling, stopPolling]);

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
    currentView.value = "overview";
    activeSessionId.value = null;
    activeZonePreference.value = null;
    startPolling();
  }, [startPolling]);

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
        class={`view ${isOverview ? "active" : ""}`}
        style={{
          position: "absolute",
          inset: 0,
          transform: isOverview ? "translateX(0)" : "translateX(-100%)",
          transition: "transform 0.25s ease",
        }}
      >
        <OverviewField
          sessions={sessions.value}
          onTapSession={openTerminal}
          onDragToBottom={(id) => openTerminal(id, "bottom")}
          onCreateSession={async () => {
            try {
              const { createSession } = await import("@/services/api");
              const resp = await createSession();
              sessions.value = [...sessions.value, resp.session];
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
          inset: 0,
          transform: isTerminal ? "translateX(0)" : "translateX(100%)",
          transition: "transform 0.25s ease",
          display: "flex",
          flexDirection: "row",
        }}
      >
        {isTerminal && (
          <ZoneManager
            sessions={sessions.value}
            activeSessionId={activeSessionId.value}
            preferZone={activeZonePreference.value}
            onShowOverview={showOverview}
            onStartPolling={startPolling}
            onStopPolling={stopPolling}
          />
        )}
      </div>
    </div>
  );
}
