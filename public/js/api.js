// API client — all fetch calls to /api/sessions endpoints

window.ThrongApi = {
  async fetchSessions() {
    const res = await fetch('/api/sessions');
    return res.json();
  },

  async createSession() {
    const res = await fetch('/api/sessions', { method: 'POST' });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error || `HTTP ${res.status}`);
    }
    return res.json();
  },

  async deleteSession(id) {
    const res = await fetch(`/api/sessions/${id}`, { method: 'DELETE' });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error || `HTTP ${res.status}`);
    }
    return res.json().catch(() => ({}));
  },
};
