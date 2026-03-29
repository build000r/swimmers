#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TUI_URL="${SWIMMERS_TUI_URL:-${TUI_URL:-http://127.0.0.1:3210}}"
WAIT_PATH="${TUI_WAIT_PATH:-/v1/sessions}"
WAIT_TIMEOUT="${TUI_WAIT_TIMEOUT:-20}"
START_TIMEOUT="${TUI_START_TIMEOUT:-120}"
PRESTART_WAIT_TIMEOUT="${TUI_PRESTART_WAIT_TIMEOUT:-2}"
WAIT_INTERVAL="${TUI_WAIT_INTERVAL:-1}"
WAIT_LOG_INTERVAL="${TUI_WAIT_LOG_INTERVAL:-5}"
WAIT_ONLY="${TUI_WAIT_ONLY:-0}"
SKIP_TUI="${TUI_SKIP_TUI:-0}"
SERVER_LOG=""
LAST_API_STATUS=""

is_true() {
  local value="${1:-}"
  case "${value,,}" in
    1|true|yes|on) return 0 ;;
    *) return 1 ;;
  esac
}

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

parse_url_host_port() {
  local url="${1:-}"
  local default_port="80"
  local rest="${url}"
  local authority=""
  local host=""
  local port=""

  if [[ "${url}" == https://* ]]; then
    default_port="443"
  fi

  if [[ "${rest}" == *"://"* ]]; then
    rest="${rest#*://}"
  fi
  authority="${rest%%/*}"

  if [[ "${authority}" =~ ^\[([0-9A-Fa-f:]+)\](:(.+))?$ ]]; then
    host="${BASH_REMATCH[1]}"
    port="${BASH_REMATCH[3]:-${default_port}}"
  else
    host="${authority%%:*}"
    if [[ "${authority}" == *:* ]]; then
      port="${authority##*:}"
    else
      port="${default_port}"
    fi
  fi

  printf '%s\t%s\n' "${host}" "${port}"
}

host_is_loopback() {
  local host="${1:-}"
  case "${host,,}" in
    localhost|127.0.0.1|::1)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

target_is_loopback() {
  local parsed host
  parsed="$(parse_url_host_port "${TUI_URL}")"
  host="${parsed%%$'\t'*}"
  host_is_loopback "${host}"
}

should_auto_start_local_api() {
  if is_true "${WAIT_ONLY}"; then
    return 1
  fi

  target_is_loopback
}

api_url() {
  printf '%s%s\n' "${TUI_URL%/}" "${WAIT_PATH}"
}

api_auth_header() {
  if [[ "${AUTH_MODE:-}" == "token" && -n "${AUTH_TOKEN:-}" ]]; then
    printf 'Authorization: Bearer %s\n' "${AUTH_TOKEN}"
  fi
}

api_status() {
  local url="${1:-$(api_url)}"
  local header

  header="$(api_auth_header)"
  if [[ -n "${header}" ]]; then
    curl -sS -o /dev/null -w '%{http_code}' \
      --connect-timeout 1 \
      --max-time 2 \
      -H "${header}" \
      "${url}" \
      2>/dev/null || true
  else
    curl -sS -o /dev/null -w '%{http_code}' \
      --connect-timeout 1 \
      --max-time 2 \
      "${url}" \
      2>/dev/null || true
  fi
}

probe_api_access() {
  local url="${1:-$(api_url)}"
  LAST_API_STATUS="$(api_status "${url}")"
  case "${LAST_API_STATUS}" in
    200)
      return 0
      ;;
    401|403)
      return 10
      ;;
    *)
      return 1
      ;;
  esac
}

show_api_auth_failure() {
  local url="${1:-$(api_url)}"

  case "${LAST_API_STATUS}" in
    401)
      printf 'swimmers API at %s requires valid auth for %s; set AUTH_MODE=token and AUTH_TOKEN to match the target API\n' \
        "${TUI_URL}" \
        "${WAIT_PATH}" >&2
      ;;
    403)
      printf 'swimmers API at %s denied session access for %s; use a token with session-list access for this TUI instance\n' \
        "${TUI_URL}" \
        "${WAIT_PATH}" >&2
      ;;
    *)
      printf 'swimmers API at %s failed auth probe for %s (status: %s)\n' \
        "${TUI_URL}" \
        "${WAIT_PATH}" \
        "${LAST_API_STATUS:-000}" >&2
      ;;
  esac
}

show_server_log_tail() {
  if [[ -z "${SERVER_LOG}" || ! -f "${SERVER_LOG}" ]]; then
    return 0
  fi

  printf 'Recent server log: %s\n' "${SERVER_LOG}" >&2
  tail -n 20 "${SERVER_LOG}" >&2 || true
}

start_local_api() {
  local parsed host port log_dir
  parsed="$(parse_url_host_port "${TUI_URL}")"
  host="${parsed%%$'\t'*}"
  port="${parsed#*$'\t'}"

  require cargo

  log_dir="${TUI_SERVER_LOG_DIR:-${TMPDIR:-/tmp}}"
  mkdir -p "${log_dir}"
  SERVER_LOG="${TUI_SERVER_LOG:-${log_dir%/}/swimmers-tui-server-${port}.log}"
  : > "${SERVER_LOG}"

  printf 'Local swimmers API is not ready; starting it on %s:%s\n' "${host}" "${port}"
  printf 'Server log: %s\n' "${SERVER_LOG}"

  (
    cd "${ROOT_DIR}"
    nohup env PORT="${port}" cargo run --bin swimmers >>"${SERVER_LOG}" 2>&1 &
  )
}

wait_for_api() {
  local timeout="${1:-${WAIT_TIMEOUT}}"
  local url
  local deadline
  local next_log_at

  url="$(api_url)"
  deadline=$((SECONDS + timeout))
  next_log_at=$((SECONDS + WAIT_LOG_INTERVAL))

  printf 'Waiting for swimmers API at %s\n' "${url}"

  while (( SECONDS <= deadline )); do
    if probe_api_access "${url}"; then
      printf 'swimmers API is ready (%s)\n' "${LAST_API_STATUS}"
      return 0
    fi
    local probe_status=$?
    if (( probe_status == 10 )); then
      show_api_auth_failure "${url}"
      return 1
    fi
    if (( WAIT_LOG_INTERVAL > 0 && SECONDS >= next_log_at )); then
      printf 'Still waiting for swimmers API at %s (elapsed: %ss, last status: %s)\n' \
        "${url}" \
        "${SECONDS}" \
        "${LAST_API_STATUS:-000}"
      next_log_at=$((SECONDS + WAIT_LOG_INTERVAL))
    fi
    sleep "${WAIT_INTERVAL}"
  done

  printf 'timed out waiting for swimmers API at %s (last status: %s)\n' "${url}" "${LAST_API_STATUS:-000}" >&2
  show_server_log_tail
  return 1
}

wait_for_api_quiet() {
  local timeout="${1:-${WAIT_TIMEOUT}}"
  local url
  local deadline

  url="$(api_url)"
  deadline=$((SECONDS + timeout))

  while (( SECONDS <= deadline )); do
    if probe_api_access "${url}"; then
      return 0
    fi
    local probe_status=$?
    if (( probe_status == 10 )); then
      return 10
    fi
    sleep "${WAIT_INTERVAL}"
  done

  return 1
}

main() {
  require curl
  local probe_status=0

  if probe_api_access "$(api_url)"; then
    printf 'swimmers API is ready (%s)\n' "${LAST_API_STATUS}"
  else
    probe_status=$?
    if (( probe_status == 10 )); then
      show_api_auth_failure "$(api_url)"
      return 1
    elif should_auto_start_local_api; then
      if wait_for_api_quiet "${PRESTART_WAIT_TIMEOUT}"; then
        printf 'swimmers API is ready (%s)\n' "${LAST_API_STATUS}"
      else
        probe_status=$?
        if (( probe_status == 10 )); then
          show_api_auth_failure "$(api_url)"
          return 1
        fi
        start_local_api
        wait_for_api "${START_TIMEOUT}"
      fi
    else
      wait_for_api "${WAIT_TIMEOUT}"
    fi
  fi

  if is_true "${WAIT_ONLY}" || is_true "${SKIP_TUI}"; then
    return 0
  fi

  cd "${ROOT_DIR}"
  SWIMMERS_TUI_URL="${TUI_URL}" cargo run --bin swimmers-tui
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
