// Thronglet renderer — DOM-based pixel art characters that wander the field

class ThrongletRenderer {
  constructor(container, onTap) {
    this.container = container;
    this.onTap = onTap;
    this.thronglets = new Map(); // id → { el, x, y, wanderInterval }
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
    };
    return map[state] || map.idle;
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
        el.innerHTML = `
          <div class="thought-bubble" style="display:none"></div>
          <img class="thronglet-sprite" src="${this._spriteForState(session.state)}" alt="">
          <div class="thronglet-label">
            <div class="thronglet-name">${this._escapeHtml(session.name)}</div>
            <div class="thronglet-activity"></div>
          </div>
        `;

        // Random initial position
        const x = Math.random() * Math.max(fieldW, 100);
        const y = 40 + Math.random() * Math.max(fieldH - 40, 60);
        el.style.left = x + 'px';
        el.style.top = y + 'px';

        el.addEventListener('click', () => {
          if (this.onTap) this.onTap(session.id);
        });

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

        t = { el, x, y, wanderInterval };
        this.thronglets.set(session.id, t);
      }

      // Update state
      t.el.className = `thronglet ${session.state}`;
      const sprite = t.el.querySelector('.thronglet-sprite');
      sprite.src = this._spriteForState(session.state);

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
      } else {
        activity.textContent = '';
        bubble.style.display = 'none';
      }
    }
  }

  _escapeHtml(s) {
    const d = document.createElement('div');
    d.textContent = s;
    return d.innerHTML;
  }
}
