import test from "node:test";
import assert from "node:assert/strict";

import {
  buttonIdsFor,
  createElement,
  fakeDocumentForIds,
  keysFor,
} from "./island_test_helpers.mjs";
import {
  THOUGHT_CONFIG_DEFAULT_COPY,
  THOUGHT_CONFIG_MODEL_INPUT_PROPS,
  THOUGHT_CONFIG_SHEET_ISLAND_HOST_PROPS,
  THOUGHT_CONFIG_SHEET_ISLAND_IDS,
  THOUGHT_CONFIG_SHEET_ISLAND_KEYS,
  ThoughtConfigSheet,
  assertStableThoughtConfigSheetIslandContainers,
  createThoughtConfigSheetContents,
  createThoughtConfigSheetElement,
  mountThoughtConfigSheetIsland,
  resolveThoughtConfigSheetIslandContainers,
} from "./thought_config_sheet_island.js";

function fakeDocument() {
  return fakeDocumentForIds(THOUGHT_CONFIG_SHEET_ISLAND_IDS);
}

test("thought config sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createThoughtConfigSheetElement(createElement);
  const children = sheet.props.children;
  const header = children[0];
  const summary = children[1];
  const form = children[2];
  const enabledField = form.props.children[0];
  const backendField = form.props.children[1];
  const modelField = form.props.children[2];
  const hint = form.props.children[3];
  const daemon = form.props.children[4];
  const result = form.props.children[5];
  const actions = form.props.children[6];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...THOUGHT_CONFIG_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.header,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.summary,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.form,
  ]);

  assert.equal(header.props.className, "sheet-header");
  assert.equal(header.props.children[0].props.className, "sheet-eyebrow");
  assert.equal(header.props.children[0].props.children[0], "Policy");
  assert.equal(header.props.children[1].props.id, THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTitle);
  assert.equal(header.props.children[1].props.children[0], "Thought Config");

  assert.equal(summary.type, "div");
  assert.equal(summary.props.className, "sheet-copy");
  assert.equal(summary.props.id, THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSummary);
  assert.equal(summary.props.children[0], THOUGHT_CONFIG_DEFAULT_COPY.summary);

  assert.equal(form.type, "form");
  assert.equal(form.props.className, "sheet-form");
  assert.equal(form.props.id, THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigForm);
  assert.deepEqual(keysFor(form.props.children), [
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.enabledField,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.backendField,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.modelField,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.hint,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.daemon,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.result,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.actions,
  ]);

  assert.equal(enabledField.type, "div");
  assert.equal(enabledField.props.className, "field");
  assert.equal(enabledField.props.children[0].props.children[0], "Enabled");
  assert.equal(enabledField.props.children[1].type, "label");
  assert.equal(enabledField.props.children[1].props.className, "toggle-row");
  assert.deepEqual(enabledField.props.children[1].props.children[0].props, {
    id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigEnabled,
    type: "checkbox",
    children: [],
  });
  assert.equal(enabledField.props.children[1].props.children[1].props.children[0], "Run the thought loop");

  assert.equal(backendField.type, "label");
  assert.equal(backendField.props.className, "field");
  assert.equal(backendField.props.children[0].props.children[0], "Backend");
  assert.deepEqual(backendField.props.children[1].props, {
    id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigBackend,
    children: [],
  });

  assert.equal(modelField.type, "label");
  assert.equal(modelField.props.className, "field");
  assert.equal(modelField.props.children[0].props.children[0], "Model");
  assert.deepEqual(modelField.props.children[1].props, {
    ...THOUGHT_CONFIG_MODEL_INPUT_PROPS,
    children: [],
  });
  assert.deepEqual(modelField.props.children[2].props, {
    id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModelPresets,
    children: [],
  });

  assert.deepEqual(hint.props, {
    className: "sheet-copy",
    id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigHint,
    key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.hint,
    children: [],
  });
  assert.deepEqual(daemon.props, {
    className: "sheet-copy",
    id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigDaemon,
    key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.daemon,
    children: [],
  });
  assert.deepEqual(result.props, {
    className: "sheet-result",
    id: THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigResult,
    key: THOUGHT_CONFIG_SHEET_ISLAND_KEYS.result,
    children: [],
  });

  assert.equal(actions.props.className, "sheet-actions");
  assert.deepEqual(buttonIdsFor(actions.props.children), [
    THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTestButton,
    THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigCloseButton,
    THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSaveButton,
  ]);
  assert.equal(actions.props.children[0].props.className, "ghost-button");
  assert.equal(actions.props.children[0].props.type, "button");
  assert.equal(actions.props.children[0].props.children[0], "Test");
  assert.equal(actions.props.children[1].props.className, "ghost-button");
  assert.equal(actions.props.children[1].props.type, "button");
  assert.equal(actions.props.children[1].props.children[0], "Close");
  assert.equal(actions.props.children[2].props.className, undefined);
  assert.equal(actions.props.children[2].props.type, "submit");
  assert.equal(actions.props.children[2].props.children[0], "Save");
});

test("thought config sheet contents can be rendered independently for fallback parity", () => {
  const contents = createThoughtConfigSheetContents(createElement);
  const form = contents[2];

  assert.deepEqual(keysFor(contents), [
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.header,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.summary,
    THOUGHT_CONFIG_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(contents[0].props.children[1].props.id, THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTitle);
  assert.equal(form.props.children[1].props.children[1].props.id, THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigBackend);
  assert.equal(form.props.children[2].props.children[2].props.id, THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigModelPresets);
  assert.throws(() => createThoughtConfigSheetContents(null), /createElement function/);
  assert.throws(() => createThoughtConfigSheetElement(null), /createElement function/);
});

test("thought config sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountThoughtConfigSheetIsland({
    documentRef,
    hydrateRootImpl(root, element) {
      calls.push(["hydrate", root.id, element.type]);
      return {
        render(nextElement) {
          calls.push(["render", nextElement.type]);
        },
        unmount() {
          calls.push(["unmount"]);
        },
      };
    },
  });
  const before = { ...handle.containers };

  assert.deepEqual(calls[0], ["hydrate", THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSheet, ThoughtConfigSheet]);
  assert.deepEqual(resolveThoughtConfigSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", ThoughtConfigSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableThoughtConfigSheetIslandContainers(before, {
      ...before,
      thoughtConfigSheet: replace(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigSheet),
    }),
    /replaced stable container thoughtConfigSheet/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("thought config sheet island detects every controller target replacement during render", () => {
  for (const key of Object.keys(THOUGHT_CONFIG_SHEET_ISLAND_IDS).filter((idKey) => idKey !== "thoughtConfigSheet")) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountThoughtConfigSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(THOUGHT_CONFIG_SHEET_ISLAND_IDS[key]);
          },
          unmount() {},
        };
      },
    });

    assert.throws(
      () => handle.render(),
      new RegExp(`replaced stable container ${key}`),
    );
  }
});

test("thought config sheet island requires every current event binding target", () => {
  const { documentRef, replace } = fakeDocument();
  const before = resolveThoughtConfigSheetIslandContainers({ documentRef });

  for (const [key, id] of Object.entries(THOUGHT_CONFIG_SHEET_ISLAND_IDS)) {
    assert.equal(before[key].id, id);
  }

  replace(THOUGHT_CONFIG_SHEET_ISLAND_IDS.thoughtConfigTestButton);
  assert.throws(
    () => assertStableThoughtConfigSheetIslandContainers(
      before,
      resolveThoughtConfigSheetIslandContainers({ documentRef }),
    ),
    /replaced stable container thoughtConfigTestButton/,
  );
});
