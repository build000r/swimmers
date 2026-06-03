#!/usr/bin/env bash
set -euo pipefail

zig_bin="${ZIG:-}"
if [ -z "$zig_bin" ]; then
  zig_bin="$(command -v zig || true)"
fi
if [ -z "$zig_bin" ] && [ -x "$HOME/.local/bin/zig" ]; then
  zig_bin="$HOME/.local/bin/zig"
fi
if [ -z "$zig_bin" ]; then
  echo "swimmers: zig compiler not found for coverage cc shim" >&2
  exit 127
fi

args=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --target=x86_64-unknown-linux-gnu)
      args+=(--target=x86_64-linux-gnu)
      shift
      ;;
    --target=x86_64-unknown-linux-musl)
      args+=(--target=x86_64-linux-musl)
      shift
      ;;
    -u)
      if [ "$#" -lt 2 ]; then
        args+=("$1")
        shift
      else
        args+=("-Wl,-u,$2")
        shift 2
      fi
      ;;
    *)
      args+=("$1")
      shift
      ;;
  esac
done

exec "$zig_bin" cc "${args[@]}"
