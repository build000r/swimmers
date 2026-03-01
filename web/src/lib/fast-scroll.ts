function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function hasFastScrollOverflow(
  scrollHeight: number,
  clientHeight: number,
): boolean {
  return scrollHeight > clientHeight + 1;
}

export function isNearScrollBottom(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
  tolerance = 6,
): boolean {
  const remaining = scrollHeight - clientHeight - scrollTop;
  return remaining <= tolerance;
}

export function computeFastScrollThumbHeight(
  clientHeight: number,
  scrollHeight: number,
  trackHeight: number,
  minThumbPx: number,
): number {
  if (trackHeight <= 0) return 0;
  if (scrollHeight <= 0 || clientHeight <= 0) return trackHeight;
  const ratio = clientHeight / scrollHeight;
  const raw = Math.round(trackHeight * ratio);
  return clamp(raw, Math.min(minThumbPx, trackHeight), trackHeight);
}

export function computeFastScrollThumbTop(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
  trackHeight: number,
  thumbHeight: number,
): number {
  const scrollRange = Math.max(0, scrollHeight - clientHeight);
  const trackRange = Math.max(0, trackHeight - thumbHeight);
  if (scrollRange === 0 || trackRange === 0) return 0;
  const ratio = clamp(scrollTop / scrollRange, 0, 1);
  return Math.round(trackRange * ratio);
}

export function computeScrollTopFromThumbOffset(
  thumbOffset: number,
  scrollHeight: number,
  clientHeight: number,
  trackHeight: number,
  thumbHeight: number,
): number {
  const scrollRange = Math.max(0, scrollHeight - clientHeight);
  const trackRange = Math.max(0, trackHeight - thumbHeight);
  if (scrollRange === 0 || trackRange === 0) return 0;
  const ratio = clamp(thumbOffset / trackRange, 0, 1);
  return scrollRange * ratio;
}
