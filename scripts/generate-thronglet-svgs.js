#!/usr/bin/env node
// Generate thronglet SVG sprites with CSS custom properties for color theming.
// Produces state variants: active, drowsy, sleeping, deep-sleep.
//
// Usage: node scripts/generate-thronglet-svgs.js
//
// Color variables (set on parent element when rendering inline):
//   --thr-body:    main body color    (default: #E07B39 orange)
//   --thr-outline: outline/border     (default: #8B3D1F dark brown)
//   --thr-accent:  darkest details    (default: #6B2A12)
//   --thr-shirt:   clothing color     (default: #7AAFC8 blue)

const fs = require("fs");
const path = require("path");

const SRC = path.join(__dirname, "..", "web", "public", "assets", "claude-thronglet.svg");
const OUT = path.join(__dirname, "..", "web", "public", "assets");

// ---- Parse original SVG ----

const raw = fs.readFileSync(SRC, "utf8");
const rects = [];
const re = /<rect x="(\d+)" y="(\d+)" width="16" height="16" fill="(#[A-Fa-f0-9]+)"\/>/g;
let m;
while ((m = re.exec(raw)) !== null) {
  rects.push({ x: +m[1], y: +m[2], fill: m[3] });
}
console.log(`Parsed ${rects.length} rects from source SVG`);

// ---- Color → CSS class mapping ----

const FILL_TO_CLASS = {
  "#E07B39": "b", // body
  "#8B3D1F": "o", // outline
  "#6B2A12": "a", // accent (darkest brown)
  "#F5C4A1": "s", // skin
  "#FFFFFF": "w", // white (eye whites)
  "#D9C8B8": "t", // tan (eye shadow)
  "#1A1A1A": "k", // black (pupils, nose, feet)
  "#7AAFC8": "c", // clothing/shirt
};

const STYLE_BLOCK = `  <style>
    .b { fill: var(--thr-body, #E07B39); }
    .o { fill: var(--thr-outline, #8B3D1F); }
    .a { fill: var(--thr-accent, #6B2A12); }
    .s { fill: #F5C4A1; }
    .w { fill: #FFFFFF; }
    .t { fill: #D9C8B8; }
    .k { fill: #1A1A1A; }
    .c { fill: var(--thr-shirt, #7AAFC8); }
  </style>`;

// Convert all rects to class-based
const baseRects = rects.map((r) => ({
  x: r.x,
  y: r.y,
  cls: FILL_TO_CLASS[r.fill] || "k",
}));

// ---- Helpers ----

function key(x, y) {
  return `${x},${y}`;
}

function applyOverrides(rects, overrides) {
  // overrides: { "x,y": "newClass", ... }
  // Returns new array; modifies matching rects, keeps the rest
  const result = rects.map((r) => {
    const k = key(r.x, r.y);
    if (k in overrides) {
      return { ...r, cls: overrides[k] };
    }
    return r;
  });
  return result;
}

function addRects(rects, extras) {
  // extras: [{ x, y, cls }, ...]
  return [...rects, ...extras];
}

function buildSvg(title, rectData) {
  const lines = rectData
    .map((r) => `  <rect x="${r.x}" y="${r.y}" width="16" height="16" class="${r.cls}"/>`)
    .join("\n");

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512" shape-rendering="crispEdges">
  <title>${title}</title>
${STYLE_BLOCK}
${lines}
</svg>
`;
}

// ---- Pixel art: "z" letter (3 wide × 4 tall at 16px grid) ----

function zPixels(bx, by, cls = "o") {
  // Renders a lowercase "z" in pixel art
  //  ███     top bar
  //    █     upper diagonal
  //  █       lower diagonal
  //  ███     bottom bar
  return [
    { x: bx, y: by, cls },
    { x: bx + 16, y: by, cls },
    { x: bx + 32, y: by, cls },
    { x: bx + 16, y: by + 16, cls },
    { x: bx, y: by + 32, cls },
    { x: bx, y: by + 48, cls },
    { x: bx + 16, y: by + 48, cls },
    { x: bx + 32, y: by + 48, cls },
  ];
}

// ---- Posture transforms ----

// Sitting: remove standing legs, widen base, add folded legs to sides
function sittingTransform(rects) {
  // Remove standing legs (y=400) and feet (y=416), then lower body by one tile
  // so the pose reads more "slouched/sitting" at small sizes.
  const loweredBody = rects
    .filter((r) => r.y < 400)
    .map((r) => ({ ...r, y: r.y + 16 }));

  // Widen seat outline at y=400, fold legs out at y=416, feet at y=432.
  return addRects(loweredBody, [
    // wider seat outline
    { x: 112, y: 400, cls: "o" },
    { x: 128, y: 400, cls: "o" },
    { x: 144, y: 400, cls: "o" },
    { x: 368, y: 400, cls: "o" },
    { x: 384, y: 400, cls: "o" },
    { x: 400, y: 400, cls: "o" },
    // legs folding out to sides
    { x: 96, y: 416, cls: "b" },
    { x: 112, y: 416, cls: "b" },
    { x: 128, y: 416, cls: "o" },
    { x: 384, y: 416, cls: "o" },
    { x: 400, y: 416, cls: "b" },
    { x: 416, y: 416, cls: "b" },
    // feet peeking out at edges
    { x: 80, y: 432, cls: "k" },
    { x: 96, y: 432, cls: "k" },
    { x: 416, y: 432, cls: "k" },
    { x: 432, y: 432, cls: "k" },
  ]);
}

// Laying down: rotate entire character 90° clockwise
// Head ends up on the right, feet on the left
function layingTransform(rects) {
  // CW 90° around center (256,256): new_x = 496 - y, new_y = x
  // (496 = 512 - 16, accounting for rect anchor being top-left)
  return rects.map((r) => ({
    x: 496 - r.y,
    y: r.x,
    cls: r.cls,
  }));
}

// ---- State variants ----

// Eye coordinates reference:
// Left eye whites:  (192,208) (208,208) (224,208)  y=208 top row
//                   (192,224) (208,224) (224,224*)  y=224 (* = tan shadow)
//                   (192,240) (208,240) (224,240*)  y=240 (* = pupil/black)
//                   (192,256) (208,256) (224,256*)  y=256 (* = pupil/black)
// Right eye whites: (288,208*) (304,208) (320,208)  y=208 (* = white)
//                   (288,224*) (304,224) (320,224)  y=224 (* = tan shadow)
//                   (288,240*) (304,240) (320,240)  y=240 (* = pupil/black)
//                   (288,256*) (304,256) (320,256)  y=256 (* = pupil/black)
// Under-eye:        (224,272) = accent, (288,272) = accent

// --- ACTIVE: standing, eyes wide open ---
const activeRects = baseRects;

// --- DROWSY: sitting down, heavy eyelids ---
const drowsyOverrides = {
  // Left eye top row → eyelid (accent = dark brown lid edge)
  [key(192, 208)]: "a",
  [key(208, 208)]: "a",
  [key(224, 208)]: "a",
  // Right eye top row → eyelid
  [key(288, 208)]: "a",
  [key(304, 208)]: "a",
  [key(320, 208)]: "a",
  // Left eye second row → eyelid skin (drooping further)
  [key(192, 224)]: "s",
  [key(208, 224)]: "s",
  // Right eye second row → eyelid skin
  [key(304, 224)]: "s",
  [key(320, 224)]: "s",
};
const drowsyRects = sittingTransform(
  applyOverrides(baseRects, drowsyOverrides),
);

// --- SLEEPING: laying down, eyes closed, single z ---
const sleepOverrides = {
  // Left eye: all whites/tan/pupils → skin, with closed-lid line at y=240
  [key(192, 208)]: "s",
  [key(208, 208)]: "s",
  [key(224, 208)]: "s",
  [key(192, 224)]: "s",
  [key(208, 224)]: "s",
  [key(224, 224)]: "s", // was tan
  [key(192, 240)]: "s",
  [key(208, 240)]: "a", // closed lid line
  [key(224, 240)]: "a", // closed lid line
  [key(192, 256)]: "s",
  [key(208, 256)]: "s",
  [key(224, 256)]: "s",
  // Right eye: same pattern
  [key(288, 208)]: "s",
  [key(304, 208)]: "s",
  [key(320, 208)]: "s",
  [key(288, 224)]: "s", // was tan
  [key(304, 224)]: "s",
  [key(320, 224)]: "s",
  [key(288, 240)]: "a", // closed lid line
  [key(304, 240)]: "a", // closed lid line
  [key(320, 240)]: "s",
  [key(288, 256)]: "s",
  [key(304, 256)]: "s",
  [key(320, 256)]: "s",
};
// Apply eye changes, then rotate to laying down, then add z above head
const sleepLaying = layingTransform(
  applyOverrides(baseRects, sleepOverrides),
);
const sleepRects = addRects(
  sleepLaying,
  zPixels(384, 48), // z floating above the horizontal character's head
);

// --- DEEP SLEEP: laying down, eyes closed, open mouth, multiple Z's ---
const deepSleepOverrides = {
  ...sleepOverrides,
  // Larger open mouth (breathing): darker 2x2-ish shape for readability.
  [key(240, 304)]: "k",
  [key(256, 304)]: "k",
  [key(272, 304)]: "k",
  [key(256, 320)]: "k",
};
const deepSleepLaying = layingTransform(
  applyOverrides(baseRects, deepSleepOverrides),
);
const deepSleepRects = addRects(deepSleepLaying, [
  ...zPixels(432, 0, "o"), // large Z (higher/right, near head)
  ...zPixels(384, 32, "a"), // medium z
  ...zPixels(464, 48, "a"), // small z (furthest right)
]);

// ---- Write output files ----

const variants = [
  { name: "thronglet-active.svg", title: "Thronglet - Active", data: activeRects },
  { name: "thronglet-drowsy.svg", title: "Thronglet - Drowsy", data: drowsyRects },
  { name: "thronglet-sleeping.svg", title: "Thronglet - Sleeping", data: sleepRects },
  { name: "thronglet-deep-sleep.svg", title: "Thronglet - Deep Sleep", data: deepSleepRects },
];

for (const v of variants) {
  const svg = buildSvg(v.title, v.data);
  const out = path.join(OUT, v.name);
  fs.writeFileSync(out, svg);
  console.log(`  ${v.name} (${v.data.length} rects)`);
}

// ---- Also output a TypeScript module for inline rendering ----
// This lets Preact components import SVGs as strings and render inline,
// which is required for CSS custom properties to cascade from the parent DOM.

const TS_OUT = path.join(__dirname, "..", "web", "src", "lib", "thronglet-svgs.ts");
const tsLines = [
  "// AUTO-GENERATED by scripts/generate-thronglet-svgs.js — do not edit",
  "// Re-run: node scripts/generate-thronglet-svgs.js",
  "",
];
for (const v of variants) {
  const constName = v.name
    .replace("thronglet-", "")
    .replace(".svg", "")
    .replace(/-/g, "_")
    .toUpperCase();
  const svg = buildSvg(v.title, v.data).replace(/\n/g, "");
  tsLines.push(`export const ${constName} = ${JSON.stringify(svg)};`);
  tsLines.push("");
}
fs.mkdirSync(path.dirname(TS_OUT), { recursive: true });
fs.writeFileSync(TS_OUT, tsLines.join("\n"));
console.log(`\n  thronglet-svgs.ts (inline module)`);

console.log("\nDone! SVGs use CSS custom properties:");
console.log("  --thr-body     body color       (default: #E07B39)");
console.log("  --thr-outline  outline color    (default: #8B3D1F)");
console.log("  --thr-accent   accent color     (default: #6B2A12)");
console.log("  --thr-shirt    clothing color   (default: #7AAFC8)");
console.log("\nTo recolor for Codex, set these on the parent element:");
console.log('  style="--thr-body: #F4C542; --thr-outline: #8B6B00; --thr-accent: #5E4600;"');
