export function runGlobalShortcutAction(plan, runtime) {
  switch (plan.type) {
    case "open_palette":
      runtime.openCommandPalette();
      break;
    case "zoom_in":
      runtime.setTerminalZoom(runtime.state.terminalZoom + runtime.terminalZoomStep, { announce: true });
      break;
    case "zoom_out":
      runtime.setTerminalZoom(runtime.state.terminalZoom - runtime.terminalZoomStep, { announce: true });
      break;
    case "zoom_reset":
      runtime.setTerminalZoom(1, { announce: true });
      break;
    case "close_sheets":
      runtime.closeSheets();
      break;
    case "close_trogdor_atlas":
      Object.assign(runtime.state, runtime.trogdorAtlasTransitionState("close"));
      runtime.renderHudSurface();
      // The close transition resets hover/reader state; sync stops the speed
      // reader's interval so it does not keep rendering a hidden surface.
      runtime.syncTrogdorReaderTimer();
      break;
    case "toggle_trogdor_atlas":
      // Mirrors toggleTrogdorAtlasSurfaceAction: open when closed, close (and
      // reset hover/reader state) when open, then sync the reader interval.
      Object.assign(
        runtime.state,
        runtime.trogdorAtlasTransitionState("toggle", runtime.state.trogdorAtlasOpen),
      );
      runtime.renderHudSurface();
      runtime.syncTrogdorReaderTimer();
      break;
    case "next_attention":
      runtime.selectNextAttentionSession?.();
      break;
    case "exit_select_mode":
      runtime.setSelectMode(false);
      break;
    case "open_sheet":
      runtime.openSheet(plan.sheetId);
      break;
    case "open_thought_config":
      runtime.openThoughtConfigSheet();
      break;
    case "open_native":
      runtime.openNativeSheet();
      break;
    case "open_mermaid":
      runtime.openMermaidSheet();
      break;
    case "toggle_follow":
      void runtime.toggleFollowPublished();
      break;
    case "toggle_select":
      runtime.setSelectMode(!runtime.state.selectMode);
      break;
    case "copy_selection":
      void runtime.copyTerminalSelection();
      break;
    case "copy_hovered_link":
      void runtime.copyHoveredLink();
      break;
    case "refresh_sessions":
      void runtime.refreshSessions();
      break;
    default:
      break;
  }
}
