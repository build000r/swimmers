#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${SWIMMERS_GLANCE_ARTIFACT_DIR:-${ROOT_DIR}/tests/artifacts/glance}"
MANIFEST="${ROOT_DIR}/tests/fixtures/glance_state_coverage_10.json"

require() {
  local name="${1:-}"
  if ! command -v "${name}" >/dev/null 2>&1; then
    printf 'swimmers Glance smoke requires %s\n' "${name}" >&2
    exit 1
  fi
}

require cargo
require python3

python3 -m json.tool "${MANIFEST}" >/dev/null

rm -rf "${ARTIFACT_DIR}"
mkdir -p "${ARTIFACT_DIR}"

cd "${ROOT_DIR}"
SWIMMERS_GLANCE_ARTIFACT_DIR="${ARTIFACT_DIR}" \
  cargo test --bin swimmers-tui fixture_manifest_ -- --nocapture --test-threads=1

for artifact in sessions.json state-observations.json tui-frame.txt native-open.json; do
  if [[ ! -s "${ARTIFACT_DIR}/${artifact}" ]]; then
    printf 'swimmers Glance smoke failed: missing artifact %s\n' "${ARTIFACT_DIR}/${artifact}" >&2
    exit 1
  fi
done

python3 - "${ARTIFACT_DIR}/state-observations.json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

observations = payload.get("observations", [])
if payload.get("session_count") != 10 or len(observations) != 10:
    raise SystemExit(f"expected 10 observations, got {len(observations)}")
if payload.get("first_frame_elapsed_ms", 999999) > 2000:
    raise SystemExit(f"first frame exceeded 2s: {payload.get('first_frame_elapsed_ms')}ms")

roles = {item.get("role") for item in observations}
required = {
    "ai_agent_compiling",
    "running_tests",
    "idle",
    "awaiting_user",
    "errored",
    "exited",
    "stale_degraded",
}
missing = sorted(required - roles)
if missing:
    raise SystemExit(f"missing required roles: {', '.join(missing)}")

for item in observations:
    if item.get("expected_sprite") != item.get("actual_sprite"):
        raise SystemExit(f"sprite mismatch: {item}")
    if item.get("expected_label") != item.get("actual_label"):
        raise SystemExit(f"label mismatch: {item}")
    if item.get("tmux_name_used_for_state"):
        raise SystemExit(f"state depended on tmux name: {item}")
PY

python3 - "${ARTIFACT_DIR}/native-open.json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    payload = json.load(handle)

if payload.get("requires_real_iterm_or_ghostty"):
    raise SystemExit("native handoff proof should be CI-safe simulation by default")
if not payload.get("duplicate_pending_open_suppressed"):
    raise SystemExit("duplicate pending native open was not suppressed")

targets = payload.get("targets", [])
roles = [target.get("role") for target in targets]
if roles != ["errored", "awaiting_user"]:
    raise SystemExit(f"unexpected native handoff target roles: {roles}")

open_calls = payload.get("open_calls", [])
target_ids = [target.get("session_id") for target in targets]
if open_calls != target_ids:
    raise SystemExit(f"open calls {open_calls} did not match targets {target_ids}")
PY

printf 'glance smoke passed; artifacts: %s\n' "${ARTIFACT_DIR}"
