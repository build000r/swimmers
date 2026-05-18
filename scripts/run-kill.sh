#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"

PORT="${PORT:-3210}"

listener_pid() {
  command -v lsof >/dev/null 2>&1 || return 1
  lsof -nP -t -iTCP:"${PORT}" -sTCP:LISTEN 2>/dev/null | head -1 || true
}

listener_command() {
  local pid="${1:-}"
  [[ -n "${pid}" ]] || return 1
  ps -p "${pid}" -o command= 2>/dev/null || true
}

is_swimmers_command() {
  local command_line="${1:-}"
  local argv0
  argv0="${command_line%% *}"
  [[ "${argv0##*/}" == "swimmers" ]]
}

wait_for_listener_to_stop() {
  local _i
  for _i in {1..50}; do
    if [[ -z "$(listener_pid)" ]]; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

main() {
  swimmers_require lsof

  local pid
  pid="$(listener_pid)"
  if [[ -z "${pid}" ]]; then
    printf 'No swimmers backend running on 127.0.0.1:%s\n' "${PORT}"
    return 0
  fi

  local command_line
  command_line="$(listener_command "${pid}")"
  if ! is_swimmers_command "${command_line}"; then
    printf 'Port %s is held by a non-swimmers process (pid %s):\n  %s\n' \
      "${PORT}" "${pid}" "${command_line:-unknown}" >&2
    printf 'Refusing to kill. Stop it yourself if you mean to.\n' >&2
    return 1
  fi

  printf 'Stopping swimmers backend on 127.0.0.1:%s (pid %s)\n' "${PORT}" "${pid}"
  kill "${pid}" 2>/dev/null || true
  if wait_for_listener_to_stop; then
    printf 'Stopped.\n'
    return 0
  fi

  printf 'SIGTERM did not free port %s after 5s; escalating to SIGKILL on pid %s\n' \
    "${PORT}" "${pid}" >&2
  kill -KILL "${pid}" 2>/dev/null || true
  if wait_for_listener_to_stop; then
    printf 'Stopped (SIGKILL).\n'
    return 0
  fi

  printf 'Failed to free port %s; pid %s may still be running.\n' "${PORT}" "${pid}" >&2
  return 1
}

main "$@"
