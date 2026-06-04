import test from "node:test";
import assert from "node:assert/strict";

import {
  bindAppEvents,
  installEventListenerBinding,
  installTerminalStageResizeObserver,
} from "./app_event_bindings.js";

function eventTarget(name, log = []) {
  return {
    name,
    listeners: [],
    addEventListener(eventType, listener, options) {
      this.listeners.push({ eventType, listener, options });
      log.push(`${name}:${eventType}:${options?.capture ? "capture" : "bubble"}`);
    },
  };
}

test("installEventListenerBinding preserves required target and document binding behavior", () => {
  const log = [];
  const documentTarget = eventTarget("document", log);
  const buttonTarget = eventTarget("button", log);
  const shortcut = () => {};
  const click = () => {};
  const runtime = {
    document: documentTarget,
    elements: { button: buttonTarget },
    handlers: { shortcut, click },
  };

  installEventListenerBinding(
    { target: "document", eventType: "keydown", handler: "shortcut", options: { passive: true } },
    runtime,
  );
  installEventListenerBinding(
    { target: "button", eventType: "click", handler: "click" },
    runtime,
  );

  assert.deepEqual(log, ["document:keydown:bubble", "button:click:bubble"]);
  assert.equal(documentTarget.listeners[0].listener, shortcut);
  assert.deepEqual(documentTarget.listeners[0].options, { passive: true });
  assert.equal(buttonTarget.listeners[0].listener, click);
});

test("installEventListenerBinding preserves optional target and optional listener behavior", () => {
  const handler = () => {};

  assert.doesNotThrow(() => installEventListenerBinding(
    { target: "missing", eventType: "click", handler: "missingHandler", optionalTarget: true },
    { elements: {}, handlers: {} },
  ));

  assert.doesNotThrow(() => installEventListenerBinding(
    { target: "targetWithoutListener", eventType: "click", handler: "handler", optionalListener: true },
    { elements: { targetWithoutListener: {} }, handlers: { handler } },
  ));

  assert.throws(
    () => installEventListenerBinding(
      { target: "targetWithoutListener", eventType: "click", handler: "handler" },
      { elements: { targetWithoutListener: {} }, handlers: { handler } },
    ),
    /target\.addEventListener is not a function/,
  );
});

test("installEventListenerBinding preserves handler missing-error semantics", () => {
  const target = eventTarget("button");

  assert.throws(
    () => installEventListenerBinding(
      { target: "button", eventType: "click", handler: "missing" },
      { elements: { button: target }, handlers: {} },
    ),
    { message: "Missing event listener handler: missing" },
  );

  assert.doesNotThrow(() => installEventListenerBinding(
    { target: "absent", eventType: "click", handler: "missing", optionalTarget: true },
    { elements: {}, handlers: {} },
  ));
});

test("bindAppEvents preserves terminal stage capture ordering and resize installation", () => {
  const log = [];
  const captureCalls = [];
  const documentTarget = eventTarget("document", log);
  const terminalStage = eventTarget("terminalStage", log);
  let resizeCallback = null;

  class FakeResizeObserver {
    constructor(callback) {
      resizeCallback = callback;
      log.push("resize:new");
    }

    observe(target) {
      log.push(`resize:observe:${target.name}`);
    }
  }

  bindAppEvents({
    document: documentTarget,
    elements: { terminalStage },
    handlers: {
      shortcut() {},
      stageClick() {},
    },
    bindTrogdorEvents() {
      log.push("trogdor");
    },
    appEventListenerBindingPlan() {
      log.push("plan");
      return {
        beforeTerminalStageCapture: [
          { target: "document", eventType: "keydown", handler: "shortcut" },
        ],
        afterTerminalStageCapture: [
          { target: "terminalStage", eventType: "click", handler: "stageClick" },
        ],
      };
    },
    terminalStageCaptureBindings() {
      log.push("capture-plan");
      return [
        { eventType: "click", action: "click", options: { capture: true } },
      ];
    },
    captureSurfaceAction(event, action) {
      captureCalls.push({ event, action });
    },
    ResizeObserver: FakeResizeObserver,
    queueMeasureAndResizeSurface(force, pushResize) {
      log.push(`resize:queue:${force}:${pushResize}`);
    },
  });

  assert.deepEqual(log, [
    "trogdor",
    "plan",
    "document:keydown:bubble",
    "capture-plan",
    "terminalStage:click:capture",
    "terminalStage:click:bubble",
    "resize:new",
    "resize:observe:terminalStage",
  ]);

  terminalStage.listeners[0].listener({ type: "click" });
  assert.deepEqual(captureCalls, [{ event: { type: "click" }, action: "click" }]);

  resizeCallback();
  assert.equal(log.at(-1), "resize:queue:true:false");
});

test("installTerminalStageResizeObserver observes terminal stage and queues forced measurement", () => {
  const log = [];
  const terminalStage = { name: "terminalStage" };
  let resizeCallback = null;

  class FakeResizeObserver {
    constructor(callback) {
      resizeCallback = callback;
      log.push("new");
    }

    observe(target) {
      log.push(`observe:${target.name}`);
    }
  }

  const observer = installTerminalStageResizeObserver({
    ResizeObserver: FakeResizeObserver,
    elements: { terminalStage },
    queueMeasureAndResizeSurface(force, pushResize) {
      log.push(`queue:${force}:${pushResize}`);
    },
  });

  assert.ok(observer instanceof FakeResizeObserver);
  assert.deepEqual(log, ["new", "observe:terminalStage"]);
  resizeCallback();
  assert.deepEqual(log, ["new", "observe:terminalStage", "queue:true:false"]);
});
