import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copyTextToClipboard } from "@/lib/clipboard";

const clipboardDescriptor = Object.getOwnPropertyDescriptor(
  navigator,
  "clipboard",
);
const execCommandDescriptor = Object.getOwnPropertyDescriptor(
  document,
  "execCommand",
);
const secureContextDescriptor = Object.getOwnPropertyDescriptor(
  window,
  "isSecureContext",
);

afterEach(() => {
  if (clipboardDescriptor) {
    Object.defineProperty(navigator, "clipboard", clipboardDescriptor);
  } else {
    delete (navigator as any).clipboard;
  }

  if (execCommandDescriptor) {
    Object.defineProperty(document, "execCommand", execCommandDescriptor);
  } else {
    delete (document as any).execCommand;
  }

  if (secureContextDescriptor) {
    Object.defineProperty(window, "isSecureContext", secureContextDescriptor);
  } else {
    delete (window as any).isSecureContext;
  }

  vi.restoreAllMocks();
});

function setSecureContext(value: boolean) {
  Object.defineProperty(window, "isSecureContext", {
    configurable: true,
    value,
  });
}

describe("clipboard helpers", () => {
  it("uses navigator.clipboard in secure context", async () => {
    setSecureContext(true);
    const writeText = vi.fn().mockResolvedValue(undefined);
    const exec = vi.fn(() => false);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: exec,
    });

    const copied = await copyTextToClipboard("hello");
    expect(copied).toBe(true);
    expect(writeText).toHaveBeenCalledWith("hello");
    expect(exec).not.toHaveBeenCalled();
  });

  it("falls back to execCommand when navigator writeText fails in secure context", async () => {
    setSecureContext(true);
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    const exec = vi.fn(() => true);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: exec,
    });

    const copied = await copyTextToClipboard("fallback");
    expect(copied).toBe(true);
    expect(writeText).toHaveBeenCalledWith("fallback");
    expect(exec).toHaveBeenCalledWith("copy");
  });

  it("skips clipboard API in non-secure context (HTTP)", async () => {
    setSecureContext(false);
    const writeText = vi.fn().mockResolvedValue(undefined);
    const exec = vi.fn(() => true);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: exec,
    });

    const copied = await copyTextToClipboard("http-copy");
    expect(copied).toBe(true);
    // Clipboard API should NOT have been called
    expect(writeText).not.toHaveBeenCalled();
    // Should have gone straight to execCommand
    expect(exec).toHaveBeenCalledWith("copy");
  });

  it("returns false for empty text", async () => {
    setSecureContext(true);
    const writeText = vi.fn().mockResolvedValue(undefined);
    const exec = vi.fn(() => true);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: exec,
    });

    const copied = await copyTextToClipboard("");
    expect(copied).toBe(false);
    expect(writeText).not.toHaveBeenCalled();
    expect(exec).not.toHaveBeenCalled();
  });
});
