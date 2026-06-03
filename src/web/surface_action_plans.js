export function surfaceActionDispatchPlan(zone, context = {}) {
  if (!zone || zone.disabled) {
    return { type: "ignore" };
  }

  switch (zone.type) {
    case "session":
      return { type: "select_session", sessionId: zone.sessionId };
    case "trogdor_agent":
      return { type: "open_trogdor_agent_terminal", sessionId: zone.sessionId };
    case "trogdor_reader":
      return { type: "ignore" };
    default:
      break;
  }

  switch (zone.actionId) {
    case "trogdor_read_toggle":
      return { type: "trogdor_read_toggle" };
    case "trogdor_wpm_down":
    case "trogdor_wpm_up":
      return { type: "trogdor_wpm", actionId: zone.actionId };
    case "toggle_trogdor_atlas":
      return { type: "toggle_trogdor_atlas" };
    case "trogdor_send":
    case "trogdor_group_send":
      return { type: "open_send_sheet_for_zone" };
    case "trogdor_launch":
      return { type: "open_create_sheet_for_zone_cwd" };
    case "trogdor_mermaid":
      return { type: "select_then_open_mermaid_for_zone" };
    case "trogdor_commit":
      return { type: "select_then_launch_commit_for_zone" };
    case "open_search":
      return { type: "open_sheet", sheetId: "search" };
    case "open_send":
      if (context.readOnly || !context.currentSession) {
        return { type: "ignore" };
      }
      return {
        type: "open_send_sheet_for_current_session",
        payload: {
          type: "session",
          sessionId: context.currentSession.session_id,
          label: context.currentSession.tmux_name || context.currentSession.session_id,
        },
      };
    case "open_auth":
      return { type: "open_sheet", sheetId: "auth" };
    case "open_config":
      return { type: "open_thought_config" };
    case "open_native":
      return { type: "open_native" };
    case "open_mermaid":
      return { type: "open_mermaid" };
    case "launch_commit":
      return { type: "launch_commit" };
    case "open_create":
      return context.readOnly ? { type: "ignore" } : { type: "open_sheet", sheetId: "create" };
    case "toggle_follow":
      return { type: "toggle_follow" };
    case "toggle_select":
      return { type: "toggle_select" };
    case "copy_selection":
      return { type: "copy_selection" };
    case "focus_terminal":
      return { type: "focus_terminal" };
    case "refresh":
      return { type: "refresh" };
    default:
      return { type: "ignore" };
  }
}

export function surfaceActionDispatchContextPlan(zone) {
  const directZoneType =
    zone?.type === "session" || zone?.type === "trogdor_agent" || zone?.type === "trogdor_reader";
  if (!zone || zone.disabled || directZoneType) {
    return { includeReadOnly: false, includeCurrentSession: false };
  }
  if (zone.actionId === "open_send") {
    return { includeReadOnly: true, includeCurrentSession: true };
  }
  if (zone.actionId === "open_create") {
    return { includeReadOnly: true, includeCurrentSession: false };
  }
  return { includeReadOnly: false, includeCurrentSession: false };
}

export function surfaceActionTrogdorReaderExecutionPlan(plan = {}, context = {}) {
  if (plan.type === "trogdor_read_toggle") {
    const toggle = context.toggle || {};
    const statePatch = {};
    if (Object.prototype.hasOwnProperty.call(toggle, "reading") && toggle.reading !== null) {
      statePatch.trogdorReading = toggle.reading;
    }
    return {
      type: "apply_trogdor_reader",
      session: toggle.session || null,
      readAgain: toggle.readAgain,
      statePatch,
      restartClock: Boolean(toggle.restartClock),
      resetAfterWpmChange: false,
      syncReaderTimer: true,
    };
  }
  if (plan.type === "trogdor_wpm") {
    return {
      type: "apply_trogdor_reader",
      session: null,
      readAgain: false,
      statePatch: { trogdorWpm: context.nextWpm },
      restartClock: false,
      resetAfterWpmChange: true,
      syncReaderTimer: false,
    };
  }
  return { type: "ignore" };
}

export function surfaceActionExecutionContextPlan(plan = {}) {
  switch (plan.type) {
    case "open_send_sheet_for_zone":
    case "open_create_sheet_for_zone_cwd":
    case "select_then_open_mermaid_for_zone":
    case "select_then_launch_commit_for_zone":
      return { includeZonePayload: true };
    default:
      return { includeZonePayload: false };
  }
}

export function surfaceActionExecutionPlan(plan = {}, context = {}) {
  switch (plan.type) {
    case "open_send_sheet_for_zone":
      return { type: "open_send_sheet", payload: context.zonePayload };
    case "open_create_sheet_for_zone_cwd":
      return { type: "open_create_sheet_for_cwd", cwd: context.zonePayload?.cwd };
    case "select_then_open_mermaid_for_zone":
      return { type: "select_then_open_mermaid", sessionId: context.zonePayload?.sessionId };
    case "select_then_launch_commit_for_zone":
      return { type: "select_then_launch_commit", sessionId: context.zonePayload?.sessionId };
    case "open_sheet":
      return { type: "open_sheet", sheetId: plan.sheetId };
    case "open_send_sheet_for_current_session":
      return { type: "open_send_sheet", payload: plan.payload };
    case "open_thought_config":
    case "open_native":
    case "open_mermaid":
    case "launch_commit":
    case "toggle_follow":
    case "toggle_select":
    case "copy_selection":
    case "refresh":
      return { type: plan.type };
    default:
      return { type: "ignore" };
  }
}

export function surfaceActionFocusTerminalExecutionPlan(status = {}) {
  return {
    type: "focus_terminal",
    atlasTransitionAction: "close",
    focusOptions: { preventScroll: true },
    statusMessage: status.message,
    statusError: status.error,
    statusTimeoutMs: status.timeoutMs,
  };
}
