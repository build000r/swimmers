export function createTerminalSearchLinksController(runtime) {
  const {
    state,
    el,
    terminalSupports,
    hasLiveTerminal,
    scheduleRender,
    renderHudSurface,
    setSearchStatus,
    setUtilityStatus,
    defaultUtilityLabel,
    shortenUrl,
    currentSession,
    frankenTermLinkPolicy,
    surfaceBusy,
    withSurfaceOperation,
    mouseCell,
    syncTerminalTools,
    navigatorRef = globalThis.navigator,
    windowRef = globalThis.window,
    URLImpl = globalThis.URL,
  } = runtime;

  function setTerminalSelectionRange(start, end) {
    if (!terminalSupports("setSelectionRange")) {
      return;
    }
    const normalizedStart = Math.min(start, end);
    const normalizedEnd = Math.max(start, end) + 1;
    state.selectionFocus = end;
    state.terminal.setSelectionRange(normalizedStart, normalizedEnd);
    scheduleRender();
  }

  function clearTerminalSelection() {
    state.selectionAnchor = null;
    state.selectionFocus = null;
    if (terminalSupports("clearSelection")) {
      state.terminal.clearSelection();
      scheduleRender();
    }
  }

  function setSelectMode(enabled) {
    state.selectMode = Boolean(enabled);
    if (!state.selectMode) {
      clearTerminalSelection();
    }
    syncTerminalTools();
  }

  function updateSearchUi(searchState) {
    state.searchState = searchState ?? null;
    if (!state.searchQuery) {
      setSearchStatus("Search idle", true);
      return;
    }

    const matchCount = Number(state.searchState?.matchCount ?? 0);
    if (matchCount > 0) {
      const activeIndex = Number(state.searchState?.activeMatchIndex ?? 0) + 1;
      setSearchStatus(`${activeIndex}/${matchCount} matches`, false);
    } else {
      setSearchStatus("No matches", true);
    }
  }

  function refreshTerminalSearch() {
    if (!state.searchQuery || !terminalSupports("setSearchQuery")) {
      updateSearchUi(null);
      return;
    }
    updateSearchUi(state.terminal.setSearchQuery(state.searchQuery, null));
    scheduleRender();
  }

  function applySearchQuery(rawQuery) {
    state.searchQuery = typeof rawQuery === "string" ? rawQuery : "";
    if (!state.searchQuery) {
      if (terminalSupports("clearSearch")) {
        state.terminal.clearSearch();
        scheduleRender();
      }
      updateSearchUi(null);
      renderHudSurface();
      return;
    }

    if (!terminalSupports("setSearchQuery")) {
      setSearchStatus("Search unavailable in this FrankenTerm build", true);
      return;
    }

    refreshTerminalSearch();
    renderHudSurface();
  }

  function cycleSearchMatch(direction) {
    if (!state.searchQuery || !hasLiveTerminal()) {
      return;
    }

    if (direction < 0 && terminalSupports("searchPrev")) {
      updateSearchUi(state.terminal.searchPrev());
      scheduleRender();
      return;
    }

    if (direction > 0 && terminalSupports("searchNext")) {
      updateSearchUi(state.terminal.searchNext());
      scheduleRender();
    }
  }

  async function copyTerminalSelection() {
    if (!hasLiveTerminal()) {
      return;
    }

    const text =
      (terminalSupports("copySelection") && state.terminal.copySelection()) ||
      (terminalSupports("extractSelectionText") && state.terminal.extractSelectionText()) ||
      "";

    if (!text) {
      setUtilityStatus("No terminal selection to copy.", true, 2200);
      return;
    }

    if (!navigatorRef.clipboard?.writeText) {
      setUtilityStatus("Clipboard write is unavailable in this browser context.", true, 3000);
      return;
    }

    try {
      await navigatorRef.clipboard.writeText(text);
      setUtilityStatus(`Copied ${text.length} characters from the terminal.`, false, 2200);
    } catch (error) {
      setUtilityStatus(`Clipboard write failed: ${error.message}`, true, 3000);
    }
  }

  function safeOpenUrl(rawUrl) {
    try {
      const url = new URLImpl(rawUrl);
      if (url.protocol !== "http:" && url.protocol !== "https:") {
        setUtilityStatus(`Blocked unsupported link protocol: ${url.protocol}`, true, 2600);
        return;
      }
      if (url.protocol === "http:" && !frankenTermLinkPolicy().allowHttp) {
        setUtilityStatus(`Blocked non-local HTTP link: ${shortenUrl(url.toString())}`, true, 2600);
        return;
      }
      windowRef.open(url.toString(), "_blank", "noopener,noreferrer");
    } catch (error) {
      setUtilityStatus(`Invalid link: ${error.message}`, true, 2600);
    }
  }

  function syncLinkTools() {
    if (!el.terminalLinkTools) {
      return;
    }
    const visible = Boolean(state.hoveredLinkUrl && currentSession() && !state.activeSheet && !state.selectMode);
    el.terminalLinkTools.classList.toggle("hidden", !visible);
    if (el.terminalLinkText) {
      el.terminalLinkText.textContent = visible ? shortenUrl(state.hoveredLinkUrl) : "";
    }
  }

  async function copyHoveredLink() {
    if (!state.hoveredLinkUrl) {
      setUtilityStatus("No terminal link is currently hovered.", true, 2200);
      return false;
    }
    if (!navigatorRef.clipboard?.writeText) {
      setUtilityStatus("Clipboard write is unavailable in this browser context.", true, 3000);
      return false;
    }
    try {
      await navigatorRef.clipboard.writeText(state.hoveredLinkUrl);
      setUtilityStatus(`Copied ${shortenUrl(state.hoveredLinkUrl)}.`, false, 2200);
      return true;
    } catch (error) {
      setUtilityStatus(`Clipboard write failed: ${error.message}`, true, 3000);
      return false;
    }
  }

  function drainTerminalLinkClicks() {
    if (!terminalSupports("drainLinkClicks")) {
      return;
    }
    const clicks = state.terminal.drainLinkClicks();
    if (!Array.isArray(clicks) || !clicks.length) {
      return;
    }
    for (const click of clicks) {
      const url = click?.url || click?.href || "";
      if (!url) {
        continue;
      }
      if (click.openAllowed === false) {
        setUtilityStatus(click.openReason || `Blocked ${shortenUrl(url)}.`, true, 2600);
        continue;
      }
      safeOpenUrl(url);
    }
  }

  async function copyTerminalFrameText() {
    const text =
      state.terminalMirrorText ||
      (terminalSupports("screenReaderMirrorText") && state.terminal.screenReaderMirrorText()) ||
      (terminalSupports("accessibilityDomSnapshot") && state.terminal.accessibilityDomSnapshot()?.value) ||
      el.terminalFallback.textContent ||
      "";
    if (!text.trim()) {
      setUtilityStatus("No terminal text is available to copy.", true, 2400);
      return false;
    }
    if (!navigatorRef.clipboard?.writeText) {
      setUtilityStatus("Clipboard write is unavailable in this browser context.", true, 3000);
      return false;
    }
    try {
      await navigatorRef.clipboard.writeText(text);
      setUtilityStatus(`Copied ${text.length} visible terminal characters.`, false, 2200);
      return true;
    } catch (error) {
      setUtilityStatus(`Clipboard write failed: ${error.message}`, true, 3000);
      return false;
    }
  }

  function clearHoveredLink(updateUi = true) {
    state.hoveredLinkUrl = "";
    if (terminalSupports("setHoveredLinkId") && !surfaceBusy()) {
      const cleared = withSurfaceOperation("setHoveredLinkId", () => state.terminal.setHoveredLinkId(0));
      if (!cleared.deferred) {
        scheduleRender();
      }
    }
    if (updateUi) {
      setUtilityStatus(defaultUtilityLabel(), true);
      syncTerminalTools();
    }
    syncLinkTools();
  }

  function updateHoveredLink(event) {
    if (!hasLiveTerminal() || !terminalSupports("linkUrlAt") || state.selectMode) {
      clearHoveredLink(true);
      return;
    }

    if (surfaceBusy()) {
      return;
    }

    const cell = mouseCell(event);
    const hover = withSurfaceOperation("link hover", () => {
      const url = state.terminal.linkUrlAt(cell.x, cell.y) ?? "";
      const linkId =
        terminalSupports("linkAt") && terminalSupports("setHoveredLinkId")
          ? state.terminal.linkAt(cell.x, cell.y)
          : null;
      if (linkId !== null) {
        state.terminal.setHoveredLinkId(linkId);
      }
      return { url, highlighted: linkId !== null };
    });
    if (hover.deferred) {
      return;
    }
    const { url, highlighted } = hover.value;
    state.hoveredLinkUrl = url;
    if (highlighted) {
      scheduleRender();
    }

    if (url) {
      setUtilityStatus(`Cmd/Ctrl-click to open ${shortenUrl(url)}.`, false);
    } else {
      setUtilityStatus("Cmd/Ctrl-click a terminal link to open it.", true);
    }
    syncLinkTools();
    syncTerminalTools();
  }

  return {
    setTerminalSelectionRange,
    clearTerminalSelection,
    setSelectMode,
    updateSearchUi,
    refreshTerminalSearch,
    applySearchQuery,
    cycleSearchMatch,
    copyTerminalSelection,
    safeOpenUrl,
    syncLinkTools,
    copyHoveredLink,
    drainTerminalLinkClicks,
    copyTerminalFrameText,
    clearHoveredLink,
    updateHoveredLink,
  };
}
