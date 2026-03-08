import { describe, expect, it } from "vitest";
import {
  hasDistinctHostSize,
  hasDistinctTerminalGridSize,
  isHeightOnlyHostResize,
  shouldFitTerminalHostResize,
  type TerminalGridSize,
} from "@/lib/terminal-resize";

describe("terminal resize guards", () => {
  it("ignores viewport resize churn when host geometry is unchanged", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 390, height: 724 };
    expect(hasDistinctHostSize(previous, next)).toBe(false);
  });

  it("detects true terminal host geometry changes", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 844, height: 390 };
    expect(hasDistinctHostSize(previous, next)).toBe(true);
  });

  it("flags height-only host resize", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 390, height: 676 };
    expect(isHeightOnlyHostResize(previous, next)).toBe(true);
  });

  it("does not flag width changes as height-only", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 386, height: 700 };
    expect(isHeightOnlyHostResize(previous, next)).toBe(false);
  });

  it("flags large height changes when width is unchanged", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 390, height: 520 };
    expect(isHeightOnlyHostResize(previous, next)).toBe(true);
  });

  it("ignores height-only viewport changes when suppression is enabled", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 390, height: 676 };
    expect(
      shouldFitTerminalHostResize(previous, next, {
        ignoreHeightOnly: true,
        source: "viewport",
      }),
    ).toBe(false);
  });

  it("still fits height-only container changes when suppression is enabled", () => {
    const previous = { width: 390, height: 724 };
    const next = { width: 390, height: 676 };
    expect(
      shouldFitTerminalHostResize(previous, next, {
        ignoreHeightOnly: true,
        source: "container",
      }),
    ).toBe(true);
  });

  it("deduplicates delayed duplicate PTY resize emits", () => {
    const sent: TerminalGridSize[] = [];
    let previous: TerminalGridSize | null = null;
    const emit = (next: TerminalGridSize) => {
      if (!hasDistinctTerminalGridSize(previous, next)) return;
      previous = next;
      sent.push(next);
    };

    emit({ cols: 84, rows: 30 });
    emit({ cols: 84, rows: 30 }); // late duplicate iOS wave
    emit({ cols: 84, rows: 30 }); // another duplicate

    expect(sent).toEqual([{ cols: 84, rows: 30 }]);
  });
});
