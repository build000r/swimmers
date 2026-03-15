import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, fireEvent, screen, waitFor } from "@testing-library/preact";
import { h } from "preact";

import { TerminalWorkspace } from "@/components/TerminalWorkspace";
import { makeSession } from "./helpers/fixtures";

const mocks = vi.hoisted(() => {
  const copyTextToClipboardMock = vi.fn(async () => true);
  const readTextFromClipboardWithFallbackMock = vi.fn(async () => "");
  const termFocusMock = vi.fn();
  const termRefreshMock = vi.fn();
  const terminalOptionsMock = vi.fn();

  const fetchSnapshotMock = vi.fn(async (sessionId: string) => ({
    session_id: sessionId,
    latest_seq: 0,
    truncated: false,
    screen_text: "",
  }));

  const listSkillsMock = vi.fn(async () => ({ skills: [] as unknown[] }));

  const realtime = {
    subscribeTerminalOutput: vi.fn(() => () => {}),
    subscribeReplayTruncated: vi.fn(() => () => {}),
    subscribeSessionSubscription: vi.fn(() => () => {}),
    subscribeSession: vi.fn(),
    unsubscribeSession: vi.fn(),
    sendResize: vi.fn(),
    sendInput: vi.fn(),
    forceResubscribe: vi.fn(),
  };

  return {
    copyTextToClipboardMock,
    readTextFromClipboardWithFallbackMock,
    termFocusMock,
    termRefreshMock,
    terminalOptionsMock,
    fetchSnapshotMock,
    listSkillsMock,
    realtime,
  };
});

vi.mock("@/lib/clipboard", () => ({
  copyTextToClipboard: mocks.copyTextToClipboardMock,
  readTextFromClipboardWithFallback: mocks.readTextFromClipboardWithFallbackMock,
}));

vi.mock("@/services/api", () => ({
  fetchSnapshot: mocks.fetchSnapshotMock,
  listSkills: mocks.listSkillsMock,
}));

vi.mock("@/app", () => ({
  realtime: mocks.realtime,
  repoThemes: { value: {} },
  spritePacks: { value: {} },
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class MockTerminal {
    cols = 80;
    rows = 24;
    buffer = {
      active: {
        viewportY: 0,
        length: 500,
      },
    };

    constructor(options?: unknown) {
      mocks.terminalOptionsMock(options);
    }

    loadAddon(_addon: unknown): void {}

    open(hostEl: HTMLElement): void {
      const textarea = document.createElement("textarea");
      hostEl.appendChild(textarea);

      const viewport = document.createElement("div");
      viewport.className = "xterm-viewport";

      let scrollTop = 600;
      Object.defineProperty(viewport, "scrollTop", {
        configurable: true,
        get() {
          return scrollTop;
        },
        set(value: number) {
          scrollTop = Number(value);
        },
      });
      Object.defineProperty(viewport, "scrollHeight", {
        configurable: true,
        get() {
          return 1200;
        },
      });
      Object.defineProperty(viewport, "clientHeight", {
        configurable: true,
        get() {
          return 400;
        },
      });

      hostEl.appendChild(viewport);
    }

    write(_data: string | Uint8Array): void {}
    clear(): void {}
    focus(): void {
      mocks.termFocusMock();
    }
    paste(_text: string): void {}
    selectAll(): void {}
    clearSelection(): void {}
    getSelection(): string {
      return "";
    }
    selectLines(_start: number, _end: number): void {}
    attachCustomKeyEventHandler(
      _handler: (event: KeyboardEvent) => boolean,
    ): void {}
    onData(_cb: (data: string) => void): { dispose: () => void } {
      return { dispose: () => {} };
    }
    onResize(_cb: (size: { cols: number; rows: number }) => void): {
      dispose: () => void;
    } {
      return { dispose: () => {} };
    }
    refresh(_start: number, _end: number): void {
      mocks.termRefreshMock();
    }
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class MockFitAddon {
    fit(): void {}
  },
}));

vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: class MockWebglAddon {
    onContextLoss(_cb: () => void): void {}
    dispose(): void {}
  },
}));

vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class MockSearchAddon {
    findNext(_query: string): boolean {
      return false;
    }

    findPrevious(_query: string): boolean {
      return false;
    }
  },
}));

describe("terminal header actions", () => {
  function setViewportWidth(width: number): void {
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      writable: true,
      value: width,
    });
  }

  beforeEach(() => {
    setViewportWidth(1024);

    mocks.copyTextToClipboardMock.mockClear();
    mocks.readTextFromClipboardWithFallbackMock.mockClear();
    mocks.termFocusMock.mockClear();
    mocks.termRefreshMock.mockClear();
    mocks.terminalOptionsMock.mockClear();
    mocks.fetchSnapshotMock.mockClear();
    mocks.listSkillsMock.mockClear();

    mocks.realtime.subscribeTerminalOutput.mockClear();
    mocks.realtime.subscribeReplayTruncated.mockClear();
    mocks.realtime.subscribeSessionSubscription.mockClear();
    mocks.realtime.subscribeSession.mockClear();
    mocks.realtime.unsubscribeSession.mockClear();
    mocks.realtime.sendResize.mockClear();
    mocks.realtime.sendInput.mockClear();
    mocks.realtime.forceResubscribe.mockClear();
  });

  it("tapping title copies attach command and does not close terminal", async () => {
    const onClose = vi.fn();

    const { container } = render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-title", tmux_name: "2" })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={onClose}
      />,
    );

    const title = container.querySelector(".zone-title") as HTMLElement | null;
    expect(title).not.toBeNull();

    fireEvent.touchStart(title as HTMLElement, {
      touches: [{ clientX: 240, clientY: 80 }],
    });
    fireEvent.touchEnd(title as HTMLElement, {
      changedTouches: [{ clientX: 236, clientY: 82 }],
    });
    fireEvent.click(title as HTMLElement);

    await waitFor(() => {
      expect(mocks.copyTextToClipboardMock).toHaveBeenCalledWith("tmux a -t 2");
    });

    expect(onClose).not.toHaveBeenCalled();
  });

  it("closes immediately on process_exit", async () => {
    const onSessionExit = vi.fn();

    render(
      <TerminalWorkspace
        session={makeSession({
          session_id: "sess-exit",
          tmux_name: "exit-demo",
          state: "exited",
          exit_reason: "process_exit",
        })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={onSessionExit}
        onClose={() => {}}
      />,
    );

    await waitFor(() => {
      expect(onSessionExit).toHaveBeenCalledTimes(1);
      expect(onSessionExit).toHaveBeenCalledWith("sess-exit");
    });
  });

  it("does not auto-close non-process exits", async () => {
    const onSessionExit = vi.fn();

    render(
      <TerminalWorkspace
        session={makeSession({
          session_id: "sess-startup-missing",
          tmux_name: "missing-demo",
          state: "exited",
          exit_reason: "startup_missing_tmux",
        })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={onSessionExit}
        onClose={() => {}}
      />,
    );

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(onSessionExit).not.toHaveBeenCalled();
  });

  it("moves mobile quick commands into the dock and exposes terminal keys", async () => {
    setViewportWidth(390);

    const { container } = render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-mobile-dock", tmux_name: "dock" })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    expect(container.querySelector(".quick-command-bar")).toBeNull();
    expect(screen.getByRole("button", { name: "Keyboard" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Quick" }));
    expect(screen.getByRole("button", { name: "ls" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Keys" }));
    expect(screen.getByRole("button", { name: "PgUp" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "↑" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "$" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "\\" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "=" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Tools" }));
    expect(screen.getByRole("button", { name: "Find" })).toBeInTheDocument();
  });

  it("uses a denser readable terminal config on mobile", () => {
    setViewportWidth(390);

    render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-mobile-font", tmux_name: "font" })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    expect(mocks.terminalOptionsMock).toHaveBeenCalled();
    const terminalCalls = mocks.terminalOptionsMock.mock.calls;
    const options = terminalCalls[terminalCalls.length - 1]?.[0] as Record<
      string,
      unknown
    >;
    expect(options.fontSize).toBe(13);
    expect(options.lineHeight).toBe(1.2);
    expect(options.customGlyphs).toBe(false);
    expect(options.minimumContrastRatio).toBe(1.15);
  });

  it("focuses the terminal from the mobile keyboard button", async () => {
    setViewportWidth(390);

    const { container } = render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-mobile-focus", tmux_name: "focus" })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    await waitFor(() => {
      expect(container.querySelector(".xterm-viewport")).not.toBeNull();
    });

    mocks.termFocusMock.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "Keyboard" }));

    expect(mocks.termFocusMock).toHaveBeenCalledTimes(1);
  });

  it("renders agent skills inside the mobile quick panel", async () => {
    setViewportWidth(390);
    mocks.listSkillsMock.mockResolvedValueOnce({
      skills: [{ name: "deploy", description: "Push the app" }],
    });

    const { container } = render(
      <TerminalWorkspace
        session={makeSession({
          session_id: "sess-mobile-skills",
          tmux_name: "skills",
          tool: "codex",
        })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    expect(container.querySelector(".quick-command-bar")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Quick" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "$deploy" })).toBeInTheDocument();
    });
  });

  it("refreshes the terminal after mobile viewport scrolling settles", async () => {
    setViewportWidth(390);

    const { container } = render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-mobile-scroll", tmux_name: "scroll" })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    await waitFor(() => {
      expect(container.querySelector(".xterm-viewport")).not.toBeNull();
    });

    const viewport = container.querySelector(".xterm-viewport") as HTMLElement;
    mocks.termRefreshMock.mockClear();
    fireEvent.scroll(viewport);

    await waitFor(() => {
      expect(mocks.termRefreshMock).toHaveBeenCalledTimes(1);
    });
  });

  it("refreshes the terminal after desktop viewport scrolling settles", async () => {
    const { container } = render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-desktop-scroll", tmux_name: "scroll" })}
        cached={null}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    await waitFor(() => {
      expect(container.querySelector(".xterm-viewport")).not.toBeNull();
    });

    const viewport = container.querySelector(".xterm-viewport") as HTMLElement;
    mocks.termRefreshMock.mockClear();
    fireEvent.scroll(viewport);

    await waitFor(() => {
      expect(mocks.termRefreshMock).toHaveBeenCalledTimes(1);
    });
  });

  it("refits cached terminals before sending resize", async () => {
    const hostEl = document.createElement("div");
    hostEl.className = "term-host";
    hostEl.style.width = "100%";
    hostEl.style.height = "100%";

    hostEl.appendChild(document.createElement("textarea"));
    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    Object.defineProperty(viewport, "scrollTop", {
      configurable: true,
      get() {
        return 0;
      },
      set() {},
    });
    Object.defineProperty(viewport, "scrollHeight", {
      configurable: true,
      get() {
        return 1200;
      },
    });
    Object.defineProperty(viewport, "clientHeight", {
      configurable: true,
      get() {
        return 400;
      },
    });
    hostEl.appendChild(viewport);

    const term = {
      cols: 80,
      rows: 24,
      buffer: {
        active: {
          viewportY: 0,
          length: 500,
        },
      },
      loadAddon: vi.fn(),
      write: vi.fn(),
      clear: vi.fn(),
      focus: vi.fn(),
      paste: vi.fn(),
      selectAll: vi.fn(),
      clearSelection: vi.fn(),
      getSelection: vi.fn(() => ""),
      selectLines: vi.fn(),
      attachCustomKeyEventHandler: vi.fn(),
      onData: vi.fn(() => ({ dispose: () => {} })),
      onResize: vi.fn(() => ({ dispose: () => {} })),
      refresh: vi.fn(),
    };
    const fitAddon = {
      fit: vi.fn(() => {
        term.cols = 118;
        term.rows = 14;
      }),
    };

    render(
      <TerminalWorkspace
        session={makeSession({ session_id: "sess-cached", tmux_name: "cached" })}
        cached={{
          term: term as never,
          fitAddon: fitAddon as never,
          hostEl,
          sessionId: "sess-cached",
          latestSeq: 12,
        }}
        observer={false}
        onCache={() => {}}
        onSessionExit={() => {}}
        onClose={() => {}}
      />,
    );

    await waitFor(() => {
      expect(fitAddon.fit).toHaveBeenCalledTimes(1);
    });

    expect(term.refresh).toHaveBeenCalledWith(0, 13);
    expect(term.focus).toHaveBeenCalledTimes(1);
    expect(mocks.realtime.sendInput).not.toHaveBeenCalled();
    expect(mocks.realtime.sendResize).toHaveBeenCalledWith("sess-cached", 118, 14);
    expect(mocks.realtime.sendResize).not.toHaveBeenCalledWith("sess-cached", 80, 24);
  });
});
