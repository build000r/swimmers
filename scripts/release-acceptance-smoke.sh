#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_ID="${SWIMMERS_ACCEPTANCE_RUN_ID:-$(date -u '+%Y%m%dT%H%M%SZ-release-acceptance')}"
ARTIFACT_DIR="${SWIMMERS_ACCEPTANCE_ARTIFACT_DIR:-${ROOT_DIR}/tests/artifacts/release-acceptance/${RUN_ID}}"

mkdir -p "${ARTIFACT_DIR}"
cd "${ROOT_DIR}"

usage() {
  cat <<'EOF'
Usage: scripts/release-acceptance-smoke.sh <profile>

Profiles:
  default-installed  Install or use default binaries, then check --help,
                     loopback server, /health, /v1/sessions, and TUI CLI boot.
  source-personal    Run the source-checkout personal workflow launcher smoke.
  native-assets      Check native handoff scripts are packaged.
  thought            Run thought bridge/fake-emitter contract tests.
  voice              Compile the optional voice-enabled TUI path.
  all                Run default-installed, native-assets, source-personal,
                     and thought. Voice stays opt-in.

Environment:
  SWIMMERS_ACCEPTANCE_ARTIFACT_DIR   Directory for logs and payload evidence.
  SWIMMERS_ACCEPTANCE_BIN_DIR        Directory containing swimmers binaries.
  SWIMMERS_ACCEPTANCE_INSTALL_ROOT   Cargo install root for default-installed.
  SWIMMERS_ACCEPTANCE_PORT           Loopback port for default-installed.
EOF
}

profile="${1:-default-installed}"
case "${profile}" in
  -h|--help|help)
    usage
    exit 0
    ;;
esac

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command for %s profile: %s\n' "${profile}" "$1" >&2
    exit 1
  fi
}

run_logged() {
  local name="$1"
  shift
  local log_path="${ARTIFACT_DIR}/${name}.log"
  printf '[release-acceptance] %s\n' "$*"
  "$@" > >(tee "${log_path}") 2>&1
}

port_open() {
  local port="$1"
  (: <"/dev/tcp/127.0.0.1/${port}") >/dev/null 2>&1
}

pick_port() {
  if [[ -n "${SWIMMERS_ACCEPTANCE_PORT:-}" ]]; then
    printf '%s\n' "${SWIMMERS_ACCEPTANCE_PORT}"
    return 0
  fi

  local base="${PORT:-34210}"
  local offset candidate
  for offset in {0..40}; do
    candidate="$((base + offset))"
    if ! port_open "${candidate}"; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  printf 'could not find a free loopback port near %s\n' "${base}" >&2
  return 1
}

wait_for_url() {
  local url="$1"
  local out_path="$2"
  local log_path="$3"
  local _i
  for _i in {1..80}; do
    if curl -fsS --connect-timeout 1 --max-time 2 "${url}" -o "${out_path}"; then
      return 0
    fi
    sleep 0.25
  done

  printf 'timed out waiting for %s\n' "${url}" >&2
  if [[ -f "${log_path}" ]]; then
    tail -n 120 "${log_path}" >&2 || true
  fi
  return 1
}

server_pid=""
cleanup_server() {
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    kill "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  server_pid=""
}

profile_default_installed() {
  require_cmd curl

  local install_root bin_dir server_bin tui_bin
  install_root="${SWIMMERS_ACCEPTANCE_INSTALL_ROOT:-${ARTIFACT_DIR}/install-root}"

  if [[ -n "${SWIMMERS_ACCEPTANCE_BIN_DIR:-}" ]]; then
    bin_dir="${SWIMMERS_ACCEPTANCE_BIN_DIR}"
  else
    require_cmd cargo
    run_logged cargo-install cargo install --path "${ROOT_DIR}" --root "${install_root}" --locked --force
    bin_dir="${install_root}/bin"
  fi

  server_bin="${SWIMMERS_ACCEPTANCE_SWIMMERS_BIN:-${bin_dir}/swimmers}"
  tui_bin="${SWIMMERS_ACCEPTANCE_SWIMMERS_TUI_BIN:-${bin_dir}/swimmers-tui}"

  [[ -x "${server_bin}" ]] || { printf 'missing executable server binary: %s\n' "${server_bin}" >&2; exit 1; }
  [[ -x "${tui_bin}" ]] || { printf 'missing executable TUI binary: %s\n' "${tui_bin}" >&2; exit 1; }

  run_logged swimmers-help "${server_bin}" --help
  grep -q "Usage: swimmers" "${ARTIFACT_DIR}/swimmers-help.log"
  run_logged swimmers-tui-help "${tui_bin}" --help
  grep -q "Usage: swimmers-tui" "${ARTIFACT_DIR}/swimmers-tui-help.log"
  run_logged swimmers-tui-version "${tui_bin}" --version

  local port base_url data_dir server_log
  port="$(pick_port)"
  base_url="http://127.0.0.1:${port}"
  data_dir="${ARTIFACT_DIR}/data"
  server_log="${ARTIFACT_DIR}/default-installed-server.log"
  mkdir -p "${data_dir}"

  cleanup_server
  trap cleanup_server EXIT
  PORT="${port}" \
    SWIMMERS_BIND="127.0.0.1" \
    AUTH_MODE="local_trust" \
    SWIMMERS_DATA_DIR="${data_dir}" \
    "${server_bin}" >"${server_log}" 2>&1 &
  server_pid="$!"

  wait_for_url "${base_url}/health" "${ARTIFACT_DIR}/health.json" "${server_log}"
  wait_for_url "${base_url}/v1/sessions" "${ARTIFACT_DIR}/sessions.json" "${server_log}"

  cleanup_server
  trap - EXIT

  printf 'default-installed acceptance passed; evidence: %s\n' "${ARTIFACT_DIR}"
}

profile_source_personal() {
  run_logged source-personal-smoke bash "${ROOT_DIR}/scripts/test-run-up.sh"
}

profile_native_assets() {
  run_logged native-assets-smoke sh "${ROOT_DIR}/scripts/smoke-native-assets-package.sh"
}

profile_thought() {
  require_cmd cargo
  run_logged thought-emitter-client cargo test --lib thought::emitter_client::tests:: -- --nocapture --test-threads=1
  run_logged thought-bridge-runner cargo test --lib thought::bridge_runner::tests:: -- --nocapture --test-threads=1
}

profile_voice() {
  require_cmd cargo
  require_cmd cmake
  run_logged voice-check cargo check --bin swimmers-tui --features voice
}

run_profile() {
  case "$1" in
    default-installed) profile_default_installed ;;
    source-personal) profile_source_personal ;;
    native-assets) profile_native_assets ;;
    thought) profile_thought ;;
    voice) profile_voice ;;
    all)
      profile_default_installed
      profile_native_assets
      profile_source_personal
      profile_thought
      ;;
    *)
      usage >&2
      printf '\nunknown release acceptance profile: %s\n' "$1" >&2
      exit 2
      ;;
  esac
}

printf '[release-acceptance] profile=%s artifacts=%s\n' "${profile}" "${ARTIFACT_DIR}"
run_profile "${profile}"
printf '[release-acceptance] %s passed\n' "${profile}"
