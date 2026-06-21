import {
  TROGDOR_DRAGON_TARGET,
  trogdorAgentGlyph,
  trogdorAgentTone,
  trogdorDomPressure,
  trogdorDomReason,
} from "./trogdor_logic.js";

const TROGDOR_DRAGON_ASSET_BASE = "/assets/dragon";
const TROGDOR_DRAGON_FIRE_STAGES = ["short", "mid", "full"];
const TROGDOR_DRAGON_BODY_FRAMES = [
  "front",
  "3q-right",
  "right",
  "back-right",
  "back",
  "back-left",
  "left",
  "3q-left",
];

const TROGDOR_AGENT_OFFSETS = [
  { x: -98, y: 30 },
  { x: 96, y: 26 },
  { x: -64, y: 92 },
  { x: 64, y: 92 },
  { x: 0, y: 106 },
  { x: -110, y: 78 },
  { x: 108, y: 78 },
];

export const TROGDOR_REPO_POSITIONS = [
  { x: 18, y: 40, size: "small", variant: "hut" },
  { x: 42, y: 32, size: "large", variant: "tower" },
  { x: 78, y: 38, size: "small", variant: "hut" },
  { x: 22, y: 78, size: "wide", variant: "burning_shack" },
  { x: 88, y: 76, size: "small", variant: "hut" },
  { x: 50, y: 84, size: "small", variant: "ruin" },
  { x: 64, y: 22, size: "small", variant: "hut" },
  { x: 12, y: 60, size: "small", variant: "hut" },
];

function clampInt(value, fallback, min, max) {
  const numeric = Number.isFinite(value) ? Math.trunc(value) : fallback;
  return Math.max(min, Math.min(max, numeric));
}

function escapeHtml(text) {
  return String(text || "").replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      case "'":
        return "&#39;";
      default:
        return char;
    }
  });
}

function escapeAttr(text) {
  return escapeHtml(text);
}

export function trogdorReadButtonLabel(reading, readComplete = false) {
  if (readComplete && reading === false) {
    return "read again";
  }
  return reading === false ? "read" : "pause";
}

export function trogdorSurfaceSignature(sessions, summary, readOnly = false) {
  const sessionSignature = sessions.map((session) => {
    return [
      session.sessionId,
      session.name,
      session.repoKey,
      session.repoLabel,
      session.targetKey,
      session.targetLabel,
      session.state,
      session.restLabel,
      session.thoughtLabel,
      session.thoughtUpdatedAt,
      session.trogdorAwaitingUser ? "awaiting" : "",
      session.trogdorBurnt ? "burnt" : "",
      session.trogdorDismissed ? "dismissed" : "",
      trogdorDomPressure(session),
      trogdorDomReason(session),
      trogdorAgentGlyph(session),
      (session.batchSendSessionIds || []).join(","),
      session.commitCandidate ? "commit" : "",
    ].join(":");
  });
  return JSON.stringify({
    sessions: sessionSignature,
    readOnly,
    score: summary.score,
    level: summary.level,
  });
}

export function renderTrogdorSurfaceFrame({
  groups = [],
  sessions = [],
  summary = {},
  dragonPose = null,
  readerMarkup = "",
  readButtonLabel = "pause",
  wpm = 200,
  readOnly = false,
  hoveredSessionId = "",
} = {}) {
  const clampedWpm = clampInt(wpm, 200, 50, 800);
  return `
    ${renderTrogdorPrintFilter()}
    <div class="trogdor-frame">
      <div class="trogdor-topbar">
        <div class="trogdor-score"><span>score:</span><strong>${escapeHtml(summary.score || "0000")}</strong></div>
        ${readerMarkup}
        <div class="trogdor-level"><span>mans: ${sessions.length}</span><span>level: ${clampInt(summary.level, 0, 0, 99)}</span></div>
      </div>
      <div class="trogdor-world" aria-label="Repository structures and agent swordsmen">
        <div class="trogdor-sun-band" aria-hidden="true"></div>
        <div class="trogdor-mountains" aria-hidden="true"></div>
        <div class="trogdor-clouds" aria-hidden="true">
          <span class="trogdor-cloud trogdor-cloud-a"></span>
          <span class="trogdor-cloud trogdor-cloud-b"></span>
        </div>
        <div class="trogdor-props" aria-hidden="true">${renderTrogdorProps()}</div>
        ${renderTrogdorDragon(dragonPose)}
        ${groups.length
          ? groups.map((group, index) => {
              const pos = TROGDOR_REPO_POSITIONS[index % TROGDOR_REPO_POSITIONS.length];
              return renderTrogdorStructure(group, index, pos, dragonPose, { hoveredSessionId });
            }).join("")
          : renderTrogdorEmptyField(readOnly)}
      </div>
      <div class="trogdor-bottombar">
        <div class="trogdor-wpm">
          <button type="button" data-action="trogdor_read_toggle">${escapeHtml(readButtonLabel)}</button>
          <button type="button" data-action="trogdor_wpm_down" aria-label="Decrease reading speed">-25</button>
          <span class="trogdor-wpm-value" data-trogdor-wpm-value="true" aria-live="polite">${clampedWpm} wpm</span>
          <button type="button" data-action="trogdor_wpm_up" aria-label="Increase reading speed">+25</button>
        </div>
        <div class="trogdor-actions">
          <button type="button" data-action="focus_terminal">terminal</button>
          <button type="button" data-action="open_create"${readOnly ? " disabled" : ""}>new agent</button>
          <button type="button" data-action="open_config">config</button>
          <button type="button" data-action="open_native">native</button>
          <button type="button" data-action="open_auth">auth</button>
          <button type="button" data-action="refresh">refresh</button>
        </div>
      </div>
    </div>
  `;
}

export function renderTrogdorPrintFilter() {
  return `
    <svg class="trogdor-svg-defs" aria-hidden="true" focusable="false" width="0" height="0">
      <defs>
        <filter id="trogdor-print" x="-12%" y="-12%" width="124%" height="124%" color-interpolation-filters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="1.05" numOctaves="3" seed="7" result="warp" />
          <feDisplacementMap in="SourceGraphic" in2="warp" scale="2.6" xChannelSelector="R" yChannelSelector="G" />
        </filter>
        <filter id="trogdor-stamp" x="-18%" y="-18%" width="136%" height="136%" color-interpolation-filters="sRGB">
          <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="3" seed="13" result="warp" />
          <feDisplacementMap in="SourceGraphic" in2="warp" scale="3.4" xChannelSelector="R" yChannelSelector="G" result="warped" />
          <feTurbulence type="fractalNoise" baseFrequency="2.6" numOctaves="2" seed="5" result="grain" />
          <feColorMatrix in="grain" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 7 -5.6" result="grainAlpha" />
          <feComposite in="warped" in2="grainAlpha" operator="out" />
        </filter>
      </defs>
    </svg>
  `;
}

export function renderTrogdorProps() {
  return `
    <svg class="trogdor-prop trogdor-prop-bone" viewBox="0 0 60 24" aria-hidden="true" filter="url(#trogdor-print)">
      <path d="M8 12 H52" />
      <circle cx="8" cy="8" r="5" />
      <circle cx="8" cy="16" r="5" />
      <circle cx="52" cy="8" r="5" />
      <circle cx="52" cy="16" r="5" />
    </svg>
    <svg class="trogdor-prop trogdor-prop-torch" viewBox="0 0 32 60" aria-hidden="true" filter="url(#trogdor-print)">
      <path class="prop-torch-stem" d="M12 24 H20 V58 H12 Z" />
      <path class="prop-torch-flame" d="M16 24 C8 14 14 12 12 2 C18 8 20 8 16 -2 C24 6 24 18 16 24 Z" />
    </svg>
    <svg class="trogdor-prop trogdor-prop-bottle" viewBox="0 0 24 40" aria-hidden="true" filter="url(#trogdor-print)">
      <path d="M9 4 H15 V12 C19 14 19 20 19 38 H5 C5 20 5 14 9 12 Z" />
      <path d="M9 22 H19" />
    </svg>
  `;
}

export function trogdorDragonAsset(pose, bodyFrame) {
  const frame = TROGDOR_DRAGON_BODY_FRAMES.includes(bodyFrame) ? bodyFrame : "right";
  return `${TROGDOR_DRAGON_ASSET_BASE}/${pose}/${frame}.png`;
}

export function renderTrogdorDragon(pose) {
  const direction = pose?.direction === "left" ? "left" : "right";
  const bodyFrame = TROGDOR_DRAGON_BODY_FRAMES.includes(pose?.bodyFrame)
    ? pose.bodyFrame
    : direction;
  const classes = [
    "trogdor-dragon",
    `is-${direction}`,
    `is-frame-${bodyFrame}`,
    pose?.heated ? "is-heated" : "",
    pose?.firing ? "is-firing" : "",
  ].filter(Boolean).join(" ");
  const style = [
    `--dragon-x:${clampInt(pose?.x, TROGDOR_DRAGON_TARGET.x, 12, 88)}%`,
    `--dragon-y:${clampInt(pose?.y, TROGDOR_DRAGON_TARGET.y, 26, 84)}%`,
    `--dragon-walk-x:${pose?.walkX || "3.2vw"}`,
    `--dragon-walk-y:${pose?.walkY || "-1.2vh"}`,
  ].join("; ");
  const idleSrc = escapeAttr(trogdorDragonAsset("mouth-closed", bodyFrame));
  const openSrc = escapeAttr(trogdorDragonAsset("mouth-open", bodyFrame));
  const fireFrames = TROGDOR_DRAGON_FIRE_STAGES.map((stage) => {
    const src = escapeAttr(trogdorDragonAsset(`fire-${direction}-${stage}`, bodyFrame));
    return `
      <img
        class="trogdor-dragon-sprite trogdor-dragon-fire is-${stage}"
        src="${src}"
        alt=""
        width="155"
        height="147"
        decoding="async"
        draggable="false"
      />
    `;
  }).join("");

  return `
    <div class="${classes}" style="${style}" aria-hidden="true" data-dragon-direction="${direction}" data-dragon-frame="${bodyFrame}">
      <span class="trogdor-dragon-sprite-stack">
        <img
          class="trogdor-dragon-sprite trogdor-dragon-idle"
          src="${idleSrc}"
          alt=""
          width="155"
          height="147"
          decoding="async"
          draggable="false"
        />
        <img
          class="trogdor-dragon-sprite trogdor-dragon-open"
          src="${openSrc}"
          alt=""
          width="155"
          height="147"
          decoding="async"
          draggable="false"
        />
        ${fireFrames}
      </span>
    </div>
  `;
}

export function renderTrogdorStructure(group, index, pos, dragonPose = null, options = {}) {
  const pressure = clampInt(group.pressure, 0, 0, 99);
  const pressureBurning = pressure >= 70;
  const baseVariant = pos.variant || "hut";
  const variant = pressureBurning && baseVariant !== "burning_shack" ? "ruin" : baseVariant;
  const burning = pressureBurning || baseVariant === "burning_shack";
  const warning = pressure >= 35 && !pressureBurning;
  const classes = [
    "trogdor-structure",
    `is-${pos.size}`,
    `is-variant-${variant}`,
    burning ? "is-burning" : "",
    warning ? "is-warning" : "",
  ].filter(Boolean).join(" ");
  const label = escapeHtml(group.label);
  const reason = escapeHtml(group.reason);
  const hostSummary = group.hostSummary ? ` · ${escapeHtml(group.hostSummary)}` : "";
  const style = `--x:${pos.x}%; --y:${pos.y}%; --delay:${index * 130}ms;`;
  const swordsmen = group.sessions.filter((session) => session.trogdorSwordsmanVisible);

  return `
    <article class="${classes}" style="${style}" aria-label="${escapeAttr(group.label)} repository pressure ${pressure}">
      ${renderStructureSvg(variant, burning)}
      <div class="trogdor-repo-label">
        <strong>${label}</strong>
        <span>${pressure} / ${reason}${hostSummary}</span>
      </div>
      <div class="trogdor-agent-pack">
        ${swordsmen.map((session, agentIndex) => renderTrogdorAgent(session, agentIndex, pos, dragonPose, options)).join("")}
      </div>
    </article>
  `;
}

export function renderStructureSvg(variant, burning) {
  const filterAttr = ' filter="url(#trogdor-stamp)"';
  let body = "";
  switch (variant) {
    case "tower":
      body = `
        <path class="structure-roof" d="M14 70 L42 38 L78 12 L116 38 L146 70 L132 78 L116 56 L80 28 L46 58 L28 78 Z" />
        <path class="structure-body" d="M26 68 L36 70 L70 68 L100 70 L132 68 L130 102 L132 130 L130 146 L94 144 L60 146 L28 144 L30 110 Z" />
        <path class="structure-arch" d="M62 146 V112 C62 96 98 96 98 112 V146 Z" />
        <g class="structure-bricks">
          <path d="M28 88 L60 90 L100 88 L132 90" />
          <path d="M28 110 L62 108 L100 110 L132 108" />
          <path d="M28 130 L66 132 L102 130 L132 132" />
          <path d="M48 70 L50 88 M80 68 L80 88 M112 70 L110 88" />
          <path d="M40 90 L42 108 M70 88 L70 110 M100 90 L100 108 M124 90 L122 110" />
          <path d="M56 110 L56 130 M104 108 L106 130" />
        </g>
        <rect class="structure-window" x="60" y="80" width="14" height="12" rx="1" />
        <rect class="structure-window" x="86" y="80" width="14" height="12" rx="1" />
      `;
      break;
    case "longhouse":
      body = `
        <path class="structure-roof" d="M8 72 L24 50 L42 30 L78 28 L122 30 L138 50 L154 72 L138 76 L120 50 L78 42 L42 50 L22 76 Z" />
        <path class="structure-body" d="M18 70 L52 68 L98 70 L142 68 L140 102 L142 130 L138 146 L100 144 L52 146 L18 144 L20 110 Z" />
        <path class="structure-arch" d="M68 146 V112 C68 96 96 96 96 112 V146 Z" />
        <g class="structure-bricks">
          <path d="M18 88 L52 90 L100 88 L142 90" />
          <path d="M18 108 L54 106 L100 108 L142 106" />
          <path d="M18 128 L60 130 L100 128 L142 130" />
          <path d="M40 70 L40 88 M70 68 L70 88 M100 70 L100 88 M126 70 L126 88" />
          <path d="M30 88 L30 108 M58 90 L58 108 M84 88 L84 108 M114 88 L114 108 M134 90 L134 108" />
          <path d="M44 108 L44 128 M118 108 L118 128" />
        </g>
        <rect class="structure-window" x="34" y="78" width="14" height="12" rx="1" />
        <rect class="structure-window" x="112" y="78" width="14" height="12" rx="1" />
      `;
      break;
    case "ruin":
      body = `
        <path class="structure-body" d="M16 86 L24 76 L36 80 L46 70 L62 78 L72 70 L84 80 L98 72 L112 84 L126 78 L138 86 L146 76 L146 144 L120 146 L82 144 L42 146 L16 144 Z" />
        <g class="structure-bricks">
          <path d="M16 100 L40 102 L80 100 L120 102 L146 100" />
          <path d="M16 118 L36 120 L80 118 L120 120 L146 118" />
          <path d="M16 134 L42 132 L80 134 L120 132 L146 134" />
          <path d="M30 86 L30 100 M52 78 L52 100 M78 80 L78 100 M104 84 L104 100 M128 86 L128 100" />
          <path d="M22 100 L22 118 M44 100 L44 118 M68 100 L68 118 M92 100 L92 118 M118 100 L118 118 M138 100 L138 118" />
          <path d="M36 118 L36 134 M76 118 L76 134 M114 118 L114 134" />
        </g>
        <path class="structure-debris" d="M-2 144 q5 -4 12 0 t12 0 M48 146 q4 -4 10 0 M82 144 q5 -4 12 0 M120 146 q5 -4 12 0 M150 144 q5 -4 12 0" />
        <path class="structure-debris" d="M28 150 q3 -3 7 0 M104 150 q3 -3 7 0 M70 150 q3 -3 7 0" />
      `;
      break;
    case "burning_shack":
      body = `
        <path class="structure-body" d="M18 78 L26 68 L40 72 L52 64 L66 70 L82 62 L100 72 L120 66 L138 76 L146 68 L146 144 L118 146 L78 144 L40 146 L18 144 Z" />
        <g class="structure-bricks">
          <path d="M18 92 L40 94 L80 92 L118 94 L146 92" />
          <path d="M18 110 L42 108 L80 110 L118 108 L146 110" />
          <path d="M18 128 L46 130 L82 128 L120 130 L146 128" />
          <path d="M36 78 L36 92 M58 70 L58 92 M82 64 L82 92 M104 74 L104 92 M126 72 L126 92" />
          <path d="M28 92 L28 110 M50 92 L50 110 M70 92 L70 110 M94 92 L94 110 M114 92 L114 110 M134 92 L134 110" />
          <path d="M40 110 L40 128 M100 110 L100 128 M124 110 L124 128" />
        </g>
        <path class="structure-arch" d="M64 146 V120 C64 108 96 108 96 120 V146 Z" />
      `;
      break;
    case "hut":
    default:
      body = `
        <path class="structure-roof" d="M14 72 L34 48 L60 26 L80 14 L100 26 L126 48 L146 72 Q146 80 116 80 Q80 78 44 80 Q14 80 14 72 Z" />
        <path class="structure-roof-thatch" d="M30 60 L78 22 M50 50 L78 28 M70 42 L80 30 M104 48 L82 24 M126 60 L82 22 M40 70 L60 40 M120 70 L100 40" />
        <path class="structure-body" d="M22 78 Q22 66 50 66 Q80 64 110 66 Q138 68 138 78 L136 102 L138 130 L134 146 L98 144 L62 146 L26 144 L24 110 Z" />
        <path class="structure-arch" d="M62 146 V112 C62 94 98 94 98 112 V146 Z" />
        <g class="structure-bricks">
          <path d="M22 92 L52 94 L100 92 L138 94" />
          <path d="M22 110 L48 108 L100 110 L138 108" />
          <path d="M22 128 L56 130 L100 128 L138 130" />
          <path d="M44 76 L44 92 M76 76 L76 92 M108 76 L108 92" />
          <path d="M34 92 L34 110 M62 92 L62 110 M98 92 L98 110 M126 92 L126 110" />
          <path d="M50 110 L50 128 M110 110 L110 128" />
        </g>
        <rect class="structure-window" x="34" y="82" width="12" height="10" rx="1" />
        <rect class="structure-window" x="114" y="82" width="12" height="10" rx="1" />
      `;
      break;
  }

  const flames = burning
    ? `
        <path class="structure-flame" d="M22 64
                                          L34 32 L40 56 L52 18 L58 56
                                          L72 8 L76 54 L92 22 L96 56
                                          L110 16 L116 56 L128 30 L132 60
                                          L140 38 L142 62 Z" />
        <path class="structure-flame hot" d="M48 60
                                              L56 32 L62 56 L70 22 L76 56
                                              L88 28 L92 56 L104 34 L110 58
                                              L120 40 L124 60 Z" />
        <path class="structure-flame-cut" d="M40 50 L48 32 M70 48 L80 28 M100 48 L110 28 M122 50 L132 36" />
        <path class="structure-smoke" d="M38 -8 q10 -8 6 -18 M118 -12 q10 -8 6 -20 M78 -10 q8 -10 4 -20" />
      `
    : "";

  return `
    <svg class="structure-svg" viewBox="0 0 160 150" aria-hidden="true"${filterAttr}>
      ${body}
      ${flames}
    </svg>
  `;
}

export function renderTrogdorAgent(session, index, structurePos = null, dragonPose = null, options = {}) {
  const offset = TROGDOR_AGENT_OFFSETS[index % TROGDOR_AGENT_OFFSETS.length];
  const hovered = session.sessionId === options.hoveredSessionId;
  const glyph = escapeHtml(trogdorAgentGlyph(session));
  const label = escapeAttr(`${session.name} ${trogdorDomReason(session)}`);
  const tone = trogdorAgentTone(session);
  const attacking = session.trogdorAwaitingUser && !session.trogdorDismissed && !session.trogdorBurnt;
  const burnt = Boolean(session.trogdorBurnt);
  const dragonTarget = dragonPose || TROGDOR_DRAGON_TARGET;
  const chargeX = structurePos ? (dragonTarget.x - structurePos.x) * 0.82 : 0;
  const chargeY = structurePos ? (dragonTarget.y - structurePos.y) * 0.62 : 0;
  const style = [
    `--ax:${offset.x}px`,
    `--ay:${offset.y}px`,
    `--walk:${900 + index * 110}ms`,
    `--charge:${22000 + index * 1200}ms`,
    `--charge-x:${chargeX.toFixed(2)}vw`,
    `--charge-y:${chargeY.toFixed(2)}vh`,
  ].join("; ");
  const classes = [
    "trogdor-agent",
    `is-${tone}`,
    hovered ? "is-hovered" : "",
    attacking ? "is-attacking" : "",
    burnt ? "is-burnt" : "",
  ].filter(Boolean).join(" ");

  return `
    <button
      type="button"
      class="${classes}"
      style="${style}"
      data-trogdor-agent="true"
      data-session-id="${escapeAttr(session.sessionId)}"
      aria-label="${label}"
    >
      <svg viewBox="0 0 90 130" aria-hidden="true" filter="url(#trogdor-print)">
        <path class="agent-plume" d="M52 18
                                      C56 4 70 0 78 8
                                      C72 10 70 16 70 22
                                      C66 18 60 18 56 22 Z" />
        <path class="agent-sword-blade" d="M76 30 L82 30 L80 84 L78 84 Z" />
        <path class="agent-sword-guard" d="M70 84 H88" />
        <circle class="agent-sword-pommel" cx="79" cy="92" r="3.6" />
        <path class="agent-helm" d="M28 36
                                     C28 18 64 18 64 36
                                     L64 50
                                     L28 50 Z" />
        <path class="agent-helm-slit" d="M34 42 H58" />
        <path class="agent-body" d="M26 50
                                     L66 50
                                     L70 96
                                     L62 96
                                     L60 108
                                     L32 108
                                     L30 96
                                     L22 96 Z" />
        <path class="agent-belt" d="M26 88 H66" />
        <path class="agent-shield" d="M2 56
                                       L24 50
                                       L26 78
                                       L18 96
                                       L10 96
                                       L4 80 Z" />
        <path class="agent-shield-rivet" d="M14 60 v22 M8 70 H22" />
        <path class="agent-leg" d="M34 108 V124 H44 V112" />
        <path class="agent-leg" d="M50 108 V124 H60 V112" />
        <text class="agent-glyph" x="14" y="76" text-anchor="middle">${glyph}</text>
      </svg>
      ${burnt
        ? `
          <span class="agent-burn-flame" aria-hidden="true"></span>
          <span class="agent-burn-smoke" aria-hidden="true"></span>
        `
        : ""}
    </button>
  `;
}

export function renderTrogdorEmptyField(readOnly = false) {
  return `
    <div class="trogdor-empty-field">
      <svg viewBox="0 0 240 180" aria-hidden="true">
        <path class="empty-bone" d="M30 126h78M46 113c-13-13-29 6-15 16-15 11 7 31 18 15M90 113c13-13 29 6 15 16 15 11-7 31-18 15" />
        <path class="empty-house" d="M142 84l40-30 42 30M152 84h62v52h-62zM174 136v-24c0-15 20-15 20 0v24" />
      </svg>
      <p>no repos</p>
      <button type="button" data-action="open_create"${readOnly ? " disabled" : ""}>launch agent</button>
    </div>
  `;
}
