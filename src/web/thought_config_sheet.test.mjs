import test from "node:test";
import assert from "node:assert/strict";

import {
  createThoughtConfigSheetController,
  fallbackThoughtBackendMetadata,
  normalizeBackendKey,
  normalizeThoughtModelForBackend,
  selectedThoughtBackendMetadata,
  thoughtBackendMetadata,
} from "./thought_config_sheet.js";

function classes() {
  const values = new Set();
  return {
    contains: (name) => values.has(name),
    toggle: (name, force) => {
      if (force) {
        values.add(name);
      } else {
        values.delete(name);
      }
      return Boolean(force);
    },
  };
}

function element(value = "") {
  return {
    value,
    checked: false,
    textContent: "",
    classList: classes(),
  };
}

function listElement() {
  const node = element();
  node.children = [];
  Object.defineProperty(node, "innerHTML", {
    get() {
      return "";
    },
    set() {
      node.children = [];
    },
  });
  node.appendChild = (child) => {
    node.children.push(child);
    if (child.selected) {
      node.value = child.value;
    }
  };
  return node;
}

function fixture(overrides = {}) {
  const state = {
    thoughtConfig: {
      loading: false,
      config: null,
      ui: null,
      result: "",
      error: "",
    },
  };
  const el = {
    thoughtConfigBackend: listElement(),
    thoughtConfigDaemon: element(),
    thoughtConfigEnabled: element(),
    thoughtConfigHint: element(),
    thoughtConfigModel: element(),
    thoughtConfigModelPresets: listElement(),
    thoughtConfigResult: element(),
    thoughtConfigSummary: element(),
  };
  const calls = {
    fetches: [],
    refreshSessions: 0,
    sync: 0,
  };
  const responses = overrides.responses ?? [];
  const controller = createThoughtConfigSheetController({
    state,
    el,
    apiFetch: async (path, init = {}) => {
      calls.fetches.push([path, init]);
      const next = responses.shift();
      if (next instanceof Error) {
        throw next;
      }
      return {
        json: async () => next ?? {},
      };
    },
    refreshSessions: async () => {
      calls.refreshSessions += 1;
    },
    syncSheetActionAvailability: () => {
      calls.sync += 1;
    },
    documentRef: {
      createElement: (tagName) => ({ tagName, value: "", textContent: "", selected: false }),
    },
  });
  return { calls, controller, el, state };
}

test("backend and model normalization preserve backend aliases and OpenRouter filtering", () => {
  assert.equal(normalizeBackendKey(" claude-cli "), "grok");
  assert.equal(normalizeBackendKey("codex_cli"), "grok");
  assert.equal(normalizeBackendKey(" OpenRouter "), "openrouter");

  assert.equal(normalizeThoughtModelForBackend("openrouter", "router/model"), "router/model");
  assert.equal(normalizeThoughtModelForBackend("openrouter", "local-default"), "");
  assert.equal(normalizeThoughtModelForBackend("grok", " grok-model "), "grok-model");
  assert.equal(normalizeThoughtModelForBackend("", "anything"), "");
});

test("metadata helpers preserve fallback presets and UI-provided backend metadata", () => {
  const fallback = fallbackThoughtBackendMetadata();
  assert.deepEqual(fallback.map((entry) => entry.key), ["", "openrouter", "grok"]);
  assert.deepEqual(fallback.find((entry) => entry.key === "openrouter").model_presets.slice(0, 2), [
    "",
    "openrouter/free",
  ]);

  const thoughtConfig = {
    config: { backend: "claude" },
    ui: { backends: [{ key: "grok", label: "Grok CLI" }] },
  };
  assert.deepEqual(thoughtBackendMetadata(thoughtConfig), thoughtConfig.ui.backends);
  assert.equal(selectedThoughtBackendMetadata(thoughtConfig).label, "Grok CLI");
});

test("applyToForm normalizes config, renders presets, daemon defaults, and action availability", () => {
  const { calls, controller, el, state } = fixture();

  controller.applyToForm({
    config: { enabled: false, backend: "claude-cli", model: " grok-fast " },
    daemon_defaults: { backend: "codex", model: "daemon-model" },
  });

  assert.deepEqual(state.thoughtConfig.config, {
    enabled: false,
    backend: "grok",
    model: "grok-fast",
  });
  assert.equal(el.thoughtConfigEnabled.checked, false);
  assert.equal(el.thoughtConfigBackend.value, "grok");
  assert.equal(el.thoughtConfigModel.value, "grok-fast");
  assert.equal(el.thoughtConfigSummary.textContent, "grok backend selected.");
  assert.equal(el.thoughtConfigDaemon.textContent, "daemon default: grok / daemon-model");
  assert.equal(el.thoughtConfigHint.textContent, "uses Grok CLI default unless a model is set");
  assert.deepEqual(el.thoughtConfigModelPresets.children.map((option) => option.value), [""]);
  assert.equal(calls.sync, 1);
});

test("draft and backend change preserve form edits and model preset behavior", () => {
  const { calls, controller, el, state } = fixture();
  state.thoughtConfig.config = { enabled: true, backend: "grok", model: "old", keep: "field" };
  el.thoughtConfigEnabled.checked = false;
  el.thoughtConfigBackend.value = " openrouter ";
  el.thoughtConfigModel.value = "not-a-router-model";

  assert.deepEqual(controller.draft(), {
    enabled: false,
    backend: "openrouter",
    model: "not-a-router-model",
    keep: "field",
  });

  controller.handleBackendChange();
  assert.equal(el.thoughtConfigModel.value, "");
  assert.equal(el.thoughtConfigHint.textContent, "presets: auto  router  cached free models");
  assert.deepEqual(el.thoughtConfigModelPresets.children.map((option) => option.value).slice(0, 2), [
    "",
    "openrouter/free",
  ]);
  assert.equal(calls.sync, 1);
});

test("refresh loads config endpoint, clears loading, and updates result text", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [{
      config: { enabled: true, backend: "openrouter", model: "openrouter/free" },
    }],
  });

  await controller.refresh();

  assert.equal(calls.fetches[0][0], "/v1/thought-config");
  assert.equal(state.thoughtConfig.loading, false);
  assert.equal(state.thoughtConfig.result, "Thought config loaded.");
  assert.equal(el.thoughtConfigResult.textContent, "Thought config loaded.");
  assert.equal(el.thoughtConfigResult.classList.contains("error"), false);
  assert.equal(calls.sync, 2);
});

test("test posts draft config and reports probe result details", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [{ ok: true, llm_calls: 2, message: "Probe ok", last_backend_error: "minor" }],
  });
  state.thoughtConfig.config = { enabled: true, backend: "grok", model: "old" };
  el.thoughtConfigEnabled.checked = true;
  el.thoughtConfigBackend.value = "grok";
  el.thoughtConfigModel.value = "grok-new";

  await controller.test();

  assert.equal(calls.fetches[0][0], "/v1/thought-config/test");
  assert.equal(calls.fetches[0][1].method, "POST");
  assert.deepEqual(JSON.parse(calls.fetches[0][1].body), {
    enabled: true,
    backend: "grok",
    model: "grok-new",
  });
  assert.match(state.thoughtConfig.result, /^Probe ok\nok: true\nllm_calls: 2\nbackend error: minor$/);
  assert.equal(el.thoughtConfigResult.classList.contains("error"), false);
  assert.equal(state.thoughtConfig.loading, false);
});

test("save puts draft config, refreshes sessions, and preserves save result state", async () => {
  const { calls, controller, el, state } = fixture({ responses: [{}] });
  state.thoughtConfig.config = { enabled: true, backend: "grok", model: "old" };
  el.thoughtConfigEnabled.checked = false;
  el.thoughtConfigBackend.value = "grok";
  el.thoughtConfigModel.value = "grok-fast";

  await controller.save();

  assert.equal(calls.fetches[0][0], "/v1/thought-config");
  assert.equal(calls.fetches[0][1].method, "PUT");
  assert.deepEqual(JSON.parse(calls.fetches[0][1].body), {
    enabled: false,
    backend: "grok",
    model: "grok-fast",
  });
  assert.deepEqual(state.thoughtConfig.config, {
    enabled: false,
    backend: "grok",
    model: "grok-fast",
  });
  assert.equal(state.thoughtConfig.result, "Thought config saved.");
  assert.equal(calls.refreshSessions, 1);
  assert.equal(state.thoughtConfig.loading, false);
});

test("load and save errors set result error class without refreshing sessions", async () => {
  const load = fixture({ responses: [new Error("offline")] });
  await load.controller.refresh();
  assert.equal(load.state.thoughtConfig.error, "Failed to load thought config: offline");
  assert.equal(load.el.thoughtConfigResult.classList.contains("error"), true);
  assert.equal(load.state.thoughtConfig.loading, false);

  const save = fixture({ responses: [new Error("denied")] });
  save.state.thoughtConfig.config = { enabled: true, backend: "grok", model: "" };
  save.el.thoughtConfigBackend.value = "grok";
  await save.controller.save();
  assert.equal(save.state.thoughtConfig.error, "Thought config save failed: denied");
  assert.equal(save.calls.refreshSessions, 0);
  assert.equal(save.el.thoughtConfigResult.classList.contains("error"), true);
});
