import React from "react";
import { hydrateRoot } from "react-dom/client";

import { normalizeBootPayload } from "./contracts.js";
import {
  TERMINAL_SURFACE_ISLAND_IDS,
  createTerminalSurfaceIslandElements,
} from "./terminal_island.js";
import {
  TROGDOR_ATLAS_ISLAND_ID,
  createTrogdorAtlasIslandElement,
} from "./trogdor_island.js";

export const SWIMMERS_REACT_ROOT_ID = "swimmers-react-root";

export const SWIMMERS_STABLE_CONTAINER_IDS = Object.freeze({
  terminalStage: "terminal-stage",
  terminalCanvas: TERMINAL_SURFACE_ISLAND_IDS.terminalCanvas,
  hudCanvas: TERMINAL_SURFACE_ISLAND_IDS.hudCanvas,
  terminalFallback: TERMINAL_SURFACE_ISLAND_IDS.terminalFallback,
  terminalA11yMirror: TERMINAL_SURFACE_ISLAND_IDS.terminalA11yMirror,
  terminalAnnouncer: TERMINAL_SURFACE_ISLAND_IDS.terminalAnnouncer,
  trogdorSurface: TROGDOR_ATLAS_ISLAND_ID,
});

const h = React.createElement;

function boolAttr(value) {
  return value ? "true" : "false";
}

export function TerminalSurface() {
  return createTerminalSurfaceIslandElements(h);
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
    h(TerminalSurface),
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
