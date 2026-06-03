import test from "node:test";
import assert from "node:assert/strict";

import {
  renderTrogdorDragon,
  renderTrogdorEmptyField,
  renderTrogdorStructure,
  renderTrogdorSurfaceFrame,
  trogdorReadButtonLabel,
  trogdorDragonAsset,
  trogdorSurfaceSignature,
} from "./trogdor_render.js";

function session(overrides = {}) {
  return {
    sessionId: "agent-1",
    name: "agent-1",
    state: "attention",
    restLabel: "active",
    actionCues: [{ kind: "awaiting_user" }],
    trogdorAwaitingUser: true,
    trogdorBurnt: false,
    trogdorDismissed: false,
    trogdorSwordsmanVisible: true,
    ...overrides,
  };
}

test("Trogdor dragon renderer emits 8-way sprite assets and fire state", () => {
  const html = renderTrogdorDragon({
    direction: "left",
    bodyFrame: "back-left",
    x: 38,
    y: 58,
    walkX: "-3.2vw",
    walkY: "-1.2vh",
    heated: true,
    firing: true,
  });

  assert.equal(trogdorDragonAsset("mouth-closed", "back-left"), "/assets/dragon/mouth-closed/back-left.png");
  assert.match(html, /class="[^"]*trogdor-dragon[^"]*is-firing/);
  assert.match(html, /data-dragon-frame="back-left"/);
  assert.match(html, /\/assets\/dragon\/fire-left-full\/back-left\.png/);
});

test("Trogdor structure renderer filters invisible agents and carries burn classes", () => {
  const group = {
    label: "swimmers",
    reason: "awaiting user",
    pressure: 80,
    sessions: [
      session({ sessionId: "visible", trogdorSwordsmanVisible: true, trogdorBurnt: true }),
      session({ sessionId: "hidden", trogdorSwordsmanVisible: false }),
    ],
  };
  const html = renderTrogdorStructure(
    group,
    0,
    { x: 18, y: 40, size: "large", variant: "hut" },
    { x: 38, y: 58 },
    { hoveredSessionId: "visible" },
  );

  assert.match(html, /is-variant-ruin/);
  assert.match(html, /is-burning/);
  assert.match(html, /data-session-id="visible"/);
  assert.doesNotMatch(html, /data-session-id="hidden"/);
  assert.match(html, /agent-burn-flame/);
  assert.match(html, /is-hovered/);
});

test("Trogdor empty field respects readonly create state", () => {
  assert.match(renderTrogdorEmptyField(false), /data-action="open_create">launch agent/);
  assert.match(renderTrogdorEmptyField(true), /data-action="open_create" disabled>launch agent/);
});

test("Trogdor surface frame renders scoreboard, controls, and structures", () => {
  const visible = session({ sessionId: "visible", trogdorSwordsmanVisible: true });
  const html = renderTrogdorSurfaceFrame({
    groups: [
      {
        label: "swimmers",
        reason: "awaiting user",
        pressure: 72,
        sessions: [visible],
      },
    ],
    sessions: [visible],
    summary: { score: "1234", level: 72 },
    dragonPose: { direction: "right", bodyFrame: "right", x: 42, y: 60 },
    readerMarkup: '<div class="trogdor-banner" data-trogdor-reader="true">burninate!</div>',
    readButtonLabel: "pause",
    wpm: 225,
    hoveredSessionId: "visible",
  });

  assert.match(html, /<strong>1234<\/strong>/);
  assert.match(html, /mans: 1/);
  assert.match(html, /225 wpm/);
  assert.match(html, /data-action="focus_terminal"/);
  assert.match(html, /data-session-id="visible"/);
  assert.match(html, /is-hovered/);
});

test("Trogdor surface signature tracks pressure and readonly changes", () => {
  const base = session({ operatorPressure: { score: 20, reason: "quiet", glyph: "q" } });
  const hot = session({ operatorPressure: { score: 80, reason: "awaiting user", glyph: "!" } });
  const summary = { score: "0080", level: 1 };

  assert.notEqual(
    trogdorSurfaceSignature([base], summary, false),
    trogdorSurfaceSignature([hot], summary, false),
  );
  assert.notEqual(
    trogdorSurfaceSignature([base], summary, false),
    trogdorSurfaceSignature([base], summary, true),
  );
});

test("Trogdor read button labels distinguish pause, read, and replay", () => {
  assert.equal(trogdorReadButtonLabel(true, false), "pause");
  assert.equal(trogdorReadButtonLabel(false, false), "read");
  assert.equal(trogdorReadButtonLabel(false, true), "read again");
});
