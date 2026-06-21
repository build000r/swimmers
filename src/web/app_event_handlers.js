import { bindAppEvents } from "./app_event_bindings.js";
import { commandPaletteResultEventPlan, commandPaletteSearchKeyPlan } from "./command_palette.js";
import { runGlobalShortcutAction } from "./global_shortcut_dispatch.js";
import {
  appEventListenerBindingPlan,
  globalShortcutPlan,
  mobileKeyboardInputExecutorPlan,
  mobileKeyboardInputPlan,
  mobileKeyboardKeydownPlan,
  mobileKeyboardKeyPlan,
  terminalInlineInputKeydownPlan,
  terminalKeyStripClickExecutorPlan,
  terminalKeyStripClickPlan,
  terminalStageCaptureBindings,
} from "./input_support.js";

const noop = () => {};

export function createAppEventHandlers(runtime = {}) {
  const {
    documentRef = globalThis.document,
    elements: el = {},
    state,
    ElementClass = globalThis.Element,
    ResizeObserverCtor = globalThis.ResizeObserver,
    applySearchQuery,
    bindTrogdorEvents = noop,
    captureSurfaceAction,
    clearHoveredLink,
    closeMobileKeyboard,
    closeSheets,
    currentSession,
    cycleSearchMatch,
    focusMobileKeyboard,
    focusTerminalInputSurface,
    forwardTerminalEvent,
    forwardTerminalKeyDown,
    globalShortcutRuntime,
    handleAuthTokenButtonAction,
    handleCreateBatchClearClick,
    handleCreateBatchVisibleAction,
    handleCreateCwdInput,
    handleCreateFormSubmit,
    handleCreateLaunchTargetChange,
    handleDirCheckboxChange,
    handleDirsListClick,
    handleDirsLoadButtonClick,
    handleDirsManagedOnlyChange,
    handleDirsPathInput,
    handleDirsPathKeydown,
    handleDirsSearchInput,
    handleDirsSpawnHereClick,
    handleDirsUpButtonClick,
    handleSendFormSubmit,
    handleSendHistoryClick,
    handleTerminalFallbackBlur,
    handleTerminalFallbackClick,
    handleTerminalFallbackFocus,
    handleTerminalFallbackKeyEvent,
    handleTerminalFallbackMousedown,
    handleTerminalFallbackPasteEvent,
    handleTerminalFallbackScroll,
    handleTerminalInlineInputFocus,
    handleTerminalStageClick,
    handleTerminalStageFocusEvent,
    handleTerminalStageKeydown,
    handleTerminalStageMouseDown,
    handleTerminalStageMouseMove,
    handleTerminalStageMouseUp,
    handleTerminalStagePaste,
    handleTerminalStageTouchEnd,
    handleTerminalStageWheel,
    keyBeginsTrogdorResponse,
    mermaidArtifactController,
    nativeDesktopSheet,
    openCommandPalette,
    openMermaidArtifactHost,
    openTrogdorAgentTerminal,
    openTrogdorAtlas,
    queueMeasureAndResizeSurface,
    refreshAgentContextForSelectedSession,
    refreshMermaidArtifact,
    refreshWorkbenchWidgetsForSelectedSession,
    renderCommandPalette,
    runCommandPaletteItem,
    sendTerminalControlKey,
    sendTerminalText,
    syncSheetActionAvailability,
    terminalKeyActionForDomEvent,
    terminalWorkbenchController,
    terminalZoomInputController,
    thoughtConfigSheet,
    updateHoveredTrogdorSurface,
    updateSendHint,
  } = runtime;

  function handleGlobalShortcut(event) {
    const plan = globalShortcutPlan(event, {
      activeSheet: state.activeSheet,
      trogdorAtlasOpen: state.trogdorAtlasOpen,
      selectMode: state.selectMode,
      readOnly: state.readOnly,
      hasCurrentSession: Boolean(currentSession()),
      hoveredLinkUrl: state.hoveredLinkUrl,
    });
    if (plan.type === "unhandled") {
      return false;
    }
    runGlobalShortcutAction(plan, globalShortcutRuntime);
    return true;
  }

  function handleMobileKeyboardProxyKeydown(event) {
    const globalShortcutHandled = handleGlobalShortcut(event);
    const keyPlan = globalShortcutHandled ? { type: "ignore" } : mobileKeyboardKeyPlan(event, {
      readOnly: state.readOnly,
      hasCurrentSession: Boolean(currentSession()),
    });
    const shouldForwardKey = !globalShortcutHandled && keyPlan.type === "forward_key";
    const plan = mobileKeyboardKeydownPlan({
      globalShortcutHandled,
      keyPlan,
      beginsResponse: shouldForwardKey && keyBeginsTrogdorResponse(event),
    });
    if (plan.preventDefault) event.preventDefault();
    if (plan.closeKeyboard) closeMobileKeyboard();
    if (plan.focusTerminal) focusTerminalInputSurface({ preventScroll: true });
    if (plan.markResponse) runtime.markTrogdorSessionsResponded([state.selectedSessionId]);
    if (plan.forwardKey) forwardTerminalKeyDown(event);
    return plan.handled;
  }

  function handleMobileKeyboardProxyInput(event) {
    const plan = mobileKeyboardInputPlan(event, {
      readOnly: state.readOnly,
      hasCurrentSession: Boolean(currentSession()),
      proxyValue: el.mobileKeyboardProxy.value,
    });
    el.mobileKeyboardProxy.value = "";
    const action = mobileKeyboardInputExecutorPlan(plan);
    if (action.forwardEvent) forwardTerminalEvent(action.forwardEvent);
    if (action.sendText) sendTerminalText(action.text);
    return action.handled;
  }

  function handleTerminalInlineInputKeydown(event) {
    const actionId = event.key === "Enter" && !event.shiftKey ? "" : terminalKeyActionForDomEvent(event);
    const plan = terminalInlineInputKeydownPlan(event, actionId);
    if (plan.preventDefault) event.preventDefault();
    if (plan.submit) void runtime.submitTerminalInputDock();
    if (plan.sendKey) sendTerminalControlKey(plan.actionId);
    if (plan.stopPropagation) event.stopPropagation();
    return plan.handled;
  }

  function handleTerminalWorkbenchWidgetsClick(event) {
    return terminalWorkbenchController.handleTerminalWorkbenchWidgetsClick(event);
  }

  function handleTerminalWorkbenchWidgetsLogEvent(event) {
    return terminalWorkbenchController.handleTerminalWorkbenchWidgetsLogEvent(event);
  }

  function handleCommandPaletteEvent(event) {
    const target = event.target instanceof ElementClass ? event.target : null;
    const plan = event.type === "keydown"
      ? commandPaletteSearchKeyPlan(event, state.paletteIndex, state.paletteItems)
      : commandPaletteResultEventPlan(event.type, target, state.paletteItems.length);
    if (plan.type === "ignore") {
      return false;
    }
    if (plan.preventDefault) {
      event.preventDefault();
    }
    if (Number.isFinite(plan.index)) {
      state.paletteIndex = plan.index;
    }
    if (plan.type === "run_item") {
      void runCommandPaletteItem();
      return true;
    }
    renderCommandPalette();
    return true;
  }

  function handleDocumentCommandPaletteShortcut(event) {
    if ((event.ctrlKey || event.metaKey) && !event.altKey && event.code === "KeyK") {
      event.preventDefault();
      openCommandPalette();
    }
  }

  function handleTerminalKeyStripClick(event) {
    const action = terminalKeyStripClickExecutorPlan(terminalKeyStripClickPlan(event.type, event.target));
    if (!action.sendKey) return;
    if (action.preventDefault) event.preventDefault();
    sendTerminalControlKey(action.actionId);
    focusTerminalInputSurface({ preventScroll: true });
  }

  function handleTerminalMobileKeyboardClick() {
    if (state.mobileKeyboardActive) {
      closeMobileKeyboard();
      focusTerminalInputSurface({ preventScroll: true });
      return;
    }
    focusMobileKeyboard();
  }

  function handleTerminalWorkbenchToggleClick() {
    runtime.setTerminalWorkbenchOpen(!state.terminalWorkbenchOpen);
    focusTerminalInputSurface({ preventScroll: true });
  }

  function handleTerminalWorkbenchRefreshClick() {
    void refreshAgentContextForSelectedSession({ force: true });
    void refreshWorkbenchWidgetsForSelectedSession({ force: true });
    focusTerminalInputSurface({ preventScroll: true });
  }

  function handleTerminalStageFocus() {
    handleTerminalStageFocusEvent("focus");
  }

  function handleTerminalStageBlur() {
    handleTerminalStageFocusEvent("blur");
  }

  function handleTerminalStageMouseleave() {
    clearHoveredLink(true);
    updateHoveredTrogdorSurface(null);
  }

  const eventListenerHandlers = {
    closeSheets,
    handleClearTokenButtonClick: () => handleAuthTokenButtonAction("clear"),
    handleCommandPaletteEvent,
    handleCreateBatchClearClick,
    handleCreateBatchVisibleAction,
    handleCreateCwdInput,
    handleCreateFormSubmit,
    handleCreateLaunchTargetChange,
    handleCreateRequestInput: () => syncSheetActionAvailability(),
    handleCreateToolChange: () => syncSheetActionAvailability(),
    handleDirCheckboxChange,
    handleDirsListClick,
    handleDirsLoadButtonClick,
    handleDirsManagedOnlyChange,
    handleDirsPathInput,
    handleDirsPathKeydown,
    handleDirsSearchInput,
    handleDirsSpawnHereClick,
    handleDirsUpButtonClick,
    handleDocumentCommandPaletteShortcut,
    handleMermaidOpenButtonClick: () => openMermaidArtifactHost(),
    handleMermaidPlanTabsClick: (event) => mermaidArtifactController.handlePlanTabsClick(event),
    handleMermaidRefreshButtonClick: () => refreshMermaidArtifact(),
    handleMobileKeyboardProxyBlur: () => runtime.handleMobileKeyboardProxyFocusEvent(false),
    handleMobileKeyboardProxyFocus: () => runtime.handleMobileKeyboardProxyFocusEvent(true),
    handleMobileKeyboardProxyInput,
    handleMobileKeyboardProxyKeydown,
    handleModalRootKeydown: (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeSheets();
      }
    },
    handleNativeAppChange: () => nativeDesktopSheet.handleNativeAppChange(),
    handleNativeFormSubmit: (event) => nativeDesktopSheet.handleNativeFormSubmit(event),
    handleNativeModeChange: () => nativeDesktopSheet.handleNativeModeChange(),
    handleNativeOpenButtonClick: () => nativeDesktopSheet.handleNativeOpenButtonClick(),
    handleNativeRefreshButtonClick: () => nativeDesktopSheet.handleNativeRefreshButtonClick(),
    handlePaletteSearchInput: () => {
      state.paletteIndex = 0;
      renderCommandPalette();
    },
    handleSaveTokenButtonClick: () => handleAuthTokenButtonAction("save"),
    handleSearchClearButtonClick: () => {
      el.terminalSearch.value = "";
      applySearchQuery("");
    },
    handleSearchFormSubmit: (event) => {
      event.preventDefault();
      closeSheets();
    },
    handleSearchNextButtonClick: () => cycleSearchMatch(1),
    handleSearchPrevButtonClick: () => cycleSearchMatch(-1),
    handleSendCloseButtonClick: () => {
      state.sendTarget = null;
      closeSheets();
    },
    handleSendFormSubmit,
    handleSendHistoryClick,
    handleSendModeChange: () => updateSendHint(),
    handleTerminalCopyFrameClick: () => { void runtime.copyTerminalFrameText(); },
    handleTerminalFallbackBlur,
    handleTerminalFallbackClick,
    handleTerminalFallbackFocus,
    handleTerminalFallbackKeyEvent,
    handleTerminalFallbackMousedown,
    handleTerminalFallbackPasteEvent,
    handleTerminalFallbackScroll,
    handleTerminalInlineInputFocus,
    handleTerminalInlineInputInput: () => terminalZoomInputController.handleTerminalInlineInputInput(),
    handleTerminalInlineInputKeydown,
    handleTerminalInputDockSubmit: (event) => terminalZoomInputController.handleTerminalInputDockSubmit(event),
    handleTerminalKeyStripClick,
    handleTerminalLinkCopyClick: () => { void runtime.copyHoveredLink(); },
    handleTerminalLinkOpenClick: () => {
      if (state.hoveredLinkUrl) runtime.safeOpenUrl(state.hoveredLinkUrl);
    },
    handleTerminalMobileKeyboardClick,
    handleTerminalPaletteClick: () => openCommandPalette(),
    handleTerminalSearchInput: (event) => applySearchQuery(event.target.value),
    handleTerminalStageBlur,
    handleTerminalStageClick,
    handleTerminalStageFocus,
    handleTerminalStageKeydown,
    handleTerminalStageMouseDown,
    handleTerminalStageMouseMove,
    handleTerminalStageMouseUp,
    handleTerminalStageMouseleave,
    handleTerminalStagePaste,
    handleTerminalStageTouchEnd,
    handleTerminalStageWheel,
    handleTerminalTrogdorBackClick: (event) => {
      event.preventDefault();
      openTrogdorAtlas();
    },
    handleTerminalWorkbenchRefreshClick,
    handleTerminalWorkbenchToggleClick,
    handleTerminalWorkbenchWidgetsClick,
    handleTerminalWorkbenchWidgetsLogEvent,
    handleTerminalZoomInClick: () => terminalZoomInputController.handleTerminalZoomInClick(),
    handleTerminalZoomOutClick: () => terminalZoomInputController.handleTerminalZoomOutClick(),
    handleTerminalZoomResetClick: () => terminalZoomInputController.handleTerminalZoomResetClick(),
    handleThoughtConfigBackendChange: () => thoughtConfigSheet.handleBackendChange(),
    handleThoughtConfigFormSubmit: (event) => thoughtConfigSheet.handleFormSubmit(event),
    handleThoughtConfigOptionChange: () => thoughtConfigSheet.handleOptionChange(),
    handleThoughtConfigTestButtonClick: () => thoughtConfigSheet.handleTestButtonClick(),
  };

  function bindEvents() {
    bindAppEvents({
      document: documentRef,
      elements: el,
      handlers: eventListenerHandlers,
      bindTrogdorEvents,
      appEventListenerBindingPlan,
      terminalStageCaptureBindings,
      captureSurfaceAction,
      ResizeObserver: ResizeObserverCtor,
      queueMeasureAndResizeSurface,
    });
  }

  return {
    bindEvents,
    handleCommandPaletteEvent,
    handleGlobalShortcut,
    handleMobileKeyboardProxyInput,
    handleMobileKeyboardProxyKeydown,
    handleTerminalInlineInputKeydown,
    handleTerminalWorkbenchWidgetsClick,
  };
}
