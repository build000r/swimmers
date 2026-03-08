# Tmux Hook Setup

Throngterm now owns a stable `clawgs tmux-emit` socket contract so tmux hooks
can trigger immediate rescans between normal interval reconciliations.

## Default socket

By default throngterm launches `clawgs tmux-emit` with:

```text
$HOME/.tmux/clawgs-tmux.sock
```

If you set `THRONGTERM_TMUX_EMIT_SOCKET`, throngterm uses that exact path
instead. A blank override is ignored and falls back to the default above.

## Install the hook snippet

Source the checked-in tmux hook file manually:

```tmux
source-file "/absolute/path/to/throngterm/tmux/throngterm-clawgs-hooks.conf"
```

Or from a running tmux session:

```bash
tmux source-file "/absolute/path/to/throngterm/tmux/throngterm-clawgs-hooks.conf"
```

The snippet is notify-only. Do not also source the upstream clawgs snippet that
starts its own `tmux-emit` daemon, because throngterm already manages that
process.

## Custom sockets

If you override `THRONGTERM_TMUX_EMIT_SOCKET`, copy or edit the snippet so each
`clawgs tmux-notify --socket ...` line uses the same socket path. The runtime
and tmux hooks must match exactly.

## Behavior without hooks

If you never source the snippet, throngterm still works. The daemon continues
its interval-based reconciliation scans and bridge startup should still succeed.
