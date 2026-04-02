import test from "node:test";
import assert from "node:assert/strict";

import { eventCell, eventClientPoint, shouldIgnoreSyntheticClick } from "./input_support.js";

test("eventClientPoint uses direct pointer coordinates when present", () => {
  assert.deepEqual(eventClientPoint({ clientX: 120, clientY: 48 }), {
    x: 120,
    y: 48,
  });
});

test("eventClientPoint falls back to changedTouches for mobile taps", () => {
  assert.deepEqual(
    eventClientPoint({
      changedTouches: [{ clientX: 33, clientY: 77 }],
    }),
    {
      x: 33,
      y: 77,
    },
  );
});

test("eventCell maps touch coordinates into the rendered grid", () => {
  const cell = eventCell(
    {
      changedTouches: [{ clientX: 75, clientY: 90 }],
    },
    { left: 0, top: 0, width: 160, height: 160 },
    16,
    16,
  );
  assert.deepEqual(cell, { x: 7, y: 9 });
});

test("shouldIgnoreSyntheticClick suppresses the follow-up click after a handled press", () => {
  assert.equal(shouldIgnoreSyntheticClick(100, 120), true);
  assert.equal(shouldIgnoreSyntheticClick(140, 120), false);
});
