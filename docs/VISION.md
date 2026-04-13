# Vision

Swimmers is the aquarium for your terminal fleet. It turns tmux sessions into animated fish whose behavior encodes real state — swimming when active, bubbling when busy, sinking when idle, showing `x` eyes when errored — so you can assess a dozen concurrent workloads at a glance instead of reading text.

## Mission

Make multi-session terminal work observable by default. When you run a fleet of AI coding agents, long builds, deploys, and log tails across tmux, you should know the state of every session without context-switching into each one.

## Vision

Terminal sessions rendered as living things on screen. A developer glances at the aquarium and knows — spatially, instantly — which sessions need attention, which are working, and which are idle. No text parsing, no session-name memorization, no `tmux ls`.

## Values

- **Sessions are living things.** The aquarium metaphor is functional, not decorative. Sprite shape, swim speed, and vertical position encode session state. This is information design, not theming.
- **The API is the truth.** The TUI is a client. The server discovers sessions, tracks state, and serves data. Multiple TUIs can point at the same API. The API can run headless. You can build your own client.
- **No infrastructure required.** No database, no Docker, no message broker. Single binary, flat-file persistence, tmux is the only runtime dependency.
- **Thoughts are first-class.** AI agent reasoning (from Claude Code, Codex, etc.) is surfaced in a side panel alongside terminal output. The thought stream is not an afterthought — it is a core data channel.
- **Zero-config by default, configurable when needed.** Loopback bind, local trust auth, sensible defaults. Token auth and remote access are one env var away, not a config file away.

## The Wedge

The thing that gets someone to install swimmers:

> "I have 8 tmux sessions running AI coding agents and I can't tell what any of them are doing."

`tmux ls` returns cryptic one-liners. Switching between sessions is a context-destroying exercise. The developer has no peripheral awareness of their fleet.

Swimmers solves this with a spatial interface: each session is a fish in a bowl. Active ones swim fast, busy ones blow bubbles, idle ones drift to the bottom. You scan the aquarium the same way you scan a Grafana dashboard — pattern recognition, not reading.

The secondary wedge is the thought rail: a side panel showing the internal monologue of AI coding agents running in each session. No other tmux tool surfaces this. For developers running parallel Claude Code or Codex agents, the thought rail is the difference between "what is that session doing?" and knowing at a glance.

## Who It Is For

- **Parallel-agent operators.** Developers running 5-15 concurrent AI coding agents (Claude Code, Codex, Gemini CLI) across tmux sessions who need to monitor progress without context-switching.
- **tmux power users.** Anyone maintaining many concurrent sessions — builds, deploys, log tails, test runs — who has outgrown `tmux ls` and fuzzy finders as a session management strategy.
- **Remote-session monitors.** Teams using tmux on shared servers or VPS instances who want a visual overview over Tailscale or any network, without SSH-ing in to check each session.
- **Single-binary minimalists.** Developers who want session observability without running Docker, databases, or web app infrastructure.

## Who It Is Not For

- **Single-session users.** If you run one or two tmux sessions, tmux itself is sufficient. Swimmers adds value at scale.
- **Session template seekers.** If you want to define tmux layouts, startup commands, and window arrangements, use tmuxinator or tmuxp. Swimmers discovers existing sessions; it does not define them.
- **Non-tmux users.** Swimmers does not support Zellij, screen, or plain terminal tabs. tmux is the only backend.
- **GUI-first developers.** If you work primarily in VS Code or a desktop IDE and rarely touch a terminal multiplexer, swimmers has nothing to offer you.
- **Multi-host aggregation.** Swimmers manages sessions on the machine it runs on. It does not aggregate sessions across a cluster of hosts (though you can run separate instances and point TUIs at each).

## Competitive Fit

| Dimension | swimmers | agent-deck | agent-of-empires | superset | emdash | sesh | tmuxp |
|-----------|----------|------------|-------------------|----------|--------|------|-------|
| **Primary job** | Visual session monitoring + thought streams | Agent session management TUI | Agent session + worktree manager | Multi-agent IDE / desktop app | Parallel agent orchestration env | Smart tmux session switcher | tmux session templating |
| **Session visualization** | Animated ASCII sprites with state encoding | List/panel view | Terminal + web list | Electron pane grid | Terminal panes | Fuzzy finder | None (config tool) |
| **AI thought streams** | Built-in side panel | No | No | No | No | No | No |
| **State detection** | Automatic (idle/busy/error/attention/sleep) | Manual labels | Process-based | Process-based | Process-based | None | None |
| **Architecture** | Rust API server + TUI client | Go TUI (Bubble Tea) | Rust TUI + web | Electron + TypeScript | TypeScript CLI | Go CLI | Python CLI |
| **Infrastructure needs** | tmux only | tmux | tmux + git worktrees | Docker optional | Docker optional | tmux + zoxide | tmux |
| **Remote access** | REST API over any network | No | Web app | Desktop app | Web app | No | No |
| **Observability** | Prometheus /metrics | No | No | No | No | No | No |
| **Language** | Rust | Go | Rust | TypeScript | TypeScript | Go | Python |
| **Stars (Apr 2026)** | 0 (new) | ~2,000 | ~1,600 | ~9,500 | ~3,800 | ~2,300 | ~4,500 |

## Market Map

```
                        AGENT-AWARE
                            ^
                            |
              emdash        |     superset
              (3.8k)        |     (9.5k)
                            |
       agent-of-empires     |
            (1.6k)          |
                            |
         agent-deck         |
           (2.0k)           |
                            |
  TEXT -----+---------------+---------------+--- VISUAL
  LIST      |               |               |
            |          swimmers              |
            |        (state-driven sprites,  |
            |         thought rail)          |
            |               |               |
       sesh (2.3k)          |               |
                            |               |
       tmuxp (4.5k)         |               |
                            |               |
     tmuxinator             |               |
                            |               |
                            |
                     SESSION-ONLY
```

Swimmers occupies a unique position: it is the only tool that combines **visual state encoding** (not just text lists or pane grids) with **AI agent awareness** (thought streams, not just process monitoring). The competitors cluster in two groups: agent orchestrators that use text/pane layouts (upper-left) and traditional session managers that have no agent awareness (lower-left). The visual-and-agent-aware quadrant (upper-right area) is unoccupied except by swimmers.

## Evidence From Comparable Repos

The demand signals from adjacent tools validate both sides of the swimmers wedge:

**Session management demand is proven.** tmuxp (4,500 stars), sesh (2,300 stars), and tmux-sessionx (1,200 stars) prove that developers actively seek better tmux session workflows. The tools succeed despite offering only text-based interfaces, suggesting a visual approach has room to differentiate.

**Agent orchestration is a fast-growing category.** superset (9,500 stars, launched 2026), emdash (3,800 stars, YC W26), and agent-deck (2,000 stars) demonstrate explosive demand for multi-agent terminal management. These tools focus on spawning and routing work to agents but do not invest in spatial/visual state representation.

**The thought-stream gap is real.** None of the surveyed competitors surface AI agent internal reasoning as a first-class UI element. Agent-deck and agent-of-empires show session output but not the structured thought data that Claude Code and Codex emit. Swimmers' thought rail is a genuinely novel feature in this space.

**Rust CLI tools in this category gain traction.** agent-of-empires (1,600 stars, Rust) demonstrates that a Rust-based tmux session manager can find an audience even in a field dominated by Go and TypeScript entries.

## Build on the Backs of Giants

Swimmers does not reinvent infrastructure. It composes proven crates and protocols:

- **tmux** — the session primitive. Swimmers discovers and monitors; tmux does the multiplexing.
- **portable-pty** — cross-platform PTY I/O without hand-rolled platform code.
- **Axum** — production-grade async HTTP server. The API layer is not a hobby web framework.
- **crossterm** — terminal rendering. The TUI draws on the same foundation as most Rust TUI apps.
- **metrics + metrics-exporter-prometheus** — observability is a library call, not a custom implementation.
- **mermaid-rs-renderer + resvg/usvg** — Mermaid diagram rendering in the terminal, built on real SVG processing.
- **clap** — CLI argument parsing with derive macros. Standard Rust CLI ergonomics.
- **chrono, serde, regex** — the Rust ecosystem staples that every tool uses and every developer trusts.

The strategic choice is to build the novel parts (state detection, sprite rendering, thought pipeline) and delegate everything else to battle-tested crates.

## Strategic Non-Goals

These are things swimmers will deliberately not become:

- **Not a tmux replacement.** Swimmers sits above tmux, not beside it. It will never manage windows, panes, or key bindings.
- **Not a session templating tool.** Swimmers discovers sessions that already exist. Defining layouts, startup commands, and window arrangements is tmuxinator's job.
- **Not a multi-host aggregator.** Each swimmers instance manages sessions on one machine. Cross-host aggregation is a different product with different failure modes.
- **Not an IDE or editor.** Swimmers shows session state and thought streams. It does not edit code, run LSP servers, or replace your terminal workflow.
- **Not an agent launcher.** Swimmers monitors agents that are already running. It does not provision, configure, or schedule AI coding agents — that is the job of the agent harness (Claude Code, Codex, etc.).
- **Not a web-first app.** The TUI is the primary interface. The browser surface exists for remote convenience, not as the flagship experience.

## Product Test

The test that validates swimmers is working:

> **The Glance Test.** A developer has 10 tmux sessions running — 3 AI agents compiling, 2 idle, 1 errored, 4 running tests. They open the swimmers TUI. Within 2 seconds, without reading any text, they can point to the errored session (dead fish, `x` eyes), identify the idle ones (sunk to the bottom), and see which agents are actively working (swimming fast, blowing bubbles). They select the errored fish with arrow keys and hit Enter to jump straight into that session in their native terminal.

If a user has to read session names or scroll a text list to get this information, swimmers has failed at its core job.
