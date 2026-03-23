#!/usr/bin/env bash
set -euo pipefail

echo "[ci-perf-gates] running Rust lifecycle/perf gate tests"
cargo test thought::loop_runner::tests

echo "[ci-perf-gates] running TUI bootstrap helper checks"
bash ./scripts/test-run-tui.sh

echo "[ci-perf-gates] all gates passed"
