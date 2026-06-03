import {
  commandPaletteExecutionPlan,
  filteredCommandPaletteItemsForState,
  renderCommandPaletteResultsHtml,
} from "./command_palette.js";

export function createCommandPaletteController({
  state,
  el,
  documentRef = document,
  requestAnimationFrameRef = requestAnimationFrame,
  currentSession,
  copyTerminalFrameText,
  clampInt,
  selectSession,
  handleSurfaceAction,
  syncSheetActionAvailability,
  renderHudSurface,
  focusTerminalInputSurface,
  clearCreateBatchSelection,
  openCreateSheet,
  refreshThoughtConfig,
  refreshNativeStatus,
  refreshMermaidArtifact,
}) {
  function filteredCommandPaletteItems() {
    return filteredCommandPaletteItemsForState({
      selectedSession: currentSession(),
      readOnly: state.readOnly,
      sessions: state.sessions,
      copyFrameAction: copyTerminalFrameText,
      query: el.paletteSearch?.value,
    });
  }

  function renderCommandPalette() {
    if (!el.paletteResults) {
      return;
    }
    state.paletteItems = filteredCommandPaletteItems();
    state.paletteIndex = clampInt(state.paletteIndex, 0, 0, Math.max(0, state.paletteItems.length - 1));
    el.paletteResults.innerHTML = renderCommandPaletteResultsHtml(state.paletteItems, state.paletteIndex);
  }

  async function runCommandPaletteItem(item = state.paletteItems[state.paletteIndex]) {
    const plan = commandPaletteExecutionPlan(item);
    if (plan.type === "none") {
      return false;
    }
    closeSheets();
    if (plan.type === "selectSession") {
      await selectSession(plan.sessionId);
    } else if (plan.type === "invokeAction") {
      await plan.action();
    } else if (plan.type === "dispatchAction") {
      await handleSurfaceAction({ type: "action", actionId: plan.actionId });
    } else {
      return false;
    }
    return true;
  }

  function openCommandPalette() {
    setActiveSheet("palette");
    if (el.paletteSearch) {
      el.paletteSearch.value = "";
    }
    state.paletteIndex = 0;
    renderCommandPalette();
    focusActiveSheet();
  }

  function setActiveSheet(sheetId) {
    state.activeSheet = sheetId;
    documentRef.body.classList.toggle("sheet-open", Boolean(sheetId));
    el.modalRoot.classList.toggle("visible", Boolean(sheetId));
    el.modalRoot.setAttribute("aria-hidden", sheetId ? "false" : "true");
    el.paletteSheet.classList.toggle("hidden", sheetId !== "palette");
    el.searchSheet.classList.toggle("hidden", sheetId !== "search");
    el.thoughtConfigSheet.classList.toggle("hidden", sheetId !== "thought-config");
    el.nativeSheet.classList.toggle("hidden", sheetId !== "native");
    el.sendSheet.classList.toggle("hidden", sheetId !== "send");
    el.authSheet.classList.toggle("hidden", sheetId !== "auth");
    el.createSheet.classList.toggle("hidden", sheetId !== "create");
    el.mermaidSheet.classList.toggle("hidden", sheetId !== "mermaid");
    syncSheetActionAvailability();
    renderHudSurface();
  }

  function focusActiveSheet() {
    requestAnimationFrameRef(() => {
      switch (state.activeSheet) {
        case "palette":
          el.paletteSearch.focus();
          el.paletteSearch.select();
          break;
        case "search":
          el.terminalSearch.focus();
          el.terminalSearch.select();
          break;
        case "thought-config":
          el.thoughtConfigModel.focus();
          el.thoughtConfigModel.select();
          break;
        case "native":
          el.nativeApp.focus();
          break;
        case "send":
          el.sendInput.focus();
          break;
        case "auth":
          el.tokenInput.focus();
          el.tokenInput.select();
          break;
        case "create":
          {
            const firstCheckbox = el.dirsList.querySelector(".dir-row-check:not(:disabled)");
            if (firstCheckbox) {
              firstCheckbox.focus();
            } else {
              el.createCwd.focus();
            }
          }
          break;
        case "mermaid":
          el.mermaidRefreshButton.focus();
          break;
        default:
          focusTerminalInputSurface({ preventScroll: true });
          break;
      }
    });
  }

  function openSheet(sheetId) {
    setActiveSheet(sheetId);
    if (sheetId === "palette") {
      renderCommandPalette();
    }
    if (sheetId === "search") {
      el.terminalSearch.value = state.searchQuery;
    }
    if (sheetId === "create") {
      void openCreateSheet();
    }
    if (sheetId === "thought-config") {
      void refreshThoughtConfig();
    }
    if (sheetId === "native") {
      void refreshNativeStatus();
    }
    if (sheetId === "mermaid") {
      void refreshMermaidArtifact();
    }
    if (sheetId === "auth") {
      el.tokenInput.value = state.token;
    }
    focusActiveSheet();
  }

  function closeSheets() {
    if (state.activeSheet === "send") {
      state.sendTarget = null;
    }
    if (state.activeSheet === "create") {
      clearCreateBatchSelection();
      state.dirBrowser.group = "";
    }
    setActiveSheet(null);
    focusTerminalInputSurface({ preventScroll: true });
  }

  return {
    filteredCommandPaletteItems,
    renderCommandPalette,
    runCommandPaletteItem,
    openCommandPalette,
    setActiveSheet,
    focusActiveSheet,
    openSheet,
    closeSheets,
  };
}
