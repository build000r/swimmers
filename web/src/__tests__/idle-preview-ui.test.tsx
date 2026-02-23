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

  it("keeps thought text over idle preview for idle sessions", () => {
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

    expect(screen.getByText("thinking through next steps")).toBeInTheDocument();
    expect(screen.queryByText("tail output from tmux buffer")).toBeNull();
  });

  it("does not override sleeping thought with idle preview", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "idle",
      thought: "Sleeping.",
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

    expect(screen.getByText("Sleeping.")).toBeInTheDocument();
    expect(screen.queryByText("tail output from tmux buffer")).toBeNull();
  });

  it("does not show command text as thought bubble for busy sessions", () => {
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

    expect(screen.queryByText("npm test")).toBeNull();
    expect(screen.queryByText("tail output from tmux buffer")).toBeNull();
  });

  it("clears sleeping bubble promptly after wake", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "idle",
      thought: "Sleeping.",
      thought_state: "sleeping",
      thought_source: "static_sleeping",
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

    expect(screen.getByText("Sleeping.")).toBeInTheDocument();

    rerender(
      <OverviewField
        sessions={[
          {
            ...session,
            state: "busy",
            thought: null,
            thought_state: "holding",
            thought_source: "carry_forward",
          },
        ]}
        idlePreviews={{}}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.queryByText("Sleeping.")).toBeNull();
  });

  it("suppresses command-like idle preview fallback", () => {
    const session = makeSession({
      session_id: "sess-001",
      state: "idle",
      thought: null,
      current_command: null,
      tool: "Claude Code",
    });

    render(
      <OverviewField
        sessions={[session]}
        idlePreviews={{ "sess-001": "npm test --watch" }}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.queryByText("npm test --watch")).toBeNull();
  });

  it("clears the thought bubble when thought state returns to holding", () => {
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
            state: "busy",
            thought: null,
            thought_state: "holding",
            thought_source: "carry_forward",
            current_command: null,
          },
        ]}
        idlePreviews={{}}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.queryByText("first thought")).toBeNull();
  });
});
