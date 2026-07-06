#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_UP="${ROOT_DIR}/scripts/run-up.sh"

if [[ ! -x "${RUN_UP}" ]]; then
  printf 'expected executable shim at %s\n' "${RUN_UP}" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  printf 'python3 is required for run-up.sh smoke tests\n' >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  printf 'cargo is required for run-up.sh smoke tests\n' >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  printf 'curl is required for run-up.sh smoke tests\n' >&2
  exit 1
fi

if ! command -v lsof >/dev/null 2>&1; then
  printf 'lsof is required for run-up.sh listener smoke tests\n' >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
fixture_pid=""
fixture_port=""

cleanup() {
  if [[ -n "${fixture_pid}" ]] && kill -0 "${fixture_pid}" 2>/dev/null; then
    kill "${fixture_pid}" 2>/dev/null || true
    wait "${fixture_pid}" 2>/dev/null || true
  fi

  if [[ -n "${fixture_port}" ]]; then
    stop_port_listener "${fixture_port}" || true
  fi

  rm -rf "${tmp_dir}"
}
trap cleanup EXIT

help_output="$("${RUN_UP}" --help)"
if [[ "${help_output}" != *"Usage: scripts/run-up.sh"* ]]; then
  printf 'expected run-up.sh help output\n' >&2
  printf '%s\n' "${help_output}" >&2
  exit 1
fi

if "${RUN_UP}" --definitely-bad >"${tmp_dir}/bad-option.out" 2>"${tmp_dir}/bad-option.err"; then
  printf 'expected run-up.sh to fail on unknown option\n' >&2
  exit 1
fi
grep -q "unknown option: --definitely-bad" "${tmp_dir}/bad-option.err"
if grep -q "make up requires FrankenTerm assets" "${tmp_dir}/bad-option.err"; then
  printf 'run-up.sh unknown option reached asset checks before failing\n' >&2
  exit 1
fi

write_server_fixture() {
  local server_py="${tmp_dir}/server_fixture.py"
  cat >"${server_py}" <<'PY'
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

mode = sys.argv[1]
port_file = Path(sys.argv[2]) if sys.argv[2] else None
fixed_port = int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3] else 0
fixed_host = sys.argv[4] if len(sys.argv) > 4 and sys.argv[4] else "127.0.0.1"

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith("/health"):
            if mode in ("ready", "slow-dirs", "missing-dirs"):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"status":"healthy"}')
            else:
                self.send_response(404)
                self.end_headers()
            return

        if self.path.startswith("/app.js"):
            if mode in ("ready", "slow-dirs", "missing-dirs"):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"console.log('swimmers fixture');")
            else:
                self.send_response(404)
                self.end_headers()
            return

        if self.path.startswith("/v1/sessions"):
            if mode in ("ready", "slow-dirs", "missing-dirs"):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"sessions":[]}')
            else:
                self.send_response(404)
                self.end_headers()
            return

        if self.path.startswith("/v1/dirs"):
            if mode == "ready":
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"entries":[]}')
            elif mode == "slow-dirs":
                time.sleep(3)
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"entries":[]}')
            else:
                self.send_response(404)
                self.end_headers()
            return

        if self.path.startswith("/v1/skills"):
            if mode in ("ready", "slow-dirs", "missing-dirs"):
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b'{"skills":[]}')
            else:
                self.send_response(404)
                self.end_headers()
            return

        self.send_response(404)
        self.end_headers()

    def log_message(self, format, *args):
        pass

server = ThreadingHTTPServer((fixed_host, fixed_port), Handler)
if port_file:
    port_file.write_text(str(server.server_port), encoding="utf-8")
server.serve_forever()
PY
  printf '%s\n' "${server_py}"
}

SERVER_FIXTURE="$(write_server_fixture)"

free_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

stop_port_listener() {
  local port="${1:-}"
  [[ -n "${port}" ]] || return 0

  local pids pid
  pids="$(lsof -nP -t -iTCP:"${port}" -sTCP:LISTEN 2>/dev/null || true)"
  [[ -n "${pids}" ]] || return 0

  for pid in ${pids}; do
    kill "${pid}" 2>/dev/null || true
  done

  local _i
  for _i in {1..50}; do
    if ! lsof -nP -t -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done

  printf 'listener on test port %s did not stop\n' "${port}" >&2
  return 1
}

start_fixture() {
  local mode="${1}"
  local argv0="${2:-python-fixture}"
  local port_file="${tmp_dir}/${mode}-${argv0}.port"
  rm -f "${port_file}"

  bash -c 'exec -a "$1" python3 "$2" "$3" "$4" ""' \
    _ "${argv0}" "${SERVER_FIXTURE}" "${mode}" "${port_file}" &
  fixture_pid=$!

  local _i
  for _i in {1..50}; do
    [[ -s "${port_file}" ]] && break
    sleep 0.1
  done

  if [[ ! -s "${port_file}" ]]; then
    printf 'fixture did not publish a port\n' >&2
    exit 1
  fi

  fixture_port="$(cat "${port_file}")"
}

stop_fixture() {
  if [[ -n "${fixture_pid}" ]] && kill -0 "${fixture_pid}" 2>/dev/null; then
    kill "${fixture_pid}" 2>/dev/null || true
    wait "${fixture_pid}" 2>/dev/null || true
  fi
  fixture_pid=""
  fixture_port=""
}

status_for() {
  local url="${1}"
  curl -sS -o /dev/null -w '%{http_code}' \
    --connect-timeout 1 \
    --max-time 2 \
    "${url}" \
    2>/dev/null || true
}

wait_for_ready_backend() {
  local port="${1}"
  local log_file="${2:-}"
  local web_status api_status dirs_status skills_status
  local _i

  for _i in {1..60}; do
    web_status="$(status_for "http://127.0.0.1:${port}/app.js")"
    api_status="$(status_for "http://127.0.0.1:${port}/v1/sessions")"
    dirs_status="$(status_for "http://127.0.0.1:${port}/v1/dirs")"
    skills_status="$(status_for "http://127.0.0.1:${port}/v1/skills?tool=codex")"
    if [[ "${web_status}" == "200" && "${api_status}" == "200" && "${dirs_status}" == "200" && "${skills_status}" == "200" ]]; then
      return 0
    fi
    sleep 0.25
  done

  printf 'backend on %s did not become ready; last /app.js=%s /v1/sessions=%s /v1/dirs=%s /v1/skills=%s\n' \
    "${port}" \
    "${web_status:-000}" \
    "${api_status:-000}" \
    "${dirs_status:-000}" \
    "${skills_status:-000}" >&2
  if [[ -n "${log_file}" ]]; then
    tail -n 80 "${log_file}" >&2 || true
  fi
  return 1
}

make_server_bin_stub() {
  local stub="${tmp_dir}/current-swimmers"
  cat >"${stub}" <<SH
#!/usr/bin/env bash
set -euo pipefail
bind_host="\${SWIMMERS_BIND:-127.0.0.1}"
bind_host="\${bind_host#\\[}"
bind_host="\${bind_host%%\\]*}"
if [[ "\${bind_host}" =~ ^([^:]+):[0-9]+$ ]]; then
  bind_host="\${BASH_REMATCH[1]}"
fi
exec -a swimmers python3 "${SERVER_FIXTURE}" "\${SWIMMERS_UP_TEST_SERVER_MODE:-ready}" "" "\${PORT:?PORT is required}" "\${bind_host}"
SH
  chmod +x "${stub}"
  printf '%s\n' "${stub}"
}

make_tailscale_stub() {
  local dir="${tmp_dir}/fake-bin"
  local stub="${dir}/tailscale"
  mkdir -p "${dir}"
  cat >"${stub}" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "ip" && "${2:-}" == "-4" ]]; then
  printf '%s\n' "${SWIMMERS_UP_TEST_TAILSCALE_IP:-}"
  exit 0
fi
exit 1
SH
  chmod +x "${stub}"
  printf '%s\n' "${dir}"
}

make_tui_stub() {
  local stub="${tmp_dir}/tui-stub.sh"
  cat >"${stub}" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
{
  printf 'SWIMMERS_TUI_URL=%s\n' "${SWIMMERS_TUI_URL:-}"
  printf 'SWIMMERS_TUI_REUSE_SERVER=%s\n' "${SWIMMERS_TUI_REUSE_SERVER:-}"
  printf 'SWIMMERS_TUI_FEATURES=%s\n' "${SWIMMERS_TUI_FEATURES:-}"
  printf 'SWIMMERS_PERSONAL_WORKFLOWS=%s\n' "${SWIMMERS_PERSONAL_WORKFLOWS:-}"
  printf 'CARGO_TARGET_DIR=%s\n' "${CARGO_TARGET_DIR:-}"
} >"${SWIMMERS_UP_TEST_CAPTURE}"
SH
  chmod +x "${stub}"
  printf '%s\n' "${stub}"
}

make_frankenterm_pkg() {
  local pkg="${tmp_dir}/frankentui/pkg"
  mkdir -p "${pkg}"
  touch "${pkg}/FrankenTerm.js" "${pkg}/FrankenTerm_bg.wasm"
  printf '%s\n' "${pkg}"
}

run_up_for_port() {
  local port="${1}"
  local capture="${2}"
  local output="${3}"
  shift 3

  PORT="${port}" \
    CARGO_TARGET_DIR= \
    SWIMMERS_UP_TARGET_DIR= \
    SWIMMERS_FRANKENTUI_PKG_DIR="${FRANKENTERM_PKG}" \
    SWIMMERS_UP_SERVER_BIN="${SERVER_BIN_STUB}" \
    SWIMMERS_UP_SERVER_LOG="${tmp_dir}/up-${port}.log" \
    SWIMMERS_UP_TUI_SHIM="${TUI_STUB}" \
    SWIMMERS_UP_TEST_CAPTURE="${capture}" \
    "${RUN_UP}" "$@" >"${output}"
}

assert_captured_target_dir() {
  local capture="${1}"
  local expected="${2}"
  local actual
  actual="$(sed -n 's/^CARGO_TARGET_DIR=//p' "${capture}")"
  if [[ "$(readlink -f "${actual}")" != "$(readlink -f "${expected}")" ]]; then
    printf 'expected CARGO_TARGET_DIR=%s, got %s\n' "${expected}" "${actual}" >&2
    exit 1
  fi
}

SERVER_BIN_STUB="$(make_server_bin_stub)"
TUI_STUB="$(make_tui_stub)"
FRANKENTERM_PKG="$(make_frankenterm_pkg)"
TAILSCALE_STUB_DIR="$(make_tailscale_stub)"

port="$(free_port)"
capture="${tmp_dir}/tui-env.txt"
PATH="${TAILSCALE_STUB_DIR}:${PATH}" \
  SWIMMERS_UP_TEST_TAILSCALE_IP=100.79.193.34 \
  run_up_for_port "${port}" "${capture}" "${tmp_dir}/fresh.out"
grep -q "swimmers shared backend target URLs" "${tmp_dir}/fresh.out"
grep -q "Using FrankenTerm assets from ${FRANKENTERM_PKG}" "${tmp_dir}/fresh.out"
grep -q "Starting swimmers backend on http://127.0.0.1:${port}" "${tmp_dir}/fresh.out"
grep -q "tailnet:  not exposed (set SWIMMERS_BIND=100.79.193.34 AUTH_MODE=tailnet_trust)" "${tmp_dir}/fresh.out"
if grep -q "tailnet:  http://100.79.193.34:${port}/" "${tmp_dir}/fresh.out"; then
  printf 'default make up should not advertise an unreachable tailnet URL\n' >&2
  exit 1
fi
grep -q "SWIMMERS_TUI_URL=http://127.0.0.1:${port}" "${capture}"
grep -q "SWIMMERS_TUI_REUSE_SERVER=1" "${capture}"
grep -q "SWIMMERS_TUI_FEATURES=" "${capture}"
grep -q "SWIMMERS_PERSONAL_WORKFLOWS=1" "${capture}"
assert_captured_target_dir "${capture}" "${ROOT_DIR}/target/swimmers-up/default"
stop_port_listener "${port}"

port="$(free_port)"
PATH="${TAILSCALE_STUB_DIR}:${PATH}" \
  SWIMMERS_UP_TEST_TAILSCALE_IP=127.0.0.2 \
  SWIMMERS_BIND=127.0.0.2 \
  AUTH_MODE=tailnet_trust \
  run_up_for_port "${port}" "${capture}" "${tmp_dir}/tailnet-bind.out"
grep -q "Starting swimmers backend on http://127.0.0.2:${port}" "${tmp_dir}/tailnet-bind.out"
grep -q "tailnet:  http://127.0.0.2:${port}/" "${tmp_dir}/tailnet-bind.out"
grep -q "SWIMMERS_TUI_URL=http://127.0.0.2:${port}" "${capture}"
stop_port_listener "${port}"

port="$(free_port)"
PATH="${TAILSCALE_STUB_DIR}:${PATH}" \
  SWIMMERS_UP_TEST_TAILSCALE_IP=100.79.193.34 \
  SWIMMERS_BIND=0.0.0.0 \
  AUTH_MODE=token \
  AUTH_TOKEN=secret \
  run_up_for_port "${port}" "${capture}" "${tmp_dir}/wildcard-bind.out"
grep -q "Starting swimmers backend on http://127.0.0.1:${port}" "${tmp_dir}/wildcard-bind.out"
grep -q "tailnet:  http://100.79.193.34:${port}/" "${tmp_dir}/wildcard-bind.out"
grep -q "SWIMMERS_TUI_URL=http://127.0.0.1:${port}" "${capture}"
stop_port_listener "${port}"

port="$(free_port)"
SWIMMERS_UP_TEST_SERVER_MODE=slow-dirs \
  run_up_for_port "${port}" "${capture}" "${tmp_dir}/slow-dirs.out"
grep -q "Backend ready on http://127.0.0.1:${port}" "${tmp_dir}/slow-dirs.out"
grep -q "note: /v1/dirs did not answer within the short startup probe" "${tmp_dir}/slow-dirs.out"
grep -q "SWIMMERS_TUI_URL=http://127.0.0.1:${port}" "${capture}"
stop_port_listener "${port}"

port="$(free_port)"
if SWIMMERS_UP_TEST_SERVER_MODE=missing-dirs \
  run_up_for_port "${port}" "${capture}" "${tmp_dir}/missing-dirs.out" 2>"${tmp_dir}/missing-dirs.err"; then
  printf 'expected run-up.sh to fail when /v1/dirs is explicitly missing\n' >&2
  exit 1
fi
grep -q "required make up route is unavailable" "${tmp_dir}/missing-dirs.err"
grep -q "/v1/dirs=404" "${tmp_dir}/missing-dirs.err"
stop_port_listener "${port}"

port="$(free_port)"
SWIMMERS_TUI_FEATURES=voice \
  run_up_for_port "${port}" "${capture}" "${tmp_dir}/fresh-with-voice.out"
grep -q "SWIMMERS_TUI_FEATURES=voice" "${capture}"
grep -q "SWIMMERS_PERSONAL_WORKFLOWS=1" "${capture}"
stop_port_listener "${port}"

start_fixture ready python-fixture
ready_port="${fixture_port}"
if PORT="${ready_port}" \
  SWIMMERS_FRANKENTUI_PKG_DIR="${FRANKENTERM_PKG}" \
  SWIMMERS_UP_SERVER_BIN="${SERVER_BIN_STUB}" \
  SWIMMERS_UP_SERVER_LOG="${tmp_dir}/refuse-${ready_port}.log" \
  SWIMMERS_UP_TUI_SHIM="${TUI_STUB}" \
  SWIMMERS_UP_TEST_CAPTURE="${capture}" \
  "${RUN_UP}" >"${tmp_dir}/non-swimmers.out" 2>"${tmp_dir}/non-swimmers.err"; then
  printf 'expected run-up.sh to refuse a ready non-swimmers listener\n' >&2
  exit 1
fi
grep -q "not this checkout's swimmers backend" "${tmp_dir}/non-swimmers.err"
kill -0 "${fixture_pid}" 2>/dev/null
stop_fixture

REAL_CARGO_TARGET_DIR="${tmp_dir}/cargo-target"
CARGO_TARGET_DIR="${REAL_CARGO_TARGET_DIR}" cargo build -q --bin swimmers
stale_port="$(free_port)"
real_server_log="${tmp_dir}/real-swimmers-${stale_port}.log"
SWIMMERS_DATA_DIR="${tmp_dir}/real-data" \
  SWIMMERS_FRANKENTUI_PKG_DIR="${FRANKENTERM_PKG}" \
  SWIMMERS_PERSONAL_WORKFLOWS=1 \
  PORT="${stale_port}" \
  "${REAL_CARGO_TARGET_DIR}/debug/swimmers" >"${real_server_log}" 2>&1 &
fixture_pid=$!
fixture_port="${stale_port}"
wait_for_ready_backend "${stale_port}" "${real_server_log}"
PORT="${stale_port}" \
  SWIMMERS_FRANKENTUI_PKG_DIR="${FRANKENTERM_PKG}" \
  SWIMMERS_UP_SERVER_BIN="${SERVER_BIN_STUB}" \
  SWIMMERS_UP_SERVER_LOG="${tmp_dir}/restart-${stale_port}.log" \
  SWIMMERS_UP_TUI_SHIM="${TUI_STUB}" \
  SWIMMERS_UP_TEST_CAPTURE="${capture}" \
  "${RUN_UP}" >"${tmp_dir}/restart.out"
grep -q "may be stale; restarting it to use" "${tmp_dir}/restart.out"
grep -q "SWIMMERS_TUI_URL=http://127.0.0.1:${stale_port}" "${capture}"
if lsof -nP -t -iTCP:"${stale_port}" -sTCP:LISTEN 2>/dev/null | grep -qx "${fixture_pid}"; then
  printf 'expected stale swimmers listener to be stopped\n' >&2
  exit 1
fi
wait "${fixture_pid}" 2>/dev/null || true
fixture_pid=""
stop_port_listener "${stale_port}"
fixture_port=""

missing_port="$(free_port)"
if PORT="${missing_port}" \
  SWIMMERS_FRANKENTUI_AUTO_DISCOVERY=0 \
  SWIMMERS_FRANKENTUI_PKG_DIR="${tmp_dir}/missing-pkg" \
  FRANKENTUI_PKG_DIR="${tmp_dir}/also-missing" \
  SWIMMERS_UP_SERVER_BIN="${SERVER_BIN_STUB}" \
  SWIMMERS_UP_TUI_SHIM="${TUI_STUB}" \
  SWIMMERS_UP_TEST_CAPTURE="${capture}" \
  "${RUN_UP}" >"${tmp_dir}/missing-assets.out" 2>"${tmp_dir}/missing-assets.err"; then
  printf 'expected run-up.sh to fail without FrankenTerm assets\n' >&2
  exit 1
fi
grep -q "make up requires FrankenTerm assets" "${tmp_dir}/missing-assets.err"

printf 'run-up.sh checks passed\n'
