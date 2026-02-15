import "@testing-library/jest-dom/vitest";

// Stub WebSocket for all tests
class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;

  binaryType: string = "blob";
  readyState: number = MockWebSocket.CONNECTING;
  url: string;
  protocol: string = "";
  bufferedAmount: number = 0;
  extensions: string = "";

  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onclose: ((ev: CloseEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;

  constructor(url: string, _protocols?: string | string[]) {
    this.url = url;
  }

  send(_data: string | ArrayBufferLike | Blob | ArrayBufferView): void {}
  close(_code?: number, _reason?: string): void {
    this.readyState = MockWebSocket.CLOSED;
  }

  addEventListener(_type: string, _listener: EventListener): void {}
  removeEventListener(_type: string, _listener: EventListener): void {}
  dispatchEvent(_event: Event): boolean {
    return true;
  }
}

// Install the mock globally
(globalThis as any).WebSocket = MockWebSocket;

// Stub navigator.vibrate
if (!navigator.vibrate) {
  (navigator as any).vibrate = () => true;
}

// Stub ResizeObserver
class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as any).ResizeObserver = MockResizeObserver;

// Stub window.visualViewport
if (!window.visualViewport) {
  (window as any).visualViewport = {
    addEventListener: () => {},
    removeEventListener: () => {},
    width: 1024,
    height: 768,
  };
}
