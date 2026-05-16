# v0.3.0 Release Boundary

This file records the source, package, and GitHub Release boundary for the
pending `v0.3.0` release.

## Version And Tag State

- `Cargo.toml` package version: `0.3.0`.
- Implementation checkpoint checked during this pass: `dd9a1c9`.
- Final Beads closeout checkpoint checked during this pass: `494fca1`.
- Latest local release tag checked during this pass: `v0.2.0`.
- `v0.3.0` tag state during this pass: absent locally.
- Release notes source: `CHANGELOG.md` contains a `## [0.3.0]` section, and
  `scripts/release-notes-from-changelog.sh v0.3.0` renders that section.

Release action: create and push tag `v0.3.0` only after the clean-tree package
and publish dry-run gates pass on the release commit.

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

The release workflow builds and publishes Linux amd64 assets for both installed
binaries:

- `swimmers-linux-amd64`
- `swimmers-linux-amd64.sha256`
- `swimmers-tui-linux-amd64`
- `swimmers-tui-linux-amd64.sha256`

This keeps the GitHub Release asset boundary aligned with the crates.io install
boundary: `cargo install swimmers` installs both `swimmers` and `swimmers-tui`.
