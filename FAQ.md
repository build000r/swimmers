# ThrongTerm FAQ

ThrongTerm is a native terminal UI for tmux-backed shell sessions.

**How do I start it?**
Run `make tui`. That starts the local API if needed and then launches the TUI.

**Does it need anything besides the TUI?**
No. The supported path is the native TUI talking to the Rust API.

**How do I connect to a remote API?**
Set `SWIMMERS_TUI_URL=http://host:port` before launching the TUI. If the API
uses token auth, also set `AUTH_MODE=token` and `AUTH_TOKEN`.

**How do I create a session?**
Create one in the TUI or directly with tmux, for example
`tmux new-session -d -s dev`.

**What happens if I close the TUI?**
Your tmux sessions keep running on the API host. Reopen the TUI to reconnect.

**Can I delete a session?**
Yes, through the TUI or manually in tmux, depending on your workflow and
session delete mode.
