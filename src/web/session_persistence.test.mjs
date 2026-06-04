import test from "node:test";
import assert from "node:assert/strict";

import {
  createSessionPersistenceController,
  sessionUrlForSelection,
} from "./session_persistence.js";

function storageFake() {
  const values = new Map();
  return {
    values,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };
}

function windowFake(href = "http://localhost:3210/?token=secret") {
  const calls = [];
  return {
    calls,
    location: new URL(href),
    history: {
      replaceState: (...args) => calls.push(args),
    },
  };
}

function documentFake() {
  const classes = new Set();
  return {
    classes,
    body: {
      classList: {
        toggle: (name, enabled) => {
          if (enabled) {
            classes.add(name);
          } else {
            classes.delete(name);
          }
        },
      },
    },
  };
}

test("sessionUrlForSelection preserves selected, follow, and token cleanup rules", () => {
  assert.equal(
    sessionUrlForSelection({
      href: "http://localhost:3210/?token=secret&follow=published",
      pathname: "/",
      selectedSessionId: "sess_1",
    }).toString(),
    "http://localhost:3210/?session=sess_1",
  );

  assert.equal(
    sessionUrlForSelection({
      href: "http://localhost:3210/?token=secret&session=sess_1",
      pathname: "/",
      followPublishedSelection: true,
      selectedSessionId: "sess_1",
    }).toString(),
    "http://localhost:3210/?follow=published",
  );

  assert.equal(
    sessionUrlForSelection({
      href: "http://localhost:3210/selected?token=secret&session=sess_1&follow=published",
      pathname: "/selected",
      followPublishedSelection: true,
      selectedSessionId: "sess_1",
    }).toString(),
    "http://localhost:3210/selected",
  );

  assert.equal(
    sessionUrlForSelection({
      href: "http://localhost:3210/?token=secret&session=sess_1&follow=published",
      pathname: "/",
    }).toString(),
    "http://localhost:3210/",
  );
});

test("persistSelectedSession preserves storage, reset, atlas, and URL side effects", () => {
  const state = {
    followPublishedSelection: false,
    selectedSessionId: "old",
  };
  const storage = storageFake();
  const win = windowFake("http://localhost:3210/?token=secret");
  const resets = [];
  const atlas = [];
  const controller = createSessionPersistenceController({
    state,
    windowRef: win,
    documentRef: documentFake(),
    storage,
    sessionStorageKey: "selected",
    normalizeSessionId: (sessionId) => String(sessionId || "").trim() || null,
    resetAgentContextForSession: (sessionId) => resets.push(["agent", sessionId]),
    resetWorkbenchWidgetsForSession: (sessionId) => resets.push(["widgets", sessionId]),
    closeTrogdorAtlasForTerminal: () => atlas.push("closed"),
  });

  controller.persistSelectedSession(" new ");
  assert.equal(state.selectedSessionId, "new");
  assert.equal(storage.values.get("selected"), "new");
  assert.deepEqual(resets, [["agent", "new"], ["widgets", "new"]]);
  assert.deepEqual(atlas, ["closed"]);
  assert.equal(win.calls[0][2].toString(), "http://localhost:3210/?session=new");

  controller.persistSelectedSession("new", { syncUrl: false });
  assert.deepEqual(resets, [["agent", "new"], ["widgets", "new"]]);
  assert.equal(win.calls.length, 1);

  controller.persistSelectedSession(null);
  assert.equal(state.selectedSessionId, null);
  assert.equal(storage.values.has("selected"), false);
  assert.equal(win.calls[1][2].toString(), "http://localhost:3210/");
});

test("setFollowPublishedSelection preserves body class, URL sync, and render call", () => {
  const state = {
    followPublishedSelection: false,
    selectedSessionId: "sess_1",
  };
  const doc = documentFake();
  const win = windowFake("http://localhost:3210/?session=sess_1");
  const renderCalls = [];
  const controller = createSessionPersistenceController({
    state,
    windowRef: win,
    documentRef: doc,
    storage: storageFake(),
    sessionStorageKey: "selected",
    renderHudSurface: () => renderCalls.push("render"),
  });

  controller.setFollowPublishedSelection(true);
  assert.equal(state.followPublishedSelection, true);
  assert.equal(doc.classes.has("following-published"), true);
  assert.equal(win.calls[0][2].toString(), "http://localhost:3210/?follow=published");
  assert.deepEqual(renderCalls, ["render"]);

  controller.setFollowPublishedSelection(false, { skipUrlSync: true });
  assert.equal(state.followPublishedSelection, false);
  assert.equal(doc.classes.has("following-published"), false);
  assert.equal(win.calls.length, 1);
  assert.deepEqual(renderCalls, ["render", "render"]);
});
