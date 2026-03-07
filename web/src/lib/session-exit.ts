import type { SessionState, SessionSummary } from "@/types";

type ExitAwareState = {
  state: SessionState;
  exit_reason?: string | null;
};

export function normalizeExitReason(
  state: SessionState,
  exitReason?: string | null,
): string | null {
  return state === "exited" ? exitReason ?? null : null;
}

export function isProcessExitState(value: ExitAwareState): boolean {
  return (
    value.state === "exited" &&
    normalizeExitReason(value.state, value.exit_reason) === "process_exit"
  );
}

export function shouldHideSessionFromOverview(session: SessionSummary): boolean {
  return isProcessExitState(session);
}
