import test from "node:test";
import assert from "node:assert/strict";

import {
  COLORS,
  STYLE_BOLD,
  actionSpec,
  cellInRect,
  computeSurfaceLayout,
  drawActionChipRow,
  drawTextBlock,
  expandedRect,
  rect,
  wrapText,
} from "./rendered_surface_draw.js";

function frame(cols = 24, rows = 8) {
  return {
    cols,
    rows,
    cells: new Uint32Array(cols * rows * 4),
    zones: [],
    masks: [],
  };
}

function lineText(surface, row) {
  let line = "";
  for (let col = 0; col < surface.cols; col += 1) {
    const index = (row * surface.cols + col) * 4;
    line += String.fromCodePoint(surface.cells[index + 2] || 32);
  }
  return line;
}

test("computeSurfaceLayout preserves rendered surface breakpoints and dimensions", () => {
  assert.deepEqual(computeSurfaceLayout(140, 40, false), {
    header: { x: 2, y: 1, w: 136, h: 4 },
    footer: { x: 2, y: 34, w: 136, h: 5 },
    sessionRail: { x: 2, y: 6, w: 32, h: 27 },
    detailRail: { x: 102, y: 6, w: 36, h: 27 },
    center: { x: 35, y: 6, w: 66, h: 27 },
  });

  assert.deepEqual(computeSurfaceLayout(140, 40, true), {
    header: { x: 2, y: 1, w: 136, h: 4 },
    footer: { x: 2, y: 34, w: 136, h: 5 },
    sessionRail: null,
    detailRail: { x: 102, y: 6, w: 36, h: 27 },
    center: { x: 2, y: 6, w: 99, h: 27 },
  });

  assert.deepEqual(computeSurfaceLayout(80, 24, false), {
    header: { x: 2, y: 1, w: 76, h: 4 },
    footer: { x: 2, y: 19, w: 76, h: 4 },
    sessionRail: null,
    detailRail: null,
    center: { x: 2, y: 6, w: 76, h: 12 },
  });
});

test("drawTextBlock preserves wrapping, truncation, color, and attrs", () => {
  const surface = frame(12, 4);

  drawTextBlock(surface, 1, 0, 6, 2, "alpha beta gamma", {
    fg: COLORS.warning,
    bg: COLORS.panelBg,
    attrs: STYLE_BOLD,
  });

  assert.equal(lineText(surface, 0).slice(1, 7), "alpha ");
  assert.equal(lineText(surface, 1).slice(1, 7), "beta  ");

  const firstCell = (0 * surface.cols + 1) * 4;
  assert.equal(surface.cells[firstCell], COLORS.panelBg);
  assert.equal(surface.cells[firstCell + 1], COLORS.warning);
  assert.equal(surface.cells[firstCell + 3], STYLE_BOLD);
  assert.deepEqual(wrapText("toolongword", 4, 1), ["t..."]);
});

test("geometry helpers preserve clipped hit testing rectangles", () => {
  const surface = frame(5, 4);
  const clipped = expandedRect(surface, 1, 1, 2, 1, 2, 2);

  assert.deepEqual(clipped, { x: 0, y: 0, w: 5, h: 4 });
  assert.equal(cellInRect({ x: 4, y: 3 }, clipped), true);
  assert.equal(cellInRect({ x: 5, y: 3 }, clipped), false);
  assert.deepEqual(rect(2, 3, 4, 5), { x: 2, y: 3, w: 4, h: 5 });
});

test("drawActionChipRow preserves zone payloads and padded hitboxes", () => {
  const surface = frame(28, 5);
  const actions = [
    actionSpec("open", "open", "open_sheet", true, { sheet: "config" }),
    actionSpec("send", "send", "send_message", false, { sessionId: "sess-1" }),
    actionSpec("hidden", "hidden", "hidden_action", true),
  ];

  drawActionChipRow(surface, 2, 2, 22, actions, "action", { hitPadY: 1 });

  assert.equal(surface.zones.length, 2);
  assert.deepEqual(surface.zones[0], {
    type: "action",
    actionId: "open_sheet",
    disabled: false,
    label: "open",
    sheet: "config",
    rect: { x: 2, y: 1, w: 8, h: 3 },
  });
  assert.deepEqual(surface.zones[1], {
    type: "action",
    actionId: "send_message",
    disabled: true,
    label: "send",
    sessionId: "sess-1",
    rect: { x: 11, y: 1, w: 8, h: 3 },
  });
});
