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
    this.textEncoder = new TextEncoder();
    this.textDecoder = new TextDecoder();
  }

  onStateChange(cb) {
    this._onStateChange = cb;
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

    this.fitAddon = new FitAddon();
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
      }
    };

    this.ws.onclose = () => {
      this.term.write('\r\n\x1b[90m[disconnected]\x1b[0m\r\n');
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

    // Refit on window/viewport resize
    this._resizeHandler = () => {
      if (this.fitAddon) this.fitAddon.fit();
    };
    window.addEventListener('resize', this._resizeHandler);
    if (window.visualViewport) {
      window.visualViewport.addEventListener('resize', this._resizeHandler);
    }
  }

  // Call this after the container is visible and has its final dimensions
  fit() {
    if (this.fitAddon) this.fitAddon.fit();
  }

  disconnect() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    if (this._resizeHandler) {
      window.removeEventListener('resize', this._resizeHandler);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener('resize', this._resizeHandler);
      }
      this._resizeHandler = null;
    }
    if (this.term) {
      this.term.dispose();
      this.term = null;
    }
    this.fitAddon = null;
    this.container.innerHTML = '';
  }

  focus() {
    if (this.term) this.term.focus();
  }
}
