import test from "node:test";
import assert from "node:assert/strict";

import {
  TERMINAL_SURFACE_ISLAND_IDS,
  TERMINAL_SURFACE_ISLAND_KEYS,
  TERMINAL_SURFACE_ISLAND_PROPS,
  assertStableTerminalSurfaceIslandRefs,
  createTerminalSurfaceIsland,
  createTerminalSurfaceIslandElements,
  resolveTerminalSurfaceIslandRefs,
  terminalSurfaceIslandCleanupPlan,
} from "./terminal_island.js";

function element(id) {
  return { id };
}

function childIds(children) {
  return children.map((child) => child?.props?.id).filter(Boolean);
}

function childKeys(children) {
  return children.map((child) => child?.props?.key).filter(Boolean);
}

function keyedExpected(key, props) {
  return { ...props, key, children: [] };
}

test("terminal island host elements preserve stable DOM ids and classes", () => {
  const children = createTerminalSurfaceIslandElements((type, props, ...nested) => ({
    type,
    props: { ...props, children: nested },
  }));
  const ids = childIds(children);

  assert.deepEqual(ids, [
    TERMINAL_SURFACE_ISLAND_IDS.terminalCanvas,
    TERMINAL_SURFACE_ISLAND_IDS.hudCanvas,
    TERMINAL_SURFACE_ISLAND_IDS.terminalFallback,
    TERMINAL_SURFACE_ISLAND_IDS.terminalA11yMirror,
    TERMINAL_SURFACE_ISLAND_IDS.terminalAnnouncer,
    TERMINAL_SURFACE_ISLAND_IDS.terminalStatusStrip,
    TERMINAL_SURFACE_ISLAND_IDS.terminalLinkTools,
    TERMINAL_SURFACE_ISLAND_IDS.loadingOverlay,
  ]);
  assert.deepEqual(childKeys(children), [
    TERMINAL_SURFACE_ISLAND_KEYS.terminalCanvas,
    TERMINAL_SURFACE_ISLAND_KEYS.hudCanvas,
    TERMINAL_SURFACE_ISLAND_KEYS.terminalFallback,
    TERMINAL_SURFACE_ISLAND_KEYS.terminalA11yMirror,
    TERMINAL_SURFACE_ISLAND_KEYS.terminalAnnouncer,
    TERMINAL_SURFACE_ISLAND_KEYS.terminalStatusStrip,
    TERMINAL_SURFACE_ISLAND_KEYS.terminalLinkTools,
    TERMINAL_SURFACE_ISLAND_KEYS.loadingOverlay,
  ]);
  assert.deepEqual(
    children[0].props,
    keyedExpected(
      TERMINAL_SURFACE_ISLAND_KEYS.terminalCanvas,
      TERMINAL_SURFACE_ISLAND_PROPS.terminalCanvas,
    ),
  );
  assert.deepEqual(
    children[1].props,
    keyedExpected(
      TERMINAL_SURFACE_ISLAND_KEYS.hudCanvas,
      TERMINAL_SURFACE_ISLAND_PROPS.hudCanvas,
    ),
  );
  assert.deepEqual(
    children[2].props,
    keyedExpected(
      TERMINAL_SURFACE_ISLAND_KEYS.terminalFallback,
      TERMINAL_SURFACE_ISLAND_PROPS.terminalFallback,
    ),
  );
  assert.deepEqual(
    children[3].props,
    keyedExpected(
      TERMINAL_SURFACE_ISLAND_KEYS.terminalA11yMirror,
      TERMINAL_SURFACE_ISLAND_PROPS.terminalA11yMirror,
    ),
  );

  const linkTools = children.find((child) => child.props.id === TERMINAL_SURFACE_ISLAND_IDS.terminalLinkTools);
  assert.deepEqual(childIds(linkTools.props.children), [
    TERMINAL_SURFACE_ISLAND_IDS.terminalLinkText,
    TERMINAL_SURFACE_ISLAND_IDS.terminalLinkOpen,
    TERMINAL_SURFACE_ISLAND_IDS.terminalLinkCopy,
  ]);
  assert.equal(linkTools.props.className, "terminal-link-tools hidden");

  assert.throws(() => createTerminalSurfaceIslandElements(null), /createElement function/);
});

test("terminal island creates one adapter from stable refs across rerenders", () => {
  const terminalCanvas = element("terminal-canvas");
  const hudCanvas = element("hud-canvas");
  const adapterCalls = [];
  const adapter = {
    setupHudSurface() {
      adapterCalls.push("setupHudSurface");
    },
    setupTerminalSurface() {
      adapterCalls.push("setupTerminalSurface");
    },
    teardownTerminal() {
      adapterCalls.push("teardownTerminal");
    },
  };
  const factoryCalls = [];
  const island = createTerminalSurfaceIsland({
    refs: {
      terminalCanvas: { current: terminalCanvas },
      hudCanvas: { current: hudCanvas },
    },
    createRuntimeAdapter(runtime) {
      factoryCalls.push(runtime.canvases);
      return adapter;
    },
  });

  assert.deepEqual(resolveTerminalSurfaceIslandRefs({
    terminalCanvas: { current: terminalCanvas },
    hudCanvas: { current: hudCanvas },
  }), { terminalCanvas, hudCanvas });
  assert.deepEqual(factoryCalls, [{ terminalCanvas, hudCanvas }]);
  assert.equal(island.adapter, adapter);
  assert.equal(island.setupHudSurface, adapter.setupHudSurface);

  island.rerender({
    refs: {
      terminalCanvas: { current: terminalCanvas },
      hudCanvas: { current: hudCanvas },
    },
  });

  assert.equal(factoryCalls.length, 1);
  assert.throws(
    () => island.rerender({ refs: { terminalCanvas: element("new-terminal"), hudCanvas } }),
    /replaced stable ref terminalCanvas/,
  );
  assert.throws(
    () => assertStableTerminalSurfaceIslandRefs({ terminalCanvas, hudCanvas }, { terminalCanvas, hudCanvas: null }),
    /stable hudCanvas/,
  );
});

test("terminal island lifecycle delegates setup and cleanup to adapter in order", async () => {
  const calls = [];
  const island = createTerminalSurfaceIsland({
    refs: {
      terminalCanvas: element("terminal-canvas"),
      hudCanvas: element("hud-canvas"),
    },
    createRuntimeAdapter() {
      return {
        async setupHudSurface() {
          calls.push("setupHudSurface");
        },
        async setupTerminalSurface() {
          calls.push("setupTerminalSurface");
        },
        teardownTerminal() {
          calls.push("teardownTerminal");
        },
      };
    },
  });

  await island.setupHudSurface();
  await island.setupTerminalSurface();
  assert.deepEqual(island.cleanup(), { teardownTerminal: true });
  assert.deepEqual(island.cleanup(), { teardownTerminal: false });
  assert.deepEqual(calls, ["setupHudSurface", "setupTerminalSurface", "teardownTerminal"]);
  assert.deepEqual(terminalSurfaceIslandCleanupPlan({ mounted: true, teardown: false }), {
    teardownTerminal: false,
  });
});
