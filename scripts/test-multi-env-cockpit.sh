#!/usr/bin/env bash
set -euo pipefail

fixture="tests/fixtures/multi_env_cockpit.json"

command -v cargo >/dev/null
command -v node >/dev/null
command -v python3 >/dev/null

python3 -m json.tool "$fixture" >/dev/null
python3 - "$fixture" <<'PY'
import json
import sys

fixture_path = sys.argv[1]
with open(fixture_path, "r", encoding="utf-8") as handle:
    data = json.load(handle)

required_command_keys = {
    "environment_inventory",
    "health_doctor",
    "remote_first_launch_preview",
    "remote_write_proxy",
    "remote_group_write_proxy",
    "same_target_guardrail",
    "fleet_lens",
    "native_fleet_filters",
    "grouped_display",
    "attention_inbox",
    "advisory_metadata",
}

assert data["fixture_id"] == "multi_env_cockpit"
targets = {target["id"]: target for target in data["targets"]}
assert targets["local"]["kind"] == "local"
assert targets["devbox"]["kind"] == "swimmers_api"
assert targets["devbox"]["credential_rule"].startswith("tokens must come from")

cases = {case["id"]: case for case in data["cwd_cases"]}
assert cases["remote_mapped_repo"]["mapping_status"] == "mapped"
assert cases["remote_mapped_repo"]["remote_cwd"].startswith("/srv/devbox/")
assert cases["remote_unmapped_scratch"]["mapping_status"] == "unmapped"
assert cases["remote_unmapped_scratch"]["remote_cwd"] is None
assert cases["remote_degraded_cached"]["health_status"] == "degraded_cached"

sessions = {case["id"]: case for case in data["session_cases"]}
assert sessions["remote_waiting"]["namespaced_session_id"].startswith("devbox::")
assert sessions["local_waiting"]["namespaced_session_id"] == sessions["local_waiting"]["session_id"]
assert sessions["remote_running_advisory"]["advisory_metadata"][0]["status"] == "external"
assert sessions["remote_running_advisory"]["advisory_metadata"][0]["stale"] is True

missing = required_command_keys.difference(data["proof_commands"])
assert not missing, f"missing proof command keys: {sorted(missing)}"
assert data["one_command_smoke"] == "make multi-env-smoke"
PY

cargo test --lib \
  api::remote_sessions::tests::environment_summary_redacts_token_values_and_credentialed_base_url \
  -- --test-threads=1
cargo test --lib \
  api::remote_sessions::tests::remote_targets_health_reports_auth_and_mapping_doctor_without_env_names \
  -- --test-threads=1
cargo test --lib \
  api::remote_sessions::tests::unmapped_launch_target_cwd_returns_stable_guidance \
  -- --test-threads=1
cargo test --lib \
  api::remote_sessions::tests::send_remote_input_posts_denamespaced_session_and_namespaces_response \
  -- --test-threads=1
cargo test --lib \
  api::remote_sessions::tests::send_remote_group_input_denamespaces_request_and_namespaces_results \
  -- --test-threads=1
cargo test --lib \
  api::sessions::group_input::tests::remote_group_input_target_rejects_mixed_remote_targets \
  -- --test-threads=1
cargo test --lib \
  api::service::attention_group::tests::attention_queue_excludes_remote_namespaced_sessions \
  -- --test-threads=1
cargo test --lib operator_pressure -- --test-threads=1
cargo test --bin swimmers-tui \
  launch_target_preview_uses_longest_mapping_and_blocks_unmapped_remote_cwds \
  -- --test-threads=1
cargo test --bin swimmers-tui \
  thought_panel_header_summarizes_cross_host_inbox \
  -- --test-threads=1
cargo test --bin swimmers-tui \
  header_filter_strip_applies_native_fleet_filters \
  -- --test-threads=1
cargo test --bin swimmers-tui \
  thought_panel_marks_advisory_metadata_as_external_and_stale \
  -- --test-threads=1
node --test \
  src/web/surface_model.test.mjs \
  src/web/rendered_surface.test.mjs \
  src/web/app_interaction_behavior.test.mjs

printf 'multi-env cockpit smoke passed\n'
