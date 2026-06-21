import test from "node:test";
import assert from "node:assert/strict";

import { fakeDocumentForIds } from "./island_test_helpers.mjs";
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

test("React shell identity guard catches synchronous container replacement", () => {
  const { documentRef, replace } = fakeDocument();
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

  assert.throws(
    () => handle.render({ franken_term_available: true }),
    /replaced stable container terminalCanvas/,
  );
});

test("React shell identity guard catches fallback and mirror replacement", () => {
  for (const key of ["terminalFallback", "terminalA11yMirror"]) {
    const { documentRef, replace } = fakeDocument();
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

    assert.throws(
      () => handle.render({ franken_term_available: true }),
      new RegExp(`replaced stable container ${key}`),
    );
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
