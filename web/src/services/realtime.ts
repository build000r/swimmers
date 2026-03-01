import type {
  ControlEvent,
  SessionStatePayload,
  SessionSkillPayload,
  ThoughtUpdatePayload,
  SessionCreatedPayload,
  SessionDeletedPayload,
  ReplayTruncatedPayload,
  SessionOverloadedPayload,
  SessionSubscriptionPayload,
  ControlErrorPayload,
  TransportHealth,
} from "@/types";
import { Opcodes } from "@/types";

// ---- Binary frame layout ----
// TERMINAL_OUTPUT (server -> client):
//   [0x11] [session_id_len: u16 BE] [session_id UTF-8] [seq: u64 BE] [data: ...]
//
// TERMINAL_INPUT (client -> server):
//   [0x10] [session_id_len: u16 BE] [session_id UTF-8] [data: ...]

const SEQ_LEN = 8;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

export interface TerminalOutputFrame {
  sessionId: string;
  seq: number;
  data: Uint8Array;
}

export interface RealtimeCallbacks {
  onSessionState?: (sessionId: string, payload: SessionStatePayload) => void;
  onSessionTitle?: (sessionId: string, title: string) => void;
  onSessionSkill?: (sessionId: string, payload: SessionSkillPayload) => void;
  onThoughtUpdate?: (sessionId: string, payload: ThoughtUpdatePayload) => void;
  onSessionCreated?: (payload: SessionCreatedPayload) => void;
  onSessionDeleted?: (
    sessionId: string,
    payload: SessionDeletedPayload,
  ) => void;
  onReplayTruncated?: (
    sessionId: string,
    payload: ReplayTruncatedPayload,
  ) => void;
  onSessionOverloaded?: (
    sessionId: string,
    payload: SessionOverloadedPayload,
  ) => void;
  onSessionSubscription?: (
    sessionId: string,
    payload: SessionSubscriptionPayload,
  ) => void;
  onControlError?: (payload: ControlErrorPayload) => void;
  onTerminalOutput?: (frame: TerminalOutputFrame) => void;
  onHealthChange?: (health: TransportHealth) => void;
}

const INITIAL_RECONNECT_MS = 500;
const MAX_RECONNECT_MS = 30_000;
const MAX_INPUT_PAYLOAD_BYTES = 16 * 1024;

export class RealtimeService {
  private ws: WebSocket | null = null;
  private url: string = "";
  private callbacks: RealtimeCallbacks = {};
  private terminalOutputListeners = new Set<
    (frame: TerminalOutputFrame) => void
  >();
  private replayTruncatedListeners = new Set<
    (sessionId: string, payload: ReplayTruncatedPayload) => void
  >();
  private sessionSubscriptionListeners = new Set<
    (sessionId: string, payload: SessionSubscriptionPayload) => void
  >();
  private reconnectMs = INITIAL_RECONNECT_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private intentionalClose = false;
  private _health: TransportHealth = "disconnected";
  private desiredSessionSubscriptions = new Map<string, number | null>();
  private pendingSessionSubscriptions = new Set<string>();
  private activeSessionSubscriptions = new Set<string>();

  get health(): TransportHealth {
    return this._health;
  }

  private setHealth(h: TransportHealth): void {
    if (this._health !== h) {
      this._health = h;
      this.callbacks.onHealthChange?.(h);
    }
  }

  /** Register event callbacks. Can be called before connect(). */
  on(cbs: RealtimeCallbacks): void {
    this.callbacks = { ...this.callbacks, ...cbs };
  }

  /** Subscribe to terminal output frames. Returns an unsubscribe function. */
  subscribeTerminalOutput(
    cb: (frame: TerminalOutputFrame) => void,
  ): () => void {
    this.terminalOutputListeners.add(cb);
    return () => {
      this.terminalOutputListeners.delete(cb);
    };
  }

  /** Subscribe to replay-truncated events. Returns an unsubscribe function. */
  subscribeReplayTruncated(
    cb: (sessionId: string, payload: ReplayTruncatedPayload) => void,
  ): () => void {
    this.replayTruncatedListeners.add(cb);
    return () => {
      this.replayTruncatedListeners.delete(cb);
    };
  }

  /** Subscribe to session lifecycle acknowledgments. */
  subscribeSessionSubscription(
    cb: (sessionId: string, payload: SessionSubscriptionPayload) => void,
  ): () => void {
    this.sessionSubscriptionListeners.add(cb);
    return () => {
      this.sessionSubscriptionListeners.delete(cb);
    };
  }

  /** Open WebSocket to the given URL. */
  connect(url: string): void {
    this.url = url;
    this.intentionalClose = false;
    this.open();
  }

  /** Disconnect and stop reconnecting. */
  disconnect(): void {
    this.intentionalClose = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    this.setHealth("disconnected");
  }

  // ---- Client -> Server: JSON control messages ----

  subscribeSession(sessionId: string, resumeFromSeq?: number): void {
    const normalizedResume =
      typeof resumeFromSeq === "number" && Number.isFinite(resumeFromSeq)
        ? Math.max(0, Math.floor(resumeFromSeq))
        : null;
    const previousResume = this.desiredSessionSubscriptions.get(sessionId);
    const mergedResume = this.mergeResume(previousResume, normalizedResume);
    this.desiredSessionSubscriptions.set(sessionId, mergedResume);

    // Idempotence: if this session is already active (or subscribe is in-flight)
    // don't re-send another subscribe request.
    if (
      this.activeSessionSubscriptions.has(sessionId) ||
      this.pendingSessionSubscriptions.has(sessionId)
    ) {
      return;
    }

    this.sendSubscribeSession(sessionId, mergedResume);
  }

  /** Force a re-subscribe by clearing idempotency guards for this session. */
  forceResubscribe(sessionId: string, resumeFromSeq?: number): void {
    this.activeSessionSubscriptions.delete(sessionId);
    this.pendingSessionSubscriptions.delete(sessionId);
    this.subscribeSession(sessionId, resumeFromSeq);
  }

  unsubscribeSession(sessionId: string): void {
    this.desiredSessionSubscriptions.delete(sessionId);
    this.pendingSessionSubscriptions.delete(sessionId);
    this.activeSessionSubscriptions.delete(sessionId);
    this.sendJson({
      type: "unsubscribe_session",
      payload: { session_id: sessionId },
    });
  }

  sendResize(sessionId: string, cols: number, rows: number): void {
    this.sendJson({
      type: "resize",
      payload: { session_id: sessionId, cols, rows },
    });
  }

  sendDismissAttention(sessionId: string): void {
    this.sendJson({
      type: "dismiss_attention",
      payload: { session_id: sessionId },
    });
  }

  // ---- Client -> Server: binary terminal input ----

  sendInput(sessionId: string, data: Uint8Array): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const idBytes = encoder.encode(sessionId);
    if (idBytes.length > 0xffff) return;
    if (data.length === 0) return;

    for (let offset = 0; offset < data.length; offset += MAX_INPUT_PAYLOAD_BYTES) {
      const chunk = data.subarray(offset, offset + MAX_INPUT_PAYLOAD_BYTES);
      const frame = new Uint8Array(1 + 2 + idBytes.length + chunk.length);
      frame[0] = Opcodes.TERMINAL_INPUT;
      frame[1] = (idBytes.length >> 8) & 0xff;
      frame[2] = idBytes.length & 0xff;
      frame.set(idBytes, 3);
      frame.set(chunk, 3 + idBytes.length);
      this.ws.send(frame);
    }
  }

  // ---- Internal ----

  private open(): void {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }

    const ws = new WebSocket(this.url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;

    ws.onopen = () => {
      this.reconnectMs = INITIAL_RECONNECT_MS;
      this.setHealth("healthy");
      this.pendingSessionSubscriptions.clear();
      this.activeSessionSubscriptions.clear();

      // Rehydrate desired subscriptions after transport reconnects.
      for (const [sessionId, resumeFromSeq] of this.desiredSessionSubscriptions) {
        this.sendSubscribeSession(sessionId, resumeFromSeq);
      }
    };

    ws.onmessage = (ev: MessageEvent) => {
      if (ev.data instanceof ArrayBuffer) {
        this.handleBinary(new Uint8Array(ev.data));
      } else if (typeof ev.data === "string") {
        this.handleJson(ev.data);
      }
    };

    ws.onclose = () => {
      this.setHealth("disconnected");
      this.pendingSessionSubscriptions.clear();
      this.activeSessionSubscriptions.clear();
      if (!this.intentionalClose) {
        this.scheduleReconnect();
      }
    };

    ws.onerror = () => {
      // onclose will fire next; just mark degraded.
      this.setHealth("degraded");
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.open();
    }, this.reconnectMs);
    this.reconnectMs = Math.min(this.reconnectMs * 2, MAX_RECONNECT_MS);
  }

  private sendJson(msg: unknown): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    this.ws.send(JSON.stringify(msg));
  }

  private sendSubscribeSession(
    sessionId: string,
    resumeFromSeq: number | null,
  ): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    this.pendingSessionSubscriptions.add(sessionId);
    this.sendJson({
      type: "subscribe_session",
      payload: {
        session_id: sessionId,
        resume_from_seq: resumeFromSeq,
      },
    });
  }

  private mergeResume(
    currentResume: number | null | undefined,
    nextResume: number | null,
  ): number | null {
    if (currentResume === undefined) return nextResume;
    if (currentResume === null) return nextResume;
    if (nextResume === null) return currentResume;
    return Math.max(currentResume, nextResume);
  }

  private handleBinary(buf: Uint8Array): void {
    if (buf.length < 3) return;
    const opcode = buf[0];

    if (opcode === Opcodes.TERMINAL_OUTPUT) {
      const sessionIdLen = (buf[1] << 8) | buf[2];
      const sessionIdStart = 3;
      const seqStart = sessionIdStart + sessionIdLen;
      const minLen = seqStart + SEQ_LEN;
      if (buf.length < minLen) return;

      const sessionId = decoder.decode(
        buf.subarray(sessionIdStart, sessionIdStart + sessionIdLen),
      );
      const seqView = new DataView(buf.buffer, buf.byteOffset + seqStart, SEQ_LEN);
      // Read as two 32-bit values since JS doesn't have native u64
      const seqHigh = seqView.getUint32(0);
      const seqLow = seqView.getUint32(4);
      const seq = seqHigh * 0x100000000 + seqLow;
      const data = buf.subarray(minLen);

      const frame = { sessionId, seq, data };
      this.callbacks.onTerminalOutput?.(frame);
      for (const listener of this.terminalOutputListeners) {
        listener(frame);
      }
    }
  }

  private handleJson(raw: string): void {
    let msg: ControlEvent;
    try {
      msg = JSON.parse(raw) as ControlEvent;
    } catch {
      return;
    }

    const { event, session_id, payload } = msg;

    switch (event) {
      case "session_state":
        this.callbacks.onSessionState?.(
          session_id,
          payload as SessionStatePayload,
        );
        break;
      case "session_title":
        this.callbacks.onSessionTitle?.(
          session_id,
          (payload as { title: string }).title,
        );
        break;
      case "session_skill":
        this.callbacks.onSessionSkill?.(
          session_id,
          payload as SessionSkillPayload,
        );
        break;
      case "thought_update":
        this.callbacks.onThoughtUpdate?.(
          session_id,
          payload as ThoughtUpdatePayload,
        );
        break;
      case "session_created":
        this.callbacks.onSessionCreated?.(payload as SessionCreatedPayload);
        break;
      case "session_deleted":
        this.callbacks.onSessionDeleted?.(
          session_id,
          payload as SessionDeletedPayload,
        );
        break;
      case "replay_truncated":
        {
          const typed = payload as ReplayTruncatedPayload;
          this.callbacks.onReplayTruncated?.(session_id, typed);
          for (const listener of this.replayTruncatedListeners) {
            listener(session_id, typed);
          }
        }
        break;
      case "session_overloaded":
        this.callbacks.onSessionOverloaded?.(
          session_id,
          payload as SessionOverloadedPayload,
        );
        break;
      case "session_subscription":
        {
          const typed = payload as SessionSubscriptionPayload;
          if (typed.state === "subscribed") {
            this.pendingSessionSubscriptions.delete(session_id);
            this.activeSessionSubscriptions.add(session_id);
          } else {
            this.pendingSessionSubscriptions.delete(session_id);
            this.activeSessionSubscriptions.delete(session_id);
          }
          this.callbacks.onSessionSubscription?.(session_id, typed);
          for (const listener of this.sessionSubscriptionListeners) {
            listener(session_id, typed);
          }
        }
        break;
      case "control_error":
        this.callbacks.onControlError?.(payload as ControlErrorPayload);
        break;
    }
  }
}
