import test from "node:test";
import assert from "node:assert/strict";

import {
  createTrogdorEventBindings,
} from "./trogdor_event_bindings.js";

class FakeElement {
  constructor({ dataset = {}, disabled = false, closestMap = {} } = {}) {
    this.dataset = dataset;
    this.disabled = disabled;
    this.closestMap = closestMap;
    this.contained = new Set();
    this.listeners = [];
  }

  addEventListener(eventName, listener, options) {
    this.listeners.push({ eventName, listener, options });
  }

  closest(selector) {
    return this.closestMap[selector] ?? null;
  }

  contains(target) {
    return target === this || this.contained.has(target);
  }
}

function fakeEvent(target, extra = {}) {
  const calls = [];
  return {
    target,
    relatedTarget: null,
    preventDefault() {
      calls.push("preventDefault");
    },
    stopPropagation() {
      calls.push("stopPropagation");
    },
    calls,
    ...extra,
  };
}

function listenerFor(target, eventName) {
  const binding = target.listeners.find((item) => item.eventName === eventName);
  assert.ok(binding, `missing ${eventName} listener`);
  return binding.listener;
}

test("bindTrogdorEvents installs launcher, passthrough, and surface listeners in order", () => {
  const launcher = new FakeElement();
  const surface = new FakeElement();
  const calls = [];
  const bindings = createTrogdorEventBindings({
    elements: { trogdorLauncher: launcher, trogdorSurface: surface },
    ElementClass: FakeElement,
    openTrogdorAtlas() {
      calls.push("open-atlas");
    },
  });

  bindings.bindTrogdorEvents();

  assert.deepEqual(launcher.listeners.map((item) => item.eventName), ["click"]);
  assert.deepEqual(surface.listeners.map((item) => item.eventName), [
    "pointerdown",
    "mousedown",
    "mouseup",
    "mousemove",
    "touchend",
    "wheel",
    "click",
    "mouseover",
    "mouseleave",
    "focusin",
    "focusout",
  ]);
  assert.equal(surface.listeners.find((item) => item.eventName === "touchend").options.passive, false);
  assert.equal(surface.listeners.find((item) => item.eventName === "wheel").options.passive, false);

  const event = fakeEvent(launcher);
  listenerFor(launcher, "click")(event);
  assert.deepEqual(event.calls, ["preventDefault"]);
  assert.deepEqual(calls, ["open-atlas"]);
});

test("Trogdor pointer and click handlers preserve default prevention and action dispatch", () => {
  const surface = new FakeElement();
  const agent = new FakeElement({ dataset: { sessionId: "agent-1" } });
  const button = new FakeElement({
    dataset: {
      action: "trogdor_send",
      sessionId: "agent-2",
      label: "Agent Two",
      sessionIds: "[\"agent-2\",\"agent-3\"]",
    },
  });
  const agentTarget = new FakeElement({ closestMap: { "[data-trogdor-agent]": agent } });
  const buttonTarget = new FakeElement({
    closestMap: {
      "button[data-action]": button,
      "[data-trogdor-agent]": agent,
    },
  });
  const actions = [];
  const terminals = [];
  let clock = 1000;
  const bindings = createTrogdorEventBindings({
    elements: { trogdorSurface: surface },
    ElementClass: FakeElement,
    handleSurfaceAction(zone) {
      actions.push(zone);
    },
    openTrogdorAgentTerminal(sessionId) {
      terminals.push(sessionId);
    },
    now: () => clock,
  });
  bindings.bindTrogdorEvents();

  const pointerEvent = fakeEvent(agentTarget);
  listenerFor(surface, "pointerdown")(pointerEvent);
  assert.deepEqual(pointerEvent.calls, ["preventDefault", "stopPropagation"]);
  assert.deepEqual(terminals, ["agent-1"]);

  const buttonClick = fakeEvent(buttonTarget);
  listenerFor(surface, "click")(buttonClick);
  assert.deepEqual(buttonClick.calls, ["preventDefault", "stopPropagation"]);
  assert.deepEqual(actions, [{
    type: "action",
    actionId: "trogdor_send",
    sessionId: "agent-2",
    label: "Agent Two",
    sessionIds: ["agent-2", "agent-3"],
  }]);

  // The synthetic click that follows the pointerdown open is suppressed, so the
  // agent terminal is not opened (or dispatched) a second time.
  const syntheticAgentClick = fakeEvent(agentTarget);
  listenerFor(surface, "click")(syntheticAgentClick);
  assert.deepEqual(syntheticAgentClick.calls, ["preventDefault", "stopPropagation"]);
  assert.equal(actions.length, 1);
  assert.deepEqual(terminals, ["agent-1"]);

  // A click past the suppression window (e.g. keyboard activation, which has no
  // preceding pointerdown) still opens the agent.
  clock += 1000;
  const keyboardAgentClick = fakeEvent(agentTarget);
  listenerFor(surface, "click")(keyboardAgentClick);
  assert.deepEqual(actions.at(-1), { type: "trogdor_agent", sessionId: "agent-1" });
});

test("Trogdor synthetic-click suppression is consumed one-shot, not held for the full window", () => {
  const surface = new FakeElement();
  const agent = new FakeElement({ dataset: { sessionId: "agent-1" } });
  const agentTarget = new FakeElement({ closestMap: { "[data-trogdor-agent]": agent } });
  const actions = [];
  const terminals = [];
  const clock = 1000; // deliberately never advances — that is the regression
  const bindings = createTrogdorEventBindings({
    elements: { trogdorSurface: surface },
    ElementClass: FakeElement,
    handleSurfaceAction(zone) {
      actions.push(zone);
    },
    openTrogdorAgentTerminal(sessionId) {
      terminals.push(sessionId);
    },
    now: () => clock,
  });
  bindings.bindTrogdorEvents();

  // Mouse open: pointerdown opens agent-1 and arms the suppress window.
  listenerFor(surface, "pointerdown")(fakeEvent(agentTarget));
  assert.deepEqual(terminals, ["agent-1"]);

  // The synthetic click that follows is suppressed AND consumes the window.
  listenerFor(surface, "click")(fakeEvent(agentTarget));
  assert.equal(actions.length, 0);

  // A genuine keyboard Enter (a click with no preceding pointerdown) within the
  // same 450ms window must still dispatch — the window was consumed one-shot,
  // not held open to swallow legitimate keyboard activation.
  listenerFor(surface, "click")(fakeEvent(agentTarget));
  assert.deepEqual(actions.at(-1), { type: "trogdor_agent", sessionId: "agent-1" });
});

test("Trogdor residual suppress window only targets the armed agent, never a different one", () => {
  const surface = new FakeElement();
  const agent1 = new FakeElement({ dataset: { sessionId: "agent-1" } });
  const agent2 = new FakeElement({ dataset: { sessionId: "agent-2" } });
  const agent1Target = new FakeElement({ closestMap: { "[data-trogdor-agent]": agent1 } });
  const agent2Target = new FakeElement({ closestMap: { "[data-trogdor-agent]": agent2 } });
  // A DOM button whose pointerup lands elsewhere — its click resolves to
  // dom_action, never to agent-1's surface_action, so the armed window lingers.
  const offButton = new FakeElement({ dataset: { action: "trogdor_refresh" } });
  const offTarget = new FakeElement({ closestMap: { "button[data-action]": offButton } });
  const actions = [];
  const terminals = [];
  const clock = 1000; // never advances: the window stays open the whole test
  const bindings = createTrogdorEventBindings({
    elements: { trogdorSurface: surface },
    ElementClass: FakeElement,
    handleSurfaceAction(zone) {
      actions.push(zone);
    },
    openTrogdorAgentTerminal(sessionId) {
      terminals.push(sessionId);
    },
    now: () => clock,
  });
  bindings.bindTrogdorEvents();

  // Mouse open on agent-1 arms the suppress window for agent-1.
  listenerFor(surface, "pointerdown")(fakeEvent(agent1Target));
  assert.deepEqual(terminals, ["agent-1"]);

  // The follow-up click lands off the agent (resolves to dom_action), so the
  // agent-1 window is never consumed — it lingers, still open.
  listenerFor(surface, "click")(fakeEvent(offTarget));

  // (b) A genuine keyboard Enter on a DIFFERENT agent within the residual window
  // must NOT be swallowed — its sessionId differs from the armed one.
  listenerFor(surface, "click")(fakeEvent(agent2Target));
  assert.deepEqual(actions.at(-1), { type: "trogdor_agent", sessionId: "agent-2" });

  // (a) The synthetic click on the ARMED agent is still suppressed exactly once
  // (no double-open), even though it arrives after the unrelated activity.
  const before = actions.length;
  listenerFor(surface, "click")(fakeEvent(agent1Target));
  assert.equal(actions.length, before);
  // ...and the next click on agent-1 (one-shot consumed) dispatches normally.
  listenerFor(surface, "click")(fakeEvent(agent1Target));
  assert.deepEqual(actions.at(-1), { type: "trogdor_agent", sessionId: "agent-1" });
});

test("Trogdor handlers preserve disabled button and non-element target behavior", async () => {
  const surface = new FakeElement();
  const disabledButton = new FakeElement({
    disabled: true,
    dataset: { action: "trogdor_send", sessionId: "agent-disabled" },
  });
  const buttonTarget = new FakeElement({ closestMap: { "button[data-action]": disabledButton } });
  const actions = [];
  const terminals = [];
  const bindings = createTrogdorEventBindings({
    elements: { trogdorSurface: surface },
    ElementClass: FakeElement,
    handleSurfaceAction(zone) {
      actions.push(zone);
    },
    openTrogdorAgentTerminal(sessionId) {
      terminals.push(sessionId);
    },
  });
  bindings.bindTrogdorEvents();

  const disabledClick = fakeEvent(buttonTarget);
  listenerFor(surface, "click")(disabledClick);
  assert.deepEqual(disabledClick.calls, ["preventDefault", "stopPropagation"]);
  await bindings.handleTrogdorDomAction(disabledButton);
  assert.deepEqual(actions, []);

  const plainClick = fakeEvent({});
  listenerFor(surface, "click")(plainClick);
  assert.deepEqual(plainClick.calls, ["preventDefault", "stopPropagation"]);
  assert.deepEqual(actions, []);

  const plainPointerDown = fakeEvent({});
  listenerFor(surface, "pointerdown")(plainPointerDown);
  assert.deepEqual(plainPointerDown.calls, []);
  assert.deepEqual(terminals, []);
});

test("Trogdor passthrough, hover, and focus handlers preserve surface updates", () => {
  const surface = new FakeElement();
  const inside = new FakeElement();
  surface.contained.add(inside);
  const agent = new FakeElement({ dataset: { sessionId: "agent-1" } });
  const action = new FakeElement({ dataset: { action: "trogdor_commit" } });
  const ignoredAction = new FakeElement({ dataset: { action: "refresh" } });
  const agentTarget = new FakeElement({ closestMap: { "[data-trogdor-agent]": agent } });
  const actionTarget = new FakeElement({ closestMap: { "button[data-action]": action } });
  const ignoredTarget = new FakeElement({ closestMap: { "button[data-action]": ignoredAction } });
  const hovers = [];
  const bindings = createTrogdorEventBindings({
    elements: { trogdorSurface: surface },
    ElementClass: FakeElement,
    updateHoveredTrogdorSurface(hover) {
      hovers.push(hover);
    },
  });
  bindings.bindTrogdorEvents();

  const passthrough = fakeEvent(surface);
  listenerFor(surface, "mousedown")(passthrough);
  assert.deepEqual(passthrough.calls, ["stopPropagation"]);

  listenerFor(surface, "mouseover")(fakeEvent(agentTarget));
  listenerFor(surface, "mouseover")(fakeEvent(actionTarget));
  listenerFor(surface, "mouseover")(fakeEvent(ignoredTarget));
  listenerFor(surface, "mouseleave")(fakeEvent(surface));
  listenerFor(surface, "focusin")(fakeEvent(agentTarget));
  listenerFor(surface, "focusout")(fakeEvent(agentTarget, { relatedTarget: inside }));
  listenerFor(surface, "focusout")(fakeEvent(agentTarget, { relatedTarget: new FakeElement() }));

  assert.deepEqual(hovers, [
    { type: "trogdor_agent", sessionId: "agent-1" },
    { type: "action", actionId: "trogdor_commit" },
    null,
    { type: "trogdor_agent", sessionId: "agent-1" },
    null,
  ]);
});
