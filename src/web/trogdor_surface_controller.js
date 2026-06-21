import {
  buildTrogdorDomGroups,
  summarizeTrogdorDom,
  trogdorClawgWords,
  trogdorDragonPose,
  trogdorHoverReaderResetState,
  trogdorHoverSessionIdForZone,
  trogdorRawSessionForHover,
  trogdorReadableHoveredSurfaceSession,
  trogdorReaderDisplayState,
  trogdorReaderTimerAction,
} from "./trogdor_logic.js";
import {
  TROGDOR_REPO_POSITIONS,
  renderTrogdorSurfaceFrame,
  trogdorReadButtonLabel,
  trogdorSurfaceSignature,
} from "./trogdor_render.js";
import {
  buildFleetLensSummary,
  resolveSurfaceSessions,
} from "./surface_model.js";

function defaultClampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

function escapeHtml(text) {
  return String(text || "").replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      case "'":
        return "&#39;";
      default:
        return char;
    }
  });
}

export function createTrogdorSurfaceController(runtime = {}) {
  const {
    state,
    el,
    documentRef = globalThis.document,
    windowRef = globalThis,
    surfaceSession = (session) => session,
    currentSession = () => null,
    currentTrogdorSurfaceSession = () => null,
    trogdorSessionCanRead = () => false,
    trogdorClawgReadComplete = () => false,
    trogdorReaderWordIndex = () => -1,
    startTrogdorReaderForSession = () => {},
    renderHudSurface = () => {},
    setUtilityStatus = () => {},
    clampInt = defaultClampInt,
  } = runtime;

  function renderTrogdorSurface() {
    if (!el.trogdorSurface) {
      return;
    }

    const visible = Boolean(state.trogdorAtlasOpen);
    applyTrogdorAtlasVisibility();
    if (!visible) {
      return;
    }

    const allSessions = state.sessions.map((session) => surfaceSession(session));
    // Share the HUD's fleet-preset/fleet-filter resolution so the atlas never
    // shows a different session set than the rest of the cockpit.
    const { surfaceSessions: sessions } = resolveSurfaceSessions(
      state,
      currentSession(),
      allSessions,
      buildFleetLensSummary(allSessions),
    );
    const groups = buildTrogdorDomGroups(sessions);
    const hovered = trogdorReadableHoveredSurfaceSession(sessions, state.hoveredTrogdorSessionId, {
      sessionCanRead: trogdorSessionCanRead,
    });
    const summary = summarizeTrogdorDom(groups, sessions);
    const dragonPose = trogdorDragonPose(groups, summary, TROGDOR_REPO_POSITIONS);
    const signature = trogdorSurfaceSignature(sessions, summary, state.readOnly);
    if (signature !== state.trogdorSurfaceSignature) {
      state.trogdorSurfaceSignature = signature;
      const wpm = clampInt(state.trogdorWpm, 200, 50, 800);
      el.trogdorSurface.innerHTML = renderTrogdorSurfaceFrame({
        groups,
        sessions,
        summary,
        dragonPose,
        readerMarkup: renderTrogdorReader(hovered),
        readButtonLabel: trogdorReadButtonLabel(state.trogdorReading, Boolean(hovered && trogdorClawgReadComplete(hovered))),
        wpm,
        readOnly: state.readOnly,
        hoveredSessionId: state.hoveredTrogdorSessionId,
      });
    }
    renderTrogdorReader(hovered);
  }

  function renderTrogdorReader(hoveredSession) {
    const wpm = clampInt(state.trogdorWpm, 200, 50, 800);
    const hovered = hoveredSession || null;
    const readerState = trogdorReaderDisplayState(hovered, {
      wordIndex: hovered ? trogdorReaderWordIndex(hovered, wpm) : -1,
      progress: state.trogdorReadProgress,
    });
    const bannerText = readerState.bannerText;
    const readerMarkup = `<div class="trogdor-banner" data-trogdor-reader="true">${escapeHtml(bannerText)}</div>`;
    if (!el.trogdorSurface) {
      return readerMarkup;
    }
    const banner = el.trogdorSurface.querySelector("[data-trogdor-reader]");
    if (banner) {
      banner.textContent = bannerText;
    }
    const readToggle = el.trogdorSurface.querySelector('button[data-action="trogdor_read_toggle"]');
    if (readToggle) {
      readToggle.textContent = trogdorReadButtonLabel(state.trogdorReading, readerState.readComplete);
    }
    const wpmValue = el.trogdorSurface.querySelector("[data-trogdor-wpm-value]");
    if (wpmValue) {
      wpmValue.textContent = `${wpm} wpm`;
    }
    return readerMarkup;
  }

  function applyTrogdorAtlasVisibility() {
    const visible = Boolean(state.trogdorAtlasOpen);
    if (el.trogdorSurface) {
      el.trogdorSurface.classList.toggle("hidden", !visible);
      el.trogdorSurface.setAttribute("aria-hidden", visible ? "false" : "true");
      el.trogdorSurface.style.display = visible ? "" : "none";
    }
    el.trogdorLauncher?.classList.toggle("hidden", visible || Boolean(state.activeSheet));
    documentRef.body.classList.toggle("trogdor-mode", visible);
  }

  function updateHoveredTrogdorSurface(zone) {
    const previousSessionId = state.hoveredTrogdorSessionId;
    const nextSessionId = trogdorHoverSessionIdForZone(zone, previousSessionId);
    if (nextSessionId === previousSessionId) {
      return;
    }
    Object.assign(state, trogdorHoverReaderResetState(nextSessionId));
    if (el.trogdorSurface) {
      const agents = el.trogdorSurface.querySelectorAll("[data-trogdor-agent]");
      for (const agent of agents) {
        agent.classList.toggle("is-hovered", Boolean(nextSessionId) && agent.dataset.sessionId === nextSessionId);
      }
    }
    if (nextSessionId) {
      const session = trogdorRawSessionForHover(state.sessions, nextSessionId, { normalize: false });
      if (session) {
        const surfaced = surfaceSession(session);
        startTrogdorReaderForSession(surfaced);
        // Announce the agent's full thought once to assistive tech. The per-word
        // banner updates far too fast to be a live region without flooding, so a
        // separate polite region carries the whole text on hover/focus start.
        if (el.trogdorReaderAnnounce) {
          el.trogdorReaderAnnounce.textContent = trogdorClawgWords(surfaced).join(" ");
        }
      }
      setUtilityStatus(
        session
          ? `Speed reading ${session.tmux_name || session.session_id} at ${state.trogdorWpm} wpm.`
          : `Speed reading agent at ${state.trogdorWpm} wpm.`,
        false,
        1200,
      );
    } else if (el.trogdorReaderAnnounce) {
      el.trogdorReaderAnnounce.textContent = "";
    }
    renderHudSurface();
    syncTrogdorReaderTimer();
  }

  function syncTrogdorReaderTimer() {
    const timerAction = trogdorReaderTimerAction(
      currentTrogdorSurfaceSession(), trogdorSessionCanRead, trogdorClawgReadComplete,
      state.trogdorReading, state.trogdorReaderTimer,
    );
    if (timerAction === "start") {
      state.trogdorReaderTimer = windowRef.setInterval(() => renderHudSurface(), 120);
      return;
    }
    if (timerAction === "stop") {
      windowRef.clearInterval(state.trogdorReaderTimer);
      state.trogdorReaderTimer = null;
    }
  }

  return {
    applyTrogdorAtlasVisibility,
    renderTrogdorReader,
    renderTrogdorSurface,
    syncTrogdorReaderTimer,
    updateHoveredTrogdorSurface,
  };
}
