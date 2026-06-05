import test from "node:test";
import assert from "node:assert/strict";

import { createDirBrowserController } from "./dir_browser_controller.js";

function createRuntime(overrides = {}) {
  const storage = new Map();
  const state = {
    readOnly: true,
    dirBrowser: {
      path: "",
      managedOnly: false,
      entries: [],
      groups: [],
      group: "",
      search: "",
      overlayLabel: "",
      launchTargets: [],
      launchTarget: "local",
      batchSelected: new Set(),
      status: "",
      error: "",
    },
  };
  const el = {
    createCwd: { value: "" },
    createLaunchTarget: null,
    createTool: { value: "grok" },
    dirsManagedOnly: { checked: false },
    dirsPath: { value: "" },
  };
  let syncCount = 0;
  const statuses = [];
  return {
    runtime: {
      state,
      el,
      apiFetch: async () => ({ json: async () => ({}) }),
      setDirStatus(message, isError = false) {
        statuses.push({ message, isError });
        state.dirBrowser.status = message;
        state.dirBrowser.error = isError ? message : "";
      },
      syncSheetActionAvailability() {
        syncCount += 1;
      },
      storage: {
        getItem(key) {
          return storage.has(key) ? storage.get(key) : null;
        },
        setItem(key, value) {
          storage.set(key, String(value));
        },
        removeItem(key) {
          storage.delete(key);
        },
      },
      pathStorageKey: "dirs.path",
      managedOnlyStorageKey: "dirs.managed",
      ...overrides,
    },
    state,
    el,
    storage,
    statuses,
    syncCount: () => syncCount,
  };
}

test("directory browser controller delegates dynamic view rendering while preserving state ownership", () => {
  const views = [];
  const { runtime, state, el, storage, statuses, syncCount } = createRuntime({
    renderDirBrowserView(view) {
      views.push(view);
      return true;
    },
  });
  state.dirBrowser.group = "core";
  state.dirBrowser.search = "dirty";
  state.dirBrowser.batchSelected = new Set(["/srv/repos/swimmers", "/stale"]);
  el.dirsManagedOnly.checked = true;

  const controller = createDirBrowserController(runtime);
  controller.renderDirEntries({
    path: "/srv/repos",
    overlay_label: "Managed Repos",
    groups: ["core", "clients"],
    entries: [
      {
        name: "swimmers",
        full_path: "/srv/repos/swimmers",
        groups: ["core"],
        repo_dirty: true,
      },
      {
        name: "clean",
        full_path: "/srv/repos/clean",
      },
    ],
  });

  assert.equal(views.length, 1);
  assert.deepEqual(views[0].entries.map((entry) => entry.name), ["swimmers"]);
  assert.deepEqual(views[0].groups, ["core", "clients"]);
  assert.equal(views[0].path, "/srv/repos");
  assert.equal(views[0].activeGroup, "core");
  assert.equal(views[0].managed, true);
  assert.equal(views[0].overlayLabel, "managed repos");
  assert.equal(views[0].readOnly, true);
  assert.equal(views[0].search, "dirty");
  assert.equal(views[0].selectedPaths, state.dirBrowser.batchSelected);
  assert.deepEqual(Array.from(state.dirBrowser.batchSelected), ["/srv/repos/swimmers"]);
  assert.deepEqual(state.dirBrowser.entries.map((entry) => entry.name), ["swimmers", "clean"]);
  assert.deepEqual(state.dirBrowser.groups, ["core", "clients"]);
  assert.equal(state.dirBrowser.path, "/srv/repos");
  assert.equal(state.dirBrowser.overlayLabel, "Managed Repos");
  assert.equal(el.dirsPath.value, "/srv/repos");
  assert.equal(el.createCwd.value, "/srv/repos");
  assert.equal(storage.get("dirs.path"), "/srv/repos");
  assert.equal(storage.get("dirs.managed"), "true");
  assert.deepEqual(statuses, [{
    message: "1 entries at /srv/repos (managed only) · group core · 1/2 search matches",
    isError: false,
  }]);
  assert.equal(syncCount(), 1);
});
