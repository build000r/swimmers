import test from "node:test";
import assert from "node:assert/strict";

import {
  TROGDOR_ATLAS_ISLAND_ARIA_LABEL,
  TROGDOR_ATLAS_ISLAND_CLASS_NAME,
  TROGDOR_ATLAS_ISLAND_ID,
  TROGDOR_ATLAS_ISLAND_PROPS,
  createTrogdorAtlasIsland,
  createTrogdorAtlasIslandElement,
} from "./trogdor_island.js";

class FakeClassList {
  constructor() {
    this.values = new Set();
  }

  add(...names) {
    for (const name of names) this.values.add(name);
  }

  toggle(name, force) {
    const enabled = force === undefined ? !this.values.has(name) : Boolean(force);
    if (enabled) {
      this.values.add(name);
      return true;
    }
    this.values.delete(name);
    return false;
  }

  contains(name) {
    return this.values.has(name);
  }
}

class FakeElement {
  constructor({ id = "", dataset = {}, disabled = false, closestMap = {} } = {}) {
    this.id = id;
    this.dataset = dataset;
    this.disabled = disabled;
    this.closestMap = closestMap;
    this.classList = new FakeClassList();
    this.style = {};
    this.attributes = new Map();
    this.listeners = [];
    this.innerHTML = "";
    this.textContent = "";
    this.contained = new Set();
    this.queryAll = new Map();
  }

  addEventListener(eventName, listener, options) {
    this.listeners.push({ eventName, listener, options });
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  getAttribute(name) {
    return this.attributes.get(name) ?? null;
  }

  closest(selector) {
    return this.closestMap[selector] ?? null;
  }

  contains(target) {
    return target === this || this.contained.has(target);
  }

  querySelector() {
    return null;
  }

  querySelectorAll(selector) {
    return this.queryAll.get(selector) ?? [];
  }
}

function listenerFor(target, eventName) {
  const binding = target.listeners.find((item) => item.eventName === eventName);
  assert.ok(binding, `missing ${eventName} listener`);
  return binding.listener;
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

function rawSession(overrides = {}) {
  return {
    session_id: "agent-1",
    tmux_name: "Agent One",
    state: "attention",
    rest_state: "active",
    cwd: "/workspace/swimmers",
    thought: "needs operator response before launch",
    action_cues: [{ kind: "awaiting_user" }],
    ...overrides,
  };
}

function surfaceSession(session) {
  return {
    sessionId: session.session_id,
    name: session.tmux_name,
    state: session.state,
    restLabel: session.rest_state,
    fullCwd: session.cwd,
    cwdLabel: "opensource/swimmers",
    repoKey: session.cwd,
    repoLabel: "swimmers",
    targetKey: session.targetKey || "local",
    targetLabel: session.targetLabel || "local",
    stateKey: String(session.state || "unknown").toLowerCase(),
    readinessKey: session.readinessKey || "needs_attention",
    transportKey: session.transportKey || "healthy",
    transportLabel: session.transportLabel || "healthy",
    thoughtLabel: session.thought,
    actionCues: session.action_cues,
    operatorPressure: { score: 82, reason: "awaiting user", glyph: "!" },
    batchSendSessionIds: [],
    commitCandidate: true,
    trogdorAwaitingUser: true,
    trogdorBurnt: false,
    trogdorDismissed: false,
    trogdorSwordsmanVisible: true,
  };
}

function islandRuntime(overrides = {}) {
  const surface = new FakeElement({ id: TROGDOR_ATLAS_ISLAND_ID });
  const launcher = new FakeElement({ id: "trogdor-launcher" });
  const body = new FakeElement({ id: "body" });
  const calls = [];
  const state = {
    sessions: [rawSession()],
    trogdorAtlasOpen: true,
    trogdorSurfaceSignature: "",
    trogdorWpm: 225,
    trogdorReading: true,
    trogdorReadProgress: {},
    trogdorReaderTimer: null,
    hoveredTrogdorSessionId: "agent-1",
    activeSheet: null,
    readOnly: false,
    ...overrides.state,
  };
  const runtime = {
    state,
    el: { trogdorSurface: surface, trogdorLauncher: launcher },
    ElementClass: FakeElement,
    documentRef: { body },
    windowRef: {
      setInterval(callback, delay) {
        calls.push(["setInterval", delay]);
        return { callback, delay };
      },
      clearInterval(timer) {
        calls.push(["clearInterval", timer]);
      },
    },
    surfaceSession,
    currentTrogdorSurfaceSession() {
      const current = state.sessions.find((session) => session.session_id === state.hoveredTrogdorSessionId);
      return current ? surfaceSession(current) : null;
    },
    trogdorSessionCanRead: () => true,
    trogdorClawgReadComplete: () => false,
    trogdorReaderWordIndex: () => 1,
    startTrogdorReaderForSession(session) {
      calls.push(["startReader", session.sessionId]);
    },
    renderHudSurface() {
      calls.push(["renderHud"]);
    },
    setUtilityStatus(message, muted, timeoutMs) {
      calls.push(["utility", message, muted, timeoutMs]);
    },
    handleSurfaceAction(zone) {
      calls.push(["action", zone]);
    },
    openTrogdorAgentTerminal(sessionId) {
      calls.push(["openTerminal", sessionId]);
    },
    openTrogdorAtlas() {
      calls.push(["openAtlas"]);
    },
    ...overrides.runtime,
  };

  return { body, calls, launcher, runtime, state, surface };
}

test("Trogdor atlas island host preserves the stable React container contract", () => {
  const element = createTrogdorAtlasIslandElement((type, props) => ({ type, props }));

  assert.equal(TROGDOR_ATLAS_ISLAND_ID, "trogdor-surface");
  assert.equal(TROGDOR_ATLAS_ISLAND_CLASS_NAME, "trogdor-surface hidden");
  assert.equal(TROGDOR_ATLAS_ISLAND_ARIA_LABEL, "Trogdor repository atlas");
  assert.equal(element.type, "section");
  assert.deepEqual(element.props, TROGDOR_ATLAS_ISLAND_PROPS);
  assert.equal(element.props.id, "trogdor-surface");
  assert.equal(element.props.className, "trogdor-surface hidden");
  assert.equal(element.props["aria-label"], "Trogdor repository atlas");
  assert.throws(() => createTrogdorAtlasIslandElement(null), /createElement function/);
});

test("Trogdor atlas island delegates visible render output through existing helpers", () => {
  const { body, launcher, runtime, state, surface } = islandRuntime();
  const island = createTrogdorAtlasIsland(runtime);

  island.renderTrogdorSurface();

  assert.equal(surface.classList.contains("hidden"), false);
  assert.equal(surface.getAttribute("aria-hidden"), "false");
  assert.equal(surface.style.display, "");
  assert.equal(launcher.classList.contains("hidden"), true);
  assert.equal(body.classList.contains("trogdor-mode"), true);
  assert.equal(state.trogdorSurfaceSignature.length > 0, true);

  const html = surface.innerHTML;
  assert.match(html, /class="trogdor-frame"/);
  assert.match(html, /class="trogdor-topbar"/);
  assert.match(html, /class="trogdor-world"/);
  assert.match(html, /class="trogdor-bottombar"/);
  assert.match(html, /data-trogdor-reader="true"/);
  assert.match(html, /data-trogdor-agent="true"/);
  assert.match(html, /data-session-id="agent-1"/);
  assert.match(html, /class="[^"]*trogdor-agent[^"]*is-hovered/);
  assert.match(html, /data-action="trogdor_read_toggle">pause/);
  assert.match(html, /data-action="trogdor_wpm_down">-25/);
  assert.match(html, /data-trogdor-wpm-value="true">225 wpm/);
  assert.match(html, /data-action="trogdor_wpm_up">\+25/);
  assert.match(html, /data-action="focus_terminal"/);
  assert.match(html, /data-action="open_create"/);
  assert.match(html, /data-action="open_config"/);
  assert.match(html, /data-action="open_native"/);
  assert.match(html, /data-action="open_auth"/);
  assert.match(html, /data-action="refresh"/);
  assert.ok(html.indexOf("trogdor-topbar") < html.indexOf("trogdor-world"));
  assert.ok(html.indexOf("trogdor-world") < html.indexOf("trogdor-bottombar"));
});

test("Trogdor atlas island applies the active fleet target filter", () => {
  const { runtime, state, surface } = islandRuntime({
    state: {
      sessions: [
        rawSession({ session_id: "local", tmux_name: "Local", targetKey: "local", targetLabel: "local" }),
        rawSession({
          session_id: "remote",
          tmux_name: "Remote",
          targetKey: "skillbox",
          targetLabel: "Skillbox devbox",
          cwd: "/srv/skillbox/repos/swimmers",
        }),
      ],
      fleetFilter: { kind: "target", key: "skillbox" },
      hoveredTrogdorSessionId: "remote",
    },
  });
  const island = createTrogdorAtlasIsland(runtime);

  island.renderTrogdorSurface();

  assert.match(surface.innerHTML, /data-session-id="remote"/);
  assert.doesNotMatch(surface.innerHTML, /data-session-id="local"/);
  assert.match(surface.innerHTML, /Skillbox devbox/);
  assert.equal(state.trogdorSurfaceSignature.includes("remote"), true);
  assert.equal(state.trogdorSurfaceSignature.includes("local"), false);
});

test("Trogdor atlas island keeps pointer, focus, and action dispatch separate from terminal capture", async () => {
  const { calls, launcher, runtime, state, surface } = islandRuntime({
    state: {
      hoveredTrogdorSessionId: null,
      trogdorReading: false,
    },
  });
  const island = createTrogdorAtlasIsland(runtime);
  const agent = new FakeElement({ dataset: { sessionId: "agent-1" } });
  surface.queryAll.set("[data-trogdor-agent]", [agent]);
  const agentTarget = new FakeElement({ closestMap: { "[data-trogdor-agent]": agent } });
  const wpmButton = new FakeElement({ dataset: { action: "trogdor_wpm_up" } });
  const actionTarget = new FakeElement({ closestMap: { "button[data-action]": wpmButton } });
  const focusOutside = new FakeElement();

  island.bindTrogdorEvents();

  const launcherClick = fakeEvent(launcher);
  listenerFor(launcher, "click")(launcherClick);
  assert.deepEqual(launcherClick.calls, ["preventDefault"]);
  assert.deepEqual(calls.at(-1), ["openAtlas"]);

  const pointerDown = fakeEvent(agentTarget);
  listenerFor(surface, "pointerdown")(pointerDown);
  assert.deepEqual(pointerDown.calls, ["preventDefault", "stopPropagation"]);
  assert.deepEqual(calls.at(-1), ["openTerminal", "agent-1"]);

  const clickAction = fakeEvent(actionTarget);
  listenerFor(surface, "click")(clickAction);
  assert.deepEqual(clickAction.calls, ["preventDefault", "stopPropagation"]);
  assert.deepEqual(calls.at(-1), ["action", { type: "action", actionId: "trogdor_wpm_up" }]);

  listenerFor(surface, "mouseover")(fakeEvent(agentTarget));
  assert.equal(state.hoveredTrogdorSessionId, "agent-1");
  assert.equal(agent.classList.contains("is-hovered"), true);
  assert.deepEqual(calls.at(-3), ["startReader", "agent-1"]);
  assert.equal(calls.at(-2)[0], "utility");
  assert.deepEqual(calls.at(-1), ["renderHud"]);

  listenerFor(surface, "focusout")(fakeEvent(agentTarget, { relatedTarget: focusOutside }));
  assert.equal(state.hoveredTrogdorSessionId, null);
  assert.equal(agent.classList.contains("is-hovered"), false);

  await island.handleTrogdorDomAction(new FakeElement({ dataset: { action: "trogdor_read_toggle" } }));
  assert.deepEqual(calls.at(-1), ["action", { type: "action", actionId: "trogdor_read_toggle" }]);
});
