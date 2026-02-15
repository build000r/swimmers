/**
 * Mock RealtimeService for testing. Captures method calls and allows
 * simulating server events / health changes.
 */

import type { TerminalOutputFrame, RealtimeCallbacks } from "@/services/realtime";
import type { ReplayTruncatedPayload, TransportHealth } from "@/types";

export class MockRealtimeService {
  callbacks: RealtimeCallbacks = {};
  terminalOutputListeners = new Set<(frame: TerminalOutputFrame) => void>();
  replayTruncatedListeners = new Set<
    (sessionId: string, payload: ReplayTruncatedPayload) => void
  >();
  connected = false;
  connectedUrl = "";
  subscribedSessions: Array<{ sessionId: string; resumeFromSeq?: number }> = [];
  sentInputs: Array<{ sessionId: string; data: Uint8Array }> = [];
  sentResizes: Array<{ sessionId: string; cols: number; rows: number }> = [];
  sentDismissAttentions: string[] = [];
  sentJsonMessages: unknown[] = [];

  private _health: TransportHealth = "disconnected";

  get health(): TransportHealth {
    return this._health;
  }

  on(cbs: RealtimeCallbacks): void {
    this.callbacks = { ...this.callbacks, ...cbs };
  }

  subscribeTerminalOutput(
    cb: (frame: TerminalOutputFrame) => void,
  ): () => void {
    this.terminalOutputListeners.add(cb);
    return () => {
      this.terminalOutputListeners.delete(cb);
    };
  }

  subscribeReplayTruncated(
    cb: (sessionId: string, payload: ReplayTruncatedPayload) => void,
  ): () => void {
    this.replayTruncatedListeners.add(cb);
    return () => {
      this.replayTruncatedListeners.delete(cb);
    };
  }

  connect(url: string): void {
    this.connected = true;
    this.connectedUrl = url;
  }

  disconnect(): void {
    this.connected = false;
    this.setHealth("disconnected");
  }

  subscribeSession(sessionId: string, resumeFromSeq?: number): void {
    this.subscribedSessions.push({ sessionId, resumeFromSeq });
  }

  sendResize(sessionId: string, cols: number, rows: number): void {
    this.sentResizes.push({ sessionId, cols, rows });
  }

  sendDismissAttention(sessionId: string): void {
    this.sentDismissAttentions.push(sessionId);
  }

  sendInput(sessionId: string, data: Uint8Array): void {
    this.sentInputs.push({ sessionId, data });
  }

  // ---- Test simulation helpers ----

  setHealth(h: TransportHealth): void {
    this._health = h;
    this.callbacks.onHealthChange?.(h);
  }

  simulateTerminalOutput(frame: TerminalOutputFrame): void {
    this.callbacks.onTerminalOutput?.(frame);
    for (const listener of this.terminalOutputListeners) {
      listener(frame);
    }
  }

  simulateReplayTruncated(
    sessionId: string,
    payload: ReplayTruncatedPayload,
  ): void {
    this.callbacks.onReplayTruncated?.(sessionId, payload);
    for (const listener of this.replayTruncatedListeners) {
      listener(sessionId, payload);
    }
  }

  reset(): void {
    this.callbacks = {};
    this.terminalOutputListeners.clear();
    this.replayTruncatedListeners.clear();
    this.connected = false;
    this.connectedUrl = "";
    this.subscribedSessions = [];
    this.sentInputs = [];
    this.sentResizes = [];
    this.sentDismissAttentions = [];
    this.sentJsonMessages = [];
    this._health = "disconnected";
  }
}
