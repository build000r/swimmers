# v0.3.0 Release Boundary

This file records the source, package, and GitHub Release boundary for the
pending `v0.3.0` release.

## Version And Tag State

- `Cargo.toml` package version: `0.3.0`.
- Implementation checkpoint checked during the original pass: `dd9a1c9`.
- Final Beads closeout checkpoint checked during the original pass: `494fca1`.
- Local release tags: `v0.1.0`, `v0.1.1`, `v0.1.2`, `v0.1.3`, `v0.2.0`,
  `v0.3.0`.
- `v0.3.0` tag state: present locally and on GitHub, pointing at commit
  `7ba0737`.
- Release notes source: `CHANGELOG.md` contains a `## [0.3.0]` section, and
  `scripts/release-notes-from-changelog.sh v0.3.0` renders that section.

The clean-tree pre-tag gates below were the gate for creating the tag. The
`v0.3.0` tag and its GitHub Release now exist, so rerun them only when cutting a
later release commit.

## Package Boundary

The crate package should include source, assets, public docs, tests, README,
license, changelog, and lockfile. It must not include local operator state or
agent orchestration artifacts such as:

- `.beads/`
- `.buildooor/`
- `.claude/`
- `.codex/`
- `.github/`
- `.mcp.json`
- `.ntm/`
- `data/`
- `scripts/`
- `target/`
- local audit/report scratch files

Clean-tree pre-tag gates:

```bash
cargo package --list
cargo publish --dry-run --locked
```

Results:

- `cargo package --list` returned 164 package entries from `dd9a1c9`.
- The package filter found no local operator paths under `.beads/`,
  `.buildooor/`, `.claude/`, `.codex/`, `.github/`, `.ntm/`, `data/`,
  `scripts/`, `target/`, or `tests/artifacts/`.
- `cargo publish --dry-run --locked` packaged 164 files, verified
  `swimmers v0.3.0`, and aborted only because the command was a dry run.
- The publish dry-run was repeated after final Beads closeout checkpoint
  `494fca1` with the same 164-file package result.

If a later commit becomes the release commit, rerun both commands before
tagging.

## GitHub Release Assets

The release workflow builds and publishes prebuilt assets for both installed
binaries on Linux amd64 and macOS arm64. The published `v0.3.0` GitHub Release
carries these eight assets:

- `swimmers-linux-amd64` / `swimmers-linux-amd64.sha256`
- `swimmers-tui-linux-amd64` / `swimmers-tui-linux-amd64.sha256`
- `swimmers-darwin-arm64` / `swimmers-darwin-arm64.sha256`
- `swimmers-tui-darwin-arm64` / `swimmers-tui-darwin-arm64.sha256`

The `curl ... | sh` installer (`scripts/install.sh`) downloads these prebuilt
binaries and verifies their checksums; macOS (Apple Silicon) and Linux x86_64
are the prebuilt platforms. Publishing to crates.io so that `cargo install
swimmers` installs both binaries from source is still planned, not yet done.

## Live Verification Status

- `git tag --list "v0.3.0"` and `gh release view v0.3.0` confirm the tag and a
  published GitHub Release carrying the eight assets above.
- `scripts/release-notes-from-changelog.sh v0.3.0` renders the changelog
  section.
- Not re-run in this reconciliation pass: end-to-end download, checksum
  verification, and install/run of the published release binaries on a clean
  machine.
