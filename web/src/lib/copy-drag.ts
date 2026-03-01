function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function computeCopyDragEdgeDirection(
  clientY: number,
  top: number,
  bottom: number,
  edgePx: number,
  maxStep = 4,
): number {
  const safeEdge = Math.max(1, edgePx);
  const ramp = Math.max(8, Math.round(safeEdge / 4));

  const topBand = top + safeEdge;
  if (clientY < topBand) {
    const depth = topBand - clientY;
    return -Math.min(maxStep, Math.max(1, Math.ceil(depth / ramp)));
  }

  const bottomBand = bottom - safeEdge;
  if (clientY > bottomBand) {
    const depth = clientY - bottomBand;
    return Math.min(maxStep, Math.max(1, Math.ceil(depth / ramp)));
  }

  return 0;
}

export function mapClientYToBufferRow(
  clientY: number,
  viewportTop: number,
  viewportHeight: number,
  rows: number,
  viewportY: number,
  bufferLength: number,
): number {
  const safeHeight = Math.max(1, viewportHeight);
  const safeRows = Math.max(1, rows);
  const relativeY = clamp(clientY - viewportTop, 0, Math.max(0, safeHeight - 1));
  const rowOffset = Math.min(
    safeRows - 1,
    Math.floor((relativeY / safeHeight) * safeRows),
  );
  const maxRow = Math.max(0, bufferLength - 1);
  return clamp(viewportY + rowOffset, 0, maxRow);
}
