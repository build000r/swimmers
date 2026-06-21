import {
  COLORS,
  STYLE_BOLD,
  STYLE_ITALIC,
  STYLE_UNDERLINE,
  actionSpec,
  cellInRect,
  chipSpec,
  clampInt,
  computeSurfaceLayout,
  drawActionChipRow,
  drawBorder,
  drawChip,
  drawPanel,
  drawText,
  drawTextBlock,
  drawTextCenter,
  drawTextRight,
  expandedRect,
  fillRect,
  pushMask,
  pushZone,
  rect,
  safeLabel,
  truncate,
} from "./rendered_surface_draw.js";

export function buildSurfaceFrame(model, reuse = null) {
  const cols = clampInt(model?.cols, 80, 32, 240);
  const rows = clampInt(model?.rows, 24, 16, 120);
  const cellCount = cols * rows * 4;
  // Reuse the caller's buffers when the grid dimensions are unchanged. This
  // render path runs on 40+ triggers (hover, byte feed, status/mode/search), so
  // a fresh Uint32Array(cols*rows*4) — up to ~460 KB at the 240x120 max — every
  // call is steady GC churn. Zeroing and refilling is byte-identical to a fresh
  // zero-initialized allocation, so the rendered frame is unchanged.
  let cells;
  let spans;
  if (reuse?.cells instanceof Uint32Array && reuse.cells.length === cellCount) {
    cells = reuse.cells;
    cells.fill(0);
    spans =
      reuse.spans instanceof Uint32Array && reuse.spans.length === 2
        ? reuse.spans
        : new Uint32Array(2);
    spans[0] = 0;
    spans[1] = cols * rows;
  } else {
    cells = new Uint32Array(cellCount);
    spans = new Uint32Array([0, cols * rows]);
  }
  const frame = {
    cols,
    rows,
    cells,
    spans,
    zones: [],
    masks: [],
  };

  const layout = computeSurfaceLayout(cols, rows, Boolean(model?.focusLayout));
  drawHeader(frame, layout, model);
  if (layout.sessionRail) {
    drawSessionRail(frame, layout.sessionRail, model);
  }
  if (layout.detailRail) {
    drawDetailRail(frame, layout.detailRail, model);
  }
  drawTerminalViewport(frame, layout.center, model);
  drawFooter(frame, layout.footer, model);
  drawCenterOverlay(frame, layout.center, model);
  frame.layout = layout;
  return frame;
}

// Compute the flat [startCell, endCell, ...] spans of cells that differ between
// the freshly built frame and the last uploaded frame, so the surface only
// re-uploads changed cells instead of the whole grid on every render. Returns
// the full-grid span when there is no comparable baseline (first frame or a
// resize), and an empty span set when nothing changed. Each cell is 4 uint32s.
export function computeSurfaceDirtySpans(currentCells, previousCells, cols, rows) {
  const cellCount = Math.max(0, Math.trunc(cols) * Math.trunc(rows));
  if (
    !(currentCells instanceof Uint32Array) ||
    !(previousCells instanceof Uint32Array) ||
    previousCells.length !== currentCells.length ||
    currentCells.length < cellCount * 4
  ) {
    return new Uint32Array([0, cellCount]);
  }
  const spans = [];
  let runStart = -1;
  for (let cell = 0; cell < cellCount; cell += 1) {
    const base = cell * 4;
    const changed =
      currentCells[base] !== previousCells[base] ||
      currentCells[base + 1] !== previousCells[base + 1] ||
      currentCells[base + 2] !== previousCells[base + 2] ||
      currentCells[base + 3] !== previousCells[base + 3];
    if (changed) {
      if (runStart < 0) {
        runStart = cell;
      }
    } else if (runStart >= 0) {
      spans.push(runStart, cell);
      runStart = -1;
    }
  }
  if (runStart >= 0) {
    spans.push(runStart, cellCount);
  }
  return new Uint32Array(spans);
}

export function surfaceActionAt(zones, cell) {
  if (!Array.isArray(zones) || !cell) {
    return null;
  }
  for (let index = zones.length - 1; index >= 0; index -= 1) {
    const zone = zones[index];
    if (zone && cellInRect(cell, zone.rect)) {
      return zone;
    }
  }
  return null;
}

export function surfaceConsumesPointer(masks, cell) {
  if (!Array.isArray(masks) || !cell) {
    return false;
  }
  return masks.some((rect) => cellInRect(cell, rect));
}

function drawHeader(frame, layout, model) {
  drawPanel(frame, layout.header, "swimmers");
  pushMask(frame, layout.header);

  const title = model?.focusLayout
    ? "tailnet control surface / published selection"
    : "tailnet control surface / rendered web shell";
  drawText(frame, layout.header.x + 2, layout.header.y + 1, title, {
    fg: COLORS.accent,
    attrs: STYLE_BOLD,
    width: layout.header.w - 4,
  });

  const chips = [
    chipSpec(`conn ${safeLabel(model?.connectionLabel, "idle")}`, model?.connectionMuted ? COLORS.muted : COLORS.text, COLORS.chipBg),
    chipSpec(`mode ${safeLabel(model?.modeLabel, "unknown")}`, model?.modeMuted ? COLORS.muted : COLORS.text, COLORS.chipBg),
    chipSpec(
      `term ${terminalStateCopy(model).shortLabel}`,
      terminalStateCopy(model).muted ? COLORS.muted : COLORS.text,
      terminalStateCopy(model).muted ? COLORS.chipBg : COLORS.chipActiveBg,
      {
        type: "action",
        actionId: "focus_terminal",
        disabled: !model?.currentSession,
      },
    ),
    chipSpec(
      model?.sessionGroupMode === "project" ? "view grouped" : "view flat",
      model?.sessionGroupMode === "project" ? COLORS.success : COLORS.text,
      model?.sessionGroupMode === "project" ? COLORS.chipActiveBg : COLORS.chipBg,
      {
        type: "action",
        actionId: "toggle_session_grouping",
      },
    ),
    chipSpec(
      model?.followPublishedSelection ? "published follow on" : "published follow off",
      model?.followPublishedSelection ? COLORS.success : COLORS.muted,
      model?.followPublishedSelection ? COLORS.chipActiveBg : COLORS.chipBg,
      {
        type: "action",
        actionId: "toggle_follow",
      },
    ),
    chipSpec("auth", COLORS.text, COLORS.chipBg, {
      type: "action",
      actionId: "open_auth",
    }),
    chipSpec("refresh", COLORS.text, COLORS.chipBg, {
      type: "action",
      actionId: "refresh",
    }),
    ...(Array.isArray(model?.fleetPresetChips) ? model.fleetPresetChips.map((chip) => chipSpec(
      safeLabel(chip.label, "lens"),
      chip.active ? COLORS.success : COLORS.muted,
      chip.active ? COLORS.chipActiveBg : COLORS.chipBg,
      {
        type: "action",
        actionId: "fleet_preset",
        presetId: chip.presetId,
      },
    )) : []),
    ...(Array.isArray(model?.fleetChips) ? model.fleetChips.map((chip) => chipSpec(
      safeLabel(chip.label, "filter"),
      chip.active ? COLORS.success : COLORS.text,
      chip.active ? COLORS.chipActiveBg : COLORS.chipBg,
      {
        type: "action",
        actionId: "fleet_filter",
        kind: chip.kind,
        key: chip.key,
      },
    )) : []),
  ];

  let cursorX = layout.header.x + 2;
  const chipY = layout.header.y + 2;
  const secondaryReserve = Math.max(16, Math.floor(layout.header.w * 0.42)) + 2;
  const chipLimit = layout.header.x + layout.header.w - 2 - secondaryReserve;
  for (const chip of chips) {
    const width = chip.label.length + 4;
    if (cursorX + width > chipLimit) {
      break;
    }
    drawChip(frame, cursorX, chipY, chip.label, chip.fg, chip.bg);
    if (chip.zone) {
      pushZone(frame, chip.zone, expandedRect(frame, cursorX, chipY, width, 1, 0, 1));
    }
    cursorX += width + 1;
  }

  const selected = model?.currentSession?.name || (model?.followPublishedSelection ? "waiting for published session" : "no session selected");
  drawTextRight(frame, layout.header.x + layout.header.w - 2, layout.header.y + 1, selected, {
    fg: COLORS.text,
    attrs: STYLE_BOLD,
    width: Math.max(12, Math.floor(layout.header.w * 0.38)),
  });

  const secondary = model?.currentSession
    ? [stateDisplayLabel(model.currentSession), model.currentSession.toolLabel, model.currentSession.cwdLabel].filter(Boolean).join(" / ")
    : model?.frankenTermAvailable
      ? "rendered surface ready"
      : "FrankenTerm assets unavailable";
  drawTextRight(frame, layout.header.x + layout.header.w - 2, layout.header.y + 2, secondary, {
    fg: COLORS.muted,
    width: Math.max(16, Math.floor(layout.header.w * 0.42)),
  });
}

function drawSessionRail(frame, rail, model) {
  const grouped = model?.sessionGroupMode === "project";
  drawPanel(frame, rail, sessionRailTitle(model, grouped));
  pushMask(frame, rail);

  const sessions = Array.isArray(model?.sessions) ? model.sessions : [];
  if (!sessions.length && !model?.trogdorAtlasOpen) {
    const message = model?.fleetEmptyMessage || "No live sessions.\nCreate one from the rendered action rail.";
    drawTextBlock(frame, rail.x + 2, rail.y + 2, rail.w - 4, 4, message, {
      fg: COLORS.muted,
    });
    return;
  }

  const rows = Array.isArray(model?.sessionRailRows) && model.sessionRailRows.length
    ? model.sessionRailRows
    : sessions.map((session) => ({ type: "session", session }));
  const entryHeight = grouped ? 5 : 4;
  const availableRows = Math.max(1, rail.h - 3);
  const visibleCount = Math.max(1, Math.floor(availableRows / entryHeight));
  const selectedIndex = Math.max(0, rows.findIndex((row) => row.session?.sessionId === model?.selectedSessionId));
  const start = clampInt(selectedIndex - Math.floor(visibleCount / 2), 0, 0, Math.max(0, rows.length - visibleCount));
  const end = Math.min(rows.length, start + visibleCount);

  let y = rail.y + 1;
  for (let index = start; index < end; index += 1) {
    const row = rows[index];
    const session = row.session;
    if (!session) continue;
    const isSelected = session.sessionId === model?.selectedSessionId;
    const isPublished = session.sessionId === model?.publishedSessionId;
    const bg = isSelected ? COLORS.sessionActiveBg : isPublished ? COLORS.sessionPublishedBg : COLORS.transparent;
    const fg = isSelected ? COLORS.text : COLORS.text;
    fillRect(frame, rail.x + 1, y, rail.w - 2, entryHeight - 1, bg);
    if (grouped) {
      const groupPrefix = row.group?.first ? "v " : "  ";
      const groupLabel = row.group
        ? `${groupPrefix}${row.group.count}x ${shortProjectLabel(row.group.label)} ${compactHostSummary(row.group.hostSummary)}`
        : `${groupPrefix}${session.repoLabel || session.cwdLabel}`;
      drawText(frame, rail.x + 2, y, truncate(groupLabel, rail.w - 6), {
        fg: row.group?.first ? COLORS.accent : COLORS.muted,
        attrs: row.group?.first ? STYLE_BOLD : 0,
        width: rail.w - 4,
      });
    }
    const nameY = grouped ? y + 1 : y;
    drawText(frame, rail.x + 2, nameY, truncate(session.name, rail.w - 6), {
      fg: isSelected ? COLORS.success : fg,
      attrs: STYLE_BOLD,
      width: rail.w - 4,
    });
    drawText(frame, rail.x + 2, nameY + 1, truncate(`${stateDisplayLabel(session)} / ${session.toolLabel}`, rail.w - 6), {
      fg: stateEvidenceIsUnverified(session) || session.transportLabel !== "healthy" ? COLORS.warning : COLORS.muted,
      width: rail.w - 4,
    });
    drawText(frame, rail.x + 2, nameY + 2, truncate(`${session.cwdLabel} :: ${session.thoughtLabel}`, rail.w - 6), {
      fg: COLORS.muted,
      width: rail.w - 4,
    });
    pushZone(
      frame,
      {
        type: "session",
        sessionId: session.sessionId,
      },
      rect(rail.x + 1, y, rail.w - 2, entryHeight - 1),
    );
    y += entryHeight;
  }

  if (end < rows.length) {
    drawText(frame, rail.x + 2, rail.y + rail.h - 2, truncate(`more ${rows.length - end} below`, rail.w - 4), {
      fg: COLORS.muted,
      attrs: STYLE_ITALIC,
      width: rail.w - 4,
    });
  }
}

function sessionRailTitle(model, grouped) {
  if (model?.focusLayout) {
    return "published";
  }
  const base = grouped ? "sessions grouped" : "sessions";
  const count = Number(model?.attentionInboxCount || 0);
  return count > 0 ? `${base} / inbox ${count}` : base;
}

function shortProjectLabel(label) {
  const parts = String(label || "").split("/").filter(Boolean);
  return parts[parts.length - 1] || String(label || "project");
}

function compactHostSummary(summary) {
  return String(summary || "")
    .split(" + ")
    .map((label) => {
      const trimmed = label.trim();
      if (!trimmed) return "";
      if (trimmed.toLowerCase() === "local") return "L";
      return trimmed.split(/\s+/).find(Boolean) || trimmed;
    })
    .filter(Boolean)
    .join("+");
}

function drawDetailRail(frame, rail, model) {
  const session = model?.currentSession;
  drawPanel(frame, rail, session ? "details" : "status");
  pushMask(frame, rail);

  if (!session) {
    const message = model?.followPublishedSelection
      ? "Waiting for the native TUI to publish a session."
      : "Select a session from the rendered rail to attach.";
    drawTextBlock(frame, rail.x + 2, rail.y + 2, rail.w - 4, 5, message, {
      fg: COLORS.muted,
    });
    return;
  }

  const lines = [
    ["state", stateDisplayLabel(session)],
    ["evidence", session.stateTrustLabel],
    ["rest", session.restLabel],
    ["transport", session.transportLabel],
    ["tool", session.toolLabel],
    ["context", session.contextLabel],
    ["skill", session.skillLabel],
    ["clients", session.attachedLabel],
    ["activity", session.activityLabel],
    ["command", session.commandLabel],
  ];

  if (model?.followPublishedSelection && model?.publishedAtLabel) {
    lines.push(["published", model.publishedAtLabel]);
  }
  if (session.commitCandidate) {
    lines.push(["commit", "candidate"]);
  }
  if (session.advisoryLabel) {
    lines.push(["advisory", session.advisoryLabel]);
  }

  let y = rail.y + 1;
  drawText(frame, rail.x + 2, y, truncate(session.name, rail.w - 4), {
    fg: COLORS.accent,
    attrs: STYLE_BOLD,
    width: rail.w - 4,
  });
  y += 2;

  for (const [label, value] of lines) {
    if (y >= rail.y + rail.h - 5) {
      break;
    }
    drawText(frame, rail.x + 2, y, truncate(label, 10), {
      fg: COLORS.muted,
      attrs: STYLE_UNDERLINE,
      width: 10,
    });
    drawText(frame, rail.x + 13, y, truncate(value || "-", rail.w - 15), {
      fg: COLORS.text,
      width: rail.w - 15,
    });
    y += 1;
  }

  if (session) {
    const actions = [
      actionSpec("mmd", "mmd", "open_mermaid", true),
      actionSpec("commit", "commit", "launch_commit", session.commitCandidate),
      actionSpec("native", "native", "open_native", true),
    ];
    const actionY = Math.min(rail.y + rail.h - 4, y + 1);
    drawText(frame, rail.x + 2, actionY, "actions", {
      fg: COLORS.muted,
      attrs: STYLE_UNDERLINE,
      width: rail.w - 4,
    });
    drawActionChipRow(frame, rail.x + 2, actionY + 1, rail.x + rail.w - 2, actions, "action", { hitPadY: 1 });
    y = Math.max(y, actionY + 3);
  }

  y += 1;
  drawText(frame, rail.x + 2, y, "thought", {
    fg: COLORS.muted,
    attrs: STYLE_UNDERLINE,
  });
  drawTextBlock(frame, rail.x + 2, y + 1, rail.w - 4, Math.max(2, rail.y + rail.h - y - 2), session.thoughtLabel, {
    fg: COLORS.text,
  });
}

function drawFooter(frame, footer, model) {
  drawPanel(frame, footer, model?.activeSheet ? `sheet ${model.activeSheet}` : "actions");
  pushMask(frame, footer);

  const search = `search ${safeLabel(model?.searchLabel, "idle")}`;
  const utility = safeLabel(model?.utilityLabel, "click a rendered action to begin");
  drawText(frame, footer.x + 2, footer.y + 1, truncate(search, footer.w - 4), {
    fg: model?.searchMuted ? COLORS.muted : COLORS.text,
    width: footer.w - 4,
  });
  if (footer.h >= 5) {
    drawText(frame, footer.x + 2, footer.y + 2, truncate(utility, footer.w - 4), {
      fg: model?.utilityMuted ? COLORS.muted : COLORS.text,
      width: footer.w - 4,
    });
  }

  const actionY = footer.h >= 5 ? footer.y + 3 : footer.y + 2;
  const actions = [
    actionSpec("terminal", "terminal", "focus_terminal", Boolean(model?.currentSession)),
    actionSpec(model?.trogdorAtlasOpen ? "trogdor on" : "trogdor", "trogdor", "toggle_trogdor_atlas", true),
    actionSpec("new", "new", "open_create", !model?.readOnly),
    actionSpec("config", "config", "open_config", true),
    actionSpec("native", "native", "open_native", true),
    actionSpec("auth", "auth", "open_auth", true),
    actionSpec("refresh", "refresh", "refresh", true),
    actionSpec("search", "search", "open_search", Boolean(model?.terminalReady)),
    actionSpec("send", "send", "open_send", Boolean(model?.currentSession) && !model?.readOnly),
    actionSpec("mmd", "mmd", "open_mermaid", Boolean(model?.currentSession)),
    actionSpec("commit", "commit", "launch_commit", Boolean(model?.currentSession?.commitCandidate)),
    actionSpec(model?.followPublishedSelection ? "following" : "follow", "follow", "toggle_follow", true),
    actionSpec(model?.selectMode ? "select on" : "select", "select", "toggle_select", Boolean(model?.terminalReady)),
    actionSpec("copy", "copy", "copy_selection", Boolean(model?.terminalReady)),
  ];

  let x = footer.x + 2;
  const limit = footer.x + footer.w - 2;
  drawActionChipRow(frame, x, actionY, limit, actions, "action", { hitPadY: 1 });
}

function drawTerminalViewport(frame, center, model) {
  const status = terminalStateCopy(model);
  drawBorder(frame, center, model?.terminalReady ? COLORS.accentSoft : COLORS.panelBorderSoft);
  drawText(frame, center.x + 2, center.y, ` ${status.label} `, {
    fg: status.muted ? COLORS.muted : COLORS.accent,
    bg: COLORS.panelBgStrong,
    attrs: STYLE_BOLD,
    width: Math.max(0, center.w - 4),
  });
}

function drawCenterOverlay(frame, center, model) {
  if (model?.currentSession && !model?.trogdorAtlasOpen) {
    const messages = [];
    if (!model?.frankenTermAvailable) {
      messages.push("Snapshot fallback active. FrankenTerm assets are unavailable on this host.");
    } else if (!model?.terminalReady) {
      messages.push(`Attaching terminal to ${model.currentSession.name}...`);
    }

    if (!messages.length) {
      return;
    }

    const width = Math.min(center.w - 4, Math.max(28, Math.floor(center.w * 0.72)));
    const height = Math.min(center.h - 2, 6);
    const x = center.x + Math.max(0, Math.floor((center.w - width) / 2));
    const y = center.y + Math.max(1, Math.floor((center.h - height) / 2));
    const overlay = rect(x, y, width, height);
    drawPanel(frame, overlay, "status", {
      bg: COLORS.overlayBg,
      border: COLORS.panelBorderSoft,
    });
    pushMask(frame, overlay);
    drawTextBlock(frame, overlay.x + 2, overlay.y + 2, overlay.w - 4, overlay.h - 3, messages.join(" "), {
      fg: COLORS.text,
    });
    return;
  }

  const sessions = Array.isArray(model?.sessions) ? model.sessions : [];
  if (!sessions.length && !model?.trogdorAtlasOpen) {
    const width = Math.min(center.w - 4, Math.max(28, Math.floor(center.w * 0.72)));
    const height = Math.min(center.h - 2, 6);
    const x = center.x + Math.max(0, Math.floor((center.w - width) / 2));
    const y = center.y + Math.max(1, Math.floor((center.h - height) / 2));
    const overlay = rect(x, y, width, height);
    drawPanel(frame, overlay, "status", {
      bg: COLORS.overlayBg,
      border: COLORS.panelBorderSoft,
    });
    pushMask(frame, overlay);
    drawTextBlock(frame, overlay.x + 2, overlay.y + 2, overlay.w - 4, overlay.h - 3, model?.fleetEmptyMessage || "No live sessions. Create one from the rendered action rail.", {
      fg: COLORS.text,
    });
    return;
  }

  const summary = sessions.length
    ? summarizeSessions(sessions, model?.publishedSessionId, model?.selectedSessionId)
    : emptyTrogdorSummary();
  const overlay = rect(
    center.x + 1,
    center.y + 1,
    Math.max(20, center.w - 2),
    Math.max(8, center.h - 2),
  );
  drawPanel(frame, overlay, "overview", {
    bg: COLORS.overlayBg,
    border: COLORS.panelBorderSoft,
  });
  pushMask(frame, overlay);
  const bannerWidth = Math.min(26, Math.max(14, Math.floor(overlay.w * 0.34)));
  const bannerX = overlay.x + Math.max(2, Math.floor((overlay.w - bannerWidth) / 2));
  fillRect(frame, bannerX, overlay.y + 1, bannerWidth, 1, COLORS.chipDangerBg);
  drawTextCenter(frame, bannerX, overlay.y + 1, bannerWidth, "burninate!", {
    fg: COLORS.parchment,
    bg: COLORS.chipDangerBg,
    attrs: STYLE_BOLD,
  });
  drawText(frame, overlay.x + 2, overlay.y + 2, "trogdor pressure / repo atlas", {
    fg: COLORS.accent,
    attrs: STYLE_BOLD,
    width: overlay.w - 4,
  });
  drawText(frame, overlay.x + 2, overlay.y + 3, truncate(summary.subtitle, overlay.w - 4), {
    fg: COLORS.muted,
    width: overlay.w - 4,
  });

  const environmentRowsUsed = drawEnvironmentMatrix(frame, overlay, model, overlay.y + 4);
  drawTrogdorPressureAtlas(frame, overlay, sessions, model, summary, environmentRowsUsed);

  drawText(frame, overlay.x + 2, overlay.y + overlay.h - 2, truncate(summary.footer, overlay.w - 4), {
    fg: COLORS.muted,
    attrs: STYLE_ITALIC,
    width: overlay.w - 4,
  });
}

function drawEnvironmentMatrix(frame, overlay, model, startY) {
  const rows = Array.isArray(model?.environmentMatrix) ? model.environmentMatrix : [];
  if (!rows.length || overlay.h < 12) {
    return 0;
  }
  const handoffCount = rows.filter((row) => row.handoffOnly).length;
  const degradedCount = rows.filter((row) => environmentRowIsDegraded(row)).length;
  const header = `envs ${rows.length} / handoff ${handoffCount} / degraded ${degradedCount}`;
  drawText(frame, overlay.x + 2, startY, truncate(header, overlay.w - 4), {
    fg: degradedCount ? COLORS.warning : COLORS.muted,
    width: overlay.w - 4,
  });

  const maxRows = overlay.h >= 20 ? 3 : 2;
  const visibleRows = rows.slice(0, maxRows);
  let y = startY + 1;
  for (const row of visibleRows) {
    const label = environmentRowLabel(row);
    const fg = environmentRowColor(row);
    drawText(frame, overlay.x + 2, y, truncate(label, overlay.w - 4), {
      fg,
      width: overlay.w - 4,
    });
    pushZone(
      frame,
      {
        type: "environment",
        actionId: "fleet_filter",
        kind: "target",
        key: row.id,
        label: row.displayHost || row.label || row.id,
      },
      rect(overlay.x + 1, y, overlay.w - 2, 1),
    );
    y += 1;
    for (const hint of environmentRowHintLines(row)) {
      if (y >= overlay.y + overlay.h - 2) {
        break;
      }
      drawText(frame, overlay.x + 4, y, truncate(hint.label, overlay.w - 6), {
        fg: hint.kind === "error" ? COLORS.danger : COLORS.text,
        width: overlay.w - 6,
      });
      if (hint.copyText) {
        pushZone(
          frame,
          {
            type: "environment_hint",
            actionId: "copy_environment_hint",
            kind: hint.kind,
            key: row.id,
            label: hint.label,
            copyText: hint.copyText,
          },
          rect(overlay.x + 3, y, overlay.w - 4, 1),
        );
      }
      y += 1;
    }
  }
  if (rows.length > visibleRows.length) {
    drawText(frame, overlay.x + 2, y, truncate(`more envs ${rows.length - visibleRows.length}`, overlay.w - 4), {
      fg: COLORS.muted,
      attrs: STYLE_ITALIC,
      width: overlay.w - 4,
    });
    y += 1;
  }
  return y - startY;
}

function environmentRowLabel(row) {
  const host = safeLabel(row?.displayHost || row?.label || row?.id, "environment");
  const count = Number(row?.sessionCount || 0);
  const readiness = safeLabel(row?.readinessLabel, "unknown");
  const caps = Array.isArray(row?.capabilityLabels) && row.capabilityLabels.length
    ? ` ${row.capabilityLabels.slice(0, 3).join("/")}`
    : "";
  const maps = Number(row?.pathMappingCount || 0) > 0 ? ` maps ${row.pathMappingCount}` : "";
  return `${host} ${count} ${readiness}${caps}${maps}`;
}

function environmentRowHintLines(row) {
  const lines = [];
  const error = String(row?.lastError || "").trim();
  if (error) {
    lines.push({ kind: "error", label: `health ${error}` });
  }
  const attach = String(row?.attachHint || "").trim();
  if (attach) {
    lines.push({ kind: "attach", label: `attach ${attach}`, copyText: attach });
  }
  const bootstrap = String(row?.bootstrapHint || "").trim();
  if (bootstrap) {
    lines.push({ kind: "bootstrap", label: `bootstrap ${bootstrap}`, copyText: bootstrap });
  }
  return lines;
}

function environmentRowColor(row) {
  if (environmentRowIsDegraded(row)) {
    return COLORS.danger;
  }
  switch (String(row?.readinessKey || "").toLowerCase()) {
    case "needs_attention":
      return COLORS.warning;
    case "ready":
      return COLORS.success;
    case "handoff":
      return COLORS.warning;
    default:
      return COLORS.muted;
  }
}

function environmentRowIsDegraded(row) {
  const status = String(row?.status || "").toLowerCase();
  const readiness = String(row?.readinessKey || "").toLowerCase();
  return Boolean(
    row?.degradedCount > 0 ||
      status === "degraded" ||
      status === "unavailable" ||
      readiness === "degraded" ||
      readiness === "blocked",
  );
}

function drawTrogdorPressureAtlas(frame, overlay, sessions, model, summary, environmentRowsUsed = 0) {
  const repoGroups = buildTrogdorRepoGroups(sessions);
  const atlasTop = overlay.y + 5 + Math.max(0, environmentRowsUsed);
  const atlasBottom = overlay.y + overlay.h - 3;
  const readerWidth = overlay.w >= 62 ? Math.min(40, Math.max(34, Math.floor(overlay.w * 0.44))) : 0;
  const atlasWidth = overlay.w - 4 - (readerWidth ? readerWidth + 1 : 0);
  const atlas = rect(overlay.x + 2, atlasTop, Math.max(22, atlasWidth), Math.max(4, atlasBottom - atlasTop));

  drawText(frame, atlas.x, atlas.y, truncate(`cues ${summary.actionCues} / agents ${sessions.length} / level ${summary.pressure}`, atlas.w), {
    fg: COLORS.warning,
    attrs: STYLE_BOLD,
    width: atlas.w,
  });

  const rowHeight = 4;
  const visibleGroups = repoGroups.slice(0, Math.max(1, Math.floor((atlas.h - 2) / rowHeight)));
  let y = atlas.y + 2;
  if (!repoGroups.length) {
    drawText(frame, atlas.x, y, truncate("      T>", atlas.w), {
      fg: COLORS.ember,
      attrs: STYLE_BOLD,
      width: atlas.w,
    });
    drawText(frame, atlas.x, y + 1, truncate("    _/\\\\_      no repos", atlas.w), {
      fg: COLORS.parchment,
      width: atlas.w,
    });
    drawText(frame, atlas.x, y + 2, truncate("   /_  _/      launch agent", atlas.w), {
      fg: COLORS.muted,
      width: atlas.w,
    });
  }
  for (const group of visibleGroups) {
    drawTrogdorRepoRow(frame, atlas.x, y, atlas.w, group, model);
    y += rowHeight;
  }

  if (repoGroups.length > visibleGroups.length) {
    drawText(frame, atlas.x, atlas.y + atlas.h - 1, truncate(`more repos ${repoGroups.length - visibleGroups.length}`, atlas.w), {
      fg: COLORS.muted,
      attrs: STYLE_ITALIC,
      width: atlas.w,
    });
  }

  const hovered =
    sessions.find((session) => session.sessionId === model?.hoveredTrogdorSessionId && sessionCanReadClawgs(session)) ||
    null;
  const readerHeight = Math.min(15, atlas.h);
  if (hovered && readerWidth && readerHeight >= 14) {
    const reader = rect(overlay.x + overlay.w - readerWidth - 2, atlasTop, readerWidth, readerHeight);
    drawTrogdorSpeedReader(frame, reader, hovered, model);
  }
}

function drawTrogdorRepoRow(frame, x, y, width, group, model) {
  const pressure = group.pressure;
  const accent = pressure >= 70 ? COLORS.danger : pressure >= 35 ? COLORS.warning : COLORS.success;
  const labelWidth = Math.max(10, Math.min(18, Math.floor(width * 0.32)));
  const roof = pressure >= 70 ? "/!\\ " : pressure >= 35 ? "/^\\ " : "/_\\ ";
  drawText(frame, x, y, `${roof}${truncate(group.label, labelWidth)}`, {
    fg: COLORS.parchment,
    attrs: STYLE_BOLD,
    width: Math.min(width, labelWidth + 4),
  });
  const hostSuffix = group.hostSummary ? ` @ ${group.hostSummary}` : "";
  drawText(frame, x, y + 1, truncate(`|##| pressure ${pressure} ${group.reason}${hostSuffix}`, width), {
    fg: accent,
    width,
  });
  drawText(frame, x, y + 2, pressure >= 70 ? " burninating" : " structure", {
    fg: pressure >= 70 ? COLORS.ember : COLORS.muted,
    width: Math.min(width, labelWidth + 4),
  });

  let agentX = x + Math.min(width - 2, labelWidth + 6);
  const swordsmen = group.sessions.filter(sessionSwordsmanVisible);
  for (const session of swordsmen.slice(0, Math.max(1, width - labelWidth - 8))) {
    const hovered = session.sessionId === model?.hoveredTrogdorSessionId;
    const glyph = hovered ? "A" : agentGlyph(session);
    const fg = hovered ? COLORS.ember : agentColor(session);
    drawText(frame, agentX, y + 2, glyph, {
      fg,
      attrs: hovered ? STYLE_BOLD : 0,
      width: 1,
    });
    if (hovered) {
      drawText(frame, agentX, y + 3, "^", {
        fg: COLORS.ember,
        attrs: STYLE_BOLD,
        width: 1,
      });
    }
    pushZone(
      frame,
      {
        type: "trogdor_agent",
        sessionId: session.sessionId,
      },
      rect(agentX, y, 1, 4),
    );
    agentX += 2;
    if (agentX >= x + width - 1) {
      break;
    }
  }
}

function drawTrogdorSpeedReader(frame, panel, session, model) {
  drawPanel(frame, panel, "speed read agent", {
    bg: COLORS.ink,
    border: COLORS.ember,
  });
  pushMask(frame, panel);
  pushZone(
    frame,
    {
      type: "trogdor_reader",
      sessionId: session.sessionId,
    },
    panel,
  );

  const wpm = clampInt(model?.trogdorWpm, 200, 50, 800);
  const word = speedReadWord(
    session,
    wpm,
    model?.trogdorReaderElapsedMs,
    model?.trogdorReading !== false,
    model?.trogdorReaderStartIndex,
  );
  drawTextCenter(frame, panel.x + 1, panel.y + 2, panel.w - 2, word, {
    fg: COLORS.parchment,
    attrs: STYLE_BOLD,
  });
  drawTextCenter(frame, panel.x + 1, panel.y + 3, panel.w - 2, `${wpm} wpm`, {
    fg: COLORS.warning,
  });
  drawText(frame, panel.x + 2, panel.y + 5, truncate(`${session.name} / ${stateDisplayLabel(session)}`, panel.w - 4), {
    fg: COLORS.text,
    width: panel.w - 4,
  });
  drawText(frame, panel.x + 2, panel.y + 6, truncate(`source ${primaryActionCueLabel(session) || "thought"} / ${session.restLabel}`, panel.w - 4), {
    fg: COLORS.muted,
    width: panel.w - 4,
  });
  drawText(frame, panel.x + 2, panel.y + 7, truncate(`repo ${session.repoLabel || repoLabel(session.fullCwd || session.cwdLabel)} / ${pressureReason(session)}`, panel.w - 4), {
    fg: COLORS.muted,
    width: panel.w - 4,
  });

  const readerActions = [
    actionSpec(trogdorReadButtonLabel(session, model), "read", "trogdor_read_toggle", true),
    actionSpec("-25", "slower", "trogdor_wpm_down", true),
    actionSpec("+25", "faster", "trogdor_wpm_up", true),
  ];
  drawActionChipRow(frame, panel.x + 2, panel.y + panel.h - 6, panel.x + panel.w - 2, readerActions, "action");

  const batchIds = Array.isArray(session.batchSendSessionIds) ? session.batchSendSessionIds : [];
  const primarySessionActions = [
    actionSpec("send", "send", "trogdor_send", !model?.readOnly, {
      sessionId: session.sessionId,
      label: session.name,
    }),
    actionSpec("batch", "batch", "trogdor_group_send", !model?.readOnly && batchIds.length > 1, {
      sessionIds: batchIds,
      label: `${batchIds.length} batch agents`,
    }),
    actionSpec("commit", "commit", "trogdor_commit", !model?.readOnly && sessionCommitReady(session), {
      sessionId: session.sessionId,
    }),
  ];
  const launchCwd = session.launchCwd ?? session.fullCwd;
  const secondarySessionActions = [
    actionSpec("launch", "launch", "trogdor_launch", !model?.readOnly && Boolean(launchCwd), {
      cwd: launchCwd,
      launchTarget: session.launchTarget || "",
    }),
    actionSpec("mmd", "mmd", "trogdor_mermaid", true, {
      sessionId: session.sessionId,
    }),
  ];
  drawActionChipRow(frame, panel.x + 2, panel.y + panel.h - 4, panel.x + panel.w - 2, primarySessionActions, "action");
  drawActionChipRow(frame, panel.x + 2, panel.y + panel.h - 2, panel.x + panel.w - 2, secondarySessionActions, "action");
}

function buildTrogdorRepoGroups(sessions) {
  const groups = new Map();
  for (const session of sessions) {
    const key = safeLabel(session.repoKey || session.fullCwd || session.cwdLabel, session.cwdLabel || session.name);
    const label = safeLabel(session.repoLabel, repoLabel(key));
    const existing = groups.get(key) || {
      key,
      label,
      sessions: [],
      hostSummary: "",
      pressure: 0,
      reason: "quiet",
    };
    existing.sessions.push(session);
    existing.hostSummary = hostSummaryForSessions(existing.sessions);
    const pressure = trogdorPressureScore(session);
    if (pressure > existing.pressure) {
      existing.pressure = pressure;
      existing.reason = trogdorPressureReason(session);
    }
    groups.set(key, existing);
  }
  return Array.from(groups.values()).sort((left, right) => {
    return right.pressure - left.pressure || left.label.localeCompare(right.label);
  });
}

function hostSummaryForSessions(sessions) {
  const counts = new Map();
  for (const session of sessions) {
    const label = safeLabel(session.targetLabel, "local");
    counts.set(label, (counts.get(label) || 0) + 1);
  }
  return Array.from(counts.entries())
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .map(([label, count]) => (count > 1 ? `${label} x${count}` : label))
    .join(" + ");
}

function repoLabel(value) {
  const parts = String(value || "").split("/").filter(Boolean);
  return parts[parts.length - 1] || String(value || "repo");
}

function trogdorPressureScore(session) {
  const pressure = operatorPressure(session);
  if (Number.isFinite(pressure.score)) {
    return clampInt(pressure.score, 1, 1, 99);
  }
  let score = 0;
  const state = String(session?.state || "").toLowerCase();
  const rest = String(session?.restLabel || "").toLowerCase();
  const transport = String(session?.transportLabel || "").toLowerCase();
  if (hasActionCue(session, "awaiting_user")) score += 55;
  if (hasActionCue(session, "commit_ready")) score += 45;
  if (hasActionCue(session, "validation_missing_after_edit")) score += 40;
  if (hasActionCue(session, "dirty_check_missing")) score += 35;
  if (state === "attention") score += 45;
  if (state === "busy") score += 12;
  if (state === "error") score += 55;
  if (rest === "sleeping") score += 35;
  if (rest === "deep_sleep") score += 20;
  if (session?.commitCandidate) score += 25;
  if (stateEvidenceIsUnverified(session)) score += 15;
  if (session?.isStale) score += 10;
  if (transport && transport !== "healthy") score += 20;
  return Math.max(1, Math.min(99, score));
}

function trogdorPressureReason(session) {
  const reason = pressureReason(session);
  if (reason) return reason;
  const cue = primaryActionCueKind(session);
  if (cue) return actionCueLabel(cue);
  const state = String(session?.state || "").toLowerCase();
  const rest = String(session?.restLabel || "").toLowerCase();
  if (state === "attention") return "needs input";
  if (state === "error") return "error";
  if (session?.commitCandidate) return "commit ready";
  if (rest === "deep_sleep") return "deep sleep";
  if (rest === "sleeping") return "sleeping";
  if (stateEvidenceIsUnverified(session)) return "untrusted";
  return state || "idle";
}

function agentGlyph(session) {
  const glyph = operatorPressure(session).glyph;
  if (glyph) return truncate(glyph, 1);
  const state = String(session?.state || "").toLowerCase();
  const rest = String(session?.restLabel || "").toLowerCase();
  if (hasActionCue(session, "awaiting_user")) return "!";
  if (hasActionCue(session, "commit_ready")) return "$";
  if (hasActionCue(session, "validation_missing_after_edit")) return "v";
  if (hasActionCue(session, "dirty_check_missing")) return "d";
  if (state === "attention") return "!";
  if (state === "error") return "x";
  if (session?.commitCandidate) return "$";
  if (rest === "sleeping" || rest === "deep_sleep") return "z";
  return "a";
}

function agentColor(session) {
  switch (String(operatorPressure(session).tone || "").toLowerCase()) {
    case "danger":
      return COLORS.ember;
    case "warning":
      return COLORS.warning;
    case "working":
      return COLORS.accent;
    case "quiet":
      return COLORS.agentBlue;
    default:
      break;
  }
  const state = String(session?.state || "").toLowerCase();
  if (primaryActionCueKind(session)) return COLORS.ember;
  if (state === "attention" || state === "error") return COLORS.ember;
  if (session?.commitCandidate) return COLORS.warning;
  return COLORS.agentBlue;
}

function speedReadWord(session, wpm, elapsedMs = 0, reading = true, readerStartIndex = null) {
  const words = clawgWords(session);
  if (!words.length) {
    return "waiting";
  }
  const baseIndex = Number.isFinite(readerStartIndex)
    ? clampInt(readerStartIndex, 0, 0, words.length)
    : clawgReadIndex(session);
  if (baseIndex >= words.length) {
    return "caught up";
  }
  if (!reading) {
    return truncate(words[baseIndex], 18);
  }
  const msPerWord = Math.max(60, 60000 / Math.max(1, wpm));
  const index = Math.min(words.length, baseIndex + Math.floor(Math.max(0, elapsedMs) / msPerWord));
  if (index >= words.length) {
    return "caught up";
  }
  return truncate(words[index], 18);
}

function clawgWords(session) {
  return String(session?.clawgText || session?.thoughtLabel || session?.commandLabel || session?.name || "waiting")
    .split(/\s+/)
    .map((word) => word.trim())
    .filter(Boolean);
}

function clawgReadIndex(session) {
  return clampInt(session?.clawgReadIndex, 0, 0, clawgWords(session).length);
}

function clawgReadComplete(session) {
  const words = clawgWords(session);
  return words.length > 0 && clawgReadIndex(session) >= words.length;
}

function trogdorReadButtonLabel(session, model) {
  if (session && clawgReadComplete(session) && model?.trogdorReading === false) {
    return "Read again";
  }
  return model?.trogdorReading === false ? "Read" : "Pause";
}

function sessionAwaitingUser(session) {
  const pressure = operatorPressure(session);
  const reasonKind = String(pressure.reason_kind || "").toLowerCase();
  const state = String(session?.state || "").toLowerCase();
  return hasActionCue(session, "awaiting_user") || reasonKind === "awaiting_user" || state === "attention";
}

function sessionHasReadyClawg(session) {
  const pressure = operatorPressure(session);
  const reasonKind = String(pressure.reason_kind || "").toLowerCase();
  return (
    actionCueKinds(session).length > 0 ||
    ["awaiting_user", "commit_ready", "validation_missing_after_edit", "dirty_check_missing"].includes(reasonKind) ||
    String(session?.state || "").toLowerCase() === "attention"
  );
}

function sessionIsSleepingOrDeepSleep(session) {
  const rest = String(session?.restLabel || "").toLowerCase();
  return rest === "sleeping" || rest === "deep_sleep";
}

function sessionSwordsmanVisible(session) {
  return Boolean(
    session?.trogdorBurnt ||
      (sessionHasReadyClawg(session) && !session?.trogdorDismissed) ||
      sessionIsSleepingOrDeepSleep(session),
  );
}

function sessionCanReadClawgs(session) {
  return Boolean(
    !session?.trogdorBurnt &&
      ((sessionHasReadyClawg(session) && !session?.trogdorDismissed) ||
        sessionIsSleepingOrDeepSleep(session)),
  );
}

function actionCueKinds(session) {
  const cues = Array.isArray(session?.actionCues) ? session.actionCues : [];
  return cues.map((cue) => String(cue?.kind || "").toLowerCase()).filter(Boolean);
}

function hasActionCue(session, kind) {
  return actionCueKinds(session).includes(kind);
}

function primaryActionCueKind(session) {
  const kinds = actionCueKinds(session);
  for (const kind of [
    "awaiting_user",
    "commit_ready",
    "validation_missing_after_edit",
    "dirty_check_missing",
  ]) {
    if (kinds.includes(kind)) {
      return kind;
    }
  }
  return "";
}

function actionCueLabel(kind) {
  switch (kind) {
    case "awaiting_user":
      return "awaiting user";
    case "commit_ready":
      return "commit ready";
    case "validation_missing_after_edit":
      return "validate";
    case "dirty_check_missing":
      return "dirty check";
    default:
      return "";
  }
}

function primaryActionCueLabel(session) {
  return actionCueLabel(primaryActionCueKind(session));
}

function operatorPressure(session) {
  return session?.operatorPressure && typeof session.operatorPressure === "object"
    ? session.operatorPressure
    : {};
}

function pressureReason(session) {
  return String(operatorPressure(session).reason || "");
}

function sessionCommitReady(session) {
  const pressure = operatorPressure(session);
  return Boolean(pressure.commit_ready || session?.commitCandidate || hasActionCue(session, "commit_ready"));
}

function summarizeSessions(sessions, publishedSessionId, selectedSessionId) {
  const summary = {
    busy: 0,
    idle: 0,
    attention: 0,
    exited: 0,
    stale: 0,
    untrusted: 0,
    commitCandidates: 0,
    codexCount: 0,
    claudeCount: 0,
    activeCommands: 0,
    pressure: 0,
    actionCues: 0,
    subtitle: "Select a session row on the left to attach its terminal.",
    footer: "Use the HUD to open config, native, Mermaid, or repo browser sheets.",
  };

  for (const session of sessions) {
    const state = String(session.state || "").toLowerCase();
    if (state === "busy") {
      summary.busy += 1;
    } else if (state === "attention") {
      summary.attention += 1;
    } else if (state === "exited") {
      summary.exited += 1;
    } else {
      summary.idle += 1;
    }

    if (session.commitCandidate) {
      summary.commitCandidates += 1;
    }
    if (session.isStale) {
      summary.stale += 1;
    }
    if (stateEvidenceIsUnverified(session)) {
      summary.untrusted += 1;
    }
    const tool = String(session.toolLabel || session.tool || "").toLowerCase();
    if (tool.includes("codex")) {
      summary.codexCount += 1;
    } else if (tool.includes("claude")) {
      summary.claudeCount += 1;
    }
    if (session.commandLabel && session.commandLabel !== "idle") {
      summary.activeCommands += 1;
    }
    summary.actionCues += actionCueKinds(session).length;
    summary.pressure = Math.max(summary.pressure, trogdorPressureScore(session));
  }

  const selected = sessions.find((session) => session.sessionId === selectedSessionId) || null;
  const published = sessions.find((session) => session.sessionId === publishedSessionId) || null;
  if (selected) {
    summary.subtitle = `Selected ${selected.name} / ${selected.cwdLabel} / ${stateDisplayLabel(selected)}`;
  } else if (published) {
    summary.subtitle = `Following published session ${published.name} / ${published.cwdLabel}.`;
  } else {
    summary.subtitle = `Watching ${sessions.length} live sessions across ${summary.codexCount + summary.claudeCount} tools.`;
  }

  return summary;
}

function emptyTrogdorSummary() {
  return {
    pressure: 0,
    actionCues: 0,
    subtitle: "No live sessions yet. The atlas is waiting for the first repository.",
    footer: "Use new to launch the first agent.",
  };
}

function terminalStateCopy(model) {
  if (model?.currentSession) {
    if (!model?.frankenTermAvailable || model?.snapshotFallback) {
      return {
        label: "snapshot fallback",
        shortLabel: "snapshot",
        muted: false,
      };
    }
    if (model?.terminalReady) {
      return {
        label: "live terminal",
        shortLabel: "live",
        muted: false,
      };
    }
    return {
      label: `attaching ${model.currentSession.name || "terminal"}`,
      shortLabel: "attaching",
      muted: false,
    };
  }

  if (model?.followPublishedSelection) {
    return {
      label: "awaiting published terminal",
      shortLabel: "waiting",
      muted: true,
    };
  }

  return {
    label: "select a session to attach its terminal",
    shortLabel: "select",
    muted: true,
  };
}

function stateDisplayLabel(session) {
  const label = safeLabel(session?.displayState, safeLabel(session?.state, "unknown"));
  if (stateEvidenceIsUnverified(session) && !label.endsWith("?")) {
    return `${label}?`;
  }
  return label;
}

function stateEvidenceIsUnverified(session) {
  const confidence = String(session?.stateConfidence || "").toLowerCase();
  return confidence !== "high" || session?.stateObserved === false;
}
