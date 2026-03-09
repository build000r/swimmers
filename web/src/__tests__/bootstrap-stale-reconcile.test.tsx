import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/preact";
import { h } from "preact";

import type { SessionSummary } from "@/types";

const mocks = vi.hoisted(() => {
  const staleSession: SessionSummary = {
    session_id: "sess-stale",
    tmux_name: "old",
    cwd: "/Users/b/repos/old",
    state: "idle",
    current_command: null,
    tool: null,
    token_count: 0,
    context_limit: 200000,
    thought: null,
    thought_state: "holding",
    thought_source: "carry_forward",
    thought_updated_at: null,
    rest_state: "drowsy",
    last_skill: null,
    is_stale: true,
    attached_clients: 0,
    transport_health: "healthy",
    last_activity_at: new Date("2026-03-01T12:00:00.000Z").toISOString(),
    sprite_pack_id: null,
    repo_theme_id: null,
  };

  const liveSession: SessionSummary = {
    ...staleSession,
    session_id: "sess-live",
    tmux_name: "live",
    cwd: "/Users/b/repos/live",
    is_stale: false,
  };

  return {
    staleSession,
    liveSession,
    bootstrapMock: vi.fn(async () => ({
      server_time: new Date().toISOString(),
      auth_mode: "operator",
      realtime_url: "ws://localhost:3210/v1/realtime",
      workspace_history_mode: "url_state_v1",
      poll_fallback_ms: 60_000,
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
      sessions: [staleSession],
      sprite_packs: {},
      repo_themes: {},
    })),
    createSessionMock: vi.fn(),
    deleteSessionMock: vi.fn(),
    fetchPaneTailMock: vi.fn(),
    fetchSessionsMock: vi.fn(async () => ({
      sessions: [liveSession],
      version: 1,
      sprite_packs: {},
      repo_themes: {},
    })),
    fetchNativeDesktopStatusMock: vi.fn(async () => ({
      supported: false,
      app: "iTerm",
    })),
    openNativeDesktopSessionMock: vi.fn(),
  };
});

vi.mock("@/services/api", () => ({
  bootstrap: mocks.bootstrapMock,
  createSession: mocks.createSessionMock,
  deleteSession: mocks.deleteSessionMock,
  fetchPaneTail: mocks.fetchPaneTailMock,
  fetchSessions: mocks.fetchSessionsMock,
  fetchNativeDesktopStatus: mocks.fetchNativeDesktopStatusMock,
  openNativeDesktopSession: mocks.openNativeDesktopSessionMock,
}));

vi.mock("@/services/realtime", () => ({
  RealtimeService: class MockRealtimeService {
    on(): void {}
    connect(): void {}
    disconnect(): void {}
    sendDismissAttention(): void {}
  },
}));

vi.mock("@/hooks/useObserverMode", () => ({
  useObserverMode: () => ({ isObserver: false }),
}));

vi.mock("@/components/OverviewField", () => ({
  OverviewField: ({ sessions }: { sessions: SessionSummary[] }) => (
    <div data-testid="overview-sessions">
      {sessions.map((session) => session.session_id).join(",") || "none"}
    </div>
  ),
}));

vi.mock("@/components/ZoneManager", () => ({
  ZoneManager: () => <div data-testid="zone-manager" />,
}));

import { App, activeSessionId, currentView, sessions } from "@/app";

describe("bootstrap stale session reconciliation", () => {
  beforeEach(() => {
    activeSessionId.value = null;
    currentView.value = "overview";
    sessions.value = [];
    mocks.bootstrapMock.mockClear();
    mocks.fetchSessionsMock.mockClear();
    mocks.fetchNativeDesktopStatusMock.mockClear();
  });

  it("replaces stale bootstrap sessions with live session inventory before first render", async () => {
    render(<App />);

    await waitFor(() => {
      expect(mocks.fetchSessionsMock).toHaveBeenCalledTimes(1);
      expect(screen.getByTestId("overview-sessions")).toHaveTextContent("sess-live");
    });

    expect(screen.getByTestId("overview-sessions")).not.toHaveTextContent("sess-stale");
    expect(sessions.value.map((session) => session.session_id)).toEqual(["sess-live"]);
  });
});
