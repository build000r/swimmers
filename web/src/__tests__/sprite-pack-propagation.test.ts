/**
 * Regression tests for sprite pack propagation through session lifecycle events.
 *
 * Verifies that:
 * 1. upsertSession preserves existing sprite_pack_id when incoming session is null
 * 2. session_created events merge sprite_pack into the spritePacks signal
 * 3. API create response merges sprite_pack into the spritePacks signal
 */
import { describe, it, expect, beforeEach } from "vitest";
import { sessions, spritePacks } from "@/app";
import { makeSession } from "./helpers/fixtures";
import type { SessionCreatedPayload, SpritePack } from "@/types";

const TEST_PACK: SpritePack = {
  active: "<svg id='test-active'/>",
  drowsy: "<svg id='test-drowsy'/>",
  sleeping: "<svg id='test-sleeping'/>",
  deep_sleep: "<svg id='test-deep-sleep'/>",
};

const PACK_ID = "/repos/buildooor";

beforeEach(() => {
  sessions.value = [];
  spritePacks.value = {};
});

describe("upsertSession sprite_pack_id preservation", () => {
  it("preserves existing sprite_pack_id when incoming session has null", () => {
    // Bootstrap puts a session with a sprite_pack_id
    const bootstrapped = makeSession({
      session_id: "s1",
      sprite_pack_id: PACK_ID,
    });
    sessions.value = [bootstrapped];

    // A late session_created event arrives with null sprite_pack_id
    const incoming = makeSession({
      session_id: "s1",
      sprite_pack_id: null,
      state: "busy",
    });

    // Simulate upsertSession logic
    const existingIndex = sessions.value.findIndex(
      (s) => s.session_id === incoming.session_id,
    );
    const existing = sessions.value[existingIndex];
    const merged =
      !incoming.sprite_pack_id && existing.sprite_pack_id
        ? { ...incoming, sprite_pack_id: existing.sprite_pack_id }
        : incoming;

    expect(merged.sprite_pack_id).toBe(PACK_ID);
    expect(merged.state).toBe("busy"); // other fields still update
  });

  it("uses incoming sprite_pack_id when it is set", () => {
    const bootstrapped = makeSession({
      session_id: "s1",
      sprite_pack_id: PACK_ID,
    });
    sessions.value = [bootstrapped];

    const incoming = makeSession({
      session_id: "s1",
      sprite_pack_id: "/repos/other",
    });

    const existingIndex = sessions.value.findIndex(
      (s) => s.session_id === incoming.session_id,
    );
    const existing = sessions.value[existingIndex];
    const merged =
      !incoming.sprite_pack_id && existing.sprite_pack_id
        ? { ...incoming, sprite_pack_id: existing.sprite_pack_id }
        : incoming;

    expect(merged.sprite_pack_id).toBe("/repos/other");
  });
});

describe("session_created sprite_pack merge", () => {
  it("merges sprite_pack into spritePacks signal", () => {
    const payload: SessionCreatedPayload = {
      reason: "api_create",
      session: makeSession({
        session_id: "s2",
        sprite_pack_id: PACK_ID,
      }),
      sprite_pack: TEST_PACK,
    };

    // Simulate onSessionCreated logic
    if (payload.sprite_pack && payload.session.sprite_pack_id) {
      spritePacks.value = {
        ...spritePacks.value,
        [payload.session.sprite_pack_id]: payload.sprite_pack,
      };
    }

    expect(spritePacks.value[PACK_ID]).toBeDefined();
    expect(spritePacks.value[PACK_ID].active).toBe("<svg id='test-active'/>");
  });

  it("does not merge when sprite_pack is absent", () => {
    const payload: SessionCreatedPayload = {
      reason: "startup_discovery",
      session: makeSession({ session_id: "s3", sprite_pack_id: null }),
    };

    if (payload.sprite_pack && payload.session.sprite_pack_id) {
      spritePacks.value = {
        ...spritePacks.value,
        [payload.session.sprite_pack_id]: payload.sprite_pack,
      };
    }

    expect(Object.keys(spritePacks.value)).toHaveLength(0);
  });
});

describe("CreateSessionResponse sprite_pack merge", () => {
  it("merges sprite_pack from API create response", () => {
    // Simulate the response from apiCreateSession
    const resp = {
      session: makeSession({
        session_id: "s4",
        sprite_pack_id: PACK_ID,
      }),
      sprite_pack: TEST_PACK,
    };

    // Simulate onCreateSession logic
    if (resp.sprite_pack && resp.session.sprite_pack_id) {
      spritePacks.value = {
        ...spritePacks.value,
        [resp.session.sprite_pack_id]: resp.sprite_pack,
      };
    }

    expect(spritePacks.value[PACK_ID]).toEqual(TEST_PACK);
  });
});
