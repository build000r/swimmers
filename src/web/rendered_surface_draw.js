export const STYLE_BOLD = 0b0000_0001;
export const STYLE_DIM = 0b0000_0010;
export const STYLE_ITALIC = 0b0000_0100;
export const STYLE_UNDERLINE = 0b0000_1000;

export const COLORS = {
  transparent: 0,
  text: rgba(214, 236, 245, 255),
  muted: rgba(141, 164, 175, 255),
  accent: rgba(125, 216, 255, 255),
  accentSoft: rgba(125, 216, 255, 212),
  success: rgba(139, 227, 191, 255),
  warning: rgba(255, 207, 122, 255),
  danger: rgba(255, 125, 148, 255),
  parchment: rgba(221, 201, 166, 255),
  ember: rgba(236, 105, 73, 255),
  ink: rgba(11, 15, 17, 242),
  agentBlue: rgba(132, 174, 231, 255),
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

export function computeSurfaceLayout(cols, rows, focusLayout) {
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

export function drawPanel(frame, panel, title, options = {}) {
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

export function drawOverviewCard(frame, panel, title, lines, accent) {
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

export function drawBorder(frame, panel, fg) {
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

export function drawChip(frame, x, y, label, fg, bg) {
  const width = label.length + 4;
  fillRect(frame, x, y, width, 1, bg);
  drawText(frame, x, y, `[${label}]`, {
    fg,
    bg,
    attrs: STYLE_BOLD,
    width,
  });
}

export function drawActionChipRow(frame, x, y, limit, actions, zoneType, options = {}) {
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
        label: action.label,
        ...action.zone,
      },
      expandedRect(frame, cursor, y, width, 1, options.hitPadX || 0, options.hitPadY || 0),
    );
    cursor += width + 1;
  }
}

export function drawText(frame, x, y, text, options = {}) {
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

export function drawTextRight(frame, rightX, y, text, options = {}) {
  const width = clampInt(options.width, String(text || "").length, 0, frame.cols);
  const content = truncate(String(text || ""), width);
  drawText(frame, rightX - content.length + 1, y, content, options);
}

export function drawTextCenter(frame, x, y, width, text, options = {}) {
  const content = truncate(String(text || ""), width);
  const left = x + Math.max(0, Math.floor((width - content.length) / 2));
  drawText(frame, left, y, content, {
    ...options,
    width: Math.max(0, width - Math.max(0, Math.floor((width - content.length) / 2))),
  });
}

export function drawTextBlock(frame, x, y, width, maxLines, text, options = {}) {
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

export function wrapText(text, width, maxLines) {
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

export function finalizeLines(lines, width, maxLines) {
  const trimmed = lines.slice(0, maxLines);
  if (lines.length > maxLines && trimmed.length) {
    trimmed[maxLines - 1] = truncate(trimmed[maxLines - 1], width);
  }
  return trimmed;
}

export function putChar(frame, x, y, char, fg, bg, attrs = 0) {
  if (x < 0 || y < 0 || x >= frame.cols || y >= frame.rows) {
    return;
  }
  const index = (y * frame.cols + x) * 4;
  frame.cells[index] = bg >>> 0;
  frame.cells[index + 1] = fg >>> 0;
  frame.cells[index + 2] = codePoint(char);
  frame.cells[index + 3] = attrs >>> 0;
}

export function fillRect(frame, x, y, width, height, bg) {
  for (let row = 0; row < height; row += 1) {
    for (let col = 0; col < width; col += 1) {
      putChar(frame, x + col, y + row, " ", COLORS.text, bg, 0);
    }
  }
}

export function pushZone(frame, zone, rectValue) {
  frame.zones.push({ ...zone, rect: rectValue });
}

export function pushMask(frame, rectValue) {
  frame.masks.push(rectValue);
}

export function rect(x, y, w, h) {
  return { x, y, w, h };
}

export function expandedRect(frame, x, y, w, h, padX = 0, padY = 0) {
  const left = Math.max(0, x - padX);
  const top = Math.max(0, y - padY);
  const right = Math.min(frame.cols, x + w + padX);
  const bottom = Math.min(frame.rows, y + h + padY);
  return rect(left, top, Math.max(0, right - left), Math.max(0, bottom - top));
}

export function cellInRect(cell, rectValue) {
  return (
    cell.x >= rectValue.x &&
    cell.x < rectValue.x + rectValue.w &&
    cell.y >= rectValue.y &&
    cell.y < rectValue.y + rectValue.h
  );
}

export function chipSpec(label, fg, bg, zone = null) {
  return { label, fg, bg, zone };
}

export function actionSpec(label, title, actionId, enabled, zone = {}) {
  return {
    label: label || title,
    actionId,
    enabled,
    zone,
  };
}

export function truncate(value, width) {
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

export function safeLabel(value, fallback) {
  const text = String(value || "").trim();
  return text || fallback;
}

export function codePoint(char) {
  return (String(char || " ").codePointAt(0) ?? 32) >>> 0;
}

export function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

export function rgba(red, green, blue, alpha = 255) {
  return (
    ((red & 255) * 0x1000000) +
    ((green & 255) << 16) +
    ((blue & 255) << 8) +
    (alpha & 255)
  ) >>> 0;
}
