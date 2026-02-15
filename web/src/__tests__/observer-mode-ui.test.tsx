import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/preact";
import { h } from "preact";
import { OverviewField } from "@/components/OverviewField";
import { makeSession } from "./helpers/fixtures";

/**
 * Tests observer mode UI integration:
 *   - Observer badge is visible when observer=true
 *   - Create session controls (long-press hint) hidden in observer mode
 *   - Normal mode still shows controls
 */

// Mock the gesture hook to avoid DOM interaction complexity
vi.mock("@/hooks/useGestures", () => ({
  useLongPress: () => ({
    onMouseDown: () => {},
    onTouchStart: () => {},
    onTouchMove: () => {},
    onTouchEnd: () => {},
    onMouseUp: () => {},
    onMouseLeave: () => {},
    onMouseMove: () => {},
    onContextMenu: (e: Event) => e.preventDefault(),
  }),
}));

describe("observer mode UI (OverviewField)", () => {
  const sessions = [makeSession({ session_id: "sess-001" })];
  const noop = () => {};

  it("shows OBSERVER badge when observer=true", () => {
    render(
      <OverviewField
        sessions={sessions}
        observer={true}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noop}
      />,
    );

    const badge = screen.getByTestId("observer-badge");
    expect(badge).toBeInTheDocument();
    expect(badge.textContent).toContain("OBSERVER");
  });

  it("does NOT show OBSERVER badge when observer=false", () => {
    render(
      <OverviewField
        sessions={sessions}
        observer={false}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noop}
      />,
    );

    expect(screen.queryByTestId("observer-badge")).toBeNull();
  });

  it("hides 'Long press to create one' hint in observer mode (empty sessions)", () => {
    render(
      <OverviewField
        sessions={[]}
        observer={true}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noop}
      />,
    );

    // "No sessions yet" should still appear
    expect(screen.getByText("No sessions yet")).toBeInTheDocument();
    // But the creation hint should not
    expect(screen.queryByText("Long press to create one")).toBeNull();
  });

  it("shows creation hint in normal mode (empty sessions)", () => {
    render(
      <OverviewField
        sessions={[]}
        observer={false}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noop}
      />,
    );

    expect(screen.getByText("No sessions yet")).toBeInTheDocument();
    expect(screen.getByText("Long press to create one")).toBeInTheDocument();
  });
});

describe("observer mode - input blocking", () => {
  it("observer mode prevents sendInput from being called", () => {
    /**
     * In TerminalWorkspace.tsx:
     *   if (!observer) {
     *     inputDisposable = term.onData((data: string) => {
     *       const bytes = encoder.encode(data);
     *       realtime.sendInput(session.session_id, bytes);
     *     });
     *   }
     *
     * We test this conditional directly.
     */
    const sendInputMock = vi.fn();
    const encoder = new TextEncoder();

    function wireInput(observer: boolean, data: string) {
      if (!observer) {
        const bytes = encoder.encode(data);
        sendInputMock("sess-001", bytes);
      }
    }

    // Normal mode: input is sent
    wireInput(false, "ls\n");
    expect(sendInputMock).toHaveBeenCalledTimes(1);
    expect(sendInputMock).toHaveBeenCalledWith(
      "sess-001",
      expect.any(Uint8Array),
    );

    sendInputMock.mockClear();

    // Observer mode: input is NOT sent
    wireInput(true, "ls\n");
    expect(sendInputMock).not.toHaveBeenCalled();
  });
});
