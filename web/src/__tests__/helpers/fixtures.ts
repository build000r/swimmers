import type { SessionSummary, BootstrapResponse } from "@/types";

export function makeSession(overrides: Partial<SessionSummary> = {}): SessionSummary {
  const session: SessionSummary = {
    session_id: "sess-001",
    tmux_name: "1",
    state: "idle",
    exit_reason: null,
    current_command: null,
    cwd: "/home/user/project",
    tool: null,
    token_count: 0,
    context_limit: 200000,
    thought: null,
    thought_state: "holding",
    thought_source: "carry_forward",
    thought_updated_at: null,
    rest_state: "drowsy",
    last_skill: null,
    is_stale: false,
    attached_clients: 1,
    transport_health: "healthy",
    last_activity_at: new Date().toISOString(),
    sprite_pack_id: null,
    repo_theme_id: null,
    ...overrides,
  };
  if (!session.rest_state) {
    session.rest_state =
      session.state === "exited"
        ? "deep_sleep"
        : session.state === "idle"
          ? "drowsy"
          : "active";
  }
  return session;
}

export function makeBootstrapResponse(
  overrides: Partial<BootstrapResponse> = {},
): BootstrapResponse {
  return {
    server_time: new Date().toISOString(),
    auth_mode: "operator",
    realtime_url: "ws://localhost:3210/v1/realtime",
    workspace_history_mode: "url_state_v1",
    poll_fallback_ms: 2000,
    thought_tick_ms: 5000,
    thoughts_enabled_default: true,
    terminal_cache_ttl_ms: 300000,
    session_delete_mode: "detach_bridge",
    legacy_parity_locked: false,
    thought_policy: {
      lifecycle_mode: "phase_gated_v1",
      cadence_ms: {
        hot: 15_000,
        warm: 45_000,
        cold: 120_000,
      },
      sleeping_after_ms: 30_000,
      bubble_precedence: "thought_first",
    },
    sessions: [makeSession()],
    sprite_packs: {},
    repo_themes: {},
    ...overrides,
  };
}
