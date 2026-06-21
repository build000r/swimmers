import {
  buildSessionSocketUrl,
  decodeTerminalOutputFrame,
  selectedSessionConnectionPlan,
  sessionSocketAttachPlan,
  sessionSocketAttachStatePlan,
  sessionSocketAuthMessageForToken,
  sessionSocketCloseExecutionPlan,
  sessionSocketClosePlan,
  sessionSocketErrorPlan,
  sessionSocketMessageExecutionPlan,
  sessionSocketOpenExecutionPlan,
  sessionSocketOpenStatusPlan,
  sessionSocketReconnectPlan,
} from "./terminal_protocol.js";
import { normalizeTerminalServerFrame } from "./contracts.js";

export function createSessionSocketController(runtime) {
  const { state } = runtime;

  async function connectSelectedSession() {
    await runtime.setupHudSurface();

    const session = runtime.currentSession();
    if (selectedSessionConnectionPlan({ session }).type === "teardown_terminal") {
      runtime.teardownTerminal();
      return;
    }

    const existingSocket = state.ws || null;
    if (existingSocket && existingSocket.sessionId !== session.session_id) {
      runtime.disconnectSocket();
    }

    await runtime.setupTerminalSurface();
    const refreshedSession = runtime.currentSession();
    if (!refreshedSession || refreshedSession.session_id !== session.session_id) {
      return;
    }
    const plan = selectedSessionConnectionPlan({
      session: refreshedSession,
      terminalSurfaceChecked: true,
      hasTerminal: Boolean(state.terminal),
      terminalFallbackActive: state.terminalFallbackActive,
      ws: state.ws,
      openReadyState: runtime.WebSocketClass.OPEN,
    });
    if (plan.type === "remote_snapshot_fallback") {
      if (state.ws) {
        runtime.disconnectSocket();
      }
      runtime.setConnectionStatus(plan.status, true);
      return;
    }
    if (plan.type !== "connect_socket") return;

    runtime.disconnectSocket();
    const generation = state.connectionGeneration;
    const url = sessionSocketUrl(session);
    const attachPlan = sessionSocketAttachPlan(url);

    const ws = new runtime.WebSocketClass(url);
    attachSelectedSessionSocket(ws, plan, attachPlan);
    ws.onopen = () => handleSelectedSessionSocketOpen(ws, generation);
    ws.onmessage = (event) => handleSelectedSessionSocketMessage(ws, generation, event);
    ws.onclose = () => handleSelectedSessionSocketClose(generation);
    ws.onerror = () => handleSelectedSessionSocketError();
  }

  function selectedSessionSocketContext(ws, generation) {
    return {
      generation,
      currentGeneration: state.connectionGeneration,
      currentSocketMatches: generation === state.connectionGeneration && state.ws === ws,
    };
  }

  function attachSelectedSessionSocket(ws, plan, attachPlan) {
    const attach = sessionSocketAttachStatePlan(plan, attachPlan);
    [ws.binaryType, ws.sessionId, ws.framedOutput] = [
      attach.binaryType,
      attach.sessionId,
      attach.framedOutput,
    ];
    state.ws = ws;
    state.readOnly = attach.readOnly;
    runtime.syncWriteAccess();
    runtime.setConnectionStatus(attach.status);
  }

  function handleSelectedSessionSocketOpen(ws, generation) {
    const openPlan = sessionSocketOpenExecutionPlan(selectedSessionSocketContext(ws, generation));
    if (openPlan.type === "close_stale") {
      ws.close();
      return;
    }
    const statusPlan = sessionSocketOpenStatusPlan(sendSessionSocketAuth(ws));
    if (openPlan.resizeTerminal) runtime.measureAndResizeSurface(true, true);
    if (openPlan.resetReconnectAttempt) state.reconnectAttempt = 0;
    runtime.setConnectionStatus(statusPlan.status);
    if (openPlan.scheduleRefresh) runtime.scheduleSessionRefresh();
  }

  function handleSelectedSessionSocketMessage(ws, generation, event) {
    const messagePlan = sessionSocketMessageExecutionPlan({
      ...selectedSessionSocketContext(ws, generation),
      data: event.data,
    });
    if (messagePlan.type === "ignore") return;
    if (messagePlan.type === "handle_text") {
      handleSocketText(messagePlan.text);
      return;
    }
    runtime.feedTerminalBytes(terminalPayloadFromSocketBytes(messagePlan.bytes, ws));
  }

  function handleSelectedSessionSocketClose(generation) {
    if (
      sessionSocketClosePlan({
        generation,
        currentGeneration: state.connectionGeneration,
      }).type === "ignore"
    ) {
      return;
    }
    const closePlan = sessionSocketCloseExecutionPlan(runtime.reconnectDelayMs());
    if (closePlan.incrementReconnectAttempt) state.reconnectAttempt += 1;
    runtime.setConnectionStatus(closePlan.status, true);
    if (closePlan.scheduleRefresh) runtime.scheduleSessionRefresh();
    state.reconnectTimer = runtime.window.setTimeout(
      () => runSelectedSessionSocketReconnect(generation),
      closePlan.delayMs,
    );
  }

  function runSelectedSessionSocketReconnect(generation) {
    state.reconnectTimer = null;
    if (
      sessionSocketReconnectPlan({
        generation,
        currentGeneration: state.connectionGeneration,
        hasCurrentSession:
          generation === state.connectionGeneration && Boolean(runtime.currentSession()),
      }).type !== "reconnect"
    ) {
      return;
    }
    connectSelectedSession();
  }

  function handleSelectedSessionSocketError() {
    const errorPlan = sessionSocketErrorPlan();
    runtime.setConnectionStatus(errorPlan.status, errorPlan.muted);
  }

  function sessionSocketUrl(session) {
    return buildSessionSocketUrl(
      session,
      runtime.window.location,
      state.lastTerminalSeqBySession.get(session.session_id),
    );
  }

  function sessionSocketAuthMessage() {
    return sessionSocketAuthMessageForToken(state.token);
  }

  function sendSessionSocketAuth(ws) {
    const message = sessionSocketAuthMessage();
    if (!message || !ws || ws.readyState !== runtime.WebSocketClass.OPEN) {
      return false;
    }
    ws.send(message);
    return true;
  }

  function terminalPayloadFromSocketBytes(bytes, ws = state.ws) {
    if (!(bytes instanceof Uint8Array) || !ws?.framedOutput) {
      return bytes;
    }
    const frame = decodeTerminalOutputFrame(bytes);
    if (!frame) {
      // Framed mode but the bytes did not decode (too short / wrong opcode):
      // drop them instead of feeding the raw opcode+seq header as terminal
      // output. feedTerminalBytes treats a non-Uint8Array as a no-op.
      return null;
    }
    if (ws.sessionId) {
      state.lastTerminalSeqBySession.set(ws.sessionId, frame.seq);
    }
    return frame.payload;
  }

  function handleSocketText(raw) {
    try {
      const message = normalizeTerminalServerFrame(JSON.parse(raw));
      switch (message.type) {
        case "ready":
          state.readOnly = Boolean(message.readOnly);
          runtime.setConnectionStatus("attached");
          runtime.setModeStatus(state.readOnly ? "observer" : "operator", !state.token);
          runtime.syncWriteAccess();
          runtime.syncTerminalTools();
          if (message.summary) {
            runtime.mergeSummary(message.summary);
          }
          runtime.scheduleSessionRefresh();
          break;
        case "replay_truncated":
          runtime.setConnectionStatus("partial replay", true);
          break;
        case "error":
          runtime.setConnectionStatus(message.code || "error", true);
          break;
        case "overloaded":
          runtime.setConnectionStatus(
            `server overloaded; input disabled; retrying in ${Math.ceil(
              (message.retryAfterMs || 4000) / 1000,
            )}s`,
            true,
          );
          break;
        case "input_ack":
          runtime.handleInputAck(message);
          break;
        case "control_event":
          runtime.applyControlEvent(message);
          break;
        case "lifecycle_event":
          runtime.applyLifecycleEvent(message);
          break;
        case "event_stream_lagged":
          runtime.setConnectionStatus("event stream lagged", true);
          void runtime.refreshSessions();
          break;
        case "pong":
          break;
        default:
          break;
      }
    } catch (_) {
      // Ignore malformed transport diagnostics.
    }
  }

  return {
    connectSelectedSession,
    sessionSocketUrl,
    sessionSocketAuthMessage,
    terminalPayloadFromSocketBytes,
    handleSocketText,
  };
}
