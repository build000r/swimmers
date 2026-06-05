import test from "node:test";
import assert from "node:assert/strict";

import {
  restoreWorkbenchWidgetOpenState,
  workbenchWidgetOpenStateByTitle,
  workbenchWidgetTitleForDetailsNode,
  writeWorkbenchWidgetsHtmlToDom,
  writeWorkbenchWidgetsViewToDom,
} from "./workbench_dom.js";

function detailsNode(title, open) {
  return {
    open,
    querySelector(selector) {
      return selector === ".workbench-widget-title" ? { textContent: title } : null;
    },
  };
}

function containerWithDetails(sequence, calls = []) {
  let callIndex = 0;
  return {
    innerHTML: "initial",
    querySelectorAll(selector) {
      calls.push(["querySelectorAll", selector]);
      const selected = sequence[Math.min(callIndex, sequence.length - 1)] || [];
      callIndex += 1;
      return selected;
    },
  };
}

test("workbenchWidgetTitleForDetailsNode reads widget titles defensively", () => {
  assert.equal(workbenchWidgetTitleForDetailsNode(detailsNode("Logs", true)), "Logs");
  assert.equal(workbenchWidgetTitleForDetailsNode({}), "");
  assert.equal(workbenchWidgetTitleForDetailsNode(null), "");
  assert.equal(workbenchWidgetTitleForDetailsNode({
    querySelector() {
      return { textContent: null };
    },
  }), "");
});

test("workbenchWidgetOpenStateByTitle snapshots non-empty titles only", () => {
  const openByTitle = workbenchWidgetOpenStateByTitle(containerWithDetails([[
    detailsNode("Turns", false),
    detailsNode("", true),
    detailsNode("Logs", true),
  ]]));

  assert.deepEqual([...openByTitle.entries()], [
    ["Turns", false],
    ["Logs", true],
  ]);
  assert.deepEqual([...workbenchWidgetOpenStateByTitle(null).entries()], []);
});

test("restoreWorkbenchWidgetOpenState restores matching details by title only", () => {
  const turns = detailsNode("Turns", true);
  const logs = detailsNode("Logs", false);
  const artifacts = detailsNode("Artifacts", true);

  restoreWorkbenchWidgetOpenState(containerWithDetails([[turns, logs, artifacts]]), new Map([
    ["Turns", false],
    ["Logs", true],
  ]));

  assert.equal(turns.open, false);
  assert.equal(logs.open, true);
  assert.equal(artifacts.open, true);
});

test("writeWorkbenchWidgetsHtmlToDom preserves missing-container and identical-html no-ops", () => {
  const widgets = { lastHtml: "same" };
  const scroller = { scrollTop: 42 };
  const rafCalls = [];

  writeWorkbenchWidgetsHtmlToDom("next", {
    widgets,
    scroller,
    requestAnimationFrame: (callback) => rafCalls.push(callback),
  });
  assert.equal(widgets.lastHtml, "same");
  assert.equal(scroller.scrollTop, 42);
  assert.deepEqual(rafCalls, []);

  const container = containerWithDetails([[detailsNode("Logs", true)]]);
  writeWorkbenchWidgetsHtmlToDom("same", {
    container,
    widgets,
    scroller,
    requestAnimationFrame: (callback) => rafCalls.push(callback),
  });
  assert.equal(container.innerHTML, "initial");
  assert.equal(widgets.lastHtml, "same");
  assert.deepEqual(rafCalls, []);
});

test("writeWorkbenchWidgetsHtmlToDom replaces HTML, restores open state, and schedules scroll restore", () => {
  const previousDetails = [
    detailsNode("Turns", false),
    detailsNode("Logs", true),
  ];
  const nextDetails = [
    detailsNode("Turns", true),
    detailsNode("Logs", false),
    detailsNode("Skills", true),
  ];
  const calls = [];
  const container = containerWithDetails([previousDetails, nextDetails], calls);
  const widgets = { lastHtml: "old" };
  const scroller = { scrollTop: 180 };
  const rafCalls = [];

  writeWorkbenchWidgetsHtmlToDom("new", {
    container,
    widgets,
    scroller,
    requestAnimationFrame: (callback) => rafCalls.push(callback),
  });

  assert.equal(container.innerHTML, "new");
  assert.equal(widgets.lastHtml, "new");
  assert.equal(nextDetails[0].open, false);
  assert.equal(nextDetails[1].open, true);
  assert.equal(nextDetails[2].open, true);
  assert.equal(scroller.scrollTop, 180);
  scroller.scrollTop = 12;
  assert.equal(rafCalls.length, 1);
  rafCalls[0]();
  assert.equal(scroller.scrollTop, 180);
  assert.deepEqual(calls, [
    ["querySelectorAll", "details.workbench-widget"],
    ["querySelectorAll", "details.workbench-widget"],
  ]);
});

test("writeWorkbenchWidgetsHtmlToDom restores scroll synchronously without requestAnimationFrame", () => {
  const widgets = { lastHtml: "old" };
  const scroller = { scrollTop: 55 };
  const container = {
    innerHTML: "old",
    querySelectorAll() {
      scroller.scrollTop = 0;
      return [];
    },
  };

  writeWorkbenchWidgetsHtmlToDom("new", { container, widgets, scroller });

  assert.equal(container.innerHTML, "new");
  assert.equal(widgets.lastHtml, "new");
  assert.equal(scroller.scrollTop, 55);
});

test("writeWorkbenchWidgetsViewToDom delegates React view rendering and preserves details and scroll", () => {
  const previousDetails = [
    detailsNode("Turns", false),
    detailsNode("Logs", true),
  ];
  const nextDetails = [
    detailsNode("Turns", true),
    detailsNode("Logs", false),
    detailsNode("Artifacts", true),
  ];
  const calls = [];
  const container = containerWithDetails([previousDetails, nextDetails], calls);
  const widgets = { lastHtml: "old" };
  const scroller = { scrollTop: 90 };
  const renderViews = [];

  writeWorkbenchWidgetsViewToDom({
    html: "<details class=\"workbench-widget\">new</details>",
    model: { items: [{ title: "Turns" }] },
  }, {
    container,
    widgets,
    scroller,
    renderWorkbenchWidgetsView(view) {
      renderViews.push(view);
      return true;
    },
  });

  assert.deepEqual(renderViews, [{
    html: "<details class=\"workbench-widget\">new</details>",
    model: { items: [{ title: "Turns" }] },
  }]);
  assert.equal(container.innerHTML, "initial", "React path does not use legacy innerHTML");
  assert.equal(widgets.lastHtml, "<details class=\"workbench-widget\">new</details>");
  assert.equal(nextDetails[0].open, false);
  assert.equal(nextDetails[1].open, true);
  assert.equal(nextDetails[2].open, true);
  assert.equal(scroller.scrollTop, 90);
  assert.deepEqual(calls, [
    ["querySelectorAll", "details.workbench-widget"],
    ["querySelectorAll", "details.workbench-widget"],
  ]);
});

test("writeWorkbenchWidgetsViewToDom falls back to legacy HTML when React view declines", () => {
  const widgets = { lastHtml: "old" };
  const container = containerWithDetails([[detailsNode("Logs", true)]]);
  const scroller = { scrollTop: 12 };

  writeWorkbenchWidgetsViewToDom({
    html: "<details class=\"workbench-widget\">fallback</details>",
    model: { items: [] },
  }, {
    container,
    widgets,
    scroller,
    renderWorkbenchWidgetsView() {
      return false;
    },
  });

  assert.equal(container.innerHTML, "<details class=\"workbench-widget\">fallback</details>");
  assert.equal(widgets.lastHtml, "<details class=\"workbench-widget\">fallback</details>");
  assert.equal(scroller.scrollTop, 12);
});
