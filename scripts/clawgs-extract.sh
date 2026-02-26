#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "$script_dir/.." && pwd -P)"

default_bin="$repo_root/../skills/clawgs/target/release/clawgs"
alt_bin="$repo_root/../opensource/skills/clawgs/target/release/clawgs"
bin_path=""

if [[ -n "${CLAWGS_BIN:-}" ]]; then
  bin_path="$CLAWGS_BIN"
else
  for candidate in "$default_bin" "$alt_bin"; do
    if [[ -x "$candidate" ]]; then
      bin_path="$candidate"
      break
    fi
  done
fi

if [[ $# -gt 0 && "$1" != -* ]]; then
  target_cwd="$1"
  shift
else
  target_cwd="$PWD"
fi

if [[ -z "$bin_path" || ! -x "$bin_path" ]]; then
  cat >&2 <<EOF
error: clawgs binary not found.

Build it from the skills repo:
  cd "/path/to/skills/clawgs"
  bash scripts/install.sh

Or set CLAWGS_BIN to an executable clawgs binary.
Checked defaults:
  $default_bin
  $alt_bin
EOF
  exit 1
fi

exec "$bin_path" extract --cwd "$target_cwd" "$@"
