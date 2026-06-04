function defaultNormalizeSessionId(sessionId) {
  return sessionId || null;
}

export function sessionUrlForSelection(context = {}) {
  const {
    href,
    pathname = "",
    followPublishedSelection = false,
    selectedSessionId = null,
    URLImpl = URL,
  } = context;
  const url = new URLImpl(href);
  const publishedRoute = pathname === "/selected";
  url.searchParams.delete("token");
  if (followPublishedSelection) {
    if (publishedRoute) {
      url.searchParams.delete("follow");
    } else {
      url.searchParams.set("follow", "published");
    }
    url.searchParams.delete("session");
  } else if (selectedSessionId) {
    url.searchParams.delete("follow");
    url.searchParams.set("session", selectedSessionId);
  } else {
    url.searchParams.delete("follow");
    url.searchParams.delete("session");
  }
  return url;
}

export function createSessionPersistenceController(deps = {}) {
  const {
    state,
    windowRef,
    documentRef,
    storage,
    sessionStorageKey,
    normalizeSessionId = defaultNormalizeSessionId,
    resetAgentContextForSession = () => {},
    resetWorkbenchWidgetsForSession = () => {},
    closeTrogdorAtlasForTerminal = () => {},
    renderHudSurface = () => {},
    URLImpl = URL,
  } = deps;

  function syncUrlState() {
    const url = sessionUrlForSelection({
      href: windowRef.location.href,
      pathname: windowRef.location.pathname,
      followPublishedSelection: state.followPublishedSelection,
      selectedSessionId: state.selectedSessionId,
      URLImpl,
    });
    windowRef.history.replaceState({}, "", url);
  }

  function persistSelectedSession(sessionId, options = {}) {
    const normalized = normalizeSessionId(sessionId);
    const previous = state.selectedSessionId;
    state.selectedSessionId = normalized;
    if (previous !== normalized) {
      resetAgentContextForSession(normalized);
      resetWorkbenchWidgetsForSession(normalized);
    }
    if (normalized) {
      storage.setItem(sessionStorageKey, normalized);
      closeTrogdorAtlasForTerminal();
    } else {
      storage.removeItem(sessionStorageKey);
    }

    if (options.syncUrl ?? true) {
      syncUrlState();
    }
  }

  function setFollowPublishedSelection(enabled, options = {}) {
    state.followPublishedSelection = Boolean(enabled);
    documentRef.body.classList.toggle("following-published", state.followPublishedSelection);
    if (!options.skipUrlSync) {
      syncUrlState();
    }
    renderHudSurface();
  }

  return {
    normalizeSessionId,
    persistSelectedSession,
    setFollowPublishedSelection,
    syncUrlState,
  };
}
