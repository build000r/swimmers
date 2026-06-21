#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  scripts/remote-rust-validate.sh [--dry-run] [--keep] [--] [cargo command...]

Examples:
  make remote-rust-validate-dry-run
  SWIMMERS_REMOTE_RUST_HOST=builder.example make remote-rust-validate
  SWIMMERS_REMOTE_RUST_HOST=builder.example scripts/remote-rust-validate.sh -- cargo test group_membership -- --test-threads=1

Environment:
  SWIMMERS_REMOTE_RUST_HOST       SSH target to use for live validation.
  SWIMMERS_REMOTE_RUST_IMAGE      Container image, default rust:1-bookworm.
  SWIMMERS_REMOTE_RUST_ENGINE     Container engine, default docker.
  SWIMMERS_REMOTE_RUST_BASE       Remote temp base, default /tmp.
  SWIMMERS_REMOTE_RUST_CACHE      Optional absolute remote cache dir for Cargo.
  SWIMMERS_REMOTE_RUST_WSL_DISTRO Optional WSL distro for Windows SSH targets.
  SWIMMERS_REMOTE_RUST_KEEP       Set to 1 to leave remote temp dirs in place.

The helper copies this checkout to a remote temp directory, excluding local
state and secrets, then runs the cargo command in a disposable Rust container
with an isolated Cargo cache/target directory. Set SWIMMERS_REMOTE_RUST_CACHE
to reuse a Cargo cache/target directory across runs on a dedicated builder.
Only tracked working-tree files are copied by default, so add new source files
to git before using this as proof. It is an operator validation lane only;
Swimmers itself still has no Docker runtime dependency.
EOF
}

die() {
  printf '%s\n' "$*" >&2
  exit 2
}

shell_join() {
  local out="" arg
  for arg in "$@"; do
    if [[ -n "${out}" ]]; then
      out+=" "
    fi
    out+="$(printf '%q' "${arg}")"
  done
  printf '%s\n' "${out}"
}

validate_ssh_target() {
  local value="$1"
  [[ -n "${value}" ]] || die "SWIMMERS_REMOTE_RUST_HOST is required for live validation"
  [[ "${value}" != -* ]] || die "SWIMMERS_REMOTE_RUST_HOST must not start with '-'"
  [[ "${value}" != *[[:space:]]* ]] || die "SWIMMERS_REMOTE_RUST_HOST must be a single SSH target, not a command"
  [[ "${value}" != */* ]] || die "SWIMMERS_REMOTE_RUST_HOST must be an SSH target, not a path"
  [[ "${value}" != *:* ]] || die "SWIMMERS_REMOTE_RUST_HOST must use an SSH alias for custom ports or IPv6"
}

validate_engine_name() {
  local value="$1"
  [[ "${value}" =~ ^[A-Za-z0-9._+-]+$ ]] || die "SWIMMERS_REMOTE_RUST_ENGINE must be an executable name such as docker or podman"
}

validate_wsl_distro() {
  local value="$1"
  [[ -z "${value}" || "${value}" =~ ^[A-Za-z0-9._+-]+$ ]] || die "SWIMMERS_REMOTE_RUST_WSL_DISTRO must be a simple WSL distro name such as Ubuntu"
}

write_tracked_manifest() {
  local output="$1"
  git -C "${ROOT_DIR}" ls-files -z | while IFS= read -r -d '' path; do
    if [[ -e "${ROOT_DIR}/${path}" || -L "${ROOT_DIR}/${path}" ]]; then
      printf '%s\0' "${path}"
    fi
  done >"${output}"
}

dry_run=0
keep="${SWIMMERS_REMOTE_RUST_KEEP:-0}"

while (($#)); do
  case "$1" in
    --dry-run)
      dry_run=1
      shift
      ;;
    --keep)
      keep=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      die "unknown option: $1"
      ;;
    *)
      break
      ;;
  esac
done

command_args=("$@")
if ((${#command_args[@]} == 0)); then
  command_args=(cargo test)
fi

host="${SWIMMERS_REMOTE_RUST_HOST:-}"
image="${SWIMMERS_REMOTE_RUST_IMAGE:-rust:1-bookworm}"
engine="${SWIMMERS_REMOTE_RUST_ENGINE:-docker}"
remote_base="${SWIMMERS_REMOTE_RUST_BASE:-/tmp}"
remote_root="${remote_base%/}/swimmers-remote-rust.XXXXXX"
remote_checkout="${remote_root}/checkout"
remote_cache_override="${SWIMMERS_REMOTE_RUST_CACHE:-}"
remote_cache="${remote_cache_override:-${remote_root}/cargo}"
wsl_distro="${SWIMMERS_REMOTE_RUST_WSL_DISTRO:-}"
command_string="$(shell_join "${command_args[@]}")"
command_payload="$(printf '%s' "${command_string}" | base64 | tr -d '\n')"

validate_engine_name "${engine}"
validate_wsl_distro "${wsl_distro}"
[[ "${remote_base}" == /* ]] || die "SWIMMERS_REMOTE_RUST_BASE must be an absolute remote path"
[[ -z "${remote_cache_override}" || "${remote_cache_override}" == /* ]] || die "SWIMMERS_REMOTE_RUST_CACHE must be an absolute remote path"
if [[ -n "${wsl_distro}" ]]; then
  [[ "${remote_base}" != *[[:space:]]* ]] || die "SWIMMERS_REMOTE_RUST_BASE must not contain whitespace in WSL mode"
  [[ -z "${remote_cache_override}" || "${remote_cache_override}" != *[[:space:]]* ]] || die "SWIMMERS_REMOTE_RUST_CACHE must not contain whitespace in WSL mode"
fi

print_plan() {
  printf 'remote Rust validation plan\n'
  printf '  repo:        %s\n' "${ROOT_DIR}"
  printf '  host:        %s\n' "${host:-<set SWIMMERS_REMOTE_RUST_HOST>}"
  printf '  checkout:    %s\n' "${remote_checkout}"
  if [[ -n "${remote_cache_override}" ]]; then
    printf '  cargo cache: %s (persistent)\n' "${remote_cache}"
  else
    printf '  cargo cache: %s (disposable)\n' "${remote_cache}"
  fi
  printf '  engine:      %s\n' "${engine}"
  printf '  image:       %s\n' "${image}"
  printf '  command:     %s\n' "${command_string}"
  printf '  keep:        %s\n' "${keep}"
  if [[ -n "${wsl_distro}" ]]; then
    printf '  remote mode: WSL distro %s through Windows SSH\n' "${wsl_distro}"
    printf '  transfer:    tracked working-tree files only via git ls-files + tar stream\n'
  else
    printf '  remote mode: Linux/POSIX SSH target\n'
    printf '  transfer:    tracked working-tree files only via git ls-files + rsync\n'
  fi
  printf '  not copied:  .git, target, .env, data, local DBs, artifacts, and untracked files\n'
  printf '  cleanup:     %s\n' "$([[ "${keep}" == "1" ]] && printf 'manual' || printf 'automatic')"
}

if [[ "${dry_run}" == "1" ]]; then
  print_plan
  exit 0
fi

validate_ssh_target "${host}"

command -v ssh >/dev/null 2>&1 || die "ssh is required"
command -v git >/dev/null 2>&1 || die "git is required"
command -v base64 >/dev/null 2>&1 || die "base64 is required"
if [[ -n "${wsl_distro}" ]]; then
  command -v tar >/dev/null 2>&1 || die "tar is required for WSL-mode transfer"
else
  command -v rsync >/dev/null 2>&1 || die "rsync is required"
fi

manifest="$(mktemp -t swimmers-remote-rust-files.XXXXXX)"
cleanup_local() {
  rm -f -- "${manifest}"
}
trap cleanup_local EXIT

remote_bash() {
  if [[ -n "${wsl_distro}" ]]; then
    local args=""
    if (($#)); then
      args=" $(shell_join "$@")"
    fi
    ssh "${host}" "wsl -d ${wsl_distro} --cd ~ --exec bash -s --${args}"
  else
    ssh "${host}" bash -s -- "$@"
  fi
}

transfer_checkout() {
  if [[ -n "${wsl_distro}" ]]; then
    COPYFILE_DISABLE=1 tar --no-xattrs -C "${ROOT_DIR}" --null -T "${manifest}" -cf - \
      | ssh "${host}" "wsl -d ${wsl_distro} --cd ~ --exec tar -C ${remote_checkout} -xf -"
  else
    rsync -a --delete --from0 --files-from="${manifest}" "${ROOT_DIR}/" "${host}:${remote_checkout}/"
  fi
}

cleanup_remote() {
  [[ -n "${remote_root}" && "${remote_root}" != *XXXXXX ]] || return 0
  if [[ "${keep}" == "1" ]]; then
    printf 'Keeping remote validation dir on %s:\n  %s\n' "${host}" "${remote_root}"
    return 0
  fi
  remote_bash "${remote_root}" "${engine}" "${image}" <<'REMOTE_CLEANUP' || true
set -euo pipefail
root="$1"
engine="$2"
image="$3"
[[ -n "${root}" ]] || exit 0
base="$(basename -- "${root}")"
[[ "${base}" == swimmers-remote-rust.* ]] || {
  printf 'refusing to cleanup unexpected path: %s\n' "${root}" >&2
  exit 1
}
if rm -rf -- "${root}" 2>/dev/null; then
  exit 0
fi

if command -v "${engine}" >/dev/null 2>&1; then
  "${engine}" run --rm -v "${root}:/cleanup:rw" "${image}" \
    bash -c 'shopt -s dotglob nullglob; rm -rf -- /cleanup/*'
  rmdir -- "${root}" 2>/dev/null || true
fi
REMOTE_CLEANUP
}
cleanup_all() {
  cleanup_remote
  cleanup_local
}
trap cleanup_all EXIT

print_plan
write_tracked_manifest "${manifest}"

remote_root="$(remote_bash "${remote_base%/}" <<'REMOTE_MKTEMP'
set -euo pipefail
base="$1"
mkdir -p -- "${base}"
mktemp -d "${base}/swimmers-remote-rust.XXXXXX"
REMOTE_MKTEMP
)"
remote_checkout="${remote_root}/checkout"
if [[ -z "${remote_cache_override}" ]]; then
  remote_cache="${remote_root}/cargo"
fi

remote_bash "${remote_checkout}" "${remote_cache}" <<'REMOTE_PREP'
set -euo pipefail
mkdir -p -- "$1" "$2"
REMOTE_PREP

transfer_checkout

remote_bash "${remote_checkout}" "${remote_cache}" "${engine}" "${image}" "${command_payload}" <<'REMOTE_RUN'
set -euo pipefail
checkout="$1"
cache="$2"
engine="$3"
image="$4"
command_payload="$5"
command_string="$(printf '%s' "${command_payload}" | base64 -d)"

command -v "${engine}" >/dev/null 2>&1
remote_uid="$(id -u)"
remote_gid="$(id -g)"
mkdir -p -- "${cache}/cargo-home" "${cache}/target" "${cache}/home"

"${engine}" run --rm \
  -u "${remote_uid}:${remote_gid}" \
  -v "${checkout}:/work:rw" \
  -v "${cache}:/swimmers-cache:rw" \
  -w /work \
  -e HOME=/swimmers-cache/home \
  -e CARGO_HOME=/swimmers-cache/cargo-home \
  -e CARGO_TARGET_DIR=/swimmers-cache/target \
  -e PATH=/usr/local/cargo/bin:/swimmers-cache/cargo-home/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  "${image}" \
  bash -c 'command -v cargo >/dev/null 2>&1 || { printf "container image does not provide cargo on PATH; set SWIMMERS_REMOTE_RUST_IMAGE to a Rust toolchain image\n" >&2; exit 127; }; exec bash -c "$1"' \
  _ "${command_string}"
REMOTE_RUN
