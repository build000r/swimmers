#!/bin/sh
# stress-dirs-concurrency.sh — regression smoke for the "swimmers API
# unavailable (timed out while trying to create a session)" class of bug.
#
# Starts a dedicated swimmers server on an isolated port with DIRS_BASE_PATH
# pointing at a tempdir full of real git repos. Fires concurrent
# `GET /v1/dirs` requests (which probe every entry via `inspect_git_repo`),
# races `POST /v1/sessions` against them, and asserts the POST returns 201
# inside a budget. If `list_dirs` ever regresses to blocking the Tokio
# worker pool, this test fails with a clear timing/status signal.

set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"

STRESS_PORT="${SWIMMERS_STRESS_PORT:-3290}"
STRESS_BASE_URL="http://127.0.0.1:${STRESS_PORT}"
NUM_REPOS="${SWIMMERS_STRESS_REPOS:-12}"
NUM_CONCURRENT_DIRS="${SWIMMERS_STRESS_DIRS_CONCURRENCY:-6}"
POST_BUDGET_S="${SWIMMERS_STRESS_POST_BUDGET_S:-3}"
READY_TIMEOUT_S="${SWIMMERS_STRESS_READY_TIMEOUT_S:-20}"

STRESS_REPOS_ROOT=""
STRESS_SERVER_PID=""
STRESS_SESSION_ID=""
STRESS_LOG=""
POST_BODY_PATH=""

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

cleanup() {
  if [ -n "${STRESS_SESSION_ID}" ]; then
    curl -fsS -X DELETE \
      "${STRESS_BASE_URL}/v1/sessions/${STRESS_SESSION_ID}?mode=kill_tmux" \
      >/dev/null 2>&1 || true
  fi
  if [ -n "${STRESS_SERVER_PID}" ] && kill -0 "${STRESS_SERVER_PID}" 2>/dev/null; then
    kill "${STRESS_SERVER_PID}" 2>/dev/null || true
    # Give it a beat, then SIGKILL anything that refused SIGTERM.
    for _ in 1 2 3 4 5; do
      kill -0 "${STRESS_SERVER_PID}" 2>/dev/null || break
      sleep 0.2
    done
    if kill -0 "${STRESS_SERVER_PID}" 2>/dev/null; then
      kill -KILL "${STRESS_SERVER_PID}" 2>/dev/null || true
    fi
  fi
  if [ -n "${POST_BODY_PATH}" ] && [ -f "${POST_BODY_PATH}" ]; then
    rm -f "${POST_BODY_PATH}"
  fi
  if [ -n "${STRESS_LOG}" ] && [ -f "${STRESS_LOG}" ]; then
    rm -f "${STRESS_LOG}"
  fi
  if [ -n "${STRESS_REPOS_ROOT}" ] && [ -d "${STRESS_REPOS_ROOT}" ]; then
    rm -rf "${STRESS_REPOS_ROOT}"
  fi
}

trap cleanup EXIT INT TERM

require curl
require git
require jq
require awk
require cargo
require mktemp

STRESS_REPOS_ROOT="$(mktemp -d -t swimmers-stress-repos.XXXXXX)"
stress_log_placeholder="$(mktemp -t swimmers-stress-server.XXXXXX)"
STRESS_LOG="${stress_log_placeholder}.log"
rm -f "${stress_log_placeholder}"
stress_post_placeholder="$(mktemp -t swimmers-stress-post.XXXXXX)"
POST_BODY_PATH="${stress_post_placeholder}.json"
rm -f "${stress_post_placeholder}"

printf '[stress] repos root: %s\n' "${STRESS_REPOS_ROOT}"
printf '[stress] server log: %s\n' "${STRESS_LOG}"

# Populate real git repos. Dirty-by-default so both `rev-parse` and
# `status --short` run and the picker produces a non-empty `repo_dirty`.
i=1
while [ "$i" -le "${NUM_REPOS}" ]; do
  repo="${STRESS_REPOS_ROOT}/repo-$(printf '%02d' "$i")"
  mkdir -p "$repo"
  (
    cd "$repo"
    git init -q
    printf 'dirty\n' > README.md
  )
  i=$((i + 1))
done
printf '[stress] built %s git repos\n' "${NUM_REPOS}"

printf '[stress] building swimmers (personal-workflows)\n'
(cd "${REPO_ROOT}" && cargo build --bin swimmers --features personal-workflows >/dev/null)

server_bin="${REPO_ROOT}/target/debug/swimmers"
if [ ! -x "${server_bin}" ]; then
  printf '[stress] missing binary at %s\n' "${server_bin}" >&2
  exit 1
fi

# Launch dedicated stress server on its own port so it does not collide
# with any swimmers instance the developer already has running locally.
DIRS_BASE_PATH="${STRESS_REPOS_ROOT}" \
PORT="${STRESS_PORT}" \
  "${server_bin}" </dev/null >"${STRESS_LOG}" 2>&1 &
STRESS_SERVER_PID=$!
printf '[stress] started server pid=%s on port=%s\n' "${STRESS_SERVER_PID}" "${STRESS_PORT}"

# Wait for the listener.
deadline=$(( $(date +%s) + READY_TIMEOUT_S ))
while :; do
  if curl -fsS --max-time 1 "${STRESS_BASE_URL}/v1/sessions" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "${STRESS_SERVER_PID}" 2>/dev/null; then
    printf '[stress] server exited before becoming ready. Recent log:\n' >&2
    tail -n 40 "${STRESS_LOG}" >&2 || true
    exit 1
  fi
  if [ "$(date +%s)" -ge "${deadline}" ]; then
    printf '[stress] server failed to become ready within %ss. Recent log:\n' \
      "${READY_TIMEOUT_S}" >&2
    tail -n 40 "${STRESS_LOG}" >&2 || true
    exit 1
  fi
  sleep 0.25
done
printf '[stress] server ready\n'

# Fan out concurrent list_dirs calls. Each one probes every repo in
# DIRS_BASE_PATH via inspect_git_repo — that is where the old regression
# pinned the Tokio workers.
dirs_codes_file="$(mktemp -t swimmers-stress-dirs.XXXXXX)"
trap 'rm -f "${dirs_codes_file}"; cleanup' EXIT INT TERM

bg_pids=""
i=1
while [ "$i" -le "${NUM_CONCURRENT_DIRS}" ]; do
  (
    code="$(curl -sS --max-time 15 -o /dev/null \
      -w '%{http_code}' \
      "${STRESS_BASE_URL}/v1/dirs?managed_only=false" || printf '000')"
    printf 'dirs=%s\n' "${code}" >>"${dirs_codes_file}"
  ) &
  bg_pids="${bg_pids} $!"
  i=$((i + 1))
done

# Small jitter so the POST lands while the list_dirs calls are in flight.
sleep 0.05

post_raw="$(
  curl -sS --max-time 15 \
    -o "${POST_BODY_PATH}" \
    -w '%{http_code} %{time_total}' \
    -X POST "${STRESS_BASE_URL}/v1/sessions" \
    -H 'Content-Type: application/json' \
    -d "{\"cwd\":\"${STRESS_REPOS_ROOT}\",\"spawn_tool\":null,\"initial_request\":null}" \
    || printf '000 %s' "${POST_BUDGET_S}"
)"

# Only wait for the curl subshells — a bare `wait` would also wait for the
# backgrounded swimmers server PID and hang forever.
for pid in ${bg_pids}; do
  wait "${pid}" 2>/dev/null || true
done

post_code="${post_raw%% *}"
post_time="${post_raw##* }"

printf '[stress] POST /v1/sessions code=%s time=%ss (budget %ss)\n' \
  "${post_code}" "${post_time}" "${POST_BUDGET_S}"

if [ -s "${dirs_codes_file}" ]; then
  printf '[stress] concurrent list_dirs results:\n'
  sed 's/^/  /' "${dirs_codes_file}"
fi

failed_dirs=0
if grep -v '^dirs=200$' "${dirs_codes_file}" >/dev/null 2>&1; then
  failed_dirs=1
fi
rm -f "${dirs_codes_file}"

if [ "${post_code}" != "201" ]; then
  printf '[stress] FAIL: POST /v1/sessions expected 201, got %s\n' "${post_code}" >&2
  exit 1
fi

if [ "${failed_dirs}" -ne 0 ]; then
  printf '[stress] FAIL: concurrent list_dirs returned non-200\n' >&2
  exit 1
fi

awk -v t="${post_time}" -v b="${POST_BUDGET_S}" 'BEGIN {
  if ((t + 0) > (b + 0)) exit 1
}' || {
  printf '[stress] FAIL: POST /v1/sessions took %ss, over %ss budget\n' \
    "${post_time}" "${POST_BUDGET_S}" >&2
  exit 1
}

STRESS_SESSION_ID="$(jq -r '.session.session_id // empty' <"${POST_BODY_PATH}" 2>/dev/null || true)"

printf '[stress] PASS: concurrent dirs load did not starve session create\n'
