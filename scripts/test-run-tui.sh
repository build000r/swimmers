#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/run-tui.sh"

assert_eq() {
  local expected="${1:-}"
  local actual="${2:-}"
  local label="${3:-assert_eq}"

  if [[ "${expected}" != "${actual}" ]]; then
    printf '%s failed\nexpected: %q\nactual:   %q\n' "${label}" "${expected}" "${actual}" >&2
    exit 1
  fi
}

assert_true() {
  local label="${1:-assert_true}"
  shift || true

  if ! "$@"; then
    printf '%s failed\n' "${label}" >&2
    exit 1
  fi
}

assert_false() {
  local label="${1:-assert_false}"
  shift || true

  if "$@"; then
    printf '%s failed\n' "${label}" >&2
    exit 1
  fi
}

probe_status_for_http_code() {
  local http_code="${1:-000}"

  api_status() {
    printf '%s' "${http_code}"
  }

  set +e
  probe_api_access 'http://example.com/v1/sessions'
  local status=$?
  set -e

  printf '%s' "${status}"
}

assert_eq $'127.0.0.1\t3210' "$(parse_url_host_port 'http://127.0.0.1:3210')" 'parse ipv4'
assert_eq $'localhost\t443' "$(parse_url_host_port 'https://localhost')" 'parse https default port'
assert_eq $'::1\t3210' "$(parse_url_host_port 'http://[::1]:3210')" 'parse ipv6 loopback'

assert_true 'loopback localhost' host_is_loopback 'localhost'
assert_true 'loopback ipv4' host_is_loopback '127.0.0.1'
assert_true 'loopback ipv6' host_is_loopback '::1'
assert_false 'non-loopback host' host_is_loopback 'example.com'

TUI_URL='http://127.0.0.1:3210'
WAIT_ONLY=0
assert_true 'auto start allowed on loopback target' should_auto_start_local_api

TUI_URL='http://example.com:3210'
WAIT_ONLY=0
assert_false 'auto start blocked on remote target' should_auto_start_local_api

TUI_URL='http://127.0.0.1:3210'
WAIT_ONLY=1
assert_false 'wait-only disables auto start' should_auto_start_local_api

AUTH_MODE='token'
AUTH_TOKEN='secret'
assert_eq 'Authorization: Bearer secret' "$(api_auth_header)" 'token auth header'

AUTH_MODE='token'
AUTH_TOKEN=''
assert_eq '' "$(api_auth_header)" 'missing token skips auth header'

AUTH_MODE=''
AUTH_TOKEN='secret'
assert_eq '' "$(api_auth_header)" 'local trust ignores auth token'

assert_eq '0' "$(probe_status_for_http_code 200)" '200 probe is ready'
assert_eq '10' "$(probe_status_for_http_code 401)" '401 probe is auth failure'
assert_eq '10' "$(probe_status_for_http_code 403)" '403 probe is auth failure'
assert_eq '1' "$(probe_status_for_http_code 503)" '503 probe keeps waiting'

stop_local_api_listener() {
  restart_stop_calls=$((restart_stop_calls + 1))
  return 0
}

start_local_api() {
  restart_start_calls=$((restart_start_calls + 1))
}

wait_for_api() {
  restart_wait_calls=$((restart_wait_calls + 1))
}

restart_stop_calls=0
restart_start_calls=0
restart_wait_calls=0
native_switch_route_status() {
  if [[ "${restart_start_calls}" -eq 0 ]]; then
    printf '404'
  else
    printf '422'
  fi
}

TUI_URL='http://127.0.0.1:3210'
WAIT_ONLY=0
ensure_native_switch_capability
assert_eq '1' "${restart_stop_calls}" 'stale native-switch route stops old listener'
assert_eq '1' "${restart_start_calls}" 'stale native-switch route restarts api'
assert_eq '1' "${restart_wait_calls}" 'stale native-switch route waits for api'

restart_stop_calls=0
restart_start_calls=0
restart_wait_calls=0
local_api_listener_exists() {
  return 0
}

handle_local_probe_failure 1
assert_eq '1' "${restart_stop_calls}" 'slow existing listener is stopped before restart'
assert_eq '1' "${restart_start_calls}" 'slow existing listener triggers restart'
assert_eq '1' "${restart_wait_calls}" 'slow existing listener waits for restarted api'

restart_stop_calls=0
restart_start_calls=0
restart_wait_calls=0
local_api_listener_exists() {
  return 1
}

handle_local_probe_failure 1
assert_eq '0' "${restart_stop_calls}" 'missing listener does not stop before start'
assert_eq '1' "${restart_start_calls}" 'missing listener still starts api'
assert_eq '1' "${restart_wait_calls}" 'missing listener waits for started api'

TUI_URL='http://127.0.0.1:33210'
WAIT_PATH='/v1/sessions'
LAST_API_STATUS='401'
auth_401_message="$(show_api_auth_failure 2>&1 || true)"
assert_eq \
  'swimmers API at http://127.0.0.1:33210 requires valid auth for /v1/sessions; set AUTH_MODE=token and AUTH_TOKEN to match the target API' \
  "${auth_401_message}" \
  '401 auth failure message'

LAST_API_STATUS='403'
auth_403_message="$(show_api_auth_failure 2>&1 || true)"
assert_eq \
  'swimmers API at http://127.0.0.1:33210 denied session access for /v1/sessions; use a token with session-list access for this TUI instance' \
  "${auth_403_message}" \
  '403 auth failure message'

printf 'run-tui.sh checks passed\n'
