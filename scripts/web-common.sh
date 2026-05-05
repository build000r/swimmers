#!/usr/bin/env bash

swimmers_require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$1" >&2
    exit 1
  fi
}

swimmers_valid_frankentui_pkg_dir() {
  local dir="${1:-}"
  [[ -n "${dir}" ]] || return 1
  [[ -f "${dir}/FrankenTerm.js" && -f "${dir}/FrankenTerm_bg.wasm" ]]
}

swimmers_resolve_frankentui_pkg_dir() {
  if swimmers_valid_frankentui_pkg_dir "${SWIMMERS_FRANKENTUI_PKG_DIR:-}"; then
    printf '%s\n' "${SWIMMERS_FRANKENTUI_PKG_DIR}"
    return 0
  fi

  if swimmers_valid_frankentui_pkg_dir "${FRANKENTUI_PKG_DIR:-}"; then
    printf '%s\n' "${FRANKENTUI_PKG_DIR}"
    return 0
  fi

  if [[ "${SWIMMERS_FRANKENTUI_AUTO_DISCOVERY:-1}" == "0" ]]; then
    return 1
  fi

  local root_dir candidate
  root_dir="${ROOT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
  for candidate in \
    "${root_dir}/../frankentui/pkg" \
    "${root_dir}/../../frankentui/pkg" \
    "${HOME:-}/projects/frankentui/pkg" \
    "${HOME:-}/repos/opensource/frankentui/pkg" \
    "${HOME:-}/repos/frankentui/pkg"
  do
    if swimmers_valid_frankentui_pkg_dir "${candidate}"; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done

  return 1
}
