import { useEffect, useCallback } from "preact/hooks";
import type { SessionSummary } from "@/types";
import "./BenchModal.css";

function throngletName(session: SessionSummary): string {
  const trimmed = session.cwd.trim();
  if (trimmed && trimmed !== "/") {
    const parts = trimmed.replace(/\/+$/, "").split("/").filter(Boolean);
    if (parts.length > 0) return parts[parts.length - 1];
  }
  return session.tmux_name;
}

function truncateThought(thought: string | null, max: number): string {
  if (!thought) return "";
  const trimmed = thought.trim();
  if (trimmed.length <= max) return trimmed;
  return trimmed.slice(0, max) + "...";
}

interface BenchModalProps {
  open: boolean;
  sessions: SessionSummary[];
  benchedIds: Set<string>;
  onClose: () => void;
  onTapSession: (sessionId: string) => void;
  onUnbench: (sessionId: string) => void;
}

export function BenchModal({
  open,
  sessions,
  benchedIds,
  onClose,
  onTapSession,
  onUnbench,
}: BenchModalProps) {
  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    },
    [onClose],
  );

  useEffect(() => {
    if (!open) return;
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, handleKeyDown]);

  if (!open) return null;

  const benchedSessions = sessions.filter((s) => benchedIds.has(s.session_id));

  return (
    <div
      class="bench-modal-overlay"
      onClick={(event: MouseEvent) => {
        event.stopPropagation();
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
      onTouchEnd={(event: TouchEvent) => {
        event.stopPropagation();
      }}
    >
      <div
        class="bench-modal-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Hidden thronglets"
        onClick={(event: MouseEvent) => event.stopPropagation()}
        onTouchEnd={(event: TouchEvent) => event.stopPropagation()}
      >
        <div class="bench-modal-header">
          <h2>Hidden</h2>
          <button
            type="button"
            class="bench-modal-close"
            onClick={(event: MouseEvent) => {
              event.stopPropagation();
              onClose();
            }}
          >
            Close
          </button>
        </div>

        <div class="bench-modal-list">
          {benchedSessions.length === 0 && (
            <div class="bench-modal-empty">No hidden thronglets</div>
          )}
          {benchedSessions.map((s) => (
            <div
              key={s.session_id}
              class="bench-modal-item"
              data-testid="bench-modal-item"
              onClick={(event: MouseEvent) => {
                event.stopPropagation();
                onTapSession(s.session_id);
              }}
            >
              <span class={`bench-modal-item-dot ${s.state}`} />
              <div class="bench-modal-item-info">
                <span class="bench-modal-item-name">
                  {throngletName(s)}
                </span>
                {s.thought && (
                  <span class="bench-modal-item-thought">
                    {truncateThought(s.thought, 40)}
                  </span>
                )}
              </div>
              <button
                type="button"
                class="bench-modal-unhide"
                onClick={(event: MouseEvent) => {
                  event.stopPropagation();
                  onUnbench(s.session_id);
                }}
              >
                Unhide
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
