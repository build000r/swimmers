import { describe, it, expect } from "vitest";
import {
  DEEP_SLEEP_AFTER_MS,
  DROWSY_AFTER_MS,
  SLEEPING_AFTER_MS,
  throngletRestStageForSession,
} from "@/lib/thronglet-motion";

function isoAgo(ms: number): string {
  return new Date(Date.now() - ms).toISOString();
}

describe("throngletRestStageForSession", () => {
  it("keeps non-resting states active", () => {
    expect(throngletRestStageForSession("busy", isoAgo(10_000_000))).toBe("active");
    expect(throngletRestStageForSession("error", isoAgo(10_000_000))).toBe("active");
  });

  it("transitions idle sessions through drowsy, sleeping, and deep sleep", () => {
    expect(throngletRestStageForSession("idle", isoAgo(DROWSY_AFTER_MS - 1))).toBe("active");
    expect(throngletRestStageForSession("idle", isoAgo(DROWSY_AFTER_MS + 1))).toBe("drowsy");
    expect(throngletRestStageForSession("idle", isoAgo(SLEEPING_AFTER_MS + 1))).toBe("sleeping");
    expect(throngletRestStageForSession("idle", isoAgo(DEEP_SLEEP_AFTER_MS + 1))).toBe("deep_sleep");
  });

  it("applies rest stages to attention as well", () => {
    expect(throngletRestStageForSession("attention", isoAgo(DROWSY_AFTER_MS + 1))).toBe(
      "drowsy",
    );
    expect(throngletRestStageForSession("attention", isoAgo(SLEEPING_AFTER_MS + 1))).toBe(
      "sleeping",
    );
  });

  it("treats exited as deep sleep", () => {
    expect(throngletRestStageForSession("exited", isoAgo(1_000))).toBe("deep_sleep");
  });

  it("defaults resting sessions to drowsy when activity is unavailable", () => {
    expect(throngletRestStageForSession("idle", undefined)).toBe("drowsy");
    expect(throngletRestStageForSession("idle", "not-a-date")).toBe("drowsy");
  });
});
