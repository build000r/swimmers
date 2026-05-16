#!/usr/bin/env bash
set -euo pipefail

ARTIFACT_ROOT="${SWIMMERS_PERF_ARTIFACT_ROOT:-tests/artifacts/perf}"
RUN_ID="${SWIMMERS_PERF_RUN_ID:-$(python3 - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-perf-gates"))
PY
)}"
RUN_DIR="${ARTIFACT_ROOT}/${RUN_ID}"
METRICS_TSV="${RUN_DIR}/gates.tsv"
SUMMARY_MD="${RUN_DIR}/summary.md"

mkdir -p "${RUN_DIR}"
printf '%s\n' "${RUN_ID}" > "${ARTIFACT_ROOT}/latest-run-id.txt"
printf 'gate\tbudget_ms\telapsed_ms\tstatus\tlog\n' > "${METRICS_TSV}"

now_ms() {
  python3 - <<'PY'
import time
print(int(time.time() * 1000))
PY
}

write_summary() {
  {
    printf '# ci-perf-gates %s\n\n' "${RUN_ID}"
    printf 'Generated: %s\n\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
    printf 'The Rust gates print their internal p95 observations when run with `--nocapture`; the elapsed column records whole-command wall time for environment diagnostics.\n\n'
    printf '| Gate | Budget ms | Elapsed ms | Status | Log |\n'
    printf '|---|---:|---:|---|---|\n'
    tail -n +2 "${METRICS_TSV}" | while IFS=$'\t' read -r gate budget elapsed status log; do
      printf '| `%s` | %s | %s | %s | `%s` |\n' \
        "${gate}" "${budget}" "${elapsed}" "${status}" "${log}"
    done
  } > "${SUMMARY_MD}"
}
trap write_summary EXIT

run_gate() {
  local gate="$1"
  local budget_ms="$2"
  shift 2
  local log_path="${RUN_DIR}/${gate}.log"
  local start_ms end_ms elapsed_ms status
  printf '[ci-perf-gates] running %s (budget %sms)\n' "${gate}" "${budget_ms}"
  start_ms="$(now_ms)"
  if "$@" > >(tee "${log_path}") 2>&1; then
    status="pass"
  else
    status="fail"
  fi
  end_ms="$(now_ms)"
  elapsed_ms="$((end_ms - start_ms))"
  printf '%s\t%s\t%s\t%s\t%s\n' \
    "${gate}" "${budget_ms}" "${elapsed_ms}" "${status}" "${log_path}" >> "${METRICS_TSV}"
  printf '[ci-perf-gates] %s %s in %sms\n' "${gate}" "${status}" "${elapsed_ms}"
  if [[ "${status}" != "pass" ]]; then
    return 1
  fi
}

run_gate thought-bridge-metrics 15000 cargo test --lib thought::bridge_runner::tests:: -- --nocapture --test-threads=1
run_gate list-sessions-p95 500 cargo test list_sessions_perf_gate --lib -- --nocapture
run_gate list-dirs-p95 2000 cargo test list_dirs_parallelizes_git_probes_under_slow_git --lib -- --nocapture
run_gate tui-bootstrap-smoke 15000 bash ./scripts/test-run-tui.sh
run_gate embedded-first-frame-p95 80 cargo test embedded_mode_first_frame_perf_gate --bin swimmers-tui -- --nocapture
run_gate web-cockpit-behavior 5000 node --test src/web/app_behavior.test.mjs

echo "[ci-perf-gates] all gates passed"
echo "[ci-perf-gates] artifacts: ${RUN_DIR}"
