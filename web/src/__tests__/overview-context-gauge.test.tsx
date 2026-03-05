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

describe("OverviewField context battery", () => {
  const noop = () => {};
  const noopCreate = async () => "";

  it("hides floating thronglet nametag while keeping the gauge", () => {
    const session = makeSession({
      session_id: "sess-001",
      cwd: "/home/user/alpha",
      token_count: 50_000,
      context_limit: 200_000,
    });

    const { container } = render(
      <OverviewField
        sessions={[session]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(container.querySelector(".thronglet .thronglet-name")).toBeNull();
    expect(container.querySelector(".thronglet .context-gauge")).toBeInTheDocument();
    expect(container.querySelector(".thronglet .context-gauge-percent")?.textContent).toBe(
      "75% left",
    );
  });

  it("renders remaining-context segments and critical state", () => {
    const { container, rerender } = render(
      <OverviewField
        sessions={[
          makeSession({
            session_id: "sess-001",
            token_count: 50_000,
            context_limit: 200_000,
          }),
        ]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const initialFill = container.querySelector(".context-gauge-fill");
    expect(initialFill).toBeInTheDocument();
    expect(initialFill?.getAttribute("style") ?? "").toContain("--gauge-segments: 6");
    expect(initialFill?.getAttribute("style") ?? "").toContain("width: 75%;");
    expect(container.querySelector(".context-gauge")?.className ?? "").not.toContain(
      "critical",
    );
    expect(container.querySelector(".context-gauge-percent")?.textContent).toBe("75% left");

    rerender(
      <OverviewField
        sessions={[
          makeSession({
            session_id: "sess-001",
            token_count: 170_000,
            context_limit: 200_000,
          }),
        ]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const criticalGauge = container.querySelector(".context-gauge");
    const criticalFill = container.querySelector(".context-gauge-fill");
    expect(criticalGauge?.className ?? "").toContain("critical");
    expect(criticalFill?.getAttribute("style") ?? "").toContain("--gauge-segments: 1");
    expect(criticalFill?.getAttribute("style") ?? "").toContain("width: 12.5%;");
    expect(container.querySelector(".context-gauge-percent")?.textContent).toBe("15% left");
  });

  it("hides the gauge when token usage is unavailable and clamps over-limit values to empty", () => {
    const { container, rerender } = render(
      <OverviewField
        sessions={[
          makeSession({
            session_id: "sess-001",
            token_count: 0,
            context_limit: 200_000,
          }),
        ]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(container.querySelector(".context-gauge")).toBeNull();
    expect(container.querySelector(".context-gauge-percent")).toBeNull();

    rerender(
      <OverviewField
        sessions={[
          makeSession({
            session_id: "sess-001",
            token_count: 250_000,
            context_limit: 200_000,
          }),
        ]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const emptyFill = container.querySelector(".context-gauge-fill");
    expect(emptyFill?.getAttribute("style") ?? "").toContain("--gauge-segments: 0");
    expect(emptyFill?.getAttribute("style") ?? "").toContain("width: 0%;");
    expect(container.querySelector(".context-gauge-percent")?.textContent).toBe("0% left");
  });

  it("does not render a gauge when context limit is zero", () => {
    const session = makeSession({
      session_id: "sess-001",
      token_count: 0,
      context_limit: 0,
    });

    const { container } = render(
      <OverviewField
        sessions={[session]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    expect(container.querySelector(".context-gauge")).toBeNull();
    expect(container.querySelector(".context-gauge-percent")).toBeNull();
  });

  it("renders last invoked skill pill above the gauge when present", () => {
    const session = makeSession({
      session_id: "sess-001",
      last_skill: "describe",
      token_count: 50_000,
      context_limit: 200_000,
    });

    const { container } = render(
      <OverviewField
        sessions={[session]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const pill = container.querySelector(".thronglet .thronglet-skill-pill");
    expect(pill).toBeInTheDocument();
    expect(pill?.textContent).toContain("describe");
    expect(container.querySelector(".thronglet .context-gauge")).toBeInTheDocument();
  });
});
