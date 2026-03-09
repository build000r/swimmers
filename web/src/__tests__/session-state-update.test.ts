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
      exit_reason: null,
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

  it("stores process_exit on exited sessions and clears it on recovery", () => {
    const session = makeSession({
      session_id: "sess-exit",
      state: "idle",
      exit_reason: null,
      current_command: null,
      transport_health: "healthy",
    });

    const exited = applySessionStatePayload(session, {
      state: "exited",
      previous_state: "idle",
      current_command: null,
      transport_health: "healthy",
      exit_reason: "process_exit",
      at: "2026-03-06T15:00:00.000Z",
    });

    expect(exited.exit_reason).toBe("process_exit");

    const recovered = applySessionStatePayload(exited, {
      state: "idle",
      previous_state: "exited",
      current_command: null,
      transport_health: "healthy",
      at: "2026-03-06T15:00:01.000Z",
    });

    expect(recovered.exit_reason).toBeNull();
  });

  it("wakes sleeping sessions to active rest state on busy transition", () => {
    const session = makeSession({
      session_id: "sess-sleep",
      state: "idle",
      rest_state: "sleeping",
      thought_state: "sleeping",
      thought: "Sleeping.",
      transport_health: "healthy",
    });

    const updated = applySessionStatePayload(session, {
      state: "busy",
      previous_state: "idle",
      current_command: "npm test",
      transport_health: "healthy",
      at: "2026-03-08T15:00:00.000Z",
    });

    expect(updated.state).toBe("busy");
    expect(updated.rest_state).toBe("active");
  });

  it("maps exited sessions to deep sleep rest state", () => {
    const session = makeSession({
      session_id: "sess-exit-rest",
      state: "busy",
      rest_state: "active",
      thought: "still thinking",
      thought_state: "holding",
      thought_source: "llm",
      thought_updated_at: "2026-03-08T15:00:00.000Z",
      transport_health: "healthy",
    });

    const updated = applySessionStatePayload(session, {
      state: "exited",
      previous_state: "busy",
      current_command: null,
      transport_health: "healthy",
      exit_reason: "process_exit",
      at: "2026-03-08T15:00:01.000Z",
    });

    expect(updated.state).toBe("exited");
    expect(updated.rest_state).toBe("deep_sleep");
    expect(updated.thought).toBeNull();
    expect(updated.thought_state).toBe("holding");
    expect(updated.thought_source).toBe("carry_forward");
    expect(updated.thought_updated_at).toBeNull();
  });
});
