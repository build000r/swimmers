#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"

PORT="${PORT:-3323}"
BASE_URL="http://127.0.0.1:${PORT}"
LOG_FILE="$(mktemp -t swimmers-web-visible.XXXXXX.log)"
SERVER_PID=""
SESSION_ID="${SESSION_ID:-}"
CREATED_SESSION=0
REUSE_SERVER="${SWIMMERS_VISIBLE_REUSE_SERVER:-0}"

cleanup() {
  if [[ "${CREATED_SESSION}" == "1" && -n "${SESSION_ID}" ]]; then
    curl -sS -X DELETE "${BASE_URL}/v1/sessions/${SESSION_ID}?mode=kill_tmux" >/dev/null 2>&1 || true
  fi

  if [[ -n "${SERVER_PID}" ]]; then
    kill "${SERVER_PID}" >/dev/null 2>&1 || true
    wait "${SERVER_PID}" >/dev/null 2>&1 || true
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

  printf 'swimmers visible terminal smoke failed: API did not become ready on %s\n' "${BASE_URL}" >&2
  tail -n 80 "${LOG_FILE}" >&2 || true
  return 1
}

wait_for_session() {
  local session_id="${1:-}"
  local attempt status
  for attempt in {1..30}; do
    status="$(curl -sS -o /dev/null -w '%{http_code}' \
      --connect-timeout 1 \
      --max-time 2 \
      "${BASE_URL}/v1/sessions/${session_id}/snapshot" 2>/dev/null || true)"
    if [[ "${status}" == "200" ]]; then
      return 0
    fi
    sleep 0.25
  done

  printf 'swimmers visible terminal smoke failed: session %s did not become attachable on %s\n' "${session_id}" "${BASE_URL}" >&2
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
  printf 'swimmers visible terminal smoke failed: port %s is already in use\n' "${PORT}" >&2
  exit 1
fi

PKG_DIR="$(swimmers_resolve_frankentui_pkg_dir || true)"
if [[ -z "${PKG_DIR}" ]]; then
  printf 'swimmers visible terminal smoke requires FrankenTerm assets; set SWIMMERS_FRANKENTUI_PKG_DIR or FRANKENTUI_PKG_DIR\n' >&2
  exit 1
fi
export SWIMMERS_FRANKENTUI_PKG_DIR="${PKG_DIR}"

cd "${ROOT_DIR}"

if [[ "${REUSE_SERVER}" != "1" ]]; then
  cargo build -q --bin swimmers >/dev/null
  env PORT="${PORT}" SWIMMERS_FRANKENTUI_PKG_DIR="${SWIMMERS_FRANKENTUI_PKG_DIR}" \
    target/debug/swimmers >"${LOG_FILE}" 2>&1 &
  SERVER_PID=$!
fi

wait_for_api

curl -sS -o /dev/null --fail "${BASE_URL}/assets/frankenterm/FrankenTerm.js"
curl -sS -o /dev/null --fail "${BASE_URL}/assets/frankenterm/FrankenTerm_bg.wasm"

if [[ -z "${SESSION_ID}" ]]; then
  CREATE_RESPONSE="$(curl -sS --fail --json "{\"cwd\":\"${ROOT_DIR}\"}" "${BASE_URL}/v1/sessions")"
  SESSION_ID="$(printf '%s' "${CREATE_RESPONSE}" | extract_json_field 'payload.session.session_id')"
  CREATED_SESSION=1
fi
wait_for_session "${SESSION_ID}"

BASE_URL="${BASE_URL}" SESSION_ID="${SESSION_ID}" node --input-type=module <<'EOF'
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";

const base = process.env.BASE_URL;
const sessionId = process.env.SESSION_ID;
const marker = "webvisible" + Date.now();
const typeIntoTerminal = process.env.SWIMMERS_VISIBLE_TYPE_TEXT !== "0";
const chromeBin =
  process.env.CHROME_BIN ||
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const userDataDir = await mkdtemp(path.join(os.tmpdir(), "swimmers-chrome-"));
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
  const listeners = new Map();

  ws.addEventListener("message", (event) => {
    const payload = JSON.parse(event.data);
    if (payload.id && pending.has(payload.id)) {
      const { resolve, reject } = pending.get(payload.id);
      pending.delete(payload.id);
      if (payload.error) {
        reject(new Error(`${payload.error.message}: ${JSON.stringify(payload.error.data || "")}`));
      } else {
        resolve(payload.result || {});
      }
      return;
    }
    const callbacks = listeners.get(payload.method) || [];
    for (const callback of callbacks) {
      callback(payload.params || {});
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
    once(method) {
      return new Promise((resolve) => {
        const callbacks = listeners.get(method) || [];
        const callback = (params) => {
          listeners.set(method, (listeners.get(method) || []).filter((item) => item !== callback));
          resolve(params);
        };
        callbacks.push(callback);
        listeners.set(method, callbacks);
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

function keyParamsForChar(char) {
  const code = char === " " ? "Space" : /^[a-z]$/.test(char) ? `Key${char.toUpperCase()}` : /^[0-9]$/.test(char) ? `Digit${char}` : "";
  return {
    key: char,
    code,
    text: char,
    unmodifiedText: char,
    windowsVirtualKeyCode: char.charCodeAt(0),
    nativeVirtualKeyCode: char.charCodeAt(0),
  };
}

async function typeText(cdp, text) {
  for (const char of text) {
    const params = keyParamsForChar(char);
    await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", ...params });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: params.key, code: params.code, windowsVirtualKeyCode: params.windowsVirtualKeyCode });
  }
}

async function pressEnter(cdp) {
  const params = { key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, nativeVirtualKeyCode: 13 };
  await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", ...params });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...params });
}

async function waitFor(predicate, label, timeoutMs = 10000) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await predicate();
    if (last) {
      return last;
    }
    await sleep(120);
  }
  throw new Error(`timeout waiting for ${label}; last=${JSON.stringify(last)}`);
}

function visibleStateExpression() {
  return `(() => {
    const fallback = document.getElementById("terminal-fallback");
    const canvas = document.getElementById("terminal-canvas");
    const loading = document.getElementById("loading-overlay");
    const status = document.getElementById("terminal-status-strip")?.textContent || "";
    const fallbackStyle = getComputedStyle(fallback);
    const canvasStyle = getComputedStyle(canvas);
    const loadingStyle = getComputedStyle(loading);
    let brightPixels = 0;
    try {
      const sample = document.createElement("canvas");
      sample.width = Math.min(160, canvas.width || 0);
      sample.height = Math.min(90, canvas.height || 0);
      const context = sample.getContext("2d", { willReadFrequently: true });
      if (context && sample.width && sample.height) {
        context.drawImage(canvas, 0, 0, sample.width, sample.height);
        const pixels = context.getImageData(0, 0, sample.width, sample.height).data;
        for (let index = 0; index < pixels.length; index += 4) {
          if (pixels[index + 3] > 0 && (pixels[index] > 48 || pixels[index + 1] > 48 || pixels[index + 2] > 48)) {
            brightPixels += 1;
          }
        }
      }
    } catch (error) {
      brightPixels = -1;
    }
    return {
      bodyClass: document.body.className,
      activeId: document.activeElement?.id || "",
      status,
      fallbackText: fallback.textContent || "",
      fallbackHidden: fallback.classList.contains("hidden"),
      fallbackDisplay: fallbackStyle.display,
      fallbackOpacity: fallbackStyle.opacity,
      fallbackPointerEvents: fallbackStyle.pointerEvents,
      fallbackRect: Array.from(fallback.getClientRects()).map((rect) => ({ width: rect.width, height: rect.height }))[0] || null,
      canvasHidden: canvas.classList.contains("hidden"),
      canvasDisplay: canvasStyle.display,
      canvasVisibility: canvasStyle.visibility,
      canvasOpacity: canvasStyle.opacity,
      canvasWidth: canvas.width,
      canvasHeight: canvas.height,
      brightPixels,
      loadingVisible: loading.classList.contains("visible"),
      loadingOpacity: loadingStyle.opacity,
    };
  })()`;
}

function visibleTerminalContent(state) {
  const fallbackVisible =
    !state.fallbackHidden &&
    state.fallbackDisplay !== "none" &&
    Number(state.fallbackOpacity) > 0.1 &&
    (typeIntoTerminal ? state.fallbackText.includes(marker) : /\S/.test(state.fallbackText));
  const canvasVisible =
    !state.canvasHidden &&
    state.canvasDisplay !== "none" &&
    state.canvasVisibility !== "hidden" &&
    Number(state.canvasOpacity) > 0.1 &&
    state.brightPixels > 24;
  return fallbackVisible || canvasVisible;
}

let cdp;
try {
  const port = await readDevToolsPort();
  cdp = createCdpClient(await cdpSocketForFirstPage(port));
  await cdp.waitOpen();
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Input.setIgnoreInputEvents", { ignore: false });

  const loadEvent = cdp.once("Page.loadEventFired");
  await cdp.send("Page.navigate", { url: `${base}/?session=${encodeURIComponent(sessionId)}&visibleSmoke=1` });
  await loadEvent;

  await waitFor(async () => {
    const state = await evaluate(cdp, visibleStateExpression());
    return state.bodyClass.includes("terminal-focus-mode") && /live|attached/.test(state.status) ? state : false;
  }, "terminal focus mode and live status");

  let snapshot;
  if (typeIntoTerminal) {
    await evaluate(cdp, `(() => {
      const dock = document.getElementById("terminal-input-dock");
      const input = document.getElementById("terminal-inline-input");
      const fallback = document.getElementById("terminal-fallback");
      const stage = document.getElementById("terminal-stage");
      const target = input && dock && !dock.classList.contains("hidden") ? input : fallback && !fallback.classList.contains("hidden") ? fallback : stage;
      target.focus({ preventScroll: true });
      return document.activeElement?.id || "";
    })()`);
    await typeText(cdp, `echo ${marker}`);
    await pressEnter(cdp);

    snapshot = await waitFor(async () => {
      const payload = await fetch(`${base}/v1/sessions/${encodeURIComponent(sessionId)}/snapshot`).then((response) => response.json());
      return payload.screen_text.includes(marker) ? payload : false;
    }, "tmux snapshot to include browser-typed marker");
  } else {
    snapshot = await waitFor(async () => {
      const payload = await fetch(`${base}/v1/sessions/${encodeURIComponent(sessionId)}/snapshot`).then((response) => response.json());
      return /\S/.test(payload.screen_text || "") ? payload : false;
    }, "tmux snapshot to include terminal content");
  }

  const finalState = await waitFor(async () => {
    const state = await evaluate(cdp, visibleStateExpression());
    return visibleTerminalContent(state) ? state : false;
  }, "browser-visible terminal content after typing", 6000).catch(async (error) => {
    const state = await evaluate(cdp, visibleStateExpression());
    error.message += `; state=${JSON.stringify({
      ...state,
      fallbackText: state.fallbackText.slice(-500),
      snapshotTail: snapshot.screen_text.slice(-500),
      marker,
      typeIntoTerminal,
    })}`;
    throw error;
  });

  console.log(JSON.stringify({
    sessionId,
    marker,
    typed: typeIntoTerminal,
    status: finalState.status,
    bodyClass: finalState.bodyClass,
    fallbackVisible: !finalState.fallbackHidden && finalState.fallbackDisplay !== "none" && Number(finalState.fallbackOpacity) > 0.1,
    fallbackContainsExpectedText: typeIntoTerminal ? finalState.fallbackText.includes(marker) : /\S/.test(finalState.fallbackText),
    canvasBrightPixels: finalState.brightPixels,
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

printf 'web visible terminal smoke passed on %s using %s\n' "${BASE_URL}" "${SESSION_ID}"
