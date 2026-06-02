#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"
cd "${ROOT_DIR}"

PORT="${PORT:-3210}"
BASE_URL="http://127.0.0.1:${PORT}"
WEB_ROUTE_PATH="${WEB_ROUTE_PATH:-/app.js}"
HEALTH_ROUTE_PATH="${HEALTH_ROUTE_PATH:-/health}"
API_ROUTE_PATH="${API_ROUTE_PATH:-/v1/sessions}"
DIRS_ROUTE_PATH="${DIRS_ROUTE_PATH:-/v1/dirs}"
SKILLS_ROUTE_PATH="${SKILLS_ROUTE_PATH:-/v1/skills?tool=codex}"
SERVER_LOG="${SWIMMERS_UP_SERVER_LOG:-${TMPDIR:-/tmp}/swimmers-up-${PORT}.log}"
RUN_TUI="${SWIMMERS_UP_TUI_SHIM:-${ROOT_DIR}/scripts/run-tui.sh}"

if [[ -n "${SWIMMERS_UP_FEATURES+x}" ]]; then
  UP_FEATURES="${SWIMMERS_UP_FEATURES}"
elif [[ -n "${SWIMMERS_TUI_FEATURES:-}" ]]; then
  UP_FEATURES="${SWIMMERS_TUI_FEATURES}"
else
  UP_FEATURES=""
fi

feature_args=()
if [[ -n "${UP_FEATURES}" ]]; then
  feature_args=(--features "${UP_FEATURES}")
fi

target_slug() {
  local value="${1:-default}"
  value="${value//,/+}"
  value="${value//[^A-Za-z0-9._+-]/-}"
  printf '%s\n' "${value:-default}"
}

if [[ -n "${SWIMMERS_UP_TARGET_DIR:-}" ]]; then
  UP_TARGET_DIR="${SWIMMERS_UP_TARGET_DIR}"
elif [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  UP_TARGET_DIR="${CARGO_TARGET_DIR}"
else
  UP_TARGET_DIR="${ROOT_DIR}/target/swimmers-up/$(target_slug "${UP_FEATURES}")"
fi

if [[ "${UP_TARGET_DIR}" != /* ]]; then
  UP_TARGET_DIR="${ROOT_DIR}/${UP_TARGET_DIR}"
fi

announce_urls() {
  printf 'swimmers shared backend\n'
  printf '  tui api:  %s\n' "${BASE_URL}"
  printf '  local:    http://127.0.0.1:%s/\n' "${PORT}"
  printf '  selected: http://127.0.0.1:%s/selected\n' "${PORT}"

  if command -v tailscale >/dev/null 2>&1; then
    local tailnet_ip
    tailnet_ip="$(tailscale ip -4 2>/dev/null | head -1 || true)"
    if [[ -n "${tailnet_ip}" ]]; then
      printf '  tailnet:  http://%s:%s/\n' "${tailnet_ip}" "${PORT}"
      printf '  focused:  http://%s:%s/selected\n' "${tailnet_ip}" "${PORT}"
    fi
  fi

  printf '\n'
}

status_for() {
  local url="${1}"
  local max_time="${2:-2}"
  curl -sS -o /dev/null -w '%{http_code}' \
    --connect-timeout 1 \
    --max-time "${max_time}" \
    "${url}" \
    2>/dev/null || true
}

health_route_status() {
  status_for "${BASE_URL}${HEALTH_ROUTE_PATH}"
}

web_route_status() {
  status_for "${BASE_URL}${WEB_ROUTE_PATH}"
}

api_route_status() {
  status_for "${BASE_URL}${API_ROUTE_PATH}"
}

dirs_route_status() {
  status_for "${BASE_URL}${DIRS_ROUTE_PATH}"
}

skills_route_status() {
  status_for "${BASE_URL}${SKILLS_ROUTE_PATH}"
}

api_status_looks_like_swimmers() {
  case "${1:-}" in
    2??|401|403) return 0 ;;
    *) return 1 ;;
  esac
}

personal_workflows_enabled() {
  case "${SWIMMERS_PERSONAL_WORKFLOWS:-1}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off|disabled|DISABLED|Disabled) return 1 ;;
    *) return 0 ;;
  esac
}

backend_is_ready() {
  local web_status="${1:-}"
  local api_status="${2:-}"
  local dirs_status="${3:-}"
  local skills_status="${4:-}"
  local health_status="${5:-}"

  api_status_looks_like_swimmers "${health_status}" \
    && [[ "${web_status}" == "200" ]] \
    && api_status_looks_like_swimmers "${api_status}" || return 1

  if personal_workflows_enabled; then
    dirs_status_looks_compatible "${dirs_status}" \
      && api_status_looks_like_swimmers "${skills_status}"
  else
    return 0
  fi
}

dirs_status_looks_compatible() {
  case "${1:-}" in
    000) return 0 ;;
    *) api_status_looks_like_swimmers "${1:-}" ;;
  esac
}

status_is_explicit_failure() {
  local status="${1:-}"
  shift
  case "${status}" in
    ""|000) return 1 ;;
  esac

  local accepted
  for accepted in "$@"; do
    case "${accepted}" in
      api)
        api_status_looks_like_swimmers "${status}" && return 1
        ;;
      dirs)
        dirs_status_looks_compatible "${status}" && return 1
        ;;
      exact:*)
        [[ "${status}" == "${accepted#exact:}" ]] && return 1
        ;;
    esac
  done

  return 0
}

backend_has_explicit_route_failure() {
  local web_status="${1:-}"
  local api_status="${2:-}"
  local dirs_status="${3:-}"
  local skills_status="${4:-}"
  local health_status="${5:-}"

  api_status_looks_like_swimmers "${health_status}" || return 1
  status_is_explicit_failure "${web_status}" exact:200 && return 0
  status_is_explicit_failure "${api_status}" api && return 0
  if personal_workflows_enabled; then
    status_is_explicit_failure "${dirs_status}" dirs && return 0
    status_is_explicit_failure "${skills_status}" api && return 0
  fi
  return 1
}

port_has_listener() {
  if command -v lsof >/dev/null 2>&1 \
    && lsof -nP -t -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
    return 0
  fi

  (: <"/dev/tcp/127.0.0.1/${PORT}") >/dev/null 2>&1
}

listener_summary() {
  if ! command -v lsof >/dev/null 2>&1; then
    printf 'unknown process'
    return 0
  fi

  local pid command_line
  pid="$(lsof -nP -t -iTCP:"${PORT}" -sTCP:LISTEN 2>/dev/null | head -1 || true)"
  if [[ -z "${pid}" ]]; then
    printf 'unknown process'
    return 0
  fi

  command_line="$(ps -p "${pid}" -o command= 2>/dev/null || true)"
  printf 'pid %s (%s)' "${pid}" "${command_line:-unknown command}"
}

listener_pid() {
  if ! command -v lsof >/dev/null 2>&1; then
    return 1
  fi

  lsof -nP -t -iTCP:"${PORT}" -sTCP:LISTEN 2>/dev/null | head -1 || true
}

listener_command() {
  local pid="${1:-}"
  [[ -n "${pid}" ]] || return 1
  ps -p "${pid}" -o command= 2>/dev/null || true
}

listener_executable_path() {
  local pid="${1:-}"
  [[ -n "${pid}" ]] || return 1
  command -v lsof >/dev/null 2>&1 || return 1
  lsof -nP -a -p "${pid}" -d txt -Fn 2>/dev/null | sed -n 's/^n//p' | head -1
}

is_swimmers_command() {
  local command_line="${1:-}"
  local argv0
  argv0="${command_line%% *}"
  [[ "${argv0##*/}" == "swimmers" ]]
}

listener_matches_binary() {
  local pid="${1:-}"
  local server_bin="${2:-}"
  [[ -n "${pid}" && -x "${server_bin}" ]] || return 1

  local executable
  executable="$(listener_executable_path "${pid}")"
  if [[ -n "${executable}" && -e "${executable}" && "${executable}" -ef "${server_bin}" ]]; then
    return 0
  fi

  return 1
}

wait_for_listener_to_stop() {
  local _i
  for _i in {1..50}; do
    if ! port_has_listener; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

stop_swimmers_listener() {
  local pid="${1}"
  printf 'Restarting incompatible local swimmers backend on 127.0.0.1:%s (pid %s)\n' \
    "${PORT}" \
    "${pid}"
  kill "${pid}" 2>/dev/null || true
  if wait_for_listener_to_stop; then
    return 0
  fi

  printf 'listener on 127.0.0.1:%s did not stop after signal\n' "${PORT}" >&2
  return 1
}

server_binary_path() {
  local target_dir="${UP_TARGET_DIR}"
  if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
    target_dir="${target_dir%/}/${CARGO_BUILD_TARGET}"
  fi

  printf '%s/debug/swimmers\n' "${target_dir%/}"
}

binary_is_current() {
  local binary="${1:-}"
  [[ -x "${binary}" ]] || return 1
  build_stamp_matches || return 1
  local stamp
  stamp="$(build_stamp_path)"

  local inputs=(Cargo.toml Cargo.lock src)
  [[ -f build.rs ]] && inputs+=(build.rs)
  [[ -d .cargo ]] && inputs+=(.cargo)

  [[ -z "$(find "${inputs[@]}" -type f -newer "${stamp}" -print -quit)" ]]
}

build_stamp_path() {
  printf '%s/.swimmers-up-build-stamp\n' "${UP_TARGET_DIR}"
}

build_stamp_matches() {
  local stamp
  stamp="$(build_stamp_path)"
  [[ -f "${stamp}" ]] || return 1
  grep -qx "features=${UP_FEATURES}" "${stamp}" || return 1
  grep -qx "cargo_build_target=${CARGO_BUILD_TARGET:-}" "${stamp}" || return 1
}

write_build_stamp() {
  local stamp
  stamp="$(build_stamp_path)"
  mkdir -p "$(dirname "${stamp}")"
  {
    printf 'features=%s\n' "${UP_FEATURES}"
    printf 'cargo_build_target=%s\n' "${CARGO_BUILD_TARGET:-}"
  } >"${stamp}"
}

resolve_server_binary() {
  if [[ -n "${SWIMMERS_UP_SERVER_BIN:-}" ]]; then
    if [[ ! -x "${SWIMMERS_UP_SERVER_BIN}" ]]; then
      printf 'SWIMMERS_UP_SERVER_BIN is not executable: %s\n' "${SWIMMERS_UP_SERVER_BIN}" >&2
      return 1
    fi
    printf '%s\n' "${SWIMMERS_UP_SERVER_BIN}"
    return 0
  fi

  swimmers_require cargo
  local server_bin
  server_bin="$(server_binary_path)"
  if binary_is_current "${server_bin}"; then
    printf 'Using current swimmers backend binary %s\n' "${server_bin}" >&2
  else
    printf 'Building swimmers backend into %s\n' "${UP_TARGET_DIR}" >&2
    CARGO_TARGET_DIR="${UP_TARGET_DIR}" cargo build --bin swimmers "${feature_args[@]}"
    write_build_stamp
  fi
  if [[ ! -x "${server_bin}" ]]; then
    printf 'expected built swimmers binary at %s\n' "${server_bin}" >&2
    return 1
  fi
  printf '%s\n' "${server_bin}"
}

start_backend() {
  local server_bin="${1:-}"
  if [[ -z "${server_bin}" ]]; then
    server_bin="$(resolve_server_binary)"
  fi

  printf 'Starting swimmers backend on %s\n' "${BASE_URL}"
  printf '  log: %s\n' "${SERVER_LOG}"
  mkdir -p "$(dirname "${SERVER_LOG}")"

  PORT="${PORT}" nohup "${server_bin}" >"${SERVER_LOG}" 2>&1 &
  local server_pid=$!
  disown "${server_pid}" 2>/dev/null || true

  wait_for_backend "${server_pid}"
}

wait_for_backend() {
  local server_pid="${1}"
  local deadline=$((SECONDS + ${SWIMMERS_UP_WAIT_SECONDS:-15}))
  local health_status web_status api_status dirs_status skills_status

  while (( SECONDS <= deadline )); do
    if ! kill -0 "${server_pid}" 2>/dev/null; then
      printf 'swimmers backend exited before it was ready. See log: %s\n' "${SERVER_LOG}" >&2
      return 1
    fi

    health_status="$(health_route_status)"
    web_status="$(web_route_status)"
    api_status="$(api_route_status)"
    dirs_status="$(dirs_route_status)"
    skills_status="$(skills_route_status)"
    if backend_is_ready "${web_status}" "${api_status}" "${dirs_status}" "${skills_status}" "${health_status}"; then
      printf 'Backend ready on %s (pid %s)\n\n' "${BASE_URL}" "${server_pid}"
      if personal_workflows_enabled && [[ "${dirs_status}" == "000" ]]; then
        printf '  note: %s did not answer within the short startup probe; continuing because %s is healthy.\n\n' \
          "${DIRS_ROUTE_PATH}" \
          "${HEALTH_ROUTE_PATH}"
      fi
      return 0
    fi

    if backend_has_explicit_route_failure "${web_status}" "${api_status}" "${dirs_status}" "${skills_status}" "${health_status}"; then
      printf 'swimmers backend is running on %s, but a required make up route is unavailable; last %s=%s, %s=%s, %s=%s, %s=%s, %s=%s. See log: %s\n' \
        "${BASE_URL}" \
        "${HEALTH_ROUTE_PATH}" \
        "${health_status:-000}" \
        "${WEB_ROUTE_PATH}" \
        "${web_status:-000}" \
        "${API_ROUTE_PATH}" \
        "${api_status:-000}" \
        "${DIRS_ROUTE_PATH}" \
        "${dirs_status:-000}" \
        "${SKILLS_ROUTE_PATH}" \
        "${skills_status:-000}" \
        "${SERVER_LOG}" >&2
      return 1
    fi

    sleep 0.25
  done

  printf 'Timed out waiting for swimmers backend on %s; last %s=%s, %s=%s, %s=%s, %s=%s, %s=%s. See log: %s\n' \
    "${BASE_URL}" \
    "${HEALTH_ROUTE_PATH}" \
    "${health_status:-000}" \
    "${WEB_ROUTE_PATH}" \
    "${web_status:-000}" \
    "${API_ROUTE_PATH}" \
    "${api_status:-000}" \
    "${DIRS_ROUTE_PATH}" \
    "${dirs_status:-000}" \
    "${SKILLS_ROUTE_PATH}" \
    "${skills_status:-000}" \
    "${SERVER_LOG}" >&2
  return 1
}

ensure_backend() {
  local server_bin health_status web_status api_status dirs_status skills_status
  server_bin="$(resolve_server_binary)"

  if ! port_has_listener; then
    start_backend "${server_bin}"
    return
  fi

  health_status="$(health_route_status)"
  web_status="$(web_route_status)"
  api_status="$(api_route_status)"
  dirs_status="$(dirs_route_status)"
  skills_status="$(skills_route_status)"

  local pid command_line
  pid="$(listener_pid)"
  command_line="$(listener_command "${pid}")"
  if [[ -n "${pid}" ]] && is_swimmers_command "${command_line}"; then
    if backend_is_ready "${web_status}" "${api_status}" "${dirs_status}" "${skills_status}" "${health_status}"; then
      if listener_matches_binary "${pid}" "${server_bin}"; then
        printf 'Existing swimmers backend on 127.0.0.1:%s is current; reusing pid %s.\n' \
          "${PORT}" \
          "${pid}"
        if personal_workflows_enabled && [[ "${dirs_status}" == "000" ]]; then
          printf '  note: %s did not answer within the short startup probe; continuing because %s is healthy.\n' \
            "${DIRS_ROUTE_PATH}" \
            "${HEALTH_ROUTE_PATH}"
        fi
        return
      fi
      printf 'Existing swimmers backend on 127.0.0.1:%s may be stale; restarting it to use %s.\n' \
        "${PORT}" \
        "${server_bin}"
    else
      printf 'Existing swimmers backend on 127.0.0.1:%s is missing required make up routes.\n' \
        "${PORT}"
      printf '  %s returned %s; %s returned %s; %s returned %s; %s returned %s; %s returned %s\n' \
        "${HEALTH_ROUTE_PATH}" \
        "${health_status:-000}" \
        "${WEB_ROUTE_PATH}" \
        "${web_status:-000}" \
        "${API_ROUTE_PATH}" \
        "${api_status:-000}" \
        "${DIRS_ROUTE_PATH}" \
        "${dirs_status:-000}" \
        "${SKILLS_ROUTE_PATH}" \
        "${skills_status:-000}"
    fi
    stop_swimmers_listener "${pid}"
    start_backend "${server_bin}"
    return
  fi

  printf 'Port %s already has a listener (%s), but it is not this checkout'\''s swimmers backend.\n' \
    "${PORT}" \
    "$(listener_summary)" >&2
  printf '  %s returned %s; %s returned %s; %s returned %s; %s returned %s; %s returned %s\n' \
    "${HEALTH_ROUTE_PATH}" \
    "${health_status:-000}" \
    "${WEB_ROUTE_PATH}" \
    "${web_status:-000}" \
    "${API_ROUTE_PATH}" \
    "${api_status:-000}" \
    "${DIRS_ROUTE_PATH}" \
    "${dirs_status:-000}" \
    "${SKILLS_ROUTE_PATH}" \
    "${skills_status:-000}" >&2
  printf 'make up will not restart or kill that listener. Stop it yourself or choose another PORT.\n' >&2
  return 1
}

main() {
  swimmers_require curl

  local pkg_dir=""
  if pkg_dir="$(swimmers_resolve_frankentui_pkg_dir)"; then
    export SWIMMERS_FRANKENTUI_PKG_DIR="${pkg_dir}"
    printf 'Using FrankenTerm assets from %s\n' "${SWIMMERS_FRANKENTUI_PKG_DIR}"
  else
    printf 'make up requires FrankenTerm assets for live browser terminal rendering.\n' >&2
    printf 'Set SWIMMERS_FRANKENTUI_PKG_DIR=/path/to/frankentui/pkg or FRANKENTUI_PKG_DIR=/path/to/frankentui/pkg.\n' >&2
    return 1
  fi

  announce_urls
  export SWIMMERS_PERSONAL_WORKFLOWS="${SWIMMERS_PERSONAL_WORKFLOWS:-1}"
  ensure_backend

  printf 'Launching TUI against %s\n\n' "${BASE_URL}"
  CARGO_TARGET_DIR="${UP_TARGET_DIR}" \
    SWIMMERS_TUI_URL="${BASE_URL}" \
    SWIMMERS_TUI_REUSE_SERVER=1 \
    SWIMMERS_TUI_FEATURES="${UP_FEATURES}" \
    "${RUN_TUI}" "$@"
}

main "$@"
