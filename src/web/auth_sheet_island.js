import React from "react";
import { hydrateRoot } from "react-dom/client";

export const AUTH_SHEET_ISLAND_IDS = Object.freeze({
  authSheet: "auth-sheet",
  authSheetTitle: "auth-sheet-title",
  tokenInput: "token-input",
  clearTokenButton: "clear-token-button",
  authCloseButton: "auth-close-button",
  saveTokenButton: "save-token-button",
});

export const AUTH_SHEET_ISLAND_KEYS = Object.freeze({
  header: "auth-header",
  copy: "auth-copy",
  form: "auth-form",
  field: "auth-field",
  actions: "auth-actions",
});

export const AUTH_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden",
  id: AUTH_SHEET_ISLAND_IDS.authSheet,
  "aria-labelledby": AUTH_SHEET_ISLAND_IDS.authSheetTitle,
});

export const AUTH_SHEET_TOKEN_INPUT_PROPS = Object.freeze({
  id: AUTH_SHEET_ISLAND_IDS.tokenInput,
  type: "password",
  placeholder: "Optional bearer token",
  autoComplete: "off",
});

export const AUTH_SHEET_COPY =
  "Paste `AUTH_TOKEN` or `OBSERVER_TOKEN` when the API is running in token mode.";

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

export function createAuthSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Auth sheet island requires a createElement function");
  }
  return [
    createElement(
      "div",
      { className: "sheet-header", key: AUTH_SHEET_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Connection"),
      createElement("h2", { id: AUTH_SHEET_ISLAND_IDS.authSheetTitle }, "Auth Token"),
    ),
    createElement(
      "div",
      { className: "sheet-copy", key: AUTH_SHEET_ISLAND_KEYS.copy },
      AUTH_SHEET_COPY,
    ),
    createElement(
      "div",
      { className: "sheet-form", key: AUTH_SHEET_ISLAND_KEYS.form },
      createElement(
        "label",
        { className: "field", key: AUTH_SHEET_ISLAND_KEYS.field },
        createElement("span", null, "Token"),
        createElement("input", AUTH_SHEET_TOKEN_INPUT_PROPS),
      ),
      createElement(
        "div",
        { className: "sheet-actions", key: AUTH_SHEET_ISLAND_KEYS.actions },
        createElement(
          "button",
          {
            className: "ghost-button",
            id: AUTH_SHEET_ISLAND_IDS.clearTokenButton,
            type: "button",
          },
          "Forget",
        ),
        createElement(
          "button",
          {
            className: "ghost-button",
            id: AUTH_SHEET_ISLAND_IDS.authCloseButton,
            type: "button",
          },
          "Close",
        ),
        createElement(
          "button",
          {
            id: AUTH_SHEET_ISLAND_IDS.saveTokenButton,
            type: "button",
          },
          "Connect",
        ),
      ),
    ),
  ];
}

export function createAuthSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Auth sheet island requires a createElement function");
  }
  return createElement(
    "section",
    AUTH_SHEET_ISLAND_HOST_PROPS,
    ...createAuthSheetContents(createElement),
  );
}

export function AuthSheet() {
  return createAuthSheetContents(h);
}

export function resolveAuthSheetIslandContainers({
  documentRef = globalThis.document,
  authSheet,
} = {}) {
  const sheet = elementFromRef(authSheet)
    ?? documentRef?.getElementById?.(AUTH_SHEET_ISLAND_IDS.authSheet)
    ?? null;
  const containers = {
    authSheet: sheet,
    authSheetTitle: documentRef?.getElementById?.(AUTH_SHEET_ISLAND_IDS.authSheetTitle) ?? null,
    tokenInput: documentRef?.getElementById?.(AUTH_SHEET_ISLAND_IDS.tokenInput) ?? null,
    clearTokenButton: documentRef?.getElementById?.(AUTH_SHEET_ISLAND_IDS.clearTokenButton) ?? null,
    authCloseButton: documentRef?.getElementById?.(AUTH_SHEET_ISLAND_IDS.authCloseButton) ?? null,
    saveTokenButton: documentRef?.getElementById?.(AUTH_SHEET_ISLAND_IDS.saveTokenButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Auth sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableAuthSheetIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Auth sheet island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountAuthSheetIsland({
  documentRef = globalThis.document,
  authSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveAuthSheetIslandContainers({ documentRef, authSheet });
  const handle = {
    containers,
    reactRoot: null,
    render() {
      const previousContainers = handle.containers;
      handle.reactRoot?.render?.(h(AuthSheet));
      handle.containers = assertStableAuthSheetIslandContainers(
        previousContainers,
        resolveAuthSheetIslandContainers({ documentRef, authSheet: containers.authSheet }),
      );
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(containers.authSheet, h(AuthSheet));
  return handle;
}
