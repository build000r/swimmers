import React from "react";
import { hydrateRoot } from "react-dom/client";

export const NATIVE_DESKTOP_SHEET_ISLAND_IDS = Object.freeze({
  nativeSheet: "native-sheet",
  nativeSheetTitle: "native-sheet-title",
  nativeForm: "native-form",
  nativeStatusCopy: "native-status-copy",
  nativeApp: "native-app",
  nativeMode: "native-mode",
  nativeStatusResult: "native-status-result",
  nativeRefreshButton: "native-refresh-button",
  nativeOpenButton: "native-open-button",
  nativeCloseButton: "native-close-button",
  nativeSaveButton: "native-save-button",
});

export const NATIVE_DESKTOP_SHEET_ISLAND_KEYS = Object.freeze({
  header: "native-header",
  statusCopy: "native-status-copy",
  form: "native-form",
  appField: "native-app-field",
  modeField: "native-mode-field",
  result: "native-status-result",
  actions: "native-actions",
});

export const NATIVE_DESKTOP_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden",
  id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheet,
  "aria-labelledby": NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheetTitle,
});

export const NATIVE_DESKTOP_DEFAULT_COPY = Object.freeze({
  status: "Loading native status…",
});

export const NATIVE_DESKTOP_APP_OPTIONS = Object.freeze([
  Object.freeze({ value: "iterm", label: "iTerm" }),
  Object.freeze({ value: "ghostty", label: "Ghostty" }),
]);

export const NATIVE_DESKTOP_MODE_OPTIONS = Object.freeze([
  Object.freeze({ value: "swap", label: "swap" }),
  Object.freeze({ value: "add", label: "add" }),
]);

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

function optionElements(createElement, options) {
  return options.map((option) => createElement(
    "option",
    { key: option.value, value: option.value },
    option.label,
  ));
}

export function createNativeDesktopSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Native desktop sheet island requires a createElement function");
  }
  return [
    createElement(
      "div",
      { className: "sheet-header", key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Desktop"),
      createElement("h2", { id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheetTitle }, "Native Open"),
    ),
    createElement(
      "div",
      {
        className: "sheet-copy",
        id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusCopy,
        key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.statusCopy,
      },
      NATIVE_DESKTOP_DEFAULT_COPY.status,
    ),
    createElement(
      "form",
      {
        className: "sheet-form",
        id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeForm,
        key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.form,
      },
      createElement(
        "label",
        { className: "field", key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.appField },
        createElement("span", null, "App"),
        createElement(
          "select",
          { id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeApp },
          ...optionElements(createElement, NATIVE_DESKTOP_APP_OPTIONS),
        ),
      ),
      createElement(
        "label",
        { className: "field", key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.modeField },
        createElement("span", null, "Ghostty mode"),
        createElement(
          "select",
          { id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeMode },
          ...optionElements(createElement, NATIVE_DESKTOP_MODE_OPTIONS),
        ),
      ),
      createElement("pre", {
        className: "sheet-result",
        id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusResult,
        key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.result,
      }),
      createElement(
        "div",
        { className: "sheet-actions", key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.actions },
        createElement(
          "button",
          {
            className: "ghost-button",
            id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeRefreshButton,
            type: "button",
          },
          "Refresh",
        ),
        createElement(
          "button",
          {
            className: "ghost-button",
            id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeOpenButton,
            type: "button",
          },
          "Open Selected",
        ),
        createElement(
          "button",
          {
            className: "ghost-button",
            id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeCloseButton,
            type: "button",
          },
          "Close",
        ),
        createElement(
          "button",
          {
            id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSaveButton,
            type: "submit",
          },
          "Apply",
        ),
      ),
    ),
  ];
}

export function createNativeDesktopSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Native desktop sheet island requires a createElement function");
  }
  return createElement(
    "section",
    NATIVE_DESKTOP_SHEET_ISLAND_HOST_PROPS,
    ...createNativeDesktopSheetContents(createElement),
  );
}

export function NativeDesktopSheet() {
  return createNativeDesktopSheetContents(h);
}

export function resolveNativeDesktopSheetIslandContainers({
  documentRef = globalThis.document,
  nativeSheet,
} = {}) {
  const sheet = elementFromRef(nativeSheet)
    ?? documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheet)
    ?? null;
  const containers = {
    nativeSheet: sheet,
    nativeSheetTitle: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheetTitle) ?? null,
    nativeForm: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeForm) ?? null,
    nativeStatusCopy: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusCopy) ?? null,
    nativeApp: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeApp) ?? null,
    nativeMode: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeMode) ?? null,
    nativeStatusResult: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusResult) ?? null,
    nativeRefreshButton: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeRefreshButton) ?? null,
    nativeOpenButton: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeOpenButton) ?? null,
    nativeCloseButton: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeCloseButton) ?? null,
    nativeSaveButton: documentRef?.getElementById?.(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSaveButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Native desktop sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableNativeDesktopSheetIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Native desktop sheet island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountNativeDesktopSheetIsland({
  documentRef = globalThis.document,
  nativeSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveNativeDesktopSheetIslandContainers({ documentRef, nativeSheet });
  const handle = {
    containers,
    reactRoot: null,
    render() {
      const previousContainers = handle.containers;
      handle.reactRoot?.render?.(h(NativeDesktopSheet));
      handle.containers = assertStableNativeDesktopSheetIslandContainers(
        previousContainers,
        resolveNativeDesktopSheetIslandContainers({
          documentRef,
          nativeSheet: containers.nativeSheet,
        }),
      );
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(containers.nativeSheet, h(NativeDesktopSheet));
  return handle;
}
