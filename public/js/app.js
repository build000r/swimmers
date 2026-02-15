// App glue — view routing, session polling, thronglet ↔ terminal bridge

(function () {
  const overviewView = document.getElementById('overview-view');
  const terminalView = document.getElementById('terminal-view');
  const emptyState = document.getElementById('empty-state');
  const field = document.getElementById('field');

  // Zone elements
  const zoneMainEl = document.getElementById('zone-main');
  const zoneBottomEl = document.getElementById('zone-bottom');
  const zoneDivider = document.getElementById('zone-divider');

  let pollInterval = null;
  let currentView = 'overview';
  let sessions = [];
  const cache = new TerminalCache();

  // Zone state
  const zones = { main: null, bottom: null };   // sessionId or null
  const zoneAge = { main: 0, bottom: 0 };       // timestamp for "replace oldest"
  let zoneSplitRatio = 0.6;

  const LONG_PRESS_MS = 500;

  function isDesktop() {
    return window.innerWidth > 768;
  }

  const renderer = new ThrongletRenderer(
    document.getElementById('thronglets-container'),
    (sessionId) => openTerminal(sessionId),
    (sessionId) => openTerminal(sessionId, 'bottom')
  );

  // --- Helpers ---

  function repoName(cwd) {
    if (!cwd || cwd === '/') return 'root';
    const parts = cwd.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || 'root';
  }

  function spriteForState(state) {
    const map = { idle: '/assets/idle.png', busy: '/assets/walking.png', error: '/assets/beep.png', attention: '/assets/idle.png' };
    return map[state] || map.idle;
  }

  function getZoneEl(zone) {
    return zone === 'main' ? zoneMainEl : zoneBottomEl;
  }

  // --- API ---

  async function fetchSessions() {
    try {
      const res = await fetch('/api/sessions');
      sessions = await res.json();
      updateField();
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
      updateField();
      openTerminal(session.id);
    } catch (e) {
      console.error('Failed to create session:', e);
    }
  }

  // --- Field filtering ---

  function updateField() {
    const fieldSessions = sessions.filter(s =>
      s.id !== zones.main && s.id !== zones.bottom
    );
    renderer.update(fieldSessions);
    emptyState.classList.toggle('hidden', sessions.length > 0);
  }

  // --- Zone management ---

  function cacheZone(zone) {
    const sessionId = zones[zone];
    if (!sessionId) return;

    const zoneEl = getZoneEl(zone);
    const container = zoneEl.querySelector('.zone-terminal');
    const hostEl = container.querySelector('.term-host');

    if (hostEl && hostEl._termWrapper) {
      hostEl._termWrapper.detachFromDOM();
      cache.put(sessionId, hostEl._termWrapper, hostEl);
    }
  }

  function openTerminal(sessionId, preferZone) {
    const session = sessions.find(s => s.id === sessionId);
    if (!session) return;

    // Already in a zone? Just focus it
    if (zones.main === sessionId || zones.bottom === sessionId) {
      focusZone(zones.main === sessionId ? 'main' : 'bottom');
      return;
    }

    // Determine target zone
    let target;
    if (!isDesktop()) {
      target = 'main';
      // Mobile: close bottom zone if occupied
      if (zones.bottom) {
        cacheZone('bottom');
        zones.bottom = null;
        zoneAge.bottom = 0;
      }
    } else if (preferZone) {
      target = preferZone;
    } else if (!zones.main) {
      target = 'main';
    } else if (!zones.bottom) {
      target = 'bottom';
    } else {
      // Both occupied, no preference → replace oldest
      target = zoneAge.main <= zoneAge.bottom ? 'main' : 'bottom';
    }

    // Cache current occupant if target is occupied
    if (zones[target]) {
      cacheZone(target);
    }

    // Handle attention state
    const dismissNeeded = session.state === 'attention';
    if (dismissNeeded) session.state = 'idle';

    // Trigger run-to-zone animation before removing from field
    const fieldRect = field.getBoundingClientRect();
    renderer.runToZone(sessionId, -80, fieldRect.height / 2);

    // Assign session to zone
    zones[target] = sessionId;
    zoneAge[target] = Date.now();

    // Update views and layout
    currentView = 'terminal';
    terminalView.classList.add('active');
    updateLayout();
    updateField();

    // Update zone header
    updateZoneHeader(target, session);

    // Init or restore terminal
    const zoneEl = getZoneEl(target);
    const container = zoneEl.querySelector('.zone-terminal');

    const cached = cache.get(sessionId);
    if (cached && cached.wrapper.isAlive) {
      restoreTerminal(cached, container, sessionId);
      if (dismissNeeded) cached.wrapper.dismissAttention();
    } else {
      if (cached) cache.evict(sessionId);
      initTerminal(sessionId, container, dismissNeeded);
    }
  }

  function closeZone(zone) {
    cacheZone(zone);
    zones[zone] = null;
    zoneAge[zone] = 0;

    if (!zones.main && !zones.bottom) {
      showOverview();
    } else {
      updateLayout();
      updateField();
    }
  }

  function closeAllZones() {
    for (const zone of ['main', 'bottom']) {
      if (zones[zone]) {
        cacheZone(zone);
        zones[zone] = null;
        zoneAge[zone] = 0;
      }
    }
    updateField();
  }

  function focusZone(zone) {
    const zoneEl = getZoneEl(zone);
    const hostEl = zoneEl.querySelector('.term-host');
    if (hostEl && hostEl._termWrapper) {
      hostEl._termWrapper.focus();
    }
  }

  function updateZoneHeader(zone, session) {
    const zoneEl = getZoneEl(zone);
    const sprite = zoneEl.querySelector('.zone-sprite');
    const name = zoneEl.querySelector('.zone-name');
    const title = zoneEl.querySelector('.zone-title');
    const dot = zoneEl.querySelector('.zone-dot');
    const displayState = session.state || 'idle';

    sprite.src = spriteForState(displayState);
    name.textContent = repoName(session.cwd);
    title.textContent = `tmux a -t ${session.name}`;
    dot.className = 'zone-dot state-dot ' + displayState;
  }

  function updateLayout() {
    const mainOccupied = !!zones.main;
    const bottomOccupied = !!zones.bottom;
    const dualZone = mainOccupied && bottomOccupied;
    const singleZone = (mainOccupied || bottomOccupied) && !dualZone;

    zoneMainEl.style.display = mainOccupied ? 'flex' : 'none';
    zoneBottomEl.style.display = bottomOccupied ? 'flex' : 'none';
    zoneDivider.style.display = dualZone ? 'block' : 'none';

    if (dualZone) {
      // Full-screen dual terminal side-by-side, hide field
      stopPolling();
      overviewView.classList.remove('active');
      overviewView.classList.add('slide-left');
      overviewView.style.left = '';
      terminalView.style.right = '';
      terminalView.style.flexDirection = 'row';
      const dividerWidth = 4;
      zoneMainEl.style.width = `calc(${zoneSplitRatio * 100}% - ${dividerWidth / 2}px)`;
      zoneMainEl.style.height = '';
      zoneBottomEl.style.width = `calc(${(1 - zoneSplitRatio) * 100}% - ${dividerWidth / 2}px)`;
      zoneBottomEl.style.height = '';
    } else if (singleZone) {
      // 50/50 vertical: terminal left half, field right half
      startPolling();
      overviewView.classList.add('active');
      overviewView.classList.remove('slide-left');
      overviewView.style.left = '50%';
      terminalView.style.right = '50%';
      terminalView.style.flexDirection = 'row';
      const z = mainOccupied ? 'main' : 'bottom';
      const zoneEl = getZoneEl(z);
      zoneEl.style.width = '100%';
      zoneEl.style.height = '';
    }

    // Refit terminals after layout changes
    requestAnimationFrame(() => {
      fitZone('main');
      fitZone('bottom');
    });
  }

  function fitZone(zone) {
    if (!zones[zone]) return;
    const zoneEl = getZoneEl(zone);
    const hostEl = zoneEl.querySelector('.term-host');
    if (hostEl && hostEl._termWrapper) {
      hostEl._termWrapper.fit();
    }
  }

  // --- Terminal lifecycle ---

  function initTerminal(sessionId, container, dismissAttention) {
    const hostEl = document.createElement('div');
    hostEl.className = 'term-host';
    hostEl.style.width = '100%';
    hostEl.style.height = '100%';
    container.appendChild(hostEl);

    const wrapper = new TerminalWrapper(hostEl);
    hostEl._termWrapper = wrapper;

    wrapper.onTitleChange((title) => {
      for (const z of ['main', 'bottom']) {
        if (zones[z] === sessionId) {
          getZoneEl(z).querySelector('.zone-title').textContent = title;
        }
      }
    });

    wrapper.onStateChange((info) => {
      if (dismissAttention && info.state === 'attention') {
        wrapper.dismissAttention();
        info.state = 'idle';
        dismissAttention = false;
      }
      for (const z of ['main', 'bottom']) {
        if (zones[z] === sessionId) {
          const zoneEl = getZoneEl(z);
          zoneEl.querySelector('.zone-dot').className = 'zone-dot state-dot ' + (info.state || 'idle');
          zoneEl.querySelector('.zone-sprite').src = spriteForState(info.state);
        }
      }
      const s = sessions.find(x => x.id === sessionId);
      if (s) {
        s.state = info.state;
        s.currentCommand = info.currentCommand;
      }
    });

    wrapper.onThought((data) => {
      const s = sessions.find(x => x.id === data.sessionId);
      if (s) s.thought = data.thought;
      updateField();
    });

    wrapper.onSessionExit(() => {
      cache.evict(sessionId);
      for (const z of ['main', 'bottom']) {
        if (zones[z] === sessionId) {
          zones[z] = null;
          zoneAge[z] = 0;
        }
      }
      if (!zones.main && !zones.bottom) {
        showOverview();
      } else {
        updateLayout();
        updateField();
      }
    });

    wrapper.open();
    wrapper.connect(sessionId);

    // Refit after layout settles
    const onReady = () => {
      wrapper.fit();
      wrapper.focus();
    };
    setTimeout(onReady, 350);
  }

  function restoreTerminal(entry, container, sessionId) {
    const { wrapper } = entry;

    wrapper.attachToDOM(container);

    // Update title callback for new zone context
    wrapper.onTitleChange((title) => {
      for (const z of ['main', 'bottom']) {
        if (zones[z] === sessionId) {
          getZoneEl(z).querySelector('.zone-title').textContent = title;
        }
      }
    });

    wrapper.onThought((data) => {
      const s = sessions.find(x => x.id === data.sessionId);
      if (s) s.thought = data.thought;
      updateField();
    });

    if (!wrapper.ws || wrapper.ws.readyState !== WebSocket.OPEN) {
      wrapper.reconnect();
    }

    requestAnimationFrame(() => {
      wrapper.fit();
      wrapper.focus();
    });
  }

  // --- Views ---

  function showOverview() {
    currentView = 'overview';
    // Reset inline positioning
    overviewView.style.left = '';
    overviewView.style.top = '';
    terminalView.style.right = '';
    terminalView.style.bottom = '';
    terminalView.style.flexDirection = '';
    overviewView.classList.add('active');
    overviewView.classList.remove('slide-left');
    terminalView.classList.remove('active');
    startPolling();
    fetchSessions();
  }

  // --- Divider drag ---

  function initDividerDrag() {
    let dragging = false;

    zoneDivider.addEventListener('mousedown', (e) => {
      e.preventDefault();
      dragging = true;
      zoneDivider.classList.add('dragging');
      document.body.style.cursor = 'ew-resize';
      document.body.style.userSelect = 'none';
    });

    document.addEventListener('mousemove', (e) => {
      if (!dragging) return;
      const viewRect = terminalView.getBoundingClientRect();
      const relX = e.clientX - viewRect.left;
      zoneSplitRatio = Math.max(0.2, Math.min(0.8, relX / viewRect.width));
      updateLayout();
    });

    document.addEventListener('mouseup', () => {
      if (!dragging) return;
      dragging = false;
      zoneDivider.classList.remove('dragging');
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      fitZone('main');
      fitZone('bottom');
    });
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

  // Zone sprite clicks → close zone
  zoneMainEl.querySelector('.zone-sprite').addEventListener('click', () => closeZone('main'));
  zoneBottomEl.querySelector('.zone-sprite').addEventListener('click', () => closeZone('bottom'));

  // Tap zone title to copy tmux attach command
  for (const zoneEl of [zoneMainEl, zoneBottomEl]) {
    zoneEl.querySelector('.zone-title').addEventListener('click', (e) => {
      const titleEl = e.target;
      const cmd = titleEl.textContent;
      navigator.clipboard.writeText(cmd).then(() => {
        const orig = cmd;
        titleEl.textContent = 'copied!';
        setTimeout(() => { titleEl.textContent = orig; }, 800);
      }).catch(() => {});
    });
  }

  // Long press on field to create a new session
  let fieldLongPress = null;

  function startFieldLongPress(e) {
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

  field.addEventListener('contextmenu', (e) => e.preventDefault());
  field.addEventListener('touchstart', startFieldLongPress, { passive: true });
  field.addEventListener('touchmove', cancelFieldLongPress, { passive: true });
  field.addEventListener('touchend', cancelFieldLongPress, { passive: true });
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
      closeAllZones();
      showOverview();
    }
  }, { passive: true });

  // Window resize → refit zones
  window.addEventListener('resize', () => {
    if (currentView === 'terminal') {
      updateLayout();
    }
  });

  // --- Init ---

  initDividerDrag();
  fetchSessions();
  startPolling();
})();
