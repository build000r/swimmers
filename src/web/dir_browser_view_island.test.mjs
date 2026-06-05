import test from "node:test";
import assert from "node:assert/strict";

import {
  createElement,
  fakeDocumentForIds,
} from "./island_test_helpers.mjs";
import {
  DIR_BROWSER_VIEW_ISLAND_IDS,
  DirBrowserGroups,
  DirBrowserList,
  assertStableDirBrowserViewIslandContainers,
  createDirBrowserGroupChipElements,
  createDirBrowserListContents,
  mountDirBrowserViewIsland,
  resolveDirBrowserViewIslandContainers,
} from "./dir_browser_view_island.js";

function fakeDocument() {
  return fakeDocumentForIds(DIR_BROWSER_VIEW_ISLAND_IDS);
}

function nonNull(children) {
  return children.filter(Boolean);
}

function childText(node) {
  return node?.props?.children?.[0] ?? "";
}

function withWindowLocation(href = "http://swimmers.test/") {
  const previous = globalThis.window;
  globalThis.window = { ...(previous || {}), location: new URL(href) };
  return () => {
    if (previous === undefined) {
      delete globalThis.window;
    } else {
      globalThis.window = previous;
    }
  };
}

test("directory browser view island preserves group chip DOM contract", () => {
  const chips = createDirBrowserGroupChipElements(createElement, {
    groups: ["clients", "core"],
    managed: true,
    activeGroup: "",
    overlayLabel: "Managed Repos",
  });

  assert.deepEqual(chips.map((chip) => chip.props.key), [
    "filter-managed",
    "filter-all",
    "filter-group:clients",
    "filter-group:core",
  ]);
  assert.deepEqual(chips.map((chip) => chip.props.type), ["button", "button", "button", "button"]);
  assert.equal(chips[0].props.className, "ghost-button dir-group-chip is-active");
  assert.equal(chips[0].props["data-filter"], "managed");
  assert.equal(chips[0].props["data-group"], "");
  assert.equal(childText(chips[0]), "managed repos");
  assert.equal(chips[1].props.className, "ghost-button dir-group-chip");
  assert.equal(chips[1].props["data-filter"], "all");
  assert.equal(childText(chips[1]), "all folders");
  assert.equal(chips[2].props["data-filter"], "group");
  assert.equal(chips[2].props["data-group"], "clients");
  assert.equal(childText(chips[2]), "clients");

  const activeGroupChips = createDirBrowserGroupChipElements(createElement, {
    groups: ["clients", "core"],
    managed: false,
    activeGroup: "core",
  });
  assert.equal(activeGroupChips[1].props.className, "ghost-button dir-group-chip");
  assert.equal(activeGroupChips[3].props.className, "ghost-button dir-group-chip is-active");
  assert.deepEqual(createDirBrowserGroupChipElements(createElement, { groups: [] }), []);
  assert.throws(() => createDirBrowserGroupChipElements(null, {}), /createElement function/);
});

test("directory browser view island preserves row, action, badge, and link contract", () => {
  const restoreWindow = withWindowLocation();
  const rows = createDirBrowserListContents(createElement, {
    path: "/srv/repos",
    groups: ["core", "clients"],
    activeGroup: "core",
    selectedPaths: new Set(["/srv/repos/swimmers", "/srv/repos/clients"]),
    readOnly: false,
    entries: [
      {
        name: "swimmers",
        full_path: "/srv/repos/swimmers",
        groups: ["core"],
        has_children: true,
        is_running: true,
        repo_dirty: true,
        open_url: "http://127.0.0.1:3210/repo",
      },
      {
        name: "clients",
        group: "clients",
        has_children: false,
        open_url: "javascript:alert(1)",
      },
    ],
  });
  try {
    const first = rows[0];
    const firstChildren = first.props.children;
    const checkbox = firstChildren[0].props.children[0];
    const main = firstChildren[1];
    const status = nonNull(firstChildren[3].props.children);
    const groupsCell = firstChildren[4];
    const groupsChildren = nonNull(groupsCell.props.children);
    const groupButtons = groupsChildren[1].props.children;
    const virtual = rows[1];
    const virtualCheckbox = virtual.props.children[0].props.children[0];
    const virtualGroupsChildren = nonNull(virtual.props.children[4].props.children);

    assert.equal(first.type, "div");
    assert.equal(first.props.className, "console-row dir-row");
    assert.equal(first.props.role, "row");
    assert.equal(first.props["data-path"], "/srv/repos/swimmers");
    assert.equal(first.props["data-has-children"], "true");
    assert.equal(first.props["data-disabled"], "false");
    assert.equal(checkbox.props.className, "dir-row-check");
    assert.equal(checkbox.props.type, "checkbox");
    assert.equal(checkbox.props["data-path"], "/srv/repos/swimmers");
    assert.equal(checkbox.props.defaultChecked, true);
    assert.equal(checkbox.props.disabled, false);
    assert.equal(checkbox.props["aria-label"], "Include swimmers in batch send");
    assert.equal(main.props.className, "col-name dir-row-main");
    assert.equal(main.props["data-path"], "/srv/repos/swimmers");
    assert.equal(main.props["data-has-children"], "true");
    assert.equal(main.props.tabIndex, -1);
    assert.equal(main.props.title, "/srv/repos/swimmers");
    assert.equal(main.props.children[0].props.className, "dir-row-kind is-dir");
    assert.equal(childText(main.props.children[0]), "▸");
    assert.equal(childText(main.props.children[1]), "swimmers");
    assert.equal(firstChildren[2].props.className, "col-path dir-row-path");
    assert.equal(childText(firstChildren[2]), "/srv/repos/swimmers");
    assert.deepEqual(status.map((badge) => badge.props.className), [
      "dir-badge is-managed",
      "dir-badge is-running",
      "dir-badge is-dirty",
    ]);
    assert.equal(status[0].props.title, "groups: core");
    assert.equal(groupsCell.props.className, "col-groups dir-row-groups");
    assert.equal(groupsChildren[0].props.className, "dir-open-url");
    assert.equal(groupsChildren[0].props.href, "http://127.0.0.1:3210/repo");
    assert.equal(groupsChildren[0].props.target, "_blank");
    assert.equal(groupsChildren[0].props.rel, "noopener noreferrer");
    assert.equal(groupsChildren[1].props.className, "dir-row-group-actions");
    assert.equal(groupsChildren[1].props["aria-label"], "Group actions for swimmers");
    assert.deepEqual(groupButtons.map((button) => button.props["data-action"]), ["remove", "move"]);
    assert.deepEqual(groupButtons.map((button) => button.props["data-group"]), ["core", "clients"]);
    assert.equal(groupButtons[1].props["data-remove-group"], "core");
    assert.deepEqual(groupButtons.map(childText), ["remove core", "move to clients"]);

    assert.equal(virtual.props["data-path"], "/srv/repos/clients");
    assert.equal(virtual.props["data-disabled"], "true");
    assert.equal(virtual.props["data-group"], "clients");
    assert.equal(virtualCheckbox.props.defaultChecked, false);
    assert.equal(virtualCheckbox.props.disabled, true);
    assert.equal(virtual.props.children[1].props["data-group"], "clients");
    assert.equal(virtualGroupsChildren.some((child) => child?.props?.className === "dir-open-url"), false);
  } finally {
    restoreWindow();
  }
});

test("directory browser view island preserves read-only disabled controls and empty states", () => {
  const rows = createDirBrowserListContents(createElement, {
    path: "/srv/repos",
    groups: ["core"],
    selectedPaths: ["/srv/repos/swimmers"],
    readOnly: true,
    entries: [{ name: "swimmers", full_path: "/srv/repos/swimmers", groups: [] }],
  });
  const checkbox = rows[0].props.children[0].props.children[0];
  const groupActions = rows[0].props.children[4].props.children[1];

  assert.equal(checkbox.props.disabled, true);
  assert.equal(groupActions.props.children[0].props.disabled, true);
  assert.equal(childText(createDirBrowserListContents(createElement, { entries: [], search: "" })[0]), "No child directories found.");
  assert.equal(childText(createDirBrowserListContents(createElement, { entries: [], search: "repo" })[0]), "No directory matches.");
  assert.throws(() => createDirBrowserListContents(null, {}), /createElement function/);
});

test("directory browser view island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountDirBrowserViewIsland({
    documentRef,
    createRootImpl(root) {
      calls.push(["createRoot", root.id]);
      return {
        render(element) {
          calls.push(["render", root.id, element.type]);
        },
        unmount() {
          calls.push(["unmount", root.id]);
        },
      };
    },
    flushSyncImpl(callback) {
      calls.push(["flush:start"]);
      callback();
      calls.push(["flush:end"]);
    },
  });
  const before = { ...handle.containers };

  assert.deepEqual(calls.slice(0, 2), [
    ["createRoot", DIR_BROWSER_VIEW_ISLAND_IDS.dirsGroups],
    ["createRoot", DIR_BROWSER_VIEW_ISLAND_IDS.dirsList],
  ]);
  assert.deepEqual(resolveDirBrowserViewIslandContainers({ documentRef }), before);
  assert.equal(handle.render({ entries: [], groups: [] }), true);
  assert.deepEqual(calls.slice(2), [
    ["flush:start"],
    ["render", DIR_BROWSER_VIEW_ISLAND_IDS.dirsGroups, DirBrowserGroups],
    ["render", DIR_BROWSER_VIEW_ISLAND_IDS.dirsList, DirBrowserList],
    ["flush:end"],
  ]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableDirBrowserViewIslandContainers(before, {
      ...before,
      dirsGroups: replace(DIR_BROWSER_VIEW_ISLAND_IDS.dirsGroups),
    }),
    /replaced stable container dirsGroups/,
  );
  handle.unmount();
  assert.deepEqual(calls.slice(-2), [
    ["unmount", DIR_BROWSER_VIEW_ISLAND_IDS.dirsGroups],
    ["unmount", DIR_BROWSER_VIEW_ISLAND_IDS.dirsList],
  ]);
});

test("directory browser view island detects missing and synchronously replaced hosts", () => {
  const missing = fakeDocument();
  missing.remove(DIR_BROWSER_VIEW_ISLAND_IDS.dirsList);
  assert.throws(
    () => resolveDirBrowserViewIslandContainers({ documentRef: missing.documentRef }),
    /missing stable container dirsList/,
  );

  const { documentRef, replace } = fakeDocument();
  const handle = mountDirBrowserViewIsland({
    documentRef,
    createRootImpl(root) {
      return {
        render(element) {
          if (root.id === DIR_BROWSER_VIEW_ISLAND_IDS.dirsList && element.type === DirBrowserList) {
            replace(DIR_BROWSER_VIEW_ISLAND_IDS.dirsList);
          }
        },
        unmount() {},
      };
    },
    flushSyncImpl(callback) {
      callback();
    },
  });

  assert.throws(
    () => handle.render({ entries: [], groups: [] }),
    /replaced stable container dirsList/,
  );
});
