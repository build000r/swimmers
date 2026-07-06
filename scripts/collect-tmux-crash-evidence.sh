#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  scripts/collect-tmux-crash-evidence.sh [options]

Options:
  --since VALUE              journalctl --since value, default "-2 hours"
  --until VALUE              journalctl --until value, default "now"
  --output DIR              evidence output directory
  --data-dir DIR            swimmers data directory containing session_registry.json
  --include-crash-files     copy matching /var/crash/*tmux* files into the bundle
  --dry-run                 print the collection plan without writing files
  -h, --help                show this help

The collector redacts common bearer-token and token-query forms from captured
text. It collects crash-file metadata by default; use --include-crash-files only
when you intend to preserve private crash payloads with mode 0600.
EOF
}

die() {
  printf '%s\n' "$*" >&2
  exit 2
}

timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
since="-2 hours"
until="now"
output_dir="${PWD}/tmux-crash-evidence-${timestamp}"
data_dir="${SWIMMERS_DATA_DIR:-}"
include_crash_files=0
dry_run=0

while (($#)); do
  case "$1" in
    --since)
      (($# >= 2)) || die "--since requires a value"
      since="$2"
      shift 2
      ;;
    --until)
      (($# >= 2)) || die "--until requires a value"
      until="$2"
      shift 2
      ;;
    --output)
      (($# >= 2)) || die "--output requires a directory"
      output_dir="$2"
      shift 2
      ;;
    --data-dir)
      (($# >= 2)) || die "--data-dir requires a directory"
      data_dir="$2"
      shift 2
      ;;
    --include-crash-files)
      include_crash_files=1
      shift
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

redact_stream() {
  sed -E \
    -e 's/(Authorization:[[:space:]]*Bearer[[:space:]]+)[^[:space:]]+/\1<redacted>/Ig' \
    -e 's/((AUTH_TOKEN|OBSERVER_TOKEN|SWIMMERS_TOKEN|SWIMMERS_AUTH_TOKEN)=)[^[:space:]]+/\1<redacted>/g' \
    -e 's/([?&](token|auth_token|observer_token)=)[^&[:space:]]+/\1<redacted>/Ig' \
    -e 's/(Bearer[[:space:]]+)[A-Za-z0-9._~+\/=-]{16,}/\1<redacted>/g'
}

run_redacted() {
  local label="$1"
  local destination="$2"
  shift 2
  {
    printf '# %s\n' "${label}"
    printf '# command:'
    printf ' %q' "$@"
    printf '\n\n'
    "$@" 2>&1 || printf '\n# command exited with status %s\n' "$?"
  } | redact_stream >"${destination}"
}

capture_function() {
  local label="$1"
  local destination="$2"
  local function_name="$3"
  {
    printf '# %s\n\n' "${label}"
    "${function_name}" 2>&1 || printf '\n# collector section exited with status %s\n' "$?"
  } | redact_stream >"${destination}"
}

grep_incident_terms() {
  if command -v rg >/dev/null 2>&1; then
    rg -i 'traps: tmux|general protection|segfault|out of memory|oom|killed process' || true
  else
    grep -Ei 'traps: tmux|general protection|segfault|out of memory|oom|killed process' || true
  fi
}

collect_kernel_journal() {
  if ! command -v journalctl >/dev/null 2>&1; then
    printf 'journalctl not found\n'
    return 0
  fi
  journalctl -k --since "${since}" --until "${until}" --no-pager | grep_incident_terms
}

collect_user_swimmers_journal() {
  if ! command -v journalctl >/dev/null 2>&1; then
    printf 'journalctl not found\n'
    return 0
  fi
  journalctl --user -u swimmers --since "${since}" --until "${until}" --no-pager || true
}

collect_system_swimmers_journal() {
  if ! command -v journalctl >/dev/null 2>&1; then
    printf 'journalctl not found\n'
    return 0
  fi
  journalctl -u swimmers --since "${since}" --until "${until}" --no-pager || true
}

collect_reboot_evidence() {
  date -u
  printf '\n## uptime -s\n'
  uptime -s || true
  printf '\n## who -b\n'
  who -b || true
  printf '\n## journalctl --list-boots\n'
  if command -v journalctl >/dev/null 2>&1; then
    journalctl --list-boots --no-pager || true
  else
    printf 'journalctl not found\n'
  fi
  printf '\n## last -x\n'
  last -x | head -n 40 || true
}

collect_crash_metadata() {
  shopt -s nullglob
  local files=(/var/crash/*tmux*)
  if ((${#files[@]} == 0)); then
    printf 'no /var/crash/*tmux* files found\n'
    return 0
  fi
  for crash_file in "${files[@]}"; do
    printf '## %s\n' "${crash_file}"
    ls -lh -- "${crash_file}" || true
    stat -- "${crash_file}" || true
    printf '\n'
  done
}

collect_coredump_metadata() {
  if ! command -v coredumpctl >/dev/null 2>&1; then
    printf 'coredumpctl not found\n'
    return 0
  fi
  coredumpctl list tmux --since "${since}" --until "${until}" --no-pager || true
}

collect_tmux_state() {
  printf '## tmux version\n'
  tmux -V || true

  printf '\n## default server sessions\n'
  tmux list-sessions || true

  printf '\n## default server sockets\n'
  tmux list-sockets || true

  if [[ -n "${SWIMMERS_TMUX_SOCKET_NAME:-}" ]]; then
    printf '\n## configured socket-name sessions: %s\n' "${SWIMMERS_TMUX_SOCKET_NAME}"
    tmux -L "${SWIMMERS_TMUX_SOCKET_NAME}" list-sessions || true
  fi

  if [[ -n "${SWIMMERS_TMUX_SOCKET_PATH:-}" ]]; then
    printf '\n## configured socket-path sessions: %s\n' "${SWIMMERS_TMUX_SOCKET_PATH}"
    tmux -S "${SWIMMERS_TMUX_SOCKET_PATH}" list-sessions || true
  fi

  printf '\n## likely tmux sockets under /tmp\n'
  find /tmp -maxdepth 3 -type s \( -name 'default' -o -name 'tmux*' -o -path '*/tmux-*/*' \) -print 2>/dev/null || true
}

collect_tmux_package() {
  tmux -V || true
  if command -v dpkg-query >/dev/null 2>&1; then
    printf '\n## dpkg-query tmux\n'
    dpkg-query -W -f='${Package}\t${Version}\t${Architecture}\n' tmux || true
  fi
  if command -v apt-cache >/dev/null 2>&1; then
    printf '\n## apt-cache policy tmux\n'
    apt-cache policy tmux || true
  fi
  if command -v rpm >/dev/null 2>&1; then
    printf '\n## rpm tmux\n'
    rpm -qi tmux || true
  fi
  if command -v pacman >/dev/null 2>&1; then
    printf '\n## pacman tmux\n'
    pacman -Qi tmux || true
  fi
  if command -v brew >/dev/null 2>&1; then
    printf '\n## brew tmux\n'
    brew info tmux || true
  fi
}

collect_swimmers_sessions() {
  local registry=""
  if [[ -n "${data_dir}" ]]; then
    registry="${data_dir%/}/session_registry.json"
  fi
  if [[ -z "${registry}" || ! -f "${registry}" ]]; then
    printf 'session_registry.json not found'
    if [[ -n "${data_dir}" ]]; then
      printf ' under %s' "${data_dir}"
    else
      printf '; set SWIMMERS_DATA_DIR or pass --data-dir'
    fi
    printf '\n'
    return 0
  fi

  printf 'registry: %s\n\n' "${registry}"
  if command -v jq >/dev/null 2>&1; then
    jq -r '
      .. | objects
      | select(has("session_id") and has("tmux_name"))
      | [
          .session_id,
          .tmux_name,
          (.tmux_target // "default"),
          (.state // ""),
          (.cwd // "")
        ]
      | @tsv
    ' "${registry}" || true
  else
    printf 'jq not found; writing first 200 redacted lines of registry for session IDs\n\n'
    sed -n '1,200p' "${registry}"
  fi
}

collect_tooling() {
  local tools=(gdb apport-retrace addr2line readelf objdump c++filt coredumpctl journalctl rg jq)
  for tool in "${tools[@]}"; do
    if command -v "${tool}" >/dev/null 2>&1; then
      printf '%-16s %s\n' "${tool}" "$(command -v "${tool}")"
    else
      printf '%-16s missing\n' "${tool}"
    fi
  done
}

write_manifest() {
  cat >"${output_dir}/README.txt" <<EOF
tmux crash evidence bundle

created_utc: ${timestamp}
repo: ${ROOT_DIR}
since: ${since}
until: ${until}
data_dir: ${data_dir:-<unset>}
included_crash_files: ${include_crash_files}

Start with:
  reboot.txt
  kernel-incident-lines.txt
  swimmers-user-journal.txt
  swimmers-system-journal.txt
  crash-metadata.txt
  tmux-state.txt
  swimmers-sessions.txt

Crash files are metadata-only unless included_crash_files=1. Preserve
/var/crash/*tmux* separately if the raw crash payload may be needed for
symbolication.
EOF
}

print_plan() {
  cat <<EOF
tmux crash evidence collection plan
  since:              ${since}
  until:              ${until}
  output:             ${output_dir}
  data_dir:           ${data_dir:-<unset>}
  include_crash_files:${include_crash_files}

sections:
  reboot evidence
  kernel incident journal lines
  user and system swimmers journals
  /var/crash/*tmux* metadata
  coredumpctl tmux metadata
  tmux version/package/sockets/session lists
  swimmers session registry IDs
  symbolication tool availability

redaction:
  Authorization: Bearer <token>
  AUTH_TOKEN/OBSERVER_TOKEN/SWIMMERS_TOKEN assignments
  token/auth_token/observer_token URL query values
EOF
}

if ((dry_run)); then
  print_plan
  collect_tooling | sed 's/^/  /'
  exit 0
fi

mkdir -p "${output_dir}"
chmod 700 "${output_dir}"

write_manifest
capture_function "reboot and host timeline" "${output_dir}/reboot.txt" collect_reboot_evidence
capture_function "kernel incident lines" "${output_dir}/kernel-incident-lines.txt" collect_kernel_journal
capture_function "user swimmers journal" "${output_dir}/swimmers-user-journal.txt" collect_user_swimmers_journal
capture_function "system swimmers journal" "${output_dir}/swimmers-system-journal.txt" collect_system_swimmers_journal
capture_function "apport crash metadata" "${output_dir}/crash-metadata.txt" collect_crash_metadata
capture_function "coredumpctl metadata" "${output_dir}/coredumpctl.txt" collect_coredump_metadata
capture_function "tmux state" "${output_dir}/tmux-state.txt" collect_tmux_state
capture_function "tmux package" "${output_dir}/tmux-package.txt" collect_tmux_package
capture_function "swimmers session registry" "${output_dir}/swimmers-sessions.txt" collect_swimmers_sessions
capture_function "symbolication tooling" "${output_dir}/symbolication-tools.txt" collect_tooling

if ((include_crash_files)); then
  mkdir -p "${output_dir}/crash-files"
  chmod 700 "${output_dir}/crash-files"
  shopt -s nullglob
  for crash_file in /var/crash/*tmux*; do
    cp -p -- "${crash_file}" "${output_dir}/crash-files/"
  done
fi

printf 'tmux crash evidence written to %s\n' "${output_dir}"
