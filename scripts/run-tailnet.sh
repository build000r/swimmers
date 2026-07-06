#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"

PORT="${PORT:-3210}"

usage() {
  cat <<'EOF'
Usage: scripts/run-tailnet.sh

Run the Swimmers server on this machine's Tailscale IPv4 address.

Environment:
  PORT                         Server port, default 3210.
  SWIMMERS_TAILNET_IP          Tailscale IPv4 override.
  SWIMMERS_BIND                Bind override, defaults to the Tailscale IP.
  AUTH_MODE                    Auth mode, default tailnet_trust.
  SWIMMERS_TAILNET_FEATURES    Optional Cargo features for this launcher.
  SWIMMERS_WEB_FEATURES        Feature fallback when SWIMMERS_TAILNET_FEATURES is unset.
  SWIMMERS_TAILNET_TARGET_DIR  Cargo target dir, default ~/.cache/swimmers-tailnet.
  SWIMMERS_TAILNET_DRY_RUN     Set to 1 to print the launch plan without running.
  SWIMMERS_FRANKENTUI_PKG_DIR  FrankenTerm asset package directory.
  FRANKENTUI_PKG_DIR           Alternate FrankenTerm asset package directory.
EOF
}

parse_args() {
  while (($#)); do
    case "$1" in
      -h|--help)
        usage
        exit 0
        ;;
      --)
        shift
        break
        ;;
      -*)
        printf 'unknown option: %s\n' "$1" >&2
        printf 'Run scripts/run-tailnet.sh --help for usage.\n' >&2
        exit 2
        ;;
      *)
        printf 'unexpected argument: %s\n' "$1" >&2
        printf 'Run scripts/run-tailnet.sh --help for usage.\n' >&2
        exit 2
        ;;
    esac
  done

  if (($#)); then
    printf 'unexpected argument: %s\n' "$1" >&2
    printf 'Run scripts/run-tailnet.sh --help for usage.\n' >&2
    exit 2
  fi
}

tailscale_ipv4() {
  swimmers_require tailscale
  tailscale ip -4 2>/dev/null | head -1 || true
}

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

stop_existing_swimmers_listener() {
  local pid="${1:-}"
  [[ -n "${pid}" ]] || return 0

  local command_line
  command_line="$(listener_command "${pid}")"
  if ! is_swimmers_command "${command_line}"; then
    printf 'Port %s is held by a non-swimmers process (pid %s):\n  %s\n' \
      "${PORT}" "${pid}" "${command_line:-unknown}" >&2
    printf 'Refusing to kill. Stop it yourself or choose another PORT.\n' >&2
    return 1
  fi

  printf 'Restarting swimmers backend on port %s for Tailnet access (pid %s)\n' \
    "${PORT}" "${pid}"
  kill "${pid}" 2>/dev/null || true
  if wait_for_listener_to_stop; then
    return 0
  fi

  printf 'SIGTERM did not free port %s after 5s; escalating to SIGKILL on pid %s\n' \
    "${PORT}" "${pid}" >&2
  kill -KILL "${pid}" 2>/dev/null || true
  if wait_for_listener_to_stop; then
    return 0
  fi

  printf 'Failed to free port %s; pid %s may still be running.\n' "${PORT}" "${pid}" >&2
  return 1
}

announce_urls() {
  local bind="${1}"
  printf 'swimmers Tailnet server target URLs\n'
  printf '  bind:     %s:%s\n' "${bind}" "${PORT}"
  printf '  auth:     %s\n' "${AUTH_MODE}"
  printf '  browser:  http://%s:%s/\n' "${bind}" "${PORT}"
  printf '  selected: http://%s:%s/selected\n' "${bind}" "${PORT}"
  printf '  local TUI command:\n'
  printf '    SWIMMERS_TUI_URL=http://%s:%s swimmers-tui\n' "${bind}" "${PORT}"
  printf '\n'
}

main() {
  parse_args "$@"
  swimmers_require cargo
  swimmers_require lsof

  local tailnet_ip
  tailnet_ip="${SWIMMERS_TAILNET_IP:-$(tailscale_ipv4)}"
  if [[ -z "${tailnet_ip}" ]]; then
    printf 'No Tailscale IPv4 address found. Start Tailscale, or set SWIMMERS_TAILNET_IP=100.x.y.z.\n' >&2
    return 1
  fi

  export SWIMMERS_BIND="${SWIMMERS_BIND:-${tailnet_ip}}"
  export AUTH_MODE="${AUTH_MODE:-tailnet_trust}"
  export SWIMMERS_PERSONAL_WORKFLOWS="${SWIMMERS_PERSONAL_WORKFLOWS:-1}"

  local tailnet_features
  tailnet_features="${SWIMMERS_TAILNET_FEATURES-${SWIMMERS_WEB_FEATURES-}}"
  local feature_args=()
  if [[ -n "${tailnet_features}" ]]; then
    feature_args=(--features "${tailnet_features}")
    printf 'swimmers tailnet features: %s\n' "${tailnet_features}"
  else
    printf 'swimmers tailnet features: (none)\n'
  fi

  if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
    export CARGO_TARGET_DIR="${SWIMMERS_TAILNET_TARGET_DIR:-${HOME:-/tmp}/.cache/swimmers-tailnet}"
  fi
  mkdir -p "${CARGO_TARGET_DIR}"
  printf 'Using Cargo target dir %s\n' "${CARGO_TARGET_DIR}"

  local pkg_dir=""
  if pkg_dir="$(swimmers_resolve_frankentui_pkg_dir)"; then
    export SWIMMERS_FRANKENTUI_PKG_DIR="${pkg_dir}"
    printf 'Using FrankenTerm assets from %s\n' "${SWIMMERS_FRANKENTUI_PKG_DIR}"
  else
    printf 'FrankenTerm assets were not found; the browser UI will use snapshot fallback mode.\n'
    printf 'Set SWIMMERS_FRANKENTUI_PKG_DIR=/path/to/frankentui/pkg for live browser terminal rendering.\n'
  fi

  local existing_pid
  existing_pid="$(listener_pid)"
  announce_urls "${SWIMMERS_BIND}"

  if [[ "${SWIMMERS_TAILNET_DRY_RUN:-0}" == "1" ]]; then
    if [[ -n "${existing_pid}" ]]; then
      printf 'dry-run: would restart existing swimmers listener on port %s (pid %s)\n' \
        "${PORT}" "${existing_pid}"
    fi
    printf 'dry-run: cargo run'
    if ((${#feature_args[@]})); then
      printf ' %q' "${feature_args[@]}"
    fi
    printf ' --bin swimmers\n'
    return 0
  fi

  stop_existing_swimmers_listener "${existing_pid}"

  cd "${ROOT_DIR}"
  exec cargo run "${feature_args[@]}" --bin swimmers
}

main "$@"
