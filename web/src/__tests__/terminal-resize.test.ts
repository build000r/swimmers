import { describe, expect, it } from "vitest";
import {
  hasDistinctHostSize,
  hasDistinctTerminalGridSize,
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
