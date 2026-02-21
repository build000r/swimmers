import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import type { SessionSummary, SpawnTool } from "@/types";
import {
  throngletRestStageForSession,
  type ThrongletRestStage,
} from "@/lib/thronglet-motion";
import { SpawnMenu } from "./SpawnMenu";
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
  x: number;
  y: number;
  compact?: boolean;
  rawMode?: boolean;
  onTap: (id: string) => void;
  onDragToBottom: (id: string) => void;
}

function ThrongletEntity({
  session,
  idlePreview,
  x,
  y,
  compact = false,
  rawMode = false,
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
  const lastBubbleRef = useRef<{
    text: string;
    isIdlePreview: boolean;
  } | null>(null);

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
    [session.session_id, onDragToBottom, throngletSize, spriteHalf],
  );

  // Thought / activity text
  let activityText = "";
  let thoughtText = "";
  let showBubble = false;
  const idlePreviewText = idlePreview?.trim() ?? "";
  const showIdlePreview =
    session.state === "idle" && idlePreviewText.length > 0;

  if (showIdlePreview) {
    activityText = idlePreviewText;
    thoughtText = idlePreviewText;
    showBubble = true;
  } else if (session.thought) {
    activityText = session.thought;
    thoughtText = session.thought;
    showBubble = true;
  } else if (session.state === "busy" && session.current_command) {
    activityText = session.current_command;
    thoughtText = session.current_command;
    showBubble = true;
  } else if (session.state === "error") {
    activityText = "error!";
    thoughtText = "!!!";
    showBubble = true;
  } else if (session.state === "attention") {
    activityText = "ready";
    thoughtText = "?";
    showBubble = true;
  }

  const liveBubble = showBubble
    ? {
        text: thoughtText,
        isIdlePreview: showIdlePreview,
      }
    : null;

  useEffect(() => {
    if (!liveBubble) return;
    lastBubbleRef.current = liveBubble;
  }, [liveBubble?.text, liveBubble?.isIdlePreview]);

  const renderedBubble = liveBubble ?? lastBubbleRef.current;
  const bubbleText = renderedBubble?.text ?? "";
  const bubbleIdlePreview = renderedBubble?.isIdlePreview ?? false;
  const showRenderedBubble = !!renderedBubble;

  const showGauge = session.context_limit > 0;
  const gaugeRatio = session.context_limit
    ? Math.min(session.token_count / session.context_limit, 1)
    : 0;

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
      class={`thronglet ${isExited ? "exited" : isEgg ? "egg" : isHatching ? "hatching-reveal" : session.state}`}
      style={{
        "--thronglet-size": `${throngletSize}px`,
        left: x + "px",
        top: y + "px",
      }}
      onClick={handleClick}
      onMouseDown={handleMouseDown}
      onTouchStart={(e: TouchEvent) => {
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
        />
      )}

      {/* Label */}
      <div class="thronglet-label">
        <div class="thronglet-name">
          {throngletName(session)}
        </div>
        {showGauge && (
          <div class={`context-gauge${gaugeRatio >= 0.8 ? " critical" : ""}`}>
            <div
              class="context-gauge-fill"
              style={{
                "--gauge-color": gaugeColor(gaugeRatio),
                "--gauge-segments": gaugeSegments(gaugeRatio),
              } as Record<string, string | number>}
            />
          </div>
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
  x: number;
  y: number;
  phase: HatchPhase;
  sessionId: string | null;
}

interface OverviewFieldProps {
  sessions: SessionSummary[];
  idlePreviews?: Record<string, string>;
  observer?: boolean;
  compact?: boolean;
  onTapSession: (id: string) => void;
  onDragToBottom: (id: string) => void;
  onCreateSession: (cwd?: string, spawnTool?: SpawnTool) => Promise<string>;
}

export function OverviewField({
  sessions,
  idlePreviews = {},
  observer = false,
  compact = false,
  onTapSession,
  onDragToBottom,
  onCreateSession,
}: OverviewFieldProps) {
  const [rawMode, setRawMode] = useState(false);
  const [hatchState, setHatchState] = useState<HatchState | null>(null);
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);
  const spawnPositionsRef = useRef<Map<string, { x: number; y: number }>>(new Map());
  const menuClickPosRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const fieldRef = useRef<HTMLDivElement>(null);

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
  }, [throngletSize, bubbleTopClearance, labelClearance]);

  const startHatch = useCallback(
    (clientX: number, clientY: number, cwd?: string, spawnTool?: SpawnTool) => {
      // Convert client coords to field-relative coords.
      const field = fieldRef.current;
      const rect = field?.getBoundingClientRect();
      const x = rect ? clientX - rect.left : clientX;
      const y = rect ? clientY - rect.top : clientY;

      setHatchState({ x, y, phase: "dropping", sessionId: null });

      // Fire API call in parallel with animation.
      void onCreateSession(cwd, spawnTool).then((sessionId) => {
        if (sessionId) {
          spawnPositionsRef.current.set(sessionId, { x, y });
          setHatchState((prev) =>
            prev ? { ...prev, sessionId } : null,
          );
        }
      });
    },
    [onCreateSession],
  );

  const handleFieldClick = useCallback(
    (e: MouseEvent) => {
      if (e.button !== 0) return;
      const target = e.target as HTMLElement;
      if (
        target.closest?.(".thronglet") ||
        target.closest?.(".hatching-egg") ||
        target.closest?.(".spawn-menu")
      ) {
        return;
      }
      menuClickPosRef.current = { x: e.clientX, y: e.clientY };
      setMenuPos({ x: e.clientX, y: e.clientY });
    },
    [],
  );

  const handleFieldTouch = useCallback(
    (e: TouchEvent) => {
      const target = e.target as HTMLElement;
      if (
        target.closest?.(".thronglet") ||
        target.closest?.(".hatching-egg") ||
        target.closest?.(".spawn-menu")
      ) {
        return;
      }
      const t = e.touches[0];
      menuClickPosRef.current = { x: t.clientX, y: t.clientY };
      setMenuPos({ x: t.clientX, y: t.clientY });
    },
    [],
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
          // Wobble done — show egg as ThrongletEntity now.
          if (prev.sessionId) {
            setTimeout(() => onTapSession(prev.sessionId!), 400);
          }
          return { ...prev, phase: "done" };
        }
        return prev;
      });
    },
    [onTapSession],
  );

  // If animation reached "done" but sessionId arrived late, navigate now.
  useEffect(() => {
    if (hatchState?.phase === "done" && hatchState.sessionId) {
      const timer = setTimeout(() => {
        onTapSession(hatchState.sessionId!);
        setHatchState(null);
      }, 400);
      return () => clearTimeout(timer);
    }
  }, [hatchState, onTapSession]);

  return (
    <div
      ref={fieldRef}
      class="field"
      style={{ flex: 1, position: "relative" }}
      onClick={observer ? undefined : handleFieldClick}
      onTouchEnd={observer ? undefined : handleFieldTouch}
      onContextMenu={observer ? undefined : (e: Event) => e.preventDefault()}
    >
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
          .map((s) => {
            const pos = positions[s.session_id];
            if (!pos) return null;
            return (
              <ThrongletEntity
                key={s.session_id}
                session={s}
                idlePreview={idlePreviews[s.session_id]}
                x={pos.x}
                y={pos.y}
                compact={compact}
                rawMode={rawMode}
                onTap={onTapSession}
                onDragToBottom={onDragToBottom}
              />
            );
          })}
      </div>

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
