import { describe, expect, it } from "vitest";
import { mergePollSessions } from "@/app";
import { makeSession } from "./helpers/fixtures";

describe("mergePollSessions", () => {
  it("preserves explicit zero token_count/context_limit values", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        token_count: 123,
        context_limit: 200000,
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "busy", // force merge path instead of identity short-circuit
        token_count: 0,
        context_limit: 0,
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].token_count).toBe(0);
    expect(merged?.[0].context_limit).toBe(0);
  });
});
