# Multi-SSH Environment V2 Proof

This proof lane covers Swimmers as a configured environment cockpit for local,
remote Swimmers API, and SSH-only handoff targets. The boundary is intentional:
Swimmers can show, sort, filter, and launch against explicitly configured
targets, but it is not an arbitrary SSH fleet manager, cluster scheduler, c0
authority, NTM scheduler, or FrankenTerm control plane.

FrankenTerm may still be useful as a terminal rendering or hypervisor layer.
For this v2 contract, the source of truth remains Swimmers environment config,
path mappings, health snapshots, launch receipts, and passive advisory metadata.

## Fixture Contract

The tracked fixture is
[multi_ssh_env_v2.json](../tests/fixtures/multi_ssh_env_v2.json). It defines:

- `local`, `swimmers_api`, and `ssh_only` target kinds
- a healthy remote API target, a degraded cached API target, and an SSH-only
  handoff target
- mapped local/remote repo cwd, an unmapped cwd, a degraded cached repo, and an
  SSH-only handoff cwd
- local and remote waiting sessions that share one canonical repo key
- stale c0 and NTM advisory metadata that remains external and non-authoritative
- saved lens cases for `all`, `current-repo`, `ssh-handoff`, `degraded`,
  `needs-attention`, and an overlay `swimmers-on-devbox` preset
- launcher receipt expectations for local launch, remote mapped launch,
  unmapped remote rejection, and SSH-only handoff
- proof command keys for fixture shape, capability matrix, UI model buckets,
  launcher receipts, advisory freshness, saved lenses, and redaction rules

The fixture is live-SSH free. It does not connect to hosts, start tmux, launch
browsers, or require a remote Swimmers service.

## One-Command Smoke

Run:

```bash
make multi-ssh-env-smoke
```

The smoke validates fixture shape and then runs focused Rust and web tests for:

- environment inventory redaction
- target health and path-mapping doctor behavior
- unmapped cwd launch guardrails
- TUI launcher preview receipts
- passive advisory freshness labels
- built-in and overlay saved fleet lenses
- web preset chips, URL/query state, and model filtering
- surface action and API contract normalization

By default, Rust build artifacts are written outside the repo with
`CARGO_TARGET_DIR=${TMPDIR:-/tmp}/swimmers-multi-ssh-env-v2-target` when the
caller has not set `CARGO_TARGET_DIR`.

For split validation, use:

```bash
SWIMMERS_MULTI_SSH_SMOKE_SKIP_RUST=1 make multi-ssh-env-smoke
SWIMMERS_MULTI_SSH_SMOKE_SKIP_JS=1 make multi-ssh-env-smoke
```

Those flags exist only to support environments where Rust and Node validation
run on different machines. The default command remains the release contract.

## Operator Setup

Use `local` for the implicit loopback or in-process target. Add
`swimmers_api` targets only for backends you intentionally run and trust:

```yaml
dev_sanity:
  agent_launch:
    default_target: devbox
    group_defaults:
      swimmers: devbox
    targets:
      - id: devbox
        label: Devbox API
        kind: swimmers_api
        base_url: http://100.101.123.63:3210
        auth_token_env: SWIMMERS_DEVBOX_TOKEN
        path_mappings:
          - local_prefix: /Users/me/repos/opensource
            remote_prefix: /srv/devbox/repos/opensource
      - id: skillbox-devbox
        label: Skillbox SSH
        kind: ssh_only
        bootstrap_hint: "ssh skillbox-devbox 'swimmers serve'"

  fleet_lenses:
    - id: swimmers-on-devbox
      label: Swimmers on devbox
      matchers:
        - type: target_kind
          kind: swimmers_api
        - type: repo
          key: /Users/me/repos/opensource/swimmers
```

Run a remote backend with either Tailnet trust or token auth:

```bash
SWIMMERS_BIND=<tailscale-ip> AUTH_MODE=tailnet_trust swimmers serve
```

For token auth, keep values in environment variables. Do not put bearer tokens
in `base_url`, screenshots, docs, lens definitions, or command history.

Built-in saved lenses are `all`, `local`, `remote-api`, `ssh-handoff`,
`current-repo`, `needs-attention`, and `degraded`. Overlay lenses are harmless
labels plus matchers. They can match target id/kind, repo key, current repo,
readiness, transport, capability, degraded, and needs-attention state.

## Supported

- Display configured local, `swimmers_api`, and `ssh_only` targets in one
  capability matrix.
- Filter and switch by target kind, target id, repo key, readiness, degraded
  state, and capability class.
- Show SSH-only targets as inventory/handoff rows with copyable attach and
  bootstrap hints.
- Launch remote sessions only through configured `swimmers_api` targets with
  explicit target/cwd receipts.
- Treat c0 and NTM metadata as stale-by-default passive badges.

## Failure Modes

| Failure mode | Expected behavior |
|--------------|-------------------|
| Wrong-host launch risk | launch preview and receipt name `target_id`, `target_kind`, requested cwd, resolved cwd, and path mapping |
| Unmapped cwd | remote launch is blocked with stable path-mapping guidance |
| Down remote API | cached sessions are marked degraded/stale and demoted below fresh local attention work |
| Missing token env | health/auth guidance is shown without printing token values |
| Stale c0 advisory | badge remains `external` and stale; it never becomes trusted status |
| SSH-only target | attach/bootstrap hints are visible, but live aggregation and input are unavailable |

## Not Supported

- Automatic discovery of every SSH host.
- Implicit remote command execution from imported SSH config.
- Live aggregation from SSH-only targets before a trusted Swimmers API is
  configured there.
- Cross-host tmux session merging or renaming.
- c0, NTM, or FrankenTerm becoming the orchestration source of truth.

## Release Checklist

Run the v1 and v2 fixture lanes:

```bash
make multi-env-smoke
make multi-ssh-env-smoke
```

Run broad repo gates before release:

```bash
cargo test
cargo test --test metamorphic
node --test src/web/*.test.mjs
make tui-check
```

Check the Beads graph without launching interactive `bv`:

```bash
br dep cycles
bv --robot-insights
```

This v2 lane complements `make multi-env-smoke`. It does not broaden Swimmers
into general SSH fleet management.
