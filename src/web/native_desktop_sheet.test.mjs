import test from "node:test";
import assert from "node:assert/strict";

import {
  createNativeDesktopSheetController,
  currentNativeModeLabel,
  formatNativeStatus,
} from "./native_desktop_sheet.js";

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
    disabled: false,
    textContent: "",
    classList: classes(),
  };
}

function fixture(overrides = {}) {
  const state = {
    nativeDesktop: {
      loading: false,
      status: null,
      result: "",
      error: "",
    },
  };
  const el = {
    nativeApp: element(),
    nativeMode: element(),
    nativeStatusCopy: element(),
    nativeStatusResult: element(),
  };
  const calls = {
    fetches: [],
    refreshSessions: 0,
    sync: 0,
  };
  const responses = overrides.responses ?? [];
  const controller = createNativeDesktopSheetController({
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
    currentSession: () => overrides.session ?? null,
    refreshSessions: async () => {
      calls.refreshSessions += 1;
    },
    syncSheetActionAvailability: () => {
      calls.sync += 1;
    },
  });
  return { calls, controller, el, state };
}

test("status formatting and current mode label preserve native copy", () => {
  assert.equal(formatNativeStatus(null), "Native status unavailable.");
  assert.equal(
    formatNativeStatus({ supported: false, reason: "unsupported platform" }),
    "Native open unavailable: unsupported platform",
  );
  assert.equal(
    formatNativeStatus({ supported: true, app: "ghostty", ghostty_mode: "New-Window" }),
    "Native open ready: ghostty / new-window",
  );
  assert.equal(currentNativeModeLabel({ status: null }), "swap");
  assert.equal(currentNativeModeLabel({ status: { ghosttyMode: "Tab" } }), "tab");
});

test("renderNativeStatusForm writes status, form fields, copy, diagnostics, and availability", () => {
  const { calls, controller, el, state } = fixture();

  controller.renderNativeStatusForm({
    supported: true,
    platform: "darwin",
    app: "ghostty",
    ghostty_mode: "Tab",
  });

  assert.deepEqual(state.nativeDesktop.status, {
    supported: true,
    platform: "darwin",
    app: "ghostty",
    ghostty_mode: "Tab",
  });
  assert.equal(el.nativeApp.value, "ghostty");
  assert.equal(el.nativeMode.value, "tab");
  assert.equal(el.nativeMode.disabled, false);
  assert.equal(el.nativeStatusCopy.textContent, "Native open ready: ghostty / tab");
  assert.equal(state.nativeDesktop.result, "supported: true\nplatform: darwin\napp: ghostty\nghostty mode: tab");
  assert.equal(el.nativeStatusResult.textContent, state.nativeDesktop.result);
  assert.equal(el.nativeStatusResult.classList.contains("error"), false);
  assert.equal(calls.sync, 1);

  controller.renderNativeStatusForm({ supported: false, reason: "unsupported host", app_id: "iterm" });
  assert.equal(el.nativeApp.value, "iterm");
  assert.equal(el.nativeMode.value, "swap");
  assert.equal(el.nativeMode.disabled, true);
  assert.equal(el.nativeStatusCopy.textContent, "Native open unavailable: unsupported host");
});

test("refreshNativeStatus loads status, replaces diagnostics with summary, and clears loading", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [{ supported: true, app: "iterm" }],
  });

  await controller.refreshNativeStatus();

  assert.equal(calls.fetches[0][0], "/v1/native/status");
  assert.equal(state.nativeDesktop.loading, false);
  assert.equal(state.nativeDesktop.result, "Native open ready: iterm");
  assert.equal(el.nativeStatusResult.textContent, "Native open ready: iterm");
  assert.equal(el.nativeStatusResult.classList.contains("error"), false);
  assert.equal(calls.sync, 2);
});

test("refreshNativeStatus reports load errors without leaving loading set", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [new Error("offline")],
  });

  await controller.refreshNativeStatus();

  assert.equal(calls.fetches[0][0], "/v1/native/status");
  assert.equal(state.nativeDesktop.loading, false);
  assert.equal(state.nativeDesktop.error, "Failed to load native status: offline");
  assert.equal(el.nativeStatusResult.classList.contains("error"), true);
  assert.equal(calls.sync, 1);
});

test("saveNativeSettings writes the app first and skips mode for non-Ghostty apps", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [{ supported: true, app: "iterm" }],
  });
  el.nativeApp.value = "iterm";
  el.nativeMode.value = "tab";

  await controller.saveNativeSettings();

  assert.equal(calls.fetches.length, 1);
  assert.equal(calls.fetches[0][0], "/v1/native/app");
  assert.equal(calls.fetches[0][1].method, "PUT");
  assert.deepEqual(JSON.parse(calls.fetches[0][1].body), { app: "iterm" });
  assert.equal(state.nativeDesktop.result, "Native settings saved: iterm");
  assert.equal(calls.refreshSessions, 1);
  assert.equal(state.nativeDesktop.loading, false);
  assert.equal(calls.sync, 2);
});

test("saveNativeSettings writes Ghostty mode after app and preserves save result", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [
      { supported: true, app: "ghostty", ghostty_mode: "swap" },
      { supported: true, app: "ghostty", ghostty_mode: "new-window" },
    ],
  });
  el.nativeApp.value = "ghostty";
  el.nativeMode.value = "new-window";

  await controller.saveNativeSettings();

  assert.deepEqual(calls.fetches.map(([path]) => path), ["/v1/native/app", "/v1/native/mode"]);
  assert.deepEqual(JSON.parse(calls.fetches[0][1].body), { app: "ghostty" });
  assert.deepEqual(JSON.parse(calls.fetches[1][1].body), { mode: "new-window" });
  assert.equal(state.nativeDesktop.result, "Native settings saved: ghostty / new-window");
  assert.equal(calls.refreshSessions, 1);
  assert.equal(calls.sync, 3);
});

test("saveNativeSettings reports errors and does not refresh sessions", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [new Error("denied")],
  });
  el.nativeApp.value = "ghostty";

  await controller.saveNativeSettings();

  assert.equal(calls.fetches[0][0], "/v1/native/app");
  assert.equal(calls.refreshSessions, 0);
  assert.equal(state.nativeDesktop.loading, false);
  assert.equal(state.nativeDesktop.error, "Failed to save native settings: denied");
  assert.equal(el.nativeStatusResult.classList.contains("error"), true);
  assert.equal(calls.sync, 1);
});

test("openSelectedNativeSession posts the selected session id and reports pane details", async () => {
  const { calls, controller, el, state } = fixture({
    session: { session_id: "sess_7" },
    responses: [{ session_id: "sess_7", pane_id: "%42" }],
  });

  await controller.openSelectedNativeSession();

  assert.equal(calls.fetches[0][0], "/v1/native/open");
  assert.equal(calls.fetches[0][1].method, "POST");
  assert.deepEqual(JSON.parse(calls.fetches[0][1].body), { session_id: "sess_7" });
  assert.equal(state.nativeDesktop.result, "Opened sess_7 in native app (%42).");
  assert.equal(el.nativeStatusResult.textContent, "Opened sess_7 in native app (%42).");
  assert.equal(calls.sync, 1);
});

test("openSelectedNativeSession is a no-op without a selected session", async () => {
  const { calls, controller, state } = fixture();

  await controller.openSelectedNativeSession();

  assert.equal(calls.fetches.length, 0);
  assert.equal(calls.sync, 0);
  assert.equal(state.nativeDesktop.result, "");
});

test("native form and field handlers preserve preventDefault and Ghostty mode availability", async () => {
  const { calls, controller, el } = fixture({
    responses: [{ supported: true, app: "iterm" }],
  });
  let prevented = false;
  el.nativeApp.value = "iterm";
  el.nativeMode.value = "swap";

  await controller.handleNativeFormSubmit({
    preventDefault() {
      prevented = true;
    },
  });

  assert.equal(prevented, true);
  assert.equal(calls.fetches[0][0], "/v1/native/app");

  el.nativeApp.value = "Ghostty";
  controller.handleNativeAppChange();
  assert.equal(el.nativeMode.disabled, false);

  el.nativeApp.value = "iterm";
  controller.handleNativeAppChange();
  assert.equal(el.nativeMode.disabled, true);
});
