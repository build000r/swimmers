import { createFrankenTermRuntimeAdapter } from "./terminal_surface_controller.js";
import { assertStableIdentity, elementFromRef } from "./react_island_identity.js";

export const TERMINAL_SURFACE_ISLAND_IDS = Object.freeze({
  terminalCanvas: "terminal-canvas",
  hudCanvas: "hud-canvas",
  terminalFallback: "terminal-fallback",
  terminalA11yMirror: "terminal-a11y-mirror",
  terminalAnnouncer: "terminal-announcer",
  terminalStatusStrip: "terminal-status-strip",
  terminalLinkTools: "terminal-link-tools",
  terminalLinkText: "terminal-link-text",
  terminalLinkOpen: "terminal-link-open",
  terminalLinkCopy: "terminal-link-copy",
  loadingOverlay: "loading-overlay",
  loadingLabel: "loading-label",
});

export const TERMINAL_SURFACE_ISLAND_PROPS = Object.freeze({
  terminalCanvas: Object.freeze({
    className: "terminal-canvas hidden",
    id: TERMINAL_SURFACE_ISLAND_IDS.terminalCanvas,
  }),
  hudCanvas: Object.freeze({
    className: "hud-canvas hidden",
    id: TERMINAL_SURFACE_ISLAND_IDS.hudCanvas,
    "aria-hidden": "true",
  }),
  terminalFallback: Object.freeze({
    className: "terminal-fallback hidden",
    id: TERMINAL_SURFACE_ISLAND_IDS.terminalFallback,
    tabIndex: 0,
    "aria-label": "Live terminal text fallback",
  }),
  terminalA11yMirror: Object.freeze({
    className: "terminal-a11y-mirror",
    id: TERMINAL_SURFACE_ISLAND_IDS.terminalA11yMirror,
    "aria-label": "Live terminal text mirror",
    readOnly: true,
    tabIndex: -1,
  }),
  terminalAnnouncer: Object.freeze({
    className: "terminal-announcer",
    id: TERMINAL_SURFACE_ISLAND_IDS.terminalAnnouncer,
    "aria-live": "polite",
    "aria-atomic": "false",
  }),
  terminalStatusStrip: Object.freeze({
    className: "terminal-status-strip",
    id: TERMINAL_SURFACE_ISLAND_IDS.terminalStatusStrip,
    "aria-live": "polite",
  }),
  terminalLinkTools: Object.freeze({
    className: "terminal-link-tools hidden",
    id: TERMINAL_SURFACE_ISLAND_IDS.terminalLinkTools,
    role: "group",
    "aria-label": "Terminal link actions",
  }),
  loadingOverlay: Object.freeze({
    className: "loading-overlay visible",
    id: TERMINAL_SURFACE_ISLAND_IDS.loadingOverlay,
    "aria-hidden": "true",
  }),
});

export const TERMINAL_SURFACE_ISLAND_KEYS = Object.freeze({
  terminalCanvas: TERMINAL_SURFACE_ISLAND_IDS.terminalCanvas,
  hudCanvas: TERMINAL_SURFACE_ISLAND_IDS.hudCanvas,
  terminalFallback: TERMINAL_SURFACE_ISLAND_IDS.terminalFallback,
  terminalA11yMirror: TERMINAL_SURFACE_ISLAND_IDS.terminalA11yMirror,
  terminalAnnouncer: TERMINAL_SURFACE_ISLAND_IDS.terminalAnnouncer,
  terminalStatusStrip: TERMINAL_SURFACE_ISLAND_IDS.terminalStatusStrip,
  terminalLinkTools: TERMINAL_SURFACE_ISLAND_IDS.terminalLinkTools,
  loadingOverlay: TERMINAL_SURFACE_ISLAND_IDS.loadingOverlay,
});

function keyedProps(key, props) {
  return { ...props, key };
}

export function resolveTerminalSurfaceIslandRefs(refs = {}) {
  const terminalCanvas = elementFromRef(refs.terminalCanvas);
  const hudCanvas = elementFromRef(refs.hudCanvas);
  if (!terminalCanvas) {
    throw new Error("Terminal surface island requires a stable terminalCanvas element or ref");
  }
  if (!hudCanvas) {
    throw new Error("Terminal surface island requires a stable hudCanvas element or ref");
  }
  return { terminalCanvas, hudCanvas };
}

export function assertStableTerminalSurfaceIslandRefs(previous, next) {
  const resolvedNext = resolveTerminalSurfaceIslandRefs(next);
  return assertStableIdentity(previous, resolvedNext, {
    label: "Terminal surface island",
    noun: "ref",
  });
}

export function terminalSurfaceIslandCleanupPlan({
  mounted = true,
  teardown = true,
  hasAdapter = true,
} = {}) {
  return {
    teardownTerminal: Boolean(mounted && teardown && hasAdapter),
  };
}

export function createTerminalSurfaceIslandElements(createElement) {
  if (typeof createElement !== "function") {
    throw new TypeError("Terminal surface island requires a createElement function");
  }
  const h = createElement;
  return [
    h(
      "canvas",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.terminalCanvas, TERMINAL_SURFACE_ISLAND_PROPS.terminalCanvas),
    ),
    h(
      "canvas",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.hudCanvas, TERMINAL_SURFACE_ISLAND_PROPS.hudCanvas),
    ),
    h(
      "pre",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.terminalFallback, TERMINAL_SURFACE_ISLAND_PROPS.terminalFallback),
    ),
    h(
      "textarea",
      keyedProps(
        TERMINAL_SURFACE_ISLAND_KEYS.terminalA11yMirror,
        TERMINAL_SURFACE_ISLAND_PROPS.terminalA11yMirror,
      ),
    ),
    h(
      "div",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.terminalAnnouncer, TERMINAL_SURFACE_ISLAND_PROPS.terminalAnnouncer),
    ),
    h(
      "div",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.terminalStatusStrip, TERMINAL_SURFACE_ISLAND_PROPS.terminalStatusStrip),
    ),
    h(
      "div",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.terminalLinkTools, TERMINAL_SURFACE_ISLAND_PROPS.terminalLinkTools),
      h("span", { id: TERMINAL_SURFACE_ISLAND_IDS.terminalLinkText }),
      h("button", { id: TERMINAL_SURFACE_ISLAND_IDS.terminalLinkOpen, type: "button" }, "Open"),
      h("button", { id: TERMINAL_SURFACE_ISLAND_IDS.terminalLinkCopy, type: "button" }, "Copy"),
    ),
    h(
      "div",
      keyedProps(TERMINAL_SURFACE_ISLAND_KEYS.loadingOverlay, TERMINAL_SURFACE_ISLAND_PROPS.loadingOverlay),
      h(
        "div",
        { className: "loading-label", id: TERMINAL_SURFACE_ISLAND_IDS.loadingLabel },
        "Loading FrankenTerm…",
      ),
      h("div", { className: "loading-bar" }, h("div", { className: "loading-bar-fill" })),
    ),
  ];
}

// React owns the stable host nodes now. The existing app/session socket still
// drives adapter effects so WebSocket, fallback, and snapshot ordering stay
// unchanged until a later bead can move those effects safely.
export function createTerminalSurfaceIsland(runtime = {}) {
  const {
    createRuntimeAdapter = createFrankenTermRuntimeAdapter,
    refs,
    canvases,
    ...adapterRuntime
  } = runtime;
  const stableRefs = resolveTerminalSurfaceIslandRefs(
    refs ?? canvases ?? {
      terminalCanvas: adapterRuntime.el?.terminalCanvas,
      hudCanvas: adapterRuntime.el?.hudCanvas,
    },
  );
  const adapter = createRuntimeAdapter({
    ...adapterRuntime,
    canvases: stableRefs,
  });
  let mounted = true;
  const island = {
    ...adapter,
    adapter,
    refs: stableRefs,
    assertStableRefs(nextRefs) {
      return assertStableTerminalSurfaceIslandRefs(stableRefs, nextRefs);
    },
    rerender(nextRuntime = {}) {
      island.assertStableRefs(nextRuntime.refs ?? nextRuntime.canvases ?? stableRefs);
      return island;
    },
    cleanup(options = {}) {
      const plan = terminalSurfaceIslandCleanupPlan({
        mounted,
        hasAdapter: Boolean(adapter),
        teardown: options.teardown !== false,
      });
      mounted = false;
      if (plan.teardownTerminal) {
        adapter.teardownTerminal?.();
      }
      return plan;
    },
  };
  return island;
}
