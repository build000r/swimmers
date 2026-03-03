import { describe, it, expect } from "vitest";
import { render } from "@testing-library/preact";
import { h } from "preact";
import { OverviewField } from "@/components/OverviewField";
import { makeSession } from "./helpers/fixtures";

describe("OverviewField exited session rendering", () => {
  const noop = () => {};
  const noopCreate = async () => "";

  it("renders exited sessions as exited sprites, not eggs", () => {
    const session = makeSession({
      session_id: "sess-exited",
      state: "exited",
      tool: null,
    });

    const { container } = render(
      <OverviewField
        sessions={[session]}
        onTapSession={noop}
        onDragToBottom={noop}
        onCreateSession={noopCreate}
      />,
    );

    const thronglet = container.querySelector(".thronglet");
    expect(thronglet).toBeInTheDocument();
    expect(thronglet).toHaveClass("exited");
    expect(thronglet).not.toHaveClass("egg");
    expect(container.querySelector(".egg-idle-sprite")).toBeNull();
    expect(container.querySelector(".thronglet-sprite")).toBeInTheDocument();
  });
});
