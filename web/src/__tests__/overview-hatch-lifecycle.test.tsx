import { describe, it, expect, vi } from "vitest";
import { render, fireEvent, waitFor, screen } from "@testing-library/preact";
import { h } from "preact";
import { OverviewField } from "@/components/OverviewField";

vi.mock("@/services/api", () => ({
  listDirs: vi.fn(async () => ({
    path: "/",
    entries: [],
  })),
}));

async function spawnFromMenu(container: HTMLElement) {
  const field = container.querySelector(".field") as HTMLDivElement | null;
  expect(field).not.toBeNull();

  fireEvent.click(field!, { button: 0, clientX: 180, clientY: 220 });

  await waitFor(() => {
    expect(container.querySelector(".spawn-menu")).toBeInTheDocument();
  });

  const spawnHere = container.querySelector(".spawn-here") as HTMLButtonElement | null;
  expect(spawnHere).not.toBeNull();
  fireEvent.click(spawnHere!);

  await waitFor(() => {
    expect(container.querySelector(".egg-sprite.dropping")).toBeInTheDocument();
  });
  fireEvent.animationEnd(container.querySelector(".egg-sprite.dropping") as Element);

  await waitFor(() => {
    expect(container.querySelector(".egg-sprite.wobbling")).toBeInTheDocument();
  });
  fireEvent.animationEnd(container.querySelector(".egg-sprite.wobbling") as Element);
}

describe("OverviewField hatch lifecycle", () => {
  it("navigates exactly once when hatch succeeds", async () => {
    const onTapSession = vi.fn();
    const { container } = render(
      <OverviewField
        sessions={[]}
        idlePreviews={{}}
        onTapSession={onTapSession}
        onDragToBottom={() => {}}
        onCreateSession={async () => "sess-new"}
      />,
    );

    await spawnFromMenu(container);

    await waitFor(() => {
      expect(onTapSession).toHaveBeenCalled();
    });

    await new Promise((resolve) => setTimeout(resolve, 500));
    expect(onTapSession).toHaveBeenCalledTimes(1);
    expect(onTapSession).toHaveBeenCalledWith("sess-new");
  });

  it("cleans up hatch when spawn resolves without session id", async () => {
    const onTapSession = vi.fn();
    const { container } = render(
      <OverviewField
        sessions={[]}
        idlePreviews={{}}
        onTapSession={onTapSession}
        onDragToBottom={() => {}}
        onCreateSession={async () => ""}
      />,
    );

    await spawnFromMenu(container);

    expect(screen.queryByText("No sessions yet")).toBeNull();

    await waitFor(() => {
      expect(screen.getByText("No sessions yet")).toBeInTheDocument();
    });
    expect(onTapSession).not.toHaveBeenCalled();
  });
});
