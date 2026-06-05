import test from "node:test";
import assert from "node:assert/strict";

import {
  NATIVE_DESKTOP_APP_OPTIONS,
  NATIVE_DESKTOP_DEFAULT_COPY,
  NATIVE_DESKTOP_MODE_OPTIONS,
  NATIVE_DESKTOP_SHEET_ISLAND_HOST_PROPS,
  NATIVE_DESKTOP_SHEET_ISLAND_IDS,
  NATIVE_DESKTOP_SHEET_ISLAND_KEYS,
  NativeDesktopSheet,
  assertStableNativeDesktopSheetIslandContainers,
  createNativeDesktopSheetContents,
  createNativeDesktopSheetElement,
  mountNativeDesktopSheetIsland,
  resolveNativeDesktopSheetIslandContainers,
} from "./native_desktop_sheet_island.js";

function fakeElement(id) {
  return { id };
}

function createElement(type, props, ...children) {
  return {
    type,
    props: { ...(props || {}), children },
  };
}

function fakeDocument() {
  const elements = new Map(Object.values(NATIVE_DESKTOP_SHEET_ISLAND_IDS).map((id) => [id, fakeElement(id)]));
  return {
    documentRef: {
      getElementById(id) {
        return elements.get(id) ?? null;
      },
    },
    delete(id) {
      elements.delete(id);
    },
    replace(id) {
      const replacement = { id, replaced: true };
      elements.set(id, replacement);
      return replacement;
    },
  };
}

function keysFor(children) {
  return children.map((child) => child?.props?.key).filter(Boolean);
}

function optionPairsFor(select) {
  return select.props.children.map((option) => [
    option.props.value,
    option.props.children[0],
  ]);
}

function buttonIdsFor(children) {
  return children.map((child) => child?.props?.id).filter(Boolean);
}

test("native desktop sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createNativeDesktopSheetElement(createElement);
  const children = sheet.props.children;
  const header = children[0];
  const statusCopy = children[1];
  const form = children[2];
  const appField = form.props.children[0];
  const modeField = form.props.children[1];
  const result = form.props.children[2];
  const actions = form.props.children[3];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...NATIVE_DESKTOP_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.header,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.statusCopy,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.form,
  ]);

  assert.equal(header.props.className, "sheet-header");
  assert.equal(header.props.children[0].props.className, "sheet-eyebrow");
  assert.equal(header.props.children[0].props.children[0], "Desktop");
  assert.equal(header.props.children[1].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheetTitle);
  assert.equal(header.props.children[1].props.children[0], "Native Open");

  assert.equal(statusCopy.type, "div");
  assert.equal(statusCopy.props.className, "sheet-copy");
  assert.equal(statusCopy.props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusCopy);
  assert.equal(statusCopy.props.children[0], NATIVE_DESKTOP_DEFAULT_COPY.status);

  assert.equal(form.type, "form");
  assert.equal(form.props.className, "sheet-form");
  assert.equal(form.props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeForm);
  assert.deepEqual(keysFor(form.props.children), [
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.appField,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.modeField,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.result,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.actions,
  ]);

  assert.equal(appField.type, "label");
  assert.equal(appField.props.className, "field");
  assert.equal(appField.props.children[0].props.children[0], "App");
  assert.equal(appField.props.children[1].type, "select");
  assert.equal(appField.props.children[1].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeApp);
  assert.deepEqual(optionPairsFor(appField.props.children[1]), NATIVE_DESKTOP_APP_OPTIONS.map((option) => [
    option.value,
    option.label,
  ]));

  assert.equal(modeField.type, "label");
  assert.equal(modeField.props.className, "field");
  assert.equal(modeField.props.children[0].props.children[0], "Ghostty mode");
  assert.equal(modeField.props.children[1].type, "select");
  assert.equal(modeField.props.children[1].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeMode);
  assert.deepEqual(optionPairsFor(modeField.props.children[1]), NATIVE_DESKTOP_MODE_OPTIONS.map((option) => [
    option.value,
    option.label,
  ]));

  assert.deepEqual(result.props, {
    className: "sheet-result",
    id: NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusResult,
    key: NATIVE_DESKTOP_SHEET_ISLAND_KEYS.result,
    children: [],
  });

  assert.equal(actions.props.className, "sheet-actions");
  assert.deepEqual(buttonIdsFor(actions.props.children), [
    NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeRefreshButton,
    NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeOpenButton,
    NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeCloseButton,
    NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSaveButton,
  ]);
  assert.equal(actions.props.children[0].props.className, "ghost-button");
  assert.equal(actions.props.children[0].props.type, "button");
  assert.equal(actions.props.children[0].props.children[0], "Refresh");
  assert.equal(actions.props.children[1].props.className, "ghost-button");
  assert.equal(actions.props.children[1].props.type, "button");
  assert.equal(actions.props.children[1].props.children[0], "Open Selected");
  assert.equal(actions.props.children[2].props.className, "ghost-button");
  assert.equal(actions.props.children[2].props.type, "button");
  assert.equal(actions.props.children[2].props.children[0], "Close");
  assert.equal(actions.props.children[3].props.className, undefined);
  assert.equal(actions.props.children[3].props.type, "submit");
  assert.equal(actions.props.children[3].props.children[0], "Apply");
});

test("native desktop sheet contents can be rendered independently for fallback parity", () => {
  const contents = createNativeDesktopSheetContents(createElement);
  const form = contents[2];

  assert.deepEqual(keysFor(contents), [
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.header,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.statusCopy,
    NATIVE_DESKTOP_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(contents[0].props.children[1].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheetTitle);
  assert.equal(form.props.children[0].props.children[1].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeApp);
  assert.equal(form.props.children[1].props.children[1].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeMode);
  assert.equal(form.props.children[2].props.id, NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeStatusResult);
  assert.throws(() => createNativeDesktopSheetContents(null), /createElement function/);
  assert.throws(() => createNativeDesktopSheetElement(null), /createElement function/);
});

test("native desktop sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountNativeDesktopSheetIsland({
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

  assert.deepEqual(calls[0], ["hydrate", NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheet, NativeDesktopSheet]);
  assert.deepEqual(resolveNativeDesktopSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", NativeDesktopSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableNativeDesktopSheetIslandContainers(before, {
      ...before,
      nativeSheet: replace(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeSheet),
    }),
    /replaced stable container nativeSheet/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("native desktop sheet island detects every controller target replacement during render", () => {
  for (const key of Object.keys(NATIVE_DESKTOP_SHEET_ISLAND_IDS).filter((idKey) => idKey !== "nativeSheet")) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountNativeDesktopSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(NATIVE_DESKTOP_SHEET_ISLAND_IDS[key]);
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

test("native desktop sheet island requires every current event binding target", () => {
  const { documentRef, delete: deleteElement, replace } = fakeDocument();
  const before = resolveNativeDesktopSheetIslandContainers({ documentRef });

  for (const [key, id] of Object.entries(NATIVE_DESKTOP_SHEET_ISLAND_IDS)) {
    assert.equal(before[key].id, id);
  }

  replace(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeOpenButton);
  assert.throws(
    () => assertStableNativeDesktopSheetIslandContainers(
      before,
      resolveNativeDesktopSheetIslandContainers({ documentRef }),
    ),
    /replaced stable container nativeOpenButton/,
  );

  for (const [key, id] of Object.entries(NATIVE_DESKTOP_SHEET_ISLAND_IDS)) {
    const missing = fakeDocument();
    missing.delete(id);
    assert.throws(
      () => resolveNativeDesktopSheetIslandContainers({ documentRef: missing.documentRef }),
      new RegExp(`missing stable container ${key}`),
    );
  }

  deleteElement(NATIVE_DESKTOP_SHEET_ISLAND_IDS.nativeMode);
  assert.throws(
    () => resolveNativeDesktopSheetIslandContainers({ documentRef }),
    /missing stable container nativeMode/,
  );
});
