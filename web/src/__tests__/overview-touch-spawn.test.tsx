import { describe, it, expect, vi } from "vitest";
import { render, fireEvent, waitFor } from "@testing-library/preact";
import { h } from "preact";
import { OverviewField } from "@/components/OverviewField";

vi.mock("@/services/api", () => ({
  listDirs: vi.fn(async () => ({
    path: "/",
    entries: [],
  })),
}));

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

describe("OverviewField touch spawn", () => {
  it("opens the spawn menu from touchend changedTouches coordinates", async () => {
    const { container } = render(
      <OverviewField
        sessions={[]}
        idlePreviews={{}}
        onTapSession={() => {}}
        onDragToBottom={() => {}}
        onCreateSession={async () => ""}
      />,
    );

    const field = container.querySelector(".field") as HTMLDivElement | null;
    expect(field).not.toBeNull();

    fireEvent.touchEnd(field!, {
      touches: [],
      changedTouches: [{ clientX: 160, clientY: 200 }],
    });

    await waitFor(() => {
      expect(container.querySelector(".spawn-menu")).toBeInTheDocument();
    });
  });
});
