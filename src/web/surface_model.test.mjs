import test from "node:test";
import assert from "node:assert/strict";

import {
  buildAttentionInbox,
  buildEnvironmentMatrix,
  buildSessionRailRows,
  buildSurfaceModel,
  surfaceSession,
} from "./surface_model.js";

function rawSession(overrides = {}) {
  return {
    session_id: "sess-1",
    tmux_name: "agent-one",
    state: "attention",
    state_evidence: {
      confidence: "high",
      observed_at: "2026-06-03T00:00:00Z",
      cause: "liveness_has_children",
    },
    rest_state: "active",
    transport_health: "healthy",
    tool: "codex",
    cwd: "/srv/skillbox/repos/swimmers",
    thought: "approve migration before commit",
    thought_updated_at: "2026-06-03T00:00:00Z",
    objective_changed_at: "2026-06-02T00:00:00Z",
    token_count: 10,
    context_limit: 100,
    last_skill: "describe",
    last_activity_at: "raw-time",
    current_command: "cargo test",
    attached_clients: 2,
    commit_candidate: true,
    action_cues: [{ kind: "awaiting_user" }],
    is_stale: true,
    ...overrides,
  };
}

function baseState(overrides = {}) {
  return {
    sessions: [rawSession()],
    currentCols: 120,
    currentRows: 36,
    followPublishedSelection: true,
    connectionLabel: "attached",
    connectionMuted: false,
    modeLabel: "operator",
    modeMuted: false,
    searchLabel: "Search idle",
    searchMuted: true,
    utilityLabel: "ready",
    utilityMuted: true,
    searchQuery: "needle",
    selectMode: true,
    readOnly: false,
    activeSheet: "send",
    hoveredLinkUrl: "https://example.test/repo",
    hoveredTrogdorSessionId: "sess-1",
    trogdorAtlasOpen: true,
    trogdorWpm: 225,
    trogdorReading: true,
    trogdorReaderStartIndex: 3,
    trogdorReaderStartedAt: 1000,
    trogdorDismissedClawgs: {},
    trogdorReadProgress: {},
    selectedSessionId: "sess-1",
    publishedSelection: {
      session_id: "sess-2",
      published_at: "published-raw",
    },
    terminal: {},
    ws: { readyState: 7 },
    ...overrides,
  };
}

test("surfaceSession preserves labels, pressure fields, and Trogdor enrichment", () => {
  const pressure = {
    pressure: { reason_kind: "awaiting_user", score: 91 },
    batch_send_session_ids: ["sess-1", "sess-2"],
    repo_key: "repo-key",
    repo_label: "swimmers",
  };

  const session = surfaceSession(rawSession({ action_cues: [] }), {
    operatorPressure: pressure,
    sessionBurnt: () => true,
  });

  assert.equal(session.sessionId, "sess-1");
  assert.equal(session.name, "agent-one");
  assert.equal(session.displayState, "attention");
  assert.equal(session.stateTrustLabel, "high observed liveness_has_children");
  assert.equal(session.cwdLabel, "repos/swimmers");
  assert.equal(session.fullCwd, "/srv/skillbox/repos/swimmers");
  assert.equal(session.thoughtLabel, "approve migration before commit");
  assert.equal(session.activityLabel, "raw-time");
  assert.equal(session.contextLabel, "10 / 100");
  assert.equal(session.skillLabel, "describe");
  assert.equal(session.lastActivityAt, "raw-time");
  assert.equal(session.attachedLabel, "2");
  assert.equal(session.commitCandidate, true);
  assert.deepEqual(session.operatorPressure, { reason_kind: "awaiting_user", score: 91 });
  assert.deepEqual(session.batchSendSessionIds, ["sess-1", "sess-2"]);
  assert.equal(session.repoKey, "repo-key");
  assert.equal(session.repoLabel, "swimmers");
  assert.equal(session.isStale, true);
  assert.equal(session.trogdorAwaitingUser, true);
  assert.equal(session.trogdorBurnt, true);
  assert.equal(session.trogdorSwordsmanVisible, true);
  assert.equal(session.clawgWordCount, 4);
});

test("surfaceSession marks low or unobserved state as uncertain and keeps detail thought full", () => {
  const longThought = `${"x".repeat(120)} final`;
  const raw = rawSession({
    state: "busy",
    state_evidence: { confidence: "medium", observed_at: "", cause: "summary_cache" },
    thought: longThought,
  });

  const summary = surfaceSession(raw);
  const detail = surfaceSession(raw, { detail: true });

  assert.equal(summary.displayState, "busy?");
  assert.equal(summary.stateTrustLabel, "medium unobserved summary_cache");
  assert.equal(summary.stateConfidence, "medium");
  assert.equal(summary.stateObserved, false);
  assert.equal(summary.thoughtLabel.length, 110);
  assert.match(summary.thoughtLabel, /\.\.\.$/);
  assert.equal(detail.thoughtLabel, longThought);
});

test("surfaceSession separates raw cwd from mapped canonical cwd and host label", () => {
  const session = surfaceSession(rawSession({
    cwd: "/srv/skillbox/repos/swimmers",
    environment: {
      scope: " remote ",
      target_id: " skillbox ",
      target_label: "Skillbox devbox",
      target_kind: "swimmers_api",
      display_host: "Skillbox devbox",
      remote_session_id: "remote-1",
      remote_cwd: "/srv/skillbox/repos/swimmers",
      local_cwd: "/Users/tester/repos/opensource/swimmers",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  }));

  assert.equal(session.fullCwd, "/srv/skillbox/repos/swimmers");
  assert.equal(session.canonicalCwd, "/Users/tester/repos/opensource/swimmers");
  assert.equal(session.launchCwd, "/Users/tester/repos/opensource/swimmers");
  assert.equal(session.launchTarget, "skillbox");
  assert.equal(session.cwdLabel, "opensource/swimmers @ Skillbox devbox");
  assert.equal(session.repoKey, "/Users/tester/repos/opensource/swimmers");
  assert.equal(session.repoLabel, "opensource/swimmers");
});

test("surfaceSession falls back honestly for unmapped remote cwd", () => {
  const session = surfaceSession(rawSession({
    cwd: "/srv/skillbox/repos/swimmers",
    environment: {
      scope: "remote",
      target_id: "skillbox",
      target_label: "Skillbox devbox",
      target_kind: "swimmers_api",
      display_host: "Skillbox devbox",
      remote_session_id: "remote-1",
      remote_cwd: "/srv/skillbox/repos/swimmers",
      local_cwd: null,
      canonical_cwd: "/srv/skillbox/repos/swimmers",
    },
  }));

  assert.equal(session.fullCwd, "/srv/skillbox/repos/swimmers");
  assert.equal(session.canonicalCwd, "/srv/skillbox/repos/swimmers");
  assert.equal(session.launchCwd, "");
  assert.equal(session.launchTarget, "");
  assert.equal(session.cwdLabel, "repos/swimmers @ Skillbox devbox");
  assert.equal(session.repoKey, "/srv/skillbox/repos/swimmers");
});

test("surfaceSession trims remote target identity before fleet grouping", () => {
  const session = surfaceSession(rawSession({
    cwd: "/srv/skillbox/repos/swimmers",
    environment: {
      scope: "remote",
      target_id: "   ",
      target_label: "  Skillbox devbox  ",
      target_kind: "swimmers_api",
      display_host: "   ",
      remote_session_id: "remote-1",
      remote_cwd: "/srv/skillbox/repos/swimmers",
      canonical_cwd: "/srv/skillbox/repos/swimmers",
    },
  }));

  assert.equal(session.targetKey, "Skillbox devbox");
  assert.equal(session.targetLabel, "Skillbox devbox");
  assert.equal(session.cwdLabel, "repos/swimmers @ Skillbox devbox");
});

test("surfaceSession exposes advisory metadata as passive external badges", () => {
  const session = surfaceSession(rawSession({
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
      advisory: [
        {
          source: "c0",
          label: "c0 group",
          value: "wave-a",
          status: "external",
          stale: true,
          group_key: "c0:wave-a",
          observed_at: "2026-06-05T00:00:00Z",
          freshness_ms: 120000,
        },
        { source: "ntm", label: "", value: "ignored", status: "external", stale: true },
      ],
    },
  }));

  assert.deepEqual(session.advisoryBadges, [{
    source: "c0",
    label: "c0 group",
    value: "wave-a",
    status: "external",
    stale: true,
    group_key: "c0:wave-a",
    observed_at: "2026-06-05T00:00:00Z",
    freshness_ms: 120000,
  }]);
  assert.equal(session.advisoryLabel, "c0 group: wave-a (external stale)");
});

test("surfaceSession only treats known action cues as attention readiness", () => {
  const quiet = surfaceSession(rawSession({
    state: "idle",
    rest_state: "active",
    commit_candidate: false,
    action_cues: [{ kind: "" }, { kind: "note" }],
  }));
  const actionable = surfaceSession(rawSession({
    state: "idle",
    rest_state: "active",
    commit_candidate: false,
    action_cues: [{ kind: " COMMIT_READY " }],
  }));

  assert.equal(quiet.readinessKey, "quiet");
  assert.equal(actionable.readinessKey, "needs_attention");
});

test("buildAttentionInbox keeps healthy actionable sessions ahead of degraded remote sessions", () => {
  const healthy = surfaceSession(rawSession({
    session_id: "healthy",
    tmux_name: "healthy",
    state: "attention",
    is_stale: false,
    transport_health: "healthy",
    last_activity_at: "2026-06-05T00:00:00Z",
  }), {
    operatorPressure: {
      pressure: { score: 45, reason_kind: "needs_input" },
    },
  });
  const degradedRemote = surfaceSession(rawSession({
    session_id: "skillbox::remote",
    tmux_name: "remote",
    state: "attention",
    is_stale: true,
    transport_health: "degraded",
    last_activity_at: "2026-06-05T00:10:00Z",
    environment: {
      scope: "remote",
      target_id: "skillbox",
      target_label: "Skillbox devbox",
      target_kind: "swimmers_api",
      display_host: "Skillbox devbox",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  }), {
    operatorPressure: {
      pressure: { score: 99, reason_kind: "awaiting_user" },
    },
  });
  const quiet = surfaceSession(rawSession({
    session_id: "quiet",
    state: "busy",
    action_cues: [],
    commit_candidate: false,
  }));

  const inbox = buildAttentionInbox([degradedRemote, quiet, healthy]);

  assert.deepEqual(inbox.map((session) => session.sessionId), ["healthy", "skillbox::remote"]);
});

test("buildSurfaceModel preserves selected/current session, terminal, reader, and HUD fields", () => {
  const selected = rawSession({ thought: `${"selected ".repeat(16)}done` });
  const state = baseState({
    sessions: [
      selected,
      rawSession({ session_id: "sess-2", tmux_name: "agent-two", cwd: "/tmp/other" }),
    ],
  });
  const pressureBySession = new Map([
    ["sess-1", {
      pressure: { reason_kind: "commit_ready", score: 70 },
      repo_label: "swimmers",
    }],
  ]);

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: true, franken_term_available: true },
    currentSession: () => selected,
    operatorPressureSnapshot: (sessionId) => pressureBySession.get(sessionId) || null,
    sessionBurnt: (session) => session.sessionId === "sess-2",
    normalizeSessionId: (sessionId) => String(sessionId || "").trim() || null,
    now: () => 1450,
    websocketOpen: 7,
  });

  assert.equal(model.cols, 120);
  assert.equal(model.rows, 36);
  assert.equal(model.focusLayout, true);
  assert.equal(model.followPublishedSelection, true);
  assert.equal(model.terminalReady, true);
  assert.equal(model.snapshotFallback, false);
  assert.equal(model.activeSheet, "send");
  assert.equal(model.hoveredLinkUrl, "https://example.test/repo");
  assert.equal(model.hoveredTrogdorSessionId, "sess-1");
  assert.equal(model.trogdorAtlasOpen, true);
  assert.equal(model.trogdorWpm, 225);
  assert.equal(model.trogdorReading, true);
  assert.equal(model.trogdorReaderStartIndex, 3);
  assert.equal(model.trogdorReaderElapsedMs, 450);
  assert.equal(model.selectedSessionId, "sess-1");
  assert.equal(model.publishedSessionId, "sess-2");
  assert.equal(model.publishedAtLabel, "published-raw");
  assert.equal(model.sessions.length, 2);
  assert.equal(model.attentionInboxCount, 2);
  assert.deepEqual(
    model.attentionInbox.map((session) => session.sessionId),
    ["sess-1", "sess-2"],
  );
  assert.equal(
    model.attentionInboxCount,
    model.filteredFleetLens.buckets.find((bucket) => bucket.kind === "readiness" && bucket.key === "needs_attention")?.count,
  );
  assert.equal(model.sessions[0].operatorPressure.reason_kind, "commit_ready");
  assert.equal(model.sessions[1].trogdorBurnt, true);
  assert.equal(model.currentSession.sessionId, "sess-1");
  assert.equal(model.currentSession.thoughtLabel, selected.thought);
  assert.notEqual(model.sessions[0].thoughtLabel, selected.thought);
});

test("buildSurfaceModel applies fleet lens filters without losing bucket counts", () => {
  const local = rawSession({
    session_id: "local",
    tmux_name: "local",
    cwd: "/Users/tester/repos/opensource/swimmers",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const remote = rawSession({
    session_id: "remote",
    tmux_name: "remote",
    cwd: "/srv/skillbox/repos/swimmers",
    state: "busy",
    transport_health: "degraded",
    environment: {
      scope: "remote",
      target_id: "skillbox",
      target_label: "Skillbox devbox",
      target_kind: "swimmers_api",
      display_host: "Skillbox devbox",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
      advisory: [{
        source: "manual",
        label: "capacity",
        value: "tight",
        status: "external",
        stale: true,
        group_key: "capacity:tight",
      }],
    },
  });
  const state = baseState({
    sessions: [local, remote],
    fleetFilter: { kind: "target", key: "skillbox" },
  });

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: false, franken_term_available: true },
    websocketOpen: 7,
  });

  assert.equal(model.allSessionCount, 2);
  assert.equal(model.sessions.length, 1);
  assert.equal(model.sessions[0].sessionId, "remote");
  assert.equal(model.attentionInboxCount, 1);
  assert.equal(model.attentionInbox[0].sessionId, "remote");
  assert.equal(model.fleetLens.total_sessions, 2);
  assert.equal(
    model.fleetLens.buckets.find((bucket) => bucket.kind === "target" && bucket.key === "skillbox")?.advisory_count,
    1,
  );
  assert.deepEqual(
    model.fleetLens.buckets
      .filter((bucket) => bucket.kind === "advisory")
      .map((bucket) => [bucket.key, bucket.label, bucket.count, bucket.stale_count]),
    [["capacity:tight", "capacity: tight", 1, 1]],
  );
  assert.equal(
    model.fleetLens.buckets.find((bucket) => bucket.kind === "repo")?.count,
    2,
  );
  assert.deepEqual(model.fleetFilter, { kind: "target", key: "skillbox" });
  assert.equal(model.fleetChips[0].label, "all 2");
  assert.equal(model.fleetChips[1].label, "target Skillbox devbox 1 · ext 1");
  assert.equal(model.fleetChips[1].active, true);
});

test("buildSurfaceModel can filter by passive advisory metadata without changing session identity", () => {
  const advisory = rawSession({
    session_id: "advisory",
    tmux_name: "advisory-agent",
    cwd: "/Users/tester/repos/opensource/swimmers",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
      advisory: [{
        source: "c0",
        label: "c0 group",
        value: "swimmers",
        status: "external",
        stale: true,
        group_key: "c0:swimmers",
      }],
    },
  });
  const plain = rawSession({
    session_id: "plain",
    tmux_name: "plain-agent",
    cwd: "/Users/tester/repos/opensource/skills",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/skills",
    },
  });

  const model = buildSurfaceModel({
    state: baseState({
      sessions: [advisory, plain],
      fleetFilter: { kind: "advisory", key: "c0:swimmers" },
    }),
    boot: { focus_layout: false, franken_term_available: true },
    websocketOpen: 7,
  });

  assert.deepEqual(model.fleetFilter, { kind: "advisory", key: "c0:swimmers" });
  assert.deepEqual(model.sessions.map((session) => session.sessionId), ["advisory"]);
  assert.equal(model.sessions[0].fullCwd, "/Users/tester/repos/opensource/swimmers");
  assert.equal(model.sessions[0].name, "advisory-agent");
  assert.equal(model.fleetChips[1].label, "advisory c0 group: swimmers 1 · ext 1");
  assert.equal(model.fleetChips[1].active, true);
});

test("buildEnvironmentMatrix keeps ssh-only handoff targets visible without fake sessions", () => {
  const liveRemote = surfaceSession(rawSession({
    session_id: "skillbox::remote",
    state: "attention",
    environment: {
      scope: "remote",
      target_id: "skillbox-api",
      target_label: "Skillbox API",
      target_kind: "swimmers_api",
      display_host: "Skillbox API",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  }));
  const rows = buildEnvironmentMatrix([
    {
      id: "local",
      label: "Local machine",
      kind: "local",
      display_host: "local",
      backend_mode: "local",
      status: "Healthy",
      capabilities: { observe_sessions: true, launch_session: true, advisory_metadata: true },
      path_mapping_count: 0,
    },
    {
      id: "skillbox-api",
      label: "Skillbox API",
      kind: "swimmers_api",
      display_host: "Skillbox API",
      backend_mode: "remote_swimmers_api",
      status: "Healthy",
      capabilities: { observe_sessions: true, launch_session: true, remote_dir_inventory: true },
      path_mapping_count: 2,
    },
    {
      id: "skillbox-devbox",
      label: "Skillbox devbox",
      kind: "ssh_only",
      display_host: "Skillbox devbox",
      backend_mode: "ssh_handoff",
      status: "NotConfigured",
      capabilities: { ssh_attach_hint: true, bootstrap_hint: true, advisory_metadata: true },
      attach_hint: "ssh skillbox-devbox",
      bootstrap_hint: "ssh skillbox-devbox 'swimmers serve'",
      last_error: null,
      path_mapping_count: 0,
    },
  ], [liveRemote]);

  assert.deepEqual(rows.map((row) => row.id), ["skillbox-api", "local", "skillbox-devbox"]);
  assert.equal(rows[0].sessionCount, 1);
  assert.equal(rows[0].attentionCount, 1);
  assert.equal(rows[0].readinessKey, "needs_attention");
  assert.equal(rows[2].sessionCount, 0);
  assert.equal(rows[2].handoffOnly, true);
  assert.equal(rows[2].observeCapable, false);
  assert.equal(rows[2].launchCapable, false);
  assert.equal(rows[2].readinessKey, "handoff");
  assert.deepEqual(rows[2].capabilityLabels, ["ssh", "bootstrap", "external"]);
  assert.equal(rows[2].attachHint, "ssh skillbox-devbox");
  assert.equal(rows[2].bootstrapHint, "ssh skillbox-devbox 'swimmers serve'");
  assert.equal(rows[2].lastError, "");
});

test("buildEnvironmentMatrix preserves down API health error and configured bootstrap hint", () => {
  const rows = buildEnvironmentMatrix([
    {
      id: "skillbox-api",
      label: "Skillbox API",
      kind: "swimmers_api",
      display_host: "Skillbox API",
      backend_mode: "remote_swimmers_api",
      status: "Unavailable",
      last_error: "base_url_unavailable",
      capabilities: { bootstrap_hint: true, advisory_metadata: true },
      bootstrap_hint: "ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'",
      path_mapping_count: 2,
    },
  ], []);

  assert.equal(rows[0].id, "skillbox-api");
  assert.equal(rows[0].sessionCount, 0);
  assert.equal(rows[0].readinessKey, "degraded");
  assert.equal(rows[0].lastError, "base_url_unavailable");
  assert.equal(
    rows[0].bootstrapHint,
    "ssh skillbox-devbox 'AUTH_TOKEN=$AUTH_TOKEN swimmers serve'",
  );
});

test("buildSurfaceModel exposes environment matrix independent of active session filter", () => {
  const local = rawSession({
    session_id: "local",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const state = baseState({
    sessions: [local],
    fleetFilter: { kind: "target", key: "missing" },
    environments: [{
      id: "skillbox-devbox",
      label: "Skillbox devbox",
      kind: "ssh_only",
      display_host: "Skillbox devbox",
      backend_mode: "ssh_handoff",
      status: "NotConfigured",
      capabilities: { ssh_attach_hint: true, bootstrap_hint: true, advisory_metadata: true },
      attach_hint: "ssh skillbox-devbox",
    }],
  });

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: false, franken_term_available: true },
    websocketOpen: 7,
  });

  assert.deepEqual(model.fleetFilter, { kind: "", key: "" });
  assert.equal(model.sessions.length, 1);
  assert.equal(model.environmentMatrix.length, 1);
  assert.equal(model.environmentMatrix[0].id, "skillbox-devbox");
  assert.equal(model.environmentMatrix[0].sessionCount, 0);
  assert.equal(model.environmentMatrix[0].readinessKey, "handoff");
});

test("buildSurfaceModel collapses repo fallback buckets without pressure data", () => {
  const root = rawSession({
    session_id: "root",
    tmux_name: "root-agent",
    cwd: "/Users/tester/repos/opensource/swimmers",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const nested = rawSession({
    session_id: "nested",
    tmux_name: "nested-agent",
    cwd: "/Users/tester/repos/opensource/swimmers/src/web",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers/src/web",
    },
  });
  const state = baseState({
    sessions: [root, nested],
    fleetFilter: { kind: "repo", key: "/Users/tester/repos/opensource/swimmers" },
  });

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: false, franken_term_available: true },
    websocketOpen: 7,
  });

  assert.deepEqual(model.fleetFilter, {
    kind: "repo",
    key: "/Users/tester/repos/opensource/swimmers",
  });
  assert.deepEqual(
    model.sessions.map((session) => session.sessionId),
    ["root", "nested"],
  );
  assert.deepEqual(
    model.fleetLens.buckets
      .filter((bucket) => bucket.kind === "repo")
      .map((bucket) => [bucket.key, bucket.label, bucket.count]),
    [["/Users/tester/repos/opensource/swimmers", "opensource/swimmers", 2]],
  );
});

test("buildSurfaceModel suppresses selected details outside the active fleet filter", () => {
  const local = rawSession({
    session_id: "local",
    tmux_name: "local",
    cwd: "/Users/tester/repos/opensource/swimmers",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const remote = rawSession({
    session_id: "remote",
    tmux_name: "remote",
    cwd: "/srv/skillbox/repos/swimmers",
    environment: {
      scope: "remote",
      target_id: "skillbox",
      target_label: "Skillbox devbox",
      target_kind: "swimmers_api",
      display_host: "Skillbox devbox",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });

  const model = buildSurfaceModel({
    state: baseState({
      sessions: [local, remote],
      selectedSessionId: "local",
      fleetFilter: { kind: "target", key: "skillbox" },
    }),
    boot: { focus_layout: false, franken_term_available: true },
    currentSession: () => local,
    websocketOpen: 7,
  });

  assert.deepEqual(model.fleetFilter, { kind: "target", key: "skillbox" });
  assert.deepEqual(model.sessions.map((session) => session.sessionId), ["remote"]);
  assert.equal(model.selectedSessionId, "local");
  assert.equal(model.currentSession, null);
});

test("buildSurfaceModel drops saved fleet filters when the bucket is unavailable", () => {
  const local = rawSession({
    session_id: "local",
    tmux_name: "local",
    cwd: "/Users/tester/repos/opensource/swimmers",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const state = baseState({
    sessions: [local],
    fleetFilter: { kind: "target", key: "missing-devbox" },
  });

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: false, franken_term_available: true },
    websocketOpen: 7,
  });

  assert.deepEqual(model.fleetFilter, { kind: "", key: "" });
  assert.equal(model.sessions.length, 1);
  assert.equal(model.fleetChips[0].label, "all 1");
  assert.equal(model.fleetChips[0].active, true);
});

test("buildSurfaceModel builds display-only project groups across local and remote sessions", () => {
  const local = rawSession({
    session_id: "local",
    tmux_name: "local-agent",
    cwd: "/Users/tester/repos/opensource/swimmers",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const remote = rawSession({
    session_id: "remote",
    tmux_name: "remote-agent",
    cwd: "/srv/skillbox/repos/swimmers",
    environment: {
      scope: "remote",
      target_id: "skillbox",
      target_label: "Skillbox devbox",
      target_kind: "swimmers_api",
      display_host: "Skillbox devbox",
      canonical_cwd: "/Users/tester/repos/opensource/swimmers",
    },
  });
  const other = rawSession({
    session_id: "other",
    tmux_name: "other-agent",
    cwd: "/Users/tester/repos/opensource/skills",
    environment: {
      scope: "local",
      target_id: "local",
      target_label: "Local machine",
      target_kind: "local",
      display_host: "local",
      canonical_cwd: "/Users/tester/repos/opensource/skills",
    },
  });

  const model = buildSurfaceModel({
    state: baseState({
      sessions: [other, remote, local],
      selectedSessionId: "remote",
      sessionGroupMode: "project",
    }),
    boot: { focus_layout: false, franken_term_available: true },
    websocketOpen: 7,
  });

  assert.equal(model.sessionGroupMode, "project");
  assert.deepEqual(
    model.sessionRailRows.map((row) => [row.session.sessionId, row.group.label, row.group.count, row.group.hostSummary, row.group.first]),
    [
      ["local", "opensource/swimmers", 2, "local + Skillbox devbox", true],
      ["remote", "opensource/swimmers", 2, "local + Skillbox devbox", false],
      ["other", "opensource/skills", 1, "local", true],
    ],
  );
  assert.deepEqual(
    buildSessionRailRows(model.sessions, "flat").map((row) => row.session.sessionId),
    ["other", "remote", "local"],
  );
});

test("buildSurfaceModel reports terminal not ready without the expected websocket state", () => {
  const state = baseState({
    hoveredTrogdorSessionId: null,
    ws: { readyState: 2 },
  });

  const model = buildSurfaceModel({
    state,
    boot: { focus_layout: true, franken_term_available: false },
    currentSession: () => null,
    now: () => 2000,
    websocketOpen: 7,
  });

  assert.equal(model.terminalReady, false);
  assert.equal(model.snapshotFallback, true);
  assert.equal(model.currentSession, null);
  assert.equal(model.trogdorReaderElapsedMs, 0);
});
