import React from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";

import { assertStableIdentity, elementFromRef } from "./react_island_identity.js";

export const COMMAND_PALETTE_ISLAND_IDS = Object.freeze({
  paletteSheet: "palette-sheet",
  paletteSheetTitle: "palette-sheet-title",
  paletteSearch: "palette-search",
  paletteResults: "palette-results",
  paletteCloseButton: "palette-close-button",
});

export const COMMAND_PALETTE_ISLAND_KEYS = Object.freeze({
  header: "palette-header",
  field: "palette-field",
  results: "palette-results",
  actions: "palette-actions",
});

export const COMMAND_PALETTE_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden palette-sheet",
  id: COMMAND_PALETTE_ISLAND_IDS.paletteSheet,
  "aria-labelledby": COMMAND_PALETTE_ISLAND_IDS.paletteSheetTitle,
});

export const COMMAND_PALETTE_RESULTS_PROPS = Object.freeze({
  className: "palette-results",
  id: COMMAND_PALETTE_ISLAND_IDS.paletteResults,
  role: "listbox",
  "aria-label": "Command palette results",
});

const h = React.createElement;

function keyedProps(key, props = {}) {
  return { ...props, key };
}

function commandPaletteItemKey(item, index) {
  return String(item?.sessionId || item?.actionId || item?.label || index);
}

function commandPaletteItemMeta(item) {
  return item?.disabled ? "unavailable" : item?.meta || "";
}

export function createCommandPaletteResultsElement(createElement, {
  items = [],
  activeIndex = 0,
} = {}) {
  if (typeof createElement !== "function") {
    throw new TypeError("Command palette island requires a createElement function");
  }
  const children = Array.isArray(items) && items.length
    ? items.map((item, index) => createElement(
      "button",
      {
        className: `palette-item${index === activeIndex ? " is-active" : ""}`,
        key: commandPaletteItemKey(item, index),
        id: `palette-option-${index}`,
        type: "button",
        role: "option",
        "aria-selected": index === activeIndex ? "true" : "false",
        "data-palette-index": String(index),
        disabled: item?.disabled ? true : undefined,
      },
      createElement("span", { className: "palette-item-title" }, item?.label || ""),
      createElement("span", { className: "palette-item-meta" }, commandPaletteItemMeta(item)),
    ))
    : [createElement("div", { className: "sheet-copy", key: "empty" }, "No matching commands.")];
  return createElement(
    "div",
    keyedProps(COMMAND_PALETTE_ISLAND_KEYS.results, COMMAND_PALETTE_RESULTS_PROPS),
    ...children,
  );
}

export function createCommandPaletteSheetContents(createElement, options = {}) {
  if (typeof createElement !== "function") {
    throw new TypeError("Command palette island requires a createElement function");
  }
  // Combobox pattern: focus stays in the search input while arrow keys move the
  // active option, so screen readers need aria-activedescendant to announce it.
  const paletteItems = Array.isArray(options.items) ? options.items : [];
  const paletteActiveIndex = Number.isInteger(options.activeIndex) ? options.activeIndex : 0;
  const activeDescendant =
    paletteItems.length && paletteActiveIndex >= 0 && paletteActiveIndex < paletteItems.length
      ? `palette-option-${paletteActiveIndex}`
      : undefined;
  return [
    createElement(
      "div",
      { className: "sheet-header", key: COMMAND_PALETTE_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Terminal Actions"),
      createElement("h2", { id: COMMAND_PALETTE_ISLAND_IDS.paletteSheetTitle }, "Command Palette"),
    ),
    createElement(
      "label",
      { className: "field", key: COMMAND_PALETTE_ISLAND_KEYS.field },
      createElement("span", null, "Command or session"),
      createElement("input", {
        id: COMMAND_PALETTE_ISLAND_IDS.paletteSearch,
        type: "search",
        placeholder: "Search actions and sessions",
        autoComplete: "off",
        role: "combobox",
        "aria-expanded": "true",
        "aria-controls": COMMAND_PALETTE_ISLAND_IDS.paletteResults,
        "aria-autocomplete": "list",
        "aria-activedescendant": activeDescendant,
      }),
    ),
    createCommandPaletteResultsElement(createElement, options),
    createElement(
      "div",
      { className: "sheet-actions", key: COMMAND_PALETTE_ISLAND_KEYS.actions },
      createElement(
        "button",
        {
          className: "ghost-button",
          id: COMMAND_PALETTE_ISLAND_IDS.paletteCloseButton,
          type: "button",
        },
        "Close",
      ),
    ),
  ];
}

export function createCommandPaletteSheetElement(createElement, options = {}) {
  if (typeof createElement !== "function") {
    throw new TypeError("Command palette island requires a createElement function");
  }
  return createElement(
    "section",
    COMMAND_PALETTE_ISLAND_HOST_PROPS,
    ...createCommandPaletteSheetContents(createElement, options),
  );
}

export function CommandPaletteResults(props) {
  return createCommandPaletteResultsElement(h, props);
}

export function CommandPaletteSheet(props) {
  return createCommandPaletteSheetContents(h, props);
}

export function resolveCommandPaletteIslandHost({
  documentRef = globalThis.document,
  paletteSheet,
} = {}) {
  const sheet = elementFromRef(paletteSheet)
    ?? documentRef?.getElementById?.(COMMAND_PALETTE_ISLAND_IDS.paletteSheet)
    ?? null;
  if (!sheet) {
    throw new Error("Command palette island missing stable container paletteSheet");
  }
  return sheet;
}

export function resolveCommandPaletteIslandContainers({
  documentRef = globalThis.document,
  paletteSheet,
} = {}) {
  const sheet = resolveCommandPaletteIslandHost({ documentRef, paletteSheet });
  const containers = {
    paletteSheet: sheet,
    paletteSearch: documentRef?.getElementById?.(COMMAND_PALETTE_ISLAND_IDS.paletteSearch) ?? null,
    paletteResults: documentRef?.getElementById?.(COMMAND_PALETTE_ISLAND_IDS.paletteResults) ?? null,
    paletteCloseButton: documentRef?.getElementById?.(COMMAND_PALETTE_ISLAND_IDS.paletteCloseButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Command palette island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableCommandPaletteIslandContainers(previous, next) {
  return assertStableIdentity(previous, next, { label: "Command palette island" });
}

export function mountCommandPaletteIsland({
  documentRef = globalThis.document,
  paletteSheet,
  createRootImpl = createRoot,
  flushSyncImpl = flushSync,
  items = [],
  activeIndex = 0,
} = {}) {
  // The palette is hidden until opened, so the SSR markup is only a no-JS
  // fallback. Mounting with createRoot (mirroring the dir-browser island)
  // replaces that markup outright instead of hydrating over it, which avoids
  // the recoverable hydration mismatches the empty SSR #palette-results and the
  // attribute-less SSR #palette-search would otherwise trigger.
  const host = resolveCommandPaletteIslandHost({ documentRef, paletteSheet });
  const handle = {
    containers: null,
    items,
    activeIndex,
    reactRoot: createRootImpl(host),
    render(next = {}) {
      const previousContainers = handle.containers;
      handle.items = Array.isArray(next.items) ? next.items : handle.items;
      handle.activeIndex = Number.isFinite(next.activeIndex)
        ? Math.trunc(next.activeIndex)
        : handle.activeIndex;
      const renderTree = () => {
        handle.reactRoot?.render?.(h(CommandPaletteSheet, {
          items: handle.items,
          activeIndex: handle.activeIndex,
        }));
      };
      // Render synchronously so the child containers (search/results/close) exist
      // before we resolve and identity-check them.
      if (typeof flushSyncImpl === "function") {
        flushSyncImpl(renderTree);
      } else {
        renderTree();
      }
      const nextContainers = resolveCommandPaletteIslandContainers({
        documentRef,
        paletteSheet: host,
      });
      handle.containers = previousContainers
        ? assertStableCommandPaletteIslandContainers(previousContainers, nextContainers)
        : nextContainers;
      return handle;
    },
    renderResults(next = {}) {
      handle.render(next);
      return true;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.render({ items: handle.items, activeIndex: handle.activeIndex });
  return handle;
}
