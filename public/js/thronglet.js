// Thronglet renderer — DOM-based pixel art characters that wander the field

class ThrongletRenderer {
  constructor(container, onTap, onDragToBottom) {
    this.container = container;
    this.onTap = onTap;
    this.onDragToBottom = onDragToBottom;
    this.thronglets = new Map(); // id → { el, x, y, wanderInterval, showingNum }
    this._fieldRect = null;
  }

  _getFieldRect() {
    return this.container.getBoundingClientRect();
  }

  _spriteForState(state) {
    const map = {
      idle: '/assets/idle.png',
      busy: '/assets/walking.png',
      error: '/assets/beep.png',
      attention: '/assets/idle.png',
    };
    return map[state] || map.idle;
  }

  _repoName(cwd) {
    if (!cwd || cwd === '/') return 'root';
    const parts = cwd.replace(/\/+$/, '').split('/');
    return parts[parts.length - 1] || 'root';
  }

  _numberPrefix(name) {
    const num = parseInt(name, 10);
    if (isNaN(num)) return name;
    return String(num).slice(-2);
  }

  update(sessions) {
    const currentIds = new Set(sessions.map((s) => s.id));

    // Remove thronglets for deleted sessions
    for (const [id, t] of this.thronglets) {
      if (!currentIds.has(id)) {
        clearInterval(t.wanderInterval);
        t.el.remove();
        this.thronglets.delete(id);
      }
    }

    const rect = this._getFieldRect();
    const fieldW = rect.width - 80;
    const fieldH = rect.height - 120;

    for (const session of sessions) {
      let t = this.thronglets.get(session.id);

      if (!t) {
        // Create new thronglet
        const el = document.createElement('div');
        el.className = `thronglet ${session.state}`;
        const toolBadge = session.tool
          ? `<div class="thronglet-tool">${this._escapeHtml(session.tool)}</div>`
          : '';
        el.innerHTML = `
          <div class="thought-bubble" style="display:none"></div>
          ${toolBadge}
          <img class="thronglet-sprite" src="${this._spriteForState(session.state)}" alt="">
          <div class="thronglet-label">
            <div class="thronglet-name">${this._escapeHtml(this._repoName(session.cwd))}</div>
            <div class="thronglet-activity"></div>
          </div>
        `;

        // Random initial position
        const x = Math.random() * Math.max(fieldW, 100);
        const y = 40 + Math.random() * Math.max(fieldH - 40, 60);
        el.style.left = x + 'px';
        el.style.top = y + 'px';

        // Tap to open terminal (but not if long pressed or dragging)
        el.addEventListener('click', () => {
          if (el._longPressed || el._isDragging) {
            el._longPressed = false;
            return;
          }
          if (this.onTap) this.onTap(session.id);
        });

        this._enableDrag(el, session.id);

        // Long press to reveal session number
        let lpTimer = null;
        el.addEventListener('touchstart', (e) => {
          lpTimer = setTimeout(() => {
            lpTimer = null;
            el._longPressed = true;
            if (navigator.vibrate) navigator.vibrate(30);

            // Show number prefix
            const nameEl = el.querySelector('.thronglet-name');
            const prefix = this._numberPrefix(session.name);
            const repo = this._repoName(session.cwd);
            nameEl.textContent = `${prefix} ${repo}`;
            t.showingNum = true;

            // Auto-hide after 2s
            setTimeout(() => {
              t.showingNum = false;
              nameEl.textContent = repo;
            }, 2000);
          }, 500);
        }, { passive: true });

        el.addEventListener('touchmove', () => {
          if (lpTimer) { clearTimeout(lpTimer); lpTimer = null; }
        }, { passive: true });

        el.addEventListener('touchend', () => {
          if (lpTimer) { clearTimeout(lpTimer); lpTimer = null; }
        }, { passive: true });

        this.container.appendChild(el);

        // Wander randomly
        const wanderInterval = setInterval(() => {
          const curRect = this._getFieldRect();
          const maxX = curRect.width - 80;
          const maxY = curRect.height - 120;
          const newX = Math.max(0, Math.min(maxX, t.x + (Math.random() - 0.5) * 100));
          const newY = Math.max(40, Math.min(maxY, t.y + (Math.random() - 0.5) * 80));
          t.x = newX;
          t.y = newY;
          el.style.left = newX + 'px';
          el.style.top = newY + 'px';
        }, 3000);

        t = { el, x, y, wanderInterval, showingNum: false };
        this.thronglets.set(session.id, t);
      }

      // Update state
      t.el.className = `thronglet ${session.state}`;
      const sprite = t.el.querySelector('.thronglet-sprite');
      sprite.src = this._spriteForState(session.state);

      // Update tool badge
      let toolEl = t.el.querySelector('.thronglet-tool');
      if (session.tool) {
        if (!toolEl) {
          toolEl = document.createElement('div');
          toolEl.className = 'thronglet-tool';
          t.el.insertBefore(toolEl, t.el.querySelector('.thronglet-sprite'));
        }
        toolEl.textContent = session.tool;
      } else if (toolEl) {
        toolEl.remove();
      }

      // Update name (tracks CWD changes) — but don't overwrite if showing number
      const nameEl = t.el.querySelector('.thronglet-name');
      if (!t.showingNum) {
        nameEl.textContent = this._repoName(session.cwd);
      }

      // Update activity label
      const activity = t.el.querySelector('.thronglet-activity');
      const bubble = t.el.querySelector('.thought-bubble');

      if (session.state === 'busy' && session.currentCommand) {
        activity.textContent = session.currentCommand;
        bubble.textContent = session.currentCommand;
        bubble.style.display = '';
      } else if (session.state === 'error') {
        activity.textContent = 'error!';
        bubble.textContent = '!!!';
        bubble.style.display = '';
      } else if (session.state === 'attention') {
        activity.textContent = 'ready';
        bubble.textContent = '?';
        bubble.style.display = '';
      } else {
        activity.textContent = '';
        bubble.style.display = 'none';
      }
    }
  }

  _enableDrag(el, sessionId) {
    let startX, startY, dragging, ghost, hint;

    el.addEventListener('mousedown', (e) => {
      if (e.button !== 0) return;
      startX = e.clientX;
      startY = e.clientY;
      dragging = false;

      const onMouseMove = (e) => {
        const dy = e.clientY - startY;
        const dx = e.clientX - startX;

        if (!dragging && dy > 10 && Math.abs(dy) > Math.abs(dx)) {
          dragging = true;
          el._isDragging = true;
          el.classList.add('dragging');

          const sprite = el.querySelector('.thronglet-sprite');
          ghost = sprite.cloneNode(true);
          ghost.className = 'drag-ghost';
          ghost.style.width = '64px';
          ghost.style.height = '64px';
          document.body.appendChild(ghost);

          hint = document.createElement('div');
          hint.className = 'drop-zone-hint';
          this.container.appendChild(hint);
        }

        if (dragging && ghost) {
          ghost.style.left = (e.clientX - 32) + 'px';
          ghost.style.top = (e.clientY - 32) + 'px';
        }
      };

      const onMouseUp = (e) => {
        document.removeEventListener('mousemove', onMouseMove);
        document.removeEventListener('mouseup', onMouseUp);

        if (dragging) {
          const dy = e.clientY - startY;
          el.classList.remove('dragging');
          if (ghost) { ghost.remove(); ghost = null; }
          if (hint) { hint.remove(); hint = null; }

          if (dy > 100 && this.onDragToBottom) {
            this.onDragToBottom(sessionId);
          }

          setTimeout(() => { el._isDragging = false; }, 50);
        }
      };

      document.addEventListener('mousemove', onMouseMove);
      document.addEventListener('mouseup', onMouseUp);
    });
  }

  removeThronglet(sessionId) {
    const t = this.thronglets.get(sessionId);
    if (!t) return;
    clearInterval(t.wanderInterval);
    t.el.remove();
    this.thronglets.delete(sessionId);
  }

  _escapeHtml(s) {
    const d = document.createElement('div');
    d.textContent = s;
    return d.innerHTML;
  }
}
