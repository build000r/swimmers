import type { SessionState } from "@/types";

export const DROWSY_AFTER_MS = 20_000;
export const SLEEPING_AFTER_MS = 60_000;
export const DEEP_SLEEP_AFTER_MS = 120_000;

export type ThrongletRestStage =
  | "active"
  | "drowsy"
  | "sleeping"
  | "deep_sleep";

export function throngletRestStageForSession(
  state: SessionState,
  lastActivityAt?: string,
  nowMs = Date.now(),
): ThrongletRestStage {
  if (state === "exited") return "deep_sleep";
  if (state !== "idle" && state !== "attention") return "active";

  if (!lastActivityAt) return "drowsy";
  const lastMs = Date.parse(lastActivityAt);
  if (!Number.isFinite(lastMs)) return "drowsy";

  const idleMs = Math.max(0, nowMs - lastMs);
  if (idleMs >= DEEP_SLEEP_AFTER_MS) return "deep_sleep";
  if (idleMs >= SLEEPING_AFTER_MS) return "sleeping";
  if (idleMs >= DROWSY_AFTER_MS) return "drowsy";
  return "active";
}
