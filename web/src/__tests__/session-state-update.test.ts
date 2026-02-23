import { describe, expect, it } from "vitest";
import type { SessionStatePayload } from "@/types";
import { applySessionStatePayload } from "@/app";
import { makeSession } from "./helpers/fixtures";

describe("applySessionStatePayload", () => {
  it("does not overwrite last_activity_at with transition timestamp", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "idle",
      current_command: null,
      last_activity_at: "2026-02-23T10:00:00.000Z",
    });

    const payload: SessionStatePayload = {
      state: "busy",
      previous_state: "idle",
      current_command: "npm test",
      transport_health: "healthy",
      at: "2026-02-23T10:00:05.000Z",
    };

    const updated = applySessionStatePayload(session, payload);
    expect(updated.last_activity_at).toBe("2026-02-23T10:00:00.000Z");
    expect(updated.state).toBe("busy");
    expect(updated.current_command).toBe("npm test");
  });

  it("returns the same object when state payload has no effective changes", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "idle",
      current_command: null,
      transport_health: "healthy",
    });

    const payload: SessionStatePayload = {
      state: "idle",
      previous_state: "busy",
      current_command: null,
      transport_health: "healthy",
      at: "2026-02-23T10:00:10.000Z",
    };

    expect(applySessionStatePayload(session, payload)).toBe(session);
  });
});
