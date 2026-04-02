const STYLE_BOLD = 0b0000_0001;
const STYLE_DIM = 0b0000_0010;
const STYLE_ITALIC = 0b0000_0100;
const STYLE_UNDERLINE = 0b0000_1000;

const COLORS = {
  transparent: 0,
  text: rgba(214, 236, 245, 255),
  muted: rgba(141, 164, 175, 255),
  accent: rgba(125, 216, 255, 255),
  accentSoft: rgba(125, 216, 255, 212),
  success: rgba(139, 227, 191, 255),
  warning: rgba(255, 207, 122, 255),
  danger: rgba(255, 125, 148, 255),
  panelBg: rgba(10, 15, 19, 214),
  panelBgStrong: rgba(7, 11, 14, 236),
  panelBorder: rgba(120, 210, 255, 176),
  panelBorderSoft: rgba(120, 210, 255, 92),
  chipBg: rgba(14, 20, 25, 228),
  chipActiveBg: rgba(18, 43, 47, 236),
  chipDangerBg: rgba(46, 18, 24, 236),
  sessionActiveBg: rgba(18, 42, 49, 240),
  sessionPublishedBg: rgba(20, 34, 30, 224),
  footerBg: rgba(8, 12, 16, 232),
  overlayBg: rgba(5, 8, 11, 188),
};

export function buildSurfaceFrame(model) {
  const cols = clampInt(model?.cols, 80, 32, 240);
  const rows = clampInt(model?.rows, 24, 16, 120);
  const frame = {
    cols,
    rows,
    cells: new Uint32Array(cols * rows * 4),
    spans: new Uint32Array([0, cols * rows]),
    zones: [],
    masks: [],
  };

  const layout = computeLayout(cols, rows, Boolean(model?.focusLayout));
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

function computeLayout(cols, rows, focusLayout) {
  const header = rect(2, 1, Math.max(28, cols - 4), 4);
  const footerHeight = rows >= 26 ? 5 : 4;
  const footer = rect(2, Math.max(6, rows - footerHeight - 1), Math.max(28, cols - 4), footerHeight);
  const panelTop = header.y + header.h + 1;
  const panelBottom = footer.y - 1;
  const panelHeight = Math.max(6, panelBottom - panelTop);
  const showSessionRail = !focusLayout && cols >= 84 && panelHeight >= 10;
  const showDetailRail = cols >= 110 && panelHeight >= 10;
  const leftWidth = showSessionRail ? Math.max(24, Math.min(32, Math.floor(cols * 0.24))) : 0;
  const rightWidth = showDetailRail ? Math.max(28, Math.min(36, Math.floor(cols * 0.27))) : 0;
  const sessionRail = showSessionRail ? rect(2, panelTop, leftWidth, panelHeight) : null;
  const detailRail = showDetailRail ? rect(cols - rightWidth - 2, panelTop, rightWidth, panelHeight) : null;
  const centerLeft = sessionRail ? sessionRail.x + sessionRail.w + 1 : 2;
  const centerRight = detailRail ? detailRail.x - 1 : cols - 2;
  const center = rect(centerLeft, panelTop, Math.max(10, centerRight - centerLeft), panelHeight);

  return {
    header,
    footer,
    sessionRail,
    detailRail,
    center,
  };
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
  ];

  let cursorX = layout.header.x + 2;
  const chipY = layout.header.y + 2;
  const chipLimit = layout.header.x + layout.header.w - 2;
  for (const chip of chips) {
    const width = chip.label.length + 4;
    if (cursorX + width > chipLimit) {
      break;
    }
    drawChip(frame, cursorX, chipY, chip.label, chip.fg, chip.bg);
    if (chip.zone) {
      pushZone(frame, chip.zone, rect(cursorX, chipY, width, 1));
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
    ? [model.currentSession.state, model.currentSession.toolLabel, model.currentSession.cwdLabel].filter(Boolean).join(" / ")
    : model?.frankenTermAvailable
      ? "rendered surface ready"
      : "FrankenTerm assets unavailable";
  drawTextRight(frame, layout.header.x + layout.header.w - 2, layout.header.y + 2, secondary, {
    fg: COLORS.muted,
    width: Math.max(16, Math.floor(layout.header.w * 0.42)),
  });
}

function drawSessionRail(frame, rail, model) {
  drawPanel(frame, rail, model?.focusLayout ? "published" : "sessions");
  pushMask(frame, rail);

  const sessions = Array.isArray(model?.sessions) ? model.sessions : [];
  if (!sessions.length) {
    drawTextBlock(frame, rail.x + 2, rail.y + 2, rail.w - 4, 4, "No live sessions.\nCreate one from the rendered action rail.", {
      fg: COLORS.muted,
    });
    return;
  }

  const entryHeight = 4;
  const availableRows = Math.max(1, rail.h - 3);
  const visibleCount = Math.max(1, Math.floor(availableRows / entryHeight));
  const selectedIndex = Math.max(0, sessions.findIndex((session) => session.sessionId === model?.selectedSessionId));
  const start = clampInt(selectedIndex - Math.floor(visibleCount / 2), 0, 0, Math.max(0, sessions.length - visibleCount));
  const end = Math.min(sessions.length, start + visibleCount);

  let y = rail.y + 1;
  for (let index = start; index < end; index += 1) {
    const session = sessions[index];
    const isSelected = session.sessionId === model?.selectedSessionId;
    const isPublished = session.sessionId === model?.publishedSessionId;
    const bg = isSelected ? COLORS.sessionActiveBg : isPublished ? COLORS.sessionPublishedBg : COLORS.transparent;
    const fg = isSelected ? COLORS.text : COLORS.text;
    fillRect(frame, rail.x + 1, y, rail.w - 2, entryHeight - 1, bg);
    drawText(frame, rail.x + 2, y, truncate(session.name, rail.w - 6), {
      fg: isSelected ? COLORS.success : fg,
      attrs: STYLE_BOLD,
      width: rail.w - 4,
    });
    drawText(frame, rail.x + 2, y + 1, truncate(`${session.state} / ${session.toolLabel}`, rail.w - 6), {
      fg: session.transportLabel === "healthy" ? COLORS.muted : COLORS.warning,
      width: rail.w - 4,
    });
    drawText(frame, rail.x + 2, y + 2, truncate(`${session.cwdLabel} :: ${session.thoughtLabel}`, rail.w - 6), {
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

  if (end < sessions.length) {
    drawText(frame, rail.x + 2, rail.y + rail.h - 2, truncate(`more ${sessions.length - end} below`, rail.w - 4), {
      fg: COLORS.muted,
      attrs: STYLE_ITALIC,
      width: rail.w - 4,
    });
  }
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
    ["state", session.state],
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
    drawActionChipRow(frame, rail.x + 2, actionY + 1, rail.x + rail.w - 2, actions, "action");
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
    actionSpec("search", "search", "open_search", Boolean(model?.terminalReady)),
    actionSpec("send", "send", "open_send", Boolean(model?.currentSession) && !model?.readOnly),
    actionSpec("mmd", "mmd", "open_mermaid", Boolean(model?.currentSession)),
    actionSpec("commit", "commit", "launch_commit", Boolean(model?.currentSession?.commitCandidate)),
    actionSpec("config", "config", "open_config", true),
    actionSpec("native", "native", "open_native", true),
    actionSpec(model?.followPublishedSelection ? "following" : "follow", "follow", "toggle_follow", true),
    actionSpec(model?.selectMode ? "select on" : "select", "select", "toggle_select", Boolean(model?.terminalReady)),
    actionSpec("copy", "copy", "copy_selection", Boolean(model?.terminalReady)),
    actionSpec("new", "new", "open_create", !model?.readOnly),
    actionSpec("auth", "auth", "open_auth", true),
    actionSpec("refresh", "refresh", "refresh", true),
  ];

  let x = footer.x + 2;
  const limit = footer.x + footer.w - 2;
  drawActionChipRow(frame, x, actionY, limit, actions, "action");
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
  if (model?.currentSession) {
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
  if (!sessions.length) {
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
    drawTextBlock(frame, overlay.x + 2, overlay.y + 2, overlay.w - 4, overlay.h - 3, "No live sessions. Create one from the rendered action rail.", {
      fg: COLORS.text,
    });
    return;
  }

  const summary = summarizeSessions(sessions, model?.publishedSessionId, model?.selectedSessionId);
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
  drawText(frame, overlay.x + 2, overlay.y + 1, "web-native aquarium / session atlas", {
    fg: COLORS.accent,
    attrs: STYLE_BOLD,
    width: overlay.w - 4,
  });
  drawText(frame, overlay.x + 2, overlay.y + 2, truncate(summary.subtitle, overlay.w - 4), {
    fg: COLORS.muted,
    width: overlay.w - 4,
  });

  const cardY = overlay.y + 4;
  const cardHeight = Math.min(7, Math.max(5, overlay.h - 6));
  const cardCount = overlay.w >= 84 ? 3 : 2;
  const cardGap = 1;
  const cardWidth = Math.max(16, Math.floor((overlay.w - 4 - cardGap * (cardCount - 1)) / cardCount));
  const cards = [
    {
      title: "live",
      accent: COLORS.success,
      lines: [
        `busy ${summary.busy}`,
        `attention ${summary.attention}`,
        `commands ${summary.activeCommands}`,
      ],
    },
    {
      title: "rest",
      accent: COLORS.warning,
      lines: [
        `idle ${summary.idle}`,
        `exited ${summary.exited}`,
        `stale ${summary.stale}`,
      ],
    },
    {
      title: "repo",
      accent: COLORS.accent,
      lines: [
        `commit ${summary.commitCandidates}`,
        `codex ${summary.codexCount}`,
        `claude ${summary.claudeCount}`,
      ],
    },
  ].slice(0, cardCount);

  let cardX = overlay.x + 2;
  for (const card of cards) {
    drawOverviewCard(frame, rect(cardX, cardY, cardWidth, cardHeight), card.title, card.lines, card.accent);
    cardX += cardWidth + cardGap;
  }

  drawText(frame, overlay.x + 2, overlay.y + overlay.h - 2, truncate(summary.footer, overlay.w - 4), {
    fg: COLORS.muted,
    attrs: STYLE_ITALIC,
    width: overlay.w - 4,
  });
}

function summarizeSessions(sessions, publishedSessionId, selectedSessionId) {
  const summary = {
    busy: 0,
    idle: 0,
    attention: 0,
    exited: 0,
    stale: 0,
    commitCandidates: 0,
    codexCount: 0,
    claudeCount: 0,
    activeCommands: 0,
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
    const tool = String(session.toolLabel || session.tool || "").toLowerCase();
    if (tool.includes("codex")) {
      summary.codexCount += 1;
    } else if (tool.includes("claude")) {
      summary.claudeCount += 1;
    }
    if (session.commandLabel && session.commandLabel !== "idle") {
      summary.activeCommands += 1;
    }
  }

  const selected = sessions.find((session) => session.sessionId === selectedSessionId) || null;
  const published = sessions.find((session) => session.sessionId === publishedSessionId) || null;
  if (selected) {
    summary.subtitle = `Selected ${selected.name} / ${selected.cwdLabel} / ${selected.state}`;
  } else if (published) {
    summary.subtitle = `Following published session ${published.name} / ${published.cwdLabel}.`;
  } else {
    summary.subtitle = `Watching ${sessions.length} live sessions across ${summary.codexCount + summary.claudeCount} tools.`;
  }

  return summary;
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

function drawOverviewCard(frame, panel, title, lines, accent) {
  drawPanel(frame, panel, title, {
    bg: COLORS.panelBg,
    border: accent,
  });
  let y = panel.y + 1;
  for (const line of lines) {
    drawText(frame, panel.x + 2, y, truncate(line, panel.w - 4), {
      fg: COLORS.text,
      width: panel.w - 4,
    });
    y += 1;
  }
}

function drawPanel(frame, panel, title, options = {}) {
  const bg = options.bg ?? COLORS.panelBg;
  const border = options.border ?? COLORS.panelBorder;
  fillRect(frame, panel.x, panel.y, panel.w, panel.h, bg);
  drawBorder(frame, panel, border);
  if (title) {
    drawText(frame, panel.x + 2, panel.y, ` ${truncate(title, Math.max(0, panel.w - 6))} `, {
      fg: border,
      bg,
      attrs: STYLE_BOLD,
      width: Math.max(0, panel.w - 4),
    });
  }
}

function drawBorder(frame, panel, fg) {
  if (panel.w < 2 || panel.h < 2) {
    return;
  }
  putChar(frame, panel.x, panel.y, "╭", fg, COLORS.transparent);
  putChar(frame, panel.x + panel.w - 1, panel.y, "╮", fg, COLORS.transparent);
  putChar(frame, panel.x, panel.y + panel.h - 1, "╰", fg, COLORS.transparent);
  putChar(frame, panel.x + panel.w - 1, panel.y + panel.h - 1, "╯", fg, COLORS.transparent);
  for (let x = panel.x + 1; x < panel.x + panel.w - 1; x += 1) {
    putChar(frame, x, panel.y, "─", fg, COLORS.transparent);
    putChar(frame, x, panel.y + panel.h - 1, "─", fg, COLORS.transparent);
  }
  for (let y = panel.y + 1; y < panel.y + panel.h - 1; y += 1) {
    putChar(frame, panel.x, y, "│", fg, COLORS.transparent);
    putChar(frame, panel.x + panel.w - 1, y, "│", fg, COLORS.transparent);
  }
}

function drawChip(frame, x, y, label, fg, bg) {
  const width = label.length + 4;
  fillRect(frame, x, y, width, 1, bg);
  drawText(frame, x, y, `[${label}]`, {
    fg,
    bg,
    attrs: STYLE_BOLD,
    width,
  });
}

function drawActionChipRow(frame, x, y, limit, actions, zoneType) {
  let cursor = x;
  for (const action of actions) {
    const width = action.label.length + 4;
    if (cursor + width > limit) {
      break;
    }
    drawChip(
      frame,
      cursor,
      y,
      action.label,
      action.enabled ? COLORS.text : COLORS.muted,
      action.enabled ? COLORS.chipBg : COLORS.panelBg,
    );
    pushZone(
      frame,
      {
        type: zoneType,
        actionId: action.actionId,
        disabled: !action.enabled,
      },
      rect(cursor, y, width, 1),
    );
    cursor += width + 1;
  }
}

function drawText(frame, x, y, text, options = {}) {
  if (typeof text !== "string" || !text) {
    return;
  }
  const width = clampInt(options.width, text.length, 0, frame.cols);
  const content = width > 0 ? truncate(text, width) : text;
  let cursor = x;
  for (const char of content) {
    if (cursor >= x + width) {
      break;
    }
    putChar(
      frame,
      cursor,
      y,
      char,
      options.fg ?? COLORS.text,
      options.bg ?? COLORS.transparent,
      options.attrs ?? 0,
    );
    cursor += 1;
  }
}

function drawTextRight(frame, rightX, y, text, options = {}) {
  const width = clampInt(options.width, String(text || "").length, 0, frame.cols);
  const content = truncate(String(text || ""), width);
  drawText(frame, rightX - content.length + 1, y, content, options);
}

function drawTextBlock(frame, x, y, width, maxLines, text, options = {}) {
  const lines = wrapText(String(text || ""), width, maxLines);
  for (let index = 0; index < lines.length; index += 1) {
    drawText(frame, x, y + index, lines[index], {
      fg: options.fg,
      bg: options.bg,
      attrs: options.attrs,
      width,
    });
  }
}

function wrapText(text, width, maxLines) {
  const safeWidth = Math.max(1, width);
  const lines = [];
  const rawLines = String(text || "").replaceAll("\r", "").split("\n");
  for (const rawLine of rawLines) {
    if (lines.length >= maxLines) {
      break;
    }
    const words = rawLine.split(/\s+/).filter(Boolean);
    if (!words.length) {
      lines.push("");
      continue;
    }
    let current = "";
    for (const word of words) {
      const candidate = current ? `${current} ${word}` : word;
      if (candidate.length <= safeWidth) {
        current = candidate;
        continue;
      }
      if (current) {
        lines.push(truncate(current, safeWidth));
        if (lines.length >= maxLines) {
          return finalizeLines(lines, safeWidth, maxLines);
        }
        current = "";
      }
      if (word.length <= safeWidth) {
        current = word;
      } else {
        lines.push(truncate(word, safeWidth));
        if (lines.length >= maxLines) {
          return finalizeLines(lines, safeWidth, maxLines);
        }
      }
    }
    if (current && lines.length < maxLines) {
      lines.push(truncate(current, safeWidth));
    }
  }
  return finalizeLines(lines, safeWidth, maxLines);
}

function finalizeLines(lines, width, maxLines) {
  const trimmed = lines.slice(0, maxLines);
  if (lines.length > maxLines && trimmed.length) {
    trimmed[maxLines - 1] = truncate(trimmed[maxLines - 1], width);
  }
  return trimmed;
}

function putChar(frame, x, y, char, fg, bg, attrs = 0) {
  if (x < 0 || y < 0 || x >= frame.cols || y >= frame.rows) {
    return;
  }
  const index = (y * frame.cols + x) * 4;
  frame.cells[index] = bg >>> 0;
  frame.cells[index + 1] = fg >>> 0;
  frame.cells[index + 2] = codePoint(char);
  frame.cells[index + 3] = attrs >>> 0;
}

function fillRect(frame, x, y, width, height, bg) {
  for (let row = 0; row < height; row += 1) {
    for (let col = 0; col < width; col += 1) {
      putChar(frame, x + col, y + row, " ", COLORS.text, bg, 0);
    }
  }
}

function pushZone(frame, zone, rectValue) {
  frame.zones.push({ ...zone, rect: rectValue });
}

function pushMask(frame, rectValue) {
  frame.masks.push(rectValue);
}

function rect(x, y, w, h) {
  return { x, y, w, h };
}

function cellInRect(cell, rectValue) {
  return (
    cell.x >= rectValue.x &&
    cell.x < rectValue.x + rectValue.w &&
    cell.y >= rectValue.y &&
    cell.y < rectValue.y + rectValue.h
  );
}

function chipSpec(label, fg, bg, zone = null) {
  return { label, fg, bg, zone };
}

function actionSpec(label, title, actionId, enabled) {
  return {
    label: label || title,
    actionId,
    enabled,
  };
}

function truncate(value, width) {
  const text = String(value || "");
  if (width <= 0) {
    return "";
  }
  if (text.length <= width) {
    return text;
  }
  if (width <= 3) {
    return text.slice(0, width);
  }
  return `${text.slice(0, width - 3)}...`;
}

function safeLabel(value, fallback) {
  const text = String(value || "").trim();
  return text || fallback;
}

function codePoint(char) {
  return (String(char || " ").codePointAt(0) ?? 32) >>> 0;
}

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

function rgba(red, green, blue, alpha = 255) {
  return (
    ((red & 255) * 0x1000000) +
    ((green & 255) << 16) +
    ((blue & 255) << 8) +
    (alpha & 255)
  ) >>> 0;
}
