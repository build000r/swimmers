import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import type { SessionSummary, SpawnTool, SpritePack } from "@/types";
import {
  throngletRestStageForSession,
  type ThrongletRestStage,
} from "@/lib/thronglet-motion";
import { spritePacks as spritePacksSignal } from "@/app";
import { SpawnMenu } from "./SpawnMenu";
import { ThoughtConfigPanel } from "./ThoughtConfigPanel";
import { BenchModal } from "./BenchModal";
import { ThrongletSprite } from "./ThrongletSprite";

// ---- Helpers ----

function cwdLabel(cwd: string): string {
  const trimmed = cwd.trim();
  if (!trimmed || trimmed === "/") return "";
  const parts = trimmed.replace(/\/+$/, "").split("/").filter(Boolean);
  if (parts.length === 0) return "";
  return parts[parts.length - 1];
}

function throngletName(session: SessionSummary): string {
  return cwdLabel(session.cwd) || session.tmux_name;
}

function gaugeColor(usageRatio: number): string {
  if (usageRatio >= 0.8) return "#E74C3C";
  if (usageRatio >= 0.5) return "#F5A623";
  return "#4FC08D";
}

function gaugeUsageRatio(tokenCount: number, contextLimit: number): number {
  if (!Number.isFinite(tokenCount) || !Number.isFinite(contextLimit) || contextLimit <= 0) {
    return 0;
  }
  return Math.max(0, Math.min(tokenCount / contextLimit, 1));
}

function gaugeSegments(usageRatio: number): number {
  const remaining = 1 - usageRatio;
  return Math.round(remaining * 8);
}

// ---- Wander / collision constants ----

const MIN_SEPARATION = 120;
const PUSH_STRENGTH = 60;
const WANDER_INTERVAL = 3000;
const WANDER_X = 100;
const WANDER_Y = 80;

interface MotionProfile {
  wanderXScale: number;
  wanderYScale: number;
  downwardBias: number;
  movable: boolean;
  allowSeparation: boolean;
}

const MOTION_BY_REST_STAGE: Record<ThrongletRestStage, MotionProfile> = {
  active: {
    wanderXScale: 1,
    wanderYScale: 1,
    downwardBias: 0,
    movable: true,
    allowSeparation: true,
  },
  drowsy: {
    wanderXScale: 0.3,
    wanderYScale: 0.25,
    downwardBias: 14,
    movable: true,
    allowSeparation: true,
  },
  sleeping: {
    wanderXScale: 0,
    wanderYScale: 0,
    downwardBias: 8,
    movable: true,
    allowSeparation: false,
  },
  deep_sleep: {
    wanderXScale: 0,
    wanderYScale: 0,
    downwardBias: 0,
    movable: false,
    allowSeparation: false,
  },
};

// ---- ThrongletEntity sub-component ----

interface ThrongletProps {
  session: SessionSummary;
  idlePreview?: string;
  spritePack?: SpritePack | null;
  x: number;
  y: number;
  axeArmed?: boolean;
  axeTargeted?: boolean;
  axeDimmed?: boolean;
  axeHit?: boolean;
  benchArmed?: boolean;
  benchTargeted?: boolean;
  benchDimmed?: boolean;
  compact?: boolean;
  rawMode?: boolean;
  onAxeHover?: (id: string | null) => void;
  onBenchHover?: (id: string | null) => void;
  onTap: (id: string) => void;
  onDragToBottom: (id: string) => void;
}

function ThrongletEntity({
  session,
  idlePreview,
  spritePack,
  x,
  y,
  axeArmed = false,
  axeTargeted = false,
  axeDimmed = false,
  axeHit = false,
  benchArmed = false,
  benchTargeted = false,
  benchDimmed = false,
  compact = false,
  rawMode = false,
  onAxeHover,
  onBenchHover,
  onTap,
  onDragToBottom,
}: ThrongletProps) {
  const elRef = useRef<HTMLDivElement>(null);
  const throngletSize = compact ? 60 : 80;
  const spriteHalf = throngletSize / 2;
  const isDraggingRef = useRef(false);
  const longPressedRef = useRef(false);
  const prevToolRef = useRef<string | null>(session.tool);
  const [isHatching, setIsHatching] = useState(false);
  const isExited = session.state === "exited";
  const isEgg = !isExited && !session.tool && !isHatching;
  const showEggSprite = !isExited && (isEgg || isHatching);

  // Hatch when tool is first detected (user typed codex/claude)
  useEffect(() => {
    if (prevToolRef.current === null && session.tool !== null) {
      setIsHatching(true);
      const timer = setTimeout(() => setIsHatching(false), 900);
      prevToolRef.current = session.tool;
      return () => clearTimeout(timer);
    }
    prevToolRef.current = session.tool;
  }, [session.tool]);

  // Long-press haptic on the thronglet itself
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleClick = useCallback(() => {
    if (longPressedRef.current || isDraggingRef.current) {
      longPressedRef.current = false;
      return;
    }
    onTap(session.session_id);
  }, [session.session_id, onTap]);

  // Desktop drag-left to assign to bottom pane
  const handleMouseDown = useCallback(
    (e: MouseEvent) => {
      if (axeArmed || benchArmed) return;
      if (e.button !== 0) return;
      e.preventDefault();
      const startX = e.clientX;
      const startY = e.clientY;
      let dragging = false;
      let ghost: HTMLImageElement | null = null;

      const onMouseMove = (me: MouseEvent) => {
        const dx = me.clientX - startX;
        const dy = me.clientY - startY;

        if (!dragging && dx < -10 && Math.abs(dx) > Math.abs(dy)) {
          dragging = true;
          isDraggingRef.current = true;
          const sprite = elRef.current?.querySelector(
            ".thronglet-sprite",
          ) as HTMLImageElement | null;
          if (sprite) {
            ghost = sprite.cloneNode(true) as HTMLImageElement;
            ghost.style.cssText =
              `position:fixed;pointer-events:none;z-index:9999;opacity:0.7;width:${throngletSize}px;height:${throngletSize}px;`;
            document.body.appendChild(ghost);
          }
        }

        if (dragging && ghost) {
          ghost.style.left = me.clientX - spriteHalf + "px";
          ghost.style.top = me.clientY - spriteHalf + "px";
        }
      };

      const onMouseUp = (me: MouseEvent) => {
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
        if (dragging) {
          if (ghost) {
            ghost.remove();
            ghost = null;
          }
          const dx = me.clientX - startX;
          if (dx < -100) {
            onDragToBottom(session.session_id);
          }
          setTimeout(() => {
            isDraggingRef.current = false;
          }, 50);
        }
      };

      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [axeArmed, benchArmed, session.session_id, onDragToBottom, throngletSize, spriteHalf],
  );

  // Thought / activity text
  let activityText = "";
  let thoughtText = "";
  let showBubble = false;
  const idlePreviewText = idlePreview?.trim() ?? "";
  const idlePreviewWords = idlePreviewText
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  const idlePreviewLooksPromptLike =
    /(?:^|\s)(?:\$|#|>|❯)\s*[a-z0-9_./~:-]/i.test(idlePreviewText) ||
    (idlePreviewWords.length > 0 &&
      idlePreviewWords.length <= 8 &&
      /^(git|npm|pnpm|yarn|cargo|python|python3|node|npx|uv|go|rustc|make|docker|kubectl|tmux|ls|cd|cat|sed|rg|grep|curl|wget|pytest|pip|bun|deno)$/i.test(
        idlePreviewWords[0] ?? "",
      ));
  const safeIdlePreviewText = idlePreviewLooksPromptLike ? "" : idlePreviewText;
  const normalizedThought = session.thought?.trim().toLowerCase() ?? "";
  const isSleepingThought =
    normalizedThought === "sleeping" || normalizedThought === "sleeping.";
  const showIdlePreview =
    !session.thought &&
    session.state === "idle" &&
    safeIdlePreviewText.length > 0 &&
    !isSleepingThought;

  if (showIdlePreview) {
    activityText = safeIdlePreviewText;
    thoughtText = safeIdlePreviewText;
    showBubble = true;
  } else if (session.thought) {
    activityText = session.thought;
    thoughtText = session.thought;
    showBubble = true;
  } else if (session.state === "error") {
    activityText = "error!";
    thoughtText = "!!!";
    showBubble = true;
  } else if (session.state === "attention") {
    activityText = "ready";
    thoughtText = "ready";
    showBubble = true;
  }

  const liveBubble = showBubble
    ? {
        text: thoughtText,
        isIdlePreview: showIdlePreview,
      }
    : null;

  const renderedBubble = liveBubble;
  const bubbleText = renderedBubble?.text ?? "";
  const bubbleIdlePreview = renderedBubble?.isIdlePreview ?? false;
  const showRenderedBubble = !!renderedBubble;

  const showGauge = Number.isFinite(session.context_limit) && session.context_limit > 0;
  const gaugeRatio = gaugeUsageRatio(session.token_count, session.context_limit);
  const gaugeFillSegments = gaugeSegments(gaugeRatio);
  const gaugeFillWidth = `${(gaugeFillSegments / 8) * 100}%`;
  const gaugePercentLeft = Math.round((1 - gaugeRatio) * 100);

  // Raw mode: show session data card instead of sprite
  if (rawMode) {
    return (
      <div
        ref={elRef}
        class="thronglet-raw"
        style={{ left: x + "px", top: y + "px" }}
        onClick={handleClick}
        onDragStart={(e: Event) => e.preventDefault()}
      >
        <div class="raw-state" data-state={session.state}>{session.state}</div>
        <div class="raw-row"><span class="raw-key">id</span> {session.session_id.slice(0, 8)}</div>
        <div class="raw-row"><span class="raw-key">tmux</span> {session.tmux_name}</div>
        <div class="raw-row"><span class="raw-key">cwd</span> {cwdLabel(session.cwd)}</div>
        {session.tool && <div class="raw-row"><span class="raw-key">tool</span> {session.tool}</div>}
        {session.current_command && <div class="raw-row"><span class="raw-key">cmd</span> {session.current_command}</div>}
        {session.thought && <div class="raw-row"><span class="raw-key">thought</span> {session.thought}</div>}
        {session.context_limit > 0 && (
          <div class="raw-row"><span class="raw-key">ctx</span> {session.token_count.toLocaleString()}/{session.context_limit.toLocaleString()}</div>
        )}
        <div class="raw-row"><span class="raw-key">health</span> {session.transport_health}</div>
        {session.is_stale && <div class="raw-row raw-stale">STALE</div>}
      </div>
    );
  }

  return (
    <div
      ref={elRef}
      class={`thronglet ${
        isExited ? "exited" : isEgg ? "egg" : isHatching ? "hatching-reveal" : session.state
      } ${axeTargeted ? "axe-targeted" : ""} ${axeDimmed ? "axe-dimmed" : ""} ${
        axeHit ? "axe-hit" : ""
      } ${benchTargeted ? "bench-targeted" : ""} ${benchDimmed ? "bench-dimmed" : ""}`}
      style={{
        "--thronglet-size": `${throngletSize}px`,
        left: x + "px",
        top: y + "px",
        boxShadow: axeTargeted
          ? "0 0 0 2px rgba(231, 76, 60, 0.95)"
          : benchTargeted
            ? "0 0 0 2px rgba(91, 148, 235, 0.95)"
            : "none",
        borderRadius: "12px",
      }}
      onClick={handleClick}
      onMouseDown={handleMouseDown}
      onMouseEnter={() => {
        if (axeArmed) onAxeHover?.(session.session_id);
        if (benchArmed) onBenchHover?.(session.session_id);
      }}
      onMouseLeave={() => {
        if (axeArmed) onAxeHover?.(null);
        if (benchArmed) onBenchHover?.(null);
      }}
      onTouchStart={(e: TouchEvent) => {
        if (axeArmed) {
          onAxeHover?.(session.session_id);
          return;
        }
        if (benchArmed) {
          onBenchHover?.(session.session_id);
          return;
        }
        longPressTimerRef.current = setTimeout(() => {
          longPressTimerRef.current = null;
          longPressedRef.current = true;
          if (navigator.vibrate) navigator.vibrate(30);
        }, 500);
      }}
      onTouchMove={() => {
        if (longPressTimerRef.current) {
          clearTimeout(longPressTimerRef.current);
          longPressTimerRef.current = null;
        }
      }}
      onTouchEnd={() => {
        if (longPressTimerRef.current) {
          clearTimeout(longPressTimerRef.current);
          longPressTimerRef.current = null;
        }
      }}
      onTouchCancel={() => {
        if (longPressTimerRef.current) {
          clearTimeout(longPressTimerRef.current);
          longPressTimerRef.current = null;
        }
        if (axeArmed) onAxeHover?.(null);
        if (benchArmed) onBenchHover?.(null);
      }}
      onDragStart={(e: Event) => e.preventDefault()}
    >
      {/* Egg sprite for unhatched / hatching sessions */}
      {showEggSprite && (
        <img
          class="egg-idle-sprite"
          src="/assets/egg.png"
          alt=""
        />
      )}

      {/* Thought bubble (hidden during egg state) */}
      {!isEgg && showRenderedBubble && (
        <div class={`thought-bubble ${bubbleIdlePreview ? "idle-preview" : ""}`}>
          <span class="thought-text">{bubbleText}</span>
          <div class="thought-circle thought-circle-lg" />
          <div class="thought-circle thought-circle-sm" />
        </div>
      )}

      {/* Sprite (not rendered during egg state) */}
      {!isEgg && (
        <ThrongletSprite
          class="thronglet-sprite"
          state={session.state}
          tool={session.tool}
          lastActivityAt={session.last_activity_at}
          spritePack={spritePack}
        />
      )}

      {/* Label */}
      <div class="thronglet-label">
        {session.last_skill && (
          <div
            class="thronglet-skill-pill"
            title={`Last invoked skill: ${session.last_skill}`}
          >
            {session.last_skill}
          </div>
        )}
        {showGauge && (
          <>
            <div class={`context-gauge${gaugeRatio >= 0.8 ? " critical" : ""}`}>
              <div
                class="context-gauge-fill"
                style={{
                  "--gauge-color": gaugeColor(gaugeRatio),
                  "--gauge-segments": gaugeFillSegments,
                  width: gaugeFillWidth,
                } as Record<string, string | number>}
              />
            </div>
            <div class="context-gauge-percent">{gaugePercentLeft}% left</div>
          </>
        )}
        {activityText && !showRenderedBubble && (
          <div class="thronglet-activity">{activityText}</div>
        )}
      </div>
    </div>
  );
}

// ---- HatchingEgg sub-component ----

type HatchPhase = "dropping" | "wobbling" | "done";

interface HatchingEggProps {
  x: number;
  y: number;
  phase: HatchPhase;
  onPhaseComplete: (completedPhase: HatchPhase) => void;
}

function HatchingEgg({ x, y, phase, onPhaseComplete }: HatchingEggProps) {
  const handleAnimationEnd = useCallback(() => {
    if (phase === "dropping") {
      onPhaseComplete("dropping");
    }
  }, [phase, onPhaseComplete]);

  const handleWobbleEnd = useCallback(() => {
    onPhaseComplete("wobbling");
  }, [onPhaseComplete]);

  if (phase === "done") return null;

  return (
    <div
      class="hatching-egg"
      style={{
        left: x - 40 + "px",
        top: y - 40 + "px",
      }}
    >
      <img
        class={`egg-sprite ${phase}`}
        src="/assets/egg.png"
        alt=""
        onAnimationEnd={phase === "wobbling" ? handleWobbleEnd : handleAnimationEnd}
      />
    </div>
  );
}

// ---- OverviewField ----

interface HatchState {
  id: number;
  x: number;
  y: number;
  phase: HatchPhase;
  sessionId: string | null;
  spawnStatus: "pending" | "success" | "failed";
}

interface OverviewFieldProps {
  sessions: SessionSummary[];
  idlePreviews?: Record<string, string>;
  observer?: boolean;
  compact?: boolean;
  axeTopOffset?: number;
  axeArmed?: boolean;
  benchArmed?: boolean;
  benchedIds?: Set<string>;
  onDisarmAxe?: () => void;
  onToggleAxe?: () => void;
  onToggleBenchArm?: () => void;
  onBenchSession?: (id: string) => void;
  onUnbenchSession?: (id: string) => void;
  onTapSession: (id: string) => void;
  onDragToBottom: (id: string) => void;
  onCreateSession: (cwd?: string, spawnTool?: SpawnTool) => Promise<string>;
}

export function OverviewField({
  sessions,
  idlePreviews = {},
  observer = false,
  compact = false,
  axeTopOffset = 8,
  axeArmed = false,
  benchArmed = false,
  benchedIds = new Set(),
  onDisarmAxe,
  onToggleAxe,
  onToggleBenchArm,
  onBenchSession,
  onUnbenchSession,
  onTapSession,
  onDragToBottom,
  onCreateSession,
}: OverviewFieldProps) {
  const [rawMode, setRawMode] = useState(false);
  const [showThoughtConfig, setShowThoughtConfig] = useState(false);
  const [hatchState, setHatchState] = useState<HatchState | null>(null);
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);
  const [axeTargetSessionId, setAxeTargetSessionId] = useState<string | null>(null);
  const [axeHitSessionId, setAxeHitSessionId] = useState<string | null>(null);
  const [benchTargetSessionId, setBenchTargetSessionId] = useState<string | null>(null);
  const [showBenchModal, setShowBenchModal] = useState(false);
  const [axeSlashFx, setAxeSlashFx] = useState<{
    key: number;
    x: number;
    y: number;
  } | null>(null);
  const spawnPositionsRef = useRef<Map<string, { x: number; y: number }>>(new Map());
  const hatchIdRef = useRef(0);
  const menuClickPosRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const fieldRef = useRef<HTMLDivElement>(null);
  const axeHitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const axeDispatchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Centralized position state for all thronglets
  const [positions, setPositions] = useState<Record<string, { x: number; y: number }>>({});
  const exitingSetRef = useRef<Set<string>>(new Set());
  const sessionsRef = useRef<SessionSummary[]>(sessions);

  // Sizing constants (shared between init, wander, and entity rendering)
  const throngletSize = compact ? 60 : 80;
  const spriteHalf = throngletSize / 2;
  const bubbleTopClearance = compact ? 86 : 96;
  const labelClearance = compact ? 105 : 120;

  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);

  useEffect(() => {
    return () => {
      if (axeHitTimerRef.current) {
        clearTimeout(axeHitTimerRef.current);
      }
      if (axeDispatchTimerRef.current) {
        clearTimeout(axeDispatchTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (!axeArmed) {
      setAxeTargetSessionId(null);
      setAxeHitSessionId(null);
    }
  }, [axeArmed]);

  useEffect(() => {
    if (!benchArmed) {
      setBenchTargetSessionId(null);
    }
  }, [benchArmed]);

  // Initialize positions for new sessions, handle exits, clean up removed sessions
  useEffect(() => {
    setPositions((prev) => {
      const next = { ...prev };
      const field = fieldRef.current;
      const fieldW = field ? field.clientWidth : window.innerWidth;
      const fieldH = field ? field.clientHeight : window.innerHeight;
      const maxX = Math.max(fieldW - throngletSize, 0);
      const maxY = Math.max(fieldH - labelClearance, 0);

      for (const s of sessions) {
        // Initialize position for new sessions
        if (!(s.session_id in next)) {
          const spawn = spawnPositionsRef.current.get(s.session_id);
          if (spawn) {
            spawnPositionsRef.current.delete(s.session_id);
            next[s.session_id] = {
              x: Math.max(0, Math.min(maxX, spawn.x - spriteHalf)),
              y: Math.max(bubbleTopClearance, Math.min(maxY, spawn.y - spriteHalf)),
            };
          } else {
            // Random position with initial repulsion from existing thronglets
            let x = Math.random() * maxX;
            let y =
              bubbleTopClearance +
              Math.random() * Math.max(maxY - bubbleTopClearance, 60);
            for (const other of Object.values(next)) {
              const dx = x - other.x;
              const dy = y - other.y;
              const dist = Math.sqrt(dx * dx + dy * dy);
              if (dist < MIN_SEPARATION && dist > 0) {
                const force = (MIN_SEPARATION - dist) / MIN_SEPARATION;
                x += (dx / dist) * force * PUSH_STRENGTH;
                y += (dy / dist) * force * PUSH_STRENGTH;
              }
            }
            next[s.session_id] = {
              x: Math.max(0, Math.min(maxX, x)),
              y: Math.max(bubbleTopClearance, Math.min(maxY, y)),
            };
          }
        }

        // Handle exit — walk toward nearest horizontal edge
        if (
          s.state === "exited" &&
          !exitingSetRef.current.has(s.session_id) &&
          s.session_id in next
        ) {
          exitingSetRef.current.add(s.session_id);
          const midX = fieldW / 2;
          next[s.session_id] = {
            ...next[s.session_id],
            x: next[s.session_id].x < midX ? -120 : fieldW + 40,
          };
        }
      }

      // Remove positions for sessions no longer in the list
      const currentIds = new Set(sessions.map((s) => s.session_id));
      for (const id of Object.keys(next)) {
        if (!currentIds.has(id)) {
          delete next[id];
          exitingSetRef.current.delete(id);
        }
      }

      return next;
    });
  }, [sessions, throngletSize, spriteHalf, bubbleTopClearance, labelClearance]);

  // Single wander interval with separation forces
  useEffect(() => {
    const interval = setInterval(() => {
      setPositions((prev) => {
        if (axeArmed || benchArmed) return prev;
        const field = fieldRef.current;
        const fieldW = field ? field.clientWidth : window.innerWidth;
        const fieldH = field ? field.clientHeight : window.innerHeight;
        const maxX = Math.max(fieldW - throngletSize, 0);
        const maxY = Math.max(fieldH - labelClearance, 0);

        const next = { ...prev };
        const nowMs = Date.now();
        const sessionById = new Map(
          sessionsRef.current.map((session) => [session.session_id, session]),
        );
        const activeIds = Object.keys(next).filter(
          (id) => !exitingSetRef.current.has(id) && sessionById.has(id),
        );
        const stageById = new Map<string, ThrongletRestStage>();
        for (const id of activeIds) {
          const session = sessionById.get(id);
          if (!session) continue;
          stageById.set(
            id,
            throngletRestStageForSession(
              session.state,
              session.last_activity_at,
              nowMs,
            ),
          );
        }

        // Step 1: Apply random wander
        for (const id of activeIds) {
          const stage = stageById.get(id) ?? "active";
          const profile = MOTION_BY_REST_STAGE[stage];
          if (!profile.movable) continue;
          next[id] = {
            x:
              next[id].x +
              (Math.random() - 0.5) * WANDER_X * profile.wanderXScale,
            y:
              next[id].y +
              (Math.random() - 0.5) * WANDER_Y * profile.wanderYScale +
              profile.downwardBias,
          };
        }

        // Step 2: Apply separation forces
        for (const id of activeIds) {
          const stage = stageById.get(id) ?? "active";
          const profile = MOTION_BY_REST_STAGE[stage];
          if (!profile.allowSeparation) continue;

          let rx = 0;
          let ry = 0;
          for (const otherId of activeIds) {
            if (otherId === id) continue;
            const dx = next[id].x - next[otherId].x;
            const dy = next[id].y - next[otherId].y;
            const dist = Math.sqrt(dx * dx + dy * dy);
            if (dist < MIN_SEPARATION && dist > 0) {
              const force = (MIN_SEPARATION - dist) / MIN_SEPARATION;
              rx += (dx / dist) * force * PUSH_STRENGTH;
              ry += (dy / dist) * force * PUSH_STRENGTH;
            }
          }
          next[id] = { x: next[id].x + rx, y: next[id].y + ry };
        }

        // Step 3: Clamp to field bounds
        for (const id of activeIds) {
          next[id] = {
            x: Math.max(0, Math.min(maxX, next[id].x)),
            y: Math.max(bubbleTopClearance, Math.min(maxY, next[id].y)),
          };
        }

        return next;
      });
    }, WANDER_INTERVAL);

    return () => clearInterval(interval);
  }, [axeArmed, benchArmed, throngletSize, bubbleTopClearance, labelClearance]);

  const handleAxeHover = useCallback(
    (sessionId: string | null) => {
      if (!axeArmed) return;
      setAxeTargetSessionId(sessionId);
    },
    [axeArmed],
  );

  const handleBenchHover = useCallback(
    (sessionId: string | null) => {
      if (!benchArmed) return;
      setBenchTargetSessionId(sessionId);
    },
    [benchArmed],
  );

  const handleSessionTap = useCallback(
    (sessionId: string) => {
      if (benchArmed) {
        onBenchSession?.(sessionId);
        return;
      }
      if (!axeArmed) {
        onTapSession(sessionId);
        return;
      }

      const field = fieldRef.current;
      const pos = positions[sessionId];
      const centerX = pos
        ? pos.x + throngletSize / 2
        : (field ? field.clientWidth : window.innerWidth) / 2;
      const centerY = pos
        ? pos.y + throngletSize / 2
        : (field ? field.clientHeight : window.innerHeight) / 2;

      setAxeTargetSessionId(sessionId);
      setAxeHitSessionId(sessionId);
      setAxeSlashFx({ key: Date.now(), x: centerX, y: centerY });

      if (axeHitTimerRef.current) clearTimeout(axeHitTimerRef.current);
      axeHitTimerRef.current = setTimeout(() => {
        setAxeHitSessionId(null);
      }, 90);

      if (axeDispatchTimerRef.current) clearTimeout(axeDispatchTimerRef.current);
      axeDispatchTimerRef.current = setTimeout(() => {
        onTapSession(sessionId);
      }, 120);
    },
    [axeArmed, onTapSession, positions, throngletSize],
  );

  const startHatch = useCallback(
    (clientX: number, clientY: number, cwd?: string, spawnTool?: SpawnTool) => {
      // Convert client coords to field-relative coords.
      const field = fieldRef.current;
      const rect = field?.getBoundingClientRect();
      const x = rect ? clientX - rect.left : clientX;
      const y = rect ? clientY - rect.top : clientY;
      const hatchId = hatchIdRef.current + 1;
      hatchIdRef.current = hatchId;

      setHatchState({
        id: hatchId,
        x,
        y,
        phase: "dropping",
        sessionId: null,
        spawnStatus: "pending",
      });

      // Fire API call in parallel with animation.
      void onCreateSession(cwd, spawnTool)
        .then((sessionId) => {
          setHatchState((prev) => {
            if (!prev || prev.id !== hatchId) return prev;
            if (!sessionId) {
              return { ...prev, spawnStatus: "failed" };
            }
            spawnPositionsRef.current.set(sessionId, { x, y });
            return { ...prev, sessionId, spawnStatus: "success" };
          });
        })
        .catch(() => {
          setHatchState((prev) => {
            if (!prev || prev.id !== hatchId) return prev;
            return { ...prev, spawnStatus: "failed" };
          });
        });
    },
    [onCreateSession],
  );

  const handleFieldClick = useCallback(
    (e: MouseEvent) => {
      if (e.button !== 0) return;
      if (axeArmed) {
        onDisarmAxe?.();
        return;
      }
      if (benchArmed) {
        onToggleBenchArm?.();
        return;
      }
      const target = e.target as HTMLElement;
      if (
        target.closest?.(".thronglet") ||
        target.closest?.(".hatching-egg") ||
        target.closest?.(".spawn-menu") ||
        target.closest?.(".thought-config-overlay") ||
        target.closest?.(".thought-config-trigger") ||
        target.closest?.(".bench-modal-overlay")
      ) {
        return;
      }
      menuClickPosRef.current = { x: e.clientX, y: e.clientY };
      setMenuPos({ x: e.clientX, y: e.clientY });
    },
    [axeArmed, benchArmed, onDisarmAxe, onToggleBenchArm],
  );

  const handleFieldTouch = useCallback(
    (e: TouchEvent) => {
      if (axeArmed) {
        onDisarmAxe?.();
        return;
      }
      if (benchArmed) {
        onToggleBenchArm?.();
        return;
      }
      const target = e.target as HTMLElement;
      if (
        target.closest?.(".thronglet") ||
        target.closest?.(".hatching-egg") ||
        target.closest?.(".spawn-menu") ||
        target.closest?.(".thought-config-overlay") ||
        target.closest?.(".thought-config-trigger") ||
        target.closest?.(".bench-modal-overlay")
      ) {
        return;
      }
      const t = e.changedTouches[0] ?? e.touches[0];
      if (!t) return;
      menuClickPosRef.current = { x: t.clientX, y: t.clientY };
      setMenuPos({ x: t.clientX, y: t.clientY });
    },
    [axeArmed, benchArmed, onDisarmAxe, onToggleBenchArm],
  );

  const handleMenuSelect = useCallback(
    (path: string, spawnTool?: SpawnTool) => {
      setMenuPos(null);
      if (navigator.vibrate) navigator.vibrate(50);
      startHatch(
        menuClickPosRef.current.x,
        menuClickPosRef.current.y,
        path,
        spawnTool,
      );
    },
    [startHatch],
  );

  const handleMenuClose = useCallback(() => {
    setMenuPos(null);
  }, []);

  const handlePhaseComplete = useCallback(
    (completedPhase: HatchPhase) => {
      setHatchState((prev) => {
        if (!prev) return null;
        if (completedPhase === "dropping") return { ...prev, phase: "wobbling" };
        if (completedPhase === "wobbling") {
          return { ...prev, phase: "done" };
        }
        return prev;
      });
    },
    [],
  );

  // Resolve hatch exactly once after animation and spawn both settle.
  useEffect(() => {
    if (!hatchState || hatchState.phase !== "done") return;
    if (hatchState.spawnStatus === "pending") return;

    const timer = setTimeout(() => {
      if (hatchState.spawnStatus === "success" && hatchState.sessionId) {
        onTapSession(hatchState.sessionId);
      }
      setHatchState((prev) => (prev && prev.id === hatchState.id ? null : prev));
    }, 400);
    return () => clearTimeout(timer);
  }, [hatchState, onTapSession]);

  const axeTargetSession = axeTargetSessionId
    ? sessions.find((session) => session.session_id === axeTargetSessionId) ?? null
    : null;
  const axeTargetName = axeTargetSession
    ? throngletName(axeTargetSession)
    : axeTargetSessionId;

  return (
    <div
      ref={fieldRef}
      class={`field ${axeArmed ? "axe-armed" : ""} ${benchArmed ? "bench-armed" : ""}`}
      style={{ flex: 1, position: "relative" }}
      onClick={observer ? undefined : handleFieldClick}
      onTouchEnd={observer ? undefined : handleFieldTouch}
      onContextMenu={observer ? undefined : (e: Event) => e.preventDefault()}
    >
      {!observer && onToggleAxe && (
        <button
          type="button"
          aria-label={axeArmed ? "Disarm axe mode" : "Arm axe mode"}
          title={axeArmed ? "Axe armed" : "Arm axe"}
          onClick={(e: MouseEvent) => {
            e.stopPropagation();
            onToggleAxe();
            if (navigator.vibrate) navigator.vibrate(axeArmed ? 10 : 25);
          }}
          style={{
            position: "absolute",
            top: `${axeTopOffset}px`,
            left: "8px",
            width: "60px",
            height: "60px",
            border: "none",
            background: "transparent",
            boxShadow: "none",
            padding: 0,
            filter: axeArmed
              ? "drop-shadow(0 0 8px rgba(231, 76, 60, 0.9))"
              : "drop-shadow(0 2px 3px rgba(0, 0, 0, 0.45))",
            display: "grid",
            placeItems: "center",
            zIndex: 220,
            cursor: "pointer",
          }}
        >
          <svg
            width="44"
            height="44"
            viewBox="0 0 32 32"
            fill="none"
            aria-hidden="true"
            shapeRendering="crispEdges"
          >
            {/* Pixel blade base */}
            <rect x="20" y="2" width="8" height="2" fill="#5a616c" />
            <rect x="18" y="4" width="10" height="2" fill="#707987" />
            <rect x="16" y="6" width="10" height="2" fill="#7f8897" />
            <rect x="14" y="8" width="10" height="2" fill="#707987" />
            <rect x="16" y="10" width="8" height="2" fill="#626a77" />
            <rect x="18" y="12" width="4" height="2" fill="#4f5561" />
            <rect x="24" y="12" width="4" height="2" fill="#5a616c" />

            {/* Blade highlights */}
            <rect x="21" y="4" width="5" height="2" fill="#c8d0da" />
            <rect x="19" y="6" width="5" height="2" fill="#d7dde5" />
            <rect x="17" y="8" width="4" height="2" fill="#bcc5d1" />
            <rect x="17" y="10" width="2" height="2" fill="#9fa9b8" />

            {/* Socket + rivet */}
            <rect x="18" y="12" width="3" height="2" fill="#3e434d" />
            <rect x="19" y="12" width="1" height="1" fill="#dbe1e9" />

            {/* Pixel handle */}
            <rect x="17" y="13" width="2" height="2" fill="#5a3419" />
            <rect x="15" y="15" width="2" height="2" fill="#734321" />
            <rect x="13" y="17" width="2" height="2" fill="#5a3419" />
            <rect x="11" y="19" width="2" height="2" fill="#734321" />
            <rect x="9" y="21" width="2" height="2" fill="#5a3419" />
            <rect x="7" y="23" width="2" height="2" fill="#734321" />
            <rect x="5" y="25" width="2" height="2" fill="#5a3419" />
            <rect x="4" y="26" width="2" height="2" fill="#b7834a" />
            <rect x="5" y="24" width="1" height="2" fill="#d3a066" />
            <rect x="7" y="22" width="1" height="2" fill="#d3a066" />
            <rect x="9" y="20" width="1" height="2" fill="#d3a066" />
            <rect x="11" y="18" width="1" height="2" fill="#d3a066" />
          </svg>
        </button>
      )}

      {!observer && onToggleBenchArm && (
        <button
          type="button"
          class={`bench-trigger ${benchArmed ? "armed" : ""}`}
          aria-label={benchArmed ? "Disarm bench mode" : "Bench mode"}
          title={benchArmed ? "Bench armed" : "Hide thronglets"}
          onClick={(e: MouseEvent) => {
            e.stopPropagation();
            if (benchArmed) {
              onToggleBenchArm();
              if (navigator.vibrate) navigator.vibrate(10);
            } else if (benchedIds.size > 0) {
              setShowBenchModal(true);
              if (navigator.vibrate) navigator.vibrate(15);
            } else {
              onToggleBenchArm();
              if (navigator.vibrate) navigator.vibrate(25);
            }
          }}
          style={{
            position: "absolute",
            top: `${axeTopOffset + 8}px`,
            left: !observer && onToggleAxe ? "74px" : "8px",
            width: "44px",
            height: "44px",
            border: "none",
            borderRadius: "12px",
            background: "rgba(17, 30, 49, 0.65)",
            boxShadow: "0 2px 8px rgba(0, 0, 0, 0.35)",
            padding: 0,
            display: "grid",
            placeItems: "center",
            zIndex: 220,
            cursor: "pointer",
          }}
        >
          <svg
            width="24"
            height="24"
            viewBox="0 0 24 24"
            fill="none"
            aria-hidden="true"
          >
            {/* Eye with slash — hide icon */}
            <path
              d="M3 12s3-7 9-7 9 7 9 7-3 7-9 7-9-7-9-7z"
              stroke={benchArmed ? "#5b94eb" : "#f5f7fb"}
              strokeWidth="1.5"
              fill="none"
            />
            <circle
              cx="12"
              cy="12"
              r="3"
              stroke={benchArmed ? "#5b94eb" : "#f5f7fb"}
              strokeWidth="1.5"
              fill="none"
            />
            <line
              x1="4"
              y1="20"
              x2="20"
              y2="4"
              stroke={benchArmed ? "#5b94eb" : "#f5f7fb"}
              strokeWidth="1.5"
            />
          </svg>
          {benchedIds.size > 0 && !benchArmed && (
            <span class="bench-count">{benchedIds.size}</span>
          )}
        </button>
      )}

      <button
        type="button"
        class="thought-config-trigger"
        aria-label="Open thought config"
        title="Thought config"
        onClick={(e: MouseEvent) => {
          e.stopPropagation();
          setShowThoughtConfig(true);
          if (navigator.vibrate) navigator.vibrate(15);
        }}
        style={{
          position: "absolute",
          top: `${axeTopOffset + 8}px`,
          left: !observer && onToggleAxe && onToggleBenchArm
            ? "124px"
            : !observer && onToggleAxe
              ? "74px"
              : "8px",
          width: "44px",
          height: "44px",
          border: "none",
          borderRadius: "12px",
          background: "rgba(17, 30, 49, 0.65)",
          boxShadow: "0 2px 8px rgba(0, 0, 0, 0.35)",
          padding: 0,
          display: "grid",
          placeItems: "center",
          zIndex: 220,
          cursor: "pointer",
        }}
      >
        <svg
          width="24"
          height="24"
          viewBox="0 0 24 24"
          fill="none"
          aria-hidden="true"
        >
          <rect x="10.8" y="1.8" width="2.4" height="4.2" rx="1" fill="#f5f7fb" />
          <rect x="10.8" y="18" width="2.4" height="4.2" rx="1" fill="#f5f7fb" />
          <rect x="1.8" y="10.8" width="4.2" height="2.4" rx="1" fill="#f5f7fb" />
          <rect x="18" y="10.8" width="4.2" height="2.4" rx="1" fill="#f5f7fb" />
          <circle
            cx="12"
            cy="12"
            r="6.2"
            fill="#2d3a53"
            stroke="#f5f7fb"
            strokeWidth="1.4"
          />
          <circle
            cx="12"
            cy="12"
            r="2.7"
            fill="#101726"
            stroke="#f5f7fb"
            strokeWidth="1.2"
          />
        </svg>
      </button>

      {/* Scenery trees */}
      <img
        class="field-tree"
        src="/assets/tree.png"
        style={{ left: "10%", bottom: "15%" }}
        alt=""
      />
      <img
        class="field-tree"
        src="/assets/tree.png"
        style={{ right: "8%", bottom: "35%" }}
        alt=""
      />
      <img
        class="field-tree field-tree-sm"
        src="/assets/tree.png"
        style={{ left: "55%", bottom: "55%" }}
        alt=""
      />

      {/* Thronglet container */}
      <div
        id="thronglets-container"
        style={{ position: "absolute", inset: 0 }}
      >
        {sessions
          .filter((s) =>
            !(hatchState && hatchState.phase !== "done" && hatchState.sessionId === s.session_id)
          )
          .filter((s) => !benchedIds.has(s.session_id))
          .map((s) => {
            const pos = positions[s.session_id];
            if (!pos) return null;
            const resolvedPack =
              s.sprite_pack_id != null
                ? (spritePacksSignal.value[s.sprite_pack_id] ?? null)
                : null;
            return (
              <ThrongletEntity
                key={s.session_id}
                session={s}
                idlePreview={idlePreviews[s.session_id]}
                spritePack={resolvedPack}
                x={pos.x}
                y={pos.y}
                axeArmed={axeArmed}
                axeTargeted={axeArmed && axeTargetSessionId === s.session_id}
                axeDimmed={
                  axeArmed &&
                  axeTargetSessionId !== null &&
                  axeTargetSessionId !== s.session_id
                }
                axeHit={axeHitSessionId === s.session_id}
                benchArmed={benchArmed}
                benchTargeted={benchArmed && benchTargetSessionId === s.session_id}
                benchDimmed={
                  benchArmed &&
                  benchTargetSessionId !== null &&
                  benchTargetSessionId !== s.session_id
                }
                compact={compact}
                rawMode={rawMode}
                onAxeHover={handleAxeHover}
                onBenchHover={handleBenchHover}
                onTap={handleSessionTap}
                onDragToBottom={onDragToBottom}
              />
            );
          })}
      </div>

      {axeSlashFx && (
        <div
          key={axeSlashFx.key}
          class="axe-slash-effect"
          style={{ left: `${axeSlashFx.x}px`, top: `${axeSlashFx.y}px` }}
          onAnimationEnd={() => setAxeSlashFx(null)}
        />
      )}

      {/* Hatching egg animation */}
      {hatchState && hatchState.phase !== "done" && (
        <HatchingEgg
          x={hatchState.x}
          y={hatchState.y}
          phase={hatchState.phase}
          onPhaseComplete={handlePhaseComplete}
        />
      )}

      {/* Spawn menu */}
      {menuPos && (
        <SpawnMenu
          x={menuPos.x}
          y={menuPos.y}
          onSelect={handleMenuSelect}
          onClose={handleMenuClose}
        />
      )}

      <ThoughtConfigPanel
        open={showThoughtConfig}
        observer={observer}
        onClose={() => setShowThoughtConfig(false)}
      />

      <BenchModal
        open={showBenchModal}
        sessions={sessions}
        benchedIds={benchedIds}
        onClose={() => setShowBenchModal(false)}
        onTapSession={(sessionId) => {
          setShowBenchModal(false);
          onTapSession(sessionId);
        }}
        onUnbench={(sessionId) => {
          onUnbenchSession?.(sessionId);
        }}
      />

      {benchArmed && !observer && (
        <div
          style={{
            position: "absolute",
            top: "10px",
            left: !observer && onToggleAxe ? "62px" : "12px",
            padding: "6px 10px",
            borderRadius: "8px",
            background: "rgba(22, 22, 22, 0.8)",
            border: "1px solid rgba(91, 148, 235, 0.7)",
            color: "#c4daff",
            fontSize: "12px",
            fontWeight: 700,
            letterSpacing: "0.01em",
            zIndex: 120,
          }}
        >
          {benchTargetSessionId
            ? (() => {
                const target = sessions.find(
                  (s) => s.session_id === benchTargetSessionId,
                );
                const name = target ? throngletName(target) : benchTargetSessionId;
                return `Hide: ${name}`;
              })()
            : "Hide mode: tap a thronglet"}
        </div>
      )}

      {axeArmed && !observer && (
        <div
          style={{
            position: "absolute",
            top: "10px",
            left: "62px",
            padding: "6px 10px",
            borderRadius: "8px",
            background: "rgba(22, 22, 22, 0.8)",
            border: "1px solid rgba(231, 76, 60, 0.7)",
            color: "#ffd9d3",
            fontSize: "12px",
            fontWeight: 700,
            letterSpacing: "0.01em",
            zIndex: 120,
          }}
        >
          {axeTargetName
            ? `Axe armed: ${axeTargetName}`
            : "Axe armed: tap a thronglet"}
        </div>
      )}

      {/* Observer badge */}
      {observer && (
        <div
          data-testid="observer-badge"
          style={{
            position: "absolute",
            top: "8px",
            right: "8px",
            background: "#16213e",
            color: "#5BC0EB",
            padding: "4px 12px",
            borderRadius: "4px",
            fontSize: "11px",
            fontWeight: 600,
            zIndex: 100,
          }}
        >
          OBSERVER
        </div>
      )}

      {/* Empty state */}
      {sessions.length === 0 && !hatchState && !menuPos && (
        <div class="empty-state">
          <p>No sessions yet</p>
          {!observer && <p class="hint">Tap anywhere to spawn one</p>}
        </div>
      )}

      {/* All benched empty state */}
      {sessions.length > 0 &&
        benchedIds.size > 0 &&
        sessions.every((s) => benchedIds.has(s.session_id) || s.state === "exited") &&
        !hatchState &&
        !menuPos && (
        <div class="empty-state">
          <p>All thronglets hidden</p>
          {!observer && (
            <p
              class="hint"
              style={{ cursor: "pointer", textDecoration: "underline" }}
              onClick={(e: MouseEvent) => {
                e.stopPropagation();
                setShowBenchModal(true);
              }}
            >
              Tap to show hidden ({benchedIds.size})
            </p>
          )}
        </div>
      )}

      {/* Secret raw-mode toggle (bottom of field) */}
      <div
        class="raw-mode-tap-zone"
        onClick={(e: MouseEvent) => {
          e.stopPropagation();
          setRawMode((v) => !v);
          if (navigator.vibrate) navigator.vibrate(15);
        }}
        onTouchEnd={(e: TouchEvent) => {
          e.stopPropagation();
          e.preventDefault();
          setRawMode((v) => !v);
          if (navigator.vibrate) navigator.vibrate(15);
        }}
      />
    </div>
  );
}
