import test from "node:test";
import assert from "node:assert/strict";

import {
  createElement,
  fakeDocumentForIds,
  idsFor,
  keysFor,
} from "./island_test_helpers.mjs";
import {
  COMMAND_PALETTE_ISLAND_HOST_PROPS,
  COMMAND_PALETTE_ISLAND_IDS,
  COMMAND_PALETTE_ISLAND_KEYS,
  COMMAND_PALETTE_RESULTS_PROPS,
  CommandPaletteSheet,
  assertStableCommandPaletteIslandContainers,
  createCommandPaletteResultsElement,
  createCommandPaletteSheetContents,
  createCommandPaletteSheetElement,
  mountCommandPaletteIsland,
  resolveCommandPaletteIslandContainers,
} from "./command_palette_island.js";

function fakeDocument() {
  return fakeDocumentForIds(COMMAND_PALETTE_ISLAND_IDS);
}

test("command palette island preserves sheet host and child DOM contract", () => {
  const sheet = createCommandPaletteSheetElement(createElement, {
    items: [{ label: "Refresh sessions", meta: "sync", actionId: "refresh" }],
    activeIndex: 0,
  });
  const children = sheet.props.children;

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...COMMAND_PALETTE_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    COMMAND_PALETTE_ISLAND_KEYS.header,
    COMMAND_PALETTE_ISLAND_KEYS.field,
    COMMAND_PALETTE_ISLAND_KEYS.results,
    COMMAND_PALETTE_ISLAND_KEYS.actions,
  ]);
  assert.equal(children[0].props.className, "sheet-header");
  assert.equal(children[0].props.children[1].props.id, COMMAND_PALETTE_ISLAND_IDS.paletteSheetTitle);
  assert.equal(children[1].props.className, "field");
  assert.deepEqual(children[1].props.children[1].props, {
    id: COMMAND_PALETTE_ISLAND_IDS.paletteSearch,
    type: "search",
    placeholder: "Search actions and sessions",
    autoComplete: "off",
    role: "combobox",
    "aria-expanded": "true",
    "aria-controls": COMMAND_PALETTE_ISLAND_IDS.paletteResults,
    "aria-autocomplete": "list",
    "aria-activedescendant": "palette-option-0",
    children: [],
  });
  assert.deepEqual(children[2].props, {
    ...COMMAND_PALETTE_RESULTS_PROPS,
    key: COMMAND_PALETTE_ISLAND_KEYS.results,
    children: children[2].props.children,
  });
  assert.equal(children[3].props.children[0].props.id, COMMAND_PALETTE_ISLAND_IDS.paletteCloseButton);
});

test("command palette island result component preserves item classes and data attrs", () => {
  const results = createCommandPaletteResultsElement(createElement, {
    items: [
      { label: "Focus terminal", meta: "terminal", actionId: "focus_terminal" },
      { label: "Send to terminal", meta: "Ctrl+Shift+S", actionId: "open_send", disabled: true },
    ],
    activeIndex: 1,
  });
  const buttons = results.props.children;

  assert.equal(results.type, "div");
  assert.equal(results.props.id, COMMAND_PALETTE_ISLAND_IDS.paletteResults);
  assert.deepEqual(idsFor([results]), [COMMAND_PALETTE_ISLAND_IDS.paletteResults]);
  assert.equal(buttons[0].type, "button");
  assert.equal(buttons[0].props.className, "palette-item");
  assert.equal(buttons[0].props["aria-selected"], "false");
  assert.equal(buttons[0].props.id, "palette-option-0");
  assert.equal(buttons[1].props.id, "palette-option-1");
  assert.equal(buttons[0].props["data-palette-index"], "0");
  assert.equal(buttons[0].props.disabled, undefined);
  assert.equal(buttons[0].props.children[0].props.children[0], "Focus terminal");
  assert.equal(buttons[0].props.children[1].props.children[0], "terminal");
  assert.equal(buttons[1].props.className, "palette-item is-active");
  assert.equal(buttons[1].props["aria-selected"], "true");
  assert.equal(buttons[1].props["data-palette-index"], "1");
  assert.equal(buttons[1].props.disabled, true);
  assert.equal(buttons[1].props.children[1].props.children[0], "unavailable");

  const empty = createCommandPaletteResultsElement(createElement, { items: [] });
  assert.equal(empty.props.children[0].props.className, "sheet-copy");
  assert.equal(empty.props.children[0].props.children[0], "No matching commands.");
  assert.throws(() => createCommandPaletteResultsElement(null), /createElement function/);
});

test("command palette island mounts, rerenders results, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountCommandPaletteIsland({
    documentRef,
    hydrateRootImpl(root, element) {
      calls.push(["hydrate", root.id, element.type, element.props]);
      return {
        render(nextElement) {
          calls.push(["render", nextElement.type, nextElement.props]);
        },
        unmount() {
          calls.push(["unmount"]);
        },
      };
    },
  });
  const before = { ...handle.containers };

  assert.equal(calls[0][0], "hydrate");
  assert.equal(calls[0][1], COMMAND_PALETTE_ISLAND_IDS.paletteSheet);
  assert.equal(calls[0][2], CommandPaletteSheet);
  assert.deepEqual(resolveCommandPaletteIslandContainers({ documentRef }), before);

  assert.equal(handle.renderResults({
    items: [{ label: "Refresh sessions", meta: "sync", actionId: "refresh" }],
    activeIndex: 0,
  }), true);

  assert.equal(calls.at(-1)[0], "render");
  assert.equal(calls.at(-1)[1], CommandPaletteSheet);
  assert.equal(calls.at(-1)[2].items[0].actionId, "refresh");
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableCommandPaletteIslandContainers(before, {
      ...before,
      paletteResults: replace(COMMAND_PALETTE_ISLAND_IDS.paletteResults),
    }),
    /replaced stable container paletteResults/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("command palette island detects search and close replacement during render", () => {
  for (const key of ["paletteSearch", "paletteCloseButton"]) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountCommandPaletteIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(COMMAND_PALETTE_ISLAND_IDS[key]);
          },
          unmount() {},
        };
      },
    });

    assert.throws(
      () => handle.renderResults({ items: [{ label: "Refresh sessions" }] }),
      new RegExp(`replaced stable container ${key}`),
    );
  }
});

test("command palette sheet contents can be rendered independently for fallback parity", () => {
  const contents = createCommandPaletteSheetContents(createElement, {
    items: [{ label: "Auth token", meta: "connection", actionId: "open_auth" }],
  });
  assert.deepEqual(keysFor(contents), Object.values(COMMAND_PALETTE_ISLAND_KEYS));
  assert.equal(contents[2].props.children[0].props.children[0].props.children[0], "Auth token");
});
