# swimmers FAQ

swimmers is a tmux-backed session UI built around a Rust API. The native TUI is
the primary workflow, and the browser surface is optional.

**How do I start it?**
Run `make tui`. That starts the local API if needed and then launches the TUI.

**Does it need anything besides the TUI?**
The Rust API is the core dependency. The native TUI is the main supported
workflow, and the browser surface is available when the API is running.

**Is there a browser UI?**
Yes. Start the API and open `/` for the full surface or `/selected` for the
published-selection view. If the API is in token mode, use the browser auth
sheet with `AUTH_TOKEN` or `OBSERVER_TOKEN`.

**How do I connect to a remote API?**
Set `SWIMMERS_TUI_URL=http://host:port` before launching the TUI. If the API
uses token auth, also set `AUTH_MODE=token` and `AUTH_TOKEN`. The remote host
must opt into a non-loopback `SWIMMERS_BIND`; the default is loopback-only, and
non-loopback `AUTH_MODE=local_trust` is refused.

**How do I create a session?**
Create one in the TUI or directly with tmux, for example
`tmux new-session -d -s dev`.

**What happens if I close the TUI?**
Your tmux sessions keep running on the API host. Reopen the TUI to reconnect.

**Can I delete a session?**
Yes, through the TUI or manually in tmux, depending on your workflow and
session delete mode.
