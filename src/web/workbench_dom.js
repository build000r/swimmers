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

function workbenchActiveFieldSelector(element) {
  if (!element || typeof element.getAttributeNames !== "function") {
    return "";
  }
  const tag = String(element.tagName || "").toUpperCase();
  if (tag !== "INPUT" && tag !== "TEXTAREA") {
    return "";
  }
  for (const name of element.getAttributeNames()) {
    if (typeof name === "string" && name.startsWith("data-workbench")) {
      return `[${name}]`;
    }
  }
  return "";
}

// Capture the focused workbench text field (and its caret) before the widget
// subtree is rewritten, so it can be restored afterward. Without this, every
// keystroke in the log-search input destroyed and recreated the input, losing
// focus and caret — making multi-character search effectively impossible.
function captureWorkbenchActiveField(container) {
  const doc = container && typeof container === "object" ? container.ownerDocument : null;
  const active = doc && typeof doc === "object" ? doc.activeElement : null;
  if (!active || typeof container.contains !== "function" || !container.contains(active)) {
    return null;
  }
  const selector = workbenchActiveFieldSelector(active);
  if (!selector) {
    return null;
  }
  const start = typeof active.selectionStart === "number" ? active.selectionStart : null;
  const end = typeof active.selectionEnd === "number" ? active.selectionEnd : null;
  return { selector, start, end };
}

function restoreWorkbenchActiveField(container, captured) {
  if (!captured || typeof container?.querySelector !== "function") {
    return;
  }
  const field = container.querySelector(captured.selector);
  if (!field || typeof field.focus !== "function") {
    return;
  }
  field.focus();
  if (captured.start !== null && typeof field.setSelectionRange === "function") {
    const end = captured.end !== null ? captured.end : captured.start;
    try {
      field.setSelectionRange(captured.start, end);
    } catch (_error) {
      // Some input types reject setSelectionRange; focus alone is enough.
    }
  }
}

export function writeWorkbenchWidgetsHtmlToDom(nextHtml, runtime = {}) {
  return writeWorkbenchWidgetsViewToDom({
    html: String(nextHtml || ""),
    model: null,
  }, runtime);
}

export function writeWorkbenchWidgetsViewToDom(view, runtime = {}) {
  const container = runtime.container;
  if (!container) {
    return;
  }
  const nextHtml = String(view?.html || "");
  if (runtime.widgets.lastHtml === nextHtml) {
    return;
  }
  const scroller = runtime.scroller;
  const prevScrollTop =
    scroller && typeof scroller.scrollTop === "number" ? scroller.scrollTop : 0;
  const openByTitle = workbenchWidgetOpenStateByTitle(container);
  const activeField = captureWorkbenchActiveField(container);

  const renderedByIsland = typeof runtime.renderWorkbenchWidgetsView === "function" &&
    runtime.renderWorkbenchWidgetsView(view) === true;
  if (!renderedByIsland) {
    container.innerHTML = nextHtml;
  }
  runtime.widgets.lastHtml = nextHtml;
  restoreWorkbenchWidgetOpenState(container, openByTitle);
  restoreWorkbenchActiveField(container, activeField);

  if (scroller && typeof runtime.requestAnimationFrame === "function") {
    runtime.requestAnimationFrame(() => {
      scroller.scrollTop = prevScrollTop;
    });
  } else if (scroller) {
    scroller.scrollTop = prevScrollTop;
  }
}
