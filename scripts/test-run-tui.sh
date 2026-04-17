#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN_TUI="${ROOT_DIR}/scripts/run-tui.sh"

if [[ ! -x "${RUN_TUI}" ]]; then
  printf 'expected executable shim at %s\n' "${RUN_TUI}" >&2
  exit 1
fi

if grep -q "Local swimmers API is not ready" "${RUN_TUI}"; then
  printf 'legacy startup string still present in %s\n' "${RUN_TUI}" >&2
  exit 1
fi

help_output="$(${RUN_TUI} --help 2>&1)"
if [[ "${help_output}" != *"Usage: swimmers-tui"* ]]; then
  printf 'expected swimmers-tui help output\n' >&2
  printf '%s\n' "${help_output}" >&2
  exit 1
fi

version_output="$(${RUN_TUI} --version 2>&1)"
if [[ "${version_output}" != *"swimmers-tui "* ]]; then
  printf 'expected swimmers-tui version output\n' >&2
  printf '%s\n' "${version_output}" >&2
  exit 1
fi

printf 'run-tui.sh checks passed\n'
