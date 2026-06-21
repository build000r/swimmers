import {
  markTrogdorBurntSessionsInMap,
  markTrogdorSessionsRespondedState,
  pruneTrogdorBurntSessionMap,
  rawTrogdorSessionAwaitingUser,
  saveTrogdorReadProgress,
  setTrogdorClawgReadIndexForProgress,
  startTrogdorReaderStateForSession,
  trogdorClawgDismissedForMap,
  trogdorClawgKey,
  trogdorClawgReadCompleteForProgress,
  trogdorCueTransitionState,
  trogdorCurrentSurfaceSessionForHover,
  trogdorHoverReaderResetState,
  trogdorReaderProgressAdvanceForSession,
  trogdorReaderStateForWpmChange,
  trogdorReaderWordIndexForProgress,
  trogdorSessionBurntInMap,
  trogdorSessionCanReadForState,
  trogdorSwordsmanVisibleForState,
} from "./trogdor_logic.js";

export function createTrogdorStateHelpers({
  state,
  operatorPressureSnapshot = () => null,
  surfaceSession = (session) => session,
  renderHudSurface = () => {},
  syncTrogdorReaderTimer = () => {},
  performanceRef = globalThis.performance,
  windowRef = globalThis,
  burnMs = 1100,
} = {}) {
  const now = () => (typeof performanceRef?.now === "function" ? performanceRef.now() : 0);

  // Persisting read progress serializes the whole map to localStorage, and the
  // per-word advance runs on the hot HUD render path. Keep the in-memory update
  // synchronous but debounce the write; flush immediately for the infrequent,
  // important transitions (reader start, session responded) and before unload.
  const READ_PROGRESS_SAVE_DEBOUNCE_MS = 750;
  let readProgressSaveTimer = null;
  function cancelPendingReadProgressSave() {
    if (readProgressSaveTimer !== null) {
      windowRef.clearTimeout?.(readProgressSaveTimer);
      readProgressSaveTimer = null;
    }
  }
  function persistTrogdorReadProgress({ immediate = false } = {}) {
    if (immediate || typeof windowRef.setTimeout !== "function") {
      cancelPendingReadProgressSave();
      saveTrogdorReadProgress(state.trogdorReadProgress);
      return;
    }
    if (readProgressSaveTimer !== null) {
      return;
    }
    readProgressSaveTimer = windowRef.setTimeout(() => {
      readProgressSaveTimer = null;
      saveTrogdorReadProgress(state.trogdorReadProgress);
    }, READ_PROGRESS_SAVE_DEBOUNCE_MS);
  }
  windowRef.addEventListener?.("beforeunload", () =>
    persistTrogdorReadProgress({ immediate: true }),
  );

  function rawSessionAwaitingUser(session) {
    return rawTrogdorSessionAwaitingUser(session, operatorPressureSnapshot(session?.session_id));
  }

  function setTrogdorClawgReadIndex(session, index) {
    const next = setTrogdorClawgReadIndexForProgress(
      state.trogdorReadProgress || {},
      session,
      index,
    );
    if (!next.changed) {
      return false;
    }
    state.trogdorReadProgress = next.progress;
    persistTrogdorReadProgress();
    return true;
  }

  function trogdorClawgReadComplete(session) {
    return trogdorClawgReadCompleteForProgress(session, state.trogdorReadProgress);
  }

  function trogdorClawgDismissed(session) {
    return trogdorClawgDismissedForMap(session, state.trogdorDismissedClawgs);
  }

  function trogdorSessionBurnt(sessionOrId) {
    const next = trogdorSessionBurntInMap(
      state.trogdorBurntSessions,
      sessionOrId,
      now(),
    );
    state.trogdorBurntSessions = next.burntSessions;
    return next.burnt;
  }

  function pruneTrogdorBurntSessions() {
    const next = pruneTrogdorBurntSessionMap(state.trogdorBurntSessions, now());
    state.trogdorBurntSessions = next.burntSessions;
    return next.changed;
  }

  function markTrogdorSessionsBurnt(sessionIds, options = {}) {
    const next = markTrogdorBurntSessionsInMap(
      state.trogdorBurntSessions,
      sessionIds,
      now(),
      burnMs,
    );
    if (!next.ids.length) {
      return;
    }
    state.trogdorBurntSessions = next.burntSessions;
    windowRef.setTimeout(() => {
      if (pruneTrogdorBurntSessions()) {
        state.trogdorSurfaceSignature = "";
        renderHudSurface();
      }
    }, burnMs + 40);
    if (options.render !== false) {
      state.trogdorSurfaceSignature = "";
      renderHudSurface();
    }
  }

  function currentTrogdorSurfaceSession() {
    return trogdorCurrentSurfaceSessionForHover({
      sessions: state.sessions,
      hoveredSessionId: state.hoveredTrogdorSessionId,
      toSurfaceSession: surfaceSession,
    });
  }

  function trogdorSwordsmanVisible(session) {
    const burnt = typeof session?.trogdorBurnt === "boolean" ? session.trogdorBurnt : trogdorSessionBurnt(session);
    const dismissed = typeof session?.trogdorDismissed === "boolean" ? session.trogdorDismissed : trogdorClawgDismissed(session);
    return trogdorSwordsmanVisibleForState(session, { burnt, dismissed });
  }

  function trogdorSessionCanRead(session) {
    const burnt = typeof session?.trogdorBurnt === "boolean" ? session.trogdorBurnt : trogdorSessionBurnt(session);
    const dismissed = typeof session?.trogdorDismissed === "boolean" ? session.trogdorDismissed : trogdorClawgDismissed(session);
    return trogdorSessionCanReadForState(session, { burnt, dismissed });
  }

  function trogdorReaderWordIndex(session, wpm) {
    return trogdorReaderWordIndexForProgress(session, {
      wpm,
      readerClawgKey: state.trogdorReaderClawgKey,
      readerStartIndex: state.trogdorReaderStartIndex,
      progress: state.trogdorReadProgress,
      reading: state.trogdorReading,
      hoveredSessionId: state.hoveredTrogdorSessionId,
      readerStartedAt: state.trogdorReaderStartedAt,
      now: now(),
    });
  }

  function advanceTrogdorReaderProgressForCurrentHover() {
    const session = currentTrogdorSurfaceSession();
    if (!session || !trogdorSessionCanRead(session)) {
      return;
    }
    if (trogdorClawgKey(session) !== state.trogdorReaderClawgKey) {
      startTrogdorReaderForSession(session);
    }
    if (state.trogdorReading === false) {
      return;
    }
    const next = trogdorReaderProgressAdvanceForSession(session, {
      wordIndex: trogdorReaderWordIndex(session, state.trogdorWpm),
      reading: state.trogdorReading,
    });
    if (!next.shouldAdvance) {
      return;
    }
    setTrogdorClawgReadIndex(session, next.nextReadIndex);
    state.trogdorReading = next.reading;
  }

  function resetTrogdorReaderAfterWpmChange() {
    Object.assign(state, trogdorReaderStateForWpmChange(currentTrogdorSurfaceSession(), {
      currentStartIndex: state.trogdorReaderStartIndex,
      progress: state.trogdorReadProgress,
      now: now(),
    }));
  }

  function startTrogdorReaderForSession(session, options = {}) {
    const next = startTrogdorReaderStateForSession(session, {
      readAgain: Boolean(options.readAgain),
      dismissedClawgs: state.trogdorDismissedClawgs || {},
      progress: state.trogdorReadProgress || {},
      now: now(),
    });
    state.trogdorDismissedClawgs = next.dismissedClawgs;
    if (next.progressChanged) {
      state.trogdorReadProgress = next.progress;
      persistTrogdorReadProgress({ immediate: true });
    }
    state.trogdorReaderClawgKey = next.readerClawgKey;
    state.trogdorReaderStartIndex = next.readerStartIndex;
    state.trogdorReaderStartedAt = next.readerStartedAt;
    state.trogdorReading = next.reading;
  }

  function markTrogdorSessionsResponded(sessionIds) {
    const next = markTrogdorSessionsRespondedState({
      sessionIds,
      sessions: state.sessions,
      toSurfaceSession: surfaceSession,
      dismissedClawgs: state.trogdorDismissedClawgs || {},
      progress: state.trogdorReadProgress || {},
      hoveredSessionId: state.hoveredTrogdorSessionId,
    });
    state.trogdorDismissedClawgs = next.dismissedClawgs;
    if (next.progressChanged) {
      state.trogdorReadProgress = next.progress;
      persistTrogdorReadProgress({ immediate: true });
    }
    if (next.burntIds.length) {
      if (next.resetReader) {
        Object.assign(state, trogdorHoverReaderResetState());
        syncTrogdorReaderTimer();
      }
      markTrogdorSessionsBurnt(next.burntIds);
    }
  }

  function syncTrogdorCueTransitions() {
    const next = trogdorCueTransitionState({
      sessions: state.sessions,
      previousAwaitingSessionIds: state.trogdorAwaitingSessionIds,
      hoveredSessionId: state.hoveredTrogdorSessionId,
      rawAwaitingUser: rawSessionAwaitingUser,
      sessionBurnt: trogdorSessionBurnt,
    });
    state.trogdorAwaitingSessionIds = next.awaitingSessionIds;
    if (next.burntIds.length) {
      markTrogdorSessionsBurnt(next.burntIds, { render: false });
    }

    if (next.resetReader) {
      Object.assign(state, trogdorHoverReaderResetState());
      syncTrogdorReaderTimer();
    }
  }

  return {
    advanceTrogdorReaderProgressForCurrentHover,
    currentTrogdorSurfaceSession,
    flushTrogdorReadProgress: () => persistTrogdorReadProgress({ immediate: true }),
    markTrogdorSessionsResponded,
    rawSessionAwaitingUser,
    resetTrogdorReaderAfterWpmChange,
    startTrogdorReaderForSession,
    syncTrogdorCueTransitions,
    trogdorClawgReadComplete,
    trogdorSessionBurnt,
    trogdorSessionCanRead,
    trogdorReaderWordIndex,
    trogdorSwordsmanVisible,
  };
}
