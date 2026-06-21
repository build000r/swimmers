#!/usr/bin/env bash
set -euo pipefail

# Regression proof for the token-safe multi-SSH live proof transport.
#
# Drives scripts/test-multi-ssh-env-live.sh in --live mode against a stub `curl`
# that records its argv (but never logs stdin) and writes fixture JSON, using a
# sentinel bearer token. It then asserts the sentinel never appears in curl
# process argv, in the script's stdout/stderr, in any persisted artifact, or in
# any temp filename. The header is expected to travel through curl's stdin
# `--config -` document instead, which is not a process-listing leak vector.
#
# This proof has teeth: against the old `-H "Authorization: Bearer <token>"`
# argv transport it fails (the sentinel shows up in the recorded argv); against
# the stdin-config transport it passes. Point it at an alternate implementation
# with SWIMMERS_LIVE_PROOF_SCRIPT to verify that property.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
proof_script="${SWIMMERS_LIVE_PROOF_SCRIPT:-${ROOT_DIR}/scripts/test-multi-ssh-env-live.sh}"

# A high-entropy sentinel that would never occur in the script or fixtures.
sentinel="SENTINEL_TOKEN_9f83c1a2b7e64d05_do_not_log"

workdir="$(mktemp -d -t swimmers-live-token-safety.XXXXXX)"
cleanup() { rm -rf -- "${workdir}"; }
trap cleanup EXIT

bindir="${workdir}/bin"
artifact_dir="${workdir}/artifacts"
argv_log="${workdir}/curl-argv.log"
script_out="${workdir}/script-out.log"
mkdir -p -- "${bindir}" "${artifact_dir}"
: >"${argv_log}"

# Stub curl: record argv, drain (but never log) stdin, and emit endpoint-shaped
# fixture JSON to the -o target so the live proof reaches every fetch and its
# post-processing succeeds.
cat >"${bindir}/curl" <<STUB
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"${argv_log}"
# Drain the --config stdin document without recording it anywhere.
cat >/dev/null 2>&1 || true
out=""
url=""
prev=""
for arg in "\$@"; do
  if [ "\${prev}" = "-o" ]; then out="\${arg}"; fi
  case "\${arg}" in
    http://*|https://*) url="\${arg}" ;;
  esac
  prev="\${arg}"
done
if [ -n "\${out}" ]; then
  case "\${url}" in
    */v1/sessions)
      printf '%s' '{"environments":[{"id":"sentinel-target","kind":"ssh_only","capabilities":{},"attach_hint":"ssh sentinel-target","bootstrap_hint":"ssh sentinel-target swimmers serve","path_mapping_count":0,"advisory":[]}],"sessions":[]}' >"\${out}"
      ;;
    *)
      printf '%s' '{"ok":true}' >"\${out}"
      ;;
  esac
fi
exit 0
STUB
chmod +x "${bindir}/curl"

export SWIMMERS_LIVE_SENTINEL_TOKEN="${sentinel}"

set +e
PATH="${bindir}:${PATH}" \
  SWIMMERS_LIVE_TARGET_APPROVED=1 \
  SWIMMERS_LIVE_TARGET_ID=sentinel-target \
  SWIMMERS_LIVE_TARGET_KIND=ssh_only \
  SWIMMERS_LIVE_COCKPIT_URL=http://127.0.0.1:9/ \
  SWIMMERS_LIVE_AUTH_TOKEN_ENV=SWIMMERS_LIVE_SENTINEL_TOKEN \
  SWIMMERS_LIVE_ARTIFACT_DIR="${artifact_dir}" \
  bash "${proof_script}" --live >"${script_out}" 2>&1
script_exit=$?
set -e

fail=0
if grep -qF -- "${sentinel}" "${argv_log}"; then
  printf 'FAIL: sentinel token leaked into curl process argv\n' >&2
  fail=1
fi
if grep -qF -- "${sentinel}" "${script_out}"; then
  printf 'FAIL: sentinel token leaked into script stdout/stderr\n' >&2
  fail=1
fi
if grep -rqF -- "${sentinel}" "${artifact_dir}" 2>/dev/null; then
  printf 'FAIL: sentinel token leaked into a persisted artifact\n' >&2
  fail=1
fi
if find "${workdir}" -name "*${sentinel}*" -print 2>/dev/null | grep -q .; then
  printf 'FAIL: sentinel token leaked into a temporary filename\n' >&2
  fail=1
fi

# Confirm the auth path actually ran: the stub must have seen at least two
# curl invocations (health + session inventory). Otherwise an empty argv log
# would pass the leak checks vacuously.
if [ "$(wc -l <"${argv_log}")" -lt 2 ]; then
  printf 'FAIL: expected at least two authenticated curl calls, saw %s\n' \
    "$(wc -l <"${argv_log}")" >&2
  fail=1
fi

if [ "${fail}" -ne 0 ]; then
  printf 'live proof token-safety regression FAILED (proof exit %s)\n' "${script_exit}" >&2
  exit 1
fi

printf 'live proof token-safety regression passed: sentinel absent from argv, output, artifacts, and temp names\n'
