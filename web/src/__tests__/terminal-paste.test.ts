import { describe, expect, it, vi } from "vitest";
import { handleTerminalPasteEvent } from "@/lib/terminal-paste";

type MockFn = ReturnType<typeof vi.fn>;

interface FakePasteEvent {
  defaultPrevented: boolean;
  clipboardData: { getData(type: string): string } | null;
  preventDefault: () => void;
  preventDefaultSpy: MockFn;
}

function makeEvent(options?: {
  defaultPrevented?: boolean;
  text?: string;
}): FakePasteEvent {
  const text = options?.text ?? "";
  const preventDefaultSpy = vi.fn();
  return {
    defaultPrevented: options?.defaultPrevented ?? false,
    clipboardData: {
      getData: vi.fn((kind: string) => (kind === "text" ? text : "")) as (
        type: string,
      ) => string,
    },
    preventDefault: () => preventDefaultSpy(),
    preventDefaultSpy,
  };
}

describe("terminal paste event policy", () => {
  it("blocks paste while observer mode is active", () => {
    const event = makeEvent({ text: "echo hi" });
    const paste = vi.fn();
    const notifyPasted = vi.fn();

    handleTerminalPasteEvent({
      observer: true,
      event,
      paste,
      notifyPasted,
    });

    expect(event.preventDefaultSpy).toHaveBeenCalledTimes(1);
    expect(paste).not.toHaveBeenCalled();
    expect(notifyPasted).not.toHaveBeenCalled();
  });

  it("ignores paste events already handled by native/xterm listeners", () => {
    const event = makeEvent({ defaultPrevented: true, text: "duplicate?" });
    const paste = vi.fn();
    const notifyPasted = vi.fn();

    handleTerminalPasteEvent({
      observer: false,
      event,
      paste,
      notifyPasted,
    });

    expect(event.preventDefaultSpy).not.toHaveBeenCalled();
    expect(paste).not.toHaveBeenCalled();
    expect(notifyPasted).not.toHaveBeenCalled();
  });

  it("ignores empty clipboard payloads", () => {
    const event = makeEvent({ text: "" });
    const paste = vi.fn();
    const notifyPasted = vi.fn();

    handleTerminalPasteEvent({
      observer: false,
      event,
      paste,
      notifyPasted,
    });

    expect(event.preventDefaultSpy).not.toHaveBeenCalled();
    expect(paste).not.toHaveBeenCalled();
    expect(notifyPasted).not.toHaveBeenCalled();
  });

  it("pastes exactly once for valid interactive paste events", () => {
    const event = makeEvent({ text: "npm test\n" });
    const paste = vi.fn();
    const notifyPasted = vi.fn();

    handleTerminalPasteEvent({
      observer: false,
      event,
      paste,
      notifyPasted,
    });

    expect(event.preventDefaultSpy).toHaveBeenCalledTimes(1);
    expect(paste).toHaveBeenCalledTimes(1);
    expect(paste).toHaveBeenCalledWith("npm test\n");
    expect(notifyPasted).toHaveBeenCalledTimes(1);
  });

  it("uses the client paste event payload without direct clipboard API reads", () => {
    const clipboardDescriptor = Object.getOwnPropertyDescriptor(
      navigator,
      "clipboard",
    );
    const readText = vi.fn().mockResolvedValue("never");
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { readText, writeText },
    });

    try {
      const event = makeEvent({ text: "from-device-b" });
      const paste = vi.fn();
      const notifyPasted = vi.fn();

      handleTerminalPasteEvent({
        observer: false,
        event,
        paste,
        notifyPasted,
      });

      expect(paste).toHaveBeenCalledWith("from-device-b");
      expect(readText).not.toHaveBeenCalled();
      expect(writeText).not.toHaveBeenCalled();
    } finally {
      if (clipboardDescriptor) {
        Object.defineProperty(navigator, "clipboard", clipboardDescriptor);
      } else {
        delete (navigator as { clipboard?: unknown }).clipboard;
      }
    }
  });
});
