#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "${ROOT_DIR}/scripts/web-common.sh"
PORT="${PORT:-3210}"
WEB_ROUTE_PATH="${WEB_ROUTE_PATH:-/app.js}"

usage() {
  cat <<'EOF'
Usage: scripts/run-web.sh

Run the Swimmers web surface server for local browser use.

Environment:
  PORT                    Loopback port, default 3210.
  SWIMMERS_WEB_FEATURES   Optional Cargo features passed to the server build.
  SWIMMERS_FRANKENTUI_PKG_DIR
                          FrankenTerm asset package directory.
  FRANKENTUI_PKG_DIR      Alternate FrankenTerm asset package directory.
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
        printf 'Run scripts/run-web.sh --help for usage.\n' >&2
        exit 2
        ;;
      *)
        printf 'unexpected argument: %s\n' "$1" >&2
        printf 'Run scripts/run-web.sh --help for usage.\n' >&2
        exit 2
        ;;
    esac
  done

  if (($#)); then
    printf 'unexpected argument: %s\n' "$1" >&2
    printf 'Run scripts/run-web.sh --help for usage.\n' >&2
    exit 2
  fi
}

announce_urls() {
  printf 'swimmers web surface target URLs\n'
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

web_probe_url() {
  printf 'http://127.0.0.1:%s%s\n' "${PORT}" "${WEB_ROUTE_PATH}"
}

web_route_status() {
  curl -sS -o /dev/null -w '%{http_code}' \
    --noproxy '*' \
    --connect-timeout 1 \
    --max-time 2 \
    "$(web_probe_url)" \
    2>/dev/null || true
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
  ps -p "${pid}" -o comm= 2>/dev/null | xargs || true
}

stop_local_listener() {
  local pid="${1:-}"
  [[ -n "${pid}" ]] || return 1

  printf 'Restarting stale listener on 127.0.0.1:%s (pid %s)\n' "${PORT}" "${pid}"
  kill "${pid}"

  local deadline=$((SECONDS + 10))
  while (( SECONDS <= deadline )); do
    if ! lsof -nP -t -iTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done

  printf 'listener on 127.0.0.1:%s did not stop after signal\n' "${PORT}" >&2
  return 1
}

handle_existing_listener() {
  local pid cmd route_status
  pid="$(listener_pid)"
  [[ -n "${pid}" ]] || return 1

  cmd="$(listener_command "${pid}")"
  route_status="$(web_route_status)"

  case "${route_status}" in
    200)
      printf 'Existing swimmers web surface already available on 127.0.0.1:%s (pid %s)\n' "${PORT}" "${pid}"
      return 10
      ;;
    404)
      if [[ "${cmd}" == "swimmers" ]]; then
        printf 'Existing swimmers listener on 127.0.0.1:%s is missing %s; restarting it to pick up the web build\n' "${PORT}" "${WEB_ROUTE_PATH}"
        stop_local_listener "${pid}"
        return 0
      fi
      ;;
  esac

  printf 'Port %s is already in use by %s (pid %s) and %s returned %s\n' \
    "${PORT}" \
    "${cmd:-unknown process}" \
    "${pid}" \
    "${WEB_ROUTE_PATH}" \
    "${route_status:-000}" >&2
  printf 'Choose a different PORT or stop the existing listener.\n' >&2
  return 2
}

main() {
  parse_args "$@"
  swimmers_require cargo

  local web_features feature_args
  web_features="${SWIMMERS_WEB_FEATURES-}"
  feature_args=()
  if [[ -n "${web_features}" ]]; then
    feature_args=(--features "${web_features}")
    printf 'swimmers web features: %s\n' "${web_features}"
  else
    printf 'swimmers web features: (none)\n'
  fi

  local pkg_dir=""
  if pkg_dir="$(swimmers_resolve_frankentui_pkg_dir)"; then
    export SWIMMERS_FRANKENTUI_PKG_DIR="${pkg_dir}"
    printf 'Using FrankenTerm assets from %s\n' "${SWIMMERS_FRANKENTUI_PKG_DIR}"
  else
    printf 'FrankenTerm assets were not found; the browser UI will use snapshot fallback mode.\n'
    printf 'Set SWIMMERS_FRANKENTUI_PKG_DIR=/path/to/frankentui/pkg for live browser terminal rendering.\n'
  fi

  announce_urls
  export SWIMMERS_PERSONAL_WORKFLOWS="${SWIMMERS_PERSONAL_WORKFLOWS:-1}"

  if handle_existing_listener; then
    :
  else
    case "$?" in
      10) exit 0 ;;
      2) exit 1 ;;
      *) ;;
    esac
  fi

  cd "${ROOT_DIR}"
  exec cargo run "${feature_args[@]}" --bin swimmers
}

main "$@"
