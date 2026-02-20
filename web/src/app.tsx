import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { signal, batch } from "@preact/signals";
import type { SessionSummary, TransportHealth, BootstrapResponse } from "@/types";
import type {
  SessionStatePayload,
  ThoughtUpdatePayload,
  SessionCreatedPayload,
  SessionDeletedPayload,
} from "@/types";
import {
  bootstrap as apiFetch,
  fetchPaneTail,
  fetchSessions,
} from "@/services/api";
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
export const idlePreviews = signal<Record<string, string>>({});

// Shared realtime service singleton
export const realtime = new RealtimeService();

const IDLE_PREVIEW_DELAY_MS = 20_000;
const IDLE_PREVIEW_REFRESH_MS = 12_000;
const IDLE_PREVIEW_SCAN_MS = 2_000;
const IDLE_PREVIEW_MAX_CHARS = 300;
const TERMINAL_OUTPUT_DECODER = new TextDecoder();

function parseIsoMs(value: string): number | null {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function pickPreferredSession(
  current: SessionSummary,
  candidate: SessionSummary,
): SessionSummary {
  if (current.is_stale !== candidate.is_stale) {
    return current.is_stale ? candidate : current;
  }
  const currentMs = parseIsoMs(current.last_activity_at) ?? 0;
  const candidateMs = parseIsoMs(candidate.last_activity_at) ?? 0;
  return candidateMs >= currentMs ? candidate : current;
}

function dedupeSessionsById(items: SessionSummary[]): SessionSummary[] {
  if (items.length < 2) return items;
  const byId = new Map<string, SessionSummary>();
  for (const session of items) {
    const existing = byId.get(session.session_id);
    if (!existing) {
      byId.set(session.session_id, session);
      continue;
    }
    byId.set(session.session_id, pickPreferredSession(existing, session));
  }
  return Array.from(byId.values());
}

function stripTerminalEscapes(raw: string): string {
  const noOsc = raw.replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "");
  const noCsi = noOsc.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "");
  const noEsc = noCsi.replace(/\x1b[@-_]/g, "");
  return noEsc.replace(/[\x00-\x08\x0B-\x1F\x7F]/g, "");
}

function buildIdlePreviewText(screenText: string): string {
  const lines = stripTerminalEscapes(screenText)
    .split(/\r?\n/)
    .map((line) =>
      line
        .replace(/[│┃┆┊╭╮╯╰─━┄┈]+/g, " ")
        .replace(/\s+/g, " ")
        .trim(),
    )
    .filter((line) => line.length > 0)
    .filter((line) => !/^[>$❯]\s*$/.test(line));

  const cleaned = lines.slice(-10).join(" ").trim();
  if (!cleaned) return "";
  if (cleaned.length <= IDLE_PREVIEW_MAX_CHARS) return cleaned;
  const keep = IDLE_PREVIEW_MAX_CHARS - 1;
  return `…${cleaned.slice(cleaned.length - keep)}`;
}

function hasMeaningfulTerminalOutput(data: Uint8Array): boolean {
  if (data.byteLength === 0) return false;
  const raw = TERMINAL_OUTPUT_DECODER.decode(data);
  const visible = stripTerminalEscapes(raw);
  return /[^\s]/.test(visible);
}

function extractCwdFromSessionTitle(title: string, fallback: string): string {
  const trimmed = title.trim();
  if (!trimmed) return fallback;
  if (trimmed.startsWith("/")) return trimmed;
  if (trimmed.startsWith("~")) return trimmed;

  const spaced = trimmed.lastIndexOf(": /");
  if (spaced !== -1) {
    return trimmed.slice(spaced + 2).trim();
  }

  const compact = trimmed.lastIndexOf(":/");
  if (compact !== -1) {
    return trimmed.slice(compact + 1).trim();
  }

  return fallback;
}

/** Smart poll merge: preserves object references for unchanged sessions and
 *  protects exiting sessions from being clobbered by stale server data. */
export function mergePollSessions(
  prev: SessionSummary[],
  next: SessionSummary[],
  exitingIds: Set<string>,
): SessionSummary[] | null {
  // Keep exiting sessions untouched — they're managed by the realtime handler
  const localExiting = prev.filter((s) => exitingIds.has(s.session_id));
  const nextActive = next.filter((s) => !exitingIds.has(s.session_id));
  const prevActive = prev.filter((s) => !exitingIds.has(s.session_id));

  const prevById = new Map(prevActive.map((s) => [s.session_id, s]));

  // Check if anything actually changed (ignore thought/token_count/context_limit
  // — REST doesn't populate these; they come from WebSocket thought_update)
  if (
    prevActive.length === nextActive.length &&
    nextActive.every((s, i) => {
      const old = prevActive[i];
      return (
        old &&
        old.session_id === s.session_id &&
        old.state === s.state &&
        old.current_command === s.current_command &&
        old.cwd === s.cwd &&
        old.tool === s.tool
      );
    })
  ) {
    return null; // No changes — skip signal update entirely
  }

  // Merge: preserve object references for unchanged sessions to avoid re-renders
  const merged = nextActive.map((s) => {
    const old = prevById.get(s.session_id);
    if (!old) return s;
    if (
      old.state === s.state &&
      old.current_command === s.current_command &&
      old.cwd === s.cwd &&
      old.tool === s.tool
    ) {
      return old; // Preserve reference — avoids re-rendering this thronglet
    }
    // Preserve WebSocket-enriched fields that REST doesn't populate
    return {
      ...s,
      last_activity_at: old.last_activity_at,
      token_count: s.token_count ?? old.token_count,
      context_limit: s.context_limit ?? old.context_limit,
      thought: s.thought ?? old.thought,
    };
  });

  return localExiting.length > 0 ? [...merged, ...localExiting] : merged;
}

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
  const outputActivityUpdateAtRef = useRef<Map<string, number>>(new Map());
  const idlePreviewFetchInFlightRef = useRef<Set<string>>(new Set());
  const idlePreviewLastFetchAtRef = useRef<Map<string, number>>(new Map());
  const exitingSessionIdsRef = useRef<Set<string>>(new Set());
  const { isObserver } = useObserverMode(authMode);

  // ---- Session helpers ----

  const updateSession = useCallback(
    (sessionId: string, updater: (s: SessionSummary) => SessionSummary) => {
      const prev = sessions.value;
      let changed = false;
      const next = prev.map((s) => {
        if (s.session_id !== sessionId) return s;
        const updated = updater(s);
        if (updated !== s) changed = true;
        return updated;
      });
      if (changed) {
        sessions.value = next;
      }
    },
    [],
  );

  const clearIdlePreview = useCallback((sessionId: string) => {
    const existing = idlePreviews.value;
    if (existing[sessionId] !== undefined) {
      const next = { ...existing };
      delete next[sessionId];
      idlePreviews.value = next;
    }
    idlePreviewLastFetchAtRef.current.delete(sessionId);
    idlePreviewFetchInFlightRef.current.delete(sessionId);
  }, []);

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
        updateSession(sessionId, (s) =>
          s.state === payload.state &&
          s.current_command === payload.current_command &&
          s.transport_health === payload.transport_health &&
          s.last_activity_at === payload.at
            ? s
            : {
                ...s,
                state: payload.state,
                current_command: payload.current_command,
                transport_health: payload.transport_health,
                last_activity_at: payload.at,
              },
        );
        if (payload.state !== "idle") {
          clearIdlePreview(sessionId);
        }
        // After exit, let the thronglet walk off screen then remove it
        if (payload.state === "exited") {
          outputActivityUpdateAtRef.current.delete(sessionId);
          exitingSessionIdsRef.current.add(sessionId);
          setTimeout(() => {
            exitingSessionIdsRef.current.delete(sessionId);
            sessions.value = sessions.value.filter(
              (s) => s.session_id !== sessionId,
            );
          }, 2500);
        }
      },

      onSessionTitle(sessionId: string, title: string) {
        updateSession(sessionId, (s) => {
          const cwd = extractCwdFromSessionTitle(title, s.cwd);
          return cwd === s.cwd ? s : { ...s, cwd };
        });
      },

      onThoughtUpdate(sessionId: string, payload: ThoughtUpdatePayload) {
        updateSession(sessionId, (s) =>
          s.thought === payload.thought &&
          s.token_count === payload.token_count &&
          s.context_limit === payload.context_limit
            ? s
            : {
                ...s,
                thought: payload.thought,
                token_count: payload.token_count,
                context_limit: payload.context_limit,
              },
        );
      },

      onSessionCreated(payload: SessionCreatedPayload) {
        const existingIndex = sessions.value.findIndex(
          (s) => s.session_id === payload.session.session_id,
        );
        if (existingIndex === -1) {
          sessions.value = [...sessions.value, payload.session];
          return;
        }
        const next = [...sessions.value];
        next[existingIndex] = payload.session;
        sessions.value = next;
      },

      onSessionDeleted(sessionId: string, _payload: SessionDeletedPayload) {
        clearIdlePreview(sessionId);
        outputActivityUpdateAtRef.current.delete(sessionId);
        exitingSessionIdsRef.current.delete(sessionId);
        sessions.value = sessions.value.filter(
          (s) => s.session_id !== sessionId,
        );
      },

      onTerminalOutput(frame) {
        if (!hasMeaningfulTerminalOutput(frame.data)) {
          return;
        }

        clearIdlePreview(frame.sessionId);

        const now = Date.now();
        const prev = outputActivityUpdateAtRef.current.get(frame.sessionId) ?? 0;
        if (now - prev < 1000) return;
        outputActivityUpdateAtRef.current.set(frame.sessionId, now);

        const updatedAt = new Date(now).toISOString();
        updateSession(frame.sessionId, (s) =>
          s.last_activity_at === updatedAt
            ? s
            : {
                ...s,
                last_activity_at: updatedAt,
              },
        );
      },

      onControlError(payload) {
        console.error("[realtime] control error:", payload.code, payload.message);
      },
    });
  }, [updateSession, clearIdlePreview]);

  // ---- Bootstrap ----

  useEffect(() => {
    let cancelled = false;

    async function init() {
      try {
        const data = await apiFetch();
        if (cancelled) return;
        bootstrapDataRef.current = data;
        const initialSessions = dedupeSessionsById(data.sessions);

        batch(() => {
          sessions.value = initialSessions;
          terminalCacheTtlMs.value = data.terminal_cache_ttl_ms;
          idlePreviews.value = {};
        });
        idlePreviewFetchInFlightRef.current.clear();
        idlePreviewLastFetchAtRef.current.clear();
        outputActivityUpdateAtRef.current.clear();

        setAuthMode(data.auth_mode);
        workspaceHistoryModeRef.current = data.workspace_history_mode;

        const initialLayout = applyWorkspaceLayout(
          parseWorkspaceLayoutFromUrl(new URL(window.location.href)),
          initialSessions,
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
        const resp = await fetchSessions();
        const polledSessions = dedupeSessionsById(resp.sessions);
        const merged = mergePollSessions(
          sessions.value,
          polledSessions,
          exitingSessionIdsRef.current,
        );
        if (merged) sessions.value = merged;
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

  // Keep overview session states fresh even when realtime is healthy.
  useEffect(() => {
    if (!bootstrapDone) return;
    if (transportHealth.value !== "healthy") return;
    const isOverviewVisible =
      currentView.value === "overview" ||
      (currentView.value === "terminal" && zoneLayout.value === "single");
    if (!isOverviewVisible) return;

    let cancelled = false;
    let running = false;
    const ms = Math.max(bootstrapDataRef.current?.poll_fallback_ms ?? 2000, 2000);

    const tick = async () => {
      if (running || cancelled) return;
      running = true;
      try {
        const resp = await fetchSessions();
        if (cancelled) return;
        const polledSessions = dedupeSessionsById(resp.sessions);
        const merged = mergePollSessions(
          sessions.value,
          polledSessions,
          exitingSessionIdsRef.current,
        );
        if (merged) sessions.value = merged;
      } catch {
        // Keep stale data on transient network errors.
      } finally {
        running = false;
      }
    };

    const interval = setInterval(() => {
      void tick();
    }, ms);
    void tick();

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [
    bootstrapDone,
    currentView.value,
    zoneLayout.value,
    transportHealth.value,
  ]);

  // ---- Idle preview snapshots (overview bubble override) ----

  useEffect(() => {
    if (!bootstrapDone) return;

    let cancelled = false;
    let running = false;

    const tick = async () => {
      if (running || cancelled) return;
      running = true;

      try {
        const now = Date.now();
        const isOverviewVisible =
          currentView.value === "overview" ||
          (currentView.value === "terminal" && zoneLayout.value === "single");

        if (!isOverviewVisible) {
          running = false;
          return;
        }

        const sessionById = new Map(
          sessions.value.map((session) => [session.session_id, session]),
        );

        // Drop stale previews that no longer qualify.
        let nextPreviews = idlePreviews.value;
        let previewsChanged = false;
        for (const sessionId of Object.keys(nextPreviews)) {
          const session = sessionById.get(sessionId);
          const lastActivityMs = session
            ? parseIsoMs(session.last_activity_at)
            : null;
          const stillEligible =
            !!session &&
            session.state === "idle" &&
            lastActivityMs !== null &&
            now - lastActivityMs >= IDLE_PREVIEW_DELAY_MS;
          if (!stillEligible) {
            if (!previewsChanged) {
              nextPreviews = { ...nextPreviews };
              previewsChanged = true;
            }
            delete nextPreviews[sessionId];
            idlePreviewLastFetchAtRef.current.delete(sessionId);
          }
        }
        if (previewsChanged) {
          idlePreviews.value = nextPreviews;
        }

        const inFlight = idlePreviewFetchInFlightRef.current;
        const lastFetchAt = idlePreviewLastFetchAtRef.current;
        let launches = 0;
        const maxLaunchesPerTick = 2;

        for (const session of sessions.value) {
          if (launches >= maxLaunchesPerTick) break;
          if (session.state !== "idle") continue;

          const lastActivityMs = parseIsoMs(session.last_activity_at);
          if (lastActivityMs === null) continue;
          if (now - lastActivityMs < IDLE_PREVIEW_DELAY_MS) continue;

          const sessionId = session.session_id;
          if (inFlight.has(sessionId)) continue;

          const lastFetchMs = lastFetchAt.get(sessionId) ?? 0;
          const hasCachedPreview = idlePreviews.value[sessionId] !== undefined;
          if (
            hasCachedPreview &&
            now - lastFetchMs < IDLE_PREVIEW_REFRESH_MS
          ) {
            continue;
          }

          launches += 1;
          inFlight.add(sessionId);
          lastFetchAt.set(sessionId, now);

          void fetchPaneTail(sessionId)
            .then((paneTail) => {
              if (cancelled) return;
              const current = sessions.value.find(
                (s) => s.session_id === sessionId,
              );
              if (!current || current.state !== "idle") return;

              const currentLastActivityMs = parseIsoMs(current.last_activity_at);
              if (currentLastActivityMs === null) return;
              if (Date.now() - currentLastActivityMs < IDLE_PREVIEW_DELAY_MS) {
                return;
              }

              const preview = buildIdlePreviewText(paneTail.text);
              const prev = idlePreviews.value[sessionId];
              if (!preview && prev !== undefined) {
                const next = { ...idlePreviews.value };
                delete next[sessionId];
                idlePreviews.value = next;
                return;
              }
              if (preview && prev !== preview) {
                idlePreviews.value = {
                  ...idlePreviews.value,
                  [sessionId]: preview,
                };
              }
            })
            .catch(() => {
              // Keep existing preview on transient snapshot failure.
            })
            .finally(() => {
              inFlight.delete(sessionId);
            });
        }
      } finally {
        running = false;
      }
    };

    const interval = setInterval(() => {
      void tick();
    }, IDLE_PREVIEW_SCAN_MS);
    void tick();

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [bootstrapDone, currentView.value, zoneLayout.value]);

  // ---- Navigation ----

  const openTerminal = useCallback(
    (sessionId: string, preferZone?: "main" | "bottom") => {
      if (!isObserver) {
        const session = sessions.value.find((s) => s.session_id === sessionId);
        if (session?.state === "attention") {
          realtime.sendDismissAttention(sessionId);
        }
      }
      activeSessionId.value = sessionId;
      // Default tap behavior should replace the main pane.
      // Secondary pane assignment is explicit via drag-left (preferZone="bottom").
      activeZonePreference.value = preferZone ?? "main";
      currentView.value = "terminal";
      stopPolling();
    },
    [isObserver, stopPolling],
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
  const splitMode = isTerminal && zoneLayout.value === "single" && window.innerWidth > 768;

  // In split mode: terminal left 50%, field right 50%
  // In dual-zone: terminal full screen, field hidden
  const overviewTransform = isOverview
    ? "none"
    : splitMode
      ? "none"
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
          idlePreviews={idlePreviews.value}
          observer={isObserver}
          compact={splitMode}
          onTapSession={openTerminal}
          onDragToBottom={(id) => openTerminal(id, "bottom")}
          onCreateSession={async (cwd?: string) => {
            if (isObserver) return "";
            try {
              const { createSession } = await import("@/services/api");
              const resp = await createSession(undefined, cwd);
              const existingIndex = sessions.value.findIndex(
                (s) => s.session_id === resp.session.session_id,
              );
              if (existingIndex === -1) {
                sessions.value = [...sessions.value, resp.session];
              } else {
                const next = [...sessions.value];
                next[existingIndex] = resp.session;
                sessions.value = next;
              }
              return resp.session.session_id;
            } catch (err) {
              console.error("Failed to create session:", err);
              return "";
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
