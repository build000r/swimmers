#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <version-or-tag>" >&2
  exit 64
fi

version="${1#v}"
changelog_path="${CHANGELOG_PATH:-CHANGELOG.md}"

if [[ ! -f "$changelog_path" ]]; then
  echo "changelog not found: $changelog_path" >&2
  exit 66
fi

if ! awk -v version="$version" '
BEGIN {
  header = "## [" version "]"
  in_section = 0
  found = 0
}
index($0, header) == 1 {
  in_section = 1
  found = 1
}
in_section && $0 ~ /^\[[^]]+\]:[[:space:]]/ {
  exit
}
in_section && index($0, "## [") == 1 && index($0, header) != 1 {
  exit
}
in_section {
  print
}
END {
  if (!found) {
    exit 2
  }
}
' "$changelog_path"; then
  echo "version section not found in $changelog_path: $version" >&2
  exit 65
fi
