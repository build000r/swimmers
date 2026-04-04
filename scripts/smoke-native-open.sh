#!/bin/sh
set -eu

BASE_URL="${SWIMMERS_BASE_URL:-http://127.0.0.1:3210}"
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"
ROOT_CWD="${SWIMMERS_SMOKE_CWD:-${REPO_ROOT}}"
STAMP="$(date +%s)"
NAME_A="iterm-smoke-${STAMP}-a"
NAME_B="iterm-smoke-${STAMP}-b"
SESSION_A=""
SESSION_B=""

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

cleanup() {
  for session_id in "$SESSION_A" "$SESSION_B"; do
    if [ -n "$session_id" ]; then
      curl -fsS -X DELETE \
        "${BASE_URL}/v1/sessions/${session_id}?mode=detach_bridge" \
        >/dev/null 2>&1 || true
    fi
  done
}

create_session() {
  name="$1"
  curl -fsS \
    -H 'Content-Type: application/json' \
    -d "{\"name\":\"${name}\",\"cwd\":\"${ROOT_CWD}\"}" \
    "${BASE_URL}/v1/sessions" |
    jq -r '.session.session_id'
}

open_native() {
  session_id="$1"
  curl -fsS \
    -H 'Content-Type: application/json' \
    -d "{\"session_id\":\"${session_id}\"}" \
    "${BASE_URL}/v1/native/open"
}

require curl
require jq

trap cleanup EXIT INT TERM

support="$(curl -fsS "${BASE_URL}/v1/native/status" | jq -r '.supported')"
if [ "$support" != "true" ]; then
  printf 'native desktop support is unavailable\n' >&2
  exit 1
fi

SESSION_A="$(create_session "$NAME_A")"
SESSION_B="$(create_session "$NAME_B")"

RESP_A="$(open_native "$SESSION_A")"
sleep 1
RESP_B="$(open_native "$SESSION_B")"
sleep 1
RESP_A_FOCUS="$(open_native "$SESSION_A")"
sleep 1
RESP_B_FOCUS="$(open_native "$SESSION_B")"

STATUS_A="$(printf '%s' "$RESP_A" | jq -r '.status')"
STATUS_B="$(printf '%s' "$RESP_B" | jq -r '.status')"
STATUS_A_FOCUS="$(printf '%s' "$RESP_A_FOCUS" | jq -r '.status')"
STATUS_B_FOCUS="$(printf '%s' "$RESP_B_FOCUS" | jq -r '.status')"
PANE_A="$(printf '%s' "$RESP_A" | jq -r '.pane_id')"
PANE_B="$(printf '%s' "$RESP_B" | jq -r '.pane_id')"
PANE_A_FOCUS="$(printf '%s' "$RESP_A_FOCUS" | jq -r '.pane_id')"
PANE_B_FOCUS="$(printf '%s' "$RESP_B_FOCUS" | jq -r '.pane_id')"

if [ "$STATUS_A" != "created" ] || [ "$STATUS_B" != "created" ]; then
  printf 'expected created statuses, got: %s / %s\n' "$STATUS_A" "$STATUS_B" >&2
  exit 1
fi

if [ "$PANE_A" = "$PANE_B" ]; then
  printf 'expected distinct pane ids, got the same id: %s\n' "$PANE_A" >&2
  exit 1
fi

if [ "$STATUS_A_FOCUS" != "focused" ] || [ "$STATUS_B_FOCUS" != "focused" ]; then
  printf 'expected focused statuses, got: %s / %s\n' "$STATUS_A_FOCUS" "$STATUS_B_FOCUS" >&2
  exit 1
fi

if [ "$PANE_A" != "$PANE_A_FOCUS" ] || [ "$PANE_B" != "$PANE_B_FOCUS" ]; then
  printf 'pane ids were not stable across focus: %s->%s / %s->%s\n' \
    "$PANE_A" "$PANE_A_FOCUS" "$PANE_B" "$PANE_B_FOCUS" >&2
  exit 1
fi

printf 'PASS %s %s %s %s\n' \
  "$SESSION_A" "$PANE_A" "$SESSION_B" "$PANE_B"
