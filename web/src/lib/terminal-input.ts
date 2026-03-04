export interface TerminalInputEncodingState {
  pendingHighSurrogate: string;
}

export interface DispatchTerminalInputOptions {
  observer: boolean;
  chunk: string;
  state: TerminalInputEncodingState;
  encoder: TextEncoder;
  send: (bytes: Uint8Array) => void;
  refocus?: () => void;
  refocusAfterSend?: boolean;
}

function isHighSurrogate(codeUnit: number): boolean {
  return codeUnit >= 0xd800 && codeUnit <= 0xdbff;
}

/**
 * Encodes terminal input while preserving UTF-16 surrogate pairs that may be
 * split across chunks (common with IME/dictation input streams).
 */
export function encodeTerminalInputChunk(
  chunk: string,
  state: TerminalInputEncodingState,
  encoder: TextEncoder,
): Uint8Array | null {
  if (!chunk && !state.pendingHighSurrogate) return null;

  let combined = `${state.pendingHighSurrogate}${chunk}`;
  state.pendingHighSurrogate = "";
  if (!combined) return null;

  const lastCodeUnit = combined.charCodeAt(combined.length - 1);
  if (isHighSurrogate(lastCodeUnit)) {
    state.pendingHighSurrogate = combined.slice(-1);
    combined = combined.slice(0, -1);
  }

  if (!combined) return null;
  return encoder.encode(combined);
}

/**
 * Normalizes and dispatches terminal input to transport, optionally re-focusing
 * the terminal for pointer/tap initiated actions.
 */
export function dispatchTerminalInput({
  observer,
  chunk,
  state,
  encoder,
  send,
  refocus,
  refocusAfterSend = false,
}: DispatchTerminalInputOptions): boolean {
  if (observer || !chunk) return false;
  const encoded = encodeTerminalInputChunk(chunk, state, encoder);
  if (!encoded || encoded.length === 0) return false;
  send(encoded);
  if (refocusAfterSend) {
    refocus?.();
  }
  return true;
}
