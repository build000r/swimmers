import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, screen } from "@testing-library/preact";
import { h } from "preact";
import { SpawnMenu } from "@/components/SpawnMenu";

const listDirsMock = vi.fn();
const restartDirServicesMock = vi.fn();

vi.mock("@/services/api", () => ({
  listDirs: listDirsMock,
  restartDirServices: restartDirServicesMock,
}));

describe("SpawnMenu restart actions", () => {
  beforeEach(() => {
    window.localStorage.clear();
    listDirsMock.mockReset();
    restartDirServicesMock.mockReset();
  });

  it("shows restart for running folders and triggers restart without spawning", async () => {
    listDirsMock.mockResolvedValue({
      path: "/repo",
      entries: [{ name: "project", has_children: true, is_running: true }],
    });
    restartDirServicesMock.mockResolvedValue({
      ok: true,
      path: "/repo/project",
      services: ["project"],
    });

    const onSelect = vi.fn();
    render(<SpawnMenu x={140} y={180} onSelect={onSelect} onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText("project")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByLabelText("Restart project"));

    await waitFor(() => {
      expect(restartDirServicesMock).toHaveBeenCalledWith("/repo/project");
    });
    expect(onSelect).not.toHaveBeenCalled();
    expect(listDirsMock).toHaveBeenCalled();
  });

  it("hides restart for non-running folders", async () => {
    listDirsMock.mockResolvedValue({
      path: "/repo",
      entries: [{ name: "project", has_children: true, is_running: false }],
    });

    render(<SpawnMenu x={140} y={180} onSelect={() => {}} onClose={() => {}} />);

    await waitFor(() => {
      expect(screen.getByText("project")).toBeInTheDocument();
    });

    expect(screen.queryByLabelText("Restart project")).toBeNull();
  });
});
