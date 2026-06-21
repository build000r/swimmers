import {
  inputAckActionPlan,
  isImeCompositionKeydown,
  terminalComposerControlAction,
} from "./input_support.js";
import {
  fallbackTextForKeyEvent,
  keyModifiers,
  terminalControlKeyEvent,
} from "./terminal_protocol.js";

export function createTerminalInputController(runtime) {
  const {
    state,
    el,
    WebSocketClass,
    windowRef,
    maxTerminalPasteBytes,
    currentSession,
    nextInputMessageId,
    updateInputDeliveryStatus,
    rejectOversizeTerminalText,
    setUtilityStatus,
    setTerminalInputEcho,
    markTrogdorSessionsResponded,
    terminalSupports,
    drainTerminalLinkClicks,
  } = runtime;

  function handleInputAck(message) {
    const plan = inputAckActionPlan(message);
    if (plan.action === "ignore") {
      return;
    }
    updateInputDeliveryStatus(plan.id, plan.status, plan.detail);
    scheduleInputAckCleanup(plan.id, plan.expectedStatus, plan.delayMs);
  }

  function scheduleInputAckCleanup(id, expectedStatus, delayMs) {
    const timer = windowRef.setTimeout(() => {
      const current = state.pendingInputMessages.get(id);
      if (current?.status === expectedStatus) {
        state.pendingInputMessages.delete(id);
      }
    }, delayMs);
    if (timer && typeof timer.unref === "function") {
      timer.unref();
    }
  }

  function flushEncodedInputBytes() {
    if (!state.terminal || !state.ws || state.ws.readyState !== WebSocketClass.OPEN || state.readOnly) {
      return;
    }

    const payload = state.terminal.drainEncodedInputBytes();
    if (!payload) {
      return;
    }

    const chunks = Array.isArray(payload) ? payload : [payload];
    for (const chunk of chunks) {
      const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
      if (bytes.byteLength > 0) {
        if (bytes.byteLength > maxTerminalPasteBytes) {
          setUtilityStatus(
            `Input blocked: ${bytes.byteLength} bytes exceeds ${maxTerminalPasteBytes}.`,
            true,
            3200,
          );
          continue;
        }
        state.ws.send(bytes);
      }
    }
  }

  function sendTerminalInputText(text) {
    if (!text || !state.ws || state.ws.readyState !== WebSocketClass.OPEN || state.readOnly) {
      return false;
    }
    if (rejectOversizeTerminalText(text, "Input")) {
      return false;
    }
    const clientMessageId = nextInputMessageId();
    state.pendingInputMessages.set(clientMessageId, { text, status: "pending", detail: "" });
    updateInputDeliveryStatus(clientMessageId, "pending");
    state.ws.send(JSON.stringify({ type: "input_text", data: text, clientMessageId }));
    return true;
  }

  function sendTerminalControlKey(actionId) {
    if (state.readOnly || !currentSession()) {
      return false;
    }
    const spec = terminalControlKeyEvent(actionId);
    if (!spec) {
      return false;
    }
    const event = {
      kind: "key",
      phase: "down",
      key: spec.key,
      code: spec.code,
      mods: spec.mods,
      repeat: false,
    };

    if ((state.terminalFallbackActive || !state.terminal) && sendFallbackTerminalEvent(event)) {
      setTerminalInputEcho(`sent: ${spec.label}`);
      return true;
    }
    if (state.terminalFallbackActive || !state.terminal) {
      setTerminalInputEcho(`failed: ${spec.label}`);
      return false;
    }

    forwardTerminalEvent(event);
    setTerminalInputEcho(`sent: ${spec.label}`);
    return true;
  }

  function terminalKeyActionForDomEvent(event) {
    return terminalComposerControlAction(event, {
      hasSelection: terminalInlineInputHasSelection(),
      inputValue: el.terminalInlineInput?.value,
    });
  }

  function terminalInlineInputHasSelection() {
    const start = Number(el.terminalInlineInput?.selectionStart);
    const end = Number(el.terminalInlineInput?.selectionEnd);
    return Number.isFinite(start) && Number.isFinite(end) && start !== end;
  }

  function sendFallbackTerminalEvent(event) {
    const text = fallbackTextForKeyEvent(event);
    if (!text) {
      return false;
    }
    return sendTerminalInputText(text);
  }

  function forwardTerminalEvent(event) {
    if (state.terminalFallbackActive && sendFallbackTerminalEvent(event)) {
      return;
    }
    if (!state.terminal || state.readOnly) {
      return;
    }
    state.terminal.input(event);
    flushEncodedInputBytes();
    drainTerminalLinkClicks();
  }

  function forwardTerminalKeyDown(event) {
    // Drop intermediate IME composition keydowns; the committed text arrives via
    // the input/compositionend path so this keeps CJK/accented input intact.
    if (isImeCompositionKeydown(event)) {
      return;
    }
    forwardTerminalEvent({
      kind: "key",
      phase: "down",
      key: typeof event.key === "string" ? event.key : "",
      code: typeof event.code === "string" ? event.code : "",
      mods: keyModifiers(event),
      repeat: Boolean(event.repeat),
    });
  }

  function forwardTerminalMouse(phase, button, hit, event) {
    forwardTerminalEvent({
      kind: "mouse",
      phase,
      button,
      x: hit.cell.x,
      y: hit.cell.y,
      mods: keyModifiers(event),
    });
  }

  function sendTerminalText(text) {
    if (!text || state.readOnly || !currentSession()) {
      return false;
    }
    if (rejectOversizeTerminalText(text, "Paste")) {
      return false;
    }
    markTrogdorSessionsResponded([state.selectedSessionId]);
    if (state.terminalFallbackActive && sendTerminalInputText(text)) {
      return true;
    }
    if (terminalSupports("pasteText")) {
      state.terminal.pasteText(text);
      flushEncodedInputBytes();
      return true;
    }
    if (state.ws && state.ws.readyState === WebSocketClass.OPEN) {
      sendTerminalInputText(text);
      return true;
    }
    forwardTerminalEvent({ kind: "paste", data: text });
    return true;
  }

  return {
    flushEncodedInputBytes,
    forwardTerminalEvent,
    forwardTerminalKeyDown,
    forwardTerminalMouse,
    handleInputAck,
    scheduleInputAckCleanup,
    sendFallbackTerminalEvent,
    sendTerminalControlKey,
    sendTerminalInputText,
    sendTerminalText,
    terminalInlineInputHasSelection,
    terminalKeyActionForDomEvent,
  };
}
