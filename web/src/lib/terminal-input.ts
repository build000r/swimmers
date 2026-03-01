export interface TerminalInputEncodingState {
  pendingHighSurrogate: string;
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
