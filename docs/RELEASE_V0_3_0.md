# v0.3.0 Release Boundary

This file records the source, package, and GitHub Release boundary for the
pending `v0.3.0` release.

## Version And Tag State

- `Cargo.toml` package version: `0.3.0`.
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

Required pre-tag gates:

```bash
cargo package --list
cargo publish --dry-run --locked
```

During the long-running Beads batch that introduced this file, Cargo refused
the exact clean-tree commands because tracked files were intentionally
uncommitted. The equivalent dirty-tree package inspection was run after the
package exclusion repair to verify that local Beads/Skillbox/Codex state is no
longer part of the crate payload. Rerun the exact commands above after the batch
is committed and before tagging.

## GitHub Release Assets

The release workflow builds and publishes Linux amd64 assets for both installed
binaries:

- `swimmers-linux-amd64`
- `swimmers-linux-amd64.sha256`
- `swimmers-tui-linux-amd64`
- `swimmers-tui-linux-amd64.sha256`

This keeps the GitHub Release asset boundary aligned with the crates.io install
boundary: `cargo install swimmers` installs both `swimmers` and `swimmers-tui`.
