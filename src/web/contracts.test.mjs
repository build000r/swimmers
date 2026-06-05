import test from "node:test";
import assert from "node:assert/strict";

import {
  normalizeBootPayload,
  normalizeDirListResponse,
  normalizeMermaidArtifactResponse,
  normalizeNativeDesktopStatusResponse,
  normalizeSessionListResponse,
  normalizeSurfaceModel,
  normalizeTerminalServerFrame,
  normalizeThoughtConfigResponse,
  normalizeWorkbenchWidgetResults,
} from "./contracts.js";

test("normalizeBootPayload preserves boot fields and tolerates malformed asset info", () => {
  assert.deepEqual(normalizeBootPayload({
    franken_term_available: true,
    franken_term_js_url: "/assets/frankenterm/FrankenTerm.js",
    franken_term_wasm_url: "/assets/frankenterm/FrankenTerm_bg.wasm",
    franken_term_font_url: "/assets/frankenterm/pragmasevka-nf-subset.woff2",
    franken_term_asset_info: {
      js: { route: "/js", size_bytes: "123", checksum: "abc" },
      wasm: { route: "/wasm", size_bytes: 456, checksum: "def" },
      font: null,
    },
    follow_published_selection: true,
    focus_layout: true,
  }), {
    franken_term_available: true,
    franken_term_js_url: "/assets/frankenterm/FrankenTerm.js",
    franken_term_wasm_url: "/assets/frankenterm/FrankenTerm_bg.wasm",
    franken_term_font_url: "/assets/frankenterm/pragmasevka-nf-subset.woff2",
    franken_term_asset_info: {
      js: { route: "/js", size_bytes: 123, checksum: "abc" },
      wasm: { route: "/wasm", size_bytes: 456, checksum: "def" },
      font: null,
    },
    follow_published_selection: true,
    focus_layout: true,
  });

  assert.equal(normalizeBootPayload({ franken_term_asset_info: { js: {} } }).franken_term_asset_info, null);
  assert.equal(normalizeBootPayload({ franken_term_available: "yes" }).franken_term_available, false);
});

test("normalizeSessionListResponse preserves SessionSummary-derived web fields with tolerant defaults", () => {
  const payload = normalizeSessionListResponse({
    version: "7",
    repo_themes: { theme1: { accent: "blue" } },
    sessions: [
      {
        session_id: "agent-1",
        tmux_name: "",
        state: "attention",
        state_evidence: { cause: "awaiting_user_input", confidence: "high" },
        cwd: "/srv/repos/app",
        tool: "codex",
        token_count: "55",
        context_limit: 100,
        thought: null,
        commit_candidate: true,
        action_cues: [
          { kind: "awaiting_user", evidence: ["prompted", 42] },
          "ignored",
        ],
        attached_clients: 2,
        transport_health: "degraded",
        last_activity_at: "2026-06-05T00:00:00Z",
      },
      null,
    ],
  });

  assert.equal(payload.version, 7);
  assert.equal(payload.sessions.length, 1);
  assert.deepEqual(payload.sessions[0], {
    session_id: "agent-1",
    tmux_name: "agent-1",
    state: "attention",
    current_command: null,
    state_evidence: {
      cause: "awaiting_user_input",
      observed_at: null,
      confidence: "high",
    },
    cwd: "/srv/repos/app",
    tool: "codex",
    token_count: 55,
    context_limit: 100,
    thought: null,
    thought_state: "holding",
    thought_source: "carry_forward",
    thought_updated_at: null,
    rest_state: "active",
    commit_candidate: true,
    action_cues: [{
      kind: "awaiting_user",
      status: "",
      source: "",
      confidence: "",
      evidence: ["prompted", "42"],
    }],
    objective_changed_at: null,
    last_skill: null,
    is_stale: false,
    attached_clients: 2,
    stale_attached_clients: 0,
    transport_health: "degraded",
    last_activity_at: "2026-06-05T00:00:00Z",
    repo_theme_id: null,
    batch: null,
  });
});

test("normalizeTerminalServerFrame preserves discriminated frames and malformed fallbacks", () => {
  const ready = normalizeTerminalServerFrame({
    type: "ready",
    sessionId: "agent-1",
    readOnly: true,
    replay: { latestSeq: "9", windowStartSeq: 2, resumeFromSeq: "4" },
    protocol: { output: "framed" },
    summary: { session_id: "agent-1", state: "idle" },
  });

  assert.equal(ready.type, "ready");
  assert.equal(ready.replay.latestSeq, 9);
  assert.equal(ready.summary.session_id, "agent-1");
  assert.deepEqual(normalizeTerminalServerFrame({ type: "overloaded", retry_after_ms: "2500" }), {
    type: "overloaded",
    retry_after_ms: "2500",
    retryAfterMs: 2500,
  });
  assert.deepEqual(normalizeTerminalServerFrame("not-json-object"), {
    type: "unknown",
    raw: "not-json-object",
  });
});

test("normalizeDirListResponse preserves directory entries and launch target fields", () => {
  const payload = normalizeDirListResponse({
    path: "/srv/repos",
    overlay_label: "main",
    groups: [" work ", ""],
    launch_targets: [{
      id: "remote",
      label: "Remote",
      kind: "ssh",
      base_url: "https://example.test",
      auth_token_env: null,
      path_mappings: [{ local_prefix: "/srv", remote_prefix: "/home" }],
    }],
    default_launch_target: "remote",
    entries: [{
      name: "app",
      has_children: true,
      is_running: true,
      repo_dirty: false,
      group: "work",
      groups: ["work", 7],
      full_path: "/srv/repos/app",
      has_restart: true,
      open_url: "http://localhost:5173",
    }],
  });

  assert.equal(payload.entries[0].full_path, "/srv/repos/app");
  assert.deepEqual(payload.entries[0].groups, ["work", "7"]);
  assert.equal(payload.launch_targets[0].path_mappings[0].remote_prefix, "/home");
});

test("normalizeMermaidArtifactResponse keeps optional fields browser-tolerant", () => {
  const artifact = normalizeMermaidArtifactResponse({
    session_id: "agent-1",
    available: true,
    path: "/tmp/schema.mmd",
    updated_at: null,
    source: "graph TD",
    error: undefined,
    slice_name: "slice",
    plan_files: ["plan.md", "", "../secret.md", 42],
  });

  assert.deepEqual(artifact, {
    session_id: "agent-1",
    available: true,
    path: "/tmp/schema.mmd",
    updated_at: null,
    source: "graph TD",
    error: null,
    slice_name: "slice",
    plan_files: ["plan.md", "../secret.md", "42"],
  });
});

test("native and thought config contracts preserve flattened backend responses", () => {
  assert.deepEqual(normalizeNativeDesktopStatusResponse({
    supported: true,
    platform: "darwin",
    app_id: "ghostty",
    ghostty_mode: "window",
  }), {
    supported: true,
    platform: "darwin",
    app_id: "ghostty",
    ghostty_mode: "window",
    app: null,
    reason: null,
  });

  assert.deepEqual(normalizeThoughtConfigResponse({
    enabled: false,
    backend: "grok",
    model: "fast",
    cadence_hot_ms: "15000",
    daemon_defaults: { backend: "openrouter", model: "openrouter/free" },
    ui: { backends: [{ key: "grok", label: "Grok", model_presets: [""] }] },
    version: "3",
  }), {
    enabled: false,
    backend: "grok",
    model: "fast",
    cadence_hot_ms: 15000,
    cadence_warm_ms: 0,
    cadence_cold_ms: 0,
    agent_prompt: null,
    terminal_prompt: null,
    daemon_defaults: {
      backend: "openrouter",
      model: "openrouter/free",
      agent_prompt: "",
      terminal_prompt: "",
    },
    ui: {
      backends: [{
        key: "grok",
        label: "Grok",
        model_presets_hint: "",
        model_presets: [""],
      }],
    },
    version: 3,
  });
});

test("normalizeWorkbenchWidgetResults normalizes workbench records without touching rejected results", () => {
  const rejected = { status: "rejected", reason: new Error("offline") };
  const results = normalizeWorkbenchWidgetResults({
    timelineResult: {
      status: "fulfilled",
      value: {
        session_id: "agent-1",
        available: true,
        events: [{ id: "evt-1", kind: "task", title: "Task", summary: "Do it" }],
      },
    },
    transcriptResult: {
      status: "fulfilled",
      value: {
        session_id: "agent-1",
        available: true,
        next_cursor: "10",
        records: [{ id: "rec-1", kind: "message", byte_start: "4", raw: "{\"ok\":true}" }],
      },
    },
    diffResult: rejected,
  });

  assert.equal(results.timelineResult.value.events[0].source, "");
  assert.equal(results.transcriptResult.value.records[0].byte_start, 4);
  assert.equal(results.transcriptResult.value.records[0].truncated, false);
  assert.equal(results.diffResult, rejected);
});

test("normalizeSurfaceModel preserves Trogdor view model fields and null current session", () => {
  const model = normalizeSurfaceModel({
    sessions: [{
      sessionId: "agent-1",
      name: "agent-1",
      state: "busy",
      restLabel: "sleeping",
      actionCues: [{ kind: "commit_ready" }],
      batchSendSessionIds: ["agent-1", null],
    }],
    currentSession: null,
    selectedSessionId: undefined,
    publishedSessionId: "agent-1",
    trogdorAtlasOpen: true,
    trogdorReading: "yes",
  });

  assert.equal(model.sessions[0].sessionId, "agent-1");
  assert.equal(model.sessions[0].actionCues[0].kind, "commit_ready");
  assert.deepEqual(model.sessions[0].batchSendSessionIds, ["agent-1"]);
  assert.equal(model.currentSession, null);
  assert.equal(model.selectedSessionId, null);
  assert.equal(model.publishedSessionId, "agent-1");
  assert.equal(model.trogdorAtlasOpen, true);
  assert.equal(model.trogdorReading, false);
});
