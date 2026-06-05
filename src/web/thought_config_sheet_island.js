import React from "react";
import { hydrateRoot } from "react-dom/client";

import { assertStableIdentity, elementFromRef } from "./react_island_identity.js";
import { mountHydratedStaticIsland } from "./static_sheet_island.js";

export const THOUGHT_CONFIG_SHEET_ISLAND_IDS = Object.freeze({
  thoughtConfigSheet: "thought-config-sheet",
  thoughtConfigTitle: "thought-config-title",
  thoughtConfigForm: "thought-config-form",
  thoughtConfigEnabled: "thought-config-enabled",
  thoughtConfigBackend: "thought-config-backend",
  thoughtConfigModel: "thought-config-model",
  thoughtConfigModelPresets: "thought-config-model-presets",
  thoughtConfigHint: "thought-config-hint",
  thoughtConfigSummary: "thought-config-summary",
  thoughtConfigDaemon: "thought-config-daemon",
  thoughtConfigResult: "thought-config-result",
  thoughtConfigTestButton: "thought-config-test-button",
  thoughtConfigCloseButton: "thought-config-close-button",
  thoughtConfigSaveButton: "thought-config-save-button",
});

export const THOUGHT_CONFIG_SHEET_ISLAND_KEYS = Object.freeze({
  header: "thought-config-header",
  summary: "thought-config-summary",
  form: "thought-config-form",
  enabledField: "thought-config-enabled-field",
  backendField: "thought-config-backend-field",
  modelField: "thought-config-model-field",
  hint: "thought-config-hint",
  daemon: "thought-config-daemon",
  result: "thought-config-result",
  actions: "thought-config-actions",
});

export const THOUGHT_CONFIG_SHEET_ISLAND_HOST_PROPS = Object.freeze({
  className: "surface-sheet hidden",
  id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSheet,
  "aria-labelledby": THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTitle,
});

export const THOUGHT_CONFIG_MODEL_INPUT_PROPS = Object.freeze({
  id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModel,
  type: "text",
  placeholder: "Use backend default or choose a preset",
  autoComplete: "off",
  list: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModelPresets,
});

export const THOUGHT_CONFIG_DEFAULT_COPY = Object.freeze({
  summary: "Loading thought config…",
});

const h = React.createElement;

export function createThoughtConfigSheetContents(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Thought config sheet island requires a createElement function");
  }
  return [
    createElement(
      "div",
      { className: "sheet-header", key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.header },
      createElement("p", { className: "sheet-eyebrow" }, "Policy"),
      createElement("h2", { id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTitle }, "Thought Config"),
    ),
    createElement(
      "div",
      {
        className: "sheet-copy",
        id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSummary,
        key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.summary,
      },
      THOUGHT_CONFIG_DEFAULT_COPY.summary,
    ),
    createElement(
      "form",
      {
        className: "sheet-form",
        id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigForm,
        key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.form,
      },
      createElement(
        "div",
        { className: "field", key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.enabledField },
        createElement("span", null, "Enabled"),
        createElement(
          "label",
          { className: "toggle-row" },
          createElement("input", {
            id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigEnabled,
            type: "checkbox",
          }),
          createElement("span", null, "Run the thought loop"),
        ),
      ),
      createElement(
        "label",
        { className: "field", key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.backendField },
        createElement("span", null, "Backend"),
        createElement("select", { id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigBackend }),
      ),
      createElement(
        "label",
        { className: "field", key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.modelField },
        createElement("span", null, "Model"),
        createElement("input", THOUGHT_CONFIG_MODEL_INPUT_PROPS),
        createElement("datalist", { id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModelPresets }),
      ),
      createElement("div", {
        className: "sheet-copy",
        id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigHint,
        key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.hint,
      }),
      createElement("div", {
        className: "sheet-copy",
        id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigDaemon,
        key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.daemon,
      }),
      createElement("pre", {
        className: "sheet-result",
        id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigResult,
        key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.result,
      }),
      createElement(
        "div",
        { className: "sheet-actions", key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.actions },
        createElement(
          "button",
          {
            className: "ghost-button",
            id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTestButton,
            type: "button",
          },
          "Test",
        ),
        createElement(
          "button",
          {
            className: "ghost-button",
            id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigCloseButton,
            type: "button",
          },
          "Close",
        ),
        createElement(
          "button",
          {
            id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSaveButton,
            type: "submit",
          },
          "Save",
        ),
      ),
    ),
  ];
}

export function createThoughtConfigSheetElement(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Thought config sheet island requires a createElement function");
  }
  return createElement(
    "section",
    THOUGHT_CONFIG_SHEET_ISLAND_HOST_PROPS,
    ...createThoughtConfigSheetContents(createElement),
  );
}

export function ThoughtConfigSheet() {
  return createThoughtConfigSheetContents(h);
}

export function resolveThoughtConfigSheetIslandContainers({
  documentRef = globalThis.document,
  thoughtConfigSheet,
} = {}) {
  const sheet = elementFromRef(thoughtConfigSheet)
    ?? documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSheet)
    ?? null;
  const containers = {
    thoughtConfigSheet: sheet,
    thoughtConfigTitle: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTitle) ?? null,
    thoughtConfigForm: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigForm) ?? null,
    thoughtConfigEnabled: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigEnabled) ?? null,
    thoughtConfigBackend: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigBackend) ?? null,
    thoughtConfigModel: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModel) ?? null,
    thoughtConfigModelPresets: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModelPresets) ?? null,
    thoughtConfigHint: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigHint) ?? null,
    thoughtConfigSummary: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSummary) ?? null,
    thoughtConfigDaemon: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigDaemon) ?? null,
    thoughtConfigResult: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigResult) ?? null,
    thoughtConfigTestButton: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTestButton) ?? null,
    thoughtConfigCloseButton: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigCloseButton) ?? null,
    thoughtConfigSaveButton: documentRef?.getElementById?.(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSaveButton) ?? null,
  };
  for (const [key, value] of Object.entries(containers)) {
    if (!value) {
      throw new Error(`Thought config sheet island missing stable container ${key}`);
    }
  }
  return containers;
}

export function assertStableThoughtConfigSheetIslandContainers(previous, next) {
  return assertStableIdentity(previous, next, { label: "Thought config sheet island" });
}

export function mountThoughtConfigSheetIsland({
  documentRef = globalThis.document,
  thoughtConfigSheet,
  hydrateRootImpl = hydrateRoot,
} = {}) {
  const containers = resolveThoughtConfigSheetIslandContainers({ documentRef, thoughtConfigSheet });
  return mountHydratedStaticIsland({
    containers,
    hydrateRootImpl,
    root: containers.thoughtConfigSheet,
    renderElement: () => h(ThoughtConfigSheet),
    refreshContainers: () => resolveThoughtConfigSheetIslandContainers({
      documentRef,
      thoughtConfigSheet: containers.thoughtConfigSheet,
    }),
    assertStableContainers: assertStableThoughtConfigSheetIslandContainers,
  });
}
