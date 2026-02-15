const pty = require('node-pty');
const { execSync } = require('child_process');
const StateDetector = require('./state-detector');

const REPLAY_BUFFER_SIZE = 4096;

class Session {
  constructor(id, tmuxName, isNew) {
    this.id = id;
    this.name = tmuxName;
    this.tmuxName = tmuxName;
    this.detector = new StateDetector();
    this.replayBuffer = Buffer.alloc(0);
    this.attachedWs = null;

    // Clean env: unset TMUX/TMUX_PANE so tmux commands work even if
    // the server itself is running inside a tmux session
    const env = { ...process.env, TERM: 'xterm-256color' };
    delete env.TMUX;
    delete env.TMUX_PANE;

    const ptyOpts = {
      name: 'xterm-256color',
      cols: 80,
      rows: 24,
      cwd: process.env.HOME,
      env,
    };

    // Spawn a PTY that attaches to the tmux session
    if (isNew) {
      this.pty = pty.spawn('tmux', ['new-session', '-s', tmuxName], ptyOpts);
    } else {
      this.pty = pty.spawn('tmux', ['attach-session', '-t', tmuxName], ptyOpts);
    }

    this.pty.onData((data) => {
      this.detector.processOutput(data);

      const buf = Buffer.from(data);
      this.replayBuffer = Buffer.concat([this.replayBuffer, buf]);
      if (this.replayBuffer.length > REPLAY_BUFFER_SIZE) {
        this.replayBuffer = this.replayBuffer.slice(
          this.replayBuffer.length - REPLAY_BUFFER_SIZE
        );
      }

      if (this.attachedWs && this.attachedWs.readyState === 1) {
        const frame = Buffer.concat([Buffer.from([0x30]), buf]);
        this.attachedWs.send(frame);
      }
    });

    this.detector.onStateChange((info) => {
      if (this.attachedWs && this.attachedWs.readyState === 1) {
        const payload = JSON.stringify(info);
        const frame = Buffer.concat([
          Buffer.from([0x02]),
          Buffer.from(payload),
        ]);
        this.attachedWs.send(frame);
      }
    });

    this.pty.onExit(({ exitCode, signal }) => {
      this._exited = true;
      if (exitCode !== 0) {
        console.error(
          `[session ${tmuxName}] PTY exited: code=${exitCode} signal=${signal}`
        );
      }
    });
  }

  attach(ws) {
    if (this.attachedWs) {
      this.detach();
    }
    this.attachedWs = ws;

    // Replay buffer so terminal isn't blank
    if (this.replayBuffer.length > 0) {
      const frame = Buffer.concat([Buffer.from([0x30]), this.replayBuffer]);
      ws.send(frame);
    }

    // Send current state
    const statePayload = JSON.stringify(this.detector.getState());
    ws.send(Buffer.concat([Buffer.from([0x02]), Buffer.from(statePayload)]));

    ws.on('message', (msg) => {
      const data = Buffer.from(msg);
      if (data.length === 0) return;

      const cmd = data[0];
      const payload = data.slice(1);

      if (cmd === 0x30) {
        if (!this._exited) this.pty.write(payload.toString());
      } else if (cmd === 0x01) {
        try {
          const { cols, rows } = JSON.parse(payload.toString());
          this.pty.resize(cols, rows);
        } catch (e) {}
      }
    });

    ws.on('close', () => {
      if (this.attachedWs === ws) {
        this.attachedWs = null;
      }
    });
  }

  detach() {
    if (this.attachedWs) {
      this.attachedWs = null;
    }
  }

  resize(cols, rows) {
    this.pty.resize(cols, rows);
  }

  destroy() {
    this.detach();
    try {
      // Detach from tmux cleanly (don't kill the tmux session)
      this.pty.write('\x02d'); // Ctrl-B d = tmux detach
      setTimeout(() => {
        try { this.pty.kill(); } catch (e) {}
      }, 500);
    } catch (e) {}
  }

  toJSON() {
    return {
      id: this.id,
      name: this.name,
      state: this.detector.state,
      currentCommand: this.detector.currentCommand,
    };
  }
}

class SessionManager {
  constructor() {
    this.sessions = new Map(); // id → Session (PTY-backed connections to tmux)
  }

  // Discover real tmux sessions from the system
  _getTmuxSessions() {
    try {
      const execEnv = { ...process.env };
      delete execEnv.TMUX;
      delete execEnv.TMUX_PANE;
      const out = execSync(
        'tmux list-sessions -F "#{session_name}\t#{session_windows}\t#{session_attached}"',
        { encoding: 'utf-8', timeout: 3000, env: execEnv }
      );
      return out.trim().split('\n').filter(Boolean).map((line) => {
        const [name, windows, attached] = line.split('\t');
        return { name, windows: parseInt(windows, 10), attached: parseInt(attached, 10) };
      });
    } catch (e) {
      return [];
    }
  }

  // Sync: discover tmux sessions, return merged list with our PTY state
  listSessions() {
    const tmuxSessions = this._getTmuxSessions();

    // Build result: for each tmux session, include our PTY state if we have one attached
    return tmuxSessions.map((ts) => {
      // Find our PTY session for this tmux session
      const ptySession = this._findByTmuxName(ts.name);
      return {
        id: ptySession ? ptySession.id : ts.name, // use tmux name as ID if no PTY
        name: ts.name,
        windows: ts.windows,
        attached: ts.attached > 0,
        state: ptySession ? ptySession.detector.state : 'idle',
        currentCommand: ptySession ? ptySession.detector.currentCommand : null,
        connected: !!ptySession, // do we have a PTY bridge for this?
      };
    });
  }

  _findByTmuxName(name) {
    for (const s of this.sessions.values()) {
      if (s.tmuxName === name) return s;
    }
    return null;
  }

  // Connect to an existing tmux session (or create a new one)
  connectSession(tmuxName) {
    // Already have a PTY bridge?
    const existing = this._findByTmuxName(tmuxName);
    if (existing && !existing._exited) return existing;

    // Clean up dead one
    if (existing) this.sessions.delete(existing.id);

    // Check if tmux session exists
    const tmuxSessions = this._getTmuxSessions();
    const exists = tmuxSessions.some((s) => s.name === tmuxName);

    const id = tmuxName; // use tmux session name as our ID for simplicity
    const session = new Session(id, tmuxName, !exists);
    this.sessions.set(id, session);
    return session;
  }

  getSession(id) {
    // Try direct lookup first, then by tmux name
    return this.sessions.get(id) || this._findByTmuxName(id);
  }

  // Create a brand new tmux session
  createSession(name) {
    const tmuxSessions = this._getTmuxSessions();
    const existingNames = new Set(tmuxSessions.map((s) => s.name));

    // Generate a unique name if not provided or if it conflicts
    let tmuxName = name;
    if (!tmuxName || existingNames.has(tmuxName)) {
      let counter = tmuxSessions.length + 1;
      while (existingNames.has(`session-${counter}`)) counter++;
      tmuxName = `session-${counter}`;
    }

    return this.connectSession(tmuxName);
  }

  destroySession(id) {
    const session = this.sessions.get(id) || this._findByTmuxName(id);
    if (session) {
      session.destroy();
      this.sessions.delete(session.id);
      return true;
    }
    return false;
  }

  attachWebSocket(id, ws) {
    // Auto-connect to tmux session if we don't have a PTY bridge yet
    let session = this.getSession(id);
    if (!session) {
      // id might be a tmux session name — try connecting
      session = this.connectSession(id);
    }
    if (!session) return false;
    session.attach(ws);
    return true;
  }
}

module.exports = SessionManager;
