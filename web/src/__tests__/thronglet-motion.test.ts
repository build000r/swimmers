import { describe, it, expect } from "vitest";
import { throngletRestStageForSession } from "@/lib/thronglet-motion";

describe("throngletRestStageForSession", () => {
  it("returns active when rest state is absent", () => {
    expect(throngletRestStageForSession()).toBe("active");
    expect(throngletRestStageForSession(null)).toBe("active");
  });

  it("passes through daemon-authored rest states", () => {
    expect(throngletRestStageForSession("active")).toBe("active");
    expect(throngletRestStageForSession("drowsy")).toBe("drowsy");
    expect(throngletRestStageForSession("sleeping")).toBe("sleeping");
    expect(throngletRestStageForSession("deep_sleep")).toBe("deep_sleep");
  });
});
