import { describe, expect, it } from "vitest";
import { resolveTerminalShortcutAction } from "@/lib/terminal-shortcuts";

describe("terminal shortcut policy", () => {
  it("routes accel+C to copy action", () => {
    const action = resolveTerminalShortcutAction(
      { metaKey: true, ctrlKey: false, altKey: false, key: "c" },
      false,
    );
    expect(action).toBe("copy");
  });

  it("routes accel+V to native paste in interactive mode", () => {
    const action = resolveTerminalShortcutAction(
      { metaKey: true, ctrlKey: false, altKey: false, key: "v" },
      false,
    );
    expect(action).toBe("native_paste");
  });

  it("blocks accel+V in observer mode", () => {
    const action = resolveTerminalShortcutAction(
      { metaKey: true, ctrlKey: false, altKey: false, key: "v" },
      true,
    );
    expect(action).toBe("block_paste");
  });

  it("ignores non-accel or alt-modified keys", () => {
    expect(
      resolveTerminalShortcutAction(
        { metaKey: false, ctrlKey: false, altKey: false, key: "v" },
        false,
      ),
    ).toBe("none");
    expect(
      resolveTerminalShortcutAction(
        { metaKey: true, ctrlKey: false, altKey: true, key: "v" },
        false,
      ),
    ).toBe("none");
  });
});
