#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

mode="${1:---dry-run}"
case "${mode}" in
  --dry-run | --live) ;;
  *)
    printf 'usage: %s [--dry-run|--live]\n' "$0" >&2
    exit 2
    ;;
esac

target_id="${SWIMMERS_LIVE_TARGET_ID:-example-ssh}"
target_kind="${SWIMMERS_LIVE_TARGET_KIND:-ssh_only}"
cockpit_url="${SWIMMERS_LIVE_COCKPIT_URL:-}"
remote_url="${SWIMMERS_LIVE_REMOTE_URL:-}"
local_cwd="${SWIMMERS_LIVE_LOCAL_CWD:-${PWD}}"
remote_cwd="${SWIMMERS_LIVE_REMOTE_CWD:-}"
attach_hint="${SWIMMERS_LIVE_ATTACH_HINT:-ssh ${target_id}}"
bootstrap_hint="${SWIMMERS_LIVE_BOOTSTRAP_HINT:-ssh ${target_id} 'swimmers serve'}"
artifact_dir="${SWIMMERS_LIVE_ARTIFACT_DIR:-tests/artifacts/multi-ssh-live/$(date -u +%Y%m%dT%H%M%SZ)}"
token_env="${SWIMMERS_LIVE_AUTH_TOKEN_ENV:-}"

sanitize_url() {
  python3 - "$1" <<'PY'
import sys
from urllib.parse import urlsplit, urlunsplit

raw = sys.argv[1].strip()
if not raw:
    print("")
    raise SystemExit
parts = urlsplit(raw)
host = parts.hostname or ""
if parts.port is not None:
    host = f"{host}:{parts.port}"
print(urlunsplit((parts.scheme, host, parts.path.rstrip("/"), "", "")))
PY
}

write_plan() {
  python3 - "$artifact_dir" <<'PY'
import json
import os
import sys
from pathlib import Path

artifact_dir = Path(sys.argv[1])
artifact_dir.mkdir(parents=True, exist_ok=True)
target_kind = os.environ["SWIMMERS_PLAN_TARGET_KIND"]
remote_cwd = os.environ.get("SWIMMERS_PLAN_REMOTE_CWD") or None
target_id = os.environ["SWIMMERS_PLAN_TARGET_ID"]

if target_kind == "ssh_only":
    launch_preview = {
        "allowed": False,
        "reason": "ssh_only_handoff",
        "receipt_fields": ["target_id", "target_kind", "attach_hint", "bootstrap_hint"],
    }
elif remote_cwd:
    launch_preview = {
        "allowed": True,
        "reason": None,
        "receipt_fields": ["target_id", "target_kind", "requested_cwd", "resolved_cwd", "path_mapping"],
    }
else:
    launch_preview = {
        "allowed": False,
        "reason": "missing_path_mapping",
        "receipt_fields": ["target_id", "target_kind", "requested_cwd"],
    }

plan = {
    "target_id": target_id,
    "target_kind": target_kind,
    "cockpit_url": os.environ.get("SWIMMERS_PLAN_COCKPIT_URL") or None,
    "remote_url": os.environ.get("SWIMMERS_PLAN_REMOTE_URL") or None,
    "local_cwd": os.environ["SWIMMERS_PLAN_LOCAL_CWD"],
    "remote_cwd": remote_cwd,
    "attach_hint": os.environ["SWIMMERS_PLAN_ATTACH_HINT"],
    "bootstrap_hint": os.environ["SWIMMERS_PLAN_BOOTSTRAP_HINT"],
    "non_destructive": True,
    "live_actions": [
        "GET cockpit /health",
        "GET cockpit /v1/sessions",
        "optional GET remote /health when SWIMMERS_LIVE_REMOTE_URL is set",
        "write JSON artifacts under tests/artifacts or SWIMMERS_LIVE_ARTIFACT_DIR",
    ],
    "will_not": [
        "ssh to a host",
        "POST /v1/sessions",
        "send input",
        "kill tmux sessions",
        "restart remote services",
        "print token values",
    ],
    "expected_launch_preview": launch_preview,
}
(artifact_dir / "plan.json").write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
PY
}

export SWIMMERS_PLAN_TARGET_ID="${target_id}"
export SWIMMERS_PLAN_TARGET_KIND="${target_kind}"
export SWIMMERS_PLAN_COCKPIT_URL="$(sanitize_url "${cockpit_url}")"
export SWIMMERS_PLAN_REMOTE_URL="$(sanitize_url "${remote_url}")"
export SWIMMERS_PLAN_LOCAL_CWD="${local_cwd}"
export SWIMMERS_PLAN_REMOTE_CWD="${remote_cwd}"
export SWIMMERS_PLAN_ATTACH_HINT="${attach_hint}"
export SWIMMERS_PLAN_BOOTSTRAP_HINT="${bootstrap_hint}"

if [[ "${mode}" == "--dry-run" ]]; then
  mkdir -p "${artifact_dir}"
  write_plan
  printf 'multi-ssh live acceptance dry run\n'
  printf 'target: %s (%s)\n' "${target_id}" "${target_kind}"
  printf 'cockpit url: %s\n' "${SWIMMERS_PLAN_COCKPIT_URL:-<set SWIMMERS_LIVE_COCKPIT_URL for live>}"
  printf 'local cwd: %s\n' "${local_cwd}"
  if [[ -n "${remote_cwd}" ]]; then
    printf 'remote cwd: %s\n' "${remote_cwd}"
  else
    printf 'remote cwd: <unset; live swimmers_api proof will report missing_path_mapping preview>\n'
  fi
  printf 'attach hint: %s\n' "${attach_hint}"
  printf 'bootstrap hint: %s\n' "${bootstrap_hint}"
  printf 'artifact plan: %s/plan.json\n' "${artifact_dir}"
  printf 'live mode requires SWIMMERS_LIVE_TARGET_APPROVED=1 and SWIMMERS_LIVE_COCKPIT_URL\n'
  printf 'legacy alias accepted: SWIMMERS_LIVE_DEVBOX_TARGET=1\n'
  exit 0
fi

live_target_approved="${SWIMMERS_LIVE_TARGET_APPROVED:-${SWIMMERS_LIVE_DEVBOX_TARGET:-0}}"
if [[ "${live_target_approved}" != "1" ]]; then
  printf 'refusing live proof: set SWIMMERS_LIVE_TARGET_APPROVED=1 after reviewing --dry-run output\n' >&2
  printf 'legacy alias accepted: SWIMMERS_LIVE_DEVBOX_TARGET=1\n' >&2
  exit 2
fi

if [[ -z "${cockpit_url}" ]]; then
  printf 'refusing live proof: SWIMMERS_LIVE_COCKPIT_URL is required\n' >&2
  exit 2
fi

command -v curl >/dev/null
command -v python3 >/dev/null

mkdir -p "${artifact_dir}"
write_plan

# Token-safe auth transport: never place the bearer token in curl process argv,
# where it would be visible via `ps` or /proc/<pid>/cmdline. When a token env is
# named, hand the Authorization header to curl through a `--config` document fed
# on stdin, so the secret stays out of argv and is never written to disk.
auth_config_on_stdin=0
auth_config_document=""
if [[ -n "${token_env}" ]]; then
  token_value="${!token_env-}"
  if [[ -z "${token_value}" ]]; then
    printf 'refusing live proof: %s is named by SWIMMERS_LIVE_AUTH_TOKEN_ENV but is unset\n' "${token_env}" >&2
    exit 2
  fi
  # Escape backslash and double-quote for a curl config double-quoted value.
  esc_token="${token_value//\\/\\\\}"
  esc_token="${esc_token//\"/\\\"}"
  printf -v auth_config_document 'header = "Authorization: Bearer %s"\n' "${esc_token}"
  auth_config_on_stdin=1
  unset token_value esc_token
fi

fetch_json() {
  local base_url="$1"
  local path="$2"
  local output="$3"
  local url="${base_url%/}${path}"
  if [[ "${auth_config_on_stdin}" == "1" ]]; then
    # printf is a bash builtin, so the token never enters a child process argv;
    # `--config -` reads the header from stdin, keeping it out of curl's argv too.
    printf '%s' "${auth_config_document}" \
      | curl -fsS --max-time "${SWIMMERS_LIVE_CURL_TIMEOUT:-10}" --config - "${url}" -o "${output}"
  else
    curl -fsS --max-time "${SWIMMERS_LIVE_CURL_TIMEOUT:-10}" "${url}" -o "${output}"
  fi
}

printf 'multi-ssh live acceptance\n'
printf 'target: %s (%s)\n' "${target_id}" "${target_kind}"
printf 'cockpit url: %s\n' "$(sanitize_url "${cockpit_url}")"
if [[ -n "${remote_url}" ]]; then
  printf 'remote url: %s\n' "$(sanitize_url "${remote_url}")"
fi
printf 'artifact dir: %s\n' "${artifact_dir}"
printf 'actions: GET health/session inventory only; no SSH, launch, input, kill, or restart\n'

if ! fetch_json "${cockpit_url}" "/health" "${artifact_dir}/cockpit-health.json"; then
  printf 'live proof failed: cockpit health probe failed\n' >&2
  exit 10
fi

if ! fetch_json "${cockpit_url}" "/v1/sessions" "${artifact_dir}/cockpit-sessions.json"; then
  printf 'live proof failed: cockpit session inventory probe failed\n' >&2
  exit 11
fi

if [[ -n "${remote_url}" ]]; then
  if ! fetch_json "${remote_url}" "/health" "${artifact_dir}/remote-health.json"; then
    printf 'live proof failed: remote API health probe failed\n' >&2
    exit 12
  fi
fi

python3 - "${artifact_dir}" "${target_id}" "${target_kind}" "${remote_cwd}" <<'PY'
import json
import sys
from pathlib import Path

artifact_dir = Path(sys.argv[1])
target_id = sys.argv[2]
target_kind = sys.argv[3]
remote_cwd = sys.argv[4] or None

payload = json.loads((artifact_dir / "cockpit-sessions.json").read_text(encoding="utf-8"))
environments = payload.get("environments") or []
sessions = payload.get("sessions") or []
target = next((row for row in environments if row.get("id") == target_id), None)
if target is None:
    raise SystemExit(f"environment matrix row not found for target_id={target_id}")

kind = target.get("kind")
if kind != target_kind:
    raise SystemExit(f"target kind mismatch for {target_id}: expected {target_kind}, got {kind}")

capabilities = target.get("capabilities") or {}
checks = {
    "environment_matrix_row": True,
    "target_kind": kind,
    "status": target.get("status"),
    "path_mapping_count": target.get("path_mapping_count", 0),
    "capabilities": capabilities,
    "attach_hint_present": bool(target.get("attach_hint")),
    "bootstrap_hint_present": bool(target.get("bootstrap_hint")),
    "session_count": len(sessions),
}

if target_kind == "ssh_only":
    if capabilities.get("observe_sessions") or capabilities.get("launch_session") or capabilities.get("send_input"):
        raise SystemExit("ssh_only target unexpectedly exposes live observe/launch/send capabilities")
    if not (target.get("attach_hint") or target.get("bootstrap_hint")):
        raise SystemExit("ssh_only target lacks attach/bootstrap hint")
else:
    if target.get("path_mapping_count", 0) < 1 and remote_cwd:
        raise SystemExit("swimmers_api target has no path mappings despite remote cwd expectation")

stale_advisory = False
for row in list(environments) + list(sessions):
    for advisory in row.get("advisory") or []:
        if advisory.get("stale") is True and advisory.get("status") == "external":
            stale_advisory = True
checks["stale_external_advisory_seen"] = stale_advisory

(artifact_dir / "live-result.json").write_text(json.dumps(checks, indent=2) + "\n", encoding="utf-8")
PY

printf 'multi-ssh live acceptance passed; artifacts: %s\n' "${artifact_dir}"
