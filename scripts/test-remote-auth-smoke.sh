#!/usr/bin/env bash
set -euo pipefail

cargo test --lib \
  api::remote_sessions::tests::remote_api_smoke_matrix_covers_launch_reads_scopes_and_redaction \
  -- --nocapture --test-threads=1
cargo test --lib auth::tests:: -- --nocapture --test-threads=1

printf 'remote auth smoke passed\n'
