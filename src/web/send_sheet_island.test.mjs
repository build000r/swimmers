import test from "node:test";
import assert from "node:assert/strict";

import {
  buttonIdsFor,
  createElement,
  fakeDocumentForIds,
  keysFor,
} from "./island_test_helpers.mjs";
import {
  SEND_SHEET_HISTORY_PROPS,
  SEND_SHEET_INPUT_PROPS,
  SEND_SHEET_ISLAND_HOST_PROPS,
  SEND_SHEET_ISLAND_IDS,
  SEND_SHEET_ISLAND_KEYS,
  SendSheet,
  assertStableSendSheetIslandContainers,
  createSendSheetContents,
  createSendSheetElement,
  mountSendSheetIsland,
  resolveSendSheetIslandContainers,
} from "./send_sheet_island.js";

function fakeDocument() {
  return fakeDocumentForIds(SEND_SHEET_ISLAND_IDS);
}

test("send sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createSendSheetElement(createElement);
  const children = sheet.props.children;
  const header = children[0];
  const form = children[1];
  const modeField = form.props.children[0];
  const inputField = form.props.children[1];
  const history = form.props.children[2];
  const hint = form.props.children[3];
  const actions = form.props.children[4];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...SEND_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    SEND_SHEET_ISLAND_KEYS.header,
    SEND_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(header.props.className, "sheet-header");
  assert.equal(header.props.children[0].props.className, "sheet-eyebrow");
  assert.equal(header.props.children[0].props.children[0], "Rendered Action");
  assert.equal(header.props.children[1].props.id, SEND_SHEET_ISLAND_IDS.sendSheetTitle);
  assert.equal(header.props.children[1].props.children[0], "Send Line");

  assert.equal(form.type, "form");
  assert.equal(form.props.className, "sheet-form");
  assert.equal(form.props.id, SEND_SHEET_ISLAND_IDS.sendForm);
  assert.deepEqual(keysFor(form.props.children), [
    SEND_SHEET_ISLAND_KEYS.modeField,
    SEND_SHEET_ISLAND_KEYS.inputField,
    SEND_SHEET_ISLAND_KEYS.history,
    SEND_SHEET_ISLAND_KEYS.hint,
    SEND_SHEET_ISLAND_KEYS.actions,
  ]);

  assert.equal(modeField.type, "label");
  assert.equal(modeField.props.className, "field");
  assert.equal(modeField.props.children[0].props.children[0], "Mode");
  assert.equal(modeField.props.children[1].type, "select");
  assert.equal(modeField.props.children[1].props.id, SEND_SHEET_ISLAND_IDS.sendMode);
  assert.equal(modeField.props.children[1].props.children[0].props.value, "line");
  assert.equal(modeField.props.children[1].props.children[0].props.children[0], "Send + Enter");
  assert.equal(modeField.props.children[1].props.children[1].props.value, "paste");
  assert.equal(modeField.props.children[1].props.children[1].props.children[0], "Paste only");

  assert.equal(inputField.type, "label");
  assert.equal(inputField.props.className, "field");
  assert.equal(inputField.props.children[0].props.children[0], "Input");
  assert.deepEqual(inputField.props.children[1].props, {
    ...SEND_SHEET_INPUT_PROPS,
    children: [],
  });

  assert.deepEqual(history.props, {
    ...SEND_SHEET_HISTORY_PROPS,
    key: SEND_SHEET_ISLAND_KEYS.history,
    children: [],
  });
  assert.equal(hint.props.className, "sheet-copy");
  assert.equal(hint.props.id, SEND_SHEET_ISLAND_IDS.sendHint);
  assert.equal(
    hint.props.children[0],
    "Send submits the text to the selected agent prompt. Paste only preserves text exactly for the selected live terminal.",
  );

  assert.equal(actions.props.className, "sheet-actions");
  assert.deepEqual(buttonIdsFor(actions.props.children), [
    SEND_SHEET_ISLAND_IDS.sendCloseButton,
    SEND_SHEET_ISLAND_IDS.sendSubmitButton,
  ]);
  assert.equal(actions.props.children[0].props.className, "ghost-button");
  assert.equal(actions.props.children[0].props.type, "button");
  assert.equal(actions.props.children[0].props.children[0], "Cancel");
  assert.equal(actions.props.children[1].props.type, "submit");
  assert.equal(actions.props.children[1].props.children[0], "Send");
});

test("send sheet contents can be rendered independently for fallback parity", () => {
  const contents = createSendSheetContents(createElement);
  const form = contents[1];

  assert.deepEqual(keysFor(contents), [
    SEND_SHEET_ISLAND_KEYS.header,
    SEND_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(form.props.children[0].props.children[1].props.id, SEND_SHEET_ISLAND_IDS.sendMode);
  assert.equal(form.props.children[1].props.children[1].props.id, SEND_SHEET_ISLAND_IDS.sendInput);
  assert.throws(() => createSendSheetContents(null), /createElement function/);
  assert.throws(() => createSendSheetElement(null), /createElement function/);
});

test("send sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountSendSheetIsland({
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

  assert.deepEqual(calls[0], ["hydrate", SEND_SHEET_ISLAND_IDS.sendSheet, SendSheet]);
  assert.deepEqual(resolveSendSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", SendSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableSendSheetIslandContainers(before, {
      ...before,
      sendSheet: replace(SEND_SHEET_ISLAND_IDS.sendSheet),
    }),
    /replaced stable container sendSheet/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("send sheet island detects controller target replacement during render", () => {
  for (const key of [
    "sendSheetTitle",
    "sendForm",
    "sendMode",
    "sendInput",
    "sendHistory",
    "sendHint",
    "sendCloseButton",
    "sendSubmitButton",
  ]) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountSendSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(SEND_SHEET_ISLAND_IDS[key]);
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

test("send sheet island requires every current event binding target", () => {
  const { documentRef, replace } = fakeDocument();
  const before = resolveSendSheetIslandContainers({ documentRef });

  assert.equal(before.sendSheet.id, SEND_SHEET_ISLAND_IDS.sendSheet);
  assert.equal(before.sendForm.id, SEND_SHEET_ISLAND_IDS.sendForm);
  assert.equal(before.sendMode.id, SEND_SHEET_ISLAND_IDS.sendMode);
  assert.equal(before.sendCloseButton.id, SEND_SHEET_ISLAND_IDS.sendCloseButton);

  replace(SEND_SHEET_ISLAND_IDS.sendHistory);
  assert.throws(
    () => assertStableSendSheetIslandContainers(before, resolveSendSheetIslandContainers({ documentRef })),
    /replaced stable container sendHistory/,
  );
});
