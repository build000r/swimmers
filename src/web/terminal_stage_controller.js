import { surfaceActionAt, surfaceConsumesPointer } from "./rendered_surface.js";
import {
  shouldIgnoreSyntheticClick,
  terminalStageClickPlan,
  terminalStageKeydownPlan,
  terminalStageMouseDownPlan,
  terminalStageMouseMovePlan,
  terminalStageMouseUpPlan,
  terminalStagePasteExecutorPlan,
  terminalStagePastePlan,
  terminalStageTouchEndPlan,
  terminalStageWheelPlan,
} from "./input_support.js";
import { keyModifiers } from "./terminal_protocol.js";

const noop = () => {};

export function createTerminalStageController(runtime = {}) {
  const {
    state,
    el,
    ElementClass = globalThis.Element,
    performanceRef = globalThis.performance,
    surfaceClickSuppressMs = 450,
    mouseCell,
    cellOffset,
    clampInt,
    handleSurfaceAction = noop,
    handleGlobalShortcut = () => false,
    shouldCaptureKey = () => false,
    keyBeginsTrogdorResponse = () => false,
    markTrogdorSessionsResponded = noop,
    forwardTerminalKeyDown = noop,
    sendTerminalText = noop,
    updateHoveredLink = noop,
    clearHoveredLink = noop,
    safeOpenUrl = noop,
    setTerminalSelectionRange = noop,
    forwardTerminalMouse = noop,
    forwardTerminalEvent = noop,
    updateHoveredTrogdorSurface = noop,
    focusMobileKeyboard = () => false,
    focusTerminalInputSurface = noop,
    isCoarsePointer = () => false,
  } = runtime;

  function now() {
    return typeof performanceRef?.now === "function" ? performanceRef.now() : 0;
  }

  function surfaceHit(event) {
    const cell = mouseCell(event);
    return {
      cell,
      action: surfaceActionAt(state.surfaceZones, cell),
      consume: surfaceConsumesPointer(state.surfaceMasks, cell),
    };
  }

  function terminalFallbackOwnsPointer(event) {
    return Boolean(
      state.terminalFallbackActive &&
        event.target instanceof ElementClass &&
        event.target.closest("#terminal-fallback"),
    );
  }

  function captureSurfaceAction(event, phase) {
    if (state.activeSheet) {
      return false;
    }
    if (terminalFallbackOwnsPointer(event)) {
      return false;
    }
    if (event.target instanceof ElementClass && event.target.closest("#trogdor-surface, #trogdor-launcher")) {
      return false;
    }
    const hit = surfaceHit(event);
    if (!hit.action && !hit.consume) {
      return false;
    }

    if (hit.action) {
      if (phase === "wheel") {
        event.preventDefault();
        stopSurfaceEvent(event);
        return true;
      }
      if (phase === "click" && shouldIgnoreSyntheticClick(now(), state.surfaceClickSuppressUntil)) {
        event.preventDefault();
        stopSurfaceEvent(event);
        return true;
      }
      if (phase === "down" || phase === "touch" || phase === "click") {
        if (phase === "down" || phase === "touch") {
          state.surfaceClickSuppressUntil = now() + surfaceClickSuppressMs;
        }
        event.preventDefault();
        stopSurfaceEvent(event);
        void handleSurfaceAction(hit.action);
        return true;
      }
    }

    if (hit.consume) {
      event.preventDefault();
      stopSurfaceEvent(event);
      return true;
    }

    return false;
  }

  function stopSurfaceEvent(event) {
    if (typeof event.stopImmediatePropagation === "function") {
      event.stopImmediatePropagation();
      return;
    }
    event.stopPropagation();
  }

  function applyTerminalStagePointerPlan(event, plan) {
    if (plan.suppressClick) state.surfaceClickSuppressUntil = now() + surfaceClickSuppressMs;
    if (plan.preventDefault) event.preventDefault();
    if (plan.handleAction) {
      void handleSurfaceAction(plan.action);
      return;
    }
    if (plan.focusMobileThenTerminal) {
      if (!isCoarsePointer() || !focusMobileKeyboard()) {
        focusTerminalInputSurface({ preventScroll: true });
      }
      return;
    }
    if (plan.focusTerminal) focusTerminalInputSurface({ preventScroll: true });
  }

  function handleTerminalStageClick(event) {
    const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
    const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
    const plan = terminalStageClickPlan({
      fallbackOwnsPointer,
      hit,
      activeSheet: state.activeSheet,
      ignoreSyntheticClick: hit.action ? shouldIgnoreSyntheticClick(now(), state.surfaceClickSuppressUntil) : false,
    });
    applyTerminalStagePointerPlan(event, plan);
  }

  function handleTerminalStageTouchEnd(event) {
    const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
    const plan = terminalStageTouchEndPlan({
      fallbackOwnsPointer,
      hit: fallbackOwnsPointer ? {} : surfaceHit(event),
      activeSheet: state.activeSheet,
    });
    applyTerminalStagePointerPlan(event, plan);
  }

  function handleTerminalStageKeydown(event) {
    const globalShortcutHandled = handleGlobalShortcut(event);
    const shouldCaptureTerminalKey = !globalShortcutHandled && shouldCaptureKey(event);
    const plan = terminalStageKeydownPlan({ globalShortcutHandled, shouldCaptureKey: shouldCaptureTerminalKey, beginsResponse: shouldCaptureTerminalKey && keyBeginsTrogdorResponse(event) });
    if (plan.preventDefault) event.preventDefault();
    if (plan.markResponse) markTrogdorSessionsResponded([state.selectedSessionId]);
    if (plan.forwardKey) forwardTerminalKeyDown(event);
  }

  function handleTerminalStagePaste(event) {
    const action = terminalStagePasteExecutorPlan(terminalStagePastePlan(state.readOnly, event.clipboardData?.getData("text") ?? ""));
    if (action.preventDefault) event.preventDefault();
    if (action.sendText) sendTerminalText(action.text);
  }

  function handleTerminalStageMouseDown(event) {
    const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
    const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
    if (!fallbackOwnsPointer && !hit.action && !hit.consume && state.terminal) updateHoveredLink(event);
    const plan = terminalStageMouseDownPlan({
      fallbackOwnsPointer,
      hit,
      hasTerminal: Boolean(state.terminal),
      modifierKey: event.metaKey || event.ctrlKey,
      hoveredLinkUrl: state.hoveredLinkUrl,
      selectMode: state.selectMode,
      button: event.button,
      readOnly: state.readOnly,
    });
    applyTerminalStageMousePlan(event, plan, hit);
  }

  function handleTerminalStageMouseUp(event) {
    const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
    const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
    if (!fallbackOwnsPointer && !hit.action && !hit.consume && state.terminal) updateHoveredLink(event);
    const plan = terminalStageMouseUpPlan({
      fallbackOwnsPointer,
      hit,
      hasTerminal: Boolean(state.terminal),
      modifierKey: event.metaKey || event.ctrlKey,
      hoveredLinkUrl: state.hoveredLinkUrl,
      selectMode: state.selectMode,
      selectionAnchor: state.selectionAnchor,
      button: event.button,
      readOnly: state.readOnly,
    });
    applyTerminalStageMousePlan(event, plan, hit);
  }

  function applyTerminalStageMousePlan(event, plan, hit) {
    if (plan.suppressClick) state.surfaceClickSuppressUntil = now() + surfaceClickSuppressMs;
    if (plan.preventDefault) event.preventDefault();
    if (plan.handleAction) {
      void handleSurfaceAction(plan.action);
    } else if (plan.openHoveredLink) {
      safeOpenUrl(state.hoveredLinkUrl);
    } else if (plan.startSelection) {
      const anchor = cellOffset(hit.cell);
      state.selectionAnchor = anchor;
      setTerminalSelectionRange(anchor, anchor);
    } else if (plan.completeSelection) {
      setTerminalSelectionRange(state.selectionAnchor, cellOffset(hit.cell));
      state.selectionAnchor = null;
    } else if (plan.forwardMouse) {
      forwardTerminalMouse(plan.mouseKind, clampInt(event.button, 0, 0, 2), hit, event);
    }
  }

  function handleTerminalStageMouseMove(event) {
    const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
    const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
    const plan = terminalStageMouseMovePlan({
      fallbackOwnsPointer, hit, hasTerminal: Boolean(state.terminal), selectMode: state.selectMode,
      selectionAnchor: state.selectionAnchor, buttons: event.buttons, readOnly: state.readOnly,
    });
    applyTerminalStageMouseMovePlan(event, plan, hit);
  }

  function applyTerminalStageMouseMovePlan(event, plan, hit) {
    if (plan.updateTrogdorSurface) updateHoveredTrogdorSurface(plan.trogdorZone);
    if (plan.clearHoveredLink) clearHoveredLink(true);
    if (plan.preventDefault) event.preventDefault();
    if (plan.updateSelectionRange) {
      setTerminalSelectionRange(state.selectionAnchor, cellOffset(hit.cell));
    }
    if (plan.updateHoveredLink) updateHoveredLink(event);
    if (plan.forwardMouse) forwardTerminalMouse("move", 0, hit, event);
  }

  function handleTerminalStageWheel(event) {
    const fallbackOwnsPointer = terminalFallbackOwnsPointer(event);
    const hit = fallbackOwnsPointer ? {} : surfaceHit(event);
    const plan = terminalStageWheelPlan({
      fallbackOwnsPointer, hit, hasTerminal: Boolean(state.terminal),
      readOnly: state.readOnly, selectMode: state.selectMode,
    });
    applyTerminalStageWheelPlan(event, plan, hit);
  }

  function applyTerminalStageWheelPlan(event, plan, hit) {
    if (plan.preventDefault) event.preventDefault();
    if (plan.forwardWheel) {
      forwardTerminalEvent({
        kind: "wheel",
        x: hit.cell.x,
        y: hit.cell.y,
        dx: Math.round(event.deltaX),
        dy: Math.round(event.deltaY),
        mods: keyModifiers(event),
      });
    }
  }

  return {
    captureSurfaceAction,
    handleTerminalStageClick,
    handleTerminalStageKeydown,
    handleTerminalStageMouseDown,
    handleTerminalStageMouseMove,
    handleTerminalStageMouseUp,
    handleTerminalStagePaste,
    handleTerminalStageTouchEnd,
    handleTerminalStageWheel,
    surfaceHit,
    terminalFallbackOwnsPointer,
  };
}
