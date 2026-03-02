import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon } from "@xterm/addon-search";
import type { SessionSummary, SkillRegistryTool, SkillSummary } from "@/types";
import { realtime, spritePacks } from "@/app";
import type { TerminalOutputFrame } from "@/services/realtime";
import type { CachedTerminal } from "@/hooks/useTerminalCache";
import { fetchSnapshot, listSkills } from "@/services/api";
import {
  copyTextToClipboard,
  readTextFromClipboardWithFallback,
} from "@/lib/clipboard";
import {
  encodeTerminalInputChunk,
  type TerminalInputEncodingState,
} from "@/lib/terminal-input";
import {
  beginTouchScroll,
  createTouchScrollState,
  endTouchScroll,
  nextTouchScrollTop,
  type TouchScrollState,
} from "@/lib/touch-scroll";
import { resolveTerminalShortcutAction } from "@/lib/terminal-shortcuts";
import {
  computeFastScrollThumbHeight,
  computeFastScrollThumbTop,
  computeScrollTopFromThumbOffset,
  hasFastScrollOverflow,
  isNearScrollBottom,
} from "@/lib/fast-scroll";
import {
  computeCopyDragEdgeDirection,
  mapClientYToBufferRow,
} from "@/lib/copy-drag";
import { orderQuickSkillChips } from "@/lib/skill-order";
import { ThrongletSprite } from "./ThrongletSprite";

function warn_silence_recovery(sessionId: string): void {
  console.warn(
    `[throngterm] silence detected for busy session ${sessionId}, triggering re-subscribe + snapshot recovery`,
  );
}

function cwdLabel(cwd: string): string {
  const trimmed = cwd.trim();
  if (!trimmed || trimmed === "/") return "";
  const parts = trimmed.replace(/\/+$/, "").split("/").filter(Boolean);
  if (parts.length === 0) return "";
  if (parts.length === 1) return parts[0];
  return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
}

interface TerminalWorkspaceProps {
  session: SessionSummary;
  /** If non-null, restore this cached terminal instead of creating a new one */
  cached: CachedTerminal | null;
  /** Observer mode disables input */
  observer?: boolean;
  /** Whether this session is benched (hidden from overview) */
  isBenched?: boolean;
  /** Toggle bench state for this session */
  onBenchToggle?: (sessionId: string) => void;
  /** Called when the workspace wants to cache its terminal (e.g., before unmount) */
  onCache: (cached: CachedTerminal) => void;
  /** Called when session exits */
  onSessionExit: (sessionId: string) => void;
  /** Called when header sprite is clicked (close zone) */
  onClose: () => void;
}

type MobileKeyId =
  | "tab"
  | "esc"
  | "ctrl_c"
  | "up"
  | "down"
  | "left"
  | "right"
  | "pgup"
  | "pgdn"
  | "pipe"
  | "slash"
  | "tilde";

type MobileKeyConfig = {
  id: MobileKeyId;
  label: string;
  input?: string;
};

type QuickCommandMap = Record<string, string[]>;
type SessionToolKind = "raw" | SkillRegistryTool;

const encoder = new TextEncoder();
const SEARCH_ADDONS_BY_TERM = new WeakMap<Terminal, SearchAddon>();
const SKILL_REGISTRY_CACHE: Partial<Record<SkillRegistryTool, SkillSummary[]>> = {};
const SKILL_REGISTRY_PENDING = new Map<SkillRegistryTool, Promise<SkillSummary[]>>();
const QUICK_COMMAND_STORAGE_KEY = "throngterm.quick-commands.v1";
const DEFAULT_QUICK_COMMANDS = ["ls", "git status", "npm test"];
const MAX_QUICK_COMMANDS = 12;
const MAX_QUICK_COMMAND_LENGTH = 80;
const SKILL_AUTO_RETRY_DELAYS_MS = [1500, 4000, 8000] as const;
const SKILL_LONG_PRESS_MS = 260;
const SKILL_LONG_PRESS_CANCEL_DISTANCE_PX = 14;
const SKILL_CLICK_SUPPRESS_MS = 450;
const LONG_PRESS_DELAY_MS = 650;
const LONG_PRESS_CANCEL_DISTANCE_PX = 8;
const ACTION_TOAST_MS = 1200;
const FAST_SCROLL_HIDE_MS = 1200;
const FAST_SCROLL_MIN_THUMB_PX = 26;
const FAST_SCROLL_BOOST_THRESHOLD = 1.15;
const COPY_DRAG_EDGE_RATIO = 0.2;
const COPY_DRAG_EDGE_MIN_PX = 56;
const COPY_DRAG_EDGE_MAX_PX = 140;
const COPY_DRAG_AUTOSCROLL_MS = 20;
const COPY_DRAG_STEP_MULTIPLIER = 1.45;

const MOBILE_KEYS: MobileKeyConfig[] = [
  { id: "tab", label: "Tab", input: "\t" },
  { id: "esc", label: "Esc", input: "\x1b" },
  { id: "ctrl_c", label: "Ctrl+C", input: "\x03\r" },
  { id: "up", label: "↑", input: "\x1b[A" },
  { id: "down", label: "↓", input: "\x1b[B" },
  { id: "left", label: "←", input: "\x1b[D" },
  { id: "right", label: "→", input: "\x1b[C" },
  { id: "pgup", label: "PgUp", input: "\x1b[5~" },
  { id: "pgdn", label: "PgDn", input: "\x1b[6~" },
  { id: "pipe", label: "|", input: "|" },
  { id: "slash", label: "/", input: "/" },
  { id: "tilde", label: "~", input: "~" },
];

function classifySessionTool(tool: string | null): SessionToolKind {
  if (!tool) return "raw";
  const normalized = tool.trim().toLowerCase();
  if (!normalized) return "raw";
  if (normalized.includes("claude")) return "claude";
  if (normalized.includes("codex")) return "codex";
  return "raw";
}

function skillInvocationPrefix(tool: SessionToolKind): string {
  if (tool === "claude") return "/";
  if (tool === "codex") return "$";
  return "";
}

async function loadSkillRegistry(
  tool: SkillRegistryTool,
  forceRefresh = false,
): Promise<SkillSummary[]> {
  if (!forceRefresh) {
    const cached = SKILL_REGISTRY_CACHE[tool];
    if (cached) return cached;
    const pending = SKILL_REGISTRY_PENDING.get(tool);
    if (pending) return pending;
  }

  const request = listSkills(tool)
    .then((resp) => {
      const skills = orderQuickSkillChips(resp.skills ?? []);
      SKILL_REGISTRY_CACHE[tool] = skills;
      SKILL_REGISTRY_PENDING.delete(tool);
      return skills;
    })
    .catch((error) => {
      SKILL_REGISTRY_PENDING.delete(tool);
      throw error;
    });

  SKILL_REGISTRY_PENDING.set(tool, request);
  return request;
}

function sanitizeQuickCommands(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const next: string[] = [];
  for (const item of value) {
    if (typeof item !== "string") continue;
    const trimmed = item.trim();
    if (!trimmed) continue;
    const command = trimmed.slice(0, MAX_QUICK_COMMAND_LENGTH);
    if (next.includes(command)) continue;
    next.push(command);
    if (next.length >= MAX_QUICK_COMMANDS) break;
  }
  return next;
}

function readQuickCommandMap(): QuickCommandMap {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(QUICK_COMMAND_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    const parsedObj = parsed as Record<string, unknown>;
    const next: QuickCommandMap = {};
    for (const [sessionId, commands] of Object.entries(parsedObj)) {
      if (!sessionId) continue;
      const sanitized = sanitizeQuickCommands(commands);
      if (sanitized.length > 0) next[sessionId] = sanitized;
    }
    return next;
  } catch {
    return {};
  }
}

function writeQuickCommandMap(map: QuickCommandMap): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(QUICK_COMMAND_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // Ignore storage failures.
  }
}

function ensureSearchAddon(term: Terminal): SearchAddon {
  let addon = SEARCH_ADDONS_BY_TERM.get(term);
  if (!addon) {
    addon = new SearchAddon();
    term.loadAddon(addon);
    SEARCH_ADDONS_BY_TERM.set(term, addon);
  }
  return addon;
}

export function TerminalWorkspace({
  session,
  cached,
  observer = false,
  isBenched = false,
  onBenchToggle,
  onCache,
  onSessionExit,
  onClose,
}: TerminalWorkspaceProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const focusTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const actionToastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressStartRef = useRef<{ x: number; y: number } | null>(null);
  const skillLongPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const skillPressRef = useRef<{
    skillName: string;
    pointerId: number;
    startX: number;
    startY: number;
  } | null>(null);
  const suppressSkillClickRef = useRef<{
    skillName: string;
    until: number;
  } | null>(null);
  const findInputRef = useRef<HTMLInputElement | null>(null);
  const seqRef = useRef<number>(0);
  const snapshotReadyRef = useRef(false);
  const pendingFramesRef = useRef<TerminalOutputFrame[]>([]);
  const initDoneRef = useRef(false);
  const autoRecoveryKeyRef = useRef<string | null>(null);
  const recoverFromSnapshotRef = useRef<(() => Promise<void>) | null>(null);
  const sessionStateRef = useRef(session.state);
  const silenceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputEncodingStateRef = useRef<TerminalInputEncodingState>({
    pendingHighSurrogate: "",
  });
  const touchScrollStateRef = useRef<TouchScrollState>(createTouchScrollState());
  const viewportRef = useRef<HTMLElement | null>(null);
  const fastScrollHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fastScrollRafRef = useRef<number | null>(null);
  const fastScrollShowPendingRef = useRef(false);
  const fastScrollVelocityRef = useRef(0);
  const fastScrollLastSampleRef = useRef<{ top: number; at: number } | null>(null);
  const fastScrollDragRef = useRef<{
    pointerId: number;
    startY: number;
    startThumbTop: number;
  } | null>(null);
  const fastScrollDragCleanupRef = useRef<(() => void) | null>(null);
  const copyDragActiveRef = useRef(false);
  const copyDragAnchorRowRef = useRef<number | null>(null);
  const copyDragPointerIdRef = useRef<number | null>(null);
  const copyDragLastClientYRef = useRef<number | null>(null);
  const copyDragEdgeDirectionRef = useRef(0);
  const copyDragAutoScrollTimerRef = useRef<ReturnType<typeof setInterval> | null>(
    null,
  );

  const [title, setTitle] = useState(`tmux a -t ${session.tmux_name}`);
  const [titleCopied, setTitleCopied] = useState(false);
  const [rushingOff, setRushingOff] = useState(false);
  const [lifecycleState, setLifecycleState] = useState<
    "attaching" | "snapshot_or_replay" | "live"
  >("attaching");
  const [recoveryBanner, setRecoveryBanner] = useState<string | null>(null);
  const [recoveryRetrying, setRecoveryRetrying] = useState(false);
  const [commandChips, setCommandChips] = useState<string[]>(
    DEFAULT_QUICK_COMMANDS,
  );
  const [editingCommandChips, setEditingCommandChips] = useState(false);
  const [showTerminalActions, setShowTerminalActions] = useState(false);
  const [showFindBar, setShowFindBar] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findNoMatch, setFindNoMatch] = useState(false);
  const [actionToast, setActionToast] = useState<string | null>(null);
  const [mobileKeybarBottom, setMobileKeybarBottom] = useState(0);
  const [fastScrollOverflow, setFastScrollOverflow] = useState(false);
  const [fastScrollVisible, setFastScrollVisible] = useState(false);
  const [fastScrollDragging, setFastScrollDragging] = useState(false);
  const [fastScrollBoosted, setFastScrollBoosted] = useState(false);
  const [fastScrollThumbTop, setFastScrollThumbTop] = useState(0);
  const [fastScrollThumbHeight, setFastScrollThumbHeight] = useState(0);
  const [fastScrollAtBottom, setFastScrollAtBottom] = useState(true);
  const [copyDragActive, setCopyDragActive] = useState(false);
  const [skillChips, setSkillChips] = useState<SkillSummary[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState<string | null>(null);
  const [skillsReloadSeq, setSkillsReloadSeq] = useState(0);
  const [skillChipHoldName, setSkillChipHoldName] = useState<string | null>(null);
  const sessionToolKind = classifySessionTool(session.tool);
  const isAgentSession =
    sessionToolKind === "claude" || sessionToolKind === "codex";

  useEffect(() => {
    sessionStateRef.current = session.state;
  }, [session.state]);

  useEffect(() => {
    setTitle(`tmux a -t ${session.tmux_name}`);
  }, [session.tmux_name]);

  useEffect(() => {
    const stored = readQuickCommandMap()[session.session_id];
    setCommandChips(stored && stored.length > 0 ? stored : DEFAULT_QUICK_COMMANDS);
    setSkillChips([]);
    setSkillsLoading(false);
    setSkillsError(null);
    setSkillsReloadSeq(0);
    setSkillChipHoldName(null);
    setEditingCommandChips(false);
    setShowTerminalActions(false);
    setShowFindBar(false);
    setFindNoMatch(false);
    inputEncodingStateRef.current.pendingHighSurrogate = "";
    viewportRef.current = null;
    setFastScrollOverflow(false);
    setFastScrollVisible(false);
    setFastScrollDragging(false);
    setFastScrollBoosted(false);
    setFastScrollThumbTop(0);
    setFastScrollThumbHeight(0);
    setFastScrollAtBottom(true);
    setCopyDragActive(false);
    fastScrollVelocityRef.current = 0;
    fastScrollLastSampleRef.current = null;
    copyDragActiveRef.current = false;
    copyDragAnchorRowRef.current = null;
    copyDragPointerIdRef.current = null;
    copyDragLastClientYRef.current = null;
    copyDragEdgeDirectionRef.current = 0;
    if (copyDragAutoScrollTimerRef.current) {
      clearInterval(copyDragAutoScrollTimerRef.current);
      copyDragAutoScrollTimerRef.current = null;
    }
  }, [session.session_id]);

  useEffect(() => {
    const viewport = window.visualViewport;
    if (!viewport) return;

    const updateBottomOffset = () => {
      const overlap = Math.max(
        0,
        window.innerHeight - (viewport.height + viewport.offsetTop),
      );
      setMobileKeybarBottom(Math.round(overlap));
    };

    updateBottomOffset();
    viewport.addEventListener("resize", updateBottomOffset);
    viewport.addEventListener("scroll", updateBottomOffset);
    window.addEventListener("orientationchange", updateBottomOffset);

    return () => {
      viewport.removeEventListener("resize", updateBottomOffset);
      viewport.removeEventListener("scroll", updateBottomOffset);
      window.removeEventListener("orientationchange", updateBottomOffset);
    };
  }, []);

  useEffect(() => {
    if (!observer) return;
    if (!copyDragActiveRef.current) return;
    copyDragActiveRef.current = false;
    setCopyDragActive(false);
    copyDragAnchorRowRef.current = null;
    copyDragPointerIdRef.current = null;
    copyDragLastClientYRef.current = null;
    copyDragEdgeDirectionRef.current = 0;
    if (copyDragAutoScrollTimerRef.current) {
      clearInterval(copyDragAutoScrollTimerRef.current);
      copyDragAutoScrollTimerRef.current = null;
    }
  }, [observer]);

  useEffect(() => {
    return () => {
      if (actionToastTimerRef.current) {
        clearTimeout(actionToastTimerRef.current);
      }
      if (longPressTimerRef.current) {
        clearTimeout(longPressTimerRef.current);
      }
      if (skillLongPressTimerRef.current) {
        clearTimeout(skillLongPressTimerRef.current);
      }
      if (fastScrollHideTimerRef.current) {
        clearTimeout(fastScrollHideTimerRef.current);
      }
      if (fastScrollRafRef.current !== null) {
        cancelAnimationFrame(fastScrollRafRef.current);
      }
      if (fastScrollDragCleanupRef.current) {
        fastScrollDragCleanupRef.current();
        fastScrollDragCleanupRef.current = null;
      }
      if (copyDragAutoScrollTimerRef.current) {
        clearInterval(copyDragAutoScrollTimerRef.current);
        copyDragAutoScrollTimerRef.current = null;
      }
      fastScrollVelocityRef.current = 0;
      fastScrollLastSampleRef.current = null;
      copyDragActiveRef.current = false;
      copyDragAnchorRowRef.current = null;
      copyDragPointerIdRef.current = null;
      copyDragLastClientYRef.current = null;
      copyDragEdgeDirectionRef.current = 0;
    };
  }, []);

  useEffect(() => {
    if (!isAgentSession) {
      setSkillChips([]);
      setSkillsLoading(false);
      setSkillsError(null);
      setSkillsReloadSeq(0);
      return;
    }

    const attempt = skillsReloadSeq;
    const forceRefresh = attempt > 0;
    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;
    setSkillsError(null);

    if (!forceRefresh) {
      const cached = SKILL_REGISTRY_CACHE[sessionToolKind];
      if (cached) {
        setSkillChips(cached);
        setSkillsLoading(false);
        return;
      }
    }

    setSkillsLoading(true);
    loadSkillRegistry(sessionToolKind, forceRefresh)
      .then((skills) => {
        if (cancelled) return;
        setSkillChips(skills);
        setSkillsLoading(false);
      })
      .catch(() => {
        if (cancelled) return;
        setSkillChips([]);
        setSkillsLoading(false);
        setSkillsError(`skills unavailable for ${sessionToolKind}`);
        const retryDelayMs = SKILL_AUTO_RETRY_DELAYS_MS[attempt];
        if (retryDelayMs !== undefined) {
          retryTimer = setTimeout(() => {
            setSkillsReloadSeq((prev) => prev + 1);
          }, retryDelayMs);
        }
      });

    return () => {
      cancelled = true;
      if (retryTimer) {
        clearTimeout(retryTimer);
      }
    };
  }, [isAgentSession, sessionToolKind, skillsReloadSeq]);

  const pushActionToast = useCallback((message: string) => {
    setActionToast(message);
    if (actionToastTimerRef.current) {
      clearTimeout(actionToastTimerRef.current);
    }
    actionToastTimerRef.current = setTimeout(() => {
      setActionToast(null);
      actionToastTimerRef.current = null;
    }, ACTION_TOAST_MS);
  }, []);

  const noteFastScrollVelocity = useCallback((scrollTop: number) => {
    const now = performance.now();
    const prev = fastScrollLastSampleRef.current;
    if (!prev) {
      fastScrollVelocityRef.current = 0;
      fastScrollLastSampleRef.current = { top: scrollTop, at: now };
      return;
    }

    const dt = Math.max(1, now - prev.at);
    const velocity = Math.abs(scrollTop - prev.top) / dt;
    fastScrollVelocityRef.current = velocity;
    fastScrollLastSampleRef.current = { top: scrollTop, at: now };
    const boosted = velocity >= FAST_SCROLL_BOOST_THRESHOLD;
    setFastScrollBoosted((current) => (current === boosted ? current : boosted));
  }, []);

  const clearFastScrollHideTimer = useCallback(() => {
    if (fastScrollHideTimerRef.current) {
      clearTimeout(fastScrollHideTimerRef.current);
      fastScrollHideTimerRef.current = null;
    }
  }, []);

  const scheduleFastScrollHide = useCallback(() => {
    if (observer) return;
    clearFastScrollHideTimer();
    const velocityBonus = Math.min(
      1200,
      Math.round(fastScrollVelocityRef.current * 700),
    );
    const hideDelay = FAST_SCROLL_HIDE_MS + velocityBonus;
    fastScrollHideTimerRef.current = setTimeout(() => {
      fastScrollHideTimerRef.current = null;
      if (fastScrollDragRef.current) return;
      setFastScrollVisible(false);
      setFastScrollBoosted(false);
      fastScrollVelocityRef.current = 0;
    }, hideDelay);
  }, [observer, clearFastScrollHideTimer]);

  const refreshFastScrollUi = useCallback(
    (show = false) => {
      const viewport = viewportRef.current;
      if (!viewport || observer) {
        setFastScrollOverflow(false);
        setFastScrollVisible(false);
        setFastScrollBoosted(false);
        setFastScrollAtBottom(true);
        setFastScrollThumbTop(0);
        setFastScrollThumbHeight(0);
        return;
      }

      const scrollTop = viewport.scrollTop;
      const scrollHeight = viewport.scrollHeight;
      const clientHeight = viewport.clientHeight;
      const overflow = hasFastScrollOverflow(scrollHeight, clientHeight);
      setFastScrollOverflow(overflow);
      setFastScrollAtBottom(
        isNearScrollBottom(scrollTop, scrollHeight, clientHeight),
      );

      if (!overflow) {
        setFastScrollVisible(false);
        setFastScrollBoosted(false);
        setFastScrollThumbTop(0);
        setFastScrollThumbHeight(0);
        return;
      }

      const trackHeight = Math.max(1, clientHeight);
      const thumbHeight = computeFastScrollThumbHeight(
        clientHeight,
        scrollHeight,
        trackHeight,
        FAST_SCROLL_MIN_THUMB_PX,
      );
      const thumbTop = computeFastScrollThumbTop(
        scrollTop,
        scrollHeight,
        clientHeight,
        trackHeight,
        thumbHeight,
      );
      setFastScrollThumbHeight(thumbHeight);
      setFastScrollThumbTop(thumbTop);

      if (show || fastScrollDragRef.current) {
        setFastScrollVisible(true);
        if (!fastScrollDragRef.current) {
          scheduleFastScrollHide();
        }
      }
    },
    [observer, scheduleFastScrollHide],
  );

  const scheduleRefreshFastScrollUi = useCallback(
    (show = false) => {
      if (show) {
        fastScrollShowPendingRef.current = true;
      }
      if (fastScrollRafRef.current !== null) return;
      fastScrollRafRef.current = window.requestAnimationFrame(() => {
        fastScrollRafRef.current = null;
        const shouldShow = fastScrollShowPendingRef.current;
        fastScrollShowPendingRef.current = false;
        refreshFastScrollUi(show || shouldShow);
      });
    },
    [refreshFastScrollUi],
  );

  const clearCopyDragAutoScroll = useCallback(() => {
    if (copyDragAutoScrollTimerRef.current) {
      clearInterval(copyDragAutoScrollTimerRef.current);
      copyDragAutoScrollTimerRef.current = null;
    }
  }, []);

  const clearCopyDragGesture = useCallback(() => {
    copyDragAnchorRowRef.current = null;
    copyDragPointerIdRef.current = null;
    copyDragLastClientYRef.current = null;
    copyDragEdgeDirectionRef.current = 0;
    clearCopyDragAutoScroll();
  }, [clearCopyDragAutoScroll]);

  const resolveCopyDragBufferRow = useCallback((clientY: number): number | null => {
    const term = termRef.current;
    const viewport = viewportRef.current;
    if (!term || !viewport) return null;
    const rect = viewport.getBoundingClientRect();
    return mapClientYToBufferRow(
      clientY,
      rect.top,
      viewport.clientHeight,
      term.rows,
      term.buffer.active.viewportY,
      term.buffer.active.length,
    );
  }, []);

  const updateCopyDragSelection = useCallback(
    (clientY: number): boolean => {
      const term = termRef.current;
      const viewport = viewportRef.current;
      const anchorRow = copyDragAnchorRowRef.current;
      if (!term || !viewport || anchorRow === null) return false;
      const row = resolveCopyDragBufferRow(clientY);
      if (row === null) return false;
      term.selectLines(anchorRow, row);
      copyDragLastClientYRef.current = clientY;
      scheduleRefreshFastScrollUi(true);
      return true;
    },
    [resolveCopyDragBufferRow, scheduleRefreshFastScrollUi],
  );

  const ensureCopyDragAutoScroll = useCallback(() => {
    if (copyDragAutoScrollTimerRef.current) return;
    copyDragAutoScrollTimerRef.current = setInterval(() => {
      if (!copyDragActiveRef.current) {
        clearCopyDragAutoScroll();
        return;
      }
      const direction = copyDragEdgeDirectionRef.current;
      if (direction === 0) {
        clearCopyDragAutoScroll();
        return;
      }
      const viewport = viewportRef.current;
      const term = termRef.current;
      if (!viewport || !term) {
        clearCopyDragAutoScroll();
        return;
      }
      const rowHeight = Math.max(1, viewport.clientHeight / Math.max(1, term.rows));
      const before = viewport.scrollTop;
      viewport.scrollTop = before + direction * rowHeight * COPY_DRAG_STEP_MULTIPLIER;
      if (viewport.scrollTop === before) return;
      noteFastScrollVelocity(viewport.scrollTop);
      scheduleRefreshFastScrollUi(true);
      const clientY = copyDragLastClientYRef.current;
      if (clientY !== null) {
        updateCopyDragSelection(clientY);
      }
    }, COPY_DRAG_AUTOSCROLL_MS);
  }, [
    clearCopyDragAutoScroll,
    noteFastScrollVelocity,
    scheduleRefreshFastScrollUi,
    updateCopyDragSelection,
  ]);

  const updateCopyDragEdgeDirection = useCallback(
    (clientY: number) => {
      const viewport = viewportRef.current;
      if (!viewport) return;
      const rect = viewport.getBoundingClientRect();
      const edgePx = Math.max(
        COPY_DRAG_EDGE_MIN_PX,
        Math.min(
          COPY_DRAG_EDGE_MAX_PX,
          Math.round(viewport.clientHeight * COPY_DRAG_EDGE_RATIO),
        ),
      );
      const direction = computeCopyDragEdgeDirection(
        clientY,
        rect.top,
        rect.bottom,
        edgePx,
      );
      copyDragEdgeDirectionRef.current = direction;
      if (direction === 0) {
        clearCopyDragAutoScroll();
      } else {
        ensureCopyDragAutoScroll();
      }
    },
    [clearCopyDragAutoScroll, ensureCopyDragAutoScroll],
  );

  const startCopyDragSelection = useCallback(
    (clientY: number, pointerId?: number): boolean => {
      if (!copyDragActiveRef.current) return false;
      const anchorRow = resolveCopyDragBufferRow(clientY);
      const term = termRef.current;
      if (anchorRow === null || !term) return false;
      copyDragAnchorRowRef.current = anchorRow;
      copyDragPointerIdRef.current =
        typeof pointerId === "number" ? pointerId : null;
      copyDragLastClientYRef.current = clientY;
      term.selectLines(anchorRow, anchorRow);
      updateCopyDragEdgeDirection(clientY);
      scheduleRefreshFastScrollUi(true);
      return true;
    },
    [
      resolveCopyDragBufferRow,
      scheduleRefreshFastScrollUi,
      updateCopyDragEdgeDirection,
    ],
  );

  const moveCopyDragSelection = useCallback(
    (clientY: number, pointerId?: number): boolean => {
      if (!copyDragActiveRef.current) return false;
      const activePointerId = copyDragPointerIdRef.current;
      if (
        typeof pointerId === "number" &&
        activePointerId !== null &&
        pointerId !== activePointerId
      ) {
        return false;
      }
      const moved = updateCopyDragSelection(clientY);
      if (!moved) return false;
      updateCopyDragEdgeDirection(clientY);
      return true;
    },
    [updateCopyDragEdgeDirection, updateCopyDragSelection],
  );

  const endCopyDragSelection = useCallback(
    (pointerId?: number): void => {
      const activePointerId = copyDragPointerIdRef.current;
      if (
        typeof pointerId === "number" &&
        activePointerId !== null &&
        pointerId !== activePointerId
      ) {
        return;
      }
      clearCopyDragGesture();
    },
    [clearCopyDragGesture],
  );

  const stopCopyDragMode = useCallback(() => {
    copyDragActiveRef.current = false;
    setCopyDragActive(false);
    clearCopyDragGesture();
  }, [clearCopyDragGesture]);

  const startCopyDragMode = useCallback(() => {
    if (observer) return;
    copyDragActiveRef.current = true;
    setCopyDragActive(true);
    setShowTerminalActions(false);
    pushActionToast("Drag to select. Hold near top/bottom to auto-scroll.");
  }, [observer, pushActionToast]);

  const toggleCopyDragMode = useCallback(() => {
    if (copyDragActiveRef.current) {
      stopCopyDragMode();
      return;
    }
    startCopyDragMode();
  }, [startCopyDragMode, stopCopyDragMode]);

  const persistCommandChips = useCallback(
    (next: string[]) => {
      const sanitized = sanitizeQuickCommands(next);
      setCommandChips(sanitized);
      const map = readQuickCommandMap();
      if (sanitized.length > 0) {
        map[session.session_id] = sanitized;
      } else {
        delete map[session.session_id];
      }
      writeQuickCommandMap(map);
    },
    [session.session_id],
  );

  const sendInput = useCallback(
    (data: string) => {
      if (observer || !data) return;
      const encoded = encodeTerminalInputChunk(
        data,
        inputEncodingStateRef.current,
        encoder,
      );
      if (!encoded || encoded.length === 0) return;
      realtime.sendInput(session.session_id, encoded);
      termRef.current?.focus();
    },
    [observer, session.session_id],
  );

  const pasteInput = useCallback(
    (text: string) => {
      if (observer || !text) return;
      const term = termRef.current;
      if (term) {
        term.paste(text);
        term.focus();
        return;
      }
      sendInput(text);
    },
    [observer, sendInput],
  );

  const clearLongPress = useCallback(() => {
    if (longPressTimerRef.current) {
      clearTimeout(longPressTimerRef.current);
      longPressTimerRef.current = null;
    }
    longPressStartRef.current = null;
  }, []);

  useEffect(() => {
    if (initDoneRef.current) return;
    initDoneRef.current = true;
    snapshotReadyRef.current = false;
    pendingFramesRef.current = [];
    autoRecoveryKeyRef.current = null;
    setRecoveryBanner(null);
    setRecoveryRetrying(false);
    setLifecycleState("attaching");

    const container = containerRef.current;
    if (!container) return;
    let disposed = false;

    let term: Terminal;
    let fitAddon: FitAddon;
    let hostEl: HTMLDivElement;

    const flushPendingFrames = () => {
      for (const frame of pendingFramesRef.current) {
        if (frame.seq > seqRef.current) {
          seqRef.current = frame.seq;
          term.write(frame.data);
        }
      }
      pendingFramesRef.current = [];
      scheduleRefreshFastScrollUi(false);
    };

    const SILENCE_TIMEOUT_MS = 5000;

    const startSilenceTimer = () => {
      if (disposed) return;
      if (!snapshotReadyRef.current) return;
      silenceTimerRef.current = setTimeout(() => {
        silenceTimerRef.current = null;
        if (disposed) return;
        if (sessionStateRef.current !== "busy") return;
        // Session is busy but no output for 5s — likely evicted subscriber.
        warn_silence_recovery(session.session_id);
        realtime.forceResubscribe(session.session_id);
        void recoverFromSnapshot();
      }, SILENCE_TIMEOUT_MS);
    };

    const resetSilenceTimer = () => {
      if (silenceTimerRef.current) {
        clearTimeout(silenceTimerRef.current);
        silenceTimerRef.current = null;
      }
      startSilenceTimer();
    };

    const markLive = () => {
      if (!disposed && snapshotReadyRef.current) {
        setLifecycleState("live");
        resetSilenceTimer();
      }
    };

    const recoverFromSnapshot = async () => {
      if (disposed) return;
      setLifecycleState("snapshot_or_replay");
      setRecoveryRetrying(true);
      try {
        const snapshot = await fetchSnapshot(session.session_id);
        if (disposed) return;
        seqRef.current = snapshot.latest_seq;
        snapshotReadyRef.current = true;
        term.clear();
        if (snapshot.screen_text) {
          term.write(snapshot.screen_text);
        }
        flushPendingFrames();
        setRecoveryBanner(null);
        setRecoveryRetrying(false);
        markLive();
        scheduleRefreshFastScrollUi(false);

        // Nudge the PTY size to force tmux to emit a full-screen ANSI
        // redraw, replacing the plain-text snapshot with properly
        // formatted output (colors, cursor positioning, TUI layout).
        const cols = term.cols;
        const rows = term.rows;
        realtime.sendResize(session.session_id, cols + 1, rows);
        setTimeout(() => {
          if (disposed) return;
          realtime.sendResize(session.session_id, cols, rows);
        }, 100);
      } catch {
        if (disposed) return;
        setRecoveryBanner(
          "Replay recovery failed. Retry snapshot to re-sync this pane.",
        );
        setRecoveryRetrying(false);
      }
    };

    recoverFromSnapshotRef.current = recoverFromSnapshot;

    if (cached) {
      term = cached.term;
      fitAddon = cached.fitAddon;
      hostEl = cached.hostEl;
      seqRef.current =
        Number.isFinite(cached.latestSeq) && cached.latestSeq > 0
          ? Math.floor(cached.latestSeq)
          : 0;
      snapshotReadyRef.current = true;
      container.appendChild(hostEl);
      setLifecycleState("snapshot_or_replay");
      realtime.subscribeSession(session.session_id, seqRef.current);
      realtime.sendResize(session.session_id, term.cols, term.rows);
      fitAddon.fit();
      term.focus();
      markLive();
      scheduleRefreshFastScrollUi(false);
    } else {
      hostEl = document.createElement("div");
      hostEl.className = "term-host";
      hostEl.style.width = "100%";
      hostEl.style.height = "100%";
      container.appendChild(hostEl);

      term = new Terminal({
        theme: {
          background: "#1a1a2e",
          foreground: "#e0e0e0",
          cursor: "#e0e0e0",
          cursorAccent: "#1a1a2e",
          selectionBackground: "rgba(255,255,255,0.2)",
        },
        fontFamily: 'Menlo, Monaco, "Courier New", monospace',
        fontSize: 14,
        scrollback: 5000,
        cursorBlink: true,
      });

      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.open(hostEl);

      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        term.loadAddon(webgl);
      } catch {
        // WebGL not available, software renderer is fine.
      }

      const textarea = hostEl.querySelector("textarea");
      if (textarea) {
        textarea.setAttribute("autocapitalize", "off");
        textarea.setAttribute("autocorrect", "off");
        textarea.setAttribute("autocomplete", "off");
        textarea.setAttribute("spellcheck", "false");
      }

      fitAddon.fit();

      setLifecycleState("snapshot_or_replay");
      realtime.subscribeSession(session.session_id);
      realtime.sendResize(session.session_id, term.cols, term.rows);

      fetchSnapshot(session.session_id)
        .then((snapshot) => {
          if (disposed) return;
          if (snapshot.screen_text) {
            term.write(snapshot.screen_text);
          }
          seqRef.current = snapshot.latest_seq;
          snapshotReadyRef.current = true;
          flushPendingFrames();
          markLive();
          scheduleRefreshFastScrollUi(false);

          // Nudge PTY size to force tmux full-screen ANSI redraw.
          // The plain-text snapshot lacks escape sequences; this
          // generates new frames with seq > latest_seq that replace it.
          const c = term.cols;
          const r = term.rows;
          realtime.sendResize(session.session_id, c + 1, r);
          setTimeout(() => {
            if (disposed) return;
            realtime.sendResize(session.session_id, c, r);
          }, 100);
        })
        .catch(() => {
          if (disposed) return;
          snapshotReadyRef.current = true;
          flushPendingFrames();
          markLive();
          scheduleRefreshFastScrollUi(false);
        });

      focusTimerRef.current = setTimeout(() => {
        if (disposed) return;
        fitAddon.fit();
        term.focus();
      }, 350);
    }

    termRef.current = term;
    fitAddonRef.current = fitAddon;
    searchAddonRef.current = ensureSearchAddon(term);
    term.attachCustomKeyEventHandler((event: KeyboardEvent) => {
      const shortcutAction = resolveTerminalShortcutAction(event, observer);
      if (shortcutAction === "copy") {
        const selected = term.getSelection();
        if (!selected) return true;
        event.preventDefault();
        void copyTextToClipboard(selected).then((copied) => {
          pushActionToast(copied ? "Copied" : "Clipboard write failed");
        });
        return false;
      }
      if (shortcutAction === "block_paste") {
        return false;
      }
      if (shortcutAction === "native_paste") {
        // Let browser/xterm handle keyboard paste natively for reliability.
        return true;
      }
      return true;
    });

    const handlePasteEvent = (event: ClipboardEvent) => {
      if (observer) {
        event.preventDefault();
        return;
      }
      const text = event.clipboardData?.getData("text");
      if (!text) return;
      event.preventDefault();
      pasteInput(text);
      pushActionToast("Pasted");
    };
    hostEl.addEventListener("paste", handlePasteEvent as EventListener);

    // iOS Safari can fail to propagate natural swipe scrolling into xterm's
    // internal viewport. Bridge one-finger touch gestures to viewport.scrollTop.
    const queryViewport = () =>
      hostEl.querySelector(".xterm-viewport") as HTMLElement | null;
    let viewportEl: HTMLElement | null = null;
    let viewportResizeObserver: ResizeObserver | null = null;
    let viewportAttachRetryTimer: ReturnType<typeof setTimeout> | null = null;
    const handleViewportScroll = () => {
      if (viewportRef.current) {
        noteFastScrollVelocity(viewportRef.current.scrollTop);
      }
      scheduleRefreshFastScrollUi(true);
    };
    const attachViewport = (): boolean => {
      const viewport = queryViewport();
      if (!viewport) return false;
      viewportEl = viewport;
      viewportRef.current = viewport;
      viewport.addEventListener("scroll", handleViewportScroll, { passive: true });
      viewportResizeObserver = new ResizeObserver(() => {
        scheduleRefreshFastScrollUi(false);
      });
      viewportResizeObserver.observe(viewport);
      scheduleRefreshFastScrollUi(false);
      return true;
    };
    if (!attachViewport()) {
      viewportAttachRetryTimer = setTimeout(() => {
        if (disposed) return;
        attachViewport();
      }, 0);
    }

    let touchViewport: HTMLElement | null = null;
    const handleTouchScrollStart = (event: TouchEvent) => {
      if (observer) return;
      if (copyDragActiveRef.current) {
        if (event.touches.length !== 1) {
          endCopyDragSelection();
          return;
        }
        const touch = event.touches[0];
        if (!touch) return;
        if (startCopyDragSelection(touch.clientY)) {
          event.preventDefault();
          event.stopPropagation();
        }
        return;
      }
      if (event.touches.length !== 1) {
        endTouchScroll(touchScrollStateRef.current);
        touchViewport = null;
        return;
      }
      const viewport = queryViewport();
      if (!viewport) return;
      const touch = event.touches[0];
      touchViewport = viewport;
      beginTouchScroll(
        touchScrollStateRef.current,
        touch.clientY,
        viewport.scrollTop,
      );
    };
    const handleTouchScrollMove = (event: TouchEvent) => {
      if (observer) return;
      if (copyDragActiveRef.current) {
        if (event.touches.length !== 1) return;
        const touch = event.touches[0];
        if (!touch) return;
        if (moveCopyDragSelection(touch.clientY)) {
          event.preventDefault();
          event.stopPropagation();
        }
        return;
      }
      if (event.touches.length !== 1) return;
      const viewport = touchViewport ?? queryViewport();
      if (!viewport) return;
      const touch = event.touches[0];
      const next = nextTouchScrollTop(
        touchScrollStateRef.current,
        touch.clientY,
      );
      if (next === null) return;
      const before = viewport.scrollTop;
      viewport.scrollTop = next;
      if (viewport.scrollTop !== before) {
        noteFastScrollVelocity(viewport.scrollTop);
        event.preventDefault();
        event.stopPropagation();
        scheduleRefreshFastScrollUi(true);
      }
    };
    const handleTouchScrollEnd = () => {
      if (copyDragActiveRef.current) {
        endCopyDragSelection();
        return;
      }
      endTouchScroll(touchScrollStateRef.current);
      touchViewport = null;
    };

    const handleCopyDragPointerDown = (event: globalThis.PointerEvent) => {
      if (observer || !copyDragActiveRef.current) return;
      if (event.pointerType === "touch") return;
      if (event.pointerType === "mouse" && event.button !== 0) return;
      if (startCopyDragSelection(event.clientY, event.pointerId)) {
        event.preventDefault();
        event.stopPropagation();
      }
    };

    const handleCopyDragPointerMove = (event: globalThis.PointerEvent) => {
      if (observer || !copyDragActiveRef.current) return;
      if (event.pointerType === "touch") return;
      if (moveCopyDragSelection(event.clientY, event.pointerId)) {
        event.preventDefault();
        event.stopPropagation();
      }
    };

    const handleCopyDragPointerEnd = (event: globalThis.PointerEvent) => {
      if (observer || !copyDragActiveRef.current) return;
      if (event.pointerType === "touch") return;
      endCopyDragSelection(event.pointerId);
    };

    if (!observer) {
      hostEl.addEventListener("touchstart", handleTouchScrollStart, {
        capture: true,
        passive: false,
      });
      hostEl.addEventListener("touchmove", handleTouchScrollMove, {
        capture: true,
        passive: false,
      });
      hostEl.addEventListener("touchend", handleTouchScrollEnd, {
        capture: true,
      });
      hostEl.addEventListener("touchcancel", handleTouchScrollEnd, {
        capture: true,
      });
      hostEl.addEventListener("pointerdown", handleCopyDragPointerDown, {
        capture: true,
      });
      hostEl.addEventListener("pointermove", handleCopyDragPointerMove, {
        capture: true,
        passive: false,
      });
      hostEl.addEventListener("pointerup", handleCopyDragPointerEnd, {
        capture: true,
      });
      hostEl.addEventListener("pointercancel", handleCopyDragPointerEnd, {
        capture: true,
      });
    }

    const handleOutput = (frame: TerminalOutputFrame) => {
      if (frame.sessionId !== session.session_id) return;
      if (!snapshotReadyRef.current) {
        pendingFramesRef.current.push(frame);
        return;
      }
      if (frame.seq <= seqRef.current) return;
      seqRef.current = frame.seq;
      term.write(frame.data);
      setLifecycleState("live");
      resetSilenceTimer();
      scheduleRefreshFastScrollUi(false);
    };

    const unsubscribeOutput = realtime.subscribeTerminalOutput(handleOutput);
    const unsubscribeReplay = realtime.subscribeReplayTruncated(
      async (sessionId, payload) => {
        if (sessionId !== session.session_id) return;
        const incidentKey = `${payload.requested_resume_from_seq}:${payload.replay_window_start_seq}:${payload.latest_seq}`;
        if (autoRecoveryKeyRef.current === incidentKey) return;
        autoRecoveryKeyRef.current = incidentKey;
        setLifecycleState("snapshot_or_replay");
        setRecoveryBanner("Replay gap detected. Attempting automatic recovery...");
        await recoverFromSnapshot();
      },
    );
    const unsubscribeSubscription = realtime.subscribeSessionSubscription(
      (sessionId, payload) => {
        if (sessionId !== session.session_id) return;
        if (payload.state === "subscribed") {
          setLifecycleState(
            snapshotReadyRef.current ? "live" : "snapshot_or_replay",
          );
        } else if (payload.state === "unsubscribed") {
          setLifecycleState("attaching");
        }
      },
    );

    let inputDisposable: { dispose: () => void } | null = null;
    if (!observer) {
      inputDisposable = term.onData((data: string) => {
        if (!data) return;
        sendInput(data);
      });
    }

    const resizeDisposable = term.onResize(({ cols, rows }) => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        realtime.sendResize(session.session_id, cols, rows);
      }, 100);
    });

    const handleWindowResize = () => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        if (fitAddonRef.current) fitAddonRef.current.fit();
        scheduleRefreshFastScrollUi(false);
      }, 100);
    };
    window.addEventListener("resize", handleWindowResize);
    if (window.visualViewport) {
      window.visualViewport.addEventListener("resize", handleWindowResize);
    }

    return () => {
      disposed = true;
      recoverFromSnapshotRef.current = null;
      if (silenceTimerRef.current) {
        clearTimeout(silenceTimerRef.current);
        silenceTimerRef.current = null;
      }
      clearLongPress();
      window.removeEventListener("resize", handleWindowResize);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener("resize", handleWindowResize);
      }
      inputDisposable?.dispose();
      resizeDisposable?.dispose();
      unsubscribeOutput();
      unsubscribeReplay();
      unsubscribeSubscription();
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      if (focusTimerRef.current) {
        clearTimeout(focusTimerRef.current);
        focusTimerRef.current = null;
      }
      realtime.unsubscribeSession(session.session_id);
      hostEl.removeEventListener("paste", handlePasteEvent as EventListener);
      if (viewportAttachRetryTimer) {
        clearTimeout(viewportAttachRetryTimer);
        viewportAttachRetryTimer = null;
      }
      if (viewportEl) {
        viewportEl.removeEventListener("scroll", handleViewportScroll);
      }
      if (viewportResizeObserver) {
        viewportResizeObserver.disconnect();
      }
      viewportRef.current = null;
      setFastScrollDragging(false);
      setFastScrollBoosted(false);
      fastScrollVelocityRef.current = 0;
      fastScrollLastSampleRef.current = null;
      clearCopyDragGesture();
      copyDragActiveRef.current = false;
      setCopyDragActive(false);
      if (!observer) {
        hostEl.removeEventListener("touchstart", handleTouchScrollStart, true);
        hostEl.removeEventListener("touchmove", handleTouchScrollMove, true);
        hostEl.removeEventListener("touchend", handleTouchScrollEnd, true);
        hostEl.removeEventListener("touchcancel", handleTouchScrollEnd, true);
        hostEl.removeEventListener("pointerdown", handleCopyDragPointerDown, true);
        hostEl.removeEventListener("pointermove", handleCopyDragPointerMove, true);
        hostEl.removeEventListener("pointerup", handleCopyDragPointerEnd, true);
        hostEl.removeEventListener(
          "pointercancel",
          handleCopyDragPointerEnd,
          true,
        );
      }

      if (hostEl.parentNode) hostEl.parentNode.removeChild(hostEl);
      onCache({
        term,
        fitAddon,
        hostEl,
        sessionId: session.session_id,
        latestSeq: seqRef.current,
      });
    };
  }, [session.session_id, observer]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !fitAddonRef.current) return;

    const resizeObserver = new ResizeObserver(() => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        if (fitAddonRef.current) fitAddonRef.current.fit();
        scheduleRefreshFastScrollUi(false);
      }, 100);
    });
    resizeObserver.observe(container);
    return () => resizeObserver.disconnect();
  }, [scheduleRefreshFastScrollUi]);

  useEffect(() => {
    if (!showFindBar) return;
    const timer = setTimeout(() => {
      findInputRef.current?.focus();
    }, 0);
    return () => clearTimeout(timer);
  }, [showFindBar]);

  useEffect(() => {
    if (session.state === "exited") {
      const timer = setTimeout(() => {
        onSessionExit(session.session_id);
      }, 600);
      return () => clearTimeout(timer);
    }
  }, [session.state, session.session_id, onSessionExit]);

  const handleClose = useCallback(() => {
    if (rushingOff) return;
    setRushingOff(true);
    setTimeout(() => onClose(), 200);
  }, [rushingOff, onClose]);

  const handleTitleClick = useCallback(() => {
    void copyTextToClipboard(title).then((copied) => {
      if (!copied) return;
      setTitleCopied(true);
      setTimeout(() => setTitleCopied(false), 800);
    });
  }, [title]);

  const handleManualRecovery = useCallback(() => {
    const recover = recoverFromSnapshotRef.current;
    if (!recover) return;
    void recover();
  }, []);

  const sendSkillInvocation = useCallback(
    (skillName: string, submit: boolean) => {
      if (!isAgentSession) return;
      const normalized = skillName.trim();
      if (!normalized) return;
      const prefix = skillInvocationPrefix(sessionToolKind);
      if (submit) {
        sendInput(`${prefix}${normalized}\r`);
      } else {
        sendInput(`${prefix}${normalized} `);
      }
    },
    [isAgentSession, sessionToolKind, sendInput],
  );

  const clearSkillPress = useCallback(() => {
    if (skillLongPressTimerRef.current) {
      clearTimeout(skillLongPressTimerRef.current);
      skillLongPressTimerRef.current = null;
    }
    skillPressRef.current = null;
    setSkillChipHoldName(null);
  }, []);

  const handleSkillChipPointerDown = useCallback(
    (skillName: string, e: PointerEvent) => {
      if (!isAgentSession) return;
      if (e.pointerType !== "touch" && e.pointerType !== "pen") return;
      clearSkillPress();
      skillPressRef.current = {
        skillName,
        pointerId: e.pointerId,
        startX: e.clientX,
        startY: e.clientY,
      };
      setSkillChipHoldName(skillName);
      skillLongPressTimerRef.current = setTimeout(() => {
        const active = skillPressRef.current;
        if (!active || active.skillName !== skillName) return;
        suppressSkillClickRef.current = {
          skillName,
          until: Date.now() + SKILL_CLICK_SUPPRESS_MS,
        };
        sendSkillInvocation(skillName, false);
        clearSkillPress();
      }, SKILL_LONG_PRESS_MS);
    },
    [isAgentSession, sendSkillInvocation, clearSkillPress],
  );

  const handleSkillChipPointerMove = useCallback(
    (e: PointerEvent) => {
      const active = skillPressRef.current;
      if (!active || active.pointerId !== e.pointerId) return;
      if (
        Math.abs(e.clientX - active.startX) > SKILL_LONG_PRESS_CANCEL_DISTANCE_PX ||
        Math.abs(e.clientY - active.startY) > SKILL_LONG_PRESS_CANCEL_DISTANCE_PX
      ) {
        clearSkillPress();
      }
    },
    [clearSkillPress],
  );

  const handleSkillChipPointerEnd = useCallback(
    (e: PointerEvent) => {
      const active = skillPressRef.current;
      if (!active || active.pointerId !== e.pointerId) return;
      clearSkillPress();
    },
    [clearSkillPress],
  );

  const handleSkillChipClick = useCallback(
    (skillName: string, e: MouseEvent) => {
      const suppressed = suppressSkillClickRef.current;
      if (suppressed) {
        if (Date.now() > suppressed.until) {
          suppressSkillClickRef.current = null;
        } else if (suppressed.skillName === skillName) {
          e.preventDefault();
          return;
        }
      }
      sendSkillInvocation(skillName, true);
    },
    [sendSkillInvocation],
  );

  const handleRefreshSkills = useCallback(() => {
    if (!isAgentSession) return;
    setSkillsReloadSeq((prev) => prev + 1);
  }, [isAgentSession]);

  const handleAddCommandChip = useCallback(() => {
    if (observer) return;
    const value = window.prompt("Add quick command", "");
    if (value === null) return;
    const trimmed = value.trim();
    if (!trimmed) return;
    persistCommandChips([...commandChips, trimmed]);
  }, [observer, commandChips, persistCommandChips]);

  const handleQuickCommandChipPress = useCallback(
    (index: number) => {
      const command = commandChips[index];
      if (!command) return;
      if (editingCommandChips) {
        const value = window.prompt(
          "Edit quick command (empty to delete)",
          command,
        );
        if (value === null) return;
        const trimmed = value.trim();
        if (!trimmed) {
          persistCommandChips(commandChips.filter((_, i) => i !== index));
          return;
        }
        const next = [...commandChips];
        next[index] = trimmed;
        persistCommandChips(next);
        return;
      }
      sendInput(`${command}\r`);
    },
    [commandChips, editingCommandChips, persistCommandChips, sendInput],
  );

  const handleMobileKeyPress = useCallback(
    (keyId: MobileKeyId) => {
      if (observer) return;
      const config = MOBILE_KEYS.find((item) => item.id === keyId);
      if (!config?.input) return;
      sendInput(config.input);
    },
    [observer, sendInput],
  );

  const handleTerminalTouchStart = useCallback(
    (e: TouchEvent) => {
      if (!observer) return;
      if (e.touches.length !== 1) return;
      const touch = e.touches[0];
      longPressStartRef.current = { x: touch.clientX, y: touch.clientY };
      if (longPressTimerRef.current) {
        clearTimeout(longPressTimerRef.current);
      }
      longPressTimerRef.current = setTimeout(() => {
        longPressTimerRef.current = null;
        setShowTerminalActions(true);
      }, LONG_PRESS_DELAY_MS);
    },
    [observer],
  );

  const handleTerminalTouchMove = useCallback(
    (e: TouchEvent) => {
      const start = longPressStartRef.current;
      if (!start || e.touches.length !== 1) return;
      const touch = e.touches[0];
      if (
        Math.abs(touch.clientX - start.x) > LONG_PRESS_CANCEL_DISTANCE_PX ||
        Math.abs(touch.clientY - start.y) > LONG_PRESS_CANCEL_DISTANCE_PX
      ) {
        clearLongPress();
      }
    },
    [clearLongPress],
  );

  const handleTerminalTouchEnd = useCallback(() => {
    clearLongPress();
  }, [clearLongPress]);

  const closeTerminalActions = useCallback(() => {
    setShowTerminalActions(false);
  }, []);

  const handleCopySelectionOnly = useCallback(async () => {
    const term = termRef.current;
    if (!term) return;
    const selected = term.getSelection();
    if (!selected) {
      pushActionToast("Drag to select text first");
      return;
    }
    const copied = await copyTextToClipboard(selected);
    if (!copied) {
      pushActionToast("Clipboard write failed");
      return;
    }
    stopCopyDragMode();
    pushActionToast("Copied");
  }, [pushActionToast, stopCopyDragMode]);

  const handleCopyAction = useCallback(async () => {
    const term = termRef.current;
    if (!term) return;
    let text = term.getSelection();
    if (!text) {
      term.selectAll();
      text = term.getSelection();
      term.clearSelection();
    }
    if (!text) {
      pushActionToast("Nothing to copy");
      return;
    }
    const copied = await copyTextToClipboard(text);
    if (copied) {
      setShowTerminalActions(false);
      if (copyDragActiveRef.current) {
        stopCopyDragMode();
      }
      pushActionToast("Copied");
    } else {
      pushActionToast("Clipboard write failed");
    }
  }, [pushActionToast, stopCopyDragMode]);

  const handlePasteAction = useCallback(async () => {
    if (observer) return;
    const text = await readTextFromClipboardWithFallback();
    if (!text) {
      pushActionToast("Paste canceled");
      return;
    }
    pasteInput(text);
    setShowTerminalActions(false);
    pushActionToast("Pasted");
  }, [observer, pasteInput, pushActionToast]);

  const handleSelectAllAction = useCallback(() => {
    const term = termRef.current;
    if (!term) return;
    term.selectAll();
    setShowTerminalActions(false);
    pushActionToast("Selected all");
  }, [pushActionToast]);

  const handleFindAction = useCallback(() => {
    stopCopyDragMode();
    setShowTerminalActions(false);
    setShowFindBar(true);
    setFindNoMatch(false);
  }, [stopCopyDragMode]);

  const handleClearAction = useCallback(() => {
    if (observer) return;
    stopCopyDragMode();
    sendInput("\x0c");
    setShowTerminalActions(false);
    pushActionToast("Sent Ctrl+L");
  }, [observer, sendInput, pushActionToast, stopCopyDragMode]);

  const handleJumpToLive = useCallback(() => {
    if (observer) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    viewport.scrollTop = viewport.scrollHeight;
    noteFastScrollVelocity(viewport.scrollTop);
    scheduleRefreshFastScrollUi(true);
    termRef.current?.focus();
  }, [observer, noteFastScrollVelocity, scheduleRefreshFastScrollUi]);

  const handleFastScrollThumbPointerDown = useCallback(
    (e: PointerEvent) => {
      if (observer || !fastScrollOverflow) return;
      const viewport = viewportRef.current;
      if (!viewport) return;
      e.preventDefault();
      e.stopPropagation();

      if (fastScrollDragCleanupRef.current) {
        fastScrollDragCleanupRef.current();
        fastScrollDragCleanupRef.current = null;
      }

      clearFastScrollHideTimer();
      setFastScrollVisible(true);
      setFastScrollDragging(true);
      fastScrollDragRef.current = {
        pointerId: e.pointerId,
        startY: e.clientY,
        startThumbTop: fastScrollThumbTop,
      };

      const handlePointerMove = (ev: globalThis.PointerEvent) => {
        const drag = fastScrollDragRef.current;
        const activeViewport = viewportRef.current;
        if (!drag || ev.pointerId !== drag.pointerId || !activeViewport) return;
        ev.preventDefault();
        const trackHeight = Math.max(1, activeViewport.clientHeight);
        const scrollTop = computeScrollTopFromThumbOffset(
          drag.startThumbTop + (ev.clientY - drag.startY),
          activeViewport.scrollHeight,
          activeViewport.clientHeight,
          trackHeight,
          fastScrollThumbHeight,
        );
        activeViewport.scrollTop = scrollTop;
        noteFastScrollVelocity(activeViewport.scrollTop);
        scheduleRefreshFastScrollUi(true);
      };

      const finish = (ev: globalThis.PointerEvent) => {
        const drag = fastScrollDragRef.current;
        if (!drag || ev.pointerId !== drag.pointerId) return;
        if (fastScrollDragCleanupRef.current) {
          fastScrollDragCleanupRef.current();
          fastScrollDragCleanupRef.current = null;
        }
        fastScrollDragRef.current = null;
        setFastScrollDragging(false);
        scheduleRefreshFastScrollUi(true);
        scheduleFastScrollHide();
      };

      window.addEventListener("pointermove", handlePointerMove, {
        passive: false,
      });
      window.addEventListener("pointerup", finish);
      window.addEventListener("pointercancel", finish);
      fastScrollDragCleanupRef.current = () => {
        window.removeEventListener("pointermove", handlePointerMove);
        window.removeEventListener("pointerup", finish);
        window.removeEventListener("pointercancel", finish);
      };
    },
    [
      observer,
      fastScrollOverflow,
      clearFastScrollHideTimer,
      fastScrollThumbTop,
      fastScrollThumbHeight,
      noteFastScrollVelocity,
      scheduleFastScrollHide,
      scheduleRefreshFastScrollUi,
    ],
  );

  const runFind = useCallback(
    (direction: "next" | "previous") => {
      const query = findQuery.trim();
      const search = searchAddonRef.current;
      if (!query || !search) return;
      const found =
        direction === "next"
          ? search.findNext(query)
          : search.findPrevious(query);
      setFindNoMatch(!found);
    },
    [findQuery],
  );

  const handleFindInput = useCallback((e: Event) => {
    const target = e.target as HTMLInputElement | null;
    setFindQuery(target?.value ?? "");
    setFindNoMatch(false);
  }, []);

  const handleFindInputKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        runFind(e.shiftKey ? "previous" : "next");
      } else if (e.key === "Escape") {
        e.preventDefault();
        setShowFindBar(false);
        termRef.current?.focus();
      }
    },
    [runFind],
  );

  const lifecycleLabel =
    lifecycleState === "attaching"
      ? "attaching"
      : lifecycleState === "snapshot_or_replay"
        ? "snapshot/replay"
        : "live";
  const skillPrefix = skillInvocationPrefix(sessionToolKind);
  const showFastScroll =
    !observer &&
    !copyDragActive &&
    fastScrollOverflow &&
    (fastScrollVisible || fastScrollDragging);
  const showJumpToLive =
    !observer && !copyDragActive && fastScrollOverflow && !fastScrollAtBottom;
  const isMobileViewport =
    typeof window !== "undefined" && window.innerWidth <= 768;
  const liveButtonBottom =
    observer || !isMobileViewport ? 12 : 64 + mobileKeybarBottom;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height: "100%",
        overflow: "hidden",
      }}
    >
      <header class={`zone-header ${rushingOff ? "rushing-off" : ""}`}>
        <div class="zone-sprite" onClick={handleClose}>
          <ThrongletSprite
            state={rushingOff ? "exited" : session.state}
            tool={session.tool}
            lastActivityAt={session.last_activity_at}
            spritePack={session.sprite_pack_id ? spritePacks.value[session.sprite_pack_id] ?? null : null}
          />
        </div>
        <span class="zone-name">{cwdLabel(session.cwd) || session.tmux_name}</span>
        <span class="zone-title" onClick={handleTitleClick}>
          {titleCopied ? "copied!" : title}
        </span>
        {onBenchToggle && !observer && (
          <button
            type="button"
            class={`zone-bench-toggle ${isBenched ? "benched" : ""}`}
            onClick={() => onBenchToggle(session.session_id)}
          >
            {isBenched ? "Show" : "Hide"}
          </button>
        )}
        <span
          style={{
            fontSize: "10px",
            letterSpacing: "0.04em",
            textTransform: "uppercase",
            opacity: lifecycleState === "live" ? 0.7 : 1,
          }}
        >
          {lifecycleLabel}
        </span>
        <span class={`zone-dot state-dot ${session.state}`} />
      </header>

      {recoveryBanner && (
        <div
          style={{
            background: "#E74C3C",
            color: "#fff",
            textAlign: "center",
            padding: "4px 8px",
            fontSize: "12px",
            fontWeight: 600,
            flexShrink: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            gap: "8px",
          }}
        >
          <span>{recoveryBanner}</span>
          <button
            type="button"
            onClick={handleManualRecovery}
            disabled={recoveryRetrying}
            style={{
              border: "1px solid rgba(255,255,255,0.5)",
              background: "rgba(0,0,0,0.2)",
              color: "#fff",
              fontSize: "11px",
              padding: "2px 8px",
              borderRadius: "4px",
              cursor: recoveryRetrying ? "not-allowed" : "pointer",
              opacity: recoveryRetrying ? 0.7 : 1,
            }}
          >
            {recoveryRetrying ? "recovering..." : "retry snapshot"}
          </button>
        </div>
      )}

      {observer && (
        <div
          style={{
            background: "#16213e",
            color: "#5BC0EB",
            textAlign: "center",
            padding: "2px 0",
            fontSize: "10px",
            fontWeight: 600,
            flexShrink: 0,
          }}
        >
          OBSERVER (read-only)
        </div>
      )}

      {!observer && isAgentSession && (
        <div class="quick-command-bar">
          {skillsLoading && <span class="quick-command-status">loading skills…</span>}
          {!skillsLoading && skillsError && (
            <span class="quick-command-status">{skillsError}</span>
          )}
          {!skillsLoading && !skillsError && skillChips.length === 0 && (
            <span class="quick-command-empty">No skills found</span>
          )}
          {skillChips.map((skill) => (
            <button
              key={`${session.session_id}-${skill.name}`}
              type="button"
              class={`quick-command-chip skill-chip ${skillChipHoldName === skill.name ? "holding" : ""}`}
              onClick={(e) => handleSkillChipClick(skill.name, e)}
              onPointerDown={(e) => handleSkillChipPointerDown(skill.name, e)}
              onPointerMove={handleSkillChipPointerMove}
              onPointerUp={handleSkillChipPointerEnd}
              onPointerCancel={handleSkillChipPointerEnd}
              onPointerLeave={handleSkillChipPointerEnd}
              title={
                skill.description
                  ? `${skillPrefix}${skill.name} — ${skill.description}`
                  : `Insert skill: ${skillPrefix}${skill.name}`
              }
            >
              {skillPrefix}
              {skill.name}
            </button>
          ))}
          <button
            type="button"
            class="quick-command-control"
            onClick={handleRefreshSkills}
          >
            refresh
          </button>
        </div>
      )}

      {!observer && !isAgentSession && (
        <div class="quick-command-bar">
          {commandChips.length === 0 && (
            <span class="quick-command-empty">No quick commands</span>
          )}
          {commandChips.map((command, index) => (
            <button
              key={`${session.session_id}-${index}-${command}`}
              type="button"
              class={`quick-command-chip ${editingCommandChips ? "editing" : ""}`}
              onClick={() => handleQuickCommandChipPress(index)}
              title={
                editingCommandChips
                  ? "Tap to edit or delete"
                  : `Run quick command: ${command}`
              }
            >
              {command}
            </button>
          ))}
          <button
            type="button"
            class="quick-command-control"
            onClick={handleAddCommandChip}
          >
            + cmd
          </button>
          <button
            type="button"
            class={`quick-command-control ${editingCommandChips ? "active" : ""}`}
            onClick={() => setEditingCommandChips((prev) => !prev)}
          >
            {editingCommandChips ? "done" : "edit"}
          </button>
        </div>
      )}

      {showFindBar && (
        <div class="terminal-find-bar">
          <input
            ref={findInputRef}
            type="text"
            value={findQuery}
            placeholder="Find in terminal"
            onInput={handleFindInput}
            onKeyDown={handleFindInputKeyDown}
          />
          <button type="button" onClick={() => runFind("previous")}>
            prev
          </button>
          <button type="button" onClick={() => runFind("next")}>
            next
          </button>
          <button
            type="button"
            onClick={() => {
              setShowFindBar(false);
              termRef.current?.focus();
            }}
          >
            close
          </button>
          {findNoMatch && <span class="terminal-find-status">no match</span>}
        </div>
      )}

      <div class="zone-terminal-stage">
        <div
          ref={containerRef}
          class={`zone-terminal ${copyDragActive ? "copy-drag-active" : ""}`}
          style={{ flex: 1, minHeight: 0 }}
          onTouchStart={observer ? handleTerminalTouchStart : undefined}
          onTouchMove={observer ? handleTerminalTouchMove : undefined}
          onTouchEnd={observer ? handleTerminalTouchEnd : undefined}
          onTouchCancel={observer ? handleTerminalTouchEnd : undefined}
          onContextMenu={(e: Event) => {
            e.preventDefault();
            setShowTerminalActions(true);
          }}
        />

        {showFastScroll && (
          <div
            class={`terminal-fast-scroll ${fastScrollDragging ? "dragging" : ""} ${fastScrollBoosted ? "boosted" : ""}`}
          >
            <div class="terminal-fast-scroll-track">
              <button
                type="button"
                class="terminal-fast-scroll-thumb"
                style={{
                  height: `${fastScrollThumbHeight}px`,
                  transform: `translate(-50%, ${fastScrollThumbTop}px)`,
                }}
                onPointerDown={handleFastScrollThumbPointerDown}
                aria-label="Fast scroll terminal history"
              >
                <span class="terminal-fast-scroll-grip" />
              </button>
            </div>
          </div>
        )}

        {showJumpToLive && (
          <button
            type="button"
            class="terminal-live-btn"
            style={{ bottom: `${liveButtonBottom}px` }}
            onClick={handleJumpToLive}
          >
            Live
          </button>
        )}

        {actionToast && <div class="terminal-action-toast">{actionToast}</div>}

        {copyDragActive && (
          <div class="terminal-copy-drag-hud">
            <span>Drag to select</span>
            <button type="button" onClick={() => void handleCopySelectionOnly()}>
              Copy
            </button>
            <button type="button" onClick={stopCopyDragMode}>
              Done
            </button>
          </div>
        )}

        {showTerminalActions && (
          <div class="terminal-actions-backdrop" onClick={closeTerminalActions}>
            <div
              class="terminal-actions-sheet"
              onClick={(e: Event) => e.stopPropagation()}
            >
              <button type="button" onClick={() => void handleCopyAction()}>
                Copy
              </button>
              <button
                type="button"
                onClick={() => void handlePasteAction()}
                disabled={observer}
              >
                Paste
              </button>
              <button type="button" onClick={handleSelectAllAction}>
                Select all
              </button>
              <button type="button" onClick={toggleCopyDragMode}>
                {copyDragActive ? "Exit drag copy" : "Drag copy"}
              </button>
              <button type="button" onClick={handleFindAction}>
                Find
              </button>
              <button
                type="button"
                onClick={handleClearAction}
                disabled={observer}
              >
                Clear
              </button>
              <button type="button" onClick={closeTerminalActions}>
                Close
              </button>
            </div>
          </div>
        )}
      </div>

      {!observer && <div class="mobile-keybar-spacer" />}

      {!observer && (
        <div class="mobile-keybar" style={{ bottom: `${mobileKeybarBottom}px` }}>
          {MOBILE_KEYS.map((key) => (
            <button
              key={key.id}
              type="button"
              class="mobile-keybar-btn"
              onClick={() => handleMobileKeyPress(key.id)}
            >
              {key.label}
            </button>
          ))}
          <button
            type="button"
            class={`mobile-keybar-btn ${copyDragActive ? "active" : ""}`}
            onClick={toggleCopyDragMode}
          >
            {copyDragActive ? "Done copy" : "Drag copy"}
          </button>
          <button
            type="button"
            class="mobile-keybar-btn"
            onClick={() => setShowTerminalActions(true)}
          >
            Actions
          </button>
        </div>
      )}
    </div>
  );
}
