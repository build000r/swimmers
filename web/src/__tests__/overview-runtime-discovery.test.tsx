import { describe, it, expect, vi } from "vitest";
import { render, waitFor } from "@testing-library/preact";
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

const noop = () => {};
const noopCreate = async () => "";

describe("OverviewField runtime discovery visibility", () => {
  it("renders a newly discovered external session and clears empty state", async () => {
    const sess1 = makeSession({ session_id: "sess_1" });
    const sess12 = makeSession({
      session_id: "sess_12",
      tmux_name: "codex-20260302-162713",
    });

    const view = render(
      <OverviewField
        sessions={[sess1]}
        benchedIds={new Set()}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    await waitFor(() => {
      expect(view.container.querySelectorAll(".thronglet")).toHaveLength(1);
    });

    view.rerender(
      <OverviewField
        sessions={[sess1, sess12]}
        benchedIds={new Set()}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    await waitFor(() => {
      expect(view.container.querySelectorAll(".thronglet")).toHaveLength(2);
    });

    expect(view.queryByText("No sessions yet")).toBeNull();
  });
});
