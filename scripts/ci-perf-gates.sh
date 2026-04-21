#!/usr/bin/env bash
set -euo pipefail

echo "[ci-perf-gates] running Rust lifecycle/perf gate tests"
cargo test thought::loop_runner::tests

echo "[ci-perf-gates] running /v1/sessions hot-path perf gate"
cargo test list_sessions_perf_gate --lib

echo "[ci-perf-gates] running /v1/dirs concurrent-safety perf gate"
cargo test list_dirs_parallelizes_git_probes_under_slow_git --lib

echo "[ci-perf-gates] running TUI bootstrap helper checks"
bash ./scripts/test-run-tui.sh

echo "[ci-perf-gates] running embedded first-frame perf gate"
cargo test embedded_mode_first_frame_perf_gate --bin swimmers-tui -- --nocapture

echo "[ci-perf-gates] all gates passed"
