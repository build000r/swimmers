import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/preact";
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

describe("OverviewField codex gauge", () => {
  const noop = () => {};
  const noopCreate = async () => "";

  it("renders codex usage with dynamic context window limits", () => {
    const { container } = render(
      <OverviewField
        sessions={[
          makeSession({
            session_id: "sess-codex",
            tool: "Codex",
            token_count: 99_735,
            context_limit: 258_400,
          }),
        ]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const fill = container.querySelector(".context-gauge-fill");
    expect(fill).toBeInTheDocument();
    expect(fill?.getAttribute("style") ?? "").toContain("--gauge-segments: 5");
    expect(fill?.getAttribute("style") ?? "").toContain("width: 62.5%;");
    expect(container.querySelector(".context-gauge-percent")?.textContent).toBe("61% left");
  });
});
