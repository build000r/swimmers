import { describe, it, expect, beforeEach, vi } from "vitest";
import { RealtimeService } from "@/services/realtime";
import type { TerminalOutputFrame } from "@/services/realtime";
import { Opcodes } from "@/types";
import type {
  ReplayTruncatedPayload,
  SessionSubscriptionPayload,
} from "@/types";

const encoder = new TextEncoder();
const MAX_INPUT_PAYLOAD_BYTES = 16 * 1024;

/**
 * Build a binary TERMINAL_OUTPUT frame matching the server wire format:
 *   [0x11] [session_id_len: u16 BE] [session_id UTF-8] [seq: u64 BE] [data: ...]
 */
function buildOutputFrame(
  sessionId: string,
  seq: number,
  data: Uint8Array,
): ArrayBuffer {
  const idBytes = encoder.encode(sessionId);
  const frame = new Uint8Array(1 + 2 + idBytes.length + 8 + data.length);
  frame[0] = Opcodes.TERMINAL_OUTPUT;
  frame[1] = (idBytes.length >> 8) & 0xff;
  frame[2] = idBytes.length & 0xff;
  frame.set(idBytes, 3);
  // seq as u64 BE
  const seqHigh = Math.floor(seq / 0x100000000);
  const seqLow = seq & 0xffffffff;
  const view = new DataView(frame.buffer, 3 + idBytes.length, 8);
  view.setUint32(0, seqHigh);
  view.setUint32(4, seqLow);
  frame.set(data, 3 + idBytes.length + 8);
  return frame.buffer;
}

/**
 * Build a JSON control event string.
 */
function buildControlEvent(
  event: string,
  sessionId: string,
  payload: unknown,
): string {
  return JSON.stringify({ event, session_id: sessionId, payload });
}

function decodeInputFrame(frame: Uint8Array): {
  sessionId: string;
  payload: Uint8Array;
} {
  expect(frame[0]).toBe(Opcodes.TERMINAL_INPUT);
  const idLen = (frame[1] << 8) | frame[2];
  const idStart = 3;
  const idEnd = idStart + idLen;
  const sessionId = new TextDecoder().decode(frame.slice(idStart, idEnd));
  return {
    sessionId,
    payload: frame.slice(idEnd),
  };
}

describe("RealtimeService", () => {
  let service: RealtimeService;
  let mockWs: any;

  beforeEach(() => {
    service = new RealtimeService();

    // Intercept WebSocket construction to capture the instance
    const OrigWs = globalThis.WebSocket;
    (globalThis as any).WebSocket = class extends OrigWs {
      constructor(url: string) {
        super(url);
        mockWs = this;
      }
    };

    service.connect("ws://localhost:3210/v1/realtime");

    // Restore original WebSocket
    (globalThis as any).WebSocket = OrigWs;
  });

  describe("terminal output dispatch", () => {
    it("dispatches binary TERMINAL_OUTPUT to subscribed listeners", () => {
      const frames: TerminalOutputFrame[] = [];
      service.subscribeTerminalOutput((frame) => frames.push(frame));

      // Simulate open
      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      // Simulate binary message
      const data = encoder.encode("hello world");
      const buf = buildOutputFrame("sess-001", 42, data);
      mockWs.onmessage?.({ data: buf } as MessageEvent);

      expect(frames).toHaveLength(1);
      expect(frames[0].sessionId).toBe("sess-001");
      expect(frames[0].seq).toBe(42);
      expect(new TextDecoder().decode(frames[0].data)).toBe("hello world");
    });

    it("dispatches to multiple independent listeners", () => {
      const framesA: TerminalOutputFrame[] = [];
      const framesB: TerminalOutputFrame[] = [];

      service.subscribeTerminalOutput((f) => framesA.push(f));
      service.subscribeTerminalOutput((f) => framesB.push(f));

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      const data = encoder.encode("test");
      const buf = buildOutputFrame("sess-001", 1, data);
      mockWs.onmessage?.({ data: buf } as MessageEvent);

      expect(framesA).toHaveLength(1);
      expect(framesB).toHaveLength(1);
    });

    it("unsubscribe stops delivery to that listener only", () => {
      const framesA: TerminalOutputFrame[] = [];
      const framesB: TerminalOutputFrame[] = [];

      const unsubA = service.subscribeTerminalOutput((f) => framesA.push(f));
      service.subscribeTerminalOutput((f) => framesB.push(f));

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      // First message - both receive
      const buf1 = buildOutputFrame("sess-001", 1, encoder.encode("msg1"));
      mockWs.onmessage?.({ data: buf1 } as MessageEvent);

      // Unsubscribe A
      unsubA();

      // Second message - only B receives
      const buf2 = buildOutputFrame("sess-001", 2, encoder.encode("msg2"));
      mockWs.onmessage?.({ data: buf2 } as MessageEvent);

      expect(framesA).toHaveLength(1);
      expect(framesB).toHaveLength(2);
    });
  });

  describe("replay_truncated event", () => {
    it("dispatches replay_truncated to subscribed listeners", () => {
      const events: Array<{
        sessionId: string;
        payload: ReplayTruncatedPayload;
      }> = [];

      service.subscribeReplayTruncated((sessionId, payload) => {
        events.push({ sessionId, payload });
      });

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      const payload: ReplayTruncatedPayload = {
        code: "replay_truncated",
        requested_resume_from_seq: 5,
        replay_window_start_seq: 100,
        latest_seq: 200,
      };

      const msg = buildControlEvent("replay_truncated", "sess-001", payload);
      mockWs.onmessage?.({ data: msg } as MessageEvent);

      expect(events).toHaveLength(1);
      expect(events[0].sessionId).toBe("sess-001");
      expect(events[0].payload.requested_resume_from_seq).toBe(5);
      expect(events[0].payload.latest_seq).toBe(200);
    });
  });

  describe("session_subscription event", () => {
    it("dispatches session_subscription to listeners", () => {
      const events: Array<{
        sessionId: string;
        payload: SessionSubscriptionPayload;
      }> = [];

      service.subscribeSessionSubscription((sessionId, payload) => {
        events.push({ sessionId, payload });
      });

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      const payload: SessionSubscriptionPayload = {
        state: "subscribed",
        resume_from_seq: 42,
        latest_seq: 120,
        replay_window_start_seq: 90,
        at: "2026-02-16T00:00:00Z",
      };

      const msg = buildControlEvent("session_subscription", "sess-001", payload);
      mockWs.onmessage?.({ data: msg } as MessageEvent);

      expect(events).toHaveLength(1);
      expect(events[0].sessionId).toBe("sess-001");
      expect(events[0].payload.state).toBe("subscribed");
      expect(events[0].payload.latest_seq).toBe(120);
    });
  });

  describe("health state transitions", () => {
    it("transitions to healthy on open", () => {
      const healthChanges: string[] = [];
      service.on({
        onHealthChange: (h) => healthChanges.push(h),
      });

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      expect(healthChanges).toContain("healthy");
      expect(service.health).toBe("healthy");
    });

    it("transitions to degraded on error", () => {
      const healthChanges: string[] = [];
      service.on({
        onHealthChange: (h) => healthChanges.push(h),
      });

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      mockWs.onerror?.(new Event("error"));
      expect(healthChanges).toContain("degraded");
    });

    it("transitions to disconnected on close", () => {
      const healthChanges: string[] = [];
      service.on({
        onHealthChange: (h) => healthChanges.push(h),
      });

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      mockWs.onclose?.(new CloseEvent("close"));
      expect(service.health).toBe("disconnected");
    });
  });

  describe("sendInput", () => {
    it("sends binary TERMINAL_INPUT frame when connected", () => {
      const sentData: Array<Uint8Array> = [];
      mockWs.readyState = WebSocket.OPEN;
      mockWs.send = (data: any) => sentData.push(new Uint8Array(data));

      service.sendInput("sess-001", encoder.encode("ls\n"));

      expect(sentData).toHaveLength(1);
      const frame = sentData[0];
      expect(frame[0]).toBe(Opcodes.TERMINAL_INPUT);
    });

    it("splits oversized terminal input into ordered chunks", () => {
      const sentData: Array<Uint8Array> = [];
      mockWs.readyState = WebSocket.OPEN;
      mockWs.send = (data: any) => sentData.push(new Uint8Array(data));

      const payload = new Uint8Array(MAX_INPUT_PAYLOAD_BYTES * 2 + 17);
      for (let i = 0; i < payload.length; i++) {
        payload[i] = i % 251;
      }

      service.sendInput("sess-001", payload);

      expect(sentData).toHaveLength(3);
      const decoded = sentData.map(decodeInputFrame);
      for (const frame of decoded) {
        expect(frame.sessionId).toBe("sess-001");
        expect(frame.payload.length).toBeLessThanOrEqual(MAX_INPUT_PAYLOAD_BYTES);
      }

      const merged = new Uint8Array(
        decoded.reduce((total, frame) => total + frame.payload.length, 0),
      );
      let offset = 0;
      for (const frame of decoded) {
        merged.set(frame.payload, offset);
        offset += frame.payload.length;
      }
      expect(merged).toEqual(payload);
    });

    it("does not send when WebSocket is not open", () => {
      const sentData: Array<any> = [];
      mockWs.readyState = WebSocket.CLOSED;
      mockWs.send = (data: any) => sentData.push(data);

      service.sendInput("sess-001", encoder.encode("ls\n"));

      expect(sentData).toHaveLength(0);
    });
  });

  describe("control messages", () => {
    it("queues subscriptions before open and sends one subscribe on connect", () => {
      const sent: string[] = [];
      mockWs.send = (data: any) => sent.push(String(data));

      service.subscribeSession("sess-001");
      service.subscribeSession("sess-001");
      service.subscribeSession("sess-001");

      mockWs.readyState = WebSocket.OPEN;
      mockWs.onopen?.(new Event("open"));

      const subscribeMessages = sent
        .map((raw) => JSON.parse(raw))
        .filter((msg) => msg.type === "subscribe_session");
      expect(subscribeMessages).toHaveLength(1);
      expect(subscribeMessages[0].payload.session_id).toBe("sess-001");
    });

    it("dedupes repeated subscribe_session while already subscribed", () => {
      const sent: string[] = [];
      mockWs.readyState = WebSocket.OPEN;
      mockWs.send = (data: any) => sent.push(String(data));
      mockWs.onopen?.(new Event("open"));

      service.subscribeSession("sess-001", 0);
      service.subscribeSession("sess-001", 0);

      // Server acknowledges the active subscription.
      const subscribed = buildControlEvent("session_subscription", "sess-001", {
        state: "subscribed",
        resume_from_seq: 0,
        latest_seq: 0,
        replay_window_start_seq: 0,
        at: "2026-02-20T00:00:00Z",
      });
      mockWs.onmessage?.({ data: subscribed } as MessageEvent);

      // A duplicate call should be ignored client-side.
      service.subscribeSession("sess-001", 0);

      const subscribeMessages = sent
        .map((raw) => JSON.parse(raw))
        .filter((msg) => msg.type === "subscribe_session");
      expect(subscribeMessages).toHaveLength(1);
    });

    it("sends unsubscribe_session JSON control message", () => {
      const sent: string[] = [];
      mockWs.readyState = WebSocket.OPEN;
      mockWs.send = (data: any) => sent.push(String(data));

      service.unsubscribeSession("sess-001");

      expect(sent).toHaveLength(1);
      const msg = JSON.parse(sent[0]);
      expect(msg.type).toBe("unsubscribe_session");
      expect(msg.payload.session_id).toBe("sess-001");
    });
  });
});
