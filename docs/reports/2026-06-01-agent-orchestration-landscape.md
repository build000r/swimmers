# Agent Orchestration Landscape for Swimmers - 2026-06-01

This report answers Bead `swimmers-deep-research-agent-orchestration-landscape-jft`.
It is scoped to local developer tools for terminal, tmux, or desktop orchestration of
AI coding-agent sessions. It does not evaluate cloud-hosted agent platforms, IDE
extensions, notebook orchestration, or general CI/CD schedulers.

Source date: June 1, 2026. GitHub star counts are current GitHub API values
captured on June 1, 2026; April 2026 counts are the snapshot in
`docs/VISION.md`.

## Executive Summary

- The single best distribution move for swimmers is a focused Hacker News launch
  with a short animated proof: "Show HN: Swimmers - see which local AI coding
  agents need attention from tmux." HN is the cleanest measured channel in this
  category: Superset had 96 points on a December 2025 Show HN and 107 points on
  a May 2026 Launch HN, while Emdash had 206 points on a February 2026 Show HN.
  [4][5][11]
- The strongest unclaimed gap remains the thought-stream rail: a first-class
  side panel that extracts Claude Code/Codex reasoning traces or structured
  internal output across many local sessions. Competitors document terminals,
  status, conversations, diff review, plan/tool-call cards, and remote control;
  none of the six named tools documents an equivalent thought rail as of
  June 1, 2026. [2][9][15][21][27][31]
- The category is converging on git worktrees, provider-agnostic CLI launchers,
  status labels, diff review, notifications, and remote/mobile access. It is
  not converging on spatial state encoding. That leaves swimmers' visual fish
  map differentiated if the thought rail is reliable.
- tmux is bifurcating, not disappearing. Agent Deck and Agent of Empires are
  tmux-native and grew from the April snapshot by about 29% and 55% respectively,
  while the fastest desktop apps, Superset and Emdash, use desktop PTYs,
  worktrees, and SSH rather than tmux as their core session primitive. [14][20][44]
- Crates.io alone is not enough for launch discovery. Swimmers has 39 total
  crate downloads and 1 download in the last 7 days, while established Rust
  adjacent CLIs like Zellij and Bacon have 1,730 and 1,863 crate downloads in
  the same 7-day window. Use crates.io as the install path; use HN, Reddit,
  Product Hunt, and Homebrew/GitHub Releases as distribution surfaces. [34][35][36]
- No named competitor shows a paid single-user terminal visualization tier.
  The market still rewards free local tooling; monetization, if any, is more
  credible around team dashboards, hosted remote access, support, or commercial
  desktop packaging than around the core local TUI.

## Current Feature Map

| Tool | GitHub stars on 2026-06-01 | Real-time state | Thought stream | Visual/spatial overview | Beyond tmux | Remote/network |
| --- | ---: | --- | --- | --- | --- | --- |
| Superset | 11,482 | Yes, agent monitoring | No documented equivalent | Desktop terminal/worktree views | Yes: macOS Electron app, worktree manager | Editor/terminal handoff; no browser remote in README |
| Emdash | 4,718 | Yes, parallel task status | No documented equivalent | Desktop app lists/diff/task views | Yes: desktop app, local SQLite, SSH/SFTP, integrations | Yes: SSH/SFTP remote machines |
| Agent Deck | 2,585 | Yes, smart polling/status labels | Partial conversation search, no side rail | Text/status TUI and web UI | Optional Docker, MCP/socket pooling, config DB; tmux core | Yes: SSH remote subcommands, optional Telegram/Slack conductors |
| Agent of Empires | 2,480 | Yes, running/waiting/idle/error | Partial ACP plan/tool-call cards, no side rail | TUI, web dashboard, mobile cockpit | Optional Docker/Podman/Apple Containers; tmux core | Yes: web dashboard, PWA, Tailscale/Cloudflare tunnel |
| Sesh | 2,552 | No AI state detection | No | Picker/list of tmux sessions/windows | Requires tmux and zoxide; no AI infra | No built-in network remote |
| tmuxp | 4,513 | No AI state detection | No | Config-driven sessions, no live fleet UI | Requires tmux plus Python/libtmux | No built-in network remote |

## Per-Competitor Profiles

### Superset - TypeScript/Electron - 11,482 stars as of 2026-06-01

- **Primary job-to-be-done:** Desktop code editor for running many CLI coding
  agents in isolated git worktrees, then reviewing diffs and opening workspaces
  in an editor or terminal. [1][2]
- **Session visualization:** Desktop terminal/worktree views with agent
  monitoring and notifications. This is visual application UI, but not a
  spatial map of many sessions. [2]
- **AI thought-stream feature:** No documented equivalent. README and release
  notes emphasize terminal panes, Run tabs, diff review, and terminal agent
  tracking; they do not describe extracting reasoning traces into a side rail.
  [2][3]
- **Infrastructure requirements:** Requires the Superset desktop app on macOS.
  Local development requires Bun, Git, GitHub CLI, Caddy, and Docker for the
  developer stack, but end users are positioned around the desktop app and
  git worktrees rather than tmux. [2]
- **Remote access:** Not found as a first-class browser/mobile remote access
  feature in the README. The documented handoff is to editor or terminal. [2]
- **Notable feature additions since April 2026:** May 2026 release notes include
  terminal preset launch fixes, Run tab reuse, terminal-agent tracker/pane
  wiring, and inline agent-comment composition in the diff pane. [3]
- **Primary distribution channel for growth:** HN plus Product Hunt plus YC.
  HN shows a December 2025 Show HN with 96 points and a May 2026 Launch HN with
  107 points; YC lists Superset as Spring 2026; Product Hunt has official
  Superset product pages. [4][5][6][7]
- **Business model:** Source-available under Elastic License 2.0 according to
  the README; GitHub API reports `NOASSERTION` for SPDX. No single-user paid
  tier found in the README. [1][2]
- **Key citations:** [1][2][3][4][5][6][7]

### Emdash - TypeScript/Desktop - 4,718 stars as of 2026-06-01

- **Primary job-to-be-done:** Provider-agnostic desktop Agentic Development
  Environment for running multiple coding agents side by side, usually from
  tasks/tickets, and turning accepted output into PRs. [8][9]
- **Session visualization:** Desktop task/session lists, diff review, PR and
  CI status, and terminal/diff support. This is structured desktop workflow UI,
  not a spatial state map. [9]
- **AI thought-stream feature:** No documented equivalent. README and recent
  releases mention terminal, diff, task editing, PR flows, and remote SSH, but
  not parsing or separately rendering reasoning traces. [9][10]
- **Infrastructure requirements:** Desktop app with local SQLite storage,
  integrations, and optional remote SSH/SFTP. It does not use tmux as its
  documented core primitive. [9]
- **Remote access:** Yes. Emdash connects to remote machines via SSH/SFTP and
  supports remote projects with SSH agent/key authentication. [9]
- **Notable feature additions since April 2026:** v1.1.27 added smarter diff
  tree behavior, compact folders, task editing improvements, terminal clipboard
  and default shell fixes, and SSH/project reliability fixes. Earlier late-May
  releases added GitHub Enterprise remotes, SSH ProxyJump/ProxyCommand/
  ForwardAgent/MaxSessions support, and terminal/diff improvements. [10]
- **Primary distribution channel for growth:** HN plus YC plus Product Hunt.
  HN shows a February 2026 Show HN with 206 points; YC lists Emdash as Winter
  2026; Product Hunt has an official Emdash product page. [11][12][13]
- **Business model:** Apache-2.0 open source. No paid tier found in README;
  the README emphasizes local storage while noting agent providers may send
  code/prompts to their own cloud APIs. [8][9]
- **Key citations:** [8][9][10][11][12][13]

### Agent Deck - Go/Bubble Tea/tmux - 2,585 stars as of 2026-06-01

- **Primary job-to-be-done:** Tmux-centered command center for many Claude,
  Gemini, Codex, and other AI coding-agent sessions, with conductors,
  worktrees, MCP management, and session search. [14][15]
- **Session visualization:** Text/status TUI plus web UI. It shows running,
  waiting, idle, and error states and filters sessions by status; it does not
  offer sprite-like spatial visualization. [15]
- **AI thought-stream feature:** Partial adjacent capability only. It supports
  global search across Claude conversations and session forking with inherited
  conversation history, but no documented thought rail that streams reasoning
  blocks beside every session. [15]
- **Infrastructure requirements:** Tmux is central. It also has optional Docker
  sandboxing, MCP socket pooling, skills/MCP registries, local state, and
  conductor configuration. [15]
- **Remote access:** Yes. README documents remote SSH servers, remote sessions,
  remote attach/update, optional Telegram/Slack conductors, and web UI. [15]
- **Notable feature additions since April 2026:** v1.9.45 added wake-nudge for
  near-instant idle-conductor delivery; v1.9.44 added durable per-parent
  conductor outboxes; v1.9.41 added installer and bind security hardening. [16]
- **Primary distribution channel for growth:** GitHub/Discord plus targeted
  Reddit more than HN. Its HN Show HN had only 3 points, while Reddit posts
  in r/ClaudeCode and r/commandline show visible niche demand for managing many
  terminal tabs or tmux-based AI sessions. [17][18][19]
- **Business model:** MIT free tool; no paid tier found. [14][15]
- **Key citations:** [14][15][16][17][18][19]

### Agent of Empires - Rust/tmux - 2,480 stars as of 2026-06-01

- **Primary job-to-be-done:** Rust TUI and browser dashboard for managing AI
  coding agents across branches, tmux sessions, worktrees, optional sandboxes,
  and remote phone/tablet access. [20][21]
- **Session visualization:** TUI dashboard, web dashboard, status column,
  tmux status bar, and mobile cockpit. It gives structured views, but not a
  spatial/sprite overview like swimmers. [21]
- **AI thought-stream feature:** Partial adjacent capability only. Cockpit uses
  Agent Client Protocol rendering with plan panels and tool-call cards, but the
  README does not document a side rail for Claude Code/Codex reasoning traces
  across tmux sessions. [21]
- **Infrastructure requirements:** Tmux is central. Optional Docker sandboxing,
  Podman, Apple Containers, worktrees, HTTP API, PWA/browser dashboard, and
  remote tunneling add more surface area. [21]
- **Remote access:** Yes. Browser dashboard, installable PWA, remote phone
  access via Tailscale Funnel or Cloudflare Tunnel, and HTTP API are documented.
  [21]
- **Notable feature additions since April 2026:** v1.9.5 added TUI mouse click
  and hover support; v1.9.4 added bracketed paste for multiline live sends;
  v1.9.1 reconciled completed Codex hook prompts and cockpit live-test fixes.
  [22]
- **Primary distribution channel for growth:** GitHub/Trendshift badge,
  Homebrew, Reddit in AI-coding communities, YouTube/X/Discord, and release
  artifacts. Homebrew reports 783 installs in 30 days; the latest 30 GitHub
  releases show 4,556 asset downloads. [21][23][24][25][43]
- **Business model:** MIT free tool; no paid tier found. Mozilla.ai support is
  noted by the README, but this is not a paid user tier. [20][21]
- **Key citations:** [20][21][22][23][24][25][43]

### Sesh - Go/tmux/zoxide - 2,552 stars as of 2026-06-01

- **Primary job-to-be-done:** Fast smart tmux session switching and session
  creation using zoxide, picker integrations, and per-project config. [26][27]
- **Session visualization:** Text picker/list of tmux sessions and windows,
  with fzf/television/gum integrations and preview commands. It is not AI
  aware. [27]
- **AI thought-stream feature:** No. It does not target AI coding agents or
  parse agent output. [27]
- **Infrastructure requirements:** Requires tmux and zoxide for the core
  experience. It can be configured for tmux-compatible multiplexers such as
  psmux, and its roadmap language mentions possible future multiplexer
  agnosticism. [27]
- **Remote access:** No built-in remote network feature found. It relies on
  tmux/shell workflows and external extensions. [27]
- **Notable feature additions since April 2026:** v2.26.2 reverted the startup
  command method to `send-keys`; v2.26.0 added picker configuration,
  blacklist flags, env-var expansion in paths, `tmux_command`, and worktree
  naming strategy. [28]
- **Primary distribution channel for growth:** Long-running organic tmux CLI
  adoption, Homebrew, GitHub Releases, and community integrations. Homebrew
  reports 970 installs in 30 days; latest 30 GitHub releases show 33,196 asset
  downloads. [28][29][40]
- **Business model:** MIT free tool; no paid tier found. [26][27]
- **Key citations:** [26][27][28][29][40]

### tmuxp - Python/libtmux - 4,513 stars as of 2026-06-01

- **Primary job-to-be-done:** Define, save, load, freeze, convert, and script
  tmux sessions through YAML/JSON/tmuxinator-style configuration files. [30][31]
- **Session visualization:** No live orchestration dashboard. It loads and
  freezes tmux layouts and exposes tmux objects through libtmux. [31]
- **AI thought-stream feature:** No. It is a general tmux session manager, not
  an AI-agent orchestration surface. [31]
- **Infrastructure requirements:** Requires tmux and Python/libtmux; packaged
  through pip, uvx, Homebrew, Debian, and Nix. [31]
- **Remote access:** No built-in remote network feature found. [31]
- **Notable feature additions since April 2026:** v1.70.0 updated libtmux for
  UTF-8 locale fixes; v1.69.0 added client awareness, native tmux filtering,
  and expanded format tokens; v1.67.0 added a `tmuxp load` progress spinner.
  [32]
- **Primary distribution channel for growth:** Mature package-manager adoption
  and tmux ecosystem usage. Homebrew reports 614 installs in 30 days; PyPI is
  the primary package surface, while GitHub release assets are not used for
  downloads. [31][32][33][41]
- **Business model:** MIT free tool; no paid tier found. [30][31]
- **Key citations:** [30][31][32][33][41]

## Thought-Stream Gap Analysis

The thought-stream feature category is unclaimed by the named competitors as
of June 1, 2026. The strongest partials are Agent Deck's global search across
Claude conversations and Agent of Empires' ACP cockpit with plan panels and
tool-call cards. Those are useful structured-state features, but neither is
documented as a live side rail that extracts and renders Claude Code/Codex
reasoning traces across many running tmux sessions. [15][21]

Evidence for demand is indirect but consistent: public launches with the
highest engagement are about monitoring many agents and reducing context
switching. Superset's HN positioning is "run 10 parallel coding agents" and
"IDE for the agents era"; Emdash positions around running multiple agents
side by side; Agent Deck's README opens with the pain of too many terminal
tabs and not knowing what is running, waiting, or done; Agent of Empires says
running five agents across branches becomes a part-time job. [4][5][9][11][15][21]

Direct public demand for raw "thought streams" is much less visible. Targeted
GitHub issue searches for "thought", "reasoning", "transcript", and
"monologue" across Superset, Emdash, Agent Deck, Agent of Empires, Sesh, and
tmuxp did not surface a named competitor issue requesting an equivalent side
rail in this pass. That absence should be treated as a market gap, not proof
that users will ask for the feature in that vocabulary.

For Superset or Emdash to replicate the feature, they would need four pieces:
agent-specific parsers for Claude Code and Codex output contracts; persistence
for extracted thought events separate from terminal scrollback; a privacy
model because reasoning traces may expose prompts, secrets, or code; and a
multi-session UI that makes reasoning glanceable without turning into another
chat transcript. They already have terminals, diff panes, and task/session
models, so the technical moat is not impossible. The defensible window for
swimmers is to ship a small, reliable implementation first and own the phrase
"thought rail for local tmux agents" before desktop competitors prioritize it.

## tmux Trajectory

The best current read is bifurcation:

- **Concrete tmux trajectory data point:** Agent Deck moved from roughly 2,000
  stars in the April snapshot to 2,585 stars on June 1, 2026, while Agent of
  Empires moved from roughly 1,600 to 2,480 over the same window; both stayed
  tmux-native. [14][20][44]
- **Toward tmux:** Agent Deck and Agent of Empires are both explicitly
  tmux-native AI-agent tools. From the April 2026 snapshot to June 1, 2026,
  Agent Deck grew from roughly 2,000 to 2,585 GitHub stars, and Agent of
  Empires grew from roughly 1,600 to 2,480. That is about 29% and 55% growth
  while keeping tmux as the session primitive. [14][20][44]
- **Away from tmux in desktop apps:** Superset and Emdash instead center
  desktop app UI, PTY/terminal panes, git worktrees, SSH, and integrations.
  They grew from roughly 9,500 to 11,482 and roughly 3,800 to 4,718 stars
  respectively from the April snapshot to June 1, 2026. That suggests many
  users will accept a desktop runtime if it gives richer review and remote
  workflows. [1][8][44]
- **General terminal alternatives have mindshare:** Zellij has 33,191 GitHub
  stars, 12,058 Homebrew installs in 30 days, and 26,238 crates.io recent
  downloads; Warp's open-source repo has 60,792 GitHub stars. These are not
  evidence that agent orchestration has moved to Zellij or Warp, but they are
  evidence that terminal UI expectations are broader than tmux alone. [37][38][39][42]

The strongest counterarguments to tmux as the only primitive are:

- Desktop and web UIs can show diffs, PRs, CI, issue metadata, plan panels,
  and terminal panes in one screen more easily than a terminal TUI.
- Remote/mobile workflows are now first-class in Emdash, Agent Deck, and Agent
  of Empires. Plain tmux over SSH is reliable, but browser/PWA/phone workflows
  are easier to demo and easier for casual users to adopt. [9][15][21]
- Isolation now means git worktrees plus optional containers. Tmux preserves
  sessions, but it does not itself solve branch isolation, dependency safety,
  or agent-provider configuration.

For swimmers, tmux is still an asset if the promise is "zero infrastructure,
works with your existing local sessions." It becomes a liability if swimmers
tries to compete directly with desktop PR/diff/review suites. The correct
orthogonal lane is peripheral awareness: show state and reasoning without
becoming the primary IDE.

## Distribution Playbook for Rust CLIs - Mid-2026

### Ranked channels by expected reach

1. **Hacker News Show HN / Launch HN.** Best first channel for a technical,
   open-source terminal tool. Evidence: Superset had 96 and 107 point HN posts;
   Emdash had a 206 point Show HN. These are the clearest public attention
   spikes among named competitors. [4][5][11]
2. **Targeted Reddit communities.** Best second wave after a demo exists.
   Evidence: Agent Deck had visible r/ClaudeCode and r/commandline posts about
   managing many AI-agent terminal/tmux sessions; Agent of Empires had visible
   r/codex and r/ClaudeAI posts about tmux, worktrees, Docker sandboxes, and
   session management. Use r/ClaudeCode, r/codex, r/commandline, and r/rust
   only when the post is specific and demo-backed. [18][19][23][24]
3. **Product Hunt.** Useful for polished visual demos and screenshots, less
   naturally aligned with a raw CLI. Superset and Emdash both have official
   Product Hunt surfaces. Use it after the HN/Reddit artifact is polished,
   not as the first proof channel. [7][13]
4. **Homebrew and GitHub Releases.** Important conversion surfaces after a
   discovery event. Agent of Empires has 783 Homebrew installs in 30 days and
   4,556 asset downloads across the latest 30 GitHub releases; Sesh has 970
   Homebrew installs in 30 days and 33,196 downloads across the latest 30
   GitHub releases. [25][29][40][43]
5. **crates.io and Rust newsletters.** Use crates.io as an install and trust
   signal, not the main discovery engine. Swimmers has 39 total crate downloads
   and 1 download in the last 7 days; established adjacent Rust CLIs show far
   more pull, but they have years of brand and package-manager presence.
   Submit to This Week in Rust, Rustacean Station, Console.dev, and terminal
   tooling newsletters after the first public launch, but do not expect them
   to substitute for HN or Reddit. [34][35][36]

### Star-growth benchmarks

- The April-to-June snapshot gives a practical two-month benchmark rather than
  exact launch-day curves: Superset added about 1,982 stars, Emdash about 918,
  Agent Deck about 585, Agent of Empires about 880, Sesh about 252, and tmuxp
  about 13. [1][8][14][20][26][30][44]
- A front-page HN hit can plausibly put a tool on a path toward hundreds of
  stars quickly; a low-engagement HN post does not. Agent Deck's HN post had
  only 3 points, so its later growth appears more tied to product surface,
  Reddit/community, Discord, and frequent releases than to HN alone. [17]
- For a new Rust CLI with no existing audience, a realistic first public
  objective is 100-300 GitHub stars after a credible HN plus targeted Reddit
  launch. A 500-star target is realistic only if the demo is immediately
  legible and one of HN, Reddit, or Product Hunt breaks through.

### Recommended first distribution action

Prepare one concrete launch artifact, then post to HN first:

`Show HN: Swimmers - see which local AI coding agents need attention from tmux`

The artifact should be a 45-60 second asciinema/GIF showing 8-12 tmux sessions
where fish state changes as agents become active, idle, errored, or waiting,
with the thought rail visible for one Claude Code and one Codex session. The
first comment should include the exact install path (`cargo install swimmers`),
GitHub repo, supported agents, a short privacy note about local parsing of
terminal output, and a limitation note that swimmers is awareness tooling, not
a PR/diff desktop environment.

Rationale: HN has the clearest public evidence for this category, but swimmers
needs the visual proof to be understood in under 10 seconds. The second wave
should reuse the same artifact in r/ClaudeCode, r/codex, r/commandline, and
r/rust, with different titles that match each community's vocabulary. The
third wave should package Homebrew/GitHub Release binaries before Product Hunt.

## Pricing and Business Models

- **Superset:** Source-available under Elastic License 2.0 per README; GitHub
  API reports `NOASSERTION`. YC company profile exists. No single-user paid
  tier was found in the README. [1][2][6]
- **Emdash:** Apache-2.0 open source, YC Winter 2026. No paid tier found in
  README; local storage is emphasized, with remote SSH and integrations. [8][9][12]
- **Agent Deck, Agent of Empires, Sesh, tmuxp:** MIT/free projects. No paid
  single-user tier found in the current README/release surfaces. [14][20][26][30]

The practical conclusion is that swimmers should not launch with monetization
as the headline. The credible monetization paths are downstream: team/shared
dashboard, hosted relay for remote state, commercial support, or paid packaged
desktop companion. The core single-binary tmux visualization should remain
free and frictionless until it has a user base.

## References

[1] GitHub API, `superset-sh/superset`, captured 2026-06-01: https://api.github.com/repos/superset-sh/superset
[2] Superset README: https://github.com/superset-sh/superset
[3] Superset desktop v1.12.1 release notes: https://github.com/superset-sh/superset/releases/tag/desktop-v1.12.1
[4] HN item 48236770, "Launch HN: Superset (YC P26) - IDE for the agents era": https://news.ycombinator.com/item?id=48236770
[5] HN item 46368739, "Show HN: Superset - Terminal to run 10 parallel coding agents": https://news.ycombinator.com/item?id=46368739
[6] YC Superset company profile: https://www.ycombinator.com/companies/superset
[7] Product Hunt Superset product page: https://www.producthunt.com/products/superset-3

[8] GitHub API, `generalaction/emdash`, captured 2026-06-01: https://api.github.com/repos/generalaction/emdash
[9] Emdash README: https://github.com/generalaction/emdash
[10] Emdash v1.1.27 release notes: https://github.com/generalaction/emdash/releases/tag/v1.1.27
[11] HN item 47140322, "Show HN: Emdash - Open-source agentic development environment": https://news.ycombinator.com/item?id=47140322
[12] YC Emdash company profile: https://www.ycombinator.com/companies/emdash
[13] Product Hunt Emdash product page: https://www.producthunt.com/products/emdash-2

[14] GitHub API, `asheshgoplani/agent-deck`, captured 2026-06-01: https://api.github.com/repos/asheshgoplani/agent-deck
[15] Agent Deck README: https://github.com/asheshgoplani/agent-deck
[16] Agent Deck v1.9.45 release notes: https://github.com/asheshgoplani/agent-deck/releases/tag/v1.9.45
[17] HN item 46276905, "Show HN: Agent Deck - Terminal Dashboard to Manage Claude/Gemini/Codex Sessions": https://news.ycombinator.com/item?id=46276905
[18] Reddit r/ClaudeCode Agent Deck post: https://www.reddit.com/r/ClaudeCode/comments/1pxyn37/i_got_tired_of_managing_15_terminal_tabs_for_my/
[19] Reddit r/commandline Agent Deck post: https://www.reddit.com/r/commandline/comments/1pn4e84/built_a_tmuxbased_dashboard_to_manage_multiple_ai/

[20] GitHub API, `agent-of-empires/agent-of-empires`, captured 2026-06-01: https://api.github.com/repos/agent-of-empires/agent-of-empires
[21] Agent of Empires README: https://github.com/agent-of-empires/agent-of-empires
[22] Agent of Empires v1.9.5 release notes: https://github.com/agent-of-empires/agent-of-empires/releases/tag/v1.9.5
[23] Reddit r/codex Agent of Empires post: https://www.reddit.com/r/codex/comments/1qkrj1z/agentofempires_codex_session_manager_with_builtin/
[24] Reddit r/ClaudeAI Agent of Empires post: https://www.reddit.com/r/ClaudeAI/comments/1qdkjh7/i_built_a_tool_that_automanages_git_worktrees_and/
[25] Homebrew formula API for `aoe`, captured 2026-06-01: https://formulae.brew.sh/api/formula/aoe.json

[26] GitHub API, `joshmedeski/sesh`, captured 2026-06-01: https://api.github.com/repos/joshmedeski/sesh
[27] Sesh README: https://github.com/joshmedeski/sesh
[28] Sesh v2.26.2 release notes: https://github.com/joshmedeski/sesh/releases/tag/v2.26.2
[29] Homebrew formula API for `sesh`, captured 2026-06-01: https://formulae.brew.sh/api/formula/sesh.json

[30] GitHub API, `tmux-python/tmuxp`, captured 2026-06-01: https://api.github.com/repos/tmux-python/tmuxp
[31] tmuxp README: https://github.com/tmux-python/tmuxp
[32] tmuxp v1.70.0 release notes: https://github.com/tmux-python/tmuxp/releases/tag/v1.70.0
[33] Homebrew formula API for `tmuxp`, captured 2026-06-01: https://formulae.brew.sh/api/formula/tmuxp.json

[34] crates.io API for `swimmers`, captured 2026-06-01: https://crates.io/api/v1/crates/swimmers
[35] crates.io downloads API for `zellij`, captured 2026-06-01: https://crates.io/api/v1/crates/zellij/downloads
[36] crates.io downloads API for `bacon`, captured 2026-06-01: https://crates.io/api/v1/crates/bacon/downloads
[37] crates.io API for `zellij`, captured 2026-06-01: https://crates.io/api/v1/crates/zellij
[38] Homebrew formula API for `zellij`, captured 2026-06-01: https://formulae.brew.sh/api/formula/zellij.json
[39] GitHub API, `zellij-org/zellij`, captured 2026-06-01: https://api.github.com/repos/zellij-org/zellij
[40] GitHub releases API for `joshmedeski/sesh`, captured 2026-06-01: https://api.github.com/repos/joshmedeski/sesh/releases?per_page=30
[41] GitHub releases API for `tmux-python/tmuxp`, captured 2026-06-01: https://api.github.com/repos/tmux-python/tmuxp/releases?per_page=30
[42] GitHub API, `warpdotdev/warp`, captured 2026-06-01: https://api.github.com/repos/warpdotdev/warp
[43] GitHub releases API for `agent-of-empires/agent-of-empires`, captured 2026-06-01: https://api.github.com/repos/agent-of-empires/agent-of-empires/releases?per_page=30
[44] Swimmers April competitor snapshot: ../VISION.md
