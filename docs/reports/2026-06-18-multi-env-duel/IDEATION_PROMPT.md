You are one lane in a dueling-idea-wizards run for the Rust project at /Users/b/repos/opensource/swimmers.

Read the project docs and code before answering, especially AGENTS.md, README.md, docs/VISION.md, docs/TROGDOR_WEB.md, docs/reports/2026-06-01-agent-orchestration-landscape.md, src/session/overlay.rs, src/api/remote_sessions.rs, src/api/service/attention_group.rs, src/bin/swimmers_tui/picker.rs, src/bin/swimmers_tui/app.rs, and src/web/rendered_surface.js.

Do not edit source files. Output your answer only to stdout.

Goal:
The operator is reconsidering whether FrankenTerm/FrankenTUI is the right answer. They suspect swimmers is already close to a better solution: one Swimmers thing that can show all relevant local and SSH/devbox environments, sort/filter/switch by host, cwd, project, state, and readiness, launch work into the right environment, and incorporate the prior c0/group-by-pwd/remote-first devbox lessons without turning Swimmers into an arbitrary cluster scheduler.

Important prior context to account for:
- Swimmers already has local tmux discovery, TUI/web surfaces, thought rail, Trogdor repo atlas, remote API mode, overlay-declared launch targets, remote session namespacing, attention groups, directory picker filters, group input, and passive Skillbox/SBP skill discovery.
- docs/VISION.md says Swimmers is not a general multi-host control plane, but may aggregate explicitly configured swimmers_api launch targets for a single operator.
- Prior operator workflows had confusion across local Mac, skillbox@skillbox-portfolio-devbox, devbox/container roots, NTM sessions, c0 labeler runs, d3/d/devl/devbox launchers, remote repo roots, and grouped-by-pwd views.
- c0 is remote-first by operator preference; local mode should be explicit. c0 --group-by-pwd once is display/grouping-oriented and should not silently move/merge sessions.
- The relevant product target is not generic SSH infra. It is a configured operator environment cockpit for local + known SSH/Tailnet/Swimmers API targets, with safe fallbacks to SSH/tmux commands.

Come up with 20 pragmatic but ambitious ideas for making Swimmers indispensable for this multi-environment orchestration use case, then winnow to your best 5. For each top idea include:
1. Title.
2. User/audience served.
3. Why this clears the indispensability bar.
4. How it fits the current Swimmers architecture.
5. What code modules or API surfaces it likely touches.
6. Main risks or ways it could go wrong.
7. Concrete acceptance tests or smoke proofs.

Be candid. Prefer ideas that are accretive, testable, and coherent with the current code. Do not propose replacing Swimmers with FrankenTerm. If you think an idea should remain an external helper instead of moving into Swimmers, say so.
