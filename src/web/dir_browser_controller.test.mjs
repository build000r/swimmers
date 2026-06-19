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
    createRequest: { value: "" },
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

function devboxTarget() {
  return {
    id: "devbox",
    label: "Devbox",
    kind: "swimmers_api",
    path_mappings: [{ local_prefix: "/workspace", remote_prefix: "/srv/workspace" }],
  };
}

function selectElement(value = "local") {
  return {
    value,
    innerHTML: "",
    children: [],
    appendChild(child) {
      this.children.push(child);
      return child;
    },
  };
}

function submitEvent() {
  return {
    prevented: false,
    preventDefault() {
      this.prevented = true;
    },
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
  assert.equal(views[0].groupActionsReadOnly, true);
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

test("directory browser controller scopes listings to the selected remote target", async () => {
  const calls = [];
  const { runtime, state } = createRuntime({
    apiFetch: async (url) => {
      calls.push(url);
      return {};
    },
    responseJson: async (_response, normalize) => normalize({
      path: "/workspace",
      entries: [],
      launch_targets: [devboxTarget()],
      default_launch_target: "devbox",
    }),
    location: new URL("http://swimmers.test/"),
    renderDirBrowserView() {
      return true;
    },
  });
  state.dirBrowser.launchTargets = [devboxTarget()];
  state.dirBrowser.launchTarget = "devbox";

  const controller = createDirBrowserController(runtime);
  await controller.loadDirListing("/workspace", true, "core");

  assert.equal(
    calls[0],
    "/v1/dirs?path=%2Fworkspace&managed_only=true&group=core&target=devbox",
  );
});

test("directory browser controller keeps local launch override for remote-only listings", () => {
  const previousDocument = globalThis.document;
  globalThis.document = {
    createElement(tagName) {
      return { tagName, value: "", textContent: "" };
    },
  };
  try {
    const { runtime, state, el } = createRuntime({
      renderDirBrowserView() {
        return true;
      },
    });
    el.createLaunchTarget = selectElement("local");

    const controller = createDirBrowserController(runtime);
    controller.renderDirEntries({
      path: "/workspace",
      entries: [],
      launch_targets: [devboxTarget()],
      default_launch_target: "devbox",
    });

    assert.deepEqual(state.dirBrowser.launchTargets.map((target) => target.id), ["local", "devbox"]);
    assert.deepEqual(el.createLaunchTarget.children.map((option) => option.value), ["local", "devbox"]);
    assert.equal(state.dirBrowser.launchTarget, "devbox");
    assert.equal(el.createLaunchTarget.value, "devbox");
  } finally {
    globalThis.document = previousDocument;
  }
});

test("directory browser controller resets stale remote target for cockpit cwd launch", () => {
  const opened = [];
  const { runtime, state, el, syncCount } = createRuntime({
    openSheet(sheet) {
      opened.push(sheet);
    },
  });
  state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
  state.dirBrowser.launchTarget = "devbox";
  state.dirBrowser.singleLaunchBlocker = { blocked: true };
  state.dirBrowser.batchLaunchBlockers = [{ localCwd: "/tmp/outside" }];
  state.dirBrowser.batchSelected = new Set(["/workspace/swimmers"]);
  el.createLaunchTarget = selectElement("devbox");

  const controller = createDirBrowserController(runtime);
  controller.openCreateSheetForCwd("/tmp/local-repo");

  assert.deepEqual(opened, ["create"]);
  assert.equal(el.createCwd.value, "/tmp/local-repo");
  assert.equal(el.dirsPath.value, "/tmp/local-repo");
  assert.equal(state.dirBrowser.path, "/tmp/local-repo");
  assert.equal(state.dirBrowser.launchTarget, "local");
  assert.equal(el.createLaunchTarget.value, "local");
  assert.equal(controller.launchTargetPayload(), null);
  assert.deepEqual(Array.from(state.dirBrowser.batchSelected), []);
  assert.deepEqual(state.dirBrowser.batchLaunchBlockers, []);
  assert.equal(state.dirBrowser.singleLaunchBlocker, null);
  assert.equal(syncCount(), 2);
});

test("directory browser controller preserves remote target for mapped cockpit launch", async () => {
  const opened = [];
  const calls = [];
  const { runtime, state, el } = createRuntime({
    apiFetch: async (...args) => {
      calls.push(args);
      return { json: async () => ({ session: { session_id: "devbox::sess_2" } }) };
    },
    openSheet(sheet) {
      opened.push(sheet);
    },
  });
  state.readOnly = false;
  state.dirBrowser.path = "/old";
  state.dirBrowser.entries = [{ name: "old", full_path: "/old/agent" }];
  state.dirBrowser.groups = ["old-group"];
  state.dirBrowser.overlayLabel = "Old overlay";
  state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
  state.dirBrowser.launchTarget = "local";
  el.createLaunchTarget = selectElement("local");
  el.createRequest.value = "continue there";

  const controller = createDirBrowserController(runtime);
  controller.openCreateSheetForCwd("/workspace/swimmers", { launchTarget: "devbox" });

  assert.deepEqual(opened, ["create"]);
  assert.equal(el.createCwd.value, "/workspace/swimmers");
  assert.equal(el.dirsPath.value, "/workspace/swimmers");
  assert.equal(state.dirBrowser.path, "/workspace/swimmers");
  assert.equal(state.dirBrowser.launchTarget, "devbox");
  assert.equal(el.createLaunchTarget.value, "devbox");
  assert.deepEqual(state.dirBrowser.entries, []);
  assert.deepEqual(state.dirBrowser.groups, []);
  assert.equal(state.dirBrowser.overlayLabel, "");

  await controller.createSessionFromSheet();

  assert.equal(calls[0][0], "/v1/sessions");
  assert.deepEqual(JSON.parse(calls[0][1].body), {
    cwd: "/workspace/swimmers",
    spawn_tool: "grok",
    launch_target: "devbox",
    initial_request: "continue there",
  });
});

test("directory browser controller reloads inventory when launch target changes", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = {
    createElement(tagName) {
      return { tagName, value: "", textContent: "" };
    },
  };
  try {
    const calls = [];
    const { runtime, state, el } = createRuntime({
      apiFetch: async (url) => {
        calls.push(url);
        return {};
      },
      responseJson: async (_response, normalize) => normalize({
        path: "/workspace",
        entries: [],
        launch_targets: [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()],
        default_launch_target: "devbox",
      }),
      location: new URL("http://swimmers.test/"),
      renderDirBrowserView() {
        return true;
      },
    });
    state.dirBrowser.path = "/workspace";
    state.dirBrowser.entries = [{ name: "swimmers", has_children: false }];
    state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
    state.dirBrowser.launchTarget = "local";
    el.dirsPath.value = "/workspace";
    el.createLaunchTarget = selectElement("devbox");

    const controller = createDirBrowserController(runtime);
    await controller.handleCreateLaunchTargetChange();

    assert.equal(state.dirBrowser.launchTarget, "devbox");
    assert.equal(calls.length, 1);
    assert.equal(calls[0], "/v1/dirs?path=%2Fworkspace&managed_only=false&target=devbox");
  } finally {
    globalThis.document = previousDocument;
  }
});

test("directory browser controller preserves explicit local target during reload", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = {
    createElement(tagName) {
      return { tagName, value: "", textContent: "" };
    },
  };
  try {
    const calls = [];
    const { runtime, state, el } = createRuntime({
      apiFetch: async (url) => {
        calls.push(url);
        return {};
      },
      responseJson: async (_response, normalize) => normalize({
        path: "/workspace",
        entries: [],
        launch_targets: [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()],
        default_launch_target: "devbox",
      }),
      location: new URL("http://swimmers.test/"),
      renderDirBrowserView() {
        return true;
      },
    });
    state.dirBrowser.path = "/workspace";
    state.dirBrowser.entries = [{ name: "swimmers", has_children: false }];
    state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
    state.dirBrowser.launchTarget = "devbox";
    el.dirsPath.value = "/workspace";
    el.createLaunchTarget = selectElement("local");

    const controller = createDirBrowserController(runtime);
    await controller.handleCreateLaunchTargetChange();

    assert.equal(state.dirBrowser.launchTarget, "local");
    assert.equal(el.createLaunchTarget.value, "local");
    assert.equal(calls.length, 1);
    assert.equal(calls[0], "/v1/dirs?path=%2Fworkspace&managed_only=false");
  } finally {
    globalThis.document = previousDocument;
  }
});

test("directory browser controller reverts target when target-change reload fails", async () => {
  const { runtime, state, el, statuses } = createRuntime({
    apiFetch: async () => {
      throw new Error("remote down");
    },
    location: new URL("http://swimmers.test/"),
  });
  state.dirBrowser.path = "/workspace";
  state.dirBrowser.entries = [{ name: "swimmers", has_children: false }];
  state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
  state.dirBrowser.launchTarget = "local";
  el.dirsPath.value = "/workspace";
  el.createLaunchTarget = selectElement("devbox");

  const controller = createDirBrowserController(runtime);
  await controller.handleCreateLaunchTargetChange();

  assert.equal(state.dirBrowser.launchTarget, "local");
  assert.equal(el.createLaunchTarget.value, "local");
  assert.deepEqual(statuses.at(-1), {
    message: "Failed to load directories: remote down",
    isError: true,
  });
});

test("directory browser controller sends local target with group edits", async () => {
  const previousDocument = globalThis.document;
  globalThis.document = {
    createElement(tagName) {
      return { tagName, value: "", textContent: "" };
    },
  };
  try {
    const calls = [];
    const { runtime, state, el } = createRuntime({
      apiFetch: async (...args) => {
        calls.push(args);
        return {};
      },
      responseJson: async (_response, normalize) => normalize({
        path: "/workspace",
        entries: [],
        launch_targets: [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()],
        default_launch_target: "devbox",
      }),
      location: new URL("http://swimmers.test/"),
      renderDirBrowserView() {
        return true;
      },
    });
    state.readOnly = false;
    state.dirBrowser.path = "/workspace";
    state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
    state.dirBrowser.launchTarget = "local";
    el.createLaunchTarget = selectElement("local");

    const controller = createDirBrowserController(runtime);
    await controller.updateDirEntryGroupMembership("/workspace/swimmers", "add", "backend");

    assert.equal(calls[0][0], "/v1/dirs/group-memberships");
    assert.deepEqual(JSON.parse(calls[0][1].body), {
      path: "/workspace/swimmers",
      target: null,
      add: ["backend"],
      remove: [],
    });
  } finally {
    globalThis.document = previousDocument;
  }
});

test("directory browser controller blocks remote group edits before fetch", async () => {
  const calls = [];
  const { runtime, state, el, statuses } = createRuntime({
    apiFetch: async (...args) => {
      calls.push(args);
      return {};
    },
  });
  state.readOnly = false;
  state.dirBrowser.launchTargets = [{ id: "local", label: "Local machine", kind: "local" }, devboxTarget()];
  state.dirBrowser.launchTarget = "devbox";
  el.createLaunchTarget = selectElement("devbox");

  const controller = createDirBrowserController(runtime);
  await controller.updateDirEntryGroupMembership("/workspace/swimmers", "add", "backend");

  assert.equal(calls.length, 0);
  assert.deepEqual(statuses.at(-1), {
    message: "Remote directory group edits are read-only from this server.",
    isError: true,
  });
});

test("directory browser controller blocks unmapped remote single creates before fetch", async () => {
  const calls = [];
  const { runtime, state, el, statuses } = createRuntime({
    apiFetch: async (...args) => {
      calls.push(args);
      return { json: async () => ({ session: { session_id: "s1" } }) };
    },
  });
  state.readOnly = false;
  state.dirBrowser.launchTargets = [devboxTarget()];
  el.createLaunchTarget = { value: "devbox" };
  el.createCwd.value = "/tmp/outside";

  const controller = createDirBrowserController(runtime);
  const event = submitEvent();
  await controller.handleCreateFormSubmit(event);

  assert.equal(event.prevented, true);
  assert.equal(calls.length, 0);
  assert.deepEqual(statuses.at(-1), {
    message: "Devbox: unmapped cwd for /tmp/outside",
    isError: true,
  });
});

test("directory browser controller keeps local override explicit in single create payload", async () => {
  const calls = [];
  const { runtime, state, el } = createRuntime({
    apiFetch: async (...args) => {
      calls.push(args);
      return { json: async () => ({ session: { session_id: "s1" } }) };
    },
  });
  state.readOnly = false;
  state.dirBrowser.launchTargets = [devboxTarget()];
  el.createLaunchTarget = { value: "local" };
  el.createCwd.value = "/tmp/outside";
  el.createRequest.value = "start here";

  const controller = createDirBrowserController(runtime);
  await controller.handleCreateFormSubmit(submitEvent());

  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "/v1/sessions");
  assert.deepEqual(JSON.parse(calls[0][1].body), {
    cwd: "/tmp/outside",
    spawn_tool: "grok",
    launch_target: null,
    initial_request: "start here",
  });
});

test("directory browser controller recomputes remote batch blockers before fetch", async () => {
  const calls = [];
  const { runtime, state, el, statuses } = createRuntime({
    apiFetch: async (...args) => {
      calls.push(args);
      return { json: async () => ({ results: [] }) };
    },
  });
  state.readOnly = false;
  state.dirBrowser.launchTargets = [devboxTarget()];
  state.dirBrowser.batchSelected = new Set(["/workspace/swimmers", "/tmp/outside"]);
  state.dirBrowser.batchLaunchBlockers = [];
  el.createLaunchTarget = { value: "devbox" };
  el.createCwd.value = "/workspace/swimmers";

  const controller = createDirBrowserController(runtime);
  await controller.handleCreateFormSubmit(submitEvent());

  assert.equal(calls.length, 0);
  assert.equal(state.dirBrowser.batchLaunchBlockers.length, 1);
  assert.deepEqual(statuses.at(-1), {
    message: "Remote batch has unmapped directories: /tmp/outside",
    isError: true,
  });
});
