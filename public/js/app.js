// App glue — view routing, session polling, thronglet <-> terminal bridge

(function () {
  const overviewView = document.getElementById('overview-view');
  const terminalView = document.getElementById('terminal-view');
  const emptyState = document.getElementById('empty-state');
  const field = document.getElementById('field');

  let pollInterval = null;
  let currentView = 'overview';
  let sessions = [];
  const cache = new TerminalCache();

  const zm = new ZoneManager({
    zoneMainEl: document.getElementById('zone-main'),
    zoneBottomEl: document.getElementById('zone-bottom'),
    zoneDivider: document.getElementById('zone-divider'),
    overviewView, terminalView,
  });
  zm.onStartPolling = startPolling;
  zm.onStopPolling = stopPolling;

  const renderer = new ThrongletRenderer(
    document.getElementById('thronglets-container'),
    (sessionId) => openTerminal(sessionId),
    (sessionId) => openTerminal(sessionId, 'bottom')
  );

  function updateField() {
    const fieldSessions = sessions.filter(s =>
      s.id !== zm.zones.main && s.id !== zm.zones.bottom
    );
    renderer.update(fieldSessions);
    emptyState.classList.toggle('hidden', sessions.length > 0);
  }

  async function fetchSessions() {
    try {
      sessions = await ThrongApi.fetchSessions();
      updateField();
    } catch (e) { /* network error, keep stale data */ }
  }

  async function createSession() {
    try {
      const session = await ThrongApi.createSession();
      sessions.push(session);
      updateField();
      openTerminal(session.id);
    } catch (e) { console.error('Failed to create session:', e); }
  }

  function openTerminal(sessionId, preferZone) {
    const session = sessions.find(s => s.id === sessionId);
    if (!session) return;

    const pick = zm.pickTargetZone(sessionId, preferZone, cache);
    if (pick.existing) { zm.focusZone(pick.existing); return; }

    const target = pick.target;
    const dismissNeeded = session.state === 'attention';
    if (dismissNeeded) session.state = 'idle';

    const fieldRect = field.getBoundingClientRect();
    renderer.runToZone(sessionId, -80, fieldRect.height / 2);
    zm.assignZone(target, sessionId);

    currentView = 'terminal';
    terminalView.classList.add('active');
    zm.updateLayout();
    updateField();
    zm.updateZoneHeader(target, session);

    const container = zm.getZoneEl(target).querySelector('.zone-terminal');
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
    if (!zm.closeZone(zone, cache)) showOverview();
    else updateField();
  }

  function closeAllZones() {
    zm.closeAllZones(cache);
    updateField();
  }

  // --- Terminal lifecycle ---

  function wireCallbacks(wrapper, sessionId) {
    wrapper.onTitleChange((title) => zm.updateZoneTitle(sessionId, title));
    wrapper.onThought((data) => {
      const s = sessions.find(x => x.id === data.sessionId);
      if (s) s.thought = data.thought;
      updateField();
    });
  }

  function initTerminal(sessionId, container, dismissAttention) {
    const hostEl = document.createElement('div');
    hostEl.className = 'term-host';
    hostEl.style.width = '100%';
    hostEl.style.height = '100%';
    container.appendChild(hostEl);

    const wrapper = new TerminalWrapper(hostEl);
    hostEl._termWrapper = wrapper;
    wireCallbacks(wrapper, sessionId);

    wrapper.onStateChange((info) => {
      if (dismissAttention && info.state === 'attention') {
        wrapper.dismissAttention();
        info.state = 'idle';
        dismissAttention = false;
      }
      zm.updateZoneState(sessionId, info.state);
      const s = sessions.find(x => x.id === sessionId);
      if (s) { s.state = info.state; s.currentCommand = info.currentCommand; }
    });

    wrapper.onSessionExit(() => {
      cache.evict(sessionId);
      for (const z of ['main', 'bottom']) {
        if (zm.zones[z] === sessionId) zm.clearZone(z);
      }
      if (!zm.zones.main && !zm.zones.bottom) showOverview();
      else { zm.updateLayout(); updateField(); }
    });

    wrapper.open();
    wrapper.connect(sessionId);
    setTimeout(() => { wrapper.fit(); wrapper.focus(); }, 350);
  }

  function restoreTerminal(entry, container, sessionId) {
    const { wrapper } = entry;
    wrapper.attachToDOM(container);
    wireCallbacks(wrapper, sessionId);
    if (!wrapper.ws || wrapper.ws.readyState !== WebSocket.OPEN) wrapper.reconnect();
    requestAnimationFrame(() => { wrapper.fit(); wrapper.focus(); });
  }

  function showOverview() {
    currentView = 'overview';
    zm.resetToOverview();
    startPolling();
    fetchSessions();
  }

  function startPolling() {
    stopPolling();
    pollInterval = setInterval(fetchSessions, 2000);
  }

  function stopPolling() {
    if (pollInterval) { clearInterval(pollInterval); pollInterval = null; }
  }

  // --- Events & Init ---

  zm.zoneMainEl.querySelector('.zone-sprite').addEventListener('click', () => closeZone('main'));
  zm.zoneBottomEl.querySelector('.zone-sprite').addEventListener('click', () => closeZone('bottom'));
  window.addEventListener('resize', () => { if (currentView === 'terminal') zm.updateLayout(); });

  zm.initDividerDrag();
  zm.initZoneTitleCopy();
  Gestures.initFieldLongPress(field, createSession);
  Gestures.initSwipeBack(terminalView, () => { closeAllZones(); showOverview(); });
  fetchSessions();
  startPolling();
})();
