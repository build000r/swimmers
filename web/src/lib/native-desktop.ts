import type { NativeDesktopStatus } from "@/types";

export function shouldOpenNativeDesktopByDefault(
  status: NativeDesktopStatus | null | undefined,
  observer: boolean,
  preferZone: "main" | "bottom" | null = null,
): boolean {
  return !!status?.supported && !observer && preferZone === null;
}
