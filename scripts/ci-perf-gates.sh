#!/usr/bin/env bash
set -euo pipefail

echo "[ci-perf-gates] running Rust lifecycle/perf gate tests"
cargo test thought::loop_runner::tests

echo "[ci-perf-gates] running targeted frontend bubble guardrail tests"
(
  cd web
  npm test -- src/__tests__/idle-preview-ui.test.tsx
)

echo "[ci-perf-gates] all gates passed"
