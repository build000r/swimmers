import test from "node:test";
import assert from "node:assert/strict";

import {
  createNativeDesktopSheetController,
  currentNativeModeLabel,
  formatNativeStatus,
  formatNativeStatusCopy,
  remoteNativeHandoffAvailable,
  remoteNativeHandoffMessage,
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
    environments: overrides.environments ?? [],
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

test("remote handoff copy uses non-secret session environment metadata", () => {
  const session = {
    session_id: "skillbox::sess_7",
    tmux_name: "[Skillbox] 7",
    cwd: "/Users/b/repos/swimmers",
    environment: {
      scope: "remote",
      target_id: "skillbox",
      target_label: "Skillbox devbox",
      display_host: "Skillbox devbox",
      launch_source: "remote_swimmers_api",
      remote_session_id: "sess_7",
      remote_cwd: "/srv/skillbox/repos/swimmers",
    },
  };
  const environments = [{
    id: "skillbox",
    backend_mode: "remote_swimmers_api",
  }];

  assert.equal(remoteNativeHandoffAvailable(session), true);
  assert.equal(remoteNativeHandoffAvailable({ session_id: "local" }), false);
  assert.equal(
    remoteNativeHandoffMessage(session, environments),
    [
      "Remote handoff: no SSH attach command is configured for this remote terminal.",
      "Add ssh_alias to Skillbox devbox (skillbox) to attach.",
      "backend: remote Swimmers API",
      "remote session: sess_7",
      "remote cwd: /srv/skillbox/repos/swimmers",
    ].join("\n"),
  );
  assert.equal(
    formatNativeStatusCopy({ supported: true, app: "ghostty" }, session, environments),
    remoteNativeHandoffMessage(session, environments),
  );
});

test("remote handoff detection ignores '::' in a local session id", () => {
  // A local tmux session name can legitimately contain "::"; the handoff must
  // not treat that as a remote session and refuse to open a native terminal.
  assert.equal(
    remoteNativeHandoffAvailable({ session_id: "a::b", environment: { scope: "local" } }),
    false,
  );
  // No environment at all (defaults to local scope) is likewise not remote.
  assert.equal(remoteNativeHandoffAvailable({ session_id: "a::b" }), false);
  // A default local target_id is not a remote signal either.
  assert.equal(
    remoteNativeHandoffAvailable({ session_id: "a::b", environment: { target_id: "local" } }),
    false,
  );
  // A real remote target_id (without scope) still flags as remote.
  assert.equal(
    remoteNativeHandoffAvailable({ session_id: "a::b", environment: { target_id: "skillbox" } }),
    true,
  );
});

test("remote handoff availability trims scope metadata", async () => {
  const { calls, controller, state } = fixture({
    environments: [{ id: "skillbox", backend_mode: "remote_swimmers_api" }],
    session: {
      session_id: "sess_7",
      environment: {
        scope: " remote ",
        target_id: "skillbox",
        target_label: "Skillbox devbox",
        remote_session_id: "sess_7",
        remote_cwd: "/srv/skillbox/repos/swimmers",
      },
    },
  });

  assert.equal(remoteNativeHandoffAvailable({
    session_id: "sess_7",
    environment: { scope: " remote " },
  }), true);

  await controller.openSelectedNativeSession();

  assert.equal(calls.fetches.length, 0);
  assert.match(state.nativeDesktop.error, /no SSH attach command is configured/);
  assert.match(state.nativeDesktop.error, /remote cwd: \/srv\/skillbox\/repos\/swimmers/);
});

test("remote handoff parses a namespaced session id for already-remote sessions and avoids false remote cwd labels", () => {
  const session = {
    session_id: "skillbox::sess_7",
    cwd: "/Users/b/repos/opensource/swimmers",
    environment: {
      // Detection is via scope; the "::" split only parses the remote session id
      // (sess_7) and target (skillbox) for the handoff copy below.
      scope: "remote",
      local_cwd: "/Users/b/repos/opensource/swimmers",
      canonical_cwd: "/Users/b/repos/opensource/swimmers",
    },
  };
  const environments = [{
    id: " skillbox ",
    label: "Skillbox devbox",
    backend_mode: "remote_swimmers_api",
  }];

  assert.equal(remoteNativeHandoffAvailable(session), true);
  assert.equal(
    remoteNativeHandoffMessage(session, environments),
    [
      "Remote handoff: no SSH attach command is configured for this remote terminal.",
      "Add ssh_alias to Skillbox devbox (skillbox) to attach.",
      "backend: remote Swimmers API",
      "remote session: sess_7",
      "local mapped cwd: /Users/b/repos/opensource/swimmers",
    ].join("\n"),
  );
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

test("renderNativeStatusForm surfaces remote backend handoff in status copy", () => {
  const { controller, el } = fixture({
    environments: [{ id: "skillbox", backend_mode: "remote_swimmers_api" }],
    session: {
      session_id: "skillbox::sess_7",
      environment: {
        scope: "remote",
        target_id: "skillbox",
        target_label: "Skillbox devbox",
        remote_session_id: "sess_7",
        remote_cwd: "/srv/repos/swimmers",
      },
    },
  });

  controller.renderNativeStatusForm({
    supported: false,
    reason: "unsupported host",
    app_id: "iterm",
  });

  assert.match(el.nativeStatusCopy.textContent, /no SSH attach command is configured/);
  assert.match(el.nativeStatusCopy.textContent, /backend: remote Swimmers API/);
  assert.match(el.nativeStatusCopy.textContent, /Skillbox devbox/);
  assert.match(el.nativeStatusCopy.textContent, /sess_7/);
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

test("native open and save skip the request when the same operation is already in flight", async () => {
  const open = fixture({
    session: { session_id: "sess_7" },
    responses: [{ session_id: "sess_7" }],
  });
  open.state.nativeDesktop.opening = true;
  await open.controller.openSelectedNativeSession();
  assert.equal(open.calls.fetches.length, 0, "no duplicate POST /v1/native/open");

  const save = fixture({ responses: [{}] });
  save.state.nativeDesktop.saving = true;
  await save.controller.saveNativeSettings();
  assert.equal(save.calls.fetches.length, 0, "no duplicate PUT /v1/native/app");
});

test("native open and save proceed while a status refresh holds the shared loading flag", async () => {
  // refreshNativeStatus sets state.nativeDesktop.loading on sheet-open; a click
  // on Open (or a save submit) during that fetch must NOT be silently dropped.
  const open = fixture({
    session: { session_id: "sess_7" },
    responses: [{ session_id: "sess_7", pane_id: "%9" }],
  });
  open.state.nativeDesktop.loading = true;
  await open.controller.openSelectedNativeSession();
  assert.equal(open.calls.fetches[0][0], "/v1/native/open", "open proceeds despite shared loading");
  assert.equal(open.state.nativeDesktop.result, "Opened sess_7 in native app (%9).");

  const save = fixture({ responses: [{ supported: true, app: "iterm" }] });
  save.el.nativeApp.value = "iterm";
  save.state.nativeDesktop.loading = true;
  await save.controller.saveNativeSettings();
  assert.equal(save.calls.fetches[0][0], "/v1/native/app", "save proceeds despite shared loading");
  assert.equal(save.state.nativeDesktop.result, "Native settings saved: iterm");
});

test("openSelectedNativeSession shows setup guidance when remote attach command is missing", async () => {
  const { calls, controller, el, state } = fixture({
    environments: [{ id: "skillbox", backend_mode: "remote_swimmers_api" }],
    session: {
      session_id: "skillbox::sess_7",
      tmux_name: "[Skillbox] 7",
      environment: {
        scope: "remote",
        target_id: "skillbox",
        target_label: "Skillbox devbox",
        display_host: "Skillbox devbox",
        launch_source: "remote_swimmers_api",
        remote_session_id: "sess_7",
        remote_cwd: "/srv/skillbox/repos/swimmers",
      },
    },
  });

  await controller.openSelectedNativeSession();

  assert.equal(calls.fetches.length, 0);
  assert.match(state.nativeDesktop.error, /no SSH attach command is configured/);
  assert.match(state.nativeDesktop.error, /backend: remote Swimmers API/);
  assert.match(state.nativeDesktop.error, /Skillbox devbox \(skillbox\)/);
  assert.match(state.nativeDesktop.error, /remote session: sess_7/);
  assert.match(state.nativeDesktop.error, /remote cwd: \/srv\/skillbox\/repos\/swimmers/);
  assert.equal(el.nativeStatusResult.classList.contains("error"), true);
  assert.equal(calls.sync, 1);
});

test("openSelectedNativeSession posts remote sessions when attach command is configured", async () => {
  const { calls, controller, el, state } = fixture({
    responses: [{ session_id: "skillbox::sess_7", pane_id: "%51" }],
    session: {
      session_id: "skillbox::sess_7",
      tmux_name: "[Skillbox] devbox-3",
      environment: {
        scope: "remote",
        target_id: "skillbox",
        remote_session_id: "sess_7",
        remote_attach_command:
          "exec ssh skillbox@skillbox-portfolio-devbox -t 'tmux attach-session -t =devbox-3'",
      },
    },
  });

  await controller.openSelectedNativeSession();

  assert.equal(calls.fetches[0][0], "/v1/native/open");
  assert.deepEqual(JSON.parse(calls.fetches[0][1].body), { session_id: "skillbox::sess_7" });
  assert.equal(state.nativeDesktop.result, "Opened skillbox::sess_7 in native app (%51).");
  assert.equal(el.nativeStatusResult.classList.contains("error"), false);
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
