#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

fixture="tests/fixtures/multi_ssh_env_v2.json"
fixture_only=0

if [[ "${1:-}" == "--fixture-only" ]]; then
  fixture_only=1
fi

command -v python3 >/dev/null

python3 -m json.tool "${fixture}" >/dev/null
python3 - "${fixture}" <<'PY'
import json
import sys

fixture_path = sys.argv[1]
with open(fixture_path, "r", encoding="utf-8") as handle:
    data = json.load(handle)

assert data["fixture_id"] == "multi_ssh_env_v2"
assert data["one_command_smoke"] == "make multi-ssh-env-smoke"
assert data["default_smoke_is_live_ssh_free"] is True

unsupported = set(data["scope"]["not_supported"])
assert "arbitrary SSH fleet discovery" in unsupported
assert "implicit SSH command execution" in unsupported
assert "FrankenTerm as the Swimmers control plane" in unsupported

targets = {target["id"]: target for target in data["targets"]}
for required in ("local", "devbox", "devbox-down", "skillbox-devbox"):
    assert required in targets, f"missing target {required}"

assert targets["local"]["kind"] == "local"
assert targets["devbox"]["kind"] == "swimmers_api"
assert targets["devbox-down"]["kind"] == "swimmers_api"
assert targets["skillbox-devbox"]["kind"] == "ssh_only"

assert targets["local"]["capabilities"]["observe_sessions"] is True
assert targets["local"]["capabilities"]["native_attach"] is True
assert targets["devbox"]["capabilities"]["send_input"] is True
assert targets["devbox"]["base_url"].startswith("http://")
assert "token" not in targets["devbox"]["base_url"].lower()
assert "secret" not in json.dumps(targets["devbox"]).lower()
assert targets["devbox-down"]["health_status"] == "degraded_cached"
assert targets["devbox-down"]["capabilities"]["send_input"] is False
assert targets["skillbox-devbox"]["capabilities"]["observe_sessions"] is False
assert targets["skillbox-devbox"]["capabilities"]["ssh_attach_hint"] is True
assert targets["skillbox-devbox"]["capabilities"]["bootstrap_hint"] is True

cwd_cases = {case["id"]: case for case in data["cwd_cases"]}
assert cwd_cases["remote_mapped_repo"]["mapping_status"] == "mapped"
assert cwd_cases["remote_mapped_repo"]["remote_cwd"].startswith("/srv/devbox/")
assert cwd_cases["remote_unmapped_scratch"]["mapping_status"] == "unmapped"
assert cwd_cases["remote_unmapped_scratch"]["remote_cwd"] is None
assert cwd_cases["degraded_api_cached_repo"]["health_status"] == "degraded_cached"
assert cwd_cases["ssh_only_handoff_repo"]["mapping_status"] == "handoff_only"

sessions = {case["id"]: case for case in data["session_cases"]}
assert sessions["local_waiting"]["namespaced_session_id"] == sessions["local_waiting"]["session_id"]
assert sessions["remote_waiting_same_repo"]["namespaced_session_id"].startswith("devbox::")
assert sessions["remote_degraded_cached"]["degraded"] is True
advisory = sessions["remote_running_stale_c0"]["advisory_metadata"]
assert {item["source"] for item in advisory} == {"c0", "ntm"}
assert all(item["status"] == "external" and item["stale"] is True for item in advisory)

lenses = {case["id"]: case for case in data["saved_lens_cases"]}
for required in ("all", "current-repo", "ssh-handoff", "degraded", "needs-attention", "swimmers-on-devbox"):
    assert required in lenses, f"missing saved lens {required}"
assert "skillbox-devbox" in lenses["ssh-handoff"]["expected_targets"]
assert "devbox::sess_remote_waiting" in lenses["current-repo"]["expected_sessions"]
assert lenses["swimmers-on-devbox"]["source"] == "overlay"

receipts = {case["id"]: case for case in data["launcher_receipts"]}
assert receipts["remote_mapped_launch"]["allowed"] is True
assert receipts["remote_mapped_launch"]["resolved_cwd"].startswith("/srv/devbox/")
assert receipts["remote_unmapped_blocked"]["allowed"] is False
assert receipts["remote_unmapped_blocked"]["reason"] == "missing_path_mapping"
assert receipts["ssh_only_handoff"]["allowed"] is False
assert receipts["ssh_only_handoff"]["reason"] == "ssh_only_handoff"

required_command_keys = {
    "fixture_manifest",
    "capability_matrix",
    "environment_inventory_redaction",
    "health_and_mapping_doctor",
    "degraded_cached_capabilities",
    "launcher_receipts",
    "unmapped_cwd_guardrail",
    "advisory_freshness",
    "saved_lens_tui",
    "saved_lens_web",
    "surface_action_contract",
}
missing = required_command_keys.difference(data["proof_commands"])
assert not missing, f"missing proof command keys: {sorted(missing)}"

for key, command in data["proof_commands"].items():
    if key not in {"fixture_manifest", "capability_matrix"}:
        assert not command.startswith("ssh "), f"{key} unexpectedly requires live SSH"
PY

if [[ "${fixture_only}" == "1" ]]; then
  printf 'multi-ssh env v2 fixture passed\n'
  exit 0
fi

if [[ "${SWIMMERS_MULTI_SSH_SMOKE_SKIP_RUST:-0}" != "1" ]]; then
  command -v cargo >/dev/null
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${TMPDIR:-/tmp}/swimmers-multi-ssh-env-v2-target}"
  cargo test --lib fleet_lens -- --test-threads=1
  cargo test --lib \
    api::remote_sessions::tests::environment_summary_redacts_token_values_and_credentialed_base_url \
    -- --test-threads=1
  cargo test --lib \
    api::remote_sessions::tests::remote_targets_health_reports_auth_and_mapping_doctor_without_env_names \
    -- --test-threads=1
  cargo test --lib \
    api::remote_sessions::tests::remote_targets_health_reports_cached_degraded_target_without_secret_values \
    -- --test-threads=1
  cargo test --lib \
    api::remote_sessions::tests::unmapped_launch_target_cwd_returns_stable_guidance \
    -- --test-threads=1
  cargo test --bin swimmers-tui \
    launch_target_preview_uses_longest_mapping_and_blocks_unmapped_remote_cwds \
    -- --test-threads=1
  cargo test --bin swimmers-tui \
    thought_panel_marks_advisory_metadata_as_external_and_stale \
    -- --test-threads=1
  cargo test --bin swimmers-tui saved_fleet_lens -- --test-threads=1
  cargo test --bin swimmers-tui \
    header_filter_strip_applies_native_fleet_filters \
    -- --test-threads=1
fi

if [[ "${SWIMMERS_MULTI_SSH_SMOKE_SKIP_JS:-0}" != "1" ]]; then
  command -v node >/dev/null
  node --test \
    src/web/surface_model.test.mjs \
    src/web/app_interaction_behavior.test.mjs \
    src/web/surface_action_plans.test.mjs \
    src/web/contracts.test.mjs
fi

printf 'multi-ssh env v2 smoke passed\n'
