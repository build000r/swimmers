import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import type { SessionSummary, SessionState } from "@/types";
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

// ---- ThrongletEntity sub-component ----

interface ThrongletProps {
  session: SessionSummary;
  idlePreview?: string;
  spawnPosition?: { x: number; y: number };
  compact?: boolean;
  onTap: (id: string) => void;
  onDragToBottom: (id: string) => void;
}

function ThrongletEntity({
  session,
  idlePreview,
  spawnPosition,
  compact = false,
  onTap,
  onDragToBottom,
}: ThrongletProps) {
  const elRef = useRef<HTMLDivElement>(null);
  const fieldElRef = useRef<HTMLElement | null>(null);
  const spawnPositionAppliedRef = useRef(Boolean(spawnPosition));
  const throngletSize = compact ? 60 : 80;
  const spriteHalf = throngletSize / 2;
  const bubbleTopClearance = compact ? 86 : 96;
  const labelClearance = compact ? 105 : 120;
  const posRef = useRef(
    spawnPosition
      ? { x: spawnPosition.x - spriteHalf, y: spawnPosition.y - spriteHalf }
      : {
          x: Math.random() * Math.max(window.innerWidth - throngletSize, 0),
          y: bubbleTopClearance +
            Math.random() *
              Math.max(
                window.innerHeight - labelClearance - bubbleTopClearance,
                60,
              ),
        },
  );
  const wanderRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const isDraggingRef = useRef(false);
  const longPressedRef = useRef(false);
  const exitingRef = useRef(false);
  const prevToolRef = useRef<string | null>(session.tool);
  const [isHatching, setIsHatching] = useState(false);
  const isExited = session.state === "exited";
  const isEgg = !isExited && !session.tool && !isHatching;
  const showEggSprite = !isExited && (isEgg || isHatching);
  const lastBubbleRef = useRef<{
    text: string;
    isIdlePreview: boolean;
  } | null>(null);

  // Resolve field container on mount for wander bounds
  useEffect(() => {
    fieldElRef.current = elRef.current?.closest?.(".field") as HTMLElement | null;
  }, []);

  // If spawn position arrives after first render, snap once to the intended
  // hatch point instead of keeping the random fallback position.
  useEffect(() => {
    if (!spawnPosition || spawnPositionAppliedRef.current) return;

    const field =
      fieldElRef.current ??
      ((elRef.current?.closest?.(".field") as HTMLElement | null) ?? null);
    if (!fieldElRef.current) {
      fieldElRef.current = field;
    }

    const fieldW = field ? field.clientWidth : window.innerWidth;
    const fieldH = field ? field.clientHeight : window.innerHeight;
    const maxX = Math.max(fieldW - throngletSize, 0);
    const maxY = Math.max(fieldH - labelClearance, 60);
    const x = Math.max(0, Math.min(maxX, spawnPosition.x - spriteHalf));
    const y = Math.max(
      bubbleTopClearance,
      Math.min(maxY, spawnPosition.y - spriteHalf),
    );
    posRef.current = { x, y };
    spawnPositionAppliedRef.current = true;

    if (elRef.current) {
      elRef.current.style.left = `${x}px`;
      elRef.current.style.top = `${y}px`;
    }
  }, [
    spawnPosition?.x,
    spawnPosition?.y,
    throngletSize,
    bubbleTopClearance,
    labelClearance,
    spriteHalf,
  ]);

  // Wander randomly every 3s
  useEffect(() => {
    wanderRef.current = setInterval(() => {
      if (exitingRef.current) return;
      const field = fieldElRef.current;
      const fieldW = field ? field.clientWidth : window.innerWidth;
      const fieldH = field ? field.clientHeight : window.innerHeight;
      const maxX = fieldW - throngletSize;
      const maxY = fieldH - labelClearance;
      posRef.current.x = Math.max(
        0,
        Math.min(maxX, posRef.current.x + (Math.random() - 0.5) * 100),
      );
      posRef.current.y = Math.max(
        bubbleTopClearance,
        Math.min(maxY, posRef.current.y + (Math.random() - 0.5) * 80),
      );
      if (elRef.current) {
        elRef.current.style.left = posRef.current.x + "px";
        elRef.current.style.top = posRef.current.y + "px";
      }
    }, 3000);

    return () => {
      if (wanderRef.current) clearInterval(wanderRef.current);
    };
  }, [throngletSize, bubbleTopClearance, labelClearance]);

  // Walk off screen when session exits
  useEffect(() => {
    if (session.state !== "exited" || exitingRef.current) return;
    exitingRef.current = true;
    if (wanderRef.current) {
      clearInterval(wanderRef.current);
      wanderRef.current = null;
    }
    // Walk toward the nearest horizontal edge
    const field = fieldElRef.current;
    const fieldW = field ? field.clientWidth : window.innerWidth;
    const midX = fieldW / 2;
    const targetX = posRef.current.x < midX ? -120 : fieldW + 40;
    posRef.current.x = targetX;
    if (elRef.current) {
      elRef.current.style.left = targetX + "px";
    }
  }, [session.state]);

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

  const showGauge = session.token_count > 0;
  const gaugeRatio = session.context_limit
    ? Math.min(session.token_count / session.context_limit, 1)
    : 0;

  return (
    <div
      ref={elRef}
      class={`thronglet ${isExited ? "exited" : isEgg ? "egg" : isHatching ? "hatching-reveal" : session.state}`}
      style={{
        "--thronglet-size": `${throngletSize}px`,
        left: posRef.current.x + "px",
        top: posRef.current.y + "px",
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
  onCreateSession: (cwd?: string) => Promise<string>;
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
  const [hatchState, setHatchState] = useState<HatchState | null>(null);
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(null);
  const spawnPositionsRef = useRef<Map<string, { x: number; y: number }>>(new Map());
  const menuClickPosRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const fieldRef = useRef<HTMLDivElement>(null);

  const startHatch = useCallback(
    (clientX: number, clientY: number, cwd?: string) => {
      // Convert client coords to field-relative coords.
      const field = fieldRef.current;
      const rect = field?.getBoundingClientRect();
      const x = rect ? clientX - rect.left : clientX;
      const y = rect ? clientY - rect.top : clientY;

      setHatchState({ x, y, phase: "dropping", sessionId: null });

      // Fire API call in parallel with animation.
      void onCreateSession(cwd).then((sessionId) => {
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
    (path: string) => {
      setMenuPos(null);
      if (navigator.vibrate) navigator.vibrate(50);
      startHatch(menuClickPosRef.current.x, menuClickPosRef.current.y, path);
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
          .map((s) => (
            <ThrongletEntity
              key={s.session_id}
              session={s}
              idlePreview={idlePreviews[s.session_id]}
              spawnPosition={spawnPositionsRef.current.get(s.session_id)}
              compact={compact}
              onTap={onTapSession}
              onDragToBottom={onDragToBottom}
            />
          ))}
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
    </div>
  );
}
