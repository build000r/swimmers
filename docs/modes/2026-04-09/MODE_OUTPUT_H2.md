# Adversarial Review / Red-Team (H2) — Analysis of swimmers

## Thesis

An attacker-lens review of swimmers against its *actual* threat model (solo
dev, loopback-first, opt-in token mode for Tailscale) finds a fundamentally
sound posture: the LocalTrust loopback gate is conservative, the native
handoff uses `Command::arg` (execve, no shell) with `shell_single_quote` for
the tmux attach string, directory traversal is blocked by
`canonicalize + starts_with`, and the only truly dangerous primitive
(`POST /v1/sessions/{id}/input`) is load-bearing by design — operator scope
*is* "run arbitrary commands as the logged-in user," and that is the product.
What this mode surfaces that "does it work" modes miss is a set of
defense-in-depth gaps that cost little to close and matter the moment the
server leaves loopback: (1) WebSocket bearer tokens travel in the URL query
string and land in proxy logs / browser history; (2) token comparison is
plain `==` on `&str` and short-circuits on first mismatch; (3) graceful
shutdown has no drain timeout, letting a single stalled client hold the
process open; (4) `sanitize_osascript_text_arg` is applied inconsistently
across the osascript argv slots; (5) the loopback gate is a literal-string
match that silently rejects legitimate loopback addresses. None of these
reach "critical" in context. All are one-line-ish fixes the owner should
consider before pushing the Tailscale story in marketing.

## Top Findings

### §F1 — WebSocket auth token travels in `?token=` query string
- **Evidence:** `src/web/mod.rs:437-451` defines `WsQuery { token: Option<String> }`
  and `session_ws` extracts it via `Query<WsQuery>`, then calls
  `resolve_ws_auth(&state.config, query.token.as_deref())`
  (`src/web/mod.rs:718-744`). The HTTP `/v1/...` routes do use the
  `Authorization: Bearer` header (`src/auth/mod.rs:150-156`) — only the WS
  upgrade diverges.
- **Reasoning:** Query-string secrets are a red-team classic. They land in
  reverse-proxy access logs, uvicorn-style access logs, browser history,
  `Referer` on any cross-origin resource loaded after the upgrade, and shell
  history if the URL is ever curled. `wss://` protects the wire, not the
  log substrate. Today swimmers has no embedded proxy, but the README
  positions Tailscale as the supported remote story — Tailscale itself
  doesn't log HTTP, but a user fronting it with Caddy/nginx for TLS would
  immediately leak the bearer into `access.log`.
- **Severity:** medium (low on pure loopback, medium once `AUTH_MODE=token`
  is in use, because this is exactly the path that promised not to leak the
  secret).
- **Confidence:** 0.9
- **So What:** Accept the token via `Sec-WebSocket-Protocol: bearer, <token>`
  (standard hack) or a short-lived ticket minted by an authenticated HTTP
  call. One-file change in `web/mod.rs` plus a matching tweak in
  `src/bin/swimmers_tui/api.rs` if the TUI ever dials the WS. Until then,
  document that `AUTH_TOKEN` will appear in any HTTP log fronting the
  server.
- **Owner-ack?** no

### §F2 — Token comparison is not constant-time
- **Evidence:** `src/auth/mod.rs:108-122` compares with
  `provided == expected` (`&str == &str` — short-circuits on first
  mismatched byte). `src/web/mod.rs:730-735` does the same
  (`config.auth_token.as_deref() == Some(token)`).
- **Reasoning:** The canonical adversarial lens on any bearer-token
  compare. In practice, with a high-entropy 32-byte token and network
  jitter in the ≥100µs range (Tailscale over Wi-Fi is worse), you cannot
  meaningfully distinguish per-byte short-circuit timing. This is why
  severity stays *low*. But it costs one `use subtle::ConstantTimeEq` and
  `.ct_eq(...).into()` to remove the finding entirely, and the README is
  already inviting external scrutiny by publishing to crates.io.
- **Severity:** low (context-dependent; would be medium on a shared
  host/LAN).
- **Confidence:** 0.95
- **So What:** Add `subtle` as a dep; switch both sites to
  `a.as_bytes().ct_eq(b.as_bytes()).into()`. Two-line diff in each file.
- **Owner-ack?** no

### §F3 — Graceful shutdown has no drain timeout
- **Evidence:** `src/main.rs:250-253`:
  `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`.
  `shutdown_signal()` (`src/main.rs:258-287`) awaits ctrl_c/SIGTERM then
  returns, handing control back to axum, which waits indefinitely for
  in-flight connections to finish.
- **Reasoning:** Red-team angle: a single slow client (stalled WS, stuck
  `/snapshot` inside a 5s oneshot wait, or a dead TCP peer keeping a
  half-open connection) will block SIGTERM handling. The TUI client can
  itself be that slow client — it polls frequently. On a developer laptop
  this shows up as "why does `Ctrl-C` take 30 seconds to actually exit?"
  which trains the user to `kill -9`, which defeats the persistence
  checkpoint spawned at `supervisor.spawn_persistence_checkpoint()`
  (`src/main.rs:217`). That's the *actual* impact: lost replay/thought
  state on shutdown.
- **Severity:** low-medium (annoyance + data-loss risk, not a security
  break).
- **Confidence:** 0.85
- **So What:** Wrap the serve with
  `tokio::select! { _ = axum::serve(...) => {}, _ = tokio::time::sleep(Duration::from_secs(10)) after shutdown_signal() => {} }`
  pattern, or use `axum::serve(...).with_graceful_shutdown(async { shutdown_signal().await; tokio::time::sleep(Duration::from_secs(10)).await; })`.
  Ten lines in `main.rs`.
- **Owner-ack?** no (recent commits hardened the panic surface and added
  graceful shutdown, but didn't bound it)

### §F4 — `sanitize_osascript_text_arg` is applied inconsistently
- **Evidence:** `src/native/mod.rs:438-449` sanitizes only `session_id` and
  `display_name`:
  ```rust
  let safe_session_id = sanitize_osascript_text_arg(session_id);
  let safe_display_name = sanitize_osascript_text_arg(display_name);
  command.arg(script)
      .arg(&safe_session_id)
      .arg(tmux_name)              // <- raw
      .arg(attach_command)         // <- raw
      .arg(&safe_display_name);
  ```
  The Ghostty variant (`src/native/mod.rs:617-629`) also leaves `tmux_name`
  and `attach_command` unsanitized. Meanwhile the sanitizer's own docstring
  (`src/native/mod.rs:587-598`) states the goal is to keep values that
  round-trip back through `parse_osascript_output` from desyncing on the
  `|` separator or on `\n\r\t\0`. `tmux_name` is user-controlled via
  `POST /v1/sessions { name }` and normalized only with `.trim()`
  (`src/session/supervisor.rs:1534-1543`).
- **Reasoning:** Today the `.scpt` files (not read here, but referenced as
  opaque text-item consumers) don't echo `tmux_name` back through stdout,
  so there is no current parse desync. But the sanitizer exists *precisely*
  because "the script might print this back" is hard to keep in your head,
  and the asymmetric application invites a future regression: add one
  debug `log "tmux=<name>"` in the `.scpt` and `parse_osascript_output`
  (`src/native/mod.rs:653-677`) starts reporting `unexpected osascript
  status: ...` for any tmux name containing `|`. It's defense-in-depth that
  doesn't defend.
- **Severity:** low (no current exploit path; correctness hazard)
- **Confidence:** 0.85
- **So What:** Either (a) sanitize `tmux_name` and `attach_command` the
  same way, or (b) tighten `normalize_requested_tmux_name`
  (`src/session/supervisor.rs:1534`) to strip/reject `|\n\r\t\0` at the
  API boundary, and leave a comment pointing at the osascript invariant.
  Option (b) is better because it also protects every *other* tmux
  consumer downstream.
- **Owner-ack?** partial — the recent "osascript arg sanitization" commit
  landed the sanitizer; this is a gap in its application.

### §F5 — `is_loopback_bind` is a literal-string allowlist
- **Evidence:** `src/cli.rs:314-316`:
  ```rust
  pub fn is_loopback_bind(bind: &str) -> bool {
      matches!(bind, "127.0.0.1" | "::1" | "localhost")
  }
  ```
  Called by `enforce_localtrust_loopback` (`src/cli.rs:374-379`), which
  refuses to start if false.
- **Reasoning:** Red-team bypass check: can you get a non-loopback bind
  past this with `LocalTrust`? `0.0.0.0`, `::`, `127.0.0.2` (also
  loopback), `[::1]`, `LOCALHOST` (case), `127.0.0.1 ` with a stray space,
  IPv4-mapped `::ffff:127.0.0.1` — every one of these *fails* the match.
  From a bypass standpoint, that's good: conservative = safe-side. The
  cost is the inverse: a user who legitimately wants to bind to
  `127.0.0.2` (loopback alias, per-service binding on macOS/Linux)
  cannot, and the error message tells them to switch to token mode,
  which is *more* dangerous for a truly-loopback deployment. The gate
  enforces an opinion ("exactly 127.0.0.1/::1/localhost") stricter than
  the actual safety invariant ("inside 127.0.0.0/8 or [::1]").
- **Severity:** low (usability, not security)
- **Confidence:** 0.95
- **So What:** Parse `bind` as an `IpAddr` via `std::net::IpAddr::from_str`
  and use `ip.is_loopback()`; keep the `"localhost"` string-case as a
  special kindness. Five lines. Unit tests already exist alongside.
- **Owner-ack?** no

### §F6 — `env_bootstrap` sources provider keys from `$SHELL -ic` at startup
- **Evidence:** `src/env_bootstrap.rs:9-27,48-55`:
  ```rust
  let output = Command::new(shell).arg("-ic").arg(script).output().ok()?;
  ```
  runs the user's interactive shell with an interpolated capture script,
  then `env::set_var(key, value)` at `src/env_bootstrap.rs:24` stuffs the
  result into process env, where every subsequently-spawned child inherits
  it (session actors, tmux, daemon runners).
- **Reasoning:** Adversarial angle in context: "the user's own `.zshrc` is
  the attack surface" is true but uninteresting — if `.zshrc` is
  compromised, the attacker owns the user. What is interesting: (a) any
  process swimmers launches after this point inherits `OPENROUTER_API_KEY`
  / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`, which includes the PTY-backed
  user shell itself. That means a tmux session spawned through swimmers
  now has those keys in its environment *even if the user's normal shell
  would not*. A malicious process inside one of those tmux sessions can
  `printenv | curl` the keys out. (b) `shell -ic` blocks startup on
  whatever your `.zshrc` does (oh-my-zsh, nvm, mise) — this races the
  `STARTUP_PHASE_WARN_THRESHOLD = 2s` budget (`src/main.rs:42`). (c)
  `parse_marked_value` (`src/env_bootstrap.rs:57-72`) silently drops the
  value if a `.zshrc` prints anything between the start and end markers
  (verbose shells, `autoload -Uz ... ; echo "loaded ..."`).
- **Severity:** low-medium — (a) is the interesting bit for an adversarial
  lens, because the threat model includes "AI agents running inside the
  tmux panes" and those agents should *not* automatically receive your
  OpenRouter key.
- **Confidence:** 0.8
- **So What:** (1) Document explicitly in README/QUICKSTART that provider
  keys leak into every session spawned via swimmers. (2) Consider opt-in
  (`SWIMMERS_BOOTSTRAP_PROVIDER_ENV=1`) rather than always-on. (3) Strip
  the keys from the actor's child-env before spawning (`portable-pty`
  accepts an env map) and only pass them to the thought daemon that
  actually needs them.
- **Owner-ack?** no

### §F7 — `POST /v1/sessions/{id}/input` is "run arbitrary commands," by design
- **Evidence:** `src/api/sessions.rs:205-299`. Requires
  `AuthScope::SessionsWrite` (line 211), then dispatches the raw body text
  via `SessionCommand::WriteInput(body.text.into_bytes())` to the PTY.
  No escaping, no filtering, no rate limit, default axum 2MB body cap.
- **Reasoning:** The prompt explicitly asks: "what is actually in scope
  and what isn't?" The adversarial answer is: this endpoint IS the product
  surface — any holder of the operator token can drive any attached tmux
  session to arbitrary command execution as the running user. That's not
  a vulnerability; that's swimmers. What *isn't* in scope and would be a
  vulnerability: (a) reaching arbitrary execution without the token; (b)
  reaching a *different user's* sessions; (c) escaping the attached tmux
  into sessions swimmers did not create. None of (a)-(c) exist in the
  reviewed code. But README/`/version`/docs don't state the scope model
  clearly — the word "operator" appears in `src/auth/mod.rs:31` comments
  only. A reader who skims "token auth" may assume "token auth protects
  read access" and not "token holder == local shell."
- **Severity:** low (documentation gap, not a code defect)
- **Confidence:** 0.95
- **So What:** Add a single sentence to README's `AUTH_MODE=token`
  section: *"An operator token is equivalent to a shell on this machine.
  Treat it like SSH key material."* Cite it from the `config doctor`
  output that `cli.rs` already has (`src/cli.rs:258-263`).
- **Owner-ack?** no (implicit in the design; not written down)

### §F8 — Replay ring swallows a single oversized frame
- **Evidence:** `src/session/replay_ring.rs:37-49`:
  ```rust
  while self.total_bytes + data.len() > self.capacity && !self.frames.is_empty() {
      if let Some(evicted) = self.frames.pop_front() { ... }
  }
  self.total_bytes += data.len();
  self.frames.push_back(Frame { seq, data: data.to_vec() });
  ```
  Comment at line 38: "If a single frame exceeds capacity, we still store
  it (evicting everything else)."
- **Reasoning:** Adversarial angle: a client that sends one enormous write
  through `POST /.../input` (or a runaway program inside a PTY that
  produces a huge burst before a yield) can push a frame >
  `replay_buffer_size` (512KB default, `src/config.rs:64`). The ring
  accepts it, `total_bytes` now exceeds capacity, and the *next* push sees
  `total_bytes + data.len() > capacity && !frames.is_empty()`, evicts the
  oversized frame, and all replay history from the burst moment onward is
  effectively lost. The invariant "replay ≤ capacity" is violated
  transiently but the *information loss* is permanent. Not a DoS (memory
  is bounded to the peak frame size), but an availability dent on the
  replay feature.
- **Severity:** low
- **Confidence:** 0.8
- **So What:** Either (a) chunk oversized frames before `push` (natural
  for PTY bytes — they're not atomic), or (b) clamp capacity floor so a
  single frame can't exceed it. (a) is a ~15-line change in `actor.rs`
  where `replay_ring.push` is called.
- **Owner-ack?** no (owner-acked limits are the README platform/transport
  list, not this)

## Risks Identified

| Risk | Sev | Likelihood (in context) |
|---|---|---|
| WS token leaks through reverse-proxy logs once anyone fronts swimmers with Caddy/nginx | medium | medium |
| Token timing side-channel on shared LAN / Tailscale | low | very low |
| Slow-client stalls SIGTERM → `kill -9` → lost persistence checkpoint | low-medium | medium (happens to real users) |
| Provider API keys leak into every PTY session's env | low-medium | medium (silent, always-on) |
| Legitimate loopback alias rejected by literal-match gate | low | low (not an attack) |
| osascript arg sanitization drift if `.scpt` starts echoing tmux_name | low | low (regression hazard) |
| Oversized PTY burst permanently evicted from replay ring | low | low |
| Token == operator shell not stated in README | low (docs) | 1.0 (every new user) |

## Recommendations

Prioritized, filtered through "solo dev, loopback-first, keep it small":

- **P0 — [low effort, high benefit]** Document in README that
  `AUTH_TOKEN` holders get shell-equivalent access. (§F7)
- **P0 — [low effort, medium benefit]** Accept WS token via
  `Sec-WebSocket-Protocol` header or a ticket handshake; stop using the
  query string. (§F1)
- **P1 — [low effort, medium benefit]** Bound `with_graceful_shutdown`
  with a `tokio::time::sleep` drain timeout. (§F3)
- **P1 — [low effort, low benefit]** Switch token compare to
  `subtle::ConstantTimeEq`. (§F2)
- **P2 — [low effort, low benefit]** Replace literal-match
  `is_loopback_bind` with `IpAddr::is_loopback()`. (§F5)
- **P2 — [low effort, defense-in-depth]** Reject `|\n\r\t\0` in
  `normalize_requested_tmux_name` at the API boundary. (§F4)
- **P2 — [medium effort, medium benefit]** Make
  `bootstrap_provider_env_from_shell` opt-in and scrub provider keys from
  the child env passed to `portable-pty`. (§F6)
- **P3 — [low effort, low benefit]** Clamp or chunk oversized
  `replay_ring.push` frames. (§F8)
- **P4 — observational:** Add a `/version` assertion that reports
  `auth_mode` (masked) and `bind` so a user can quickly verify they're
  actually on loopback.

**Filtered out by Identity Check / project values:**
- "Add rate limiting to `/input`" — operator scope is *supposed* to be
  able to flood a PTY; a rate limit breaks legitimate paste-large-plan
  flows. Not recommended.
- "Move to a real IAM/JWT story" — violates no-infrastructure value.
- "Sandbox the PTY" — would break tmux identity.

## New Ideas and Extensions

- **[incremental] `swimmers config doctor` — add a "security posture"
  section.** The doctor already exists (`src/cli.rs:run_doctor_checks`).
  Teach it to emit: bind address, whether `AUTH_MODE=token`, whether WS
  token is still on query string, whether `OPENROUTER_API_KEY` is in the
  process env. Respects no-infra.
- **[incremental] Observer token → operator token upgrade flow.** Already
  have two scope levels (`src/auth/mod.rs:32-39`). A "read-only demo"
  mode for showing swimmers to a friend over Tailscale gets more valuable
  if you add a `POST /v1/auth/upgrade` that trades observer for operator
  with a typed-in PIN. Matches "aquarium is a demoable thing."
- **[incremental] Startup-time refuse-to-run if `HOME` is unset and
  `SWIMMERS_DATA_DIR` unset.** Currently falls back to `./data/swimmers/`
  (`src/main.rs:52-58`). An attacker-lens concern: a user who runs
  swimmers from `/tmp` with `HOME` unset now has persistence in a
  world-writable directory. Low likelihood, trivial fix.
- **[significant] Per-session scope tokens.** Mint a short-lived
  per-session token at create time and require it on `/input` and `/ws`.
  Limits blast radius if a token leaks. Needs a token table in the
  file_store; borderline against "no infrastructure."
- **[radical / filter-out] Seccomp/AppArmor wrapping of child PTYs.**
  Breaks "it's tmux" identity.

## Assumptions Ledger

**Unstated assumptions this analysis depends on:**
- The referenced `.scpt` files (not read here) treat each argv slot as
  opaque AppleScript text, as the docstring at
  `src/native/mod.rs:590-594` claims. Finding §F4 is sized on that being
  *currently* true.
- `axum 0.8` default body limit is 2 MB for JSON extractors. If the
  project has a global `DefaultBodyLimit` override, §F7's "2MB burst"
  number is wrong but the qualitative claim stands.
- `portable-pty` honors an explicit env map when one is supplied
  (assumption behind §F6's remediation).
- Tailscale is not front-proxied. If a user puts Caddy in front for
  HTTPS, §F1 becomes high-likelihood immediately.

**Assumptions the project itself makes that this mode questions:**
- "LocalTrust on loopback means same-machine actor already has user
  privileges, so no auth is fine." *Mostly* true, but it silently
  extends to "any process the user runs can drive any other tmux session
  the user is in" — which matters when the user has AI agents running.
  The aquarium metaphor encourages this; the threat model should name it.
- "Bootstrapping provider env from `$SHELL -ic` is harmless because it's
  the user's own shell." Ignores that the *consumers* of that env are
  PTY children who wouldn't otherwise see the keys.
- "Graceful shutdown finishes eventually." Doesn't in the presence of a
  stuck peer.

## Questions for the Project Owner

1. Is the WS endpoint meant to be used outside of the TUI/web clients,
   or is it an implementation detail? If implementation detail, can the
   TUI client just use HTTP + the `Authorization` header and let the WS
   die? That deletes §F1 entirely.
2. What's the actual story for fronting swimmers with TLS? If the answer
   is "Tailscale handles that, don't front it," §F1 is downgraded and
   should be documented; if the answer is "use Caddy," §F1 goes up.
3. Do you *want* provider keys to be available inside spawned PTY
   sessions (so the AI agent the user starts there can use them)? If
   yes, §F6 is "as designed" and should be documented. If no, §F6 needs
   a scrub.
4. Is there a scenario where a non-`127.0.0.1` loopback bind matters to
   you (e.g., macOS loopback alias per-project)? If not, §F5 stays as
   "don't bother."
5. The repo has `feature = "personal-workflows"` gating `dirs`/`skills`/
   `web_actions`/`host_actions`. Is the public crates.io build shipping
   *without* that feature? If yes, several endpoints I checked are not
   in the default attack surface and should be explicitly noted in
   security docs.

## Points of Uncertainty

- **Real exploitability of §F2 (timing compare)** — without a concrete
  experiment against the Tailscale data path, I cannot state a bit-rate
  for token recovery. The theoretical channel exists; practical
  exploitation in the deployment context is very doubtful.
- **§F4's blast radius** depends on `.scpt` contents not reviewed here.
  If the scripts never print argv back, the sanitization gap is purely a
  latent regression risk.
- **§F6's "keys leak into PTY env"** assumes `portable-pty` inherits
  parent env by default. Verified by convention, not by reading
  `portable-pty`'s source in this pass.
- **Graceful-shutdown stall (§F3)** — I have not reproduced the
  "Ctrl-C hangs" behavior empirically; the claim is reasoned from the
  code + axum's documented drain semantics.

## Agreements and Tensions with Other Modes

- **Agreement with Systems (cc1):** Will likely flag §F6 (env leakage
  into PTY children) and §F3 (shutdown drain bound) under a
  "lifecycle/boundary" lens. Expect cc1 to add observability/metrics
  gaps I didn't chase.
- **Agreement with Root-Cause (cod1):** §F3 and §F8 are "symptom vs
  root cause" findings that cod1's framework reaches independently.
- **Agreement with Failure-Mode (cod3) and Edge-Case (cod4):** §F5
  (loopback literal match), §F4 (sanitizer asymmetry), and §F8
  (oversized frame) are all edge/failure surface findings; expect
  overlap.
- **Tension with Deductive (cod2):** cod2 will likely *agree on the
  code facts* but may rate §F1 lower by reasoning "the WS is only used
  by local clients under Tailscale, so query-string leakage needs a
  proxy that doesn't exist." I stand on "the moment someone fronts it,
  it's too late."
- **Tension with Counterfactual (cc4):** cc4 will probably say "what if
  this project stayed solo forever?" — in which case every finding
  here is overkill. I'm rating against "early public crates.io release,
  remote story in README." If the project explicitly renounces the
  remote story, re-rate.
- **Tension with Debiasing (cc5):** cc5 will (correctly) push back on
  severity inflation for §F1/§F2. I've pre-corrected by pinning both
  to "low unless non-loopback + fronted-by-proxy."
- **Agreement with Inductive (cod5):** Patterns of "inconsistency"
  (§F4, §F5) and "defense-in-depth not applied uniformly" are the same
  shape cod5 should surface.
- **Orthogonal to Perspective (cc3):** §F7 (documenting operator scope
  == shell) is the finding cc3 probably owns in its lens; we reach it
  via "what does the adversarial reader of the README conclude?"

## Confidence: 0.82

**What would raise this:**
- Reading the actual `.scpt` files referenced by `src/native/mod.rs`
  (sharpens §F4 from "potential regression" to "either confirmed-safe or
  confirmed-gap").
- Reproducing §F3's shutdown stall empirically.
- Verifying `portable-pty`'s default env-inheritance behavior in
  `SessionActor::spawn` (`src/session/actor.rs:251`, not read in depth
  here).
- Skimming `api/mod.rs` router composition to confirm which routes are
  default vs `personal-workflows`-gated on the crates.io build.

**What would lower this:**
- Discovering that `axum::serve` has a built-in drain timeout default I
  missed (would moot §F3).
- Discovering that the WS token query-string handling has a cookie
  fallback I missed (would moot §F1).
- Discovering the project already has a `SECURITY.md` or README section
  stating "token == shell" (would moot §F7).
