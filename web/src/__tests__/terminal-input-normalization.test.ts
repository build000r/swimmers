import { describe, expect, it } from "vitest";
import {
  encodeTerminalInputChunk,
  type TerminalInputEncodingState,
} from "@/lib/terminal-input";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function newState(): TerminalInputEncodingState {
  return { pendingHighSurrogate: "" };
}

function decode(bytes: Uint8Array | null): string {
  return bytes ? decoder.decode(bytes) : "";
}

describe("terminal input normalization", () => {
  it("encodes plain text unchanged", () => {
    const state = newState();
    const bytes = encodeTerminalInputChunk("ls -la\n", state, encoder);
    expect(decode(bytes)).toBe("ls -la\n");
    expect(state.pendingHighSurrogate).toBe("");
  });

  it("buffers a dangling high surrogate until the next chunk", () => {
    const state = newState();

    const first = encodeTerminalInputChunk("\uD83D", state, encoder);
    expect(first).toBeNull();
    expect(state.pendingHighSurrogate).toBe("\uD83D");

    const second = encodeTerminalInputChunk("\uDE80", state, encoder);
    expect(decode(second)).toBe("🚀");
    expect(state.pendingHighSurrogate).toBe("");
  });

  it("preserves ordering across sequential chunks", () => {
    const state = newState();
    const first = decode(encodeTerminalInputChunk("ship ", state, encoder));
    const second = decode(encodeTerminalInputChunk("it ", state, encoder));
    const third = decode(encodeTerminalInputChunk("today\n", state, encoder));

    expect(`${first}${second}${third}`).toBe("ship it today\n");
    expect(state.pendingHighSurrogate).toBe("");
  });

  it("passes through dictation-style unicode text", () => {
    const state = newState();
    const bytes = encodeTerminalInputChunk(
      "deploy café — ready ✅\n",
      state,
      encoder,
    );
    expect(decode(bytes)).toBe("deploy café — ready ✅\n");
    expect(state.pendingHighSurrogate).toBe("");
  });
});
