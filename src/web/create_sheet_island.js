import React from "react";
import { hydrateRoot } from "react-dom/client";

export const CREATE_SHEET_ISLAND_IDS = Object.freeze({
  createSheet: "create-sheet",
  createSheetTitle: "create-sheet-title",
  createCloseButton: "create-close-button",
  dirsSearch: "dirs-search",
  dirsManagedOnly: "dirs-managed-only",
  createBatchVisible: "create-batch-visible",
  dirsGroups: "dirs-groups",
  dirsPath: "dirs-path",
  dirsUpButton: "dirs-up-button",
  dirsLoadButton: "dirs-load-button",
  dirsSpawnHere: "dirs-spawn-here",
  dirsList: "dirs-list",
  createForm: "create-form",
  createCwd: "create-cwd",
  createTool: "create-tool",
  createLaunchTarget: "create-launch-target",
  createRequest: "create-request",
  dirsSummary: "dirs-summary",
  createBatchBar: "create-batch-bar",
  createBatchCount: "create-batch-count",
  createBatchTool: "create-batch-tool",
  createBatchPreview: "create-batch-preview",
  createBatchClear: "create-batch-clear",
  createBatchSubmit: "create-batch-submit",
  createButton: "create-button",
});

export const CREATE_SHEET_ISLAND_KEYS = Object.freeze({
  header: "create-header",
  toolbar: "create-toolbar",
  groups: "create-groups",
  pathbar: "create-pathbar",
  table: "create-table",
  form: "create-form",
});

export const CREATE_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden create-console",
  id: CREATE_SHEET_ISLAND_IDS.createSheet,
  "aria-labelledby": CREATE_SHEET_ISLAND_IDS.createSheetTitle,
});

export const CREATE_SHEET_SEARCH_INPUT_PROPS = Object.freeze({
  id: CREATE_SHEET_ISLAND_IDS.dirsSearch,
  type: "search",
  placeholder: "Search repos, paths, groups…",
  autoComplete: "off",
  "aria-label": "Search repositories",
});

export const CREATE_SHEET_PATH_INPUT_PROPS = Object.freeze({
  id: CREATE_SHEET_ISLAND_IDS.dirsPath,
  type: "text",
  placeholder: "/absolute/path",
  autoComplete: "off",
  "aria-label": "Browse path",
});

export const CREATE_SHEET_CWD_INPUT_PROPS = Object.freeze({
  id: CREATE_SHEET_ISLAND_IDS.createCwd,
  type: "text",
  placeholder: "/absolute/path",
  autoComplete: "off",
});

export const CREATE_SHEET_REQUEST_PROPS = Object.freeze({
  id: CREATE_SHEET_ISLAND_IDS.createRequest,
  rows: 2,
  placeholder: "Optional first message for the new session",
});

export const CREATE_SHEET_DIR_LIST_PROPS = Object.freeze({
  className: "console-body browser-list",
  id: CREATE_SHEET_ISLAND_IDS.dirsList,
  role: "rowgroup",
  "aria-label": "Directory entries",
});

export const CREATE_SHEET_DEFAULT_COPY = Object.freeze({
  dirsSummary: "Browse directories before creating a session.",
  createBatchCount: "0 selected",
  createBatchTool: "tool: grok",
  createBatchPreview: "request: (none)",
});

export const CREATE_SHEET_TOOL_OPTIONS = Object.freeze([
  Object.freeze({ value: "grok", label: "Grok" }),
  Object.freeze({ value: "codex", label: "Codex" }),
  Object.freeze({ value: "claude", label: "Claude" }),
]);

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

function searchIcon(createElement) {
  return createElement(
    "svg",
    {
      className: "console-search-icon",
      viewBox: "0 0 16 16",
      width: "15",
      height: "15",
      fill: "none",
      "aria-hidden": "true",
    },
    createElement("circle", {
      cx: "7",
      cy: "7",
      r: "4.5",
      stroke: "currentColor",
      strokeWidth: "1.5",
    }),
    createElement("path", {
      d: "M11 11l3.2 3.2",
      stroke: "currentColor",
      strokeWidth: "1.5",
      strokeLinecap: "round",
    }),
  );
}

function selectChevron(createElement) {
  return createElement(
    "svg",
    {
      viewBox: "0 0 10 6",
      width: "10",
      height: "6",
      fill: "none",
      "aria-hidden": "true",
    },
    createElement("path", {
      d: "M1 1l4 4 4-4",
      stroke: "currentColor",
      strokeWidth: "1.5",
      strokeLinecap: "round",
      strokeLinejoin: "round",
    }),
  );
}

function ghostButton(createElement, id, label, className = "console-ghost") {
  return createElement(
    "button",
    {
      className,
      id,
      type: "button",
    },
    label,
  );
}

export function createCreateSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Create sheet island requires a createElement function");
  }
  return [
    createElement(
      "header",
      { className: "console-head", key: CREATE_SHEET_ISLAND_KEYS.header },
      createElement(
        "div",
        { className: "console-heading" },
        createElement("p", { className: "console-eyebrow" }, "Repository atlas"),
        createElement("h2", { id: CREATE_SHEET_ISLAND_IDS.createSheetTitle }, "Create session"),
      ),
      createElement(
        "button",
        {
          className: "console-dismiss",
          id: CREATE_SHEET_ISLAND_IDS.createCloseButton,
          type: "button",
          "aria-label": "Close",
        },
        "esc",
      ),
    ),
    createElement(
      "div",
      { className: "console-toolbar", key: CREATE_SHEET_ISLAND_KEYS.toolbar },
      createElement(
        "div",
        { className: "console-search" },
        searchIcon(createElement),
        createElement("input", CREATE_SHEET_SEARCH_INPUT_PROPS),
      ),
      createElement(
        "label",
        { className: "console-toggle" },
        createElement("input", {
          id: CREATE_SHEET_ISLAND_IDS.dirsManagedOnly,
          type: "checkbox",
        }),
        createElement("span", null, "Managed only"),
      ),
      ghostButton(createElement, CREATE_SHEET_ISLAND_IDS.createBatchVisible, "Select all"),
    ),
    createElement("div", {
      className: "console-chips",
      id: CREATE_SHEET_ISLAND_IDS.dirsGroups,
      key: CREATE_SHEET_ISLAND_KEYS.groups,
      role: "group",
      "aria-label": "Repository groups",
    }),
    createElement(
      "div",
      { className: "console-pathbar", key: CREATE_SHEET_ISLAND_KEYS.pathbar },
      createElement("span", { className: "console-pathbar-kicker" }, "Browsing"),
      createElement("input", CREATE_SHEET_PATH_INPUT_PROPS),
      ghostButton(createElement, CREATE_SHEET_ISLAND_IDS.dirsUpButton, "Up"),
      ghostButton(createElement, CREATE_SHEET_ISLAND_IDS.dirsLoadButton, "Load"),
      ghostButton(
        createElement,
        CREATE_SHEET_ISLAND_IDS.dirsSpawnHere,
        "Spawn here",
        "console-ghost console-ghost-accent",
      ),
    ),
    createElement(
      "div",
      {
        className: "console-table",
        key: CREATE_SHEET_ISLAND_KEYS.table,
        role: "table",
        "aria-label": "Repositories",
      },
      createElement(
        "div",
        { className: "console-row console-row-head", role: "row" },
        createElement("span", { className: "col-select", "aria-hidden": "true" }),
        createElement("span", { className: "col-name", role: "columnheader" }, "Repository"),
        createElement("span", { className: "col-path", role: "columnheader" }, "Path"),
        createElement("span", { className: "col-status", role: "columnheader" }, "Status"),
        createElement("span", { className: "col-groups", role: "columnheader" }, "Groups"),
      ),
      createElement("div", CREATE_SHEET_DIR_LIST_PROPS),
    ),
    createElement(
      "form",
      {
        className: "console-dock",
        id: CREATE_SHEET_ISLAND_IDS.createForm,
        key: CREATE_SHEET_ISLAND_KEYS.form,
      },
      createElement(
        "div",
        { className: "console-dock-grid" },
        createElement(
          "label",
          { className: "dock-field dock-field-wide" },
          createElement("span", null, "Working directory"),
          createElement("input", CREATE_SHEET_CWD_INPUT_PROPS),
        ),
        createElement(
          "label",
          { className: "dock-field" },
          createElement("span", null, "Tool"),
          createElement(
            "span",
            { className: "dock-select" },
            createElement(
              "select",
              { id: CREATE_SHEET_ISLAND_IDS.createTool },
              ...CREATE_SHEET_TOOL_OPTIONS.map((option) => createElement(
                "option",
                { key: option.value, value: option.value },
                option.label,
              )),
            ),
            selectChevron(createElement),
          ),
        ),
        createElement(
          "label",
          { className: "dock-field" },
          createElement("span", null, "Launch target"),
          createElement(
            "span",
            { className: "dock-select" },
            createElement("select", { id: CREATE_SHEET_ISLAND_IDS.createLaunchTarget }),
            selectChevron(createElement),
          ),
        ),
      ),
      createElement(
        "label",
        { className: "dock-field dock-field-prompt" },
        createElement(
          "span",
          null,
          "Boot prompt ",
          createElement("em", null, "optional"),
        ),
        createElement("textarea", CREATE_SHEET_REQUEST_PROPS),
      ),
      createElement(
        "div",
        { className: "console-dock-foot" },
        createElement(
          "p",
          {
            className: "console-status",
            id: CREATE_SHEET_ISLAND_IDS.dirsSummary,
          },
          CREATE_SHEET_DEFAULT_COPY.dirsSummary,
        ),
        createElement(
          "div",
          {
            className: "console-batch hidden",
            id: CREATE_SHEET_ISLAND_IDS.createBatchBar,
            "aria-live": "polite",
          },
          createElement(
            "div",
            { className: "console-batch-copy" },
            createElement(
              "span",
              {
                className: "console-batch-count",
                id: CREATE_SHEET_ISLAND_IDS.createBatchCount,
              },
              CREATE_SHEET_DEFAULT_COPY.createBatchCount,
            ),
            createElement(
              "span",
              {
                className: "console-batch-tool",
                id: CREATE_SHEET_ISLAND_IDS.createBatchTool,
              },
              CREATE_SHEET_DEFAULT_COPY.createBatchTool,
            ),
            createElement(
              "span",
              {
                className: "console-batch-preview",
                id: CREATE_SHEET_ISLAND_IDS.createBatchPreview,
              },
              CREATE_SHEET_DEFAULT_COPY.createBatchPreview,
            ),
          ),
          createElement(
            "button",
            {
              className: "console-ghost console-batch-clear",
              id: CREATE_SHEET_ISLAND_IDS.createBatchClear,
              type: "button",
            },
            "Clear",
          ),
          createElement(
            "button",
            {
              className: "console-batch-submit",
              id: CREATE_SHEET_ISLAND_IDS.createBatchSubmit,
              type: "submit",
              form: CREATE_SHEET_ISLAND_IDS.createForm,
            },
            "Batch send",
          ),
        ),
        createElement(
          "button",
          {
            className: "console-create",
            id: CREATE_SHEET_ISLAND_IDS.createButton,
            type: "submit",
          },
          "Create session",
        ),
      ),
    ),
  ];
}

export function createCreateSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Create sheet island requires a createElement function");
  }
  return createElement(
    "section",
    CREATE_SHEET_ISLAND_HOST_PROPS,
    ...createCreateSheetContents(createElement),
  );
}

export function CreateSheet() {
  return createCreateSheetContents(h);
}

export function resolveCreateSheetIslandContainers({
  documentRef = globalThis.document,
  createSheet,
} = {}) {
  const sheet = elementFromRef(createSheet)
    ?? documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createSheet)
    ?? null;
  const containers = {
    createSheet: sheet,
    createSheetTitle: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createSheetTitle) ?? null,
    createCloseButton: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createCloseButton) ?? null,
    dirsSearch: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsSearch) ?? null,
    dirsManagedOnly: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsManagedOnly) ?? null,
    createBatchVisible: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchVisible) ?? null,
    dirsGroups: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsGroups) ?? null,
    dirsPath: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsPath) ?? null,
    dirsUpButton: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsUpButton) ?? null,
    dirsLoadButton: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsLoadButton) ?? null,
    dirsSpawnHere: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsSpawnHere) ?? null,
    dirsList: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsList) ?? null,
    createForm: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createForm) ?? null,
    createCwd: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createCwd) ?? null,
    createTool: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createTool) ?? null,
    createLaunchTarget: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createLaunchTarget) ?? null,
    createRequest: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createRequest) ?? null,
    dirsSummary: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.dirsSummary) ?? null,
    createBatchBar: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchBar) ?? null,
    createBatchCount: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchCount) ?? null,
    createBatchTool: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchTool) ?? null,
    createBatchPreview: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchPreview) ?? null,
    createBatchClear: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchClear) ?? null,
    createBatchSubmit: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createBatchSubmit) ?? null,
    createButton: documentRef?.getElementById?.(CREATE_SHEET_ISLAND_IDS.createButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Create sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableCreateSheetIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Create sheet island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountCreateSheetIsland({
  documentRef = globalThis.document,
  createSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveCreateSheetIslandContainers({ documentRef, createSheet });
  const handle = {
    containers,
    reactRoot: null,
    render() {
      const previousContainers = handle.containers;
      handle.reactRoot?.render?.(h(CreateSheet));
      handle.containers = assertStableCreateSheetIslandContainers(
        previousContainers,
        resolveCreateSheetIslandContainers({ documentRef, createSheet: containers.createSheet }),
      );
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(containers.createSheet, h(CreateSheet));
  return handle;
}
