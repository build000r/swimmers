import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, screen } from "@testing-library/preact";
import { h } from "preact";
import { SpawnMenu } from "@/components/SpawnMenu";

const listDirsMock = vi.fn(async () => ({
  path: "/repo",
  entries: [
    { name: "project", has_children: true },
  ],
}));

vi.mock("@/services/api", () => ({
  listDirs: listDirsMock,
}));

describe("SpawnMenu folder tap behavior", () => {
  beforeEach(() => {
    window.localStorage.clear();
    listDirsMock.mockClear();
    listDirsMock.mockResolvedValue({
      path: "/repo",
      entries: [{ name: "project", has_children: true }],
    });
  });

  it("spawns immediately when tapping a folder entry", async () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    render(
      <SpawnMenu x={120} y={160} onSelect={onSelect} onClose={onClose} />,
    );

    await waitFor(() => {
      expect(screen.getByText("project")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText("project"));

    await waitFor(() => {
      expect(onSelect).toHaveBeenCalledWith("/repo/project", "codex");
    });

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(listDirsMock).toHaveBeenCalledTimes(1);
  });
});
