import test from "node:test";
import assert from "node:assert/strict";

import {
  createRequestPreviewText,
  dirEntryBatchSelectable,
  dirEntryGroups,
  dirEntryMatchesSearch,
  dirEntryResolvedPath,
  joinPath,
  launchTargetPayload,
  renderCreateBatchBar,
  selectedLaunchTarget,
  visibleDirBatchPlan,
  visibleDirEntries,
  visibleSelectableDirPaths,
} from "./dir_browser.js";

function element(value = "") {
  const classes = new Set();
  return {
    value,
    textContent: "",
    classList: {
      contains: (name) => classes.has(name),
      toggle: (name, force) => {
        const enabled = Boolean(force);
        if (enabled) {
          classes.add(name);
        } else {
          classes.delete(name);
        }
        return enabled;
      },
    },
  };
}

test("directory path helpers preserve legacy root joining and explicit paths", () => {
  assert.equal(joinPath("", ""), "/");
  assert.equal(joinPath("", "repo"), "/repo");
  assert.equal(joinPath("/", "repo"), "/repo");
  assert.equal(joinPath("/srv/repos/", "/swimmers"), "/srv/repos/swimmers");
  assert.equal(dirEntryResolvedPath("/srv/repos", { name: "swimmers" }), "/srv/repos/swimmers");
  assert.equal(
    dirEntryResolvedPath("/srv/repos", { name: "swimmers", full_path: "/override/swimmers" }),
    "/override/swimmers",
  );
});

test("directory group and search helpers keep membership and repo status searchable", () => {
  const entry = {
    name: "swimmers",
    full_path: "/srv/repos/swimmers",
    groups: ["core", "", "rust"],
    group: "active",
    has_children: false,
    is_running: true,
    repo_dirty: true,
  };

  assert.deepEqual(dirEntryGroups(entry), ["core", "rust", "active"]);
  assert.equal(dirEntryMatchesSearch(entry, entry.full_path, "dirty"), true);
  assert.equal(dirEntryMatchesSearch(entry, entry.full_path, "running rust"), false);
  assert.deepEqual(visibleDirEntries([entry], "/srv/repos", "core"), [entry]);
});

test("directory batch selectability rejects virtual group rows without a full path", () => {
  assert.equal(dirEntryBatchSelectable({ name: "repo" }, "/srv/repos/repo"), true);
  assert.equal(dirEntryBatchSelectable({ name: "group", group: "clients" }, "/srv/repos/group"), false);
  assert.equal(
    dirEntryBatchSelectable({ name: "group", group: "clients", full_path: "/srv/repos/group" }, "/srv/repos/group"),
    true,
  );
});

test("visible selectable paths respects search and virtual group rows", () => {
  const dirBrowser = {
    path: "/srv/repos",
    search: "dirty",
    entries: [
      { name: "swimmers", repo_dirty: true },
      { name: "clients", group: "clients", repo_dirty: true },
      { name: "clean" },
    ],
  };

  assert.deepEqual(visibleSelectableDirPaths(dirBrowser), ["/srv/repos/swimmers"]);
});

test("visibleDirBatchPlan preserves paths, fallbacks, and status copy", () => {
  assert.deepEqual(visibleDirBatchPlan(["/srv/repos/a", "/srv/repos/b"], "/current", "/typed"), {
    paths: ["/srv/repos/a", "/srv/repos/b"],
    firstPath: "/srv/repos/a",
    statusLabel: "Batching 2 visible directories.",
    statusMuted: false,
  });
  assert.deepEqual(visibleDirBatchPlan([], "", "/typed"), {
    paths: [],
    firstPath: "/typed",
    statusLabel: "No visible directories to batch.",
    statusMuted: true,
  });
  assert.deepEqual(visibleDirBatchPlan([], "", ""), {
    paths: [],
    firstPath: "",
    statusLabel: "No visible directories to batch.",
    statusMuted: true,
  });
});

test("launch target and batch bar helpers preserve payload and label semantics", () => {
  const el = {
    createLaunchTarget: element("remote-a"),
    createTool: element("Codex"),
    createRequest: element("  run the smoke tests   with extra spacing  "),
    createBatchBar: element(),
    createBatchCount: element(),
    createBatchTool: element(),
    createBatchPreview: element(),
  };
  const dirBrowser = {
    launchTarget: "local",
    batchSelected: new Set(["/srv/repos/swimmers", "/srv/repos/other"]),
  };

  assert.equal(selectedLaunchTarget(el, dirBrowser), "remote-a");
  assert.equal(launchTargetPayload(el, dirBrowser), "remote-a");
  renderCreateBatchBar({ el, dirBrowser });

  assert.equal(el.createBatchBar.classList.contains("hidden"), false);
  assert.equal(el.createBatchCount.textContent, "2 selected");
  assert.equal(el.createBatchTool.textContent, "tool: codex -> remote-a");
  assert.equal(el.createBatchPreview.textContent, "request: run the smoke tests with extra spacing");
  assert.equal(createRequestPreviewText({ createRequest: element("") }), "(none)");
});
