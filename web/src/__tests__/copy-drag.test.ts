import { describe, expect, it } from "vitest";
import {
  computeCopyDragEdgeDirection,
  mapClientYToBufferRow,
} from "@/lib/copy-drag";

describe("copy drag helpers", () => {
  it("accelerates upward and downward near viewport edges", () => {
    expect(computeCopyDragEdgeDirection(175, 100, 700, 80)).toBe(-1);
    expect(computeCopyDragEdgeDirection(70, 100, 700, 80)).toBeLessThan(-1);
    expect(computeCopyDragEdgeDirection(621, 100, 700, 80)).toBe(1);
    expect(computeCopyDragEdgeDirection(730, 100, 700, 80)).toBeGreaterThan(1);
    expect(computeCopyDragEdgeDirection(360, 100, 700, 80)).toBe(0);
  });

  it("maps client Y to buffer rows across the viewport", () => {
    const topRow = mapClientYToBufferRow(100, 100, 600, 24, 300, 5000);
    const middleRow = mapClientYToBufferRow(400, 100, 600, 24, 300, 5000);
    const bottomRow = mapClientYToBufferRow(699, 100, 600, 24, 300, 5000);

    expect(topRow).toBe(300);
    expect(middleRow).toBe(312);
    expect(bottomRow).toBe(323);
  });

  it("clamps selection rows within available buffer", () => {
    expect(mapClientYToBufferRow(10, 100, 600, 24, 0, 30)).toBe(0);
    expect(mapClientYToBufferRow(9999, 100, 600, 24, 20, 30)).toBe(29);
  });
});
