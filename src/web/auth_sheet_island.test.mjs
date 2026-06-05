import test from "node:test";
import assert from "node:assert/strict";

import {
  buttonIdsFor,
  createElement,
  fakeDocumentForIds,
  keysFor,
} from "./island_test_helpers.mjs";
import {
  AUTH_SHEET_COPY,
  AUTH_SHEET_ISLAND_HOST_PROPS,
  AUTH_SHEET_ISLAND_IDS,
  AUTH_SHEET_ISLAND_KEYS,
  AUTH_SHEET_TOKEN_INPUT_PROPS,
  AuthSheet,
  assertStableAuthSheetIslandContainers,
  createAuthSheetContents,
  createAuthSheetElement,
  mountAuthSheetIsland,
  resolveAuthSheetIslandContainers,
} from "./auth_sheet_island.js";

function fakeDocument() {
  return fakeDocumentForIds(AUTH_SHEET_ISLAND_IDS);
}

test("auth sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createAuthSheetElement(createElement);
  const children = sheet.props.children;
  const header = children[0];
  const copy = children[1];
  const form = children[2];
  const field = form.props.children[0];
  const actions = form.props.children[1];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...AUTH_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    AUTH_SHEET_ISLAND_KEYS.header,
    AUTH_SHEET_ISLAND_KEYS.copy,
    AUTH_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(header.props.className, "sheet-header");
  assert.equal(header.props.children[0].props.className, "sheet-eyebrow");
  assert.equal(header.props.children[0].props.children[0], "Connection");
  assert.equal(header.props.children[1].props.id, AUTH_SHEET_ISLAND_IDS.authSheetTitle);
  assert.equal(header.props.children[1].props.children[0], "Auth Token");

  assert.equal(copy.type, "div");
  assert.equal(copy.props.className, "sheet-copy");
  assert.equal(copy.props.children[0], AUTH_SHEET_COPY);
  assert.match(copy.props.children[0], /AUTH_TOKEN/);
  assert.match(copy.props.children[0], /OBSERVER_TOKEN/);

  assert.equal(form.type, "div");
  assert.equal(form.props.className, "sheet-form");
  assert.deepEqual(keysFor(form.props.children), [
    AUTH_SHEET_ISLAND_KEYS.field,
    AUTH_SHEET_ISLAND_KEYS.actions,
  ]);
  assert.equal(field.type, "label");
  assert.equal(field.props.className, "field");
  assert.equal(field.props.children[0].props.children[0], "Token");
  assert.deepEqual(field.props.children[1].props, {
    ...AUTH_SHEET_TOKEN_INPUT_PROPS,
    children: [],
  });

  assert.equal(actions.props.className, "sheet-actions");
  assert.deepEqual(buttonIdsFor(actions.props.children), [
    AUTH_SHEET_ISLAND_IDS.clearTokenButton,
    AUTH_SHEET_ISLAND_IDS.authCloseButton,
    AUTH_SHEET_ISLAND_IDS.saveTokenButton,
  ]);
  assert.equal(actions.props.children[0].props.className, "ghost-button");
  assert.equal(actions.props.children[0].props.type, "button");
  assert.equal(actions.props.children[0].props.children[0], "Forget");
  assert.equal(actions.props.children[1].props.className, "ghost-button");
  assert.equal(actions.props.children[1].props.type, "button");
  assert.equal(actions.props.children[1].props.children[0], "Close");
  assert.equal(actions.props.children[2].props.className, undefined);
  assert.equal(actions.props.children[2].props.type, "button");
  assert.equal(actions.props.children[2].props.children[0], "Connect");
});

test("auth sheet contents can be rendered independently for fallback parity", () => {
  const contents = createAuthSheetContents(createElement);
  const form = contents[2];

  assert.deepEqual(keysFor(contents), [
    AUTH_SHEET_ISLAND_KEYS.header,
    AUTH_SHEET_ISLAND_KEYS.copy,
    AUTH_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(contents[0].props.children[1].props.id, AUTH_SHEET_ISLAND_IDS.authSheetTitle);
  assert.equal(form.props.children[0].props.children[1].props.id, AUTH_SHEET_ISLAND_IDS.tokenInput);
  assert.throws(() => createAuthSheetContents(null), /createElement function/);
  assert.throws(() => createAuthSheetElement(null), /createElement function/);
});

test("auth sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountAuthSheetIsland({
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

  assert.deepEqual(calls[0], ["hydrate", AUTH_SHEET_ISLAND_IDS.authSheet, AuthSheet]);
  assert.deepEqual(resolveAuthSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", AuthSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableAuthSheetIslandContainers(before, {
      ...before,
      authSheet: replace(AUTH_SHEET_ISLAND_IDS.authSheet),
    }),
    /replaced stable container authSheet/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("auth sheet island detects every controller target replacement during render", () => {
  for (const key of Object.keys(AUTH_SHEET_ISLAND_IDS).filter((idKey) => idKey !== "authSheet")) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountAuthSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(AUTH_SHEET_ISLAND_IDS[key]);
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

test("auth sheet island requires every current event binding target", () => {
  const { documentRef, replace } = fakeDocument();
  const before = resolveAuthSheetIslandContainers({ documentRef });

  for (const [key, id] of Object.entries(AUTH_SHEET_ISLAND_IDS)) {
    assert.equal(before[key].id, id);
  }

  replace(AUTH_SHEET_ISLAND_IDS.saveTokenButton);
  assert.throws(
    () => assertStableAuthSheetIslandContainers(before, resolveAuthSheetIslandContainers({ documentRef })),
    /replaced stable container saveTokenButton/,
  );
});
