# OSS Launch Playbook for Swimmers - 2026-06-01

This report answers Bead `swimmers-deep-research-gtm-oss-launch-playbook-3p7`.
It complements the Round-1 competitive landscape report by focusing on organic
launch mechanics: Show HN framing, community channels, search gaps, crates.io
packaging, and the first three GTM moves for a Rust/tmux/agent-tooling project.

Scope date: June 1, 2026. Current GitHub and crates.io values are live captures
from official APIs on June 1, 2026 unless marked otherwise. Historical star
milestones come from GitHub's stargazers endpoint with star timestamps where
available; inaccessible values are marked `not-found` or `not-checked`.

## Executive Recommendation

Build the animated demo first, then launch on HN.

The strongest observed pattern is not "agent orchestration" alone. It is a
concrete before/after hook plus a visual proof that the tool is real in the
first screen. Successful terminal-tool HN posts had simple titles that explain
the job directly: "made me faster at Git" for lazygit, "Rust-based terminal" for
Warp, "interactive tree command written in Rust" for lstr, and "lazygit-style
TUI for SQL databases" for Sqlit. The weak agent-tool HN posts had category
labels but did not make the pain visually legible fast enough.

Recommended first launch artifact:

> A 45-60 second GIF/asciinema showing 8-12 tmux sessions as fish, with one
> active, one waiting, one errored, one idle, and the thought rail appearing for
> Claude Code/Codex sessions. The GIF must answer "which agent needs me?" before
> the viewer reads the README.

Recommended first title:

> Show HN: Swimmers - an aquarium for monitoring tmux sessions

Use the agent hook in the first paragraph and demo, not as the whole title. The
fish/aquarium metaphor is distinctive enough to get curiosity; the subtitle and
OP should immediately anchor it to "local AI coding agents in tmux."

## Section A - Comp Launch Teardowns

### Tool: Zellij

- GitHub: https://github.com/zellij-org/zellij
- Stars today: 33,193 via GitHub API. [1]
- Launch event: Reddit r/rust beta announcement and official beta post,
  April 20, 2021. [2][3]
- GitHub stars at launch event: 500th star on 2021-04-20, the same date as the
  beta announcement. [5]
- Title/hook: "Zellij: a Rusty terminal workspace releases a beta." [2]
- Demo artifact: Official beta blog linked from the Reddit post; current
  project home still leads with a visual terminal workspace and one-line install.
  [3][4]
- Star timeline: first star on 2020-11-01; 100th on 2020-11-02; 500th on
  2021-04-20; 1,000th on 2021-04-22; 2,000th on 2021-04-24. [5]
- crates.io before/after: crate created 2021-02-10, before the April 2021 beta
  spike. [6]
- What worked:
  - The hook was a category claim, not a feature list: "Rusty terminal workspace"
    and "tmux alternative" were immediately understandable.
  - The beta announcement coincided with a visible 500 to 2,000 star run in four
    days. [5]
  - Repeated later Reddit releases kept giving concrete feature deltas, for
    example stacked panes and native Windows support. [7][8]
- What to borrow for swimmers: Launch a named concept, not only a feature list:
  "aquarium for tmux sessions" is easier to remember than "agent orchestration
  terminal dashboard."
- Confidence: high for current stars, event title, crate timing, and GitHub
  milestone timestamps; medium for causal attribution because Reddit and the
  official blog were observed, but full referral analytics are not public.

### Tool: lazygit

- GitHub: https://github.com/jesseduffield/lazygit
- Stars today: 78,721 via GitHub API. [9]
- Launch event: HN Show HN, August 5, 2018. [10]
- GitHub stars at launch event: not-checked; the HN event itself is the primary
  measurable spike candidate in this pass. [10]
- Title/hook: "Show HN: I made a tool that made me faster at Git." [10]
- Demo artifact: GitHub repo link in the HN post; HN commenters discussed the
  videos/demos and compared the product to Magit. [10]
- Star timeline: exact 0 to 100 and 0 to 500 timestamps were not-checked because
  the GitHub stargazers API rate limit was reached during this pass. The HN
  thread itself is a specific star-spike candidate: 551 points and 229 comments.
  [10]
- crates.io before/after: not applicable; Go project.
- What worked:
  - The title used a personal outcome, "made me faster at Git," instead of a
    category label.
  - The product made a painful existing workflow concrete: fewer direct git
    commands, faster staging, commit, stash, and diff work.
  - The thread generated comparison comments to Magit and command-line git,
    which is useful because comparison debates keep HN threads alive. [10]
- What to borrow for swimmers: Make the outcome title about the operator's
  pain, not the implementation: "see which tmux session needs attention" beats
  "Rust TUI for agent orchestration."
- Confidence: high for HN engagement and title; low for exact star timeline
  because historical milestone timestamps were not-checked.

### Tool: gitui

- GitHub: https://github.com/gitui-org/gitui
- Stars today: 22,025 via GitHub API. [11]
- Launch event: early GitHub/release adoption in May 2020, followed by a small
  HN Show HN for v0.8 on July 6, 2020. [12][13]
- GitHub stars at launch event: 100th star on 2020-05-14 and 500th on
  2020-05-25 for the early adoption run; July 2020 HN-day count not-checked.
  [14]
- Title/hook: README hook "Blazing fast terminal-ui for git written in Rust";
  HN title "Show HN: GitUI v0.8 - fast terminal client for Git." [12][13]
- Demo artifact: README/release screenshots and benchmark framing against
  lazygit/tig on the Linux repository. [12]
- Star timeline: first star on 2020-03-19; 100th on 2020-05-14; 250th on
  2020-05-18; 500th on 2020-05-25. Later milestone fetches were blocked by the
  GitHub API rate limit. [14]
- crates.io before/after: crate created 2020-03-24, before the 100 to 500 star
  May 2020 run. [15]
- What worked:
  - The hook used a known category, terminal git UI, and a speed claim.
  - The README made the competitive frame explicit: lazygit, tig, GitUp, Magit
    adjacent workflows. [12]
  - The July HN post had only 2 points, so HN was not the launch driver. GitHub,
    package managers, releases, and sustained README clarity appear to have
    mattered more. [13]
- What to borrow for swimmers: Put the comparison in the README, not the HN
  title: "like `tmux ls`, but visual and state-aware" can convert visitors after
  the hook lands.
- Confidence: high for GitHub/crates milestones and the weak HN result; medium
  for attribution to GitHub/release adoption because the exact first external
  post was not found.

### Tool: lstr

- GitHub: https://github.com/bgreenwell/lstr
- Stars today: 1,522 via GitHub API. [16]
- Launch event: HN Show HN, June 18, 2025. [17]
- GitHub stars at launch event: not-checked; current stars exceed 1,500 after
  the HN event, but exact HN-day count was not fetched. [16][17]
- Title/hook: "Show HN: Lstr - A modern, interactive tree command written in
  Rust." [17]
- Demo artifact: The OP included a direct animated demo link and GitHub plus
  crates.io links. [17]
- Star timeline: exact historical milestones were not-checked after API rate
  limits. Current stars exceed 1,500 less than one year after the HN event. [16]
- crates.io before/after: crate created 2025-06-07, before the HN post. [18]
- What worked:
  - The title remixed a familiar Unix primitive, `tree`, with "interactive" and
    "written in Rust."
  - The OP led with a personal itch, then showed concrete features and an
    install surface.
  - Engagement was healthy for a small tool: 227 points and 66 comments. [17]
- What to borrow for swimmers: Reference the old primitive directly. "tmux ls"
  is swimmers' `tree`.
- Confidence: high for HN event, title, current stars, and crate timing; low
  for exact star milestone timing.

## Section B - Show HN Playbook for Swimmers

### Ranked Candidate Titles

1. `Show HN: Swimmers - an aquarium for monitoring tmux sessions`
   - Best fit. Distinctive, visual, short, and grounded in tmux. The agent use
     case can be the first sentence.
2. `Show HN: Swimmers - see which local AI coding agents need attention`
   - Strong ICP fit. Slightly less self-explanatory for HN readers who are not
     already running many agents.
3. `Show HN: Swimmers - animated fish for your tmux and Claude Code sessions`
   - Most memorable, but risks sounding novelty-first unless the demo is strong.

### What To Show

The OP should contain:

- A GIF/asciinema in the first GitHub README viewport and linked from the first
  HN comment.
- First paragraph: "I built Swimmers because `tmux ls` does not tell me which
  of my local coding agents is working, idle, errored, or waiting for input.
  Swimmers turns each tmux session into a fish whose motion reflects state, and
  shows a thought rail for Claude Code/Codex sessions."
- Install line: `cargo install swimmers && swimmers-tui`.
- Privacy/locality note: local tmux, local files, no hosted service required.
- Limitation note: tmux-only, not a PR/diff IDE, not a session templater.

Evidence basis:

- High-scoring terminal HN posts had simple, direct hooks: lazygit at 551
  points, Warp at 946, fd at 456, lstr at 227, Sqlit at 190, Ferrite at 241,
  and Fresh at 187. [10][17][19][20][21][22][23]
- Low-scoring agent-session HN examples had accurate category titles but weak
  immediate differentiation: Agent Deck at 3 points, Metateam at 3, Lazyagent
  at 4. [24][25][26]

### Timing Recommendation

Evidence is mixed, so treat this as medium confidence. In the sampled successful
posts, high-scoring items often landed during US/EU waking hours: Warp at
16:40 UTC on Tuesday, Emdash at 18:00 UTC on Tuesday, Superset at 19:52 UTC on
Tuesday, Sqlit at 15:47 UTC on Monday, and Fresh at 14:45 UTC on Wednesday.
There are counterexamples, including lazygit on a Sunday and lstr/Ferrite at
roughly 02:00 UTC. [10][17][20][21][22][23][27][28]

Recommendation: post Tuesday or Wednesday between 14:00 and 18:00 UTC, only
after the GIF and README first screen are ready. Do not delay for an ideal slot
if the artifact is already excellent.

### Failure Modes

1. Leading with "agent orchestration" jargon before proving the visual loop.
   The low-scoring agent HN posts show that "one terminal UI for agents" is not
   enough by itself. [24][25][26]
2. Making the fish metaphor look decorative. The demo must show state changes:
   active, waiting, errored, idle, thought rail.
3. Overclaiming around AI thoughts. Call it a local thought rail or transcript
   rail and explain what it parses; do not imply private model internals that
   swimmers cannot access.

## Section C - Channel Priority Stack

### Channel: Hacker News Show HN

- Evidence for this category: lazygit 551 points/229 comments; Warp 946/726;
  fd 456/215; lstr 227/66; Sqlit 190/42; Fresh 187/150; Emdash 206/71. [10][17][19][20][21][22][23][27]
- Fit for swimmers ICP: high. HN rewards technical, local-first developer tools
  when the title names a clear job and the demo proves it.
- First post recommendation: Use the aquarium/tmux title, include GIF and a
  first comment with install, limitations, and "why not just tmux?".
- Estimated effort: 4-6 hours after product is demo-ready.

### Channel: Reddit r/ClaudeCode and r/ClaudeAI

- Evidence for this category: Agent Deck's r/ClaudeCode post about managing
  15 terminal tabs for Claude sessions had 307 upvotes; cmux's r/ClaudeCode
  launch had 148; agent-view had 76 in r/ClaudeCode and 103 in r/ClaudeAI. [29][30][31][32]
- Fit for swimmers ICP: high. These communities already describe the exact pain:
  too many sessions, thinking vs waiting, needing an at-a-glance view.
- First post recommendation: Lead with the problem, not the brand: "I kept
  losing track of which tmux Claude session needed input, so I turned sessions
  into an aquarium."
- Estimated effort: 2-3 hours to adapt the HN GIF and write a community-native
  post.

### Channel: Reddit r/codex and r/tmux

- Evidence for this category: A tmux plugin for Codex/Claude status had visible
  r/codex traction; r/tmux posts show explicit demand for agent visual feedback
  and finish/fail/attention signals. [33][34][35]
- Fit for swimmers ICP: medium-high. Smaller reach than r/ClaudeCode, but the
  vocabulary is closer to tmux state and Codex status.
- First post recommendation: Post a technical walkthrough of the state detector
  and thought rail, with the same GIF plus a short config/install snippet.
- Estimated effort: 2 hours.

### Channel: r/rust

- Evidence for this category: Zellij release posts repeatedly performed well in
  r/rust, including 265 upvotes for 0.35.1 and 149 for 0.44. [7][8]
- Fit for swimmers ICP: medium. Rust identity helps install trust, but r/rust is
  less specifically about AI-agent operations.
- First post recommendation: Wait until after HN or the first release, then post
  as a Rust TUI implementation note with architecture details and crate link.
- Estimated effort: 2-4 hours.

### Channel: X/Twitter and short video

- Evidence for this category: direct engagement numbers were not-checked in
  this pass because X increasingly hides data without a logged-in session.
  However, visual terminal tools are GIF-friendly, and the same artifact can be
  reused by maintainers and Rust/terminal accounts without changing the launch
  sequence.
- Fit for swimmers ICP: medium. Useful for reach and influencer reposts, weaker
  as a first proof channel than HN/Reddit because public metrics are harder to
  verify.
- First post recommendation: A 20-second looping GIF with one sentence: "I made
  tmux sessions into fish so I can see which local coding agent needs me."
- Estimated effort: 1 hour after GIF exists.

## Section D - SEO / Search Query Table

| Query string | Where observed | Top result today | Gap for swimmers |
| --- | --- | --- | --- |
| `manage multiple Claude Code sessions tmux` | Reddit titles and web search around Claude/tmux sessions. [29][30][31] | llmux, cmux, dmux, and guide pages appeared in current search results. [36] | Swimmers needs a docs page that says "manage multiple Claude Code sessions in tmux" verbatim. |
| `tmux AI agent status across sessions` | r/codex and r/tmux posts about status plugins. [33][34] | tmux-agent-status Reddit/GitHub surfaces appear for this wording. [33] | Swimmers should target "status across sessions" with a feature page, not only "thought rail." |
| `terminal UI coding agents tmux` | HN low-score titles and Reddit agent tooling posts. [24][25][26][29] | Agentmux/cmux/dmux-style pages appear in search. [36] | Swimmers can own "terminal UI for tmux sessions" if the README title includes it. |
| `run many Claude Codex Gemini CLI instances one terminal UI` | HN Metateam title. [25] | Metateam/Lazyagent style HN pages and agent dashboards. [25][26] | This query is crowded and generic; use only in secondary copy. |
| `which Claude Code session is waiting for input` | Repeated Reddit pain language: thinking vs waiting, needs attention. [29][30][31] | Current search returns session managers and guide posts, not a dominant exact match. [36] | Strong gap. Swimmers should use "waiting for input" in README and docs headings. |
| `tmux sessions visual overview` | Swimmers wedge vs `tmux ls`; Reddit posts ask for visual overview. [31][37] | General tmux/session manager results and visual dashboards. [36] | Strong fit for aquarium framing and screenshots. |
| `cargo install tmux session manager rust` | crates.io/Rust install path plus Zellij/gitui/lstr patterns. [6][15][18] | Rust terminal tools and crates pages. [6][15][18] | Add crates.io keywords and first paragraph around `tmux`, `session`, `tui`, `terminal`. |
| `thought stream AI coding agent terminal` | Swimmers positioning term; not strongly observed in community titles. | not-found as a clear organic query. | Avoid making "thought stream" the SEO lead. Explain it after state/waiting language. |

## Section E - crates.io README Brief

Concrete recommendations:

- First paragraph: lead with `tmux`, `sessions`, `terminal`, `TUI`, and
  "AI coding agents." Avoid a whimsical first sentence without search nouns.
- GIF placement: put the GIF or asciinema preview immediately after the first
  paragraph and before long feature tables. HN/crates visitors need visual proof
  before they parse fish state language.
- Install command placement: first screen, before prerequisites. Use
  `cargo install swimmers` and `swimmers-tui`.
- Keywords: `tmux`, `tui`, `terminal`, `session-manager`, `ai-agents` if the
  crate metadata can fit them. Secondary terms in README copy: `Claude Code`,
  `Codex`, `agent status`, `waiting for input`.
- Badges: keep license/Rust badges, but do not let badges push the hook and GIF
  below the fold.

Draft first paragraph:

> Swimmers is a Rust TUI for monitoring many local tmux sessions, especially
> sessions running AI coding agents such as Claude Code, Codex, Gemini CLI, and
> Aider. It turns each session into an animated fish whose motion shows whether
> the session is active, idle, errored, or waiting for input, so you can scan a
> terminal fleet without cycling through panes or reading `tmux ls`. Run
> `cargo install swimmers`, start `swimmers-tui`, and your existing tmux sessions
> become a local aquarium with a thought rail for agent context.

Evidence basis:

- Zellij, gitui, bottom, lstr, and swimmers all use crates.io as a conversion
  surface; current crate pages expose total/recent downloads and README content.
  [6][15][18][38][39][40]
- The strongest HN posts for terminal tools used immediately legible category
  nouns and outcome nouns, then linked GitHub/crates/install surfaces. [10][17][19][20][21]

GIF/download caveat: no primary source found proving that a crates.io README GIF
materially increases installs. Treat GIF placement as conversion common sense
from HN/README behavior, not as measured crates.io analytics.

## Section F - Prioritized 3-Move Launch Sequence

### Move 1: Ship the proof GIF and README first screen

- Artifact: 45-60 second GIF/asciinema plus a README first screen containing
  hook, GIF, install, local/privacy note, and limitations.
- Platform: GitHub README and crates.io README.
- Target metric: A new visitor can explain swimmers in one sentence without
  scrolling; at least 5 friendly dev-tool users can install from the README.
- Evidence basis: successful posts such as lstr and Sqlit included visual/demo
  proof and direct install surfaces; weak agent posts show category language
  alone is not enough. [17][21][24][25][26]

### Move 2: Show HN

- Artifact: HN post using the top title plus first comment with install,
  limitations, and "why this exists."
- Platform: Hacker News Show HN.
- Target metric: 100+ HN points, 25+ comments, 100+ GitHub stars in the first
  week, or clear qualitative feedback about install blockers.
- Evidence basis: HN is the clearest public spike channel for comparable
  terminal/dev tools: lazygit 551, Warp 946, fd 456, lstr 227, Sqlit 190,
  Emdash 206. [10][17][19][20][21][27]

### Move 3: Same GIF, narrower Reddit posts

- Artifact: two community-native posts:
  - r/ClaudeCode/r/ClaudeAI: "I kept losing track of which Claude tmux session
    needed input, so I made an aquarium."
  - r/codex/r/tmux: "tmux session status and thought rail for Codex/Claude
    sessions."
- Platform: Reddit r/ClaudeCode, r/ClaudeAI, r/codex, and r/tmux.
- Target metric: 50+ upvotes or 10+ substantive comments in one AI-agent
  community, plus at least 3 issue/feature requests from real users.
- Evidence basis: r/ClaudeCode and r/ClaudeAI already reward this exact pain
  language: Agent Deck 307, cmux 148, agent-view 76/103. [29][30][31][32]

## Residual Evidence Gaps

- Exact star timelines for lazygit and lstr were not-checked because the GitHub
  stargazers API rate limit was reached during this pass.
- Direct X/Twitter launch metrics were not-checked because primary public access
  is unreliable without logged-in scraping.
- No primary crates.io analytics source was found that isolates the effect of a
  README GIF on installs.
- GitHub star causality is inferred unless the platform event and milestone
  timestamps line up tightly, as with Zellij's April 2021 beta run.

## References

[1] GitHub API, `zellij-org/zellij`, captured 2026-06-01: https://api.github.com/repos/zellij-org/zellij
[2] Reddit r/rust, "Zellij: a Rusty terminal workspace releases a beta": https://www.reddit.com/r/rust/comments/mupycg
[3] Zellij beta announcement: https://zellij.dev/news/beta/
[4] Zellij official site: https://zellij.dev/
[5] GitHub Stargazers API, `zellij-org/zellij`, captured 2026-06-01 with `Accept: application/vnd.github.star+json`: https://api.github.com/repos/zellij-org/zellij/stargazers
[6] crates.io API for `zellij`, captured 2026-06-01: https://crates.io/api/v1/crates/zellij

[7] Reddit r/rust, "Zellij 0.35.1 brings stacked panes to your terminal": https://www.reddit.com/r/rust/comments/11kw3b9
[8] Reddit r/rust, "Zellij 0.44 released: native Windows support...": https://www.reddit.com/r/rust/comments/1s1cg2v/zellij_044_released_native_windows_support_new/

[9] GitHub API, `jesseduffield/lazygit`, captured 2026-06-01: https://api.github.com/repos/jesseduffield/lazygit
[10] HN item 17689014, "Show HN: I made a tool that made me faster at Git": https://news.ycombinator.com/item?id=17689014

[11] GitHub API, `gitui-org/gitui`, captured 2026-06-01: https://api.github.com/repos/gitui-org/gitui
[12] GitUI README: https://github.com/gitui-org/gitui
[13] HN item 23748231, "Show HN: GitUI v0.8 - fast terminal client for Git": https://news.ycombinator.com/item?id=23748231
[14] GitHub Stargazers API, `gitui-org/gitui`, captured 2026-06-01 with `Accept: application/vnd.github.star+json`: https://api.github.com/repos/gitui-org/gitui/stargazers
[15] crates.io API for `gitui`, captured 2026-06-01: https://crates.io/api/v1/crates/gitui

[16] GitHub API, `bgreenwell/lstr`, captured 2026-06-01: https://api.github.com/repos/bgreenwell/lstr
[17] HN item 44306041, "Show HN: Lstr - A modern, interactive tree command written in Rust": https://news.ycombinator.com/item?id=44306041
[18] crates.io API for `lstr`, captured 2026-06-01: https://crates.io/api/v1/crates/lstr

[19] HN item 30921231, "Show HN: Warp, a Rust-based terminal": https://news.ycombinator.com/item?id=30921231
[20] HN item 15429390, "Show HN: A simple, fast and user-friendly alternative to find, written in Rust": https://news.ycombinator.com/item?id=15429390
[21] HN item 46276002, "Show HN: Sqlit - A lazygit-style TUI for SQL databases": https://news.ycombinator.com/item?id=46276002
[22] HN item 46571980, "Show HN: Ferrite - Markdown editor in Rust with native Mermaid diagram rendering": https://news.ycombinator.com/item?id=46571980
[23] HN item 46135067, "Show HN: Fresh - A new terminal editor built in Rust": https://news.ycombinator.com/item?id=46135067
[24] HN item 46276905, "Show HN: Agent Deck - Terminal Dashboard to Manage Claude/Gemini/Codex Sessions": https://news.ycombinator.com/item?id=46276905
[25] HN item 47274120, "Show HN: Metateam: run many Claude/Codex/Gemini CLI instances in one terminal UI": https://news.ycombinator.com/item?id=47274120
[26] HN item 47349851, "Show HN: Lazyagent - One terminal UI for all your coding agents": https://news.ycombinator.com/item?id=47349851
[27] HN item 47140322, "Show HN: Emdash - Open-source agentic development environment": https://news.ycombinator.com/item?id=47140322
[28] HN item 46368739, "Show HN: Superset - Terminal to run 10 parallel coding agents": https://news.ycombinator.com/item?id=46368739

[29] Reddit r/ClaudeCode, Agent Deck launch post: https://www.reddit.com/r/ClaudeCode/comments/1pxyn37/i_got_tired_of_managing_15_terminal_tabs_for_my/
[30] Reddit r/ClaudeCode, cmux launch post: https://www.reddit.com/r/ClaudeCode/comments/1r43cdr/introducing_cmux_tmux_for_claude_code/
[31] Reddit r/ClaudeCode, agent-view launch post: https://www.reddit.com/r/ClaudeCode/comments/1r7jrmy/i_got_tired_of_managing_10_terminal_tabs_for_my/
[32] Reddit r/ClaudeAI, agent-view cross-post: https://www.reddit.com/r/ClaudeAI/comments/1rb4jvs/i_got_tired_of_managing_10_terminal_tabs_for_my/
[33] Reddit r/codex, tmux-agent-status post: https://www.reddit.com/r/codex/comments/1rozuul/tmux_plugin_to_track_codex_cli_status_across/
[34] Reddit r/tmux, agent finish/fail/attention status post: https://www.reddit.com/r/tmux/comments/1s9w3sy/i_just_wanted_to_know_when_my_agents_finish_fail/
[35] Reddit r/tmux, agent visual feedback post: https://www.reddit.com/r/tmux/comments/1r5b7kr/tmux_plugin_for_ai_agent_visual_feedback/
[36] Current web search results for `manage multiple Claude Code sessions tmux`, captured 2026-06-01.
[37] Reddit r/ClaudeCode, Kova/macOS visual tmux post: https://www.reddit.com/r/ClaudeCode/comments/1rw654c/i_built_a_macos_terminal_workspace_for_managing/

[38] crates.io API for `bottom`, captured 2026-06-01: https://crates.io/api/v1/crates/bottom
[39] crates.io API for `swimmers`, captured 2026-06-01: https://crates.io/api/v1/crates/swimmers
[40] crates.io API for `ferrite`, captured 2026-06-01: https://crates.io/api/v1/crates/ferrite
