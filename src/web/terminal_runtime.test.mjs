import test from "node:test";
import assert from "node:assert/strict";

import {
  assertFrankenTermModule,
  canvasHasVisiblePixels,
  frankenTermAssetSummary,
  isFrankenTermReentryError,
  surfaceBusy,
  surfaceSupports,
  validateFrankenTermSurface,
  withSurfaceOperation,
} from "./terminal_runtime.js";

test("FrankenTerm module and surface validators preserve error messages", () => {
  const mod = { default() {}, FrankenTermWeb() {} };
  assert.equal(assertFrankenTermModule(mod), mod);
  assert.throws(
    () => assertFrankenTermModule({ FrankenTermWeb() {} }),
    /missing its wasm initializer/,
  );
  assert.throws(
    () => assertFrankenTermModule({ default() {} }),
    /missing FrankenTermWeb/,
  );

  const surface = { init() {}, render() {} };
  assert.equal(surfaceSupports(surface, "render"), true);
  assert.equal(validateFrankenTermSurface(surface, ["init", "render"], "HUD"), surface);
  assert.throws(
    () => validateFrankenTermSurface(surface, ["init", "resize", "feed"], "HUD"),
    /HUD missing methods: resize, feed/,
  );
});

test("FrankenTerm asset summary preserves js wasm font ordering and optional fields", () => {
  assert.equal(frankenTermAssetSummary(null), "");
  assert.equal(
    frankenTermAssetSummary({
      wasm: { checksum: "sha256:wasm", size_bytes: 12 },
      js: { checksum: "sha256:js" },
      font: { size_bytes: 34 },
      ignored: { checksum: "nope", size_bytes: 99 },
    }),
    "js sha256:js; wasm sha256:wasm 12b; font 34b",
  );
});

test("surface operation guard defers busy and records recursive renderer errors", () => {
  const busyState = {
    surfaceInitInProgress: 1,
    surfaceOperationDepth: 0,
    lastRendererDiagnosticError: "",
  };
  assert.deepEqual(withSurfaceOperation(busyState, "render", () => "unused"), { deferred: true });

  const readyState = {
    surfaceInitInProgress: 0,
    surfaceOperationDepth: 0,
    lastRendererDiagnosticError: "",
  };
  assert.deepEqual(withSurfaceOperation(readyState, "render", () => "ok"), {
    deferred: false,
    value: "ok",
  });
  assert.equal(readyState.surfaceOperationDepth, 0);
  assert.equal(surfaceBusy(readyState), false);

  const recursive = withSurfaceOperation(readyState, "render", () => {
    throw new Error("recursive use of an object");
  });
  assert.equal(recursive.deferred, true);
  assert.match(readyState.lastRendererDiagnosticError, /render: recursive use of an object/);
  assert.equal(isFrankenTermReentryError(recursive.error), true);

  assert.throws(
    () => withSurfaceOperation(readyState, "render", () => {
      throw new Error("ordinary failure");
    }),
    /ordinary failure/,
  );
  assert.equal(readyState.surfaceOperationDepth, 0);
});

test("canvas visible pixel probe preserves dimensions threshold and failure fallbacks", () => {
  assert.equal(canvasHasVisiblePixels(null, {}), false);
  assert.equal(canvasHasVisiblePixels({ width: 0, height: 10 }, {}), false);

  const calls = [];
  const documentRef = {
    createElement() {
      return {
        width: 0,
        height: 0,
        getContext() {
          return {
            drawImage(_canvas, _x, _y, width, height) {
              calls.push({ width, height });
            },
            getImageData() {
              return { data: new Uint8ClampedArray([0, 0, 0, 255, 33, 0, 0, 255]) };
            },
          };
        },
      };
    },
  };

  assert.equal(canvasHasVisiblePixels({ width: 500, height: 300 }, documentRef), true);
  assert.deepEqual(calls, [{ width: 180, height: 120 }]);

  const blankDocument = {
    createElement() {
      return {
        width: 0,
        height: 0,
        getContext() {
          return {
            drawImage() {},
            getImageData() {
              return { data: new Uint8ClampedArray([32, 32, 32, 255]) };
            },
          };
        },
      };
    },
  };
  assert.equal(canvasHasVisiblePixels({ width: 20, height: 10 }, blankDocument), false);

  const throwingDocument = {
    createElement() {
      return {
        width: 0,
        height: 0,
        getContext() {
          return {
            drawImage() {
              throw new Error("tainted");
            },
          };
        },
      };
    },
  };
  assert.equal(canvasHasVisiblePixels({ width: 20, height: 10 }, throwingDocument), false);
});
