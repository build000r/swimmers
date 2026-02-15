// xterm.js wrapper — manages terminal instance and WebSocket connection
// Based on ttyd's approach to xterm.js initialization

class TerminalWrapper {
  constructor(container) {
    this.container = container;
    this.term = null;
    this.fitAddon = null;
    this.ws = null;
    this.sessionId = null;
    this._resizeHandler = null;
    this._onStateChange = null;
    this._onSessionExit = null;
    this.textEncoder = new TextEncoder();
    this.textDecoder = new TextDecoder();
  }

  onStateChange(cb) {
    this._onStateChange = cb;
  }

  onSessionExit(cb) {
    this._onSessionExit = cb;
  }

  open() {
    this.term = new Terminal({
      theme: {
        background: '#1a1a2e',
        foreground: '#e0e0e0',
        cursor: '#e0e0e0',
        cursorAccent: '#1a1a2e',
        selectionBackground: 'rgba(255,255,255,0.2)',
      },
      fontFamily: 'Menlo, Monaco, "Courier New", monospace',
      fontSize: 14,
      scrollback: 5000,
      cursorBlink: true,
    });

    const FitAddonClass = typeof FitAddon.FitAddon === 'function' ? FitAddon.FitAddon : FitAddon;
    this.fitAddon = new FitAddonClass();
    this.term.loadAddon(this.fitAddon);
    this.term.open(this.container);

    // Disable autocapitalize/autocorrect on the hidden textarea
    const textarea = this.container.querySelector('textarea');
    if (textarea) {
      textarea.setAttribute('autocapitalize', 'off');
      textarea.setAttribute('autocorrect', 'off');
      textarea.setAttribute('autocomplete', 'off');
      textarea.setAttribute('spellcheck', 'false');
    }

    this.fitAddon.fit();
  }

  connect(sessionId) {
    this.sessionId = sessionId;

    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    this.ws = new WebSocket(`${proto}//${location.host}/ws/${sessionId}`);
    this.ws.binaryType = 'arraybuffer';

    this.ws.onopen = () => {
      // Send initial size so server can resize the PTY
      const msg = JSON.stringify({ cols: this.term.cols, rows: this.term.rows });
      const payload = this.textEncoder.encode(msg);
      const frame = new Uint8Array(1 + payload.length);
      frame[0] = 0x01; // resize command
      frame.set(payload, 1);
      this.ws.send(frame);
    };

    this.ws.onmessage = (ev) => {
      const rawData = ev.data;
      const cmd = new Uint8Array(rawData)[0];
      const data = rawData.slice(1);

      switch (cmd) {
        case 0x30: // terminal output
          this.term.write(new Uint8Array(data));
          break;
        case 0x01: // title
          document.getElementById('terminal-title').textContent = this.textDecoder.decode(data);
          break;
        case 0x02: // state
          try {
            const state = JSON.parse(this.textDecoder.decode(data));
            if (this._onStateChange) this._onStateChange(state);
          } catch (e) {}
          break;
        case 0x03: // session exited
          if (this._onSessionExit) this._onSessionExit();
          break;
      }
    };

    this.ws.onclose = () => {
      if (this.term) this.term.write('\r\n\x1b[90m[disconnected]\x1b[0m\r\n');
    };

    // Send user input to server
    this.term.onData((data) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
      const payload = this.textEncoder.encode(data);
      const frame = new Uint8Array(1 + payload.length);
      frame[0] = 0x30; // input command
      frame.set(payload, 1);
      this.ws.send(frame);
    });

    // Send resize events
    this.term.onResize(({ cols, rows }) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
      const payload = this.textEncoder.encode(JSON.stringify({ cols, rows }));
      const frame = new Uint8Array(1 + payload.length);
      frame[0] = 0x01;
      frame.set(payload, 1);
      this.ws.send(frame);
    });

    this._addResizeListeners();
  }

  // Call this after the container is visible and has its final dimensions
  fit() {
    if (this.fitAddon) this.fitAddon.fit();
  }

  // --- Resize listener helpers ---

  _addResizeListeners() {
    this._resizeHandler = () => {
      clearTimeout(this._resizeTimer);
      this._resizeTimer = setTimeout(() => {
        if (this.fitAddon) this.fitAddon.fit();
      }, 150);
    };
    window.addEventListener('resize', this._resizeHandler);
    if (window.visualViewport) {
      window.visualViewport.addEventListener('resize', this._resizeHandler);
    }
  }

  _removeResizeListeners() {
    if (this._resizeHandler) {
      clearTimeout(this._resizeTimer);
      window.removeEventListener('resize', this._resizeHandler);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener('resize', this._resizeHandler);
      }
      this._resizeHandler = null;
    }
  }

  // --- Cache support ---

  // Remove host element from DOM and stop resize listeners.
  // Keeps xterm instance and WebSocket alive for cache.
  detachFromDOM() {
    this._removeResizeListeners();
    if (this.container && this.container.parentNode) {
      this.container.parentNode.removeChild(this.container);
    }
  }

  // Re-append host element into a parent, restore resize listeners, refit.
  attachToDOM(parent) {
    parent.appendChild(this.container);
    this._addResizeListeners();
    if (this.fitAddon) this.fitAddon.fit();
  }

  // Reopen WebSocket if it died while cached.
  reconnect() {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return;
    // Close stale socket if lingering
    if (this.ws) {
      try { this.ws.close(); } catch (e) {}
      this.ws = null;
    }
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    this.ws = new WebSocket(`${proto}//${location.host}/ws/${this.sessionId}`);
    this.ws.binaryType = 'arraybuffer';

    this.ws.onopen = () => {
      const msg = JSON.stringify({ cols: this.term.cols, rows: this.term.rows });
      const payload = this.textEncoder.encode(msg);
      const frame = new Uint8Array(1 + payload.length);
      frame[0] = 0x01;
      frame.set(payload, 1);
      this.ws.send(frame);
    };

    this.ws.onmessage = (ev) => {
      const rawData = ev.data;
      const cmd = new Uint8Array(rawData)[0];
      const data = rawData.slice(1);
      switch (cmd) {
        case 0x30:
          this.term.write(new Uint8Array(data));
          break;
        case 0x01:
          document.getElementById('terminal-title').textContent = this.textDecoder.decode(data);
          break;
        case 0x02:
          try {
            const state = JSON.parse(this.textDecoder.decode(data));
            if (this._onStateChange) this._onStateChange(state);
          } catch (e) {}
          break;
        case 0x03:
          if (this._onSessionExit) this._onSessionExit();
          break;
      }
    };

    this.ws.onclose = () => {
      if (this.term) this.term.write('\r\n\x1b[90m[disconnected]\x1b[0m\r\n');
    };
  }

  get isAlive() {
    return !!(this.term && this.fitAddon);
  }

  // Full teardown — used by cache eviction and normal cleanup
  disconnect() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    this._removeResizeListeners();
    if (this.term) {
      this.term.dispose();
      this.term = null;
    }
    this.fitAddon = null;
    if (this.container) this.container.innerHTML = '';
  }

  focus() {
    if (this.term) this.term.focus();
  }
}
