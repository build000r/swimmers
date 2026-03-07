import { describe, expect, it, beforeEach, vi } from "vitest";
import { render, fireEvent, screen, waitFor } from "@testing-library/preact";
import { h } from "preact";
import { ZoneManager } from "@/components/ZoneManager";
import { makeSession } from "./helpers/fixtures";

const { unsubscribeSessionMock } = vi.hoisted(() => ({
  unsubscribeSessionMock: vi.fn(),
}));

vi.mock("@/app", () => ({
  realtime: {
    unsubscribeSession: unsubscribeSessionMock,
  },
  terminalCacheTtlMs: { value: 300_000 },
  zoneLayout: { value: "single" },
}));

vi.mock("@/hooks/useTerminalCache", () => ({
  useTerminalCache: () => ({
    get: () => null,
    put: () => {},
    evict: () => {},
  }),
}));

vi.mock("@/components/TerminalWorkspace", () => ({
  TerminalWorkspace: ({
    session,
    onClose,
  }: {
    session: { session_id: string };
    onClose: () => void;
  }) => (
    <div data-testid={`terminal-${session.session_id}`}>
      <button
        type="button"
        data-testid={`close-${session.session_id}`}
        onClick={() => setTimeout(onClose, 0)}
      >
        close
      </button>
    </div>
  ),
}));

function setViewportWidth(width: number): void {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: width,
  });
}

async function settleEffects(): Promise<void> {
  await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("ZoneManager close behavior", () => {
  beforeEach(() => {
    unsubscribeSessionMock.mockReset();
  });

  it("closes the only mobile main zone and returns to overview", async () => {
    setViewportWidth(390);
    const onShowOverview = vi.fn();

    render(
      <ZoneManager
        sessions={[makeSession({ session_id: "sess-1", tmux_name: "1" })]}
        activeSessionId="sess-1"
        preferZone={null}
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("close-sess-1")).toBeInTheDocument();
    });
    await settleEffects();

    fireEvent.click(screen.getByTestId("close-sess-1"));

    await waitFor(() => {
      expect(unsubscribeSessionMock).toHaveBeenCalledTimes(1);
      expect(unsubscribeSessionMock).toHaveBeenCalledWith("sess-1");
      expect(onShowOverview).toHaveBeenCalledTimes(1);
    });
  });

  it("returns to overview when the only open session disappears from the live list", async () => {
    setViewportWidth(390);
    const onShowOverview = vi.fn();

    const view = render(
      <ZoneManager
        sessions={[makeSession({ session_id: "sess-1", tmux_name: "1" })]}
        activeSessionId="sess-1"
        preferZone={null}
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("terminal-sess-1")).toBeInTheDocument();
    });

    unsubscribeSessionMock.mockClear();

    view.rerender(
      <ZoneManager
        sessions={[]}
        activeSessionId={null}
        preferZone={null}
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(unsubscribeSessionMock).toHaveBeenCalledWith("sess-1");
      expect(onShowOverview).toHaveBeenCalledTimes(1);
      expect(screen.queryByTestId("terminal-sess-1")).toBeNull();
    });
  });

  it("closing bottom zone on desktop keeps main zone open", async () => {
    setViewportWidth(1280);
    const sessions = [
      makeSession({ session_id: "sess-main", tmux_name: "main" }),
      makeSession({ session_id: "sess-bottom", tmux_name: "bottom" }),
    ];
    const onShowOverview = vi.fn();

    const view = render(
      <ZoneManager
        sessions={sessions}
        activeSessionId="sess-main"
        preferZone={null}
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("terminal-sess-main")).toBeInTheDocument();
    });

    view.rerender(
      <ZoneManager
        sessions={sessions}
        activeSessionId="sess-bottom"
        preferZone="bottom"
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("terminal-sess-bottom")).toBeInTheDocument();
    });
    await settleEffects();

    fireEvent.click(screen.getByTestId("close-sess-bottom"));

    await waitFor(() => {
      expect(unsubscribeSessionMock).toHaveBeenCalledWith("sess-bottom");
      expect(onShowOverview).not.toHaveBeenCalled();
      expect(screen.getByTestId("terminal-sess-main")).toBeInTheDocument();
    });
  });

  it("removing an exited bottom session from the live session list keeps main zone open", async () => {
    setViewportWidth(1280);
    const sessions = [
      makeSession({ session_id: "sess-main", tmux_name: "main" }),
      makeSession({ session_id: "sess-exit", tmux_name: "exit" }),
    ];
    const onShowOverview = vi.fn();

    const view = render(
      <ZoneManager
        sessions={sessions}
        activeSessionId="sess-main"
        preferZone={null}
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("terminal-sess-main")).toBeInTheDocument();
    });

    view.rerender(
      <ZoneManager
        sessions={sessions}
        activeSessionId="sess-exit"
        preferZone="bottom"
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId("terminal-sess-exit")).toBeInTheDocument();
    });

    unsubscribeSessionMock.mockClear();

    view.rerender(
      <ZoneManager
        sessions={[sessions[0]]}
        activeSessionId={null}
        preferZone={null}
        restoreRequest={null}
        onShowOverview={onShowOverview}
        onStartPolling={() => {}}
        onStopPolling={() => {}}
        onLayoutChange={() => {}}
      />,
    );

    await waitFor(() => {
      expect(unsubscribeSessionMock).toHaveBeenCalledWith("sess-exit");
      expect(onShowOverview).not.toHaveBeenCalled();
      expect(screen.getByTestId("terminal-sess-main")).toBeInTheDocument();
      expect(screen.queryByTestId("terminal-sess-exit")).toBeNull();
    });
  });
});
