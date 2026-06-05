import test from "node:test";
import assert from "node:assert/strict";

import {
  createElement,
  fakeDocumentForIds,
  idsFor,
  keysFor,
} from "./island_test_helpers.mjs";
import {
  CREATE_SHEET_CWD_INPUT_PROPS,
  CREATE_SHEET_DEFAULT_COPY,
  CREATE_SHEET_DIR_LIST_PROPS,
  CREATE_SHEET_ISLAND_HOST_PROPS,
  CREATE_SHEET_ISLAND_IDS,
  CREATE_SHEET_ISLAND_KEYS,
  CREATE_SHEET_PATH_INPUT_PROPS,
  CREATE_SHEET_REQUEST_PROPS,
  CREATE_SHEET_SEARCH_INPUT_PROPS,
  CREATE_SHEET_TOOL_OPTIONS,
  CreateSheet,
  assertStableCreateSheetIslandContainers,
  createCreateSheetContents,
  createCreateSheetElement,
  mountCreateSheetIsland,
  resolveCreateSheetIslandContainers,
} from "./create_sheet_island.js";

function fakeDocument() {
  return fakeDocumentForIds(CREATE_SHEET_ISLAND_IDS);
}

function textFor(node) {
  return node?.props?.children?.[0] ?? "";
}

test("create sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createCreateSheetElement(createElement);
  const children = sheet.props.children;
  const header = children[0];
  const toolbar = children[1];
  const groups = children[2];
  const pathbar = children[3];
  const table = children[4];
  const form = children[5];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...CREATE_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), Object.values(CREATE_SHEET_ISLAND_KEYS));

  assert.equal(header.type, "header");
  assert.equal(header.props.className, "console-head");
  assert.equal(header.props.children[0].props.className, "console-heading");
  assert.equal(header.props.children[0].props.children[0].props.children[0], "Repository atlas");
  assert.equal(header.props.children[0].props.children[1].props.id, CREATE_SHEET_ISLAND_IDS.createSheetTitle);
  assert.equal(header.props.children[0].props.children[1].props.children[0], "Create session");
  assert.equal(header.props.children[1].props.className, "console-dismiss");
  assert.equal(header.props.children[1].props.id, CREATE_SHEET_ISLAND_IDS.createCloseButton);
  assert.equal(header.props.children[1].props["aria-label"], "Close");
  assert.equal(header.props.children[1].props.children[0], "esc");

  assert.equal(toolbar.props.className, "console-toolbar");
  assert.equal(toolbar.props.children[0].props.className, "console-search");
  assert.equal(toolbar.props.children[0].props.children[0].type, "svg");
  assert.deepEqual(toolbar.props.children[0].props.children[1].props, {
    ...CREATE_SHEET_SEARCH_INPUT_PROPS,
    children: [],
  });
  assert.equal(toolbar.props.children[1].props.className, "console-toggle");
  assert.deepEqual(toolbar.props.children[1].props.children[0].props, {
    id: CREATE_SHEET_ISLAND_IDS.dirsManagedOnly,
    type: "checkbox",
    children: [],
  });
  assert.equal(toolbar.props.children[1].props.children[1].props.children[0], "Managed only");
  assert.equal(toolbar.props.children[2].props.id, CREATE_SHEET_ISLAND_IDS.createBatchVisible);
  assert.equal(toolbar.props.children[2].props.children[0], "Select all");

  assert.deepEqual(groups.props, {
    className: "console-chips",
    id: CREATE_SHEET_ISLAND_IDS.dirsGroups,
    key: CREATE_SHEET_ISLAND_KEYS.groups,
    role: "group",
    "aria-label": "Repository groups",
    children: [],
  });

  assert.equal(pathbar.props.className, "console-pathbar");
  assert.equal(pathbar.props.children[0].props.children[0], "Browsing");
  assert.deepEqual(pathbar.props.children[1].props, {
    ...CREATE_SHEET_PATH_INPUT_PROPS,
    children: [],
  });
  assert.deepEqual(idsFor(pathbar.props.children.slice(2)), [
    CREATE_SHEET_ISLAND_IDS.dirsUpButton,
    CREATE_SHEET_ISLAND_IDS.dirsLoadButton,
    CREATE_SHEET_ISLAND_IDS.dirsSpawnHere,
  ]);
  assert.equal(pathbar.props.children[4].props.className, "console-ghost console-ghost-accent");

  assert.equal(table.props.className, "console-table");
  assert.equal(table.props.role, "table");
  assert.equal(table.props["aria-label"], "Repositories");
  assert.equal(table.props.children[0].props.className, "console-row console-row-head");
  assert.deepEqual(table.props.children[0].props.children.map(textFor), ["", "Repository", "Path", "Status", "Groups"]);
  assert.deepEqual(table.props.children[1].props, {
    ...CREATE_SHEET_DIR_LIST_PROPS,
    children: [],
  });

  assert.equal(form.type, "form");
  assert.equal(form.props.className, "console-dock");
  assert.equal(form.props.id, CREATE_SHEET_ISLAND_IDS.createForm);
});

test("create sheet island preserves dock form defaults and batch contract", () => {
  const form = createCreateSheetElement(createElement).props.children[5];
  const grid = form.props.children[0];
  const requestField = form.props.children[1];
  const foot = form.props.children[2];
  const cwdField = grid.props.children[0];
  const toolField = grid.props.children[1];
  const launchField = grid.props.children[2];
  const toolSelect = toolField.props.children[1].props.children[0];
  const launchSelect = launchField.props.children[1].props.children[0];
  const batchBar = foot.props.children[1];

  assert.equal(grid.props.className, "console-dock-grid");
  assert.equal(cwdField.props.className, "dock-field dock-field-wide");
  assert.equal(cwdField.props.children[0].props.children[0], "Working directory");
  assert.deepEqual(cwdField.props.children[1].props, {
    ...CREATE_SHEET_CWD_INPUT_PROPS,
    children: [],
  });

  assert.equal(toolField.props.children[0].props.children[0], "Tool");
  assert.equal(toolSelect.type, "select");
  assert.equal(toolSelect.props.id, CREATE_SHEET_ISLAND_IDS.createTool);
  assert.deepEqual(toolSelect.props.children.map((option) => ({
    value: option.props.value,
    label: option.props.children[0],
  })), CREATE_SHEET_TOOL_OPTIONS);
  assert.equal(launchField.props.children[0].props.children[0], "Launch target");
  assert.equal(launchSelect.type, "select");
  assert.equal(launchSelect.props.id, CREATE_SHEET_ISLAND_IDS.createLaunchTarget);
  assert.deepEqual(launchSelect.props.children, []);

  assert.equal(requestField.props.className, "dock-field dock-field-prompt");
  assert.equal(requestField.props.children[0].props.children[0], "Boot prompt ");
  assert.equal(requestField.props.children[0].props.children[1].type, "em");
  assert.equal(requestField.props.children[0].props.children[1].props.children[0], "optional");
  assert.deepEqual(requestField.props.children[1].props, {
    ...CREATE_SHEET_REQUEST_PROPS,
    children: [],
  });

  assert.equal(foot.props.className, "console-dock-foot");
  assert.equal(foot.props.children[0].props.id, CREATE_SHEET_ISLAND_IDS.dirsSummary);
  assert.equal(foot.props.children[0].props.children[0], CREATE_SHEET_DEFAULT_COPY.dirsSummary);
  assert.equal(batchBar.props.className, "console-batch hidden");
  assert.equal(batchBar.props.id, CREATE_SHEET_ISLAND_IDS.createBatchBar);
  assert.equal(batchBar.props["aria-live"], "polite");
  assert.deepEqual(batchBar.props.children[0].props.children.map((child) => child.props.id), [
    CREATE_SHEET_ISLAND_IDS.createBatchCount,
    CREATE_SHEET_ISLAND_IDS.createBatchTool,
    CREATE_SHEET_ISLAND_IDS.createBatchPreview,
  ]);
  assert.equal(batchBar.props.children[0].props.children[0].props.children[0], CREATE_SHEET_DEFAULT_COPY.createBatchCount);
  assert.equal(batchBar.props.children[0].props.children[1].props.children[0], CREATE_SHEET_DEFAULT_COPY.createBatchTool);
  assert.equal(batchBar.props.children[0].props.children[2].props.children[0], CREATE_SHEET_DEFAULT_COPY.createBatchPreview);
  assert.equal(batchBar.props.children[1].props.id, CREATE_SHEET_ISLAND_IDS.createBatchClear);
  assert.equal(batchBar.props.children[1].props.type, "button");
  assert.equal(batchBar.props.children[1].props.children[0], "Clear");
  assert.equal(batchBar.props.children[2].props.id, CREATE_SHEET_ISLAND_IDS.createBatchSubmit);
  assert.equal(batchBar.props.children[2].props.type, "submit");
  assert.equal(batchBar.props.children[2].props.form, CREATE_SHEET_ISLAND_IDS.createForm);
  assert.equal(batchBar.props.children[2].props.children[0], "Batch send");
  assert.equal(foot.props.children[2].props.id, CREATE_SHEET_ISLAND_IDS.createButton);
  assert.equal(foot.props.children[2].props.type, "submit");
  assert.equal(foot.props.children[2].props.children[0], "Create session");
});

test("create sheet contents can be rendered independently for fallback parity", () => {
  const contents = createCreateSheetContents(createElement);
  const form = contents[5];

  assert.deepEqual(keysFor(contents), Object.values(CREATE_SHEET_ISLAND_KEYS));
  assert.equal(contents[0].props.children[0].props.children[1].props.id, CREATE_SHEET_ISLAND_IDS.createSheetTitle);
  assert.equal(form.props.children[0].props.children[0].props.children[1].props.id, CREATE_SHEET_ISLAND_IDS.createCwd);
  assert.equal(form.props.children[2].props.children[1].props.id, CREATE_SHEET_ISLAND_IDS.createBatchBar);
  assert.throws(() => createCreateSheetContents(null), /createElement function/);
  assert.throws(() => createCreateSheetElement(null), /createElement function/);
});

test("create sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountCreateSheetIsland({
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

  assert.deepEqual(calls[0], ["hydrate", CREATE_SHEET_ISLAND_IDS.createSheet, CreateSheet]);
  assert.deepEqual(resolveCreateSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", CreateSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableCreateSheetIslandContainers(before, {
      ...before,
      createSheet: replace(CREATE_SHEET_ISLAND_IDS.createSheet),
    }),
    /replaced stable container createSheet/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("create sheet island detects every controller target replacement during render", () => {
  for (const key of Object.keys(CREATE_SHEET_ISLAND_IDS).filter((idKey) => idKey !== "createSheet")) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountCreateSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(CREATE_SHEET_ISLAND_IDS[key]);
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

test("create sheet island requires every current event binding target", () => {
  const { documentRef, replace } = fakeDocument();
  const before = resolveCreateSheetIslandContainers({ documentRef });

  for (const [key, id] of Object.entries(CREATE_SHEET_ISLAND_IDS)) {
    assert.equal(before[key].id, id);
  }

  replace(CREATE_SHEET_ISLAND_IDS.dirsList);
  assert.throws(
    () => assertStableCreateSheetIslandContainers(before, resolveCreateSheetIslandContainers({ documentRef })),
    /replaced stable container dirsList/,
  );
});
