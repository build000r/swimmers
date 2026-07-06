# tmux Crash Evidence Runbook

When a tmux session disappears or Swimmers starts timing out against tmux,
collect evidence before logs rotate or Apport removes crash payloads:

```bash
scripts/collect-tmux-crash-evidence.sh \
  --since "2026-07-06 00:45:00 UTC" \
  --until "2026-07-06 01:05:00 UTC" \
  --output /tmp/swimmers-tmux-crash-20260706
```

Use `--dry-run` first when you only want to verify command availability and
redaction behavior. Pass `--data-dir "$SWIMMERS_DATA_DIR"` if Swimmers is using
a non-default persistence directory. The collector writes a `0700` bundle and
redacts common bearer-token, env-token, and URL token forms from text output.

## First Classification

Start with these files in the bundle:

| Question | Evidence |
|----------|----------|
| Did the host reboot? | `reboot.txt`: `uptime -s`, `who -b`, `journalctl --list-boots`, `last -x` |
| Did the kernel kill a process? | `kernel-incident-lines.txt`: OOM and killed-process lines |
| Did tmux fault? | `kernel-incident-lines.txt`: `traps: tmux`, `general protection`, or `segfault` |
| Did Swimmers stop or restart? | `swimmers-user-journal.txt` and `swimmers-system-journal.txt` |
| Which sessions were at risk? | `tmux-state.txt` and `swimmers-sessions.txt` |
| Is there a crash payload? | `crash-metadata.txt` |

If `/var/crash/_usr_bin_tmux.1000.crash` exists, preserve it before rotation.
The collector records metadata by default. Re-run with `--include-crash-files`
only when you intentionally want the raw crash payload copied into the private
bundle.

## Apport vs coredumpctl

On Ubuntu-like hosts, Apport may capture the tmux crash as
`/var/crash/_usr_bin_tmux.1000.crash`. In that case `coredumpctl` can be empty
or irrelevant because systemd-coredump did not keep the canonical crash record.
Use `crash-metadata.txt` as the source of truth for Apport files and preserve
the raw file for later symbolication.

For symbolication, install the missing tools shown in
`symbolication-tools.txt`. Typical needs are `gdb`, `apport-retrace`,
`binutils` tools such as `addr2line`/`readelf`/`objdump`, and matching debug
symbols for the installed tmux package.

## Isolated tmux Sockets

For critical work, run Swimmers against a separate tmux server so a default
tmux crash does not erase every lane:

```bash
tmux -L tiktok new-session -d -s tiktok
SWIMMERS_TMUX_SOCKET_NAME=tiktok swimmers-tui
```

Use an explicit socket path when you need a fixed filesystem location:

```bash
tmux -S /tmp/swimmers-tiktok.sock new-session -d -s tiktok
SWIMMERS_TMUX_SOCKET_PATH=/tmp/swimmers-tiktok.sock swimmers-tui
```

Set only one of `SWIMMERS_TMUX_SOCKET_NAME` or `SWIMMERS_TMUX_SOCKET_PATH`.
Unset both to return to the default tmux server. During an incident, the
collector records default tmux sessions plus the configured isolated target
when those environment variables are present.

## Manual Smoke Proof

Use this when validating crash-resilience changes on a development host:

```bash
tmux new-session -d -s swimmers-default-smoke
tmux -L swimmers-critical-smoke new-session -d -s swimmers-critical-smoke

tmux list-sessions -F '#{session_name}' | grep swimmers-default-smoke
tmux -L swimmers-critical-smoke list-sessions -F '#{session_name}' | grep swimmers-critical-smoke

SWIMMERS_TMUX_SOCKET_NAME=swimmers-critical-smoke \
  scripts/collect-tmux-crash-evidence.sh --dry-run --since "-10 minutes"
```

Expected result: the default `tmux list-sessions` sees only the default smoke
session, the `tmux -L swimmers-critical-smoke` command sees only the isolated
smoke session, and the collector dry run reports the configured socket-name
target without writing an evidence bundle.

Clean up with:

```bash
tmux kill-session -t =swimmers-default-smoke
tmux -L swimmers-critical-smoke kill-session -t =swimmers-critical-smoke
```

Residual risk remains: tmux can still crash. The accepted hardening goal is to
reduce background probe pressure, reduce blast radius with isolated sockets,
and leave enough evidence to classify reboot, OOM, tmux fault, or service
shutdown after a disappearance.
