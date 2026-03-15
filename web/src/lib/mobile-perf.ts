export interface DevicePlatformSnapshot {
  userAgent: string;
  platform: string;
  maxTouchPoints: number;
}

const IOS_DEVICE_RE = /iPad|iPhone|iPod/i;
const IPAD_DESKTOP_UA_RE = /\bMacintosh\b.*\bMobile\/[\w.]+/i;
const IOS_VERSION_RE = /\bOS (\d+)[._]\d+/i;
const SAFARI_VERSION_RE = /\bVersion\/(\d+)(?:\.\d+)?/i;

export function isLikelyIOSDevice(snapshot: DevicePlatformSnapshot): boolean {
  if (IOS_DEVICE_RE.test(snapshot.userAgent)) return true;
  // iPadOS Safari desktop mode can report MacIntel; gate on the Mobile token
  // so macOS desktops with synthetic touch points do not get classified as iOS.
  return (
    snapshot.platform === "MacIntel" &&
    snapshot.maxTouchPoints > 1 &&
    IPAD_DESKTOP_UA_RE.test(snapshot.userAgent)
  );
}

function readLikelyIOSWebkitMajorVersion(
  snapshot: DevicePlatformSnapshot,
): number | null {
  const iosMatch = snapshot.userAgent.match(IOS_VERSION_RE);
  if (iosMatch) {
    const major = Number.parseInt(iosMatch[1] ?? "", 10);
    if (Number.isFinite(major)) return major;
  }
  const safariMatch = snapshot.userAgent.match(SAFARI_VERSION_RE);
  if (safariMatch) {
    const major = Number.parseInt(safariMatch[1] ?? "", 10);
    if (Number.isFinite(major)) return major;
  }
  return null;
}

export function shouldEnableTerminalWebgl(
  snapshot: DevicePlatformSnapshot,
  override: string | null,
): boolean {
  const normalized = (override ?? "").trim().toLowerCase();
  if (normalized === "on") return true;
  if (normalized === "off") return false;
  if (!isLikelyIOSDevice(snapshot)) return true;
  const iosWebkitMajor = readLikelyIOSWebkitMajorVersion(snapshot);
  return iosWebkitMajor !== null && iosWebkitMajor >= 16;
}

export function shouldIgnoreHeightOnlyTerminalFit(
  snapshot: DevicePlatformSnapshot,
  isMobileViewport: boolean,
): boolean {
  return isMobileViewport || isLikelyIOSDevice(snapshot);
}

export function computeVisualViewportBottomInsetPx(
  innerHeight: number,
  viewportHeight: number,
  viewportOffsetTop: number,
): number {
  const safeInnerHeight = Number.isFinite(innerHeight) ? innerHeight : 0;
  const safeViewportHeight = Number.isFinite(viewportHeight)
    ? viewportHeight
    : safeInnerHeight;
  const safeViewportOffsetTop = Number.isFinite(viewportOffsetTop)
    ? viewportOffsetTop
    : 0;
  const overlap = Math.max(
    0,
    safeInnerHeight - (safeViewportHeight + safeViewportOffsetTop),
  );
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
