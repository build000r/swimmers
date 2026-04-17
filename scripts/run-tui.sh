#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

# swimmers-tui now owns server lifecycle:
# - default: embedded mode (in-process API)
# - SWIMMERS_TUI_URL=http://...: external HTTP mode (+ loopback auto-spawn)
# Removed startup-tuning vars: TUI_WAIT_PATH TUI_WAIT_TIMEOUT TUI_START_TIMEOUT
# TUI_PRESTART_WAIT_TIMEOUT TUI_WAIT_INTERVAL TUI_WAIT_LOG_INTERVAL
# TUI_WAIT_ONLY TUI_SKIP_TUI TUI_NATIVE_SWITCH_PATH TUI_DIR_PICKER_PATH
exec cargo run --quiet --bin swimmers-tui -- "$@"
