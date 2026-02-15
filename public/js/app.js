// App glue — view routing, session polling, thronglet ↔ terminal bridge

(function () {
  const overviewView = document.getElementById('overview-view');
  const terminalView = document.getElementById('terminal-view');
  const backBtn = document.getElementById('back-btn');
  const termContainer = document.getElementById('terminal-container');
  const termTitle = document.getElementById('terminal-title');
  const stateDot = document.getElementById('terminal-state-dot');
  const emptyState = document.getElementById('empty-state');
  const field = document.getElementById('field');

  let pollInterval = null;
  let currentView = 'overview';
  let activeSessionId = null;
  let sessions = [];
  const cache = new TerminalCache();

  const LONG_PRESS_MS = 500;

  const renderer = new ThrongletRenderer(
    document.getElementById('thronglets-container'),
    (sessionId) => openTerminal(sessionId)
  );

  // --- Helpers ---

  function repoName(cwd) {
    if (!cwd || cwd === '/') return 'root';
    const parts = cwd.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || 'root';
  }

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
      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        throw new Error(err.error || `HTTP ${res.status}`);
      }
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
    // Cache the active terminal instead of destroying it
    if (activeSessionId) {
      const entry = cache.get(activeSessionId);
      if (entry) {
        // Already in cache (shouldn't happen, but be safe)
        entry.wrapper.detachFromDOM();
        cache.put(activeSessionId, entry.wrapper, entry.hostEl);
      } else {
        // Find the wrapper's host element in termContainer
        const hostEl = termContainer.querySelector('.term-host');
        if (hostEl) {
          const wrapper = hostEl._termWrapper;
          if (wrapper) {
            wrapper.detachFromDOM();
            cache.put(activeSessionId, wrapper, hostEl);
          }
        }
      }
      activeSessionId = null;
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

    // If switching from one terminal to another, cache the old one
    if (activeSessionId && activeSessionId !== sessionId) {
      const hostEl = termContainer.querySelector('.term-host');
      if (hostEl && hostEl._termWrapper) {
        hostEl._termWrapper.detachFromDOM();
        cache.put(activeSessionId, hostEl._termWrapper, hostEl);
      }
    }

    activeSessionId = sessionId;
    termTitle.textContent = `tmux a -t ${session.name}`;
    updateStateDot(session.state);

    // Show terminal view
    overviewView.classList.remove('active');
    overviewView.classList.add('slide-left');
    terminalView.classList.add('active');

    // Check cache
    const cached = cache.get(sessionId);
    if (cached && cached.wrapper.isAlive) {
      restoreTerminal(cached);
    } else {
      if (cached) cache.evict(sessionId); // stale entry
      initTerminal(sessionId);
    }
  }

  function initTerminal(sessionId) {
    // Create a per-session host div
    const hostEl = document.createElement('div');
    hostEl.className = 'term-host';
    hostEl.style.width = '100%';
    hostEl.style.height = '100%';
    termContainer.appendChild(hostEl);

    const wrapper = new TerminalWrapper(hostEl);
    hostEl._termWrapper = wrapper;

    wrapper.onStateChange((info) => {
      updateStateDot(info.state);
      const s = sessions.find((x) => x.id === sessionId);
      if (s) {
        s.state = info.state;
        s.currentCommand = info.currentCommand;
      }
    });
    wrapper.onSessionExit(() => {
      cache.evict(sessionId);
      if (activeSessionId === sessionId) showOverview();
    });

    wrapper.open();
    wrapper.connect(sessionId);

    // Refit after layout settles, then focus
    const onReady = () => {
      wrapper.fit();
      wrapper.focus();
    };
    terminalView.addEventListener('transitionend', function onEnd() {
      terminalView.removeEventListener('transitionend', onEnd);
      onReady();
    }, { once: true });
    // Fallback if transitionend doesn't fire
    setTimeout(onReady, 350);
  }

  function restoreTerminal(entry) {
    const { wrapper, hostEl } = entry;

    wrapper.attachToDOM(termContainer);

    // Reconnect WS if it dropped while cached
    if (!wrapper.ws || wrapper.ws.readyState !== WebSocket.OPEN) {
      wrapper.reconnect();
    }

    requestAnimationFrame(() => {
      wrapper.fit();
      wrapper.focus();
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

  backBtn.addEventListener('click', showOverview);

  // Tap terminal title to copy tmux attach command
  termTitle.addEventListener('click', () => {
    const cmd = termTitle.textContent;
    navigator.clipboard.writeText(cmd).then(() => {
      const orig = termTitle.textContent;
      termTitle.textContent = 'copied!';
      setTimeout(() => { termTitle.textContent = orig; }, 800);
    }).catch(() => {});
  });

  // Long press on field to create a new session
  let fieldLongPress = null;

  function startFieldLongPress(e) {
    // Don't trigger if touching a thronglet
    const target = e.target || e.srcElement;
    if (target.closest && target.closest('.thronglet')) return;
    fieldLongPress = setTimeout(() => {
      fieldLongPress = null;
      if (navigator.vibrate) navigator.vibrate(50);
      createSession();
    }, LONG_PRESS_MS);
  }

  function cancelFieldLongPress() {
    if (fieldLongPress) {
      clearTimeout(fieldLongPress);
      fieldLongPress = null;
    }
  }

  // Suppress native context menu on the field (long-press on mobile triggers it)
  field.addEventListener('contextmenu', (e) => e.preventDefault());

  // Touch events (mobile)
  field.addEventListener('touchstart', startFieldLongPress, { passive: true });
  field.addEventListener('touchmove', cancelFieldLongPress, { passive: true });
  field.addEventListener('touchend', cancelFieldLongPress, { passive: true });

  // Mouse events (desktop)
  field.addEventListener('mousedown', startFieldLongPress);
  field.addEventListener('mousemove', cancelFieldLongPress);
  field.addEventListener('mouseup', cancelFieldLongPress);
  field.addEventListener('mouseleave', cancelFieldLongPress);

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
