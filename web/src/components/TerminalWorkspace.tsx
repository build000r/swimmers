import { useEffect, useRef, useCallback, useState } from "preact/hooks";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import type { SessionSummary, SessionState } from "@/types";
import { realtime } from "@/app";
import type { TerminalOutputFrame } from "@/services/realtime";
import type { CachedTerminal } from "@/hooks/useTerminalCache";

// ---- Helpers ----

function spriteForState(state: SessionState): string {
  const map: Record<string, string> = {
    idle: "/assets/idle.png",
    busy: "/assets/walking.png",
    error: "/assets/beep.png",
    attention: "/assets/idle.png",
    exited: "/assets/sad.png",
  };
  return map[state] ?? map.idle;
}

function repoName(cwd: string): string {
  if (!cwd || cwd === "/") return "root";
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || "root";
}

// ---- Component ----

interface TerminalWorkspaceProps {
  session: SessionSummary;
  /** If non-null, restore this cached terminal instead of creating a new one */
  cached: CachedTerminal | null;
  /** Observer mode disables input */
  observer?: boolean;
  /** Called when the workspace wants to cache its terminal (e.g., before unmount) */
  onCache: (cached: CachedTerminal) => void;
  /** Called when session exits */
  onSessionExit: (sessionId: string) => void;
  /** Called when header sprite is clicked (close zone) */
  onClose: () => void;
  /** Recovery info */
  recoveryBanner?: string | null;
}

const encoder = new TextEncoder();

export function TerminalWorkspace({
  session,
  cached,
  observer = false,
  onCache,
  onSessionExit,
  onClose,
  recoveryBanner,
}: TerminalWorkspaceProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const hostElRef = useRef<HTMLDivElement | null>(null);
  const resizeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seqRef = useRef<number>(0);
  const [title, setTitle] = useState(`tmux a -t ${session.tmux_name}`);
  const [titleCopied, setTitleCopied] = useState(false);
  const initDoneRef = useRef(false);

  // ---- Terminal init / restore ----

  useEffect(() => {
    if (initDoneRef.current) return;
    initDoneRef.current = true;

    const container = containerRef.current;
    if (!container) return;

    let term: Terminal;
    let fitAddon: FitAddon;
    let hostEl: HTMLDivElement;

    if (cached) {
      // Restore from cache
      term = cached.term;
      fitAddon = cached.fitAddon;
      hostEl = cached.hostEl;
      container.appendChild(hostEl);
      fitAddon.fit();
      term.focus();
    } else {
      // Create new terminal
      hostEl = document.createElement("div");
      hostEl.className = "term-host";
      hostEl.style.width = "100%";
      hostEl.style.height = "100%";
      container.appendChild(hostEl);

      term = new Terminal({
        theme: {
          background: "#1a1a2e",
          foreground: "#e0e0e0",
          cursor: "#e0e0e0",
          cursorAccent: "#1a1a2e",
          selectionBackground: "rgba(255,255,255,0.2)",
        },
        fontFamily: 'Menlo, Monaco, "Courier New", monospace',
        fontSize: 14,
        scrollback: 5000,
        cursorBlink: true,
      });

      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.open(hostEl);

      // Try loading WebGL addon for performance
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        term.loadAddon(webgl);
      } catch {
        // WebGL not available, software renderer is fine
      }

      // Disable iOS autocorrect on the hidden textarea
      const textarea = hostEl.querySelector("textarea");
      if (textarea) {
        textarea.setAttribute("autocapitalize", "off");
        textarea.setAttribute("autocorrect", "off");
        textarea.setAttribute("autocomplete", "off");
        textarea.setAttribute("spellcheck", "false");
      }

      fitAddon.fit();

      // Subscribe to session via realtime
      realtime.subscribeSession(session.session_id);

      // Send initial resize
      realtime.sendResize(session.session_id, term.cols, term.rows);

      // Focus after layout settles
      setTimeout(() => {
        fitAddon.fit();
        term.focus();
      }, 350);
    }

    termRef.current = term;
    fitAddonRef.current = fitAddon;
    hostElRef.current = hostEl;

    // ---- Wire terminal output from realtime ----

    const handleOutput = (frame: TerminalOutputFrame) => {
      if (frame.sessionId !== session.session_id) return;
      seqRef.current = frame.seq;
      term.write(frame.data);
    };

    realtime.on({ onTerminalOutput: handleOutput });

    // ---- Wire terminal input (unless observer) ----

    let inputDisposable: { dispose: () => void } | null = null;
    if (!observer) {
      inputDisposable = term.onData((data: string) => {
        const bytes = encoder.encode(data);
        realtime.sendInput(session.session_id, bytes);
      });
    }

    // ---- Resize with debounce ----

    let resizeDisposable: { dispose: () => void } | null = null;
    resizeDisposable = term.onResize(({ cols, rows }) => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        realtime.sendResize(session.session_id, cols, rows);
      }, 100);
    });

    // Window/viewport resize -> refit
    const handleWindowResize = () => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        if (fitAddonRef.current) fitAddonRef.current.fit();
      }, 100);
    };
    window.addEventListener("resize", handleWindowResize);
    if (window.visualViewport) {
      window.visualViewport.addEventListener("resize", handleWindowResize);
    }

    // Cleanup on unmount: cache the terminal instead of destroying it
    return () => {
      window.removeEventListener("resize", handleWindowResize);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener("resize", handleWindowResize);
      }
      inputDisposable?.dispose();
      resizeDisposable?.dispose();
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);

      // Detach from DOM but keep alive for cache
      if (hostEl.parentNode) hostEl.parentNode.removeChild(hostEl);
      onCache({ term, fitAddon, hostEl, sessionId: session.session_id });
    };
  }, [session.session_id]); // Only re-run if the session changes

  // ---- Refit when the workspace container resizes (e.g., zone split change) ----

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !fitAddonRef.current) return;

    const observer = new ResizeObserver(() => {
      if (resizeTimerRef.current) clearTimeout(resizeTimerRef.current);
      resizeTimerRef.current = setTimeout(() => {
        if (fitAddonRef.current) fitAddonRef.current.fit();
      }, 100);
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  // ---- Title copy handler ----

  const handleTitleClick = useCallback(() => {
    navigator.clipboard.writeText(title).then(() => {
      setTitleCopied(true);
      setTimeout(() => setTitleCopied(false), 800);
    }).catch(() => {});
  }, [title]);

  // ---- Render ----

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        height: "100%",
        overflow: "hidden",
      }}
    >
      {/* Zone header */}
      <header class="zone-header">
        <img
          class="zone-sprite"
          src={spriteForState(session.state)}
          alt="Close"
          onClick={onClose}
        />
        <span class="zone-name">{repoName(session.cwd)}</span>
        <span class="zone-title" onClick={handleTitleClick}>
          {titleCopied ? "copied!" : title}
        </span>
        <span class={`zone-dot state-dot ${session.state}`} />
      </header>

      {/* Recovery banner */}
      {recoveryBanner && (
        <div
          style={{
            background: "#E74C3C",
            color: "#fff",
            textAlign: "center",
            padding: "4px 8px",
            fontSize: "12px",
            fontWeight: 600,
            flexShrink: 0,
          }}
        >
          {recoveryBanner}
        </div>
      )}

      {/* Observer badge */}
      {observer && (
        <div
          style={{
            background: "#16213e",
            color: "#5BC0EB",
            textAlign: "center",
            padding: "2px 0",
            fontSize: "10px",
            fontWeight: 600,
            flexShrink: 0,
          }}
        >
          OBSERVER (read-only)
        </div>
      )}

      {/* Terminal container */}
      <div
        ref={containerRef}
        class="zone-terminal"
        style={{ flex: 1, minHeight: 0 }}
      />
    </div>
  );
}
