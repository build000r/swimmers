# Multi-Environment Cockpit Proof

This proof lane covers Swimmers as a configured local plus remote cockpit. The scope is deliberately narrow: one local Swimmers API can aggregate explicitly configured `kind: swimmers_api` launch targets from the operator's Skillbox overlay. Swimmers is not an arbitrary SSH fleet manager, a cluster scheduler, an NTM/c0 scheduler, or a FrankenTerm control plane.

FrankenTerm can still be useful as a renderer or terminal-hypervisor integration later. The orchestration truth for this feature remains Swimmers API state, launch-target config, path mappings, health snapshots, and namespaced remote session IDs.

## Fixture Contract

The tracked fixture is [multi_env_cockpit.json](../tests/fixtures/multi_env_cockpit.json). It defines:

- one local target and one fake remote `swimmers_api` target named `devbox`
- mapped and unmapped cwd cases
- a degraded cached remote case
- local and remote waiting sessions
- a remote advisory metadata badge that is external and stale by design
- proof command keys for inventory, health doctor, launch preview, remote input, group input, fleet lens, native TUI fleet filters, grouped display, attention inbox, and advisory metadata

The fixture is not a generated artifact and does not start tmux, SSH, browsers, or remote servers. It is a compact release contract that points at focused tests already covering the behavior.

## One-Command Smoke

Run:

```bash
make multi-env-smoke
```

This validates the fixture JSON, checks that its required proof-command map is complete, then runs focused Rust and web tests for:

- remote environment inventory with token redaction
- target health and path-mapping doctor output
- mapped and unmapped remote launch preview
- remote single-session and same-target group input proxying
- mixed-target group input rejection
- local attention-group boundary preservation
- cross-host operator pressure inbox behavior
- TUI remote launch, native fleet filters, and thought rail summaries
- web fleet lens filters and grouped display
- passive advisory metadata display as external and stale

The smoke is safe to run on a laptop because it uses test doubles and fixtures only.

## Operator Setup

For a real remote target, run `swimmers serve` on the remote machine and expose it only through a trusted network path:

```bash
SWIMMERS_BIND=<tailscale-ip> AUTH_MODE=tailnet_trust swimmers serve
```

For token-auth targets, put the token in the environment named by the overlay's `auth_token_env`. Do not put tokens in `base_url`, command history, docs, screenshots, or UI labels.

In the Skillbox overlay, declare an `agent_launch` target with:

- `kind: swimmers_api`
- a stable target id and host label
- `base_url` pointing at the remote Swimmers API
- `auth_token_env` only when the target uses `AUTH_MODE=token`
- `path_mappings` from local repo roots to remote repo roots

The local TUI can then launch mapped directories on the remote API while keeping returned IDs namespaced as `target::session_id`. `SWIMMERS_TUI_URL` is different: it points the whole TUI at one backend instead of adding selectable remote launch targets to the local cockpit.

## Failure Modes

- Missing auth or observer credentials: health surfaces auth guidance without printing token env names or token values.
- Target URL contains credentials: the environment summary redacts the credentialed URL.
- Target points back at the same API: remote launch avoids recursive launch-target propagation.
- Cwd has no path mapping: launch preview blocks the remote target with stable mapping guidance.
- Remote target is unreachable: cached remote sessions are marked degraded/stale and should not outrank fresh local attention work.
- Group input mixes local and remote sessions: the request is rejected.
- Group input mixes two remote targets: the request is rejected.
- Advisory JSON is malformed or tries to set trusted status: malformed entries are ignored, and imported entries remain `external` and `stale`.
- FrankenTerm is present: treat it as optional renderer/integration evidence, not as Swimmers' environment source of truth.

## Release Checklist

Run the focused lane first:

```bash
make multi-env-smoke
```

Run the broad repo gates before release:

```bash
cargo test
cargo test --test metamorphic
node --test src/web/*.test.mjs
make tui-check
```

Run the perf/concurrency gate when the local machine has enough disk and runtime headroom:

```bash
make ci-perf-gates
```

Validate the Beads graph and graph-aware triage without launching interactive `bv`:

```bash
br dep cycles
bv --robot-insights
```

Before closing the multi-env epic, check that every implementation bead names its validation command and that this proof doc still matches the current fixture.
