import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/preact";
import { h } from "preact";

import type { SessionStatePayload, SessionSummary } from "@/types";

const mocks = vi.hoisted(() => {
  const exitSession: SessionSummary = {
    session_id: "sess-exit",
    tmux_name: "exit-demo",
    state: "idle",
    exit_reason: null,
    current_command: null,
    cwd: "/Users/b/repos/exit-demo",
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
    attached_clients: 0,
    transport_health: "healthy",
    last_activity_at: new Date("2026-03-06T15:00:00.000Z").toISOString(),
    sprite_pack_id: null,
    repo_theme_id: null,
  };

  const keepSession: SessionSummary = {
    ...exitSession,
    session_id: "sess-keep",
    tmux_name: "keep-demo",
    cwd: "/Users/b/repos/keep-demo",
  };

  return {
    exitSession,
    keepSession,
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
      sessions: [exitSession, keepSession],
      sprite_packs: {},
      repo_themes: {},
    })),
    fetchPaneTailMock: vi.fn(async (sessionId: string) => ({
      session_id: sessionId,
      text: "",
    })),
    fetchNativeDesktopStatusMock: vi.fn(async () => ({
      supported: false,
      app: "iTerm",
    })),
    realtimeInstance: null as null | {
      callbacks: {
        onSessionState?: (sessionId: string, payload: SessionStatePayload) => void;
      };
    },
  };
});

vi.mock("@/services/api", () => ({
  bootstrap: mocks.bootstrapMock,
  createSession: vi.fn(),
  deleteSession: vi.fn(),
  fetchPaneTail: mocks.fetchPaneTailMock,
  fetchSessions: vi.fn(),
  fetchNativeDesktopStatus: mocks.fetchNativeDesktopStatusMock,
  openNativeDesktopSession: vi.fn(),
}));

vi.mock("@/services/realtime", () => ({
  RealtimeService: class MockRealtimeService {
    callbacks: {
      onSessionState?: (sessionId: string, payload: SessionStatePayload) => void;
    } = {};

    constructor() {
      mocks.realtimeInstance = this;
    }

    on(cbs: Record<string, unknown>) {
      this.callbacks = { ...this.callbacks, ...(cbs as typeof this.callbacks) };
    }

    connect(): void {}
    disconnect(): void {}
    sendDismissAttention(): void {}
    subscribeSession(): void {}
    unsubscribeSession(): void {}
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

describe("App process_exit overview cleanup", () => {
  beforeEach(() => {
    activeSessionId.value = null;
    currentView.value = "overview";
    sessions.value = [];
    mocks.bootstrapMock.mockClear();
    mocks.fetchPaneTailMock.mockClear();
    mocks.fetchNativeDesktopStatusMock.mockClear();
    if (mocks.realtimeInstance) {
      mocks.realtimeInstance.callbacks = {};
    }
  });

  it("hides process_exit sessions from the overview immediately", async () => {
    render(<App />);

    await waitFor(() => {
      expect(screen.getByTestId("overview-sessions")).toHaveTextContent(
        "sess-exit,sess-keep",
      );
      expect(mocks.realtimeInstance).not.toBeNull();
    });

    act(() => {
      mocks.realtimeInstance?.callbacks.onSessionState?.("sess-exit", {
        state: "exited",
        previous_state: "idle",
        current_command: null,
        transport_health: "healthy",
        exit_reason: "process_exit",
        at: "2026-03-06T15:00:00.000Z",
      });
    });

    expect(screen.getByTestId("overview-sessions")).toHaveTextContent("sess-keep");
    expect(screen.getByTestId("overview-sessions")).not.toHaveTextContent("sess-exit");
  });
});
