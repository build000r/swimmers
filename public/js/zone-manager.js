// Zone management — session-to-zone assignment, layout, divider dragging, headers

window.ZoneManager = class ZoneManager {
  constructor({ zoneMainEl, zoneBottomEl, zoneDivider, overviewView, terminalView }) {
    this.zoneMainEl = zoneMainEl;
    this.zoneBottomEl = zoneBottomEl;
    this.zoneDivider = zoneDivider;
    this.overviewView = overviewView;
    this.terminalView = terminalView;

    // Zone state
    this.zones = { main: null, bottom: null };     // sessionId or null
    this.zoneAge = { main: 0, bottom: 0 };         // timestamp for "replace oldest"
    this.zoneSplitRatio = 0.6;

    // Callbacks set by app.js
    this.onStartPolling = null;
    this.onStopPolling = null;
  }

  isDesktop() {
    return window.innerWidth > 768;
  }

  getZoneEl(zone) {
    return zone === 'main' ? this.zoneMainEl : this.zoneBottomEl;
  }

  // --- Helpers ---

  repoName(cwd) {
    if (!cwd || cwd === '/') return 'root';
    const parts = cwd.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || 'root';
  }

  spriteForState(state) {
    const map = { idle: '/assets/idle.png', busy: '/assets/walking.png', error: '/assets/beep.png', attention: '/assets/idle.png' };
    return map[state] || map.idle;
  }

  // --- Zone header ---

  updateZoneHeader(zone, session) {
    const zoneEl = this.getZoneEl(zone);
    const sprite = zoneEl.querySelector('.zone-sprite');
    const name = zoneEl.querySelector('.zone-name');
    const title = zoneEl.querySelector('.zone-title');
    const dot = zoneEl.querySelector('.zone-dot');
    const displayState = session.state || 'idle';

    sprite.src = this.spriteForState(displayState);
    name.textContent = this.repoName(session.cwd);
    title.textContent = `tmux a -t ${session.name}`;
    dot.className = 'zone-dot state-dot ' + displayState;
  }

  // --- Cache helpers ---

  cacheZone(zone, cache) {
    const sessionId = this.zones[zone];
    if (!sessionId) return;

    const zoneEl = this.getZoneEl(zone);
    const container = zoneEl.querySelector('.zone-terminal');
    const hostEl = container.querySelector('.term-host');

    if (hostEl && hostEl._termWrapper) {
      hostEl._termWrapper.detachFromDOM();
      cache.put(sessionId, hostEl._termWrapper, hostEl);
    }
  }

  // --- Zone target selection ---

  pickTargetZone(sessionId, preferZone, cache) {
    // Already in a zone? Return which one
    if (this.zones.main === sessionId) return { existing: 'main' };
    if (this.zones.bottom === sessionId) return { existing: 'bottom' };

    let target;
    if (!this.isDesktop()) {
      target = 'main';
      // Mobile: close bottom zone if occupied
      if (this.zones.bottom) {
        this.cacheZone('bottom', cache);
        this.zones.bottom = null;
        this.zoneAge.bottom = 0;
      }
    } else if (preferZone) {
      target = preferZone;
    } else if (!this.zones.main) {
      target = 'main';
    } else if (!this.zones.bottom) {
      target = 'bottom';
    } else {
      // Both occupied, no preference -> replace oldest
      target = this.zoneAge.main <= this.zoneAge.bottom ? 'main' : 'bottom';
    }

    // Cache current occupant if target is occupied
    if (this.zones[target]) {
      this.cacheZone(target, cache);
    }

    return { target };
  }

  assignZone(target, sessionId) {
    this.zones[target] = sessionId;
    this.zoneAge[target] = Date.now();
  }

  // --- Close zones ---

  closeZone(zone, cache) {
    this.cacheZone(zone, cache);
    this.zones[zone] = null;
    this.zoneAge[zone] = 0;

    const anyOpen = this.zones.main || this.zones.bottom;
    if (anyOpen) {
      this.updateLayout();
    }
    return anyOpen;
  }

  closeAllZones(cache) {
    for (const zone of ['main', 'bottom']) {
      if (this.zones[zone]) {
        this.cacheZone(zone, cache);
        this.zones[zone] = null;
        this.zoneAge[zone] = 0;
      }
    }
  }

  clearZone(zone) {
    this.zones[zone] = null;
    this.zoneAge[zone] = 0;
  }

  // --- Focus ---

  focusZone(zone) {
    const zoneEl = this.getZoneEl(zone);
    const hostEl = zoneEl.querySelector('.term-host');
    if (hostEl && hostEl._termWrapper) {
      hostEl._termWrapper.focus();
    }
  }

  fitZone(zone) {
    if (!this.zones[zone]) return;
    const zoneEl = this.getZoneEl(zone);
    const hostEl = zoneEl.querySelector('.term-host');
    if (hostEl && hostEl._termWrapper) {
      hostEl._termWrapper.fit();
    }
  }

  // --- Layout ---

  updateLayout() {
    const mainOccupied = !!this.zones.main;
    const bottomOccupied = !!this.zones.bottom;
    const dualZone = mainOccupied && bottomOccupied;
    const singleZone = (mainOccupied || bottomOccupied) && !dualZone;

    this.zoneMainEl.style.display = mainOccupied ? 'flex' : 'none';
    this.zoneBottomEl.style.display = bottomOccupied ? 'flex' : 'none';
    this.zoneDivider.style.display = dualZone ? 'block' : 'none';

    if (dualZone) {
      // Full-screen dual terminal side-by-side, hide field
      if (this.onStopPolling) this.onStopPolling();
      this.overviewView.classList.remove('active');
      this.overviewView.classList.add('slide-left');
      this.overviewView.style.left = '';
      this.terminalView.style.right = '';
      this.terminalView.style.flexDirection = 'row';
      const dividerWidth = 4;
      this.zoneMainEl.style.width = `calc(${this.zoneSplitRatio * 100}% - ${dividerWidth / 2}px)`;
      this.zoneMainEl.style.height = '';
      this.zoneBottomEl.style.width = `calc(${(1 - this.zoneSplitRatio) * 100}% - ${dividerWidth / 2}px)`;
      this.zoneBottomEl.style.height = '';
    } else if (singleZone) {
      // 50/50 vertical: terminal left half, field right half
      if (this.onStartPolling) this.onStartPolling();
      this.overviewView.classList.add('active');
      this.overviewView.classList.remove('slide-left');
      this.overviewView.style.left = '50%';
      this.terminalView.style.right = '50%';
      this.terminalView.style.flexDirection = 'row';
      const z = mainOccupied ? 'main' : 'bottom';
      const zoneEl = this.getZoneEl(z);
      zoneEl.style.width = '100%';
      zoneEl.style.height = '';
    }

    // Refit terminals after layout changes
    requestAnimationFrame(() => {
      this.fitZone('main');
      this.fitZone('bottom');
    });
  }

  // --- Divider drag ---

  initDividerDrag() {
    let dragging = false;

    this.zoneDivider.addEventListener('mousedown', (e) => {
      e.preventDefault();
      dragging = true;
      this.zoneDivider.classList.add('dragging');
      document.body.style.cursor = 'ew-resize';
      document.body.style.userSelect = 'none';
    });

    document.addEventListener('mousemove', (e) => {
      if (!dragging) return;
      const viewRect = this.terminalView.getBoundingClientRect();
      const relX = e.clientX - viewRect.left;
      this.zoneSplitRatio = Math.max(0.2, Math.min(0.8, relX / viewRect.width));
      this.updateLayout();
    });

    document.addEventListener('mouseup', () => {
      if (!dragging) return;
      dragging = false;
      this.zoneDivider.classList.remove('dragging');
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
      this.fitZone('main');
      this.fitZone('bottom');
    });
  }

  // --- Wrapper callback helpers (used by app.js terminal lifecycle) ---

  updateZoneTitle(sessionId, title) {
    for (const z of ['main', 'bottom']) {
      if (this.zones[z] === sessionId) {
        this.getZoneEl(z).querySelector('.zone-title').textContent = title;
      }
    }
  }

  updateZoneState(sessionId, state) {
    for (const z of ['main', 'bottom']) {
      if (this.zones[z] === sessionId) {
        const zoneEl = this.getZoneEl(z);
        zoneEl.querySelector('.zone-dot').className = 'zone-dot state-dot ' + (state || 'idle');
        zoneEl.querySelector('.zone-sprite').src = this.spriteForState(state);
      }
    }
  }

  // --- View reset ---

  resetToOverview() {
    this.overviewView.style.left = '';
    this.overviewView.style.top = '';
    this.terminalView.style.right = '';
    this.terminalView.style.bottom = '';
    this.terminalView.style.flexDirection = '';
    this.overviewView.classList.add('active');
    this.overviewView.classList.remove('slide-left');
    this.terminalView.classList.remove('active');
  }

  // --- Zone title copy ---

  initZoneTitleCopy() {
    for (const zoneEl of [this.zoneMainEl, this.zoneBottomEl]) {
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
  }
};
