import { runAgentContextRefresh } from "./agent_context_refresh.js";
import {
  writeWorkbenchWidgetsHtmlToDom,
  writeWorkbenchWidgetsViewToDom,
} from "./workbench_dom.js";
import { runWorkbenchWidgetRefresh } from "./workbench_refresh.js";
import {
  agentActionLabel,
  buildWorkbenchWidgetsViewModel,
  operatorPressureSummary,
  renderWorkbenchWidgetsViewModelHtml,
  renderTerminalWorkbenchActions,
  resetWorkbenchWidgetsState,
  selectedWorkbenchWidgetsSnapshot,
  truncateWorkbenchText,
  workbenchWidgetClickPlan,
  workbenchWidgetLogPlan,
} from "./workbench_render.js";

export function createTerminalWorkbenchController({
  state,
  el,
  refreshMs = 5000,
  currentSession,
  normalizeSessionId = (sessionId) => sessionId || null,
  sessionDisplayName = (session) => session?.session_id || "No session",
  summarizeThought = (session) => session?.thought || "",
  apiFetch,
  apiMaybeFetch,
  responseJson,
  responseJsonOrNull,
  openSheet = () => {},
  focusTerminalInputSurface = () => {},
  documentRef = globalThis.document,
  requestAnimationFrameRef = globalThis.requestAnimationFrame,
  renderWorkbenchWidgetsView = null,
} = {}) {
  function resetAgentContextForSession(sessionId) {
    state.agentContextSessionId = normalizeSessionId(sessionId);
    state.agentContextLoading = false;
    state.agentContextPayload = null;
    state.agentContextError = "";
    state.agentContextLastLoadedAt = 0;
    renderTerminalWorkbench();
  }

  function resetWorkbenchWidgetsForSession(sessionId) {
    resetWorkbenchWidgetsState(state.workbenchWidgets, normalizeSessionId(sessionId));
    state.workbenchLogMode = "lens";
    state.workbenchLogFilter = "all";
    state.workbenchLogSearch = "";
    state.workbenchSelectedTurnId = "";
    renderWorkbenchWidgets();
  }

  function terminalWorkbenchVisible() {
    return Boolean(currentSession() && !state.trogdorAtlasOpen && state.terminalWorkbenchOpen);
  }

  function syncTerminalWorkbench() {
    const hasSession = Boolean(currentSession() && !state.trogdorAtlasOpen);
    const visible = terminalWorkbenchVisible();
    documentRef.body.classList.toggle("terminal-workbench-open", visible);
    if (el.terminalWorkbenchToggle) {
      el.terminalWorkbenchToggle.disabled = !hasSession;
      el.terminalWorkbenchToggle.setAttribute("aria-pressed", visible ? "true" : "false");
    }
    if (el.terminalWorkbench) {
      el.terminalWorkbench.classList.toggle("hidden", !visible);
      el.terminalWorkbench.setAttribute("aria-hidden", visible ? "false" : "true");
    }
    renderTerminalWorkbench();
  }

  function setTerminalWorkbenchOpen(open) {
    state.terminalWorkbenchOpen = Boolean(open);
    syncTerminalWorkbench();
    if (state.terminalWorkbenchOpen) {
      void refreshAgentContextForSelectedSession({ force: true });
      void refreshWorkbenchWidgetsForSelectedSession({ force: true });
    }
  }

  function selectedAgentContextPayload() {
    return state.agentContextSessionId === state.selectedSessionId
      ? state.agentContextPayload
      : null;
  }

  function renderTerminalWorkbench() {
    if (!el.terminalWorkbench) {
      return;
    }

    const session = currentSession();
    const payload = selectedAgentContextPayload();
    const tool = payload?.tool || session?.tool || "unknown";
    const cwd = payload?.cwd || session?.cwd || "";
    const status = state.agentContextLoading
      ? "loading context"
      : state.agentContextError
        ? state.agentContextError
        : payload?.available
          ? "structured context"
          : payload?.message || "waiting for context";
    const task = payload?.user_task || summarizeThought(session);
    const current = agentActionLabel(payload?.current_tool) || "No current action.";
    const pressure = operatorPressureSummary(session, payload);
    const actions = Array.isArray(payload?.recent_actions) ? payload.recent_actions : [];

    el.terminalWorkbenchTitle.textContent = session ? sessionDisplayName(session) : "No session";
    el.terminalWorkbenchMeta.textContent = session ? `${tool} · ${cwd}` : "";
    el.terminalWorkbenchStatus.textContent = status;
    el.terminalWorkbenchTask.textContent = truncateWorkbenchText(task || "No task context.");
    el.terminalWorkbenchCurrent.textContent = truncateWorkbenchText(current, 140);
    el.terminalWorkbenchPressure.textContent = truncateWorkbenchText(pressure, 160);
    el.terminalWorkbenchRefresh.disabled = !session || state.agentContextLoading;

    el.terminalWorkbenchActions.innerHTML = renderTerminalWorkbenchActions(actions, Boolean(payload?.available));
    renderWorkbenchWidgets();
  }

  async function refreshAgentContextForSelectedSession(options = {}) {
    await runAgentContextRefresh(options, {
      state,
      throttleMs: refreshMs,
      now: () => Date.now(),
      currentSession,
      apiFetch,
      responseJson,
      renderTerminalWorkbench,
    });
  }

  function selectedWorkbenchWidgets() {
    return selectedWorkbenchWidgetsSnapshot(state.workbenchWidgets, state.selectedSessionId);
  }

  function writeWorkbenchWidgetsHtml(nextHtml) {
    writeWorkbenchWidgetsHtmlToDom(nextHtml, {
      container: el.terminalWorkbenchWidgets,
      scroller: el.terminalWorkbench,
      widgets: state.workbenchWidgets,
      requestAnimationFrame: typeof requestAnimationFrameRef === "function"
        ? (callback) => requestAnimationFrameRef(callback)
        : null,
    });
  }

  function writeWorkbenchWidgetsView(view) {
    writeWorkbenchWidgetsViewToDom(view, {
      container: el.terminalWorkbenchWidgets,
      scroller: el.terminalWorkbench,
      widgets: state.workbenchWidgets,
      requestAnimationFrame: typeof requestAnimationFrameRef === "function"
        ? (callback) => requestAnimationFrameRef(callback)
        : null,
      renderWorkbenchWidgetsView,
    });
  }

  function renderWorkbenchWidgets() {
    if (!el.terminalWorkbenchWidgets) {
      return;
    }

    const session = currentSession();
    const widgets = selectedWorkbenchWidgets();
    if (!session) {
      writeWorkbenchWidgetsHtml(
        `<div class="workbench-action-detail">No session selected.</div>`,
      );
      return;
    }

    const contextPayload = selectedAgentContextPayload();
    const model = buildWorkbenchWidgetsViewModel({
      widgets,
      contextPayload,
      selectedTurnId: state.workbenchSelectedTurnId,
      logState: {
        mode: state.workbenchLogMode,
        filter: state.workbenchLogFilter,
        query: state.workbenchLogSearch,
      },
    });
    writeWorkbenchWidgetsView({
      html: renderWorkbenchWidgetsViewModelHtml(model),
      model,
    });
  }

  async function refreshWorkbenchWidgetsForSelectedSession(options = {}) {
    await runWorkbenchWidgetRefresh(options, {
      state,
      throttleMs: refreshMs,
      currentSession,
      renderWorkbenchWidgets,
      apiMaybeFetch,
      responseJsonOrNull,
    });
  }

  function handleTerminalWorkbenchWidgetsClick(event) {
    const plan = workbenchWidgetClickPlan(event.target);
    if (plan.type === "ignore") {
      return false;
    }
    event.preventDefault();
    if (plan.type === "open_mermaid") {
      openSheet("mermaid");
      return;
    }
    const refreshWidgets = plan.type === "select_turn";
    if (refreshWidgets) {
      state.workbenchSelectedTurnId = plan.turnId;
      state.workbenchWidgets.transcript = null;
      state.workbenchWidgets.transcriptTurnId = "";
      state.workbenchWidgets.transcriptNextCursor = 0;
    } else {
      state.workbenchLogMode = plan.mode;
    }
    renderWorkbenchWidgets();
    if (refreshWidgets) {
      void refreshWorkbenchWidgetsForSelectedSession({ force: true, silent: true });
    }
    focusTerminalInputSurface({ preventScroll: true });
  }

  function handleTerminalWorkbenchWidgetsLogEvent(event) {
    const plan = workbenchWidgetLogPlan(event.type, event.target);
    if (plan.type === "set_log_search") {
      state.workbenchLogSearch = plan.query;
    } else if (plan.type === "set_log_filter") {
      state.workbenchLogFilter = plan.filter;
    } else {
      return;
    }
    renderWorkbenchWidgets();
  }

  return {
    handleTerminalWorkbenchWidgetsClick,
    handleTerminalWorkbenchWidgetsLogEvent,
    refreshAgentContextForSelectedSession,
    refreshWorkbenchWidgetsForSelectedSession,
    renderTerminalWorkbench,
    renderWorkbenchWidgets,
    resetAgentContextForSession,
    resetWorkbenchWidgetsForSession,
    selectedAgentContextPayload,
    selectedWorkbenchWidgets,
    setTerminalWorkbenchOpen,
    syncTerminalWorkbench,
    terminalWorkbenchVisible,
    writeWorkbenchWidgetsHtml,
  };
}
