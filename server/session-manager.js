const pty = require('node-pty');
const { execSync, execFile } = require('child_process');
const crypto = require('crypto');
const StateDetector = require('./state-detector');
const ScrollGuard = require('./scroll-guard');

const REPLAY_BUFFER_SIZE = 4096;

class Session {
  constructor(id, tmuxName, isNew) {
    this.id = id;
    this.name = tmuxName;
    this.tmuxName = tmuxName;
    this.detector = new StateDetector();
    this.replayBuffer = Buffer.alloc(0);
    this.attachedWs = null;
    this.tokenCount = 0;
    this.thought = null;
    this._lastReplayHash = null;
    this._lastThoughtContext = null;

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

    // ScrollGuard coalesces rapid full-screen redraws from tmux
    // (triggered by scroll in another attached client) to prevent
    // visual garbage in xterm.js.
    this.scrollGuard = new ScrollGuard((data) => {
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

    this.pty.onData((data) => {
      // Estimate tokens from visible output (~4 chars/token)
      const visible = this._stripAnsi(typeof data === 'string' ? data : data.toString('utf-8'));
      this.tokenCount += Math.ceil(visible.length / 4);

      // State detector always sees all output (unfiltered)
      this.detector.processOutput(data);
      // ScrollGuard decides what reaches the client
      this.scrollGuard.process(data);
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
      // Notify attached client that the session ended
      if (this.attachedWs && this.attachedWs.readyState === 1) {
        this.attachedWs.send(Buffer.from([0x03]));
        this.attachedWs.close();
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

    // Send current thought if available
    if (this.thought) {
      const thoughtPayload = JSON.stringify({ sessionId: this.id, thought: this.thought });
      ws.send(Buffer.concat([Buffer.from([0x05]), Buffer.from(thoughtPayload)]));
    }

    ws.on('message', (msg) => {
      const data = Buffer.from(msg);
      if (data.length === 0) return;

      const cmd = data[0];
      const payload = data.slice(1);

      if (cmd === 0x30) {
        if (!this._exited) {
          this.scrollGuard.notifyInput();
          this.pty.write(payload.toString());
        }
      } else if (cmd === 0x01) {
        try {
          const { cols, rows } = JSON.parse(payload.toString());
          this.pty.resize(cols, rows);
        } catch (e) {}
      } else if (cmd === 0x04) {
        this.detector.dismissAttention();
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

  _stripAnsi(str) {
    return str
      .replace(/\x1b\[[0-9;]*[A-Za-z]/g, '')
      .replace(/\x1b\][^\x07]*\x07/g, '')
      .replace(/\x1b[()][0-9A-B]/g, '')
      .replace(/\x1b[>=<]/g, '')
      .replace(/[\x00-\x08\x0e-\x1f]/g, '');
  }

  getThoughtContext() {
    const raw = this.replayBuffer.toString('utf-8');
    const stripped = this._stripAnsi(raw);
    return stripped.slice(-500);
  }

  replayHash() {
    return crypto.createHash('md5').update(this.replayBuffer).digest('hex');
  }

  resize(cols, rows) {
    this.pty.resize(cols, rows);
  }

  destroy() {
    this.detach();
    this.scrollGuard.destroy();
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
      cwd: process.env.HOME,
      thought: this.thought,
    };
  }
}

class SessionManager {
  constructor() {
    this.sessions = new Map(); // id → Session (PTY-backed connections to tmux)
  }

  // Context window limits per CLI tool (in tokens)
  static CONTEXT_LIMITS = {
    'Claude Code': 200000,
    'Codex': 192000,
    'Amp': 200000,
    'OpenCode': 128000,
    'Aider': 128000,
    'Goose': 200000,
    'Cline': 200000,
    'Cursor': 200000,
  };
  static DEFAULT_CONTEXT_LIMIT = 128000;

  // Map process names to CLI tool display names
  static CLI_TOOLS = {
    claude: 'Claude Code',
    codex: 'Codex',
    amp: 'Amp',
    opencode: 'OpenCode',
    aider: 'Aider',
    goose: 'Goose',
    cline: 'Cline',
    cursor: 'Cursor',
  };

  // Detect CLI tool by inspecting child processes of the pane's shell.
  // pane_current_command is unreliable (claude reports its version, codex
  // shows as "node"), so we check the shell's children via pgrep + ps.
  _detectToolForPid(panePid) {
    if (!panePid) return null;
    try {
      const childInfo = execSync(
        `pgrep -P ${panePid} | head -4 | xargs -I{} ps -p {} -o comm= -o args= 2>/dev/null`,
        { encoding: 'utf-8', timeout: 2000 }
      ).trim();
      if (!childInfo) return null;

      for (const line of childInfo.split('\n')) {
        const parts = line.trim().split(/\s+/);
        const comm = (parts[0] || '').toLowerCase();
        // Direct match on comm (e.g. "claude", "amp", "aider")
        if (SessionManager.CLI_TOOLS[comm]) return SessionManager.CLI_TOOLS[comm];
        // Check args for tools that run via node (e.g. "node /path/to/codex")
        const args = parts.slice(1).join(' ').toLowerCase();
        for (const tool of Object.keys(SessionManager.CLI_TOOLS)) {
          if (args.includes(`/${tool}`) || args.includes(`${tool}`)) {
            return SessionManager.CLI_TOOLS[tool];
          }
        }
      }
      return null;
    } catch (e) {
      return null;
    }
  }

  // Discover real tmux sessions from the system
  _getTmuxSessions() {
    try {
      const execEnv = { ...process.env };
      delete execEnv.TMUX;
      delete execEnv.TMUX_PANE;
      const out = execSync(
        'tmux list-sessions -F "#{session_name}\t#{session_windows}\t#{session_attached}\t#{pane_current_path}\t#{pane_pid}"',
        { encoding: 'utf-8', timeout: 3000, env: execEnv }
      );
      return out.trim().split('\n').filter(Boolean).map((line) => {
        const [name, windows, attached, cwd, panePid] = line.split('\t');
        return {
          name,
          windows: parseInt(windows, 10),
          attached: parseInt(attached, 10),
          cwd: cwd || process.env.HOME,
          tool: this._detectToolForPid(panePid),
        };
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
      const contextLimit = ts.tool
        ? (SessionManager.CONTEXT_LIMITS[ts.tool] || SessionManager.DEFAULT_CONTEXT_LIMIT)
        : SessionManager.DEFAULT_CONTEXT_LIMIT;
      return {
        id: ptySession ? ptySession.id : ts.name, // use tmux name as ID if no PTY
        name: ts.name,
        windows: ts.windows,
        attached: ts.attached > 0,
        state: ptySession ? ptySession.detector.state : 'idle',
        currentCommand: ptySession ? ptySession.detector.currentCommand : null,
        connected: !!ptySession, // do we have a PTY bridge for this?
        cwd: ts.cwd,
        tool: ts.tool,
        tokenCount: ptySession ? ptySession.tokenCount : 0,
        contextLimit,
        thought: ptySession ? ptySession.thought : null,
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

  // Create a brand new tmux session with incrementing numeric name
  createSession() {
    const tmuxSessions = this._getTmuxSessions();
    const existingNames = new Set(tmuxSessions.map((s) => s.name));

    // Find highest numeric session name and increment
    let maxNum = 0;
    for (const s of tmuxSessions) {
      const n = parseInt(s.name, 10);
      if (!isNaN(n) && n > maxNum) maxNum = n;
    }
    let tmuxName = String(maxNum + 1);
    while (existingNames.has(tmuxName)) {
      tmuxName = String(parseInt(tmuxName, 10) + 1);
    }

    return this.connectSession(tmuxName);
  }

  startThoughtLoop() {
    setInterval(this._generateThoughts.bind(this), 15000);
    console.log('  thought generation loop started');
  }

  async _generateThoughts() {
    const sessionCount = this.sessions.size;
    console.log(`[thought] tick — ${sessionCount} sessions`);

    for (const session of this.sessions.values()) {
      if (session._exited) {
        console.log(`[thought] ${session.id}: skip (exited)`);
        continue;
      }

      const state = session.detector.state;

      const hash = session.replayHash();
      if (hash === session._lastReplayHash) {
        console.log(`[thought] ${session.id}: skip (unchanged hash)`);
        continue;
      }
      session._lastReplayHash = hash;

      const context = session.getThoughtContext();
      if (!context.trim()) {
        console.log(`[thought] ${session.id}: skip (empty context)`);
        continue;
      }

      const prevContext = session._lastThoughtContext;
      session._lastThoughtContext = context;

      console.log(`[thought] ${session.id}: calling codex (state=${state}, first=${!prevContext}, context=${context.length} chars)`);

      try {
        const thought = await this._callCodex(context, state, prevContext);
        console.log(`[thought] ${session.id}: codex returned: "${thought}"`);
        if (thought) {
          session.thought = thought;
          this._broadcastThought(session);
        }
      } catch (e) {
        console.error(`[thought] ${session.id}: codex error:`, e.message);
      }
    }
  }

  _callCodex(context) {
    const prompt = `You are watching a terminal session. Summarize what's happening in 6 words or fewer, like a character's thought bubble. Be concise and playful. Terminal output:\n${context}`;
    return new Promise((resolve, reject) => {
      execFile(
        'codex',
        ['exec', '-c', 'model_reasoning_effort="low"', '--ephemeral', prompt],
        { timeout: 15000 },
        (err, stdout, stderr) => {
          if (err) {
            console.error(`[thought] execFile failed: ${err.message}`);
            if (stderr) console.error(`[thought] stderr: ${stderr.slice(0, 200)}`);
            return reject(err);
          }
          resolve(stdout.trim());
        }
      );
    });
  }

  _broadcastThought(session) {
    const payload = JSON.stringify({ sessionId: session.id, thought: session.thought });
    const frame = Buffer.concat([Buffer.from([0x05]), Buffer.from(payload)]);

    const hasWs = !!(session.attachedWs && session.attachedWs.readyState === 1);
    console.log(`[thought] ${session.id}: broadcasting "${session.thought}" (ws attached: ${hasWs})`);

    if (hasWs) {
      session.attachedWs.send(frame);
    }
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
