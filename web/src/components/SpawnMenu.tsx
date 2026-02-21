import { useState, useEffect, useCallback, useRef } from "preact/hooks";
import type { DirEntry, SpawnTool } from "@/types";

interface SpawnMenuProps {
  x: number;
  y: number;
  onSelect: (path: string, spawnTool?: SpawnTool) => void;
  onClose: () => void;
}

type SpawnMode = SpawnTool | "none";

const MENU_WIDTH = 280;
const MENU_MAX_HEIGHT = 560;
const EDGE_PADDING = 8;
const SPAWN_TOOL_STORAGE_KEY = "spawn_tool_preference_v1";

export function SpawnMenu({ x, y, onSelect, onClose }: SpawnMenuProps) {
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [currentPath, setCurrentPath] = useState<string>("");
  const [basePath, setBasePath] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [spawnMode, setSpawnMode] = useState<SpawnMode>(() => {
    try {
      const stored = window.localStorage.getItem(SPAWN_TOOL_STORAGE_KEY);
      if (stored === "claude" || stored === "codex" || stored === "none") {
        return stored;
      }
    } catch {
      // Ignore storage failures.
    }
    return "codex";
  });
  const menuRef = useRef<HTMLDivElement>(null);

  const fetchDirs = useCallback(async (path?: string) => {
    setLoading(true);
    setError(null);
    try {
      const { listDirs } = await import("@/services/api");
      const resp = await listDirs(path);
      setEntries(resp.entries);
      setCurrentPath(resp.path);
      // First fetch (no path arg) returns the base — remember it.
      if (!path) setBasePath(resp.path);
    } catch (err) {
      setError("failed to list");
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchDirs();
  }, [fetchDirs]);

  useEffect(() => {
    try {
      window.localStorage.setItem(SPAWN_TOOL_STORAGE_KEY, spawnMode);
    } catch {
      // Ignore storage failures.
    }
  }, [spawnMode]);

  // Close on outside click or Escape.
  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    // Delay listener so the opening click doesn't immediately close.
    const timer = setTimeout(() => {
      document.addEventListener("mousedown", handleClick);
      document.addEventListener("touchstart", handleClick as any);
      document.addEventListener("keydown", handleKey);
    }, 50);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("touchstart", handleClick as any);
      document.removeEventListener("keydown", handleKey);
    };
  }, [onClose]);

  // Compute position: flip if near edges.
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const menuW = Math.min(MENU_WIDTH, vw - EDGE_PADDING * 2);
  const menuH = Math.min(MENU_MAX_HEIGHT, vh - EDGE_PADDING * 2);

  let left = x;
  let top = y;

  // Horizontal: prefer right of click, flip left if overflows.
  if (left + menuW > vw - EDGE_PADDING) {
    left = Math.max(EDGE_PADDING, left - menuW);
  }
  // Vertical: prefer below click, flip up if overflows.
  if (top + menuH > vh - EDGE_PADDING) {
    top = Math.max(EDGE_PADDING, top - menuH);
  }

  // On narrow screens, center horizontally.
  if (vw < 400) {
    left = (vw - menuW) / 2;
  }

  // Final clamp so the menu always stays on-screen.
  left = Math.max(EDGE_PADDING, Math.min(left, vw - menuW - EDGE_PADDING));
  top = Math.max(EDGE_PADDING, Math.min(top, vh - menuH - EDGE_PADDING));

  // Show only the portion after the base path (e.g. "throngterm/src").
  // At the root level, show "/".
  const relative = basePath && currentPath.startsWith(basePath)
    ? currentPath.slice(basePath.replace(/\/$/, "").length) || "/"
    : currentPath;
  const pathLabel = relative;
  const parentPath = currentPath.replace(/\/[^/]+\/?$/, "") || "/";
  const atRoot = !basePath || currentPath === basePath.replace(/\/$/, "");

  return (
    <div
      ref={menuRef}
      class="spawn-menu"
      style={{
        left: left + "px",
        top: top + "px",
        width: menuW + "px",
        maxHeight: menuH + "px",
      }}
    >
      {/* Header with path breadcrumb */}
      <div class="spawn-menu-header">
        {!atRoot && (
          <button
            class="spawn-menu-back"
            onClick={() => fetchDirs(parentPath)}
            aria-label="Go up"
          >
            ..
          </button>
        )}
        <span class="spawn-menu-path">{pathLabel}</span>
        <button
          class="spawn-menu-copy"
          onClick={(e: MouseEvent) => {
            e.stopPropagation();
            navigator.clipboard.writeText(`cd ${currentPath} && tmux`);
            const btn = e.currentTarget as HTMLButtonElement;
            btn.textContent = "ok";
            setTimeout(() => { btn.textContent = "cp"; }, 600);
          }}
          aria-label="Copy cd + tmux command"
        >
          cp
        </button>
        <button
          class="spawn-menu-close"
          onClick={onClose}
          aria-label="Close"
        >
          x
        </button>
      </div>

      <div class="spawn-menu-tools">
        <button
          class={`spawn-menu-tool ${spawnMode === "none" ? "active" : ""}`}
          onClick={() => setSpawnMode("none")}
          type="button"
        >
          Spawn only
        </button>
        <button
          class={`spawn-menu-tool ${spawnMode === "claude" ? "active" : ""}`}
          onClick={() => setSpawnMode("claude")}
          type="button"
        >
          Claude
        </button>
        <button
          class={`spawn-menu-tool ${spawnMode === "codex" ? "active" : ""}`}
          onClick={() => setSpawnMode("codex")}
          type="button"
        >
          Codex
        </button>
      </div>

      {/* Spawn here button */}
      <button
        class="spawn-menu-item spawn-here"
        onClick={() =>
          onSelect(currentPath, spawnMode === "none" ? undefined : spawnMode)
        }
      >
        <span class="spawn-menu-icon">+</span>
        <span>
          {spawnMode === "none"
            ? "spawn here"
            : `spawn here + run ${spawnMode}`}
        </span>
      </button>

      {/* Directory list */}
      <div class="spawn-menu-list">
        {loading && (
          <div class="spawn-menu-loading">...</div>
        )}
        {error && (
          <div class="spawn-menu-error">{error}</div>
        )}
        {!loading &&
          !error &&
          entries.map((entry) => (
            <button
              key={entry.name}
              class="spawn-menu-item"
              onClick={() => {
                const childPath = currentPath.replace(/\/$/, "") + "/" + entry.name;
                if (entry.has_children) {
                  fetchDirs(childPath);
                } else {
                  onSelect(
                    childPath,
                    spawnMode === "none" ? undefined : spawnMode,
                  );
                }
              }}
            >
              <span class="spawn-menu-icon">{entry.has_children ? ">" : " "}</span>
              <span class="spawn-menu-name">{entry.name}</span>
            </button>
          ))}
        {!loading && !error && entries.length === 0 && (
          <div class="spawn-menu-empty">empty</div>
        )}
      </div>
    </div>
  );
}
