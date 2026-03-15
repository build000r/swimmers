import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { signal, batch } from "@preact/signals";
import type {
  SessionSummary,
  TransportHealth,
  BootstrapResponse,
  SpawnTool,
  RepoTheme,
  SpritePack,
  NativeDesktopStatus,
  CreateSessionResponse,
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
  fetchNativeDesktopStatus,
  openNativeDesktopSession,
} from "@/services/api";
import { RealtimeService } from "@/services/realtime";
import type { WorkspaceLayoutState } from "@/services/workspace-history";
import {
  applyWorkspaceLayoutToUrl,
  normalizeWorkspaceLayout,
  parseWorkspaceLayoutFromUrl,
} from "@/services/workspace-history";
import {
  isProcessExitState,
  normalizeExitReason,
  shouldHideSessionFromOverview,
} from "@/lib/session-exit";
import { OverviewField } from "@/components/OverviewField";
import { ZoneManager } from "@/components/ZoneManager";
import { useObserverMode } from "@/hooks/useObserverMode";
import { shouldOpenNativeDesktopByDefault } from "@/lib/native-desktop";

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
export const repoThemes = signal<Record<string, RepoTheme>>({});

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

function fallbackRestStateForSessionState(
  state: SessionSummary["state"],
  thoughtState: SessionSummary["thought_state"],
): SessionSummary["rest_state"] {
  switch (state) {
    case "exited":
      return "deep_sleep";
    case "idle":
      switch (thoughtState) {
        case "active":
          return "active";
        case "sleeping":
          return "sleeping";
        case "holding":
        default:
          return "drowsy";
      }
    case "busy":
    case "error":
    case "attention":
    default:
      return "active";
  }
}

export function applySessionStatePayload(
  session: SessionSummary,
  payload: SessionStatePayload,
): SessionSummary {
  const exitReason = normalizeExitReason(payload.state, payload.exit_reason);
  const exited = payload.state === "exited";
  const restState =
    session.state === payload.state
      ? session.rest_state
      : fallbackRestStateForSessionState(payload.state, session.thought_state);
  const nextThought = exited ? null : session.thought;
  const nextThoughtState = exited ? "holding" : session.thought_state;
  const nextThoughtSource = exited ? "carry_forward" : session.thought_source;
  const nextThoughtUpdatedAt = exited ? null : session.thought_updated_at;
  if (
    session.state === payload.state &&
    (session.exit_reason ?? null) === exitReason &&
    session.current_command === payload.current_command &&
    session.transport_health === payload.transport_health &&
    session.rest_state === restState &&
    session.thought === nextThought &&
    session.thought_state === nextThoughtState &&
    session.thought_source === nextThoughtSource &&
    session.thought_updated_at === nextThoughtUpdatedAt
  ) {
    return session;
  }

  return {
    ...session,
    state: payload.state,
    exit_reason: exitReason,
    current_command: payload.current_command,
    transport_health: payload.transport_health,
    rest_state: restState,
    thought: nextThought,
    thought_state: nextThoughtState,
    thought_source: nextThoughtSource,
    thought_updated_at: nextThoughtUpdatedAt,
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
        old.sprite_pack_id === s.sprite_pack_id &&
        old.repo_theme_id === s.repo_theme_id &&
        old.thought === s.thought &&
        old.thought_state === s.thought_state &&
        old.thought_source === s.thought_source &&
        old.thought_updated_at === s.thought_updated_at &&
        old.rest_state === s.rest_state &&
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
      old.sprite_pack_id === s.sprite_pack_id &&
      old.repo_theme_id === s.repo_theme_id &&
      old.thought === s.thought &&
      old.thought_state === s.thought_state &&
      old.thought_source === s.thought_source &&
      old.thought_updated_at === s.thought_updated_at &&
      old.rest_state === s.rest_state &&
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
      sprite_pack_id: s.sprite_pack_id,
      repo_theme_id: s.repo_theme_id,
      thought: s.thought,
      thought_state: s.thought_state,
      thought_source: s.thought_source,
      thought_updated_at: s.thought_updated_at,
      rest_state: s.rest_state,
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

interface NativeFailureState {
  label: string;
  message: string;
}

type NativeOpenResult = "opened" | "fallback" | "missing";

const NATIVE_DESKTOP_UNSUPPORTED: NativeDesktopStatus = {
  supported: false,
  platform: null,
  app: null,
  reason: null,
};

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

type OverviewTapIntent =
  | { type: "bench"; sessionId: string }
  | { type: "refresh" }
  | { type: "open-native-or-terminal"; session: SessionSummary }
  | { type: "open-terminal"; sessionId: string }
  | { type: "delete"; sessionId: string }
  | { type: "noop" };

interface ResolveOverviewTapIntentInput {
  sessionId: string;
  sessionsList: SessionSummary[];
  benchArmed: boolean;
  axeArmed: boolean;
  nativeDesktop: NativeDesktopStatus;
  isObserver: boolean;
}

export function resolveOverviewTapIntent(
  input: ResolveOverviewTapIntentInput,
): OverviewTapIntent {
  if (input.benchArmed) {
    return { type: "bench", sessionId: input.sessionId };
  }

  const target = input.sessionsList.find(
    (session) => session.session_id === input.sessionId,
  );
  if (!target) {
    return { type: "noop" };
  }

  if (input.axeArmed) {
    return { type: "delete", sessionId: input.sessionId };
  }

  if (target.is_stale) {
    return { type: "refresh" };
  }

  if (shouldOpenNativeDesktopByDefault(input.nativeDesktop, input.isObserver)) {
    return { type: "open-native-or-terminal", session: target };
  }

  return { type: "open-terminal", sessionId: input.sessionId };
}

interface AppViewFlagsInput {
  view: "overview" | "terminal";
  zoneLayout: "single" | "dual";
  viewportWidth: number;
  transport: TransportHealth;
}

interface AppViewFlags {
  isOverview: boolean;
  isTerminal: boolean;
  splitMode: boolean;
  showTransportBanner: boolean;
  fieldAxeTopOffset: number;
  overviewInteractive: boolean;
  terminalInteractive: boolean;
  overviewTransform: "none" | "translateX(-100%)";
}

export function deriveAppViewFlags(input: AppViewFlagsInput): AppViewFlags {
  const isOverview = input.view === "overview";
  const isTerminal = input.view === "terminal";
  const splitMode =
    isTerminal && input.zoneLayout === "single" && input.viewportWidth > 768;
  const showTransportBanner =
    input.transport !== "healthy" && input.transport !== "disconnected";

  return {
    isOverview,
    isTerminal,
    splitMode,
    showTransportBanner,
    fieldAxeTopOffset: showTransportBanner ? 30 : 8,
    overviewInteractive: isOverview || splitMode,
    terminalInteractive: isTerminal,
    overviewTransform: isOverview
      ? "none"
      : splitMode
        ? "none"
        : "translateX(-100%)",
  };
}

function readBenchedIdsFromStorage(): Set<string> {
  try {
    const raw = localStorage.getItem("throngterm.benched-sessions.v1");
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.every((id) => typeof id === "string")) {
      return new Set(parsed);
    }
  } catch {
    // Ignore malformed local storage state.
  }
  return new Set();
}

function persistBenchedIds(ids: Set<string>): void {
  try {
    localStorage.setItem("throngterm.benched-sessions.v1", JSON.stringify([...ids]));
  } catch {
    // localStorage write failure — ignore
  }
}

function applyCreateSessionAssets(response: CreateSessionResponse): void {
  if (response.sprite_pack && response.session.sprite_pack_id) {
    spritePacks.value = {
      ...spritePacks.value,
      [response.session.sprite_pack_id]: response.sprite_pack,
    };
  }
  if (response.repo_theme && response.session.repo_theme_id) {
    repoThemes.value = {
      ...repoThemes.value,
      [response.session.repo_theme_id]: response.repo_theme,
    };
  }
}

interface CreateOverviewSessionInput {
  cwd?: string;
  spawnTool?: SpawnTool;
  isObserver: boolean;
  nativeDesktopSupported: boolean;
  upsertSession: (session: SessionSummary) => void;
  openNativeSessionOrTerminal: (session: SessionSummary) => Promise<boolean>;
}

async function createOverviewSession(
  input: CreateOverviewSessionInput,
): Promise<string> {
  if (input.isObserver) return "";
  try {
    const response = await apiCreateSession(undefined, input.cwd, input.spawnTool);
    applyCreateSessionAssets(response);
    input.upsertSession(response.session);
    if (input.nativeDesktopSupported) {
      void input.openNativeSessionOrTerminal(response.session);
    }
    return response.session.session_id;
  } catch (err) {
    console.error("Failed to create session:", err);
    return "";
  }
}

interface OpenNativeSessionInput {
  session: SessionSummary;
  nativeDesktopSupported: boolean;
  isObserver: boolean;
  showNativeFailureToast: (label: string, message: string) => void;
  refreshSessionsFromServer: () => Promise<SessionSummary[]>;
  clearNativeFailure: () => void;
}

async function openNativeSessionWithFallback(
  input: OpenNativeSessionInput,
): Promise<NativeOpenResult> {
  if (!input.nativeDesktopSupported || input.isObserver) return "fallback";
  if (input.session.state === "exited") {
    input.showNativeFailureToast(
      sessionLabel(input.session),
      "Session has already exited.",
    );
    return "missing";
  }
  if (input.session.is_stale) {
    void input.refreshSessionsFromServer().catch(() => {});
    return "missing";
  }

  if (input.session.state === "attention") {
    realtime.sendDismissAttention(input.session.session_id);
  }

  try {
    await openNativeDesktopSession(input.session.session_id);
    input.clearNativeFailure();
    return "opened";
  } catch (err) {
    const message =
      err instanceof Error ? err.message : "Failed to open session in iTerm";
    if (message === "SESSION_NOT_FOUND") {
      input.showNativeFailureToast(
        sessionLabel(input.session),
        "Session is no longer available.",
      );
      void input.refreshSessionsFromServer().catch(() => {});
      return "missing";
    }
    input.showNativeFailureToast(sessionLabel(input.session), message);
    return "fallback";
  }
}

interface OverviewTapExecutionInput {
  intent: OverviewTapIntent;
  handleBenchSession: (sessionId: string) => void;
  refreshSessionsFromServer: () => Promise<SessionSummary[]>;
  openNativeSessionOrTerminal: (session: SessionSummary) => Promise<boolean>;
  openTerminal: (sessionId: string) => void;
  setAxeArmed: (next: boolean) => void;
  deleteSessionWithFeedback: (sessionId: string) => Promise<void>;
}

function executeOverviewTapIntent(input: OverviewTapExecutionInput): void {
  switch (input.intent.type) {
    case "bench":
      input.handleBenchSession(input.intent.sessionId);
      return;
    case "refresh":
      void input.refreshSessionsFromServer().catch(() => {});
      return;
    case "open-native-or-terminal":
      void input.openNativeSessionOrTerminal(input.intent.session);
      return;
    case "open-terminal":
      input.openTerminal(input.intent.sessionId);
      return;
    case "delete":
      input.setAxeArmed(false);
      if (navigator.vibrate) navigator.vibrate([20, 30, 20]);
      void input.deleteSessionWithFeedback(input.intent.sessionId);
      return;
    case "noop":
    default:
      return;
  }
}

export function App() {
  const [bootstrapDone, setBootstrapDone] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authMode, setAuthMode] = useState<string | undefined>(undefined);
  const [restoreRequest, setRestoreRequest] =
    useState<RestoreLayoutRequest | null>(null);
  const [axeArmed, setAxeArmed] = useState(false);
  const [benchArmed, setBenchArmed] = useState(false);
  const [benchedIds, setBenchedIds] = useState<Set<string>>(readBenchedIdsFromStorage);
  const [pendingUndoDelete, setPendingUndoDelete] =
    useState<UndoDeleteState | null>(null);
  const [undoInFlight, setUndoInFlight] = useState(false);
  const [deleteFailure, setDeleteFailure] = useState<DeleteFailureState | null>(null);
  const [nativeDesktop, setNativeDesktop] = useState<NativeDesktopStatus>(
    NATIVE_DESKTOP_UNSUPPORTED,
  );
  const [nativeFailure, setNativeFailure] = useState<NativeFailureState | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const bootstrapDataRef = useRef<BootstrapResponse | null>(null);
  const workspaceHistoryModeRef = useRef("url_state_v1");
  const undoToastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const nativeFailureTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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
    persistBenchedIds(benchedIds);
  }, [benchedIds]);

  useEffect(() => {
    return () => {
      if (undoToastTimerRef.current) {
        clearTimeout(undoToastTimerRef.current);
      }
      if (nativeFailureTimerRef.current) {
        clearTimeout(nativeFailureTimerRef.current);
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
    const withTheme =
      !merged.repo_theme_id && existing.repo_theme_id
        ? { ...merged, repo_theme_id: existing.repo_theme_id }
        : merged;
    const next = [...sessions.value];
    next[existingIndex] = withTheme;
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

  const refreshSessionsFromServer = useCallback(async () => {
    const resp = await fetchSessions();
    if (Object.keys(resp.sprite_packs ?? {}).length > 0) {
      spritePacks.value = {
        ...spritePacks.value,
        ...resp.sprite_packs,
      };
    }
    if (Object.keys(resp.repo_themes ?? {}).length > 0) {
      repoThemes.value = {
        ...repoThemes.value,
        ...resp.repo_themes,
      };
    }
    const liveSessions = dedupeSessionsById(resp.sessions);
    const merged = mergePollSessions(
      sessions.value,
      liveSessions,
      exitingSessionIdsRef.current,
    );
    if (merged) {
      sessions.value = merged;
    }

    const liveIds = new Set(liveSessions.map((session) => session.session_id));
    setBenchedIds((prev) => {
      const pruned = new Set([...prev].filter((id) => liveIds.has(id)));
      return pruned.size === prev.size ? prev : pruned;
    });

    return liveSessions;
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
      applyCreateSessionAssets(resp);
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

  const showNativeFailureToast = useCallback((label: string, message: string) => {
    if (nativeFailureTimerRef.current) {
      clearTimeout(nativeFailureTimerRef.current);
      nativeFailureTimerRef.current = null;
    }
    setNativeFailure({ label, message });
    nativeFailureTimerRef.current = setTimeout(() => {
      setNativeFailure(null);
      nativeFailureTimerRef.current = null;
    }, 5000);
  }, []);

  const openNativeSession = useCallback(
    (session: SessionSummary): Promise<NativeOpenResult> =>
      openNativeSessionWithFallback({
        session,
        nativeDesktopSupported: nativeDesktop.supported,
        isObserver,
        showNativeFailureToast,
        refreshSessionsFromServer,
        clearNativeFailure: () => setNativeFailure(null),
      }),
    [
      isObserver,
      nativeDesktop.supported,
      refreshSessionsFromServer,
      showNativeFailureToast,
    ],
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
        updateSession(sessionId, (s) => applySessionStatePayload(s, payload));
        if (payload.state !== "idle") {
          clearIdlePreview(sessionId);
        }
        if (isProcessExitState(payload)) {
          outputActivityUpdateAtRef.current.delete(sessionId);
          exitingSessionIdsRef.current.delete(sessionId);
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
            s.rest_state === payload.rest_state &&
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
            rest_state: payload.rest_state,
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
        if (payload.repo_theme && payload.session.repo_theme_id) {
          repoThemes.value = {
            ...repoThemes.value,
            [payload.session.repo_theme_id]: payload.repo_theme,
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
        const [data, nativeDesktopStatus] = await Promise.all([
          apiFetch(),
          fetchNativeDesktopStatus().catch(() => NATIVE_DESKTOP_UNSUPPORTED),
        ]);
        if (cancelled) return;
        bootstrapDataRef.current = data;
        let initialSessions = dedupeSessionsById(data.sessions);
        if (initialSessions.some((session) => session.is_stale)) {
          try {
            const resp = await fetchSessions();
            if (cancelled) return;
            initialSessions = dedupeSessionsById(resp.sessions);
            data.sprite_packs = {
              ...(data.sprite_packs ?? {}),
              ...(resp.sprite_packs ?? {}),
            };
            data.repo_themes = {
              ...(data.repo_themes ?? {}),
              ...(resp.repo_themes ?? {}),
            };
          } catch {
            // Keep bootstrap data if the active session refresh fails.
          }
        }

        batch(() => {
          sessions.value = initialSessions;
          terminalCacheTtlMs.value = data.terminal_cache_ttl_ms;
          idlePreviews.value = {};
          spritePacks.value = data.sprite_packs ?? {};
          repoThemes.value = data.repo_themes ?? {};
        });
        idlePreviewFetchInFlightRef.current.clear();
        idlePreviewLastFetchAtRef.current.clear();
        outputActivityUpdateAtRef.current.clear();

        setAuthMode(data.auth_mode);
        setNativeDesktop(nativeDesktopStatus);
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

  const openNativeSessionOrTerminal = useCallback(
    async (session: SessionSummary) => {
      const result = await openNativeSession(session);
      if (result === "fallback" && session.state !== "exited") {
        openTerminal(session.session_id);
      }
      return result === "opened";
    },
    [openNativeSession, openTerminal],
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
      const intent = resolveOverviewTapIntent({
        sessionId,
        sessionsList: sessions.value,
        benchArmed,
        axeArmed,
        nativeDesktop,
        isObserver,
      });
      executeOverviewTapIntent({
        intent,
        handleBenchSession,
        refreshSessionsFromServer,
        openNativeSessionOrTerminal,
        openTerminal,
        setAxeArmed,
        deleteSessionWithFeedback,
      });
    },
    [
      axeArmed,
      benchArmed,
      deleteSessionWithFeedback,
      handleBenchSession,
      isObserver,
      nativeDesktop,
      openNativeSessionOrTerminal,
      refreshSessionsFromServer,
      openTerminal,
    ],
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

  const handleToggleAxe = useCallback(() => {
    setAxeArmed((prev) => {
      if (!prev) setBenchArmed(false); // mutual exclusion
      return !prev;
    });
  }, []);

  const handleCreateSession = useCallback(
    (cwd?: string, spawnTool?: SpawnTool) =>
      createOverviewSession({
        cwd,
        spawnTool,
        isObserver,
        nativeDesktopSupported: nativeDesktop.supported,
        upsertSession,
        openNativeSessionOrTerminal,
      }),
    [isObserver, nativeDesktop.supported, upsertSession, openNativeSessionOrTerminal],
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

  const viewFlags = deriveAppViewFlags({
    view: currentView.value,
    zoneLayout: zoneLayout.value,
    viewportWidth: window.innerWidth,
    transport: transportHealth.value,
  });
  const overviewSessions = sessions.value.filter(
    (session) => !shouldHideSessionFromOverview(session),
  );

  return (
    <div style={{ position: "absolute", inset: 0, overflow: "hidden" }}>
      {/* Transport health banner */}
      {viewFlags.showTransportBanner && (
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
        class={`view ${viewFlags.overviewInteractive ? "active" : ""}`}
        style={{
          position: "absolute",
          top: 0,
          left: viewFlags.splitMode ? "50%" : 0,
          right: viewFlags.splitMode ? "auto" : 0,
          width: viewFlags.splitMode ? "50%" : "100%",
          bottom: 0,
          transform: viewFlags.overviewTransform,
          transition: "transform 0.25s ease, left 0.25s ease",
          zIndex: viewFlags.splitMode ? 2 : 1,
          pointerEvents: viewFlags.overviewInteractive ? "auto" : "none",
        }}
      >
        <OverviewField
          sessions={overviewSessions}
          idlePreviews={idlePreviews.value}
          observer={isObserver}
          compact={viewFlags.splitMode}
          axeTopOffset={viewFlags.fieldAxeTopOffset}
          axeArmed={axeArmed}
          benchArmed={benchArmed}
          benchedIds={benchedIds}
          onToggleAxe={handleToggleAxe}
          onDisarmAxe={disarmAxeMode}
          onToggleBenchArm={handleToggleBenchArm}
          onBenchSession={handleBenchSession}
          onUnbenchSession={handleUnbenchSession}
          onTapSession={handleOverviewTap}
          onDragToBottom={(id) => openTerminal(id, "bottom")}
          onCreateSession={handleCreateSession}
        />
      </div>

      {/* Terminal workspace */}
      <div
        class={`view ${viewFlags.isTerminal ? "active" : ""}`}
        style={{
          position: "absolute",
          top: 0,
          left: 0,
          right: viewFlags.splitMode ? "auto" : 0,
          width: viewFlags.splitMode ? "50%" : "100%",
          bottom: 0,
          transform: viewFlags.isTerminal ? "translateX(0)" : "translateX(100%)",
          transition: "transform 0.25s ease, right 0.25s ease",
          display: "flex",
          flexDirection: "row",
          zIndex: viewFlags.splitMode ? 1 : 2,
          pointerEvents: viewFlags.terminalInteractive ? "auto" : "none",
        }}
      >
        {viewFlags.isTerminal && (
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

      {nativeFailure && (
        <div class="axe-error-toast" role="alert">
          <div class="axe-error-title">{`iTerm open failed: ${nativeFailure.label}`}</div>
          <div class="axe-error-message">{nativeFailure.message}</div>
          <div class="axe-error-actions">
            <button
              type="button"
              class="axe-error-btn dismiss"
              onClick={() => setNativeFailure(null)}
            >
              Dismiss
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
