#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"

PORT="${PORT:-3331}"
BASE_URL="http://127.0.0.1:${PORT}"
LOG_FILE="$(mktemp -t swimmers-web-workbench.XXXXXX.log)"
SERVER_PID=""
SESSION_ID="${SESSION_ID:-}"
CREATED_SESSION=0
REUSE_SERVER="${SWIMMERS_WORKBENCH_REUSE_SERVER:-0}"
FEATURES="${SWIMMERS_WORKBENCH_FEATURES:-personal-workflows}"
SCREENSHOT_PATH="${SWIMMERS_WORKBENCH_SCREENSHOT_PATH:-${ROOT_DIR}/tests/artifacts/web-workbench.png}"
MOBILE_SCREENSHOT_PATH="${SWIMMERS_WORKBENCH_MOBILE_SCREENSHOT_PATH:-${ROOT_DIR}/tests/artifacts/web-workbench-mobile.png}"

cleanup() {
  if [[ "${CREATED_SESSION}" == "1" && -n "${SESSION_ID}" ]]; then
    curl -sS -X DELETE "${BASE_URL}/v1/sessions/${SESSION_ID}?mode=kill_tmux" >/dev/null 2>&1 || true
  fi

  if [[ -n "${SERVER_PID}" ]]; then
    kill "${SERVER_PID}" >/dev/null 2>&1 || true
    for _ in {1..20}; do
      if ! kill -0 "${SERVER_PID}" >/dev/null 2>&1; then
        wait "${SERVER_PID}" >/dev/null 2>&1 || true
        SERVER_PID=""
        break
      fi
      sleep 0.25
    done
    if [[ -n "${SERVER_PID}" ]]; then
      kill -9 "${SERVER_PID}" >/dev/null 2>&1 || true
      wait "${SERVER_PID}" >/dev/null 2>&1 || true
      SERVER_PID=""
    fi
  fi

  rm -f "${LOG_FILE}"
}

wait_for_api() {
  local attempt status
  for attempt in {1..30}; do
    status="$(curl -sS -o /dev/null -w '%{http_code}' \
      --connect-timeout 1 \
      --max-time 2 \
      "${BASE_URL}/v1/sessions" 2>/dev/null || true)"
    if [[ "${status}" == "200" ]]; then
      return 0
    fi
    sleep 1
  done

  printf 'swimmers workbench smoke failed: API did not become ready on %s\n' "${BASE_URL}" >&2
  tail -n 80 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_session() {
  local session_id="${1:-}"
  local attempt status
  for attempt in {1..20}; do
    status="$(curl -sS -o /dev/null -w '%{http_code}' \
      --connect-timeout 1 \
      --max-time 2 \
      "${BASE_URL}/v1/sessions/${session_id}/snapshot" 2>/dev/null || true)"
    if [[ "${status}" == "200" ]]; then
      return 0
    fi
    sleep 0.25
  done

  printf 'swimmers workbench smoke failed: session %s did not become attachable on %s\n' "${session_id}" "${BASE_URL}" >&2
  tail -n 80 "${LOG_FILE}" >&2 || true
  return 1
}

extract_json_field() {
  local expression="${1:-}"
  node -e '
    let input = "";
    process.stdin.on("data", (chunk) => input += chunk);
    process.stdin.on("end", () => {
      const payload = JSON.parse(input);
      const value = (new Function("payload", "return " + process.argv[1]))(payload);
      if (value === undefined || value === null) {
        process.exit(1);
      }
      process.stdout.write(String(value));
    });
  ' "${expression}"
}

trap cleanup EXIT

swimmers_require cargo
swimmers_require curl
swimmers_require node

if [[ "${REUSE_SERVER}" != "1" ]] && command -v lsof >/dev/null 2>&1 && lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
  printf 'swimmers workbench smoke failed: port %s is already in use\n' "${PORT}" >&2
  exit 1
fi

cd "${ROOT_DIR}"

if [[ "${REUSE_SERVER}" != "1" ]]; then
  if [[ -n "${FEATURES}" && "${FEATURES}" != "none" ]]; then
    cargo build -q --bin swimmers --features "${FEATURES}" >/dev/null
  else
    cargo build -q --bin swimmers >/dev/null
  fi
  env PORT="${PORT}" target/debug/swimmers >"${LOG_FILE}" 2>&1 &
  SERVER_PID=$!
fi

wait_for_api

SMOKE_LOG_COMMAND='printf "You ran smoke log\ncargo test\nerror: smoke status\n@@ -1 +1 @@\n+smoke diff\nplain output\n"'

if [[ -z "${SESSION_ID}" ]]; then
  CREATE_BODY="$(node -e 'process.stdout.write(JSON.stringify({ cwd: process.argv[1], initial_request: process.argv[2] }))' "${ROOT_DIR}" "${SMOKE_LOG_COMMAND}")"
  CREATE_RESPONSE="$(curl -sS --fail --json "${CREATE_BODY}" "${BASE_URL}/v1/sessions")"
  SESSION_ID="$(printf '%s' "${CREATE_RESPONSE}" | extract_json_field 'payload.session.session_id')"
  CREATED_SESSION=1
fi

wait_for_session "${SESSION_ID}"

if [[ "${CREATED_SESSION}" == "1" ]]; then
  SMOKE_LOG_BODY="$(node -e 'process.stdout.write(JSON.stringify({ text: process.argv[1], submit: true }))' "${SMOKE_LOG_COMMAND}")"
  curl -sS --fail --json "${SMOKE_LOG_BODY}" "${BASE_URL}/v1/sessions/${SESSION_ID}/input" >/dev/null
  sleep 1
fi

BASE_URL="${BASE_URL}" SESSION_ID="${SESSION_ID}" SCREENSHOT_PATH="${SCREENSHOT_PATH}" MOBILE_SCREENSHOT_PATH="${MOBILE_SCREENSHOT_PATH}" node --input-type=module <<'EOF'
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";

const base = process.env.BASE_URL;
const sessionId = process.env.SESSION_ID;
const screenshotPath = process.env.SCREENSHOT_PATH;
const mobileScreenshotPath = process.env.MOBILE_SCREENSHOT_PATH;
const chromeBin =
  process.env.CHROME_BIN ||
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const userDataDir = await mkdtemp(path.join(os.tmpdir(), "swimmers-workbench-chrome-"));
const chrome = spawn(chromeBin, [
  "--headless=new",
  "--remote-debugging-port=0",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-background-networking",
  "--disable-component-update",
  "--disable-popup-blocking",
  "--window-size=1280,720",
  `--user-data-dir=${userDataDir}`,
  "about:blank",
], { stdio: ["ignore", "ignore", "pipe"] });

let stderr = "";
chrome.stderr.on("data", (chunk) => {
  stderr += chunk.toString();
});

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function readDevToolsPort() {
  const activePath = path.join(userDataDir, "DevToolsActivePort");
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    try {
      const active = await readFile(activePath, "utf8");
      const [port] = active.trim().split(/\n/);
      if (port) {
        return Number(port);
      }
    } catch (_) {
      await sleep(100);
    }
  }
  throw new Error(`Chrome did not expose DevToolsActivePort. stderr=${stderr.slice(-1200)}`);
}

async function cdpSocketForFirstPage(port) {
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) {
    const response = await fetch(`http://127.0.0.1:${port}/json/list`).catch(() => null);
    if (response?.ok) {
      const targets = await response.json();
      const page = targets.find((target) => target.type === "page" && target.webSocketDebuggerUrl);
      if (page) {
        return page.webSocketDebuggerUrl;
      }
    }
    await sleep(100);
  }
  throw new Error("Chrome did not expose a page target");
}

function createCdpClient(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let nextId = 1;
  const pending = new Map();

  ws.addEventListener("message", (event) => {
    const payload = JSON.parse(event.data);
    if (!payload.id || !pending.has(payload.id)) {
      return;
    }
    const { resolve, reject } = pending.get(payload.id);
    pending.delete(payload.id);
    if (payload.error) {
      reject(new Error(`${payload.error.message}: ${JSON.stringify(payload.error.data || "")}`));
    } else {
      resolve(payload.result || {});
    }
  });

  return {
    waitOpen() {
      if (ws.readyState === WebSocket.OPEN) {
        return Promise.resolve();
      }
      return new Promise((resolve, reject) => {
        ws.addEventListener("open", resolve, { once: true });
        ws.addEventListener("error", () => reject(new Error("CDP websocket error")), { once: true });
      });
    },
    send(method, params = {}) {
      const id = nextId++;
      ws.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
      });
    },
    close() {
      ws.close();
    },
  };
}

async function evaluate(cdp, expression) {
  const result = await cdp.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(JSON.stringify(result.exceptionDetails));
  }
  return result.result?.value;
}

async function waitFor(cdp, expression, label, timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await evaluate(cdp, expression);
    if (last?.ok) {
      return last;
    }
    await sleep(120);
  }
  throw new Error(`timeout waiting for ${label}; last=${JSON.stringify(last)}`);
}

async function navigateToWorkbench(cdp) {
  await cdp.send("Page.navigate", {
    url: `${base}/?session=${encodeURIComponent(sessionId)}&workbenchSmoke=1`,
  });
  await sleep(500);
}

async function assertWorkbenchViewport(cdp, label, outputPath, metrics) {
  await cdp.send("Emulation.setDeviceMetricsOverride", metrics);
  await navigateToWorkbench(cdp);
  const finalState = await waitFor(cdp, stateExpression, `${label} workbench widgets`);
  await mkdir(path.dirname(outputPath), { recursive: true });
  const screenshot = await cdp.send("Page.captureScreenshot", { format: "png", fromSurface: true });
  await writeFile(outputPath, Buffer.from(screenshot.data, "base64"));
  const backClickState = await evaluate(cdp, backClickExpression);
  if (!backClickState?.ok) {
    throw new Error(`${label} Trogdor back control failed: ${JSON.stringify(backClickState)}`);
  }
  return {
    label,
    screenshotPath: outputPath,
    widgetText: finalState.widgetText.slice(0, 240),
    noDockOverlap: finalState.noDockOverlap,
    noControlsDockOverlap: finalState.noControlsDockOverlap,
    noWorkbenchControlsOverlap: finalState.noWorkbenchControlsOverlap,
    backVisible: finalState.backVisible,
    noBackWorkbenchOverlap: finalState.noBackWorkbenchOverlap,
    stageRect: finalState.stageRect,
  };
}

const stateExpression = `(() => {
  const workbench = document.getElementById("terminal-workbench");
  const widgets = document.getElementById("terminal-workbench-widgets");
  const dock = document.getElementById("terminal-input-dock");
  const controls = document.getElementById("terminal-control-strip");
  const stage = document.getElementById("terminal-stage");
  const back = document.getElementById("terminal-trogdor-back");
  const keyStrip = document.getElementById("terminal-key-strip");
  if (workbench?.classList.contains("hidden")) {
    document.getElementById("terminal-workbench-toggle")?.click();
  }
  const workbenchRect = workbench?.getBoundingClientRect();
  const dockRect = dock?.getBoundingClientRect();
  const controlsRect = controls?.getBoundingClientRect();
  const stageRect = stage?.getBoundingClientRect();
  const backRect = back?.getBoundingClientRect();
  const widgetText = widgets?.innerText || "";
  const keyStripText = keyStrip?.innerText || "";
  const logLens = widgets?.querySelector(".workbench-log-lens");
  const logRawButton = widgets?.querySelector("[data-workbench-log-mode='raw']");
  const logSearch = widgets?.querySelector("[data-workbench-log-search]");
  const logFilter = widgets?.querySelector("[data-workbench-log-filter]");
  const requiredPanelLabels = ["Activity", "Diffs", "Logs", "Artifacts", "Skills"];
  const panelNodes = [...(widgets?.querySelectorAll(".workbench-widget") || [])];
  const panelByLabel = (label) => panelNodes.find((panel) => panel.querySelector(".workbench-widget-title")?.innerText?.trim() === label);
  const hasRequiredPanels = requiredPanelLabels.every((label) => Boolean(panelByLabel(label)));
  const panelsHaveContent = requiredPanelLabels.every((label) => {
    const panel = panelByLabel(label);
    const bodyText = panel?.querySelector(".workbench-widget-body")?.textContent?.trim() || "";
    const summaryText = panel?.querySelector("summary")?.textContent?.trim() || "";
    return bodyText.length > 0 || summaryText.length > label.length;
  });
  const backVisible = Boolean(back && !back.classList.contains("hidden") && !back.disabled);
  const keyStripVisible = Boolean(keyStrip && !keyStrip.closest(".hidden") && keyStripText.includes("Ctrl-C") && keyStripText.includes("Esc"));
  const noDockOverlap = Boolean(workbenchRect && dockRect && workbenchRect.bottom <= dockRect.top + 1);
  const noControlsDockOverlap = Boolean(controlsRect && dockRect && controlsRect.bottom <= dockRect.top + 1);
  const noWorkbenchControlsOverlap = Boolean(
    workbenchRect &&
    controlsRect &&
    (
      workbenchRect.right <= controlsRect.left ||
      controlsRect.right <= workbenchRect.left ||
      workbenchRect.bottom <= controlsRect.top ||
      controlsRect.bottom <= workbenchRect.top
    )
  );
  const noBackWorkbenchOverlap = Boolean(
    backRect &&
    workbenchRect &&
    (
      backRect.right <= workbenchRect.left ||
      workbenchRect.right <= backRect.left ||
      backRect.bottom <= workbenchRect.top ||
      workbenchRect.bottom <= backRect.top
    )
  );
  return {
    ok:
      document.body.classList.contains("terminal-focus-mode") &&
      workbench &&
      !workbench.classList.contains("hidden") &&
      backVisible &&
      keyStripVisible &&
      hasRequiredPanels &&
      panelsHaveContent &&
      widgetText.includes("Recent output") &&
      logLens &&
      logRawButton &&
      logSearch &&
      logFilter &&
      noDockOverlap &&
      noControlsDockOverlap &&
      noWorkbenchControlsOverlap &&
      noBackWorkbenchOverlap,
    bodyClass: document.body.className,
    widgetText,
    hasLogLens: Boolean(logLens),
    hasLogRawButton: Boolean(logRawButton),
    hasLogSearch: Boolean(logSearch),
    hasLogFilter: Boolean(logFilter),
    hasRequiredPanels,
    panelsHaveContent,
    backVisible,
    keyStripVisible,
    noDockOverlap,
    noControlsDockOverlap,
    noWorkbenchControlsOverlap,
    noBackWorkbenchOverlap,
    workbenchRect: workbenchRect && { top: workbenchRect.top, right: workbenchRect.right, bottom: workbenchRect.bottom, left: workbenchRect.left },
    dockRect: dockRect && { top: dockRect.top, bottom: dockRect.bottom },
    controlsRect: controlsRect && { top: controlsRect.top, right: controlsRect.right, bottom: controlsRect.bottom, left: controlsRect.left },
    backRect: backRect && { top: backRect.top, right: backRect.right, bottom: backRect.bottom, left: backRect.left },
    stageRect: stageRect && { width: stageRect.width, height: stageRect.height },
  };
})()`;

const backClickExpression = `(() => {
  const back = document.getElementById("terminal-trogdor-back");
  const workbench = document.getElementById("terminal-workbench");
  const surface = document.getElementById("trogdor-surface");
  if (!back || back.classList.contains("hidden") || back.disabled) {
    return { ok: false, reason: "back hidden or disabled" };
  }
  back.click();
  return {
    ok:
      document.body.classList.contains("trogdor-mode") &&
      surface &&
      !surface.classList.contains("hidden") &&
      workbench &&
      workbench.classList.contains("hidden") &&
      back.classList.contains("hidden"),
    bodyClass: document.body.className,
    surfaceHidden: surface?.classList.contains("hidden"),
    workbenchHidden: workbench?.classList.contains("hidden"),
    backHidden: back.classList.contains("hidden"),
  };
})()`;

let cdp;
try {
  const port = await readDevToolsPort();
  cdp = createCdpClient(await cdpSocketForFirstPage(port));
  await cdp.waitOpen();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");

  const desktop = await assertWorkbenchViewport(cdp, "desktop", screenshotPath, {
    width: 1280,
    height: 720,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const mobile = await assertWorkbenchViewport(cdp, "mobile", mobileScreenshotPath, {
    width: 390,
    height: 844,
    deviceScaleFactor: 2,
    mobile: true,
  });
  console.log(JSON.stringify({
    sessionId,
    screenshots: [desktop.screenshotPath, mobile.screenshotPath],
    desktop,
    mobile,
  }));
} finally {
  cdp?.close();
  if (!chrome.killed) {
    chrome.kill("SIGTERM");
  }
  await Promise.race([
    new Promise((resolve) => chrome.once("exit", resolve)),
    sleep(1500),
  ]);
  await rm(userDataDir, { recursive: true, force: true, maxRetries: 6, retryDelay: 120 });
}
EOF

printf 'web workbench smoke passed on %s using %s; screenshots: %s %s\n' "${BASE_URL}" "${SESSION_ID}" "${SCREENSHOT_PATH}" "${MOBILE_SCREENSHOT_PATH}"
