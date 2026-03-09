import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, fireEvent, screen, waitFor } from "@testing-library/preact";
import { h } from "preact";

import type { SessionSummary } from "@/types";

const mocks = vi.hoisted(() => {
  const session: SessionSummary = {
    session_id: "sess-cfo",
    tmux_name: "4",
    cwd: "/Users/b/repos/cfo",
    state: "busy",
    current_command: null,
    tool: null,
    token_count: 0,
    context_limit: 200000,
    thought: null,
    thought_state: "holding",
    thought_source: "carry_forward",
    thought_updated_at: null,
    rest_state: "active",
    last_skill: null,
    is_stale: false,
    attached_clients: 1,
    transport_health: "healthy",
    last_activity_at: new Date().toISOString(),
    sprite_pack_id: null,
    repo_theme_id: null,
  };

  return {
    session,
    bootstrapMock: vi.fn(async () => ({
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
      sessions: [session],
      sprite_packs: {},
      repo_themes: {},
    })),
    createSessionMock: vi.fn(),
    deleteSessionMock: vi.fn(),
    fetchPaneTailMock: vi.fn(),
    fetchSessionsMock: vi.fn(async () => ({
      sessions: [session],
      version: 1,
      sprite_packs: {},
      repo_themes: {},
    })),
    fetchNativeDesktopStatusMock: vi.fn(async () => ({
      supported: true,
      app: "iTerm",
    })),
    openNativeDesktopSessionMock: vi.fn(
      async () => Promise.reject(new Error("Failed to fetch")),
    ),
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
  OverviewField: ({
    sessions,
    onTapSession,
  }: {
    sessions: SessionSummary[];
    onTapSession: (sessionId: string) => void;
  }) => (
    <button
      type="button"
      data-testid="tap-overview-session"
      onClick={() => onTapSession(sessions[0].session_id)}
    >
      tap session
    </button>
  ),
}));

vi.mock("@/components/ZoneManager", () => ({
  ZoneManager: ({
    activeSessionId,
  }: {
    activeSessionId: string | null;
  }) => <div data-testid="active-session">{activeSessionId ?? "none"}</div>,
}));

import { App, activeSessionId, currentView, sessions } from "@/app";

describe("native open fallback", () => {
  beforeEach(() => {
    activeSessionId.value = null;
    currentView.value = "overview";
    sessions.value = [];
    mocks.bootstrapMock.mockClear();
    mocks.fetchNativeDesktopStatusMock.mockClear();
    mocks.fetchSessionsMock.mockClear();
    mocks.openNativeDesktopSessionMock.mockClear();
  });

  it("falls back to inline terminal when native open fetch fails", async () => {
    render(<App />);

    await waitFor(() => {
      expect(screen.getByTestId("tap-overview-session")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tap-overview-session"));

    await waitFor(() => {
      expect(mocks.openNativeDesktopSessionMock).toHaveBeenCalledWith("sess-cfo");
      expect(currentView.value).toBe("terminal");
      expect(activeSessionId.value).toBe("sess-cfo");
      expect(screen.getByTestId("active-session")).toHaveTextContent("sess-cfo");
    });

    expect(
      screen.getByText("iTerm open failed: cfo"),
    ).toBeInTheDocument();
    expect(screen.getByText("Failed to fetch")).toBeInTheDocument();
  });

  it("stays in overview when native open reports the session is missing", async () => {
    mocks.openNativeDesktopSessionMock.mockRejectedValueOnce(
      new Error("SESSION_NOT_FOUND"),
    );
    mocks.fetchSessionsMock.mockResolvedValueOnce({
      sessions: [],
      version: 2,
      sprite_packs: {},
      repo_themes: {},
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByTestId("tap-overview-session")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("tap-overview-session"));

    await waitFor(() => {
      expect(mocks.openNativeDesktopSessionMock).toHaveBeenCalledWith("sess-cfo");
      expect(mocks.fetchSessionsMock).toHaveBeenCalledTimes(1);
    });

    expect(currentView.value).toBe("overview");
    expect(activeSessionId.value).toBeNull();
    expect(screen.queryByTestId("active-session")).not.toBeInTheDocument();
    expect(
      screen.getByText("iTerm open failed: cfo"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Session is no longer available."),
    ).toBeInTheDocument();
  });
});
