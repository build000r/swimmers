import type {
  ControlEvent,
  SessionStatePayload,
  ThoughtUpdatePayload,
  SessionCreatedPayload,
  SessionDeletedPayload,
  ReplayTruncatedPayload,
  SessionOverloadedPayload,
  ControlErrorPayload,
  TransportHealth,
} from "@/types";
import { Opcodes } from "@/types";

// ---- Binary frame layout ----
// TERMINAL_OUTPUT (server -> client):
//   [0x11] [session_id: 36 bytes UTF-8] [seq: 8 bytes big-endian u64] [data: ...]
//
// TERMINAL_INPUT (client -> server):
//   [0x10] [session_id: 36 bytes UTF-8] [data: ...]

const SESSION_ID_LEN = 36;
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
  onControlError?: (payload: ControlErrorPayload) => void;
  onTerminalOutput?: (frame: TerminalOutputFrame) => void;
  onHealthChange?: (health: TransportHealth) => void;
}

const INITIAL_RECONNECT_MS = 500;
const MAX_RECONNECT_MS = 30_000;

export class RealtimeService {
  private ws: WebSocket | null = null;
  private url: string = "";
  private callbacks: RealtimeCallbacks = {};
  private reconnectMs = INITIAL_RECONNECT_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private intentionalClose = false;
  private _health: TransportHealth = "disconnected";

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
    this.sendJson({
      type: "subscribe_session",
      payload: {
        session_id: sessionId,
        resume_from_seq: resumeFromSeq ?? null,
      },
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
    const frame = new Uint8Array(1 + SESSION_ID_LEN + data.length);
    frame[0] = Opcodes.TERMINAL_INPUT;
    frame.set(idBytes.subarray(0, SESSION_ID_LEN), 1);
    frame.set(data, 1 + SESSION_ID_LEN);
    this.ws.send(frame);
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

  private handleBinary(buf: Uint8Array): void {
    if (buf.length < 1) return;
    const opcode = buf[0];

    if (opcode === Opcodes.TERMINAL_OUTPUT) {
      // [opcode(1)] [session_id(36)] [seq(8)] [data(...)]
      const minLen = 1 + SESSION_ID_LEN + SEQ_LEN;
      if (buf.length < minLen) return;

      const sessionId = decoder.decode(buf.subarray(1, 1 + SESSION_ID_LEN));
      const seqView = new DataView(
        buf.buffer,
        buf.byteOffset + 1 + SESSION_ID_LEN,
        SEQ_LEN,
      );
      // Read as two 32-bit values since JS doesn't have native u64
      const seqHigh = seqView.getUint32(0);
      const seqLow = seqView.getUint32(4);
      const seq = seqHigh * 0x100000000 + seqLow;
      const data = buf.subarray(minLen);

      this.callbacks.onTerminalOutput?.({ sessionId, seq, data });
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
        this.callbacks.onReplayTruncated?.(
          session_id,
          payload as ReplayTruncatedPayload,
        );
        break;
      case "session_overloaded":
        this.callbacks.onSessionOverloaded?.(
          session_id,
          payload as SessionOverloadedPayload,
        );
        break;
      case "control_error":
        this.callbacks.onControlError?.(payload as ControlErrorPayload);
        break;
    }
  }
}
