export interface DevicePlatformSnapshot {
  userAgent: string;
  platform: string;
  maxTouchPoints: number;
}

const IOS_DEVICE_RE = /iPad|iPhone|iPod/i;

export function isLikelyIOSDevice(snapshot: DevicePlatformSnapshot): boolean {
  if (IOS_DEVICE_RE.test(snapshot.userAgent)) return true;
  // iPadOS can report "MacIntel" while still being touch-capable.
  return snapshot.platform === "MacIntel" && snapshot.maxTouchPoints > 1;
}

export function shouldEnableTerminalWebgl(
  snapshot: DevicePlatformSnapshot,
  override: string | null,
): boolean {
  const normalized = (override ?? "").trim().toLowerCase();
  if (normalized === "on") return true;
  if (normalized === "off") return false;
  return !isLikelyIOSDevice(snapshot);
}

export function computeVisualViewportBottomInsetPx(
  innerHeight: number,
  viewportHeight: number,
  viewportOffsetTop: number,
): number {
  const overlap = Math.max(0, innerHeight - (viewportHeight + viewportOffsetTop));
  return Math.round(overlap);
}

export function hasMeaningfulDelta(
  previous: number,
  next: number,
  minDeltaPx: number,
): boolean {
  if (previous === next) return false;
  if (minDeltaPx <= 0) return true;
  return Math.abs(next - previous) >= minDeltaPx;
}
