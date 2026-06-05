import React from "react";
import { hydrateRoot } from "react-dom/client";

import { normalizeBootPayload } from "./contracts.js";
import {
  TROGDOR_ATLAS_ISLAND_ID,
  createTrogdorAtlasIslandElement,
} from "./trogdor_island.js";

export const SWIMMERS_REACT_ROOT_ID = "swimmers-react-root";

export const SWIMMERS_STABLE_CONTAINER_IDS = Object.freeze({
  terminalStage: "terminal-stage",
  terminalCanvas: "terminal-canvas",
  hudCanvas: "hud-canvas",
  terminalFallback: "terminal-fallback",
  terminalA11yMirror: "terminal-a11y-mirror",
  terminalAnnouncer: "terminal-announcer",
  trogdorSurface: TROGDOR_ATLAS_ISLAND_ID,
});

const h = React.createElement;

function boolAttr(value) {
  return value ? "true" : "false";
}

export function SwimmersRootShell({ boot }) {
  const normalizedBoot = normalizeBootPayload(boot);
  return h(
    "main",
    {
      className: "terminal-stage",
      id: "terminal-stage",
      tabIndex: 0,
      role: "application",
      "aria-label": "swimmers rendered control surface",
      "data-franken-term-available": boolAttr(normalizedBoot.franken_term_available),
      "data-focus-layout": boolAttr(normalizedBoot.focus_layout),
    },
    h("canvas", { className: "terminal-canvas hidden", id: "terminal-canvas" }),
    h("canvas", {
      className: "hud-canvas hidden",
      id: "hud-canvas",
      "aria-hidden": "true",
    }),
    h("pre", {
      className: "terminal-fallback hidden",
      id: "terminal-fallback",
      tabIndex: 0,
      "aria-label": "Live terminal text fallback",
    }),
    h("textarea", {
      className: "terminal-a11y-mirror",
      id: "terminal-a11y-mirror",
      "aria-label": "Live terminal text mirror",
      readOnly: true,
      tabIndex: -1,
    }),
    h("div", {
      className: "terminal-announcer",
      id: "terminal-announcer",
      "aria-live": "polite",
      "aria-atomic": "false",
    }),
    h("div", {
      className: "terminal-status-strip",
      id: "terminal-status-strip",
      "aria-live": "polite",
    }),
    h(
      "div",
      {
        className: "terminal-link-tools hidden",
        id: "terminal-link-tools",
        role: "group",
        "aria-label": "Terminal link actions",
      },
      h("span", { id: "terminal-link-text" }),
      h("button", { id: "terminal-link-open", type: "button" }, "Open"),
      h("button", { id: "terminal-link-copy", type: "button" }, "Copy"),
    ),
    h(
      "div",
      {
        className: "loading-overlay visible",
        id: "loading-overlay",
        "aria-hidden": "true",
      },
      h("div", { className: "loading-label", id: "loading-label" }, "Loading FrankenTerm…"),
      h("div", { className: "loading-bar" }, h("div", { className: "loading-bar-fill" })),
    ),
    createTrogdorAtlasIslandElement(h),
  );
}

export function resolveSwimmersReactRoot(documentRef = globalThis.document) {
  return documentRef?.getElementById?.(SWIMMERS_REACT_ROOT_ID) ?? null;
}

export function resolveStableShellContainers(documentRef = globalThis.document) {
  const containers = {};
  for (const [key, id] of Object.entries(SWIMMERS_STABLE_CONTAINER_IDS)) {
    const element = documentRef?.getElementById?.(id) ?? null;
    if (!element) {
      throw new Error(`Swimmers React shell missing stable container #${id}`);
    }
    containers[key] = element;
  }
  return containers;
}

export function assertStableShellContainerIdentity(previous, next) {
  for (const key of Object.keys(SWIMMERS_STABLE_CONTAINER_IDS)) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Swimmers React shell replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountSwimmersRootShell(options = {}) {
  const documentRef = options.documentRef ?? globalThis.document;
  const windowRef = options.windowRef ?? globalThis.window;
  const root = options.root ?? resolveSwimmersReactRoot(documentRef);
  if (!root) {
    throw new Error(`Swimmers React shell requires #${SWIMMERS_REACT_ROOT_ID}`);
  }

  const hydrateRootImpl = options.hydrateRootImpl ?? hydrateRoot;
  const handle = {
    root,
    boot: normalizeBootPayload(options.boot ?? windowRef?.__SWIMMERS_BOOT__),
    containers: resolveStableShellContainers(documentRef),
    reactRoot: null,
    render(nextBoot = handle.boot) {
      const previousContainers = handle.containers;
      handle.boot = normalizeBootPayload(nextBoot);
      handle.reactRoot?.render?.(h(SwimmersRootShell, { boot: handle.boot }));
      handle.containers = assertStableShellContainerIdentity(
        previousContainers,
        resolveStableShellContainers(documentRef),
      );
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(root, h(SwimmersRootShell, { boot: handle.boot }));
  return handle;
}
