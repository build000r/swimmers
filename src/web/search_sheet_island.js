import React from "react";
import { hydrateRoot } from "react-dom/client";

export const SEARCH_SHEET_ISLAND_IDS = Object.freeze({
  searchSheet: "search-sheet",
  searchSheetTitle: "search-sheet-title",
  searchForm: "search-form",
  terminalSearch: "terminal-search",
  searchPrevButton: "search-prev-button",
  searchNextButton: "search-next-button",
  searchClearButton: "search-clear-button",
  searchCloseButton: "search-close-button",
});

export const SEARCH_SHEET_ISLAND_KEYS = Object.freeze({
  header: "search-header",
  form: "search-form",
  field: "search-field",
  actions: "search-actions",
});

export const SEARCH_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden",
  id: SEARCH_SHEET_ISLAND_IDS.searchSheet,
  "aria-labelledby": SEARCH_SHEET_ISLAND_IDS.searchSheetTitle,
});

export const SEARCH_SHEET_INPUT_PROPS = Object.freeze({
  id: SEARCH_SHEET_ISLAND_IDS.terminalSearch,
  type: "search",
  placeholder: "Find text in the current terminal view",
  autoComplete: "off",
});

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

function keyedProps(key, props = {}) {
  return { ...props, key };
}

export function createSearchSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Search sheet island requires a createElement function");
  }
  return [
    createElement(
      "div",
      { className: "sheet-header", key: SEARCH_SHEET_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Rendered Action"),
      createElement("h2", { id: SEARCH_SHEET_ISLAND_IDS.searchSheetTitle }, "Search Terminal"),
    ),
    createElement(
      "form",
      {
        className: "sheet-form",
        id: SEARCH_SHEET_ISLAND_IDS.searchForm,
        key: SEARCH_SHEET_ISLAND_KEYS.form,
      },
      createElement(
        "label",
        { className: "field", key: SEARCH_SHEET_ISLAND_KEYS.field },
        createElement("span", null, "Query"),
        createElement("input", SEARCH_SHEET_INPUT_PROPS),
      ),
      createElement(
        "div",
        { className: "sheet-actions", key: SEARCH_SHEET_ISLAND_KEYS.actions },
        createElement(
          "button",
          {
            className: "ghost-button",
            id: SEARCH_SHEET_ISLAND_IDS.searchPrevButton,
            type: "button",
          },
          "Prev",
        ),
        createElement(
          "button",
          {
            className: "ghost-button",
            id: SEARCH_SHEET_ISLAND_IDS.searchNextButton,
            type: "button",
          },
          "Next",
        ),
        createElement(
          "button",
          {
            className: "ghost-button",
            id: SEARCH_SHEET_ISLAND_IDS.searchClearButton,
            type: "button",
          },
          "Clear",
        ),
        createElement(
          "button",
          {
            id: SEARCH_SHEET_ISLAND_IDS.searchCloseButton,
            type: "submit",
          },
          "Done",
        ),
      ),
    ),
  ];
}

export function createSearchSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Search sheet island requires a createElement function");
  }
  return createElement(
    "section",
    SEARCH_SHEET_ISLAND_HOST_PROPS,
    ...createSearchSheetContents(createElement),
  );
}

export function SearchSheet() {
  return createSearchSheetContents(h);
}

export function resolveSearchSheetIslandContainers({
  documentRef = globalThis.document,
  searchSheet,
} = {}) {
  const sheet = elementFromRef(searchSheet)
    ?? documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.searchSheet)
    ?? null;
  const containers = {
    searchSheet: sheet,
    searchForm: documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.searchForm) ?? null,
    terminalSearch: documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.terminalSearch) ?? null,
    searchPrevButton: documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.searchPrevButton) ?? null,
    searchNextButton: documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.searchNextButton) ?? null,
    searchClearButton: documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.searchClearButton) ?? null,
    searchCloseButton: documentRef?.getElementById?.(SEARCH_SHEET_ISLAND_IDS.searchCloseButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Search sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableSearchSheetIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Search sheet island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountSearchSheetIsland({
  documentRef = globalThis.document,
  searchSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveSearchSheetIslandContainers({ documentRef, searchSheet });
  const handle = {
    containers,
    reactRoot: null,
    render() {
      const previousContainers = handle.containers;
      handle.reactRoot?.render?.(h(SearchSheet));
      handle.containers = assertStableSearchSheetIslandContainers(
        previousContainers,
        resolveSearchSheetIslandContainers({ documentRef, searchSheet: containers.searchSheet }),
      );
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(containers.searchSheet, h(SearchSheet));
  return handle;
}
