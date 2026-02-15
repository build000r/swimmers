// Terminal cache — keeps xterm + WebSocket alive across view navigations
// Entries are evicted after TTL_MS of inactivity (i.e. while the terminal
// is not the active view).  Navigating back clears the timer.

class TerminalCache {
  constructor(ttlMs = 5 * 60 * 1000) {
    this.ttlMs = ttlMs;
    this._entries = new Map(); // sessionId → { wrapper, hostEl, timer }
  }

  get(id) {
    const entry = this._entries.get(id);
    if (!entry) return null;
    clearTimeout(entry.timer);
    entry.timer = null;
    return entry;
  }

  put(id, wrapper, hostEl) {
    // If there's already an entry for this id, clear its old timer
    const existing = this._entries.get(id);
    if (existing) clearTimeout(existing.timer);

    const timer = setTimeout(() => this.evict(id), this.ttlMs);
    this._entries.set(id, { wrapper, hostEl, timer });
  }

  evict(id) {
    const entry = this._entries.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    entry.wrapper.disconnect();
    if (entry.hostEl.parentNode) entry.hostEl.parentNode.removeChild(entry.hostEl);
    this._entries.delete(id);
  }

  has(id) {
    return this._entries.has(id);
  }
}
