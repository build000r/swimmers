import React from "react";
import { hydrateRoot } from "react-dom/client";

import { assertStableIdentity, elementFromRef } from "./react_island_identity.js";
import { mountHydratedStaticIsland } from "./static_sheet_island.js";

export const MERMAID_SHEET_ISLAND_IDS = Object.freeze({
  mermaidSheet: "mermaid-sheet",
  mermaidSheetTitle: "mermaid-sheet-title",
  mermaidSummary: "mermaid-summary",
  mermaidPreview: "mermaid-preview",
  mermaidSource: "mermaid-source",
  mermaidPlanTabs: "mermaid-plan-tabs",
  mermaidPlanContent: "mermaid-plan-content",
  mermaidRefreshButton: "mermaid-refresh-button",
  mermaidOpenButton: "mermaid-open-button",
  mermaidCloseButton: "mermaid-close-button",
});

export const MERMAID_SHEET_ISLAND_KEYS = Object.freeze({
  header: "mermaid-header",
  summary: "mermaid-summary",
  preview: "mermaid-preview",
  source: "mermaid-source",
  planTabs: "mermaid-plan-tabs",
  planContent: "mermaid-plan-content",
  actions: "mermaid-actions",
});

export const MERMAID_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden",
  id: MERMAID_SHEET_ISLAND_IDS.mermaidSheet,
  "aria-labelledby": MERMAID_SHEET_ISLAND_IDS.mermaidSheetTitle,
});

export const MERMAID_SHEET_DEFAULT_COPY = Object.freeze({
  summary: "Loading Mermaid artifact…",
});

const h = React.createElement;

export function createMermaidSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Mermaid sheet island requires a createElement function");
  }
  return [
    createElement(
      "div",
      { className: "sheet-header", key: MERMAID_SHEET_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Artifact"),
      createElement("h2", { id: MERMAID_SHEET_ISLAND_IDS.mermaidSheetTitle }, "Mermaid Diagram"),
    ),
    createElement(
      "div",
      {
        className: "sheet-copy",
        id: MERMAID_SHEET_ISLAND_IDS.mermaidSummary,
        key: MERMAID_SHEET_ISLAND_KEYS.summary,
      },
      MERMAID_SHEET_DEFAULT_COPY.summary,
    ),
    createElement("div", {
      className: "mermaid-preview",
      id: MERMAID_SHEET_ISLAND_IDS.mermaidPreview,
      key: MERMAID_SHEET_ISLAND_KEYS.preview,
      "aria-live": "polite",
    }),
    createElement("pre", {
      className: "sheet-result",
      id: MERMAID_SHEET_ISLAND_IDS.mermaidSource,
      key: MERMAID_SHEET_ISLAND_KEYS.source,
    }),
    createElement("div", {
      className: "plan-tabs hidden",
      id: MERMAID_SHEET_ISLAND_IDS.mermaidPlanTabs,
      key: MERMAID_SHEET_ISLAND_KEYS.planTabs,
      "aria-label": "Plan files",
    }),
    createElement("pre", {
      className: "sheet-result hidden",
      id: MERMAID_SHEET_ISLAND_IDS.mermaidPlanContent,
      key: MERMAID_SHEET_ISLAND_KEYS.planContent,
    }),
    createElement(
      "div",
      { className: "sheet-actions", key: MERMAID_SHEET_ISLAND_KEYS.actions },
      createElement(
        "button",
        {
          className: "ghost-button",
          id: MERMAID_SHEET_ISLAND_IDS.mermaidRefreshButton,
          type: "button",
        },
        "Refresh",
      ),
      createElement(
        "button",
        {
          className: "ghost-button",
          id: MERMAID_SHEET_ISLAND_IDS.mermaidOpenButton,
          type: "button",
        },
        "Open Host Artifact",
      ),
      createElement(
        "button",
        {
          className: "ghost-button",
          id: MERMAID_SHEET_ISLAND_IDS.mermaidCloseButton,
          type: "button",
        },
        "Close",
      ),
    ),
  ];
}

export function createMermaidSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Mermaid sheet island requires a createElement function");
  }
  return createElement(
    "section",
    MERMAID_SHEET_ISLAND_HOST_PROPS,
    ...createMermaidSheetContents(createElement),
  );
}

export function MermaidSheet() {
  return createMermaidSheetContents(h);
}

export function resolveMermaidSheetIslandContainers({
  documentRef = globalThis.document,
  mermaidSheet,
} = {}) {
  const sheet = elementFromRef(mermaidSheet)
    ?? documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidSheet)
    ?? null;
  const containers = {
    mermaidSheet: sheet,
    mermaidSheetTitle: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidSheetTitle) ?? null,
    mermaidSummary: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidSummary) ?? null,
    mermaidPreview: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidPreview) ?? null,
    mermaidSource: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidSource) ?? null,
    mermaidPlanTabs: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidPlanTabs) ?? null,
    mermaidPlanContent: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidPlanContent) ?? null,
    mermaidRefreshButton: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidRefreshButton) ?? null,
    mermaidOpenButton: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidOpenButton) ?? null,
    mermaidCloseButton: documentRef?.getElementById?.(MERMAID_SHEET_ISLAND_IDS.mermaidCloseButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Mermaid sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableMermaidSheetIslandContainers(previous, next) {
  return assertStableIdentity(previous, next, { label: "Mermaid sheet island" });
}

export function mountMermaidSheetIsland({
  documentRef = globalThis.document,
  mermaidSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveMermaidSheetIslandContainers({ documentRef, mermaidSheet });
  return mountHydratedStaticIsland({
    containers,
    hydrateRootImpl,
    root: containers.mermaidSheet,
    renderElement: () => h(MermaidSheet),
    refreshContainers: () => resolveMermaidSheetIslandContainers({
      documentRef,
      mermaidSheet: containers.mermaidSheet,
    }),
    assertStableContainers: assertStableMermaidSheetIslandContainers,
  });
}
