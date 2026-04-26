#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

truthy() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

is_help_or_version() {
  for arg in "$@"; do
    case "${arg}" in
      -h|--help|-V|--version) return 0 ;;
    esac
  done
  return 1
}

loopback_port_from_url() {
  local url="${1}"
  local scheme="${url%%://*}"
  local rest="${url#*://}"
  local authority="${rest%%/*}"
  local host port

  if [[ "${url}" == "${rest}" ]]; then
    return 1
  fi

  if [[ "${authority}" == \[*\]* ]]; then
    host="${authority%%]*}"
    host="${host#[}"
    port="${authority##*]:}"
    [[ "${port}" == "${authority}" ]] && port=""
  else
    host="${authority%%:*}"
    port="${authority#*:}"
    [[ "${port}" == "${authority}" ]] && port=""
  fi

  case "${host}" in
    localhost|127.*|::1) ;;
    *) return 1 ;;
  esac

  if [[ -z "${port}" ]]; then
    case "${scheme}" in
      http) port=80 ;;
      https) port=443 ;;
      *) return 1 ;;
    esac
  fi

  [[ "${port}" =~ ^[0-9]+$ ]] || return 1
  printf '%s\n' "${port}"
}

pid_command() {
  local pid="${1}"
  ps -p "${pid}" -o command= 2>/dev/null || true
}

is_swimmers_server_command() {
  local command_line="${1}"
  local argv0
  argv0="${command_line%% *}"
  [[ "${argv0##*/}" == "swimmers" ]]
}

wait_for_pid_to_exit() {
  local pid="${1}"
  local _i
  for _i in {1..30}; do
    if ! kill -0 "${pid}" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

restart_loopback_swimmers_if_needed() {
  if truthy "${SWIMMERS_TUI_REUSE_SERVER:-}"; then
    return 0
  fi

  if is_help_or_version "$@"; then
    return 0
  fi

  if ! command -v lsof >/dev/null 2>&1; then
    printf 'swimmers-tui: lsof not found; cannot clear stale local swimmers server\n' >&2
    return 0
  fi

  local target_url port
  target_url="${SWIMMERS_TUI_URL:-http://127.0.0.1:${PORT:-3210}}"
  port="$(loopback_port_from_url "${target_url}")" || return 0

  local pids=()
  local found_pid
  while IFS= read -r found_pid; do
    [[ -n "${found_pid}" ]] && pids+=("${found_pid}")
  done < <(lsof -nP -tiTCP:"${port}" -sTCP:LISTEN 2>/dev/null || true)
  if [[ "${#pids[@]}" -eq 0 ]]; then
    return 0
  fi

  local pid command_line killed=0
  for pid in "${pids[@]}"; do
    command_line="$(pid_command "${pid}")"
    if ! is_swimmers_server_command "${command_line}"; then
      printf 'swimmers-tui: leaving non-swimmers listener on port %s alone: pid %s (%s)\n' \
        "${port}" "${pid}" "${command_line:-unknown}" >&2
      continue
    fi

    printf 'swimmers-tui: restarting stale local swimmers API on port %s (pid %s)\n' \
      "${port}" "${pid}" >&2
    kill "${pid}" 2>/dev/null || true
    if ! wait_for_pid_to_exit "${pid}"; then
      kill -KILL "${pid}" 2>/dev/null || true
      wait_for_pid_to_exit "${pid}" || true
    fi
    killed=1
  done

  if [[ "${killed}" -eq 1 ]]; then
    sleep 0.1
  fi
}

# swimmers-tui now owns server lifecycle:
# - default: embedded mode (in-process API)
# - SWIMMERS_TUI_URL=http://...: external HTTP mode (+ loopback auto-spawn)
# Removed startup-tuning vars: TUI_WAIT_PATH TUI_WAIT_TIMEOUT TUI_START_TIMEOUT
# TUI_PRESTART_WAIT_TIMEOUT TUI_WAIT_INTERVAL TUI_WAIT_LOG_INTERVAL
# TUI_WAIT_ONLY TUI_SKIP_TUI TUI_NATIVE_SWITCH_PATH TUI_DIR_PICKER_PATH
feature_args=()
if [[ -n "${SWIMMERS_TUI_FEATURES:-}" ]]; then
  feature_args=(--features "${SWIMMERS_TUI_FEATURES}")
fi
restart_loopback_swimmers_if_needed "$@"
exec cargo run --quiet "${feature_args[@]}" --bin swimmers-tui -- "$@"
