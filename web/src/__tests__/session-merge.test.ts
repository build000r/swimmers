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

  it("adopts newer last_activity_at from poll fallback snapshots", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        last_activity_at: "2026-02-23T10:00:00.000Z",
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "busy",
        last_activity_at: "2026-02-23T10:00:45.000Z",
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].last_activity_at).toBe("2026-02-23T10:00:45.000Z");
  });

  it("does not regress last_activity_at when poll snapshot is older", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "busy",
        last_activity_at: "2026-02-23T10:00:45.000Z",
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        last_activity_at: "2026-02-23T10:00:00.000Z",
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].last_activity_at).toBe("2026-02-23T10:00:45.000Z");
  });

  it("accepts explicit null thought fields from poll snapshots", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        thought: "old thought",
        thought_state: "active",
        thought_source: "llm",
        thought_updated_at: "2026-02-23T10:00:00.000Z",
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "busy",
        thought: null,
        thought_state: "holding",
        thought_source: "carry_forward",
        thought_updated_at: null,
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].thought).toBeNull();
    expect(merged?.[0].thought_state).toBe("holding");
    expect(merged?.[0].thought_source).toBe("carry_forward");
    expect(merged?.[0].thought_updated_at).toBeNull();
  });

  it("applies sprite_pack_id when poll snapshot fills it in", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        tool: null,
        cwd: "/Users/rjb/repos/canva",
        sprite_pack_id: null,
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        tool: null,
        cwd: "/Users/rjb/repos/canva",
        sprite_pack_id: "/Users/rjb/repos/canva",
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].sprite_pack_id).toBe("/Users/rjb/repos/canva");
  });

  it("keeps no-op short-circuit when sprite_pack_id is unchanged", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        sprite_pack_id: "/Users/rjb/repos/canva",
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        sprite_pack_id: "/Users/rjb/repos/canva",
      }),
    ];

    expect(mergePollSessions(prev, next, new Set())).toBeNull();
  });

  it("does not regress last_activity_at while adopting sprite_pack_id", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "busy",
        last_activity_at: "2026-03-01T17:00:45.000Z",
        sprite_pack_id: null,
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        last_activity_at: "2026-03-01T17:00:00.000Z",
        sprite_pack_id: "/Users/rjb/repos/canva",
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].last_activity_at).toBe("2026-03-01T17:00:45.000Z");
    expect(merged?.[0].sprite_pack_id).toBe("/Users/rjb/repos/canva");
  });

  it("applies repo_theme_id when poll snapshot fills it in", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        cwd: "/Users/rjb/repos/canva",
        repo_theme_id: null,
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        cwd: "/Users/rjb/repos/canva",
        repo_theme_id: "/Users/rjb/repos/canva",
      }),
    ];

    const merged = mergePollSessions(prev, next, new Set());
    expect(merged).not.toBeNull();
    expect(merged?.[0].repo_theme_id).toBe("/Users/rjb/repos/canva");
  });

  it("keeps no-op short-circuit when repo_theme_id is unchanged", () => {
    const prev = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        repo_theme_id: "/Users/rjb/repos/canva",
      }),
    ];

    const next = [
      makeSession({
        session_id: "sess_0",
        state: "idle",
        repo_theme_id: "/Users/rjb/repos/canva",
      }),
    ];

    expect(mergePollSessions(prev, next, new Set())).toBeNull();
  });
});
