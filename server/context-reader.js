const fs = require('fs');
const path = require('path');

const BOOTSTRAP_CHUNK = 128 * 1024; // 128KB chunks for initial backward scan
const BOOTSTRAP_MAX = 1024 * 1024; // 1MB max for bootstrap

/**
 * Read a byte range from a file without reading the whole thing.
 * Safe for append-only files being written by another process.
 */
function readRange(filePath, start, end) {
  const fd = fs.openSync(filePath, 'r');
  try {
    const len = end - start;
    const buf = Buffer.alloc(len);
    fs.readSync(fd, buf, 0, len, start);
    return buf;
  } finally {
    fs.closeSync(fd);
  }
}

/**
 * Parse JSONL lines from a buffer, skipping malformed lines.
 */
function parseJsonlLines(buf) {
  const text = buf.toString('utf-8');
  const lines = text.split('\n').filter(Boolean);
  const results = [];
  for (const line of lines) {
    try {
      results.push(JSON.parse(line));
    } catch {
      // partial line at chunk boundary — skip
    }
  }
  return results;
}

// --- Claude Code Reader ---

class ClaudeCodeReader {
  constructor(cwd) {
    this.cwd = cwd;
    this._filePath = null;
    this._fileSize = 0;
    this._userTask = null;
    this._recentActions = [];
    this._currentTool = null;
    this._lastTimestamp = null;
    this._bootstrapped = false;
  }

  /**
   * Claude Code stores sessions in ~/.claude/projects/-{cwd-dashes}/{SESSION}.jsonl
   * Find the most recently modified one.
   */
  _discoverFile() {
    const cwdSlug = this.cwd.replace(/\//g, '-');
    const projectDir = path.join(
      process.env.HOME,
      '.claude',
      'projects',
      cwdSlug
    );

    try {
      const files = fs.readdirSync(projectDir)
        .filter((f) => f.endsWith('.jsonl'))
        .map((f) => {
          const full = path.join(projectDir, f);
          return { path: full, mtime: fs.statSync(full).mtimeMs };
        })
        .sort((a, b) => b.mtime - a.mtime);

      return files.length > 0 ? files[0].path : null;
    } catch {
      return null;
    }
  }

  _parseEntries(entries) {
    for (const entry of entries) {
      const type = entry.type;
      const msg = entry.message;

      // User message → task
      if (type === 'user' && msg && msg.role === 'user') {
        const content = msg.content;
        if (typeof content === 'string' && content.trim()) {
          this._userTask = content.trim().slice(0, 300);
        } else if (Array.isArray(content)) {
          for (const block of content) {
            if (block.type === 'text' && block.text && block.text.trim()) {
              this._userTask = block.text.trim().slice(0, 300);
              break;
            }
          }
        }
      }

      // Assistant message → tool uses and text
      if (type === 'assistant' && msg && msg.role === 'assistant' && Array.isArray(msg.content)) {
        for (const block of msg.content) {
          if (block.type === 'tool_use') {
            const action = { tool: block.name };
            // Extract a short detail from input
            if (block.input) {
              if (block.input.file_path) {
                action.detail = path.basename(block.input.file_path);
              } else if (block.input.command) {
                action.detail = block.input.command.slice(0, 80);
              } else if (block.input.pattern) {
                action.detail = block.input.pattern;
              }
            }
            this._recentActions.push(action);
            // Keep only last 10
            if (this._recentActions.length > 10) {
              this._recentActions = this._recentActions.slice(-10);
            }
            this._currentTool = action;
          } else if (block.type === 'text' && block.text && block.text.trim()) {
            const text = block.text.trim();
            if (text.length > 5) {
              this._recentActions.push({
                tool: 'said',
                detail: text.slice(0, 100),
              });
              if (this._recentActions.length > 10) {
                this._recentActions = this._recentActions.slice(-10);
              }
            }
          }
        }
      }
    }
  }

  read() {
    // Discover or re-check file
    const filePath = this._discoverFile();
    if (!filePath) return null;

    let stat;
    try {
      stat = fs.statSync(filePath);
    } catch {
      return null;
    }

    const currentSize = stat.size;

    // File changed?
    if (filePath === this._filePath && currentSize === this._fileSize) {
      return null; // no new data
    }

    // New file or different file
    if (filePath !== this._filePath) {
      this._filePath = filePath;
      this._fileSize = 0;
      this._userTask = null;
      this._recentActions = [];
      this._currentTool = null;
      this._bootstrapped = false;
    }

    if (!this._bootstrapped) {
      // Bootstrap: backward scan up to BOOTSTRAP_MAX
      const start = Math.max(0, currentSize - BOOTSTRAP_MAX);
      const buf = readRange(filePath, start, currentSize);
      const entries = parseJsonlLines(buf);
      this._parseEntries(entries);
      this._fileSize = currentSize;
      this._bootstrapped = true;
    } else {
      // Incremental: read only new bytes
      const buf = readRange(filePath, this._fileSize, currentSize);
      const entries = parseJsonlLines(buf);
      this._parseEntries(entries);
      this._fileSize = currentSize;
    }

    return {
      snapshot: {
        userTask: this._userTask,
        recentActions: this._recentActions.slice(-5),
        currentTool: this._currentTool,
      },
      delta: {
        userTask: this._userTask,
        recentActions: this._recentActions.slice(-5),
        currentTool: this._currentTool,
      },
    };
  }
}

// --- Codex Reader ---

class CodexReader {
  constructor(cwd) {
    this.cwd = cwd;
    this._filePath = null;
    this._fileSize = 0;
    this._userTask = null;
    this._recentActions = [];
    this._currentTool = null;
    this._bootstrapped = false;
  }

  /**
   * Codex stores sessions in ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl
   * Walk date dirs in reverse, match CWD from session_meta.
   */
  _discoverFile() {
    const sessionsDir = path.join(process.env.HOME, '.codex', 'sessions');
    try {
      // Walk year dirs in reverse
      const years = fs.readdirSync(sessionsDir)
        .filter((d) => /^\d{4}$/.test(d))
        .sort()
        .reverse();

      for (const year of years) {
        const yearDir = path.join(sessionsDir, year);
        const months = fs.readdirSync(yearDir)
          .filter((d) => /^\d{2}$/.test(d))
          .sort()
          .reverse();

        for (const month of months) {
          const monthDir = path.join(yearDir, month);
          const days = fs.readdirSync(monthDir)
            .filter((d) => /^\d{2}$/.test(d))
            .sort()
            .reverse();

          for (const day of days) {
            const dayDir = path.join(monthDir, day);
            const files = fs.readdirSync(dayDir)
              .filter((f) => f.startsWith('rollout-') && f.endsWith('.jsonl'))
              .sort()
              .reverse();

            for (const f of files) {
              const full = path.join(dayDir, f);
              // Check session_meta for matching CWD
              if (this._matchesCwd(full)) return full;
            }
          }
        }
      }
      return null;
    } catch {
      return null;
    }
  }

  _matchesCwd(filePath) {
    try {
      // Read just the first line (session_meta)
      const fd = fs.openSync(filePath, 'r');
      try {
        const buf = Buffer.alloc(2048);
        const bytesRead = fs.readSync(fd, buf, 0, 2048, 0);
        const text = buf.slice(0, bytesRead).toString('utf-8');
        const firstLine = text.split('\n')[0];
        if (!firstLine) return false;
        const entry = JSON.parse(firstLine);
        if (entry.type === 'session_meta' && entry.payload) {
          return entry.payload.cwd === this.cwd;
        }
        return false;
      } finally {
        fs.closeSync(fd);
      }
    } catch {
      return false;
    }
  }

  _parseEntries(entries) {
    for (const entry of entries) {
      const type = entry.type;
      const payload = entry.payload || {};

      // User message → task
      if (type === 'response_item' && payload.role === 'user' && Array.isArray(payload.content)) {
        for (const block of payload.content) {
          if (block.type === 'input_text' && block.text) {
            // Skip system/developer prompts — they tend to be very long
            const text = block.text.trim();
            if (text.length > 0 && text.length < 1000 && !text.startsWith('<')) {
              this._userTask = text.slice(0, 300);
            }
          }
        }
      }

      // event_msg with user_message → cleaner task source
      if (type === 'event_msg' && payload.type === 'user_message' && payload.message) {
        this._userTask = payload.message.trim().slice(0, 300);
      }

      // Function calls → actions
      if (type === 'response_item' && payload.type === 'function_call') {
        const action = { tool: payload.name };
        try {
          const args = JSON.parse(payload.arguments || '{}');
          if (args.command) action.detail = args.command.slice(0, 80);
          else if (args.file_path) action.detail = path.basename(args.file_path);
        } catch {
          // ignore parse errors
        }
        this._recentActions.push(action);
        if (this._recentActions.length > 10) {
          this._recentActions = this._recentActions.slice(-10);
        }
        this._currentTool = action;
      }

      // Agent reasoning → currentTool
      if (type === 'event_msg' && payload.type === 'agent_reasoning' && payload.text) {
        this._currentTool = { tool: 'thinking', detail: payload.text.slice(0, 100) };
      }

      // Reasoning summary → currentTool
      if (type === 'response_item' && payload.type === 'reasoning' && Array.isArray(payload.summary)) {
        for (const s of payload.summary) {
          if (s.type === 'summary_text' && s.text) {
            this._currentTool = { tool: 'thinking', detail: s.text.slice(0, 100) };
          }
        }
      }
    }
  }

  read() {
    const filePath = this._filePath || this._discoverFile();
    if (!filePath) return null;

    let stat;
    try {
      stat = fs.statSync(filePath);
    } catch {
      return null;
    }

    const currentSize = stat.size;

    if (filePath === this._filePath && currentSize === this._fileSize) {
      return null; // no new data
    }

    if (filePath !== this._filePath) {
      this._filePath = filePath;
      this._fileSize = 0;
      this._userTask = null;
      this._recentActions = [];
      this._currentTool = null;
      this._bootstrapped = false;
    }

    if (!this._bootstrapped) {
      const start = Math.max(0, currentSize - BOOTSTRAP_MAX);
      const buf = readRange(filePath, start, currentSize);
      const entries = parseJsonlLines(buf);
      this._parseEntries(entries);
      this._fileSize = currentSize;
      this._bootstrapped = true;
    } else {
      const buf = readRange(filePath, this._fileSize, currentSize);
      const entries = parseJsonlLines(buf);
      this._parseEntries(entries);
      this._fileSize = currentSize;
    }

    return {
      snapshot: {
        userTask: this._userTask,
        recentActions: this._recentActions.slice(-5),
        currentTool: this._currentTool,
      },
      delta: {
        userTask: this._userTask,
        recentActions: this._recentActions.slice(-5),
        currentTool: this._currentTool,
      },
    };
  }
}

// --- Factory ---

const ContextReader = {
  for(tool, cwd) {
    if (!tool || !cwd) return null;
    if (tool === 'Claude Code') return new ClaudeCodeReader(cwd);
    if (tool === 'Codex') return new CodexReader(cwd);
    return null;
  },
};

module.exports = { ContextReader, ClaudeCodeReader, CodexReader, readRange, parseJsonlLines };
