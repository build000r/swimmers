import { useEffect, useRef, useCallback } from "preact/hooks";
import type { SessionSummary, SessionState } from "@/types";
import { useLongPress } from "@/hooks/useGestures";

// ---- Helpers ----

function spriteForState(state: SessionState): string {
  const map: Record<string, string> = {
    idle: "/assets/idle.png",
    busy: "/assets/walking.png",
    error: "/assets/beep.png",
    attention: "/assets/idle.png",
    exited: "/assets/sad.png",
  };
  return map[state] ?? map.idle;
}

function repoName(cwd: string): string {
  if (!cwd || cwd === "/") return "root";
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || "root";
}

function numberPrefix(name: string): string {
  const num = parseInt(name, 10);
  if (isNaN(num)) return name;
  return String(num).slice(-2);
}

function gaugeColor(ratio: number): string {
  if (ratio >= 0.8) return "#E74C3C";
  if (ratio >= 0.5) return "#F5A623";
  return "#4FC08D";
}

// ---- ThrongletEntity sub-component ----

interface ThrongletProps {
  session: SessionSummary;
  onTap: (id: string) => void;
  onDragToBottom: (id: string) => void;
}

function ThrongletEntity({ session, onTap, onDragToBottom }: ThrongletProps) {
  const elRef = useRef<HTMLDivElement>(null);
  const posRef = useRef({
    x: Math.random() * (window.innerWidth - 80),
    y: 40 + Math.random() * Math.max(window.innerHeight - 200, 60),
  });
  const wanderRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const isDraggingRef = useRef(false);
  const longPressedRef = useRef(false);
  const exitingRef = useRef(false);

  // Wander randomly every 3s
  useEffect(() => {
    wanderRef.current = setInterval(() => {
      if (exitingRef.current) return;
      const maxX = window.innerWidth - 80;
      const maxY = window.innerHeight - 120;
      posRef.current.x = Math.max(
        0,
        Math.min(maxX, posRef.current.x + (Math.random() - 0.5) * 100),
      );
      posRef.current.y = Math.max(
        40,
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
  }, []);

  // Walk off screen when session exits
  useEffect(() => {
    if (session.state !== "exited" || exitingRef.current) return;
    exitingRef.current = true;
    if (wanderRef.current) {
      clearInterval(wanderRef.current);
      wanderRef.current = null;
    }
    // Walk toward the nearest horizontal edge
    const midX = window.innerWidth / 2;
    const targetX = posRef.current.x < midX ? -120 : window.innerWidth + 40;
    posRef.current.x = targetX;
    if (elRef.current) {
      elRef.current.style.left = targetX + "px";
    }
  }, [session.state]);

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
              "position:fixed;pointer-events:none;z-index:9999;opacity:0.7;width:64px;height:64px;";
            document.body.appendChild(ghost);
          }
        }

        if (dragging && ghost) {
          ghost.style.left = me.clientX - 32 + "px";
          ghost.style.top = me.clientY - 32 + "px";
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
    [session.session_id, onDragToBottom],
  );

  // Thought / activity text
  let activityText = "";
  let thoughtText = "";
  let showBubble = false;

  if (session.thought) {
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

  const showGauge = session.token_count > 0;
  const gaugeRatio = session.context_limit
    ? Math.min(session.token_count / session.context_limit, 1)
    : 0;

  return (
    <div
      ref={elRef}
      class={`thronglet ${session.state}`}
      style={{
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
      {/* Thought bubble */}
      {showBubble && (
        <div class="thought-bubble">
          <span class="thought-text">{thoughtText}</span>
          <div class="thought-circle thought-circle-lg" />
          <div class="thought-circle thought-circle-sm" />
        </div>
      )}

      {/* Tool badge */}
      {session.tool && <div class="thronglet-tool">{session.tool}</div>}

      {/* Sprite */}
      <img
        class="thronglet-sprite"
        src={spriteForState(session.state)}
        alt=""
      />

      {/* Label */}
      <div class="thronglet-label">
        <div class="thronglet-name">
          {numberPrefix(session.tmux_name) + " " + repoName(session.cwd)}
        </div>
        {showGauge && (
          <div class="context-gauge" style={{ display: "block" }}>
            <div
              class="context-gauge-fill"
              style={{
                width: gaugeRatio * 100 + "%",
                background: gaugeColor(gaugeRatio),
              }}
            />
          </div>
        )}
        {activityText && !showBubble && (
          <div class="thronglet-activity">{activityText}</div>
        )}
      </div>
    </div>
  );
}

// ---- OverviewField ----

interface OverviewFieldProps {
  sessions: SessionSummary[];
  observer?: boolean;
  onTapSession: (id: string) => void;
  onDragToBottom: (id: string) => void;
  onCreateSession: () => void;
}

export function OverviewField({
  sessions,
  observer = false,
  onTapSession,
  onDragToBottom,
  onCreateSession,
}: OverviewFieldProps) {
  // Disable long-press create when in observer mode
  const longPress = useLongPress(observer ? () => {} : onCreateSession, 500);

  return (
    <div
      class="field"
      style={{ flex: 1, position: "relative" }}
      onMouseDown={observer ? undefined : longPress.onMouseDown}
      onMouseUp={observer ? undefined : longPress.onMouseUp}
      onMouseMove={observer ? undefined : longPress.onMouseMove}
      onMouseLeave={observer ? undefined : longPress.onMouseLeave}
      onTouchStart={observer ? undefined : longPress.onTouchStart}
      onTouchMove={observer ? undefined : longPress.onTouchMove}
      onTouchEnd={observer ? undefined : longPress.onTouchEnd}
      onContextMenu={observer ? undefined : longPress.onContextMenu}
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
        {sessions.map((s) => (
          <ThrongletEntity
            key={s.session_id}
            session={s}
            onTap={onTapSession}
            onDragToBottom={onDragToBottom}
          />
        ))}
      </div>

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
      {sessions.length === 0 && (
        <div class="empty-state">
          <p>No sessions yet</p>
          {!observer && <p class="hint">Long press to create one</p>}
        </div>
      )}
    </div>
  );
}
