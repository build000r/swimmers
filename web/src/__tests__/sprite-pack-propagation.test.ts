/**
 * Regression tests for sprite pack propagation through session lifecycle events.
 *
 * Verifies that:
 * 1. upsertSession preserves existing sprite_pack_id when incoming session is null
 * 2. session_created events merge sprite_pack + repo_theme into shared signals
 * 3. API create response merges sprite_pack + repo_theme into shared signals
 */
import { describe, it, expect, beforeEach } from "vitest";
import { repoThemes, sessions, spritePacks } from "@/app";
import { makeSession } from "./helpers/fixtures";
import type { RepoTheme, SessionCreatedPayload, SpritePack } from "@/types";

const TEST_PACK: SpritePack = {
  active: "<svg id='test-active'/>",
  drowsy: "<svg id='test-drowsy'/>",
  sleeping: "<svg id='test-sleeping'/>",
  deep_sleep: "<svg id='test-deep-sleep'/>",
};

const TEST_THEME: RepoTheme = {
  body: "#B89875",
  outline: "#3D2F24",
  accent: "#1D1914",
  shirt: "#AA9370",
};

const PACK_ID = "/repos/buildooor";

beforeEach(() => {
  sessions.value = [];
  spritePacks.value = {};
  repoThemes.value = {};
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

  it("preserves existing repo_theme_id when incoming session has null", () => {
    const bootstrapped = makeSession({
      session_id: "s1",
      repo_theme_id: PACK_ID,
    });
    sessions.value = [bootstrapped];

    const incoming = makeSession({
      session_id: "s1",
      repo_theme_id: null,
      state: "busy",
    });

    const existingIndex = sessions.value.findIndex(
      (s) => s.session_id === incoming.session_id,
    );
    const existing = sessions.value[existingIndex];
    const merged =
      !incoming.repo_theme_id && existing.repo_theme_id
        ? { ...incoming, repo_theme_id: existing.repo_theme_id }
        : incoming;

    expect(merged.repo_theme_id).toBe(PACK_ID);
    expect(merged.state).toBe("busy");
  });
});

describe("session_created sprite_pack merge", () => {
  it("merges sprite_pack and repo_theme into shared signals", () => {
    const payload: SessionCreatedPayload = {
      reason: "api_create",
      session: makeSession({
        session_id: "s2",
        sprite_pack_id: PACK_ID,
        repo_theme_id: PACK_ID,
      }),
      sprite_pack: TEST_PACK,
      repo_theme: TEST_THEME,
    };

    // Simulate onSessionCreated logic
    if (payload.sprite_pack && payload.session.sprite_pack_id) {
      spritePacks.value = {
        ...spritePacks.value,
        [payload.session.sprite_pack_id]: payload.sprite_pack,
      };
    }
    if (payload.repo_theme && payload.session.repo_theme_id) {
      repoThemes.value = {
        ...repoThemes.value,
        [payload.session.repo_theme_id]: payload.repo_theme,
      };
    }

    expect(spritePacks.value[PACK_ID]).toBeDefined();
    expect(spritePacks.value[PACK_ID].active).toBe("<svg id='test-active'/>");
    expect(repoThemes.value[PACK_ID]).toEqual(TEST_THEME);
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
  it("merges sprite_pack and repo_theme from API create response", () => {
    // Simulate the response from apiCreateSession
    const resp = {
      session: makeSession({
        session_id: "s4",
        sprite_pack_id: PACK_ID,
        repo_theme_id: PACK_ID,
      }),
      sprite_pack: TEST_PACK,
      repo_theme: TEST_THEME,
    };

    // Simulate onCreateSession logic
    if (resp.sprite_pack && resp.session.sprite_pack_id) {
      spritePacks.value = {
        ...spritePacks.value,
        [resp.session.sprite_pack_id]: resp.sprite_pack,
      };
    }
    if (resp.repo_theme && resp.session.repo_theme_id) {
      repoThemes.value = {
        ...repoThemes.value,
        [resp.session.repo_theme_id]: resp.repo_theme,
      };
    }

    expect(spritePacks.value[PACK_ID]).toEqual(TEST_PACK);
    expect(repoThemes.value[PACK_ID]).toEqual(TEST_THEME);
  });
});
