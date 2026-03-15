import type { RestState } from "@/types";

export type ThrongletRestStage =
  | "active"
  | "drowsy"
  | "sleeping"
  | "deep_sleep";

export function throngletRestStageForSession(
  restState?: RestState | null,
): ThrongletRestStage {
  return restState ?? "active";
}
