#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"

PORT="${PORT:-3322}"
BASE_URL="http://127.0.0.1:${PORT}"
LOG_FILE="$(mktemp -t swimmers-web-smoke.XXXXXX.log)"
SERVER_PID=""
SESSION_ID=""

cleanup() {
  if [[ -n "${SESSION_ID}" ]]; then
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
  for attempt in {1..20}; do
    status="$(curl -sS -o /dev/null -w '%{http_code}' \
      --connect-timeout 1 \
      --max-time 2 \
      "${BASE_URL}/v1/sessions" 2>/dev/null || true)"
    if [[ "${status}" == "200" ]]; then
      return 0
    fi
    sleep 1
  done

  printf 'swimmers web smoke failed: API did not become ready on %s\n' "${BASE_URL}" >&2
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

  printf 'swimmers web smoke failed: session %s did not become attachable on %s\n' "${session_id}" "${BASE_URL}" >&2
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

if command -v lsof >/dev/null 2>&1 && lsof -nP -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
  printf 'swimmers web smoke failed: port %s is already in use\n' "${PORT}" >&2
  exit 1
fi

PKG_DIR="$(swimmers_resolve_frankentui_pkg_dir || true)"
if [[ -z "${PKG_DIR}" ]]; then
  printf 'swimmers web smoke requires FrankenTerm assets; set SWIMMERS_FRANKENTUI_PKG_DIR or FRANKENTUI_PKG_DIR\n' >&2
  exit 1
fi
export SWIMMERS_FRANKENTUI_PKG_DIR="${PKG_DIR}"

cd "${ROOT_DIR}"
cargo build -q --bin swimmers >/dev/null

env PORT="${PORT}" SWIMMERS_FRANKENTUI_PKG_DIR="${SWIMMERS_FRANKENTUI_PKG_DIR}" \
  target/debug/swimmers >"${LOG_FILE}" 2>&1 &
SERVER_PID=$!

wait_for_api

curl -sS -o /dev/null --fail "${BASE_URL}/assets/frankenterm/FrankenTerm.js"
curl -sS -o /dev/null --fail "${BASE_URL}/assets/frankenterm/FrankenTerm_bg.wasm"

CREATE_RESPONSE="$(curl -sS --fail --json "{\"cwd\":\"${ROOT_DIR}\"}" "${BASE_URL}/v1/sessions")"
SESSION_ID="$(printf '%s' "${CREATE_RESPONSE}" | extract_json_field 'payload.session.session_id')"
wait_for_session "${SESSION_ID}"

BASE_URL="${BASE_URL}" SESSION_ID="${SESSION_ID}" node --input-type=module <<'EOF'
const base = process.env.BASE_URL;
const sessionId = process.env.SESSION_ID;
const marker = 'WEB_SMOKE_' + Date.now();
const ws = new WebSocket(base.replace(/^http/, 'ws') + '/ws/sessions/' + sessionId);
let sawReady = false;
let sawPong = false;
let markerInFrame = false;
let binaryFrames = 0;
const deadline = Date.now() + 5000;

const done = new Promise((resolve, reject) => {
  const timer = setInterval(() => {
    if (Date.now() > deadline) {
      clearInterval(timer);
      reject(new Error('timeout waiting for live terminal marker'));
      return;
    }
    if (sawReady && markerInFrame) {
      clearInterval(timer);
      resolve();
    }
  }, 50);

  ws.addEventListener('message', async (event) => {
    if (typeof event.data === 'string') {
      const parsed = JSON.parse(event.data);
      if (parsed.type === 'ready') {
        sawReady = true;
        ws.send(JSON.stringify({ type: 'ping' }));
        await fetch(base + '/v1/sessions/' + sessionId + '/input', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ text: 'printf "' + marker + '\\n"\\n' }),
        });
      }
      if (parsed.type === 'pong') {
        sawPong = true;
      }
      return;
    }

    binaryFrames += 1;
    const text = Buffer.from(await event.data.arrayBuffer()).toString('utf8');
    if (text.includes(marker)) {
      markerInFrame = true;
    }
  });

  ws.addEventListener('error', (event) => {
    reject(new Error(event.message || 'websocket error'));
  });
});

await done;

const snapshot = await fetch(base + '/v1/sessions/' + sessionId + '/snapshot').then((response) => response.json());
const hasMarker = snapshot.screen_text.includes(marker);
if (!sawReady || !sawPong || !markerInFrame || !hasMarker) {
  console.error(JSON.stringify({ sawReady, sawPong, markerInFrame, hasMarker, binaryFrames, marker }));
  process.exit(1);
}

console.log(JSON.stringify({ sessionId, binaryFrames, marker }));
ws.close();
setTimeout(() => process.exit(0), 100);
EOF

printf 'web live terminal smoke passed on %s using %s\n' "${BASE_URL}" "${SESSION_ID}"
