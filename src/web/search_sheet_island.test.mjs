import test from "node:test";
import assert from "node:assert/strict";

import {
  SEARCH_SHEET_INPUT_PROPS,
  SEARCH_SHEET_ISLAND_HOST_PROPS,
  SEARCH_SHEET_ISLAND_IDS,
  SEARCH_SHEET_ISLAND_KEYS,
  SearchSheet,
  assertStableSearchSheetIslandContainers,
  createSearchSheetContents,
  createSearchSheetElement,
  mountSearchSheetIsland,
  resolveSearchSheetIslandContainers,
} from "./search_sheet_island.js";

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
  const elements = new Map(Object.values(SEARCH_SHEET_ISLAND_IDS).map((id) => [id, fakeElement(id)]));
  return {
    documentRef: {
      getElementById(id) {
        return elements.get(id) ?? null;
      },
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

function buttonIdsFor(children) {
  return children.map((child) => child?.props?.id).filter(Boolean);
}

test("search sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createSearchSheetElement(createElement);
  const children = sheet.props.children;
  const form = children[1];
  const label = form.props.children[0];
  const actions = form.props.children[1];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...SEARCH_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    SEARCH_SHEET_ISLAND_KEYS.header,
    SEARCH_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(children[0].props.className, "sheet-header");
  assert.equal(children[0].props.children[0].props.children[0], "Rendered Action");
  assert.equal(children[0].props.children[1].props.id, SEARCH_SHEET_ISLAND_IDS.searchSheetTitle);
  assert.equal(children[0].props.children[1].props.children[0], "Search Terminal");

  assert.equal(form.type, "form");
  assert.equal(form.props.className, "sheet-form");
  assert.equal(form.props.id, SEARCH_SHEET_ISLAND_IDS.searchForm);
  assert.equal(label.type, "label");
  assert.equal(label.props.className, "field");
  assert.equal(label.props.children[0].props.children[0], "Query");
  assert.deepEqual(label.props.children[1].props, {
    ...SEARCH_SHEET_INPUT_PROPS,
    children: [],
  });

  assert.equal(actions.props.className, "sheet-actions");
  assert.deepEqual(buttonIdsFor(actions.props.children), [
    SEARCH_SHEET_ISLAND_IDS.searchPrevButton,
    SEARCH_SHEET_ISLAND_IDS.searchNextButton,
    SEARCH_SHEET_ISLAND_IDS.searchClearButton,
    SEARCH_SHEET_ISLAND_IDS.searchCloseButton,
  ]);
  assert.equal(actions.props.children[0].props.className, "ghost-button");
  assert.equal(actions.props.children[0].props.type, "button");
  assert.equal(actions.props.children[1].props.children[0], "Next");
  assert.equal(actions.props.children[2].props.children[0], "Clear");
  assert.equal(actions.props.children[3].props.type, "submit");
  assert.equal(actions.props.children[3].props.children[0], "Done");
});

test("search sheet contents can be rendered independently for fallback parity", () => {
  const contents = createSearchSheetContents(createElement);
  assert.deepEqual(keysFor(contents), [
    SEARCH_SHEET_ISLAND_KEYS.header,
    SEARCH_SHEET_ISLAND_KEYS.form,
  ]);
  assert.equal(contents[1].props.children[0].props.children[1].props.id, SEARCH_SHEET_ISLAND_IDS.terminalSearch);
  assert.throws(() => createSearchSheetContents(null), /createElement function/);
  assert.throws(() => createSearchSheetElement(null), /createElement function/);
});

test("search sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountSearchSheetIsland({
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

  assert.deepEqual(calls[0], ["hydrate", SEARCH_SHEET_ISLAND_IDS.searchSheet, SearchSheet]);
  assert.deepEqual(resolveSearchSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", SearchSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableSearchSheetIslandContainers(before, {
      ...before,
      terminalSearch: replace(SEARCH_SHEET_ISLAND_IDS.terminalSearch),
    }),
    /replaced stable container terminalSearch/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("search sheet island detects button replacement during render", () => {
  for (const key of ["searchPrevButton", "searchNextButton", "searchClearButton", "searchCloseButton"]) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountSearchSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(SEARCH_SHEET_ISLAND_IDS[key]);
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

test("search sheet island requires every current event binding target", () => {
  const { documentRef, replace } = fakeDocument();
  const before = resolveSearchSheetIslandContainers({ documentRef });

  assert.equal(before.searchSheet.id, SEARCH_SHEET_ISLAND_IDS.searchSheet);
  assert.equal(before.searchForm.id, SEARCH_SHEET_ISLAND_IDS.searchForm);
  assert.equal(before.searchCloseButton.id, SEARCH_SHEET_ISLAND_IDS.searchCloseButton);

  replace(SEARCH_SHEET_ISLAND_IDS.searchForm);
  assert.throws(
    () => assertStableSearchSheetIslandContainers(before, resolveSearchSheetIslandContainers({ documentRef })),
    /replaced stable container searchForm/,
  );
});
