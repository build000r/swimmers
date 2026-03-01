import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { signal, batch } from "@preact/signals";
import type {
  SessionSummary,
  TransportHealth,
  BootstrapResponse,
  SpawnTool,
  SpritePack,
} from "@/types";
import type {
  SessionStatePayload,
  SessionSkillPayload,
  ThoughtUpdatePayload,
  SessionCreatedPayload,
  SessionDeletedPayload,
} from "@/types";
import {
  bootstrap as apiFetch,
  createSession as apiCreateSession,
  deleteSession as apiDeleteSession,
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
export const spritePacks = signal<Record<string, SpritePack>>({});

// Shared realtime service singleton
export const realtime = new RealtimeService();

const IDLE_PREVIEW_DELAY_MS = 20_000;
const IDLE_PREVIEW_REFRESH_MS = 12_000;
const IDLE_PREVIEW_SCAN_MS = 2_000;
const IDLE_PREVIEW_MAX_CHARS = 160;
const TERMINAL_OUTPUT_DECODER = new TextDecoder();

function parseIsoMs(value: string | null | undefined): number | null {
  if (!value) return null;
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

function pickNewerActivityAt(
  current: string,
  candidate: string,
): string {
  const currentMs = parseIsoMs(current);
  const candidateMs = parseIsoMs(candidate);
  if (currentMs === null) return candidate;
  if (candidateMs === null) return current;
  return candidateMs >= currentMs ? candidate : current;
}

export function applySessionStatePayload(
  session: SessionSummary,
  payload: SessionStatePayload,
): SessionSummary {
  if (
    session.state === payload.state &&
    session.current_command === payload.current_command &&
    session.transport_health === payload.transport_health
  ) {
    return session;
  }

  return {
    ...session,
    state: payload.state,
    current_command: payload.current_command,
    transport_health: payload.transport_health,
  };
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

  // Check if anything actually changed.
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
        old.tool === s.tool &&
        old.thought === s.thought &&
        old.thought_state === s.thought_state &&
        old.thought_source === s.thought_source &&
        old.thought_updated_at === s.thought_updated_at &&
        old.last_skill === s.last_skill &&
        old.token_count === s.token_count &&
        old.context_limit === s.context_limit &&
        old.last_activity_at === s.last_activity_at
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
      old.tool === s.tool &&
      old.thought === s.thought &&
      old.thought_state === s.thought_state &&
      old.thought_source === s.thought_source &&
      old.thought_updated_at === s.thought_updated_at &&
      old.last_skill === s.last_skill &&
      old.token_count === s.token_count &&
      old.context_limit === s.context_limit &&
      old.last_activity_at === s.last_activity_at
    ) {
      return old; // Preserve reference — avoids re-rendering this thronglet
    }
    // Prefer fresher activity timestamps from either stream while still avoiding
    // regressions when a delayed poll response arrives.
    return {
      ...s,
      last_activity_at: pickNewerActivityAt(old.last_activity_at, s.last_activity_at),
      token_count: s.token_count ?? old.token_count,
      context_limit: s.context_limit ?? old.context_limit,
      thought: s.thought,
      thought_state: s.thought_state,
      thought_source: s.thought_source,
      thought_updated_at: s.thought_updated_at,
      last_skill: s.last_skill,
    };
  });

  return localExiting.length > 0 ? [...merged, ...localExiting] : merged;
}

interface RestoreLayoutRequest extends WorkspaceLayoutState {
  requestId: number;
}

interface UndoDeleteState {
  sessionId: string;
  label: string;
  cwd: string;
  spawnTool?: SpawnTool;
}

interface DeleteFailureState {
  sessionId: string;
  label: string;
  message: string;
}

function sessionLabel(session: SessionSummary): string {
  const trimmed = session.cwd.trim();
  if (trimmed && trimmed !== "/") {
    const parts = trimmed.replace(/\/+$/, "").split("/").filter(Boolean);
    if (parts.length > 0) {
      return parts[parts.length - 1];
    }
  }
  return session.tmux_name;
}

function inferSpawnToolFromSessionTool(tool: string | null): SpawnTool | undefined {
  if (!tool) return undefined;
  const normalized = tool.trim().toLowerCase();
  if (normalized.includes("codex")) return "codex";
  if (normalized.includes("claude")) return "claude";
  return undefined;
}

export function App() {
  const [bootstrapDone, setBootstrapDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authMode, setAuthMode] = useState<string | undefined>(undefined);
  const [restoreRequest, setRestoreRequest] =
    useState<RestoreLayoutRequest | null>(null);
  const [axeArmed, setAxeArmed] = useState(false);
  const [benchArmed, setBenchArmed] = useState(false);
  const [benchedIds, setBenchedIds] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem("throngterm.benched-sessions.v1");
      if (!raw) return new Set();
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed) && parsed.every((id) => typeof id === "string")) {
        return new Set(parsed);
      }
      return new Set();
    } catch {
      return new Set();
    }
  });
  const [pendingUndoDelete, setPendingUndoDelete] =
    useState<UndoDeleteState | null>(null);
  const [undoInFlight, setUndoInFlight] = useState(false);
  const [deleteFailure, setDeleteFailure] = useState<DeleteFailureState | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const bootstrapDataRef = useRef<BootstrapResponse | null>(null);
  const workspaceHistoryModeRef = useRef("url_state_v1");
  const undoToastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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

  useEffect(() => {
    if (isObserver) {
      setAxeArmed(false);
      setBenchArmed(false);
    }
  }, [isObserver]);

  useEffect(() => {
    try {
      localStorage.setItem(
        "throngterm.benched-sessions.v1",
        JSON.stringify([...benchedIds]),
      );
    } catch {
      // localStorage write failure — ignore
    }
  }, [benchedIds]);

  useEffect(() => {
    return () => {
      if (undoToastTimerRef.current) {
        clearTimeout(undoToastTimerRef.current);
      }
    };
  }, []);

  // ---- Session helpers ----

  const upsertSession = useCallback((session: SessionSummary) => {
    const existingIndex = sessions.value.findIndex(
      (s) => s.session_id === session.session_id,
    );
    if (existingIndex === -1) {
      sessions.value = [...sessions.value, session];
      return;
    }
    const existing = sessions.value[existingIndex];
    // Preserve sprite_pack_id from existing session if the incoming one is null —
    // prevents late session_created events from wiping bootstrapped data.
    const merged =
      !session.sprite_pack_id && existing.sprite_pack_id
        ? { ...session, sprite_pack_id: existing.sprite_pack_id }
        : session;
    const next = [...sessions.value];
    next[existingIndex] = merged;
    sessions.value = next;
  }, []);

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

  const showUndoDeleteToast = useCallback((deletedSession: SessionSummary) => {
    if (undoToastTimerRef.current) {
      clearTimeout(undoToastTimerRef.current);
      undoToastTimerRef.current = null;
    }
    setPendingUndoDelete({
      sessionId: deletedSession.session_id,
      label: sessionLabel(deletedSession),
      cwd: deletedSession.cwd,
      spawnTool: inferSpawnToolFromSessionTool(deletedSession.tool),
    });
    undoToastTimerRef.current = setTimeout(() => {
      setPendingUndoDelete(null);
      undoToastTimerRef.current = null;
    }, 4500);
  }, []);

  const deleteSessionWithFeedback = useCallback(
    async (sessionId: string) => {
      const target = sessions.value.find((s) => s.session_id === sessionId);
      if (!target) {
        setDeleteFailure({
          sessionId,
          label: sessionId,
          message: "Session is no longer available.",
        });
        return;
      }

      try {
        await apiDeleteSession(sessionId, "kill_tmux");
        setDeleteFailure(null);
        showUndoDeleteToast(target);
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "Failed to delete session";
        setDeleteFailure({
          sessionId,
          label: sessionLabel(target),
          message,
        });
      }
    },
    [showUndoDeleteToast],
  );

  const handleUndoDelete = useCallback(async () => {
    if (!pendingUndoDelete || undoInFlight) return;
    const payload = pendingUndoDelete;
    if (undoToastTimerRef.current) {
      clearTimeout(undoToastTimerRef.current);
      undoToastTimerRef.current = null;
    }
    setPendingUndoDelete(null);
    setUndoInFlight(true);
    try {
      const resp = await apiCreateSession(
        undefined,
        payload.cwd,
        payload.spawnTool,
      );
      if (resp.sprite_pack && resp.session.sprite_pack_id) {
        spritePacks.value = {
          ...spritePacks.value,
          [resp.session.sprite_pack_id]: resp.sprite_pack,
        };
      }
      upsertSession(resp.session);
      setDeleteFailure(null);
      if (navigator.vibrate) navigator.vibrate(20);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to restore session";
      setDeleteFailure({
        sessionId: payload.sessionId,
        label: payload.label,
        message: `Undo failed: ${message}`,
      });
    } finally {
      setUndoInFlight(false);
    }
  }, [pendingUndoDelete, undoInFlight, upsertSession]);

  const retryDelete = useCallback(() => {
    if (!deleteFailure) return;
    void deleteSessionWithFeedback(deleteFailure.sessionId);
  }, [deleteFailure, deleteSessionWithFeedback]);

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
        updateSession(sessionId, (s) => applySessionStatePayload(s, payload));
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

      onSessionSkill(sessionId: string, payload: SessionSkillPayload) {
        updateSession(sessionId, (s) =>
          s.last_skill === payload.last_skill
            ? s
            : {
                ...s,
                last_skill: payload.last_skill,
              },
        );
      },

      onThoughtUpdate(sessionId: string, payload: ThoughtUpdatePayload) {
        const incomingUpdatedAtMs = parseIsoMs(payload.at);
        updateSession(sessionId, (s) => {
          const sameFields =
            s.thought === payload.thought &&
            s.thought_state === payload.thought_state &&
            s.thought_source === payload.thought_source &&
            s.token_count === payload.token_count &&
            s.context_limit === payload.context_limit;

          if (sameFields) {
            const currentUpdatedAtMs = parseIsoMs(s.thought_updated_at);
            if (
              incomingUpdatedAtMs !== null &&
              currentUpdatedAtMs !== null &&
              incomingUpdatedAtMs <= currentUpdatedAtMs
            ) {
              return s;
            }

            if (
              incomingUpdatedAtMs === null &&
              currentUpdatedAtMs === null &&
              s.thought_updated_at === payload.at
            ) {
              return s;
            }
          }

          return {
            ...s,
            thought: payload.thought,
            thought_state: payload.thought_state,
            thought_source: payload.thought_source,
            thought_updated_at: payload.at,
            token_count: payload.token_count,
            context_limit: payload.context_limit,
          };
        });
      },

      onSessionCreated(payload: SessionCreatedPayload) {
        if (payload.sprite_pack && payload.session.sprite_pack_id) {
          spritePacks.value = {
            ...spritePacks.value,
            [payload.session.sprite_pack_id]: payload.sprite_pack,
          };
        }
        upsertSession(payload.session);
      },

      onSessionDeleted(sessionId: string, _payload: SessionDeletedPayload) {
        clearIdlePreview(sessionId);
        outputActivityUpdateAtRef.current.delete(sessionId);
        exitingSessionIdsRef.current.delete(sessionId);
        sessions.value = sessions.value.filter(
          (s) => s.session_id !== sessionId,
        );
        setBenchedIds((prev) => {
          if (!prev.has(sessionId)) return prev;
          const next = new Set(prev);
          next.delete(sessionId);
          return next;
        });
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
  }, [updateSession, clearIdlePreview, upsertSession]);

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
          spritePacks.value = data.sprite_packs ?? {};
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

        // Prune benched IDs against live sessions
        const liveIds = new Set(initialSessions.map((s) => s.session_id));
        setBenchedIds((prev) => {
          const pruned = new Set([...prev].filter((id) => liveIds.has(id)));
          return pruned.size === prev.size ? prev : pruned;
        });

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
      setAxeArmed(false);
      setBenchArmed(false);
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

  const disarmAxeMode = useCallback(() => {
    setAxeArmed(false);
  }, []);

  const handleToggleBenchArm = useCallback(() => {
    setBenchArmed((prev) => {
      if (!prev) setAxeArmed(false); // mutual exclusion
      return !prev;
    });
  }, []);

  const handleBenchSession = useCallback((sessionId: string) => {
    setBenchArmed(false);
    setBenchedIds((prev) => new Set([...prev, sessionId]));
    if (navigator.vibrate) navigator.vibrate(20);
  }, []);

  const handleUnbenchSession = useCallback((sessionId: string) => {
    setBenchedIds((prev) => {
      const next = new Set(prev);
      next.delete(sessionId);
      return next;
    });
  }, []);

  const handleBenchToggleFromTerminal = useCallback((sessionId: string) => {
    setBenchedIds((prev) => {
      const next = new Set(prev);
      if (next.has(sessionId)) {
        next.delete(sessionId);
      } else {
        next.add(sessionId);
      }
      return next;
    });
  }, []);

  const handleOverviewTap = useCallback(
    (sessionId: string) => {
      if (benchArmed) {
        handleBenchSession(sessionId);
        return;
      }
      if (!axeArmed) {
        openTerminal(sessionId);
        return;
      }

      setAxeArmed(false);
      if (navigator.vibrate) navigator.vibrate([20, 30, 20]);
      void deleteSessionWithFeedback(sessionId);
    },
    [axeArmed, benchArmed, openTerminal, deleteSessionWithFeedback, handleBenchSession],
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
  const showTransportBanner =
    transportHealth.value !== "healthy" &&
    transportHealth.value !== "disconnected";
  const fieldAxeTopOffset = showTransportBanner ? 30 : 8;
  const overviewInteractive = isOverview || splitMode;
  const terminalInteractive = isTerminal;

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
      {showTransportBanner && (
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
        class={`view ${overviewInteractive ? "active" : ""}`}
        style={{
          position: "absolute",
          top: 0,
          left: splitMode ? "50%" : 0,
          right: splitMode ? "auto" : 0,
          width: splitMode ? "50%" : "100%",
          bottom: 0,
          transform: overviewTransform,
          transition: "transform 0.25s ease, left 0.25s ease",
          zIndex: splitMode ? 2 : 1,
          pointerEvents: overviewInteractive ? "auto" : "none",
        }}
      >
        <OverviewField
          sessions={sessions.value}
          idlePreviews={idlePreviews.value}
          observer={isObserver}
          compact={splitMode}
          axeTopOffset={fieldAxeTopOffset}
          axeArmed={axeArmed}
          benchArmed={benchArmed}
          benchedIds={benchedIds}
          onToggleAxe={() => {
            setAxeArmed((prev) => {
              if (!prev) setBenchArmed(false); // mutual exclusion
              return !prev;
            });
          }}
          onDisarmAxe={disarmAxeMode}
          onToggleBenchArm={handleToggleBenchArm}
          onBenchSession={handleBenchSession}
          onUnbenchSession={handleUnbenchSession}
          onTapSession={handleOverviewTap}
          onDragToBottom={(id) => openTerminal(id, "bottom")}
          onCreateSession={async (cwd?: string, spawnTool?: SpawnTool) => {
            if (isObserver) return "";
            try {
              const resp = await apiCreateSession(undefined, cwd, spawnTool);
              if (resp.sprite_pack && resp.session.sprite_pack_id) {
                spritePacks.value = {
                  ...spritePacks.value,
                  [resp.session.sprite_pack_id]: resp.sprite_pack,
                };
              }
              upsertSession(resp.session);
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
          right: splitMode ? "auto" : 0,
          width: splitMode ? "50%" : "100%",
          bottom: 0,
          transform: isTerminal ? "translateX(0)" : "translateX(100%)",
          transition: "transform 0.25s ease, right 0.25s ease",
          display: "flex",
          flexDirection: "row",
          zIndex: splitMode ? 1 : 2,
          pointerEvents: terminalInteractive ? "auto" : "none",
        }}
      >
        {isTerminal && (
          <ZoneManager
            sessions={sessions.value}
            activeSessionId={activeSessionId.value}
            preferZone={activeZonePreference.value}
            restoreRequest={restoreRequest}
            observer={isObserver}
            benchedIds={benchedIds}
            onBenchToggle={handleBenchToggleFromTerminal}
            onShowOverview={showOverview}
            onStartPolling={startPolling}
            onStopPolling={stopPolling}
            onLayoutChange={handleLayoutChange}
          />
        )}
      </div>

      {pendingUndoDelete && (
        <div class="axe-toast">
          <span class="axe-toast-label">{`Axed ${pendingUndoDelete.label}`}</span>
          <button
            type="button"
            class="axe-toast-btn"
            onClick={() => void handleUndoDelete()}
            disabled={undoInFlight}
          >
            {undoInFlight ? "Restoring..." : "Undo"}
          </button>
        </div>
      )}

      {deleteFailure && (
        <div class="axe-error-toast" role="alert">
          <div class="axe-error-title">{`Axe failed: ${deleteFailure.label}`}</div>
          <div class="axe-error-message">{deleteFailure.message}</div>
          <div class="axe-error-actions">
            <button
              type="button"
              class="axe-error-btn"
              onClick={retryDelete}
            >
              Retry
            </button>
            <button
              type="button"
              class="axe-error-btn dismiss"
              onClick={() => setDeleteFailure(null)}
            >
              Dismiss
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
