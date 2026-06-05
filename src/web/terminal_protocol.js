export const TERMINAL_OUTPUT_OPCODE = 0x11;

export function buildSessionSocketUrl(session, location, resumeFromSeq = "") {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const url = new URL(`${protocol}//${location.host}/ws/sessions/${encodeURIComponent(session.session_id)}`);
  url.searchParams.set("framed", "1");
  if (resumeFromSeq && /^\d+$/.test(String(resumeFromSeq)) && String(resumeFromSeq) !== "0") {
    url.searchParams.set("resume_from_seq", String(resumeFromSeq));
  }
  return url;
}

export function sessionSocketAuthMessageForToken(token) {
  const normalized = String(token || "").trim();
  if (!normalized) {
    return null;
  }
  return JSON.stringify({ type: "auth", token: normalized });
}

export function selectedSessionConnectionPlan(context = {}) {
  const session = context.session || null;
  if (!session) {
    return { type: "teardown_terminal" };
  }
  const sessionId = session.session_id;
  if (!context.terminalSurfaceChecked) {
    return { type: "setup_terminal", sessionId };
  }
  if (!context.hasTerminal && !context.terminalFallbackActive) {
    return { type: "await_terminal_surface", sessionId };
  }
  const ws = context.ws || null;
  if (ws && ws.readyState <= context.openReadyState && ws.sessionId === sessionId) {
    return { type: "reuse_socket", sessionId };
  }
  return { type: "connect_socket", sessionId };
}

export function sessionSocketAttachPlan(url) {
  const resumeFromSeq = url.searchParams.get("resume_from_seq") || "";
  const framedOutput = url.searchParams.get("framed") === "1";
  return {
    resumeFromSeq,
    framedOutput,
    status: resumeFromSeq ? `connecting; resuming from seq ${resumeFromSeq}` : "connecting; input disabled",
  };
}

export function sessionSocketAttachStatePlan(connectionPlan = {}, attachPlan = {}) {
  return {
    type: "attach_socket",
    binaryType: "arraybuffer",
    sessionId: connectionPlan.sessionId,
    framedOutput: Boolean(attachPlan.framedOutput),
    readOnly: true,
    status: attachPlan.status,
  };
}

export function sessionSocketOpenPlan(context = {}) {
  if (context.generation !== context.currentGeneration || !context.currentSocketMatches) {
    return { type: "close_stale" };
  }
  return { type: "attach" };
}

export function sessionSocketOpenExecutionPlan(context = {}) {
  if (sessionSocketOpenPlan(context).type === "close_stale") {
    return { type: "close_stale", closeSocket: true };
  }
  return {
    type: "attach",
    sendAuth: true,
    resizeTerminal: true,
    resetReconnectAttempt: true,
    scheduleRefresh: true,
  };
}

export function sessionSocketOpenStatus(sentAuth) {
  return sentAuth ? "authenticating; input disabled" : "attached";
}

export function sessionSocketOpenStatusPlan(sentAuth) {
  return { type: "status", status: sessionSocketOpenStatus(sentAuth) };
}

export function sessionSocketMessagePlan(context = {}) {
  if (context.generation !== context.currentGeneration || !context.currentSocketMatches) {
    return { type: "ignore" };
  }
  if (typeof context.data === "string") {
    return { type: "handle_text", text: context.data };
  }
  return { type: "feed_binary", data: context.data };
}

export function sessionSocketMessageExecutionPlan(context = {}) {
  const plan = sessionSocketMessagePlan(context);
  if (plan.type !== "feed_binary") {
    return plan;
  }
  return { type: "feed_binary", data: plan.data, bytes: new Uint8Array(plan.data) };
}

export function sessionSocketClosePlan(context = {}) {
  if (context.generation !== context.currentGeneration) {
    return { type: "ignore" };
  }
  return { type: "schedule_reconnect" };
}

export function sessionSocketReconnectStatus(delayMs) {
  return `disconnected; input disabled; retrying in ${Math.ceil(delayMs / 1000)}s`;
}

export function sessionSocketCloseExecutionPlan(delayMs) {
  return {
    type: "schedule_reconnect",
    incrementReconnectAttempt: true,
    status: sessionSocketReconnectStatus(delayMs),
    scheduleRefresh: true,
    delayMs,
  };
}

export function sessionSocketReconnectPlan(context = {}) {
  if (context.generation !== context.currentGeneration || !context.hasCurrentSession) {
    return { type: "ignore" };
  }
  return { type: "reconnect" };
}

export function sessionSocketErrorPlan() {
  return { type: "set_status", status: "attach failed; input disabled", muted: true };
}

export function decodeTerminalOutputFrame(bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength < 9 || bytes[0] !== TERMINAL_OUTPUT_OPCODE) {
    return null;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const high = view.getUint32(1);
  const low = view.getUint32(5);
  const seq = readUint64Decimal(high, low);
  return {
    seq,
    payload: bytes.subarray(9),
  };
}

export function readUint64Decimal(high, low) {
  if (typeof BigInt === "function") {
    return ((BigInt(high) << 32n) | BigInt(low)).toString();
  }
  const numeric = high * 4294967296 + low;
  return Number.isSafeInteger(numeric) ? String(numeric) : "";
}

export function fallbackTextForKeyEvent(event) {
  if (!event || event.kind !== "key" || event.phase !== "down") {
    return "";
  }

  const key = typeof event.key === "string" ? event.key : "";
  const mods = Number(event.mods) || 0;
  const shift = (mods & 1) !== 0;
  const alt = (mods & 2) !== 0;
  const ctrl = (mods & 4) !== 0;
  const prefix = alt ? "\x1b" : "";

  if (ctrl && key.length === 1) {
    const upper = key.toUpperCase();
    const code = upper.charCodeAt(0);
    if (code >= 64 && code <= 95) {
      return prefix + String.fromCharCode(code - 64);
    }
  }

  if (!ctrl && key.length === 1) {
    return prefix + key;
  }

  switch (key) {
    case "Enter":
      return "\r";
    case "Backspace":
      return "\x7f";
    case "Delete":
      return "\x1b[3~";
    case "Tab":
      return shift ? "\x1b[Z" : "\t";
    case "Escape":
      return "\x1b";
    case "ArrowUp":
      return "\x1b[A";
    case "ArrowDown":
      return "\x1b[B";
    case "ArrowRight":
      return "\x1b[C";
    case "ArrowLeft":
      return "\x1b[D";
    case "Home":
      return "\x1b[H";
    case "End":
      return "\x1b[F";
    case "PageUp":
      return "\x1b[5~";
    case "PageDown":
      return "\x1b[6~";
    default:
      return "";
  }
}

export function terminalControlKeyEvent(actionId) {
  switch (String(actionId || "")) {
    case "ctrl-c":
      return { key: "c", code: "KeyC", mods: 4, label: "Ctrl-C" };
    case "escape":
      return { key: "Escape", code: "Escape", mods: 0, label: "Esc" };
    case "tab":
      return { key: "Tab", code: "Tab", mods: 0, label: "Tab" };
    case "arrow-up":
      return { key: "ArrowUp", code: "ArrowUp", mods: 0, label: "Up" };
    case "arrow-down":
      return { key: "ArrowDown", code: "ArrowDown", mods: 0, label: "Down" };
    case "arrow-left":
      return { key: "ArrowLeft", code: "ArrowLeft", mods: 0, label: "Left" };
    case "arrow-right":
      return { key: "ArrowRight", code: "ArrowRight", mods: 0, label: "Right" };
    case "home":
      return { key: "Home", code: "Home", mods: 0, label: "Home" };
    case "end":
      return { key: "End", code: "End", mods: 0, label: "End" };
    case "page-up":
      return { key: "PageUp", code: "PageUp", mods: 0, label: "PgUp" };
    case "page-down":
      return { key: "PageDown", code: "PageDown", mods: 0, label: "PgDn" };
    default:
      return null;
  }
}

export function keyModifiers(event) {
  return (event.shiftKey ? 1 : 0) | (event.altKey ? 2 : 0) | (event.ctrlKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}
