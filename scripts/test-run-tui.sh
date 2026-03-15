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

printf 'run-tui.sh checks passed\n'
