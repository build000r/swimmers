#!/usr/bin/env bash
set -euo pipefail

# Guardrail: every serious status finding in the project-status MMDX must point
# at a tracking Bead, so the status stack cannot drift into an uncovered RED/GAP
# claim (the overclaiming risk this proof pass found).
#
# Convention (deliberately explicit, not heuristic):
#   - Any node classed ":::red" is a serious finding and MUST be covered.
#   - Any node whose label contains the word "GAP" is an asserted gap and MUST
#     be covered.
#   - Coverage is the literal phrase "covered by swimmers-<bead-id>" in the node
#     label. Plain descriptive YELLOW/GREEN status lines need no coverage.
#
# Hard failure: a RED or GAP node with no "covered by swimmers-..." reference.
# Soft warning: a referenced Bead that is already closed (the node should be
# refreshed toward GREEN) — only checked when `br` is on PATH; never fatal.
#
# Cheap to run during status refresh: a single pass over the diagram.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
diagram="${1:-${ROOT_DIR}/diagrams/swimmers-project-status.mmdx}"

if [[ ! -f "${diagram}" ]]; then
  printf 'status-red-bead-coverage: diagram not found: %s\n' "${diagram}" >&2
  exit 2
fi

coverage_re='covered by[[:space:]]+swimmers-[a-z0-9-]+'
bead_ref_re='swimmers-[a-z0-9-]+'

violations=0
checked=0
lineno=0
declare -a referenced_beads=()

while IFS= read -r line || [[ -n "${line}" ]]; do
  lineno=$((lineno + 1))

  needs_coverage=0
  if [[ "${line}" == *':::red'* ]]; then
    needs_coverage=1
  elif printf '%s' "${line}" | grep -qE '\bGAP\b'; then
    needs_coverage=1
  fi
  [[ "${needs_coverage}" -eq 1 ]] || continue

  checked=$((checked + 1))
  if printf '%s' "${line}" | grep -qE "${coverage_re}"; then
    while read -r bead; do
      [[ -n "${bead}" ]] && referenced_beads+=("${bead}")
    done < <(printf '%s' "${line}" | grep -oE "${coverage_re}" | grep -oE "${bead_ref_re}")
  else
    printf 'UNCOVERED status finding (line %d):\n  %s\n' "${lineno}" "${line}" >&2
    violations=$((violations + 1))
  fi
done <"${diagram}"

# Soft warning: flag findings still pointing at a closed Bead (refresh to GREEN).
if command -v br >/dev/null 2>&1 && ((${#referenced_beads[@]})); then
  seen=" "
  for bead in "${referenced_beads[@]}"; do
    case "${seen}" in *" ${bead} "*) continue ;; esac
    seen="${seen}${bead} "
    if CI=1 br show "${bead}" --json 2>/dev/null \
      | grep -qE '"status"[[:space:]]*:[[:space:]]*"closed"'; then
      printf 'note: status finding references closed Bead %s; refresh that node toward GREEN\n' \
        "${bead}" >&2
    fi
  done
fi

if [[ "${violations}" -ne 0 ]]; then
  printf 'status-red-bead-coverage: %d RED/GAP finding(s) lack a "covered by swimmers-..." Bead reference\n' \
    "${violations}" >&2
  exit 1
fi

printf 'status-red-bead-coverage: OK (%d RED/GAP finding(s) checked, all Bead-covered)\n' "${checked}"
