// App glue — view routing, session polling, thronglet ↔ terminal bridge

(function () {
  const overviewView = document.getElementById('overview-view');
  const terminalView = document.getElementById('terminal-view');
  const addBtn = document.getElementById('add-session-btn');
  const backBtn = document.getElementById('back-btn');
  const termContainer = document.getElementById('terminal-container');
  const termTitle = document.getElementById('terminal-title');
  const stateDot = document.getElementById('terminal-state-dot');
  const emptyState = document.getElementById('empty-state');

  let pollInterval = null;
  let currentView = 'overview';
  let termWrapper = null;
  let sessions = [];

  const renderer = new ThrongletRenderer(
    document.getElementById('thronglets-container'),
    (sessionId) => openTerminal(sessionId)
  );

  // --- API ---

  async function fetchSessions() {
    try {
      const res = await fetch('/api/sessions');
      sessions = await res.json();
      renderer.update(sessions);
      emptyState.classList.toggle('hidden', sessions.length > 0);
    } catch (e) {
      // network error, keep showing stale data
    }
  }

  async function createSession() {
    try {
      const res = await fetch('/api/sessions', { method: 'POST' });
      const session = await res.json();
      sessions.push(session);
      renderer.update(sessions);
      emptyState.classList.toggle('hidden', sessions.length > 0);
      openTerminal(session.id);
    } catch (e) {
      console.error('Failed to create session:', e);
    }
  }

  // --- Views ---

  function showOverview() {
    if (termWrapper) {
      termWrapper.disconnect();
      termWrapper = null;
    }
    currentView = 'overview';
    overviewView.classList.add('active');
    overviewView.classList.remove('slide-left');
    terminalView.classList.remove('active');
    startPolling();
    fetchSessions();
  }

  function openTerminal(sessionId) {
    const session = sessions.find((s) => s.id === sessionId);
    if (!session) return;

    currentView = 'terminal';
    stopPolling();

    // Clean up any previous terminal
    if (termWrapper) {
      termWrapper.disconnect();
      termWrapper = null;
    }

    termTitle.textContent = session.name;
    updateStateDot(session.state);

    // Show terminal view
    overviewView.classList.remove('active');
    overviewView.classList.add('slide-left');
    terminalView.classList.add('active');

    // Wait for CSS transition to finish so container has real dimensions,
    // then open xterm, fit it, and connect the WebSocket
    terminalView.addEventListener('transitionend', function onEnd() {
      terminalView.removeEventListener('transitionend', onEnd);
      initTerminal(sessionId);
    }, { once: true });

    // Fallback if transitionend doesn't fire (e.g. reduced motion)
    setTimeout(() => {
      if (!termWrapper) initTerminal(sessionId);
    }, 350);
  }

  function initTerminal(sessionId) {
    if (termWrapper) return; // already initialized

    termWrapper = new TerminalWrapper(termContainer);
    termWrapper.onStateChange((info) => {
      updateStateDot(info.state);
      const s = sessions.find((x) => x.id === sessionId);
      if (s) {
        s.state = info.state;
        s.currentCommand = info.currentCommand;
      }
    });

    // 1. Open terminal into the now-visible container
    termWrapper.open();
    // 2. Connect WebSocket (sends initial resize on open)
    termWrapper.connect(sessionId);
    // 3. Extra fit after a tick to ensure dimensions are correct
    requestAnimationFrame(() => {
      termWrapper.fit();
      termWrapper.focus();
    });
  }

  function updateStateDot(state) {
    stateDot.className = 'state-dot ' + (state || 'idle');
  }

  // --- Polling ---

  function startPolling() {
    stopPolling();
    pollInterval = setInterval(fetchSessions, 2000);
  }

  function stopPolling() {
    if (pollInterval) {
      clearInterval(pollInterval);
      pollInterval = null;
    }
  }

  // --- Events ---

  addBtn.addEventListener('click', createSession);
  backBtn.addEventListener('click', showOverview);

  // Swipe right to go back on terminal view
  let touchStartX = 0;
  terminalView.addEventListener('touchstart', (e) => {
    touchStartX = e.touches[0].clientX;
  }, { passive: true });

  terminalView.addEventListener('touchend', (e) => {
    const dx = e.changedTouches[0].clientX - touchStartX;
    if (touchStartX < 40 && dx > 80) {
      showOverview();
    }
  }, { passive: true });

  // --- Init ---

  fetchSessions();
  startPolling();
})();
