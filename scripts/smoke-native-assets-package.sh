#!/bin/sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)"

cd "$REPO_ROOT"

PACKAGE_LIST="$(cargo package --allow-dirty --list)"

for asset in scripts/iterm-focus.scpt scripts/ghostty-open.scpt; do
  case "$PACKAGE_LIST" in
    *"$asset"*) ;;
    *)
      printf 'packaged native asset missing: %s\n' "$asset" >&2
      exit 1
      ;;
  esac
done

cargo package --allow-dirty

printf 'PASS packaged native handoff assets are available\n'
