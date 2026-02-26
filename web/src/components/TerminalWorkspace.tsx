import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { SearchAddon } from "@xterm/addon-search";
import type { SessionSummary, SkillRegistryTool, SkillSummary } from "@/types";
import { realtime } from "@/app";
import type { TerminalOutputFrame } from "@/services/realtime";
import type { CachedTerminal } from "@/hooks/useTerminalCache";
import { fetchSnapshot, listSkills } from "@/services/api";
import { copyTextToClipboard, readTextFromClipboard } from "@/lib/clipboard";
import { ThrongletSprite } from "./ThrongletSprite";

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
const LONG_PRESS_DELAY_MS = 450;
const LONG_PRESS_CANCEL_DISTANCE_PX = 16;
const ACTION_TOAST_MS = 1200;

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
      const skills = resp.skills ?? [];
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

function isAccelShortcut(event: KeyboardEvent, key: string): boolean {
  return (
    (event.metaKey || event.ctrlKey) &&
    !event.altKey &&
    event.key.toLowerCase() === key
  );
}

export function TerminalWorkspace({
  session,
  cached,
  observer = false,
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
  const [skillChips, setSkillChips] = useState<SkillSummary[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState<string | null>(null);
  const [skillsReloadSeq, setSkillsReloadSeq] = useState(0);
  const [skillChipHoldName, setSkillChipHoldName] = useState<string | null>(null);
  const sessionToolKind = classifySessionTool(session.tool);
  const isAgentSession =
    sessionToolKind === "claude" || sessionToolKind === "codex";

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
      realtime.sendInput(session.session_id, encoder.encode(data));
      termRef.current?.focus();
    },
    [observer, session.session_id],
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
    };

    const markLive = () => {
      if (!disposed && snapshotReadyRef.current) {
        setLifecycleState("live");
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
        })
        .catch(() => {
          if (disposed) return;
          snapshotReadyRef.current = true;
          flushPendingFrames();
          markLive();
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
      if (isAccelShortcut(event, "c")) {
        const selected = term.getSelection();
        if (!selected) return true;
        event.preventDefault();
        void copyTextToClipboard(selected).then((copied) => {
          pushActionToast(copied ? "Copied" : "Clipboard write failed");
        });
        return false;
      }
      return true;
    });

    const scheduleAutoCopySelection = () => {
      setTimeout(() => {
        const selected = term.getSelection();
        if (!selected) return;
        void copyTextToClipboard(selected);
      }, 0);
    };
    hostEl.addEventListener("mouseup", scheduleAutoCopySelection);
    hostEl.addEventListener("touchend", scheduleAutoCopySelection);
    const handlePasteEvent = (event: ClipboardEvent) => {
      if (observer) return;
      const text = event.clipboardData?.getData("text");
      if (!text) return;
      event.preventDefault();
      sendInput(text);
      pushActionToast("Pasted");
    };
    hostEl.addEventListener("paste", handlePasteEvent as EventListener);

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
        realtime.sendInput(session.session_id, encoder.encode(data));
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
      }, 100);
    };
    window.addEventListener("resize", handleWindowResize);
    if (window.visualViewport) {
      window.visualViewport.addEventListener("resize", handleWindowResize);
    }

    return () => {
      disposed = true;
      recoverFromSnapshotRef.current = null;
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
      hostEl.removeEventListener("mouseup", scheduleAutoCopySelection);
      hostEl.removeEventListener("touchend", scheduleAutoCopySelection);
      hostEl.removeEventListener("paste", handlePasteEvent as EventListener);

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

    const observer = new ResizeObserver(() => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        if (fitAddonRef.current) fitAddonRef.current.fit();
      }, 100);
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

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

  const handleTerminalTouchStart = useCallback((e: TouchEvent) => {
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
  }, []);

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
      pushActionToast("Copied");
    } else {
      pushActionToast("Clipboard write failed");
    }
  }, [pushActionToast]);

  const handlePasteAction = useCallback(async () => {
    if (observer) return;
    try {
      const text = await readTextFromClipboard();
      if (!text) {
        pushActionToast("Clipboard is empty");
        return;
      }
      sendInput(text);
      setShowTerminalActions(false);
      pushActionToast("Pasted");
    } catch {
      pushActionToast("Clipboard read failed");
    }
  }, [observer, sendInput, pushActionToast]);

  const handleSelectAllAction = useCallback(() => {
    const term = termRef.current;
    if (!term) return;
    term.selectAll();
    setShowTerminalActions(false);
    pushActionToast("Selected all");
  }, [pushActionToast]);

  const handleFindAction = useCallback(() => {
    setShowTerminalActions(false);
    setShowFindBar(true);
    setFindNoMatch(false);
  }, []);

  const handleClearAction = useCallback(() => {
    if (observer) return;
    sendInput("\x0c");
    setShowTerminalActions(false);
    pushActionToast("Sent Ctrl+L");
  }, [observer, sendInput, pushActionToast]);

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
          />
        </div>
        <span class="zone-name">{cwdLabel(session.cwd) || session.tmux_name}</span>
        <span class="zone-title" onClick={handleTitleClick}>
          {titleCopied ? "copied!" : title}
        </span>
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
          class="zone-terminal"
          style={{ flex: 1, minHeight: 0 }}
          onTouchStart={handleTerminalTouchStart}
          onTouchMove={handleTerminalTouchMove}
          onTouchEnd={handleTerminalTouchEnd}
          onTouchCancel={handleTerminalTouchEnd}
          onContextMenu={(e: Event) => {
            e.preventDefault();
            setShowTerminalActions(true);
          }}
        />

        {actionToast && <div class="terminal-action-toast">{actionToast}</div>}

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
