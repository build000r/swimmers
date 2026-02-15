const express = require('express');
const http = require('http');
const { WebSocketServer } = require('ws');
const path = require('path');
const SessionManager = require('./session-manager');
const { startThoughtLoop } = require('./thought-loop');

const app = express();
const server = http.createServer(app);
const wss = new WebSocketServer({ noServer: true });
const manager = new SessionManager();

const PORT = process.env.PORT || 3210;

// Serve static files
app.use(express.static(path.join(__dirname, '..', 'dist')));
app.use(express.json());

// REST API
app.get('/api/sessions', (req, res) => {
  res.json(manager.listSessions());
});

app.post('/api/sessions', (req, res) => {
  try {
    const session = manager.createSession();
    res.status(201).json(session.toJSON());
  } catch (e) {
    console.error('Failed to create session:', e.message);
    res.status(500).json({ error: 'Failed to create session' });
  }
});

const SESSION_ID_RE = /^[a-zA-Z0-9_-]+$/;

app.delete('/api/sessions/:id', (req, res) => {
  const { id } = req.params;
  if (!id || !SESSION_ID_RE.test(id)) {
    return res.status(400).json({ error: 'invalid session id' });
  }
  const ok = manager.destroySession(id);
  if (ok) {
    res.json({ ok: true });
  } else {
    res.status(404).json({ error: 'session not found' });
  }
});

// WebSocket upgrade — route /ws/:sessionId
server.on('upgrade', (req, socket, head) => {
  const match = req.url.match(/^\/ws\/([\w.-]+)$/);
  if (!match) {
    socket.destroy();
    return;
  }

  const sessionId = match[1];

  wss.handleUpgrade(req, socket, head, (ws) => {
    const ok = manager.attachWebSocket(sessionId, ws);
    if (!ok) {
      ws.close(1008, 'session not found');
    }
  });
});

server.listen(PORT, '0.0.0.0', () => {
  console.log(`Throngterm running on http://0.0.0.0:${PORT}`);

  // Auto-connect PTY bridges to all existing tmux sessions
  // so state detection works immediately on restart
  const existing = manager.listSessions();
  for (const s of existing) {
    if (!s.connected) {
      manager.connectSession(s.name);
      console.log(`  ↳ attached to tmux session: ${s.name}`);
    }
  }

  startThoughtLoop(manager);
});
