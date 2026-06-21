import test from "node:test";
import assert from "node:assert/strict";

import { fakeDocumentForIds } from "./island_test_helpers.mjs";
import {
  assertStableIdentity,
  reportIdentityDrift,
  setIdentityDriftReporter,
} from "./react_island_identity.js";
import {
  SWIMMERS_REACT_ROOT_ID,
  SWIMMERS_STABLE_CONTAINER_IDS,
  SwimmersRootShell,
  TerminalSurface,
  mountSwimmersRootShell,
  resolveStableShellContainers,
} from "./react_shell.js";

function fakeDocument() {
  return fakeDocumentForIds({
    root: SWIMMERS_REACT_ROOT_ID,
    ...SWIMMERS_STABLE_CONTAINER_IDS,
  });
}

test("React shell mount normalizes boot payload and preserves unmount semantics", () => {
  const { documentRef } = fakeDocument();
  const calls = [];
  const handle = mountSwimmersRootShell({
    documentRef,
    root: documentRef.getElementById(SWIMMERS_REACT_ROOT_ID),
    boot: {
      franken_term_available: true,
      franken_term_js_url: "/assets/frankenterm/FrankenTerm.js",
      focus_layout: true,
      franken_term_asset_info: { js: { route: "/bad" } },
    },
    hydrateRootImpl(root, element) {
      calls.push(["hydrate", root.id, element.type, element.props.boot]);
      return {
        render(nextElement) {
          calls.push(["render", nextElement.type, nextElement.props.boot]);
        },
        unmount() {
          calls.push(["unmount"]);
        },
      };
    },
  });

  assert.equal(calls[0][0], "hydrate");
  assert.equal(calls[0][1], SWIMMERS_REACT_ROOT_ID);
  assert.equal(calls[0][2], SwimmersRootShell);
  assert.equal(handle.boot.franken_term_available, true);
  assert.equal(handle.boot.franken_term_js_url, "/assets/frankenterm/FrankenTerm.js");
  assert.equal(handle.boot.franken_term_asset_info, null);
  assert.equal(handle.boot.focus_layout, true);

  handle.render({ franken_term_available: false, focus_layout: false });
  assert.equal(handle.boot.franken_term_available, false);
  assert.equal(handle.boot.focus_layout, false);
  assert.equal(calls[1][0], "render");
  assert.equal(calls[1][1], SwimmersRootShell);

  handle.unmount();
  assert.deepEqual(calls.at(-1), ["unmount"]);
});

test("React shell keeps terminal and Trogdor container identity across observable rerenders", () => {
  const { documentRef } = fakeDocument();
  const handle = mountSwimmersRootShell({
    documentRef,
    root: documentRef.getElementById(SWIMMERS_REACT_ROOT_ID),
    hydrateRootImpl() {
      return {
        render() {},
        unmount() {},
      };
    },
  });
  const before = { ...handle.containers };

  handle.render({ franken_term_available: true, focus_layout: true });

  for (const key of Object.keys(SWIMMERS_STABLE_CONTAINER_IDS)) {
    assert.equal(handle.containers[key], before[key], `${key} identity changed`);
  }
  assert.deepEqual(resolveStableShellContainers(documentRef), before);
});

test("React shell renders the speed-reader aria-live region so SSR hydration keeps it", () => {
  const shell = SwimmersRootShell({ boot: {} });
  const children = [].concat(shell.props.children);
  const announce = children.find((child) => child?.props?.id === "trogdor-reader-announce");
  assert.ok(announce, "shell must render #trogdor-reader-announce to match the SSR shell");
  assert.equal(announce.props["aria-live"], "polite");
  assert.equal(announce.props["aria-atomic"], "true");
});

test("React shell identity drift reports instead of crashing the live surface", () => {
  const { documentRef, replace } = fakeDocument();
  const originalError = console.error;
  const errors = [];
  const drifts = [];
  console.error = (...args) => errors.push(args);
  setIdentityDriftReporter((detail) => drifts.push(detail));
  try {
    const handle = mountSwimmersRootShell({
      documentRef,
      root: documentRef.getElementById(SWIMMERS_REACT_ROOT_ID),
      hydrateRootImpl() {
        return {
          render() {
            replace(SWIMMERS_STABLE_CONTAINER_IDS.terminalCanvas);
          },
        };
      },
    });

    // Downgraded contract: drift must NOT throw (that would take the terminal
    // surface down), but must console.error + telemetry and keep the handle
    // pointed at the latest live containers.
    assert.doesNotThrow(() => handle.render({ franken_term_available: true }));
    assert.ok(errors.some((args) => /replaced stable container terminalCanvas/.test(String(args[0]))));
    assert.ok(drifts.some((detail) => detail.key === "terminalCanvas"));
    assert.equal(handle.containers.terminalCanvas.replaced, true);
  } finally {
    console.error = originalError;
    setIdentityDriftReporter(null);
  }
});

test("React shell identity drift reports fallback and mirror replacement", () => {
  for (const key of ["terminalFallback", "terminalA11yMirror"]) {
    const { documentRef, replace } = fakeDocument();
    const originalError = console.error;
    const errors = [];
    console.error = (...args) => errors.push(args);
    try {
      const handle = mountSwimmersRootShell({
        documentRef,
        root: documentRef.getElementById(SWIMMERS_REACT_ROOT_ID),
        hydrateRootImpl() {
          return {
            render() {
              replace(SWIMMERS_STABLE_CONTAINER_IDS[key]);
            },
          };
        },
      });

      assert.doesNotThrow(() => handle.render({ franken_term_available: true }));
      assert.ok(errors.some((args) => new RegExp(`replaced stable container ${key}`).test(String(args[0]))));
    } finally {
      console.error = originalError;
    }
  }
});

test("React shell element declares the stable terminal and Trogdor island host", () => {
  const element = SwimmersRootShell({
    boot: { franken_term_available: true, focus_layout: true },
  });
  const childIds = element.props.children
    .map((child) => child?.props?.id)
    .filter(Boolean);
  const terminalSurface = element.props.children.find((child) => child?.type === TerminalSurface);
  const terminalSurfaceIds = TerminalSurface()
    .map((child) => child?.props?.id)
    .filter(Boolean);

  assert.equal(element.type, "main");
  assert.equal(element.props.id, SWIMMERS_STABLE_CONTAINER_IDS.terminalStage);
  assert.equal(element.props["data-franken-term-available"], "true");
  assert.equal(element.props["data-focus-layout"], "true");
  assert.equal(terminalSurface.type, TerminalSurface);
  assert.ok(terminalSurfaceIds.includes(SWIMMERS_STABLE_CONTAINER_IDS.terminalCanvas));
  assert.ok(terminalSurfaceIds.includes(SWIMMERS_STABLE_CONTAINER_IDS.hudCanvas));
  assert.ok(terminalSurfaceIds.includes(SWIMMERS_STABLE_CONTAINER_IDS.terminalFallback));
  assert.ok(terminalSurfaceIds.includes(SWIMMERS_STABLE_CONTAINER_IDS.terminalA11yMirror));
  assert.ok(childIds.includes(SWIMMERS_STABLE_CONTAINER_IDS.trogdorSurface));
});

test("assertStableIdentity throws by default but downgrades when throwOnDrift is false", () => {
  const a = { node: { id: "a" } };
  const drifted = { node: { id: "b" } };

  // Default contract preserved: drift throws.
  assert.throws(
    () => assertStableIdentity(a, drifted, { label: "Test" }),
    /Test replaced stable container node/,
  );

  // No drift returns the next snapshot untouched, both modes.
  const same = { node: a.node };
  assert.equal(assertStableIdentity(a, same, { throwOnDrift: false }).node, a.node);

  // Downgraded mode: report instead of throw, and return the latest containers.
  const originalError = console.error;
  const errors = [];
  const drifts = [];
  console.error = (...args) => errors.push(args);
  setIdentityDriftReporter((detail) => drifts.push(detail));
  try {
    const result = assertStableIdentity(a, drifted, { label: "Test", throwOnDrift: false });
    assert.equal(result, drifted);
    assert.equal(result.node.id, "b");
    assert.ok(errors.some((args) => /Test replaced stable container node/.test(String(args[0]))));
    assert.deepEqual(drifts.at(-1), {
      message: "Test replaced stable container node",
      label: "Test",
      noun: "container",
      key: "node",
    });
  } finally {
    console.error = originalError;
    setIdentityDriftReporter(null);
  }
});

test("reportIdentityDrift swallows reporter errors so telemetry can't crash the surface", () => {
  const originalError = console.error;
  console.error = () => {};
  setIdentityDriftReporter(() => {
    throw new Error("telemetry sink is down");
  });
  try {
    assert.doesNotThrow(() => reportIdentityDrift("drift", { key: "x" }));
  } finally {
    console.error = originalError;
    setIdentityDriftReporter(null);
  }
});
