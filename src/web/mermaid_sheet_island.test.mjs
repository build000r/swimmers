import test from "node:test";
import assert from "node:assert/strict";

import {
  MERMAID_SHEET_DEFAULT_COPY,
  MERMAID_SHEET_ISLAND_HOST_PROPS,
  MERMAID_SHEET_ISLAND_IDS,
  MERMAID_SHEET_ISLAND_KEYS,
  MermaidSheet,
  assertStableMermaidSheetIslandContainers,
  createMermaidSheetContents,
  createMermaidSheetElement,
  mountMermaidSheetIsland,
  resolveMermaidSheetIslandContainers,
} from "./mermaid_sheet_island.js";

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
  const elements = new Map(Object.values(MERMAID_SHEET_ISLAND_IDS).map((id) => [id, fakeElement(id)]));
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

function buttonIdsFor(children) {
  return children.map((child) => child?.props?.id).filter(Boolean);
}

test("mermaid sheet island preserves sheet host and child DOM contract", () => {
  const sheet = createMermaidSheetElement(createElement);
  const children = sheet.props.children;
  const header = children[0];
  const summary = children[1];
  const preview = children[2];
  const source = children[3];
  const planTabs = children[4];
  const planContent = children[5];
  const actions = children[6];

  assert.equal(sheet.type, "section");
  assert.deepEqual(sheet.props, { ...MERMAID_SHEET_ISLAND_HOST_PROPS, children });
  assert.deepEqual(keysFor(children), [
    MERMAID_SHEET_ISLAND_KEYS.header,
    MERMAID_SHEET_ISLAND_KEYS.summary,
    MERMAID_SHEET_ISLAND_KEYS.preview,
    MERMAID_SHEET_ISLAND_KEYS.source,
    MERMAID_SHEET_ISLAND_KEYS.planTabs,
    MERMAID_SHEET_ISLAND_KEYS.planContent,
    MERMAID_SHEET_ISLAND_KEYS.actions,
  ]);

  assert.equal(header.props.className, "sheet-header");
  assert.equal(header.props.children[0].props.className, "sheet-eyebrow");
  assert.equal(header.props.children[0].props.children[0], "Artifact");
  assert.equal(header.props.children[1].props.id, MERMAID_SHEET_ISLAND_IDS.mermaidSheetTitle);
  assert.equal(header.props.children[1].props.children[0], "Mermaid Diagram");

  assert.equal(summary.type, "div");
  assert.equal(summary.props.className, "sheet-copy");
  assert.equal(summary.props.id, MERMAID_SHEET_ISLAND_IDS.mermaidSummary);
  assert.equal(summary.props.children[0], MERMAID_SHEET_DEFAULT_COPY.summary);

  assert.deepEqual(preview.props, {
    className: "mermaid-preview",
    id: MERMAID_SHEET_ISLAND_IDS.mermaidPreview,
    key: MERMAID_SHEET_ISLAND_KEYS.preview,
    "aria-live": "polite",
    children: [],
  });
  assert.deepEqual(source.props, {
    className: "sheet-result",
    id: MERMAID_SHEET_ISLAND_IDS.mermaidSource,
    key: MERMAID_SHEET_ISLAND_KEYS.source,
    children: [],
  });
  assert.deepEqual(planTabs.props, {
    className: "plan-tabs hidden",
    id: MERMAID_SHEET_ISLAND_IDS.mermaidPlanTabs,
    key: MERMAID_SHEET_ISLAND_KEYS.planTabs,
    "aria-label": "Plan files",
    children: [],
  });
  assert.deepEqual(planContent.props, {
    className: "sheet-result hidden",
    id: MERMAID_SHEET_ISLAND_IDS.mermaidPlanContent,
    key: MERMAID_SHEET_ISLAND_KEYS.planContent,
    children: [],
  });

  assert.equal(actions.props.className, "sheet-actions");
  assert.deepEqual(buttonIdsFor(actions.props.children), [
    MERMAID_SHEET_ISLAND_IDS.mermaidRefreshButton,
    MERMAID_SHEET_ISLAND_IDS.mermaidOpenButton,
    MERMAID_SHEET_ISLAND_IDS.mermaidCloseButton,
  ]);
  assert.equal(actions.props.children[0].props.className, "ghost-button");
  assert.equal(actions.props.children[0].props.type, "button");
  assert.equal(actions.props.children[0].props.children[0], "Refresh");
  assert.equal(actions.props.children[1].props.className, "ghost-button");
  assert.equal(actions.props.children[1].props.type, "button");
  assert.equal(actions.props.children[1].props.children[0], "Open Host Artifact");
  assert.equal(actions.props.children[2].props.className, "ghost-button");
  assert.equal(actions.props.children[2].props.type, "button");
  assert.equal(actions.props.children[2].props.children[0], "Close");
});

test("mermaid sheet contents can be rendered independently for fallback parity", () => {
  const contents = createMermaidSheetContents(createElement);

  assert.deepEqual(keysFor(contents), [
    MERMAID_SHEET_ISLAND_KEYS.header,
    MERMAID_SHEET_ISLAND_KEYS.summary,
    MERMAID_SHEET_ISLAND_KEYS.preview,
    MERMAID_SHEET_ISLAND_KEYS.source,
    MERMAID_SHEET_ISLAND_KEYS.planTabs,
    MERMAID_SHEET_ISLAND_KEYS.planContent,
    MERMAID_SHEET_ISLAND_KEYS.actions,
  ]);
  assert.equal(contents[0].props.children[1].props.id, MERMAID_SHEET_ISLAND_IDS.mermaidSheetTitle);
  assert.equal(contents[2].props.id, MERMAID_SHEET_ISLAND_IDS.mermaidPreview);
  assert.equal(contents[4].props.id, MERMAID_SHEET_ISLAND_IDS.mermaidPlanTabs);
  assert.throws(() => createMermaidSheetContents(null), /createElement function/);
  assert.throws(() => createMermaidSheetElement(null), /createElement function/);
});

test("mermaid sheet island mounts, rerenders, and guards stable nodes", () => {
  const { documentRef, replace } = fakeDocument();
  const calls = [];
  const handle = mountMermaidSheetIsland({
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

  assert.deepEqual(calls[0], ["hydrate", MERMAID_SHEET_ISLAND_IDS.mermaidSheet, MermaidSheet]);
  assert.deepEqual(resolveMermaidSheetIslandContainers({ documentRef }), before);

  handle.render();

  assert.deepEqual(calls.at(-1), ["render", MermaidSheet]);
  assert.deepEqual(handle.containers, before);

  assert.throws(
    () => assertStableMermaidSheetIslandContainers(before, {
      ...before,
      mermaidSheet: replace(MERMAID_SHEET_ISLAND_IDS.mermaidSheet),
    }),
    /replaced stable container mermaidSheet/,
  );
  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("mermaid sheet island detects every controller target replacement during render", () => {
  for (const key of Object.keys(MERMAID_SHEET_ISLAND_IDS).filter((idKey) => idKey !== "mermaidSheet")) {
    const { documentRef, replace } = fakeDocument();
    const handle = mountMermaidSheetIsland({
      documentRef,
      hydrateRootImpl() {
        return {
          render() {
            replace(MERMAID_SHEET_ISLAND_IDS[key]);
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

test("mermaid sheet island requires every current event binding target", () => {
  const { documentRef, delete: deleteElement, replace } = fakeDocument();
  const before = resolveMermaidSheetIslandContainers({ documentRef });

  for (const [key, id] of Object.entries(MERMAID_SHEET_ISLAND_IDS)) {
    assert.equal(before[key].id, id);
  }

  replace(MERMAID_SHEET_ISLAND_IDS.mermaidPlanTabs);
  assert.throws(
    () => assertStableMermaidSheetIslandContainers(
      before,
      resolveMermaidSheetIslandContainers({ documentRef }),
    ),
    /replaced stable container mermaidPlanTabs/,
  );

  for (const [key, id] of Object.entries(MERMAID_SHEET_ISLAND_IDS)) {
    const missing = fakeDocument();
    missing.delete(id);
    assert.throws(
      () => resolveMermaidSheetIslandContainers({ documentRef: missing.documentRef }),
      new RegExp(`missing stable container ${key}`),
    );
  }

  deleteElement(MERMAID_SHEET_ISLAND_IDS.mermaidPreview);
  assert.throws(
    () => resolveMermaidSheetIslandContainers({ documentRef }),
    /missing stable container mermaidPreview/,
  );
});
