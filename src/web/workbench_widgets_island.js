import React from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";

export const WORKBENCH_WIDGETS_ISLAND_IDS = Object.freeze({
  terminalWorkbenchWidgets: "terminal-workbench-widgets",
});

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

function normalizeItems(model = {}) {
  return Array.isArray(model?.items) ? model.items : [];
}

function widgetKey(item, index) {
  return String(item?.key || item?.title || `widget-${index}`);
}

export function createWorkbenchWidgetsElements(createElement, model = {}) {
  if (typeof createElement !== "function") {
    throw new TypeError("Workbench widgets island requires a createElement function");
  }
  const nodes = [];
  if (model?.statusText) {
    nodes.push(createElement(
      "div",
      {
        className: "workbench-action-detail",
        key: "workbench-status",
      },
      String(model.statusText),
    ));
  }
  normalizeItems(model).forEach((item, index) => {
    nodes.push(createElement(
      "details",
      {
        className: "workbench-widget",
        key: widgetKey(item, index),
        open: Boolean(item?.open),
      },
      createElement(
        "summary",
        { key: "summary" },
        createElement("span", { className: "workbench-widget-title", key: "title" }, String(item?.title || "")),
        createElement("span", { className: "workbench-widget-meta", key: "meta" }, String(item?.meta || "")),
      ),
      createElement("div", {
        className: "workbench-widget-body",
        dangerouslySetInnerHTML: { __html: String(item?.bodyHtml || "") },
        key: "body",
      }),
    ));
  });
  return nodes;
}

export function WorkbenchWidgets({ model }) {
  return createWorkbenchWidgetsElements(h, model);
}

export function resolveWorkbenchWidgetsIslandContainers({
  documentRef = globalThis.document,
  terminalWorkbenchWidgets,
} = {}) {
  const containers = {
    terminalWorkbenchWidgets: documentRef?.getElementById?.(WORKBENCH_WIDGETS_ISLAND_IDS.terminalWorkbenchWidgets)
      ?? elementFromRef(terminalWorkbenchWidgets)
      ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Workbench widgets island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableWorkbenchWidgetsIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Workbench widgets island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountWorkbenchWidgetsIsland({
  documentRef = globalThis.document,
  terminalWorkbenchWidgets,
  createRootImpl = createRoot,
  flushSyncImpl = flushSync,
} = {}) {
  const containers = resolveWorkbenchWidgetsIslandContainers({ documentRef, terminalWorkbenchWidgets });
  const handle = {
    containers,
    reactRoot: createRootImpl(containers.terminalWorkbenchWidgets),
    render(model = {}) {
      const previousContainers = handle.containers;
      const renderRoot = () => {
        handle.reactRoot?.render?.(h(WorkbenchWidgets, { model }));
      };
      if (typeof flushSyncImpl === "function") {
        flushSyncImpl(renderRoot);
      } else {
        renderRoot();
      }
      handle.containers = assertStableWorkbenchWidgetsIslandContainers(
        previousContainers,
        resolveWorkbenchWidgetsIslandContainers({ documentRef, terminalWorkbenchWidgets }),
      );
      return true;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  return handle;
}
