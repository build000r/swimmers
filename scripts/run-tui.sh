#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TUI_URL="${THRONGTERM_TUI_URL:-${TUI_URL:-http://127.0.0.1:3210}}"
WAIT_PATH="${TUI_WAIT_PATH:-/v1/sessions}"
WAIT_TIMEOUT="${TUI_WAIT_TIMEOUT:-20}"
WAIT_INTERVAL="${TUI_WAIT_INTERVAL:-1}"
WAIT_ONLY="${TUI_WAIT_ONLY:-0}"

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

wait_for_api() {
  local url="${TUI_URL%/}${WAIT_PATH}"
  local deadline=$((SECONDS + WAIT_TIMEOUT))
  local status=""

  printf 'Waiting for throngterm API at %s\n' "${url}"

  while (( SECONDS <= deadline )); do
    status="$(
      curl -sS -o /dev/null -w '%{http_code}' \
        --connect-timeout 1 \
        --max-time 2 \
        "${url}" \
        2>/dev/null || true
    )"
    case "${status}" in
      200|401|403)
        printf 'throngterm API is ready (%s)\n' "${status}"
        return 0
        ;;
    esac
    sleep "${WAIT_INTERVAL}"
  done

  printf 'timed out waiting for throngterm API at %s (last status: %s)\n' "${url}" "${status:-000}" >&2
  return 1
}

main() {
  require curl
  wait_for_api

  if is_true "${WAIT_ONLY}"; then
    return 0
  fi

  cd "${ROOT_DIR}"
  THRONGTERM_TUI_URL="${TUI_URL}" cargo run --bin throngterm-tui
}

main "$@"
