export function workbenchWidgetTitleForDetailsNode(node) {
  const titleEl =
    typeof node?.querySelector === "function"
      ? node.querySelector(".workbench-widget-title")
      : null;
  return titleEl ? titleEl.textContent ?? "" : "";
}

export function workbenchWidgetOpenStateByTitle(container) {
  const openByTitle = new Map();
  if (typeof container?.querySelectorAll !== "function") {
    return openByTitle;
  }
  for (const node of container.querySelectorAll("details.workbench-widget")) {
    const key = workbenchWidgetTitleForDetailsNode(node);
    if (key) {
      openByTitle.set(key, Boolean(node.open));
    }
  }
  return openByTitle;
}

export function restoreWorkbenchWidgetOpenState(container, openByTitle) {
  if (!openByTitle?.size || typeof container?.querySelectorAll !== "function") {
    return;
  }
  for (const node of container.querySelectorAll("details.workbench-widget")) {
    const key = workbenchWidgetTitleForDetailsNode(node);
    if (openByTitle.has(key)) {
      node.open = openByTitle.get(key);
    }
  }
}

export function writeWorkbenchWidgetsHtmlToDom(nextHtml, runtime = {}) {
  const container = runtime.container;
  if (!container) {
    return;
  }
  if (runtime.widgets.lastHtml === nextHtml) {
    return;
  }
  const scroller = runtime.scroller;
  const prevScrollTop =
    scroller && typeof scroller.scrollTop === "number" ? scroller.scrollTop : 0;
  const openByTitle = workbenchWidgetOpenStateByTitle(container);

  container.innerHTML = nextHtml;
  runtime.widgets.lastHtml = nextHtml;
  restoreWorkbenchWidgetOpenState(container, openByTitle);

  if (scroller && typeof runtime.requestAnimationFrame === "function") {
    runtime.requestAnimationFrame(() => {
      scroller.scrollTop = prevScrollTop;
    });
  } else if (scroller) {
    scroller.scrollTop = prevScrollTop;
  }
}
