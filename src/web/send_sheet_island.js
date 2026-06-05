import React from "react";
import { hydrateRoot } from "react-dom/client";

export const SEND_SHEET_ISLAND_IDS = Object.freeze({
  sendSheet: "send-sheet",
  sendSheetTitle: "send-sheet-title",
  sendForm: "send-form",
  sendMode: "send-mode",
  sendInput: "send-input",
  sendHistory: "send-history",
  sendHint: "send-hint",
  sendCloseButton: "send-close-button",
  sendSubmitButton: "send-submit-button",
});

export const SEND_SHEET_ISLAND_KEYS = Object.freeze({
  header: "send-header",
  form: "send-form",
  modeField: "send-mode-field",
  inputField: "send-input-field",
  history: "send-history",
  hint: "send-hint",
  actions: "send-actions",
});

export const SEND_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden",
  id: SEND_SHEET_ISLAND_IDS.sendSheet,
  "aria-labelledby": SEND_SHEET_ISLAND_IDS.sendSheetTitle,
});

export const SEND_SHEET_INPUT_PROPS = Object.freeze({
  id: SEND_SHEET_ISLAND_IDS.sendInput,
  rows: 5,
  placeholder: "Type a command or paste text. Send appends a newline.",
});

export const SEND_SHEET_HISTORY_PROPS = Object.freeze({
  className: "send-history",
  id: SEND_SHEET_ISLAND_IDS.sendHistory,
  "aria-label": "Recent sends",
});

const h = React.createElement;

function elementFromRef(ref) {
  return ref?.current ?? ref;
}

function keyedProps(key, props = {}) {
  return { ...props, key };
}

export function createSendSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Send sheet island requires a createElement function");
  }
  return [
    createElement(
      "div",
      { className: "sheet-header", key: SEND_SHEET_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Rendered Action"),
      createElement("h2", { id: SEND_SHEET_ISLAND_IDS.sendSheetTitle }, "Send Line"),
    ),
    createElement(
      "form",
      {
        className: "sheet-form",
        id: SEND_SHEET_ISLAND_IDS.sendForm,
        key: SEND_SHEET_ISLAND_KEYS.form,
      },
      createElement(
        "label",
        { className: "field", key: SEND_SHEET_ISLAND_KEYS.modeField },
        createElement("span", null, "Mode"),
        createElement(
          "select",
          { id: SEND_SHEET_ISLAND_IDS.sendMode },
          createElement("option", { value: "line" }, "Send + Enter"),
          createElement("option", { value: "paste" }, "Paste only"),
        ),
      ),
      createElement(
        "label",
        { className: "field", key: SEND_SHEET_ISLAND_KEYS.inputField },
        createElement("span", null, "Input"),
        createElement("textarea", SEND_SHEET_INPUT_PROPS),
      ),
      createElement("div", keyedProps(SEND_SHEET_ISLAND_KEYS.history, SEND_SHEET_HISTORY_PROPS)),
      createElement(
        "div",
        {
          className: "sheet-copy",
          id: SEND_SHEET_ISLAND_IDS.sendHint,
          key: SEND_SHEET_ISLAND_KEYS.hint,
        },
        "Send submits the text to the selected agent prompt. Paste only preserves text exactly for the selected live terminal.",
      ),
      createElement(
        "div",
        { className: "sheet-actions", key: SEND_SHEET_ISLAND_KEYS.actions },
        createElement(
          "button",
          {
            className: "ghost-button",
            id: SEND_SHEET_ISLAND_IDS.sendCloseButton,
            type: "button",
          },
          "Cancel",
        ),
        createElement(
          "button",
          {
            id: SEND_SHEET_ISLAND_IDS.sendSubmitButton,
            type: "submit",
          },
          "Send",
        ),
      ),
    ),
  ];
}

export function createSendSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Send sheet island requires a createElement function");
  }
  return createElement(
    "section",
    SEND_SHEET_ISLAND_HOST_PROPS,
    ...createSendSheetContents(createElement),
  );
}

export function SendSheet() {
  return createSendSheetContents(h);
}

export function resolveSendSheetIslandContainers({
  documentRef = globalThis.document,
  sendSheet,
} = {}) {
  const sheet = elementFromRef(sendSheet)
    ?? documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendSheet)
    ?? null;
  const containers = {
    sendSheet: sheet,
    sendSheetTitle: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendSheetTitle) ?? null,
    sendForm: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendForm) ?? null,
    sendMode: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendMode) ?? null,
    sendInput: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendInput) ?? null,
    sendHistory: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendHistory) ?? null,
    sendHint: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendHint) ?? null,
    sendCloseButton: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendCloseButton) ?? null,
    sendSubmitButton: documentRef?.getElementById?.(SEND_SHEET_ISLAND_IDS.sendSubmitButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Send sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableSendSheetIslandContainers(previous, next) {
  for (const key of Object.keys(previous || {})) {
    if (previous?.[key] !== next?.[key]) {
      throw new Error(`Send sheet island replaced stable container ${key}`);
    }
  }
  return next;
}

export function mountSendSheetIsland({
  documentRef = globalThis.document,
  sendSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveSendSheetIslandContainers({ documentRef, sendSheet });
  const handle = {
    containers,
    reactRoot: null,
    render() {
      const previousContainers = handle.containers;
      handle.reactRoot?.render?.(h(SendSheet));
      handle.containers = assertStableSendSheetIslandContainers(
        previousContainers,
        resolveSendSheetIslandContainers({ documentRef, sendSheet: containers.sendSheet }),
      );
      return handle;
    },
    unmount() {
      handle.reactRoot?.unmount?.();
    },
  };
  handle.reactRoot = hydrateRootImpl(containers.sendSheet, h(SendSheet));
  return handle;
}
