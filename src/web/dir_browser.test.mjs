import test from "node:test";
import assert from "node:assert/strict";

import {
  createRequestPreviewText,
  dirCheckboxChangePlan,
  dirEntryBatchSelectable,
  dirEntryGroups,
  dirEntryMatchesSearch,
  dirEntryResolvedPath,
  dirGroupChipClickPlan,
  dirGroupMembershipClickPlan,
  dirRowClickPlan,
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

test("dirCheckboxChangePlan preserves ignored, reset, add, and remove decisions", () => {
  const checkboxFor = (path, checked = true) => {
    const checkbox = { checked, dataset: { path } };
    return {
      checkbox,
      target: {
        closest(selector) {
          return selector === ".dir-row-check" ? checkbox : null;
        },
      },
    };
  };
  const blank = checkboxFor(" ");
  assert.deepEqual(dirCheckboxChangePlan("click", checkboxFor("/srv/repos/a").target), { type: "ignore" });
  assert.deepEqual(dirCheckboxChangePlan("change", { closest: () => null }), { type: "ignore" });
  const resetPlan = dirCheckboxChangePlan("change", blank.target);
  assert.equal(resetPlan.type, "reset_checkbox");
  assert.equal(resetPlan.checkbox, blank.checkbox);
  assert.deepEqual(dirCheckboxChangePlan("change", checkboxFor("/srv/repos/a", true).target), {
    type: "add",
    path: "/srv/repos/a",
  });
  assert.deepEqual(dirCheckboxChangePlan("change", checkboxFor("/srv/repos/a", false).target), {
    type: "remove",
    path: "/srv/repos/a",
  });
});

test("dirGroupChipClickPlan preserves managed, all, group, blank, and ignored decisions", () => {
  const chipFor = (dataset) => {
    const chip = { dataset };
    return {
      chip,
      target: {
        closest(selector) {
          return selector === ".dir-group-chip" ? chip : null;
        },
      },
    };
  };

  assert.deepEqual(dirGroupChipClickPlan("keydown", chipFor({ filter: "managed" }).target, false, "/current", "/typed"), {
    type: "ignore",
  });
  assert.deepEqual(dirGroupChipClickPlan("click", { closest: () => null }, false, "/current", "/typed"), {
    type: "ignore",
  });
  assert.deepEqual(dirGroupChipClickPlan("click", chipFor({ filter: "managed", group: "clients" }).target, false, "/current", "/typed"), {
    type: "filter",
    group: "",
    managedOnly: true,
    path: "/current",
  });
  assert.deepEqual(dirGroupChipClickPlan("click", chipFor({ filter: "all", group: "clients" }).target, true, "", "/typed"), {
    type: "filter",
    group: "",
    managedOnly: false,
    path: "/typed",
  });
  assert.deepEqual(dirGroupChipClickPlan("click", chipFor({ filter: "group", group: " clients " }).target, true, "/current", "/typed"), {
    type: "filter",
    group: "clients",
    managedOnly: true,
    path: "/current",
  });
  assert.deepEqual(dirGroupChipClickPlan("click", chipFor({ filter: "group", group: " " }).target, false, "", "/typed"), {
    type: "filter",
    group: "",
    managedOnly: false,
    path: "/typed",
  });
});

test("dirGroupMembershipClickPlan preserves action dataset forwarding and ignores", () => {
  const actionFor = (dataset) => {
    const action = { dataset };
    return {
      action,
      target: {
        closest(selector) {
          return selector === ".dir-entry-group-action" ? action : null;
        },
      },
    };
  };

  assert.deepEqual(dirGroupMembershipClickPlan("keydown", actionFor({ action: "add" }).target), {
    type: "ignore",
  });
  assert.deepEqual(dirGroupMembershipClickPlan("click", { closest: () => null }), {
    type: "ignore",
  });
  assert.deepEqual(dirGroupMembershipClickPlan("click", actionFor({ path: "/srv/repos/a", action: "add", group: "clients" }).target), {
    type: "membership",
    path: "/srv/repos/a",
    action: "add",
    group: "clients",
    removeGroup: undefined,
  });
  assert.deepEqual(dirGroupMembershipClickPlan("click", actionFor({ path: "/srv/repos/a", action: "remove", group: "clients" }).target), {
    type: "membership",
    path: "/srv/repos/a",
    action: "remove",
    group: "clients",
    removeGroup: undefined,
  });
  assert.deepEqual(dirGroupMembershipClickPlan("click", actionFor({ path: "/srv/repos/a", action: "move", group: "new", removeGroup: "old" }).target), {
    type: "membership",
    path: "/srv/repos/a",
    action: "move",
    group: "new",
    removeGroup: "old",
  });
  assert.deepEqual(dirGroupMembershipClickPlan("click", actionFor({}).target), {
    type: "membership",
    path: undefined,
    action: undefined,
    group: undefined,
    removeGroup: undefined,
  });
});

test("dirRowClickPlan preserves row path trimming, child detection, and ignores", () => {
  const rowFor = (dataset) => {
    const row = { dataset };
    return {
      row,
      target: {
        closest(selector) {
          return selector === ".dir-row-main" ? row : null;
        },
      },
    };
  };

  assert.deepEqual(dirRowClickPlan("keydown", rowFor({ path: "/srv/repos/a", hasChildren: "true" }).target), {
    type: "ignore",
  });
  assert.deepEqual(dirRowClickPlan("click", { closest: () => null }), {
    type: "ignore",
  });
  assert.deepEqual(dirRowClickPlan("click", rowFor({ path: " " }).target), {
    type: "ignore",
  });
  assert.deepEqual(dirRowClickPlan("click", rowFor({ path: " /srv/repos/a ", hasChildren: "true" }).target), {
    type: "row",
    path: "/srv/repos/a",
    hasChildren: true,
  });
  assert.deepEqual(dirRowClickPlan("click", rowFor({ path: "/srv/repos/a", hasChildren: "false" }).target), {
    type: "row",
    path: "/srv/repos/a",
    hasChildren: false,
  });
  assert.deepEqual(dirRowClickPlan("click", rowFor({ path: "/srv/repos/a", hasChildren: true }).target), {
    type: "row",
    path: "/srv/repos/a",
    hasChildren: false,
  });
  assert.deepEqual(dirRowClickPlan("click", rowFor({ path: "/srv/repos/a" }).target), {
    type: "row",
    path: "/srv/repos/a",
    hasChildren: false,
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
