import type { SessionSummary, BootstrapResponse } from "@/types";

export function makeSession(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_id: "sess-001",
    tmux_name: "1",
    state: "idle",
    current_command: null,
    cwd: "/home/user/project",
    tool: null,
    token_count: 0,
    context_limit: 200000,
    thought: null,
    is_stale: false,
    attached_clients: 1,
    transport_health: "healthy",
    last_activity_at: new Date().toISOString(),
    ...overrides,
  };
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
    sessions: [makeSession()],
    ...overrides,
  };
}
