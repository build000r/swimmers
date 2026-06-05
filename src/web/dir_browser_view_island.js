import React from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";

import {
  dirEntryBatchSelectable,
  dirEntryGroups,
  dirEntryResolvedPath,
  renderDirGroupActionPlan,
} from "./dir_browser.js";
import { safeAnchorHref } from "./terminal_safety.js";

export const DIR_BROWSER_VIEW_ISLAND_IDS = Object.freeze({
  dirsGroups: "dirs-groups",
  dirsList: "dirs-list",
});

export const DIR_BROWSER_VIEW_ISLAND_KEYS = Object.freeze({
  managedFilter: "filter-managed",
  allFilter: "filter-all",
  empty: "dirs-empty",
});

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

function selectedPathSet(selectedPaths) {
  if (selectedPaths instanceof Set) {
    return selectedPaths;
  }
  if (Array.isArray(selectedPaths)) {
    return new Set(selectedPaths.map((path) => String(path || "")));
  }
  return new Set();
}

function classNames(...names) {
  return names.filter(Boolean).join(" ");
}

function activeChipClass(active) {
  return classNames("ghost-button", "dir-group-chip", active ? "is-active" : "");
}

function entryName(entry) {
  return String(entry?.name || "(unnamed)");
}

function groupActionButton(createElement, entry, entryPath, action, readOnly) {
  const props = {
    className: classNames("ghost-button", "dir-entry-group-action", action.isMember ? "is-member" : ""),
    "data-action": action.action,
    "data-group": action.groupName,
    "data-path": entryPath,
    disabled: readOnly,
    key: `group-action:${entryPath}:${action.groupName}:${action.action}`,
    type: "button",
  };
  if (action.removeGroup) {
    props["data-remove-group"] = action.removeGroup;
  }
  return createElement("button", props, action.label);
}

export function createDirBrowserGroupChipElements(createElement, view = {}) {
  if (typeof createElement !== "function") {
    throw new TypeError("Directory browser view island requires a createElement function");
  }
  const groups = Array.isArray(view.groups) ? view.groups : [];
  if (!groups.length) {
    return [];
  }
  const activeGroup = String(view.activeGroup || "").trim();
  const managed = Boolean(view.managed);
  const overlayLabel = String(view.overlayLabel || "managed").trim().toLowerCase() || "managed";
  return [
    createElement(
      "button",
      {
        className: activeChipClass(managed && !activeGroup),
        "data-filter": "managed",
        "data-group": "",
        key: DIR_BROWSER_VIEW_ISLAND_KEYS.managedFilter,
        type: "button",
      },
      overlayLabel,
    ),
    createElement(
      "button",
      {
        className: activeChipClass(!managed && !activeGroup),
        "data-filter": "all",
        "data-group": "",
        key: DIR_BROWSER_VIEW_ISLAND_KEYS.allFilter,
        type: "button",
      },
      "all folders",
    ),
    ...groups.map((groupName) => {
      const normalized = String(groupName || "");
      return createElement(
        "button",
        {
          className: activeChipClass(normalized === activeGroup),
          "data-filter": "group",
          "data-group": normalized,
          key: `filter-group:${normalized}`,
          type: "button",
        },
        normalized,
      );
    }),
  ];
}

function createDirBrowserEntryElement(createElement, entry, view, selected) {
  const path = String(view.path || "");
  const entryPath = dirEntryResolvedPath(path, entry);
  const selectable = dirEntryBatchSelectable(entry, entryPath);
  const readOnly = Boolean(view.readOnly);
  const running = Boolean(entry?.is_running);
  const dirty = Boolean(entry?.repo_dirty);
  const memberships = dirEntryGroups(entry);
  const managed = memberships.length > 0 || Boolean(entry?.group);
  const managedTitle = memberships.length ? `groups: ${memberships.join(", ")}` : "managed repository";
  const openHref = safeAnchorHref(entry?.open_url);
  const groupActions = renderDirGroupActionPlan(entry, entryPath, view.groups, view.activeGroup);
  const checked = selectable && selected.has(entryPath);

  const checkbox = createElement("input", {
    "aria-label": `Include ${entry?.name} in batch send`,
    className: "dir-row-check",
    "data-path": entryPath,
    defaultChecked: checked,
    disabled: readOnly || !selectable,
    key: `select:${entryPath}:${checked ? "checked" : "open"}`,
    type: "checkbox",
  });

  const mainProps = {
    className: "col-name dir-row-main",
    "data-has-children": String(Boolean(entry?.has_children)),
    "data-path": entryPath,
    disabled: entryPath ? undefined : true,
    tabIndex: -1,
    title: entryPath,
    type: "button",
  };
  if (entry?.group) {
    mainProps["data-group"] = String(entry.group);
  }

  const rowProps = {
    className: "console-row dir-row",
    "data-disabled": String(!selectable),
    "data-has-children": String(Boolean(entry?.has_children)),
    "data-path": entryPath,
    key: `dir:${entryPath}:${String(entry?.name || "")}:${String(entry?.group || "")}`,
    role: "row",
  };
  if (entry?.group) {
    rowProps["data-group"] = String(entry.group);
  }

  return createElement(
    "div",
    rowProps,
    createElement(
      "div",
      { className: "col-select dir-select-cell", key: "select" },
      checkbox,
    ),
    createElement(
      "button",
      mainProps,
      createElement(
        "span",
        {
          "aria-hidden": "true",
          className: classNames("dir-row-kind", entry?.has_children ? "is-dir" : "is-repo"),
        },
        entry?.has_children ? "▸" : "◆",
      ),
      createElement("span", { className: "dir-row-name" }, entryName(entry)),
    ),
    createElement(
      "span",
      { className: "col-path dir-row-path", key: "path", title: entryPath },
      entryPath || "(no path)",
    ),
    createElement(
      "div",
      { className: "col-status dir-row-status", key: "status" },
      createElement(
        "span",
        {
          className: classNames("dir-badge", managed ? "is-managed" : "is-unmanaged"),
          key: "managed",
          title: managed ? managedTitle : "not in a managed group",
        },
        managed ? "managed" : "local",
      ),
      running
        ? createElement("span", { className: "dir-badge is-running", key: "running" }, "running")
        : null,
      dirty
        ? createElement("span", { className: "dir-badge is-dirty", key: "dirty" }, "dirty")
        : null,
    ),
    createElement(
      "div",
      { className: "col-groups dir-row-groups", key: "groups" },
      openHref
        ? createElement(
          "a",
          {
            className: "dir-open-url",
            href: openHref,
            key: "open-url",
            rel: "noopener noreferrer",
            target: "_blank",
          },
          "open url",
        )
        : null,
      groupActions.length
        ? createElement(
          "div",
          {
            "aria-label": `Group actions for ${entry?.name || entryPath}`,
            className: "dir-row-group-actions",
            key: "group-actions",
          },
          ...groupActions.map((action) => groupActionButton(createElement, entry, entryPath, action, readOnly)),
        )
        : null,
    ),
  );
}

export function createDirBrowserListContents(createElement, view = {}) {
  if (typeof createElement !== "function") {
    throw new TypeError("Directory browser view island requires a createElement function");
  }
  const entries = Array.isArray(view.entries) ? view.entries : [];
  if (!entries.length) {
    return [
      createElement(
        "div",
        { className: "console-empty", key: DIR_BROWSER_VIEW_ISLAND_KEYS.empty },
        String(view.search || "").trim() ? "No directory matches." : "No child directories found.",
      ),
    ];
  }
  const selected = selectedPathSet(view.selectedPaths);
  return entries.map((entry) => createDirBrowserEntryElement(createElement, entry, view, selected));
}

export function DirBrowserGroups({ view }) {
  return createDirBrowserGroupChipElements(h, view);
}

export function DirBrowserList({ view }) {
  return createDirBrowserListContents(h, view);
}

export function resolveDirBrowserViewIslandContainers({
  documentRef = globalThis.document,
  dirsGroups,
  dirsList,
} = {}) {
  const containers = {
    dirsGroups: documentRef?.getElementById?.(DIR_BROWSER_VIEW_ISLAND_IDS.dirsGroups)
      ?? elementFromRef(dirsGroups)
      ?? null,
    dirsList: documentRef?.getElementById?.(DIR_BROWSER_VIEW_ISLAND_IDS.dirsList)
      ?? elementFromRef(dirsList)
      ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Directory browser view island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableDirBrowserViewIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Directory browser view island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountDirBrowserViewIsland({
  documentRef = globalThis.document,
  dirsGroups,
  dirsList,
  createRootImpl = createRoot,
  flushSyncImpl = flushSync,
} = {}) {
  const containers = resolveDirBrowserViewIslandContainers({ documentRef, dirsGroups, dirsList });
  const handle = {
    containers,
    groupsRoot: createRootImpl(containers.dirsGroups),
    listRoot: createRootImpl(containers.dirsList),
    render(view = {}) {
      const previousContainers = handle.containers;
      const renderRoots = () => {
        handle.groupsRoot?.render?.(h(DirBrowserGroups, { view }));
        handle.listRoot?.render?.(h(DirBrowserList, { view }));
      };
      if (typeof flushSyncImpl === "function") {
        flushSyncImpl(renderRoots);
      } else {
        renderRoots();
      }
      handle.containers = assertStableDirBrowserViewIslandContainers(
        previousContainers,
        resolveDirBrowserViewIslandContainers({ documentRef, dirsGroups, dirsList }),
      );
      return true;
    },
    unmount() {
      handle.groupsRoot?.unmount?.();
      handle.listRoot?.unmount?.();
    },
  };
  return handle;
}
