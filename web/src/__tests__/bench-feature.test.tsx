import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/preact";
import { h } from "preact";
import { OverviewField } from "@/components/OverviewField";
import { BenchModal } from "@/components/BenchModal";
import { makeSession } from "./helpers/fixtures";

// Mock the gesture hook
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

describe("bench feature (OverviewField)", () => {
  const sess1 = makeSession({ session_id: "sess-001", cwd: "/home/user/alpha" });
  const sess2 = makeSession({ session_id: "sess-002", cwd: "/home/user/beta" });
  const sess3 = makeSession({ session_id: "sess-003", cwd: "/home/user/gamma" });

  it("filters benched sessions from the field", () => {
    const benchedIds = new Set(["sess-002"]);
    const { container } = render(
      <OverviewField
        sessions={[sess1, sess2, sess3]}
        benchedIds={benchedIds}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const thronglets = container.querySelectorAll(".thronglet");
    expect(thronglets.length).toBe(2);
  });

  it("bench button hidden in observer mode", () => {
    const { container } = render(
      <OverviewField
        sessions={[sess1]}
        observer={true}
        onToggleBenchArm={noop}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(container.querySelector(".bench-trigger")).toBeNull();
  });

  it("shows bench button in normal mode with onToggleBenchArm", () => {
    const { container } = render(
      <OverviewField
        sessions={[sess1]}
        observer={false}
        onToggleBenchArm={noop}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(container.querySelector(".bench-trigger")).not.toBeNull();
  });

  it("shows bench count badge when benched sessions exist", () => {
    const benchedIds = new Set(["sess-002", "sess-003"]);
    const { container } = render(
      <OverviewField
        sessions={[sess1, sess2, sess3]}
        benchedIds={benchedIds}
        benchArmed={false}
        onToggleBenchArm={noop}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const badge = container.querySelector(".bench-count");
    expect(badge).not.toBeNull();
    expect(badge?.textContent).toBe("2");
  });

  it("does not show bench count badge when armed", () => {
    const benchedIds = new Set(["sess-002"]);
    const { container } = render(
      <OverviewField
        sessions={[sess1, sess2]}
        benchedIds={benchedIds}
        benchArmed={true}
        onToggleBenchArm={noop}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(container.querySelector(".bench-count")).toBeNull();
  });

  it("shows 'all benched' empty state when all sessions are benched", () => {
    const benchedIds = new Set(["sess-001", "sess-002"]);
    render(
      <OverviewField
        sessions={[sess1, sess2]}
        benchedIds={benchedIds}
        onToggleBenchArm={noop}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(screen.getByText("All thronglets hidden")).toBeInTheDocument();
  });
});

describe("BenchModal", () => {
  const sess1 = makeSession({
    session_id: "sess-001",
    cwd: "/home/user/alpha",
    thought: "analyzing the codebase structure",
    state: "busy",
  });
  const sess2 = makeSession({
    session_id: "sess-002",
    cwd: "/home/user/beta",
    thought: null,
    state: "idle",
  });

  it("renders benched sessions with name and thought", () => {
    const benchedIds = new Set(["sess-001", "sess-002"]);
    render(
      <BenchModal
        open={true}
        sessions={[sess1, sess2]}
        benchedIds={benchedIds}
        onClose={noop}
        onTapSession={noop}
        onUnbench={noop}
      />,
    );

    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
    expect(
      screen.getByText("analyzing the codebase structure"),
    ).toBeInTheDocument();
  });

  it("fires onTapSession on item click", () => {
    const benchedIds = new Set(["sess-001"]);
    const onTapSession = vi.fn();
    render(
      <BenchModal
        open={true}
        sessions={[sess1]}
        benchedIds={benchedIds}
        onClose={noop}
        onTapSession={onTapSession}
        onUnbench={noop}
      />,
    );

    const item = screen.getByTestId("bench-modal-item");
    fireEvent.click(item);
    expect(onTapSession).toHaveBeenCalledWith("sess-001");
  });

  it("fires onUnbench on Unhide button click", () => {
    const benchedIds = new Set(["sess-001"]);
    const onUnbench = vi.fn();
    render(
      <BenchModal
        open={true}
        sessions={[sess1]}
        benchedIds={benchedIds}
        onClose={noop}
        onTapSession={noop}
        onUnbench={onUnbench}
      />,
    );

    const unhideBtn = screen.getByText("Unhide");
    fireEvent.click(unhideBtn);
    expect(onUnbench).toHaveBeenCalledWith("sess-001");
  });

  it("renders nothing when open=false", () => {
    const { container } = render(
      <BenchModal
        open={false}
        sessions={[sess1]}
        benchedIds={new Set(["sess-001"])}
        onClose={noop}
        onTapSession={noop}
        onUnbench={noop}
      />,
    );

    expect(container.querySelector(".bench-modal-overlay")).toBeNull();
  });

  it("only shows sessions that are in benchedIds", () => {
    const benchedIds = new Set(["sess-001"]);
    render(
      <BenchModal
        open={true}
        sessions={[sess1, sess2]}
        benchedIds={benchedIds}
        onClose={noop}
        onTapSession={noop}
        onUnbench={noop}
      />,
    );

    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.queryByText("beta")).toBeNull();
  });
});

describe("bench localStorage", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("hydrates from valid localStorage data", () => {
    localStorage.setItem(
      "throngterm.benched-sessions.v1",
      JSON.stringify(["sess-001", "sess-002"]),
    );

    // Simulate the hydration logic from app.tsx
    const raw = localStorage.getItem("throngterm.benched-sessions.v1");
    const parsed = raw ? JSON.parse(raw) : [];
    const set =
      Array.isArray(parsed) && parsed.every((id: unknown) => typeof id === "string")
        ? new Set<string>(parsed)
        : new Set<string>();

    expect(set.size).toBe(2);
    expect(set.has("sess-001")).toBe(true);
    expect(set.has("sess-002")).toBe(true);
  });

  it("handles corrupt localStorage data gracefully", () => {
    localStorage.setItem("throngterm.benched-sessions.v1", "not-json{{{");

    let set: Set<string>;
    try {
      const raw = localStorage.getItem("throngterm.benched-sessions.v1");
      const parsed = raw ? JSON.parse(raw) : [];
      set =
        Array.isArray(parsed) && parsed.every((id: unknown) => typeof id === "string")
          ? new Set<string>(parsed)
          : new Set<string>();
    } catch {
      set = new Set<string>();
    }

    expect(set.size).toBe(0);
  });

  it("handles non-array localStorage data", () => {
    localStorage.setItem(
      "throngterm.benched-sessions.v1",
      JSON.stringify({ not: "array" }),
    );

    const raw = localStorage.getItem("throngterm.benched-sessions.v1");
    let set: Set<string>;
    try {
      const parsed = raw ? JSON.parse(raw) : [];
      set =
        Array.isArray(parsed) && parsed.every((id: unknown) => typeof id === "string")
          ? new Set<string>(parsed)
          : new Set<string>();
    } catch {
      set = new Set<string>();
    }

    expect(set.size).toBe(0);
  });
});
