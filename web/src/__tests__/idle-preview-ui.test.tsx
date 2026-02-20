import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/preact";
import { h } from "preact";
import { OverviewField } from "@/components/OverviewField";
import { makeSession } from "./helpers/fixtures";

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

describe("idle preview bubble override", () => {
  const noop = () => {};
  const noopCreate = async () => "";

  it("shows idle preview text instead of thought text for idle sessions", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "idle",
      thought: "thinking through next steps",
      tool: "Claude Code",
    });

    render(
      <OverviewField
        sessions={[session]}
        idlePreviews={{ "sess-001": "tail output from tmux buffer" }}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.getByText("tail output from tmux buffer")).toBeInTheDocument();
    expect(screen.queryByText("thinking through next steps")).toBeNull();
  });

  it("does not override bubble text for non-idle sessions", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "busy",
      thought: null,
      current_command: "npm test",
      tool: "Claude Code",
    });

    render(
      <OverviewField
        sessions={[session]}
        idlePreviews={{ "sess-001": "tail output from tmux buffer" }}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.getByText("npm test")).toBeInTheDocument();
    expect(screen.queryByText("tail output from tmux buffer")).toBeNull();
  });

  it("keeps the previous thought bubble until a new one replaces it", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "busy",
      thought: "first thought",
      current_command: null,
      tool: "Claude Code",
    });

    const { rerender } = render(
      <OverviewField
        sessions={[session]}
        idlePreviews={{}}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.getByText("first thought")).toBeInTheDocument();

    rerender(
      <OverviewField
        sessions={[
          {
            ...session,
            state: "idle",
            thought: null,
            current_command: null,
          },
        ]}
        idlePreviews={{}}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.getByText("first thought")).toBeInTheDocument();

    rerender(
      <OverviewField
        sessions={[
          {
            ...session,
            state: "busy",
            thought: "next thought",
            current_command: null,
          },
        ]}
        idlePreviews={{}}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.getByText("next thought")).toBeInTheDocument();
    expect(screen.queryByText("first thought")).toBeNull();
  });
});
