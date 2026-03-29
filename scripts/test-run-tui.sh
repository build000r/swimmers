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
