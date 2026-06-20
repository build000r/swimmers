#!/bin/sh
# Swimmers installer — downloads prebuilt `swimmers` and `swimmers-tui` binaries
# from GitHub Releases, verifies their checksums, and installs them to
# ~/.local/bin. No source checkout or Rust toolchain required.
#
# Usage:
#   curl --proto '=https' --tlsv1.2 -sSf \
#     https://raw.githubusercontent.com/build000r/swimmers/main/scripts/install.sh | sh
#
# Environment overrides:
#   SWIMMERS_INSTALL_DIR   Install location          (default: $HOME/.local/bin)
#   SWIMMERS_VERSION       Release tag to install    (default: latest, e.g. v0.3.0)
#   SWIMMERS_PLATFORM      Force the asset platform  (e.g. linux-amd64, darwin-arm64)
#
# Prebuilt platforms: macOS arm64 (darwin-arm64), Linux x86_64 (linux-amd64).
# Anything else: build from source — see the README (cargo install --path .).
#
# Requires curl or wget, plus sha256sum or shasum. tmux is a runtime dependency
# (not installed here).

set -eu

REPO="build000r/swimmers"
BINARIES="swimmers swimmers-tui"

PLATFORM=""
VERSION=""

say() { printf 'swimmers-install: %s\n' "$1"; }
err() { printf 'swimmers-install: error: %s\n' "$1" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1; }

# Detect the release asset suffix for this machine. Sets PLATFORM.
detect_platform() {
  if [ -n "${SWIMMERS_PLATFORM:-}" ]; then
    PLATFORM="$SWIMMERS_PLATFORM"
    return
  fi
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64 | aarch64) PLATFORM="darwin-arm64" ;;
        *) err "no prebuilt binary for macOS $arch; build from source (README: cargo install --path .)" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64 | amd64) PLATFORM="linux-amd64" ;;
        *) err "no prebuilt binary for Linux $arch; build from source (README: cargo install --path .)" ;;
      esac
      ;;
    *)
      err "no prebuilt binary for $os/$arch; build from source (README: cargo install --path .)"
      ;;
  esac
}

# Download a URL ($1) to a file ($2).
download() {
  if need curl; then
    curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
  elif need wget; then
    wget -qO "$2" "$1"
  else
    err "need curl or wget to download"
  fi
}

# Fetch a URL ($1) to stdout.
fetch() {
  if need curl; then
    curl --proto '=https' --tlsv1.2 -fsSL "$1"
  elif need wget; then
    wget -qO- "$1"
  else
    err "need curl or wget to download"
  fi
}

# Resolve the release tag to install. Sets VERSION.
resolve_version() {
  if [ -n "${SWIMMERS_VERSION:-}" ]; then
    VERSION="$SWIMMERS_VERSION"
    return
  fi
  body="$(fetch "https://api.github.com/repos/${REPO}/releases/latest")" ||
    err "could not reach the GitHub API to resolve the latest release (set SWIMMERS_VERSION to override)"
  # Line looks like:   "tag_name": "v0.3.0",  -> field 4 when split on '"'.
  VERSION="$(printf '%s\n' "$body" | grep '"tag_name"' | head -n 1 | cut -d'"' -f4)"
  [ -n "$VERSION" ] || err "could not parse the latest release tag (set SWIMMERS_VERSION to override)"
}

# Print the sha256 hash of a file ($1).
sha256_of() {
  if need sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif need shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    err "need sha256sum or shasum to verify checksums"
  fi
}

main() {
  detect_platform
  resolve_version

  install_dir="${SWIMMERS_INSTALL_DIR:-$HOME/.local/bin}"
  base="https://github.com/${REPO}/releases/download/${VERSION}"

  say "installing swimmers ${VERSION} (${PLATFORM}) -> ${install_dir}"

  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf \"$tmp\"" EXIT INT TERM

  for bin in $BINARIES; do
    asset="${bin}-${PLATFORM}"
    say "downloading ${asset}"
    download "${base}/${asset}" "${tmp}/${bin}" ||
      err "failed to download ${asset} from ${VERSION} (does that release include ${PLATFORM} binaries?)"
    download "${base}/${asset}.sha256" "${tmp}/${bin}.sha256" ||
      err "failed to download checksum for ${asset}"

    expected="$(awk '{print $1}' "${tmp}/${bin}.sha256")"
    actual="$(sha256_of "${tmp}/${bin}")"
    [ -n "$expected" ] || err "empty checksum for ${asset}"
    if [ "$expected" != "$actual" ]; then
      err "checksum mismatch for ${asset} (expected ${expected}, got ${actual})"
    fi
    chmod +x "${tmp}/${bin}"
  done

  mkdir -p "$install_dir"
  for bin in $BINARIES; do
    mv -f "${tmp}/${bin}" "${install_dir}/${bin}"
    say "installed ${install_dir}/${bin}"
  done

  say "done."

  case ":${PATH}:" in
    *":${install_dir}:"*) : ;;
    *)
      say "note: ${install_dir} is not on your PATH. Add it with:"
      printf '  export PATH="%s:$PATH"\n' "$install_dir"
      ;;
  esac

  if ! need tmux; then
    say "note: swimmers needs tmux at runtime — install it with 'brew install tmux' (macOS) or 'apt install tmux' (Debian/Ubuntu)."
  fi

  say "run 'swimmers-tui' to start."
}

main
