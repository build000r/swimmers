//! Port of thought-loop.js — async thought generation worker.
//!
//! Runs on a configurable interval, generating short natural-language summaries
//! of what each session is doing. Uses context-aware prompts when a structured
//! agent reader is available, falling back to raw terminal output otherwise.
//!
//! Thought updates intentionally include lifecycle metadata so the frontend can
//! prioritize safe thought bubbles over raw terminal previews.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::time::Duration;

use chrono::{DateTime, Utc};
use std::collections::hash_map::RandomState;
use tokio::sync::broadcast;
use tracing::{debug, error, info};

#[cfg(test)]
use crate::thought::context::AgentAction;
use crate::thought::context::{context_reader_for, ContextReader, ContextSnapshot};
use crate::types::{
    BubblePrecedence, ControlEvent, SessionState, ThoughtPolicy, ThoughtSource, ThoughtState,
    ThoughtUpdatePayload,
};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const SUMMARY_HISTORY_CAP: usize = 10;
const CODEX_TIMEOUT: Duration = Duration::from_secs(15);
const TERMINAL_CONTEXT_CHARS: usize = 800;
const TERMINAL_MIN_MEANINGFUL_DELTA_CHARS: usize = 100;
const STATIC_SLEEPING_THOUGHT: &str = "Sleeping.";

// ---------------------------------------------------------------------------
// Per-session thought state
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SessionThoughtState {
    context_reader: Option<Box<dyn ContextReader>>,
    summary_history: Vec<String>,
    last_replay_hash: u64,
    last_thought_context: Option<String>,
    last_context_prompt_hash: u64,
    last_focus_hash: u64,
    last_emitted_thought: Option<String>,
    last_emitted_at: Option<DateTime<Utc>>,
    last_call_at: Option<DateTime<Utc>>,
    sleeping_emitted: bool,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    objective_fingerprint: Option<String>,
    objective_stable_since: DateTime<Utc>,
    last_tool: Option<String>,
}

impl SessionThoughtState {
    fn initialize_from_session_info(info: &SessionInfo, now: DateTime<Utc>) -> Self {
        let mut summary_history = Vec::new();
        if let Some(thought) = info.thought.as_ref() {
            summary_history.push(thought.clone());
        }

        let thought_updated_at = info.thought_updated_at.unwrap_or(now);
        Self {
            context_reader: None,
            summary_history,
            last_replay_hash: 0,
            last_thought_context: None,
            last_context_prompt_hash: 0,
            last_focus_hash: 0,
            last_emitted_thought: info.thought.clone(),
            last_emitted_at: info.thought_updated_at,
            last_call_at: info.thought_updated_at,
            sleeping_emitted: is_sleeping_text(info.thought.as_deref()),
            thought_state: info.thought_state,
            thought_source: info.thought_source,
            objective_fingerprint: info.objective_fingerprint.clone(),
            objective_stable_since: thought_updated_at,
            last_tool: info.tool.clone(),
        }
    }

    fn cadence_for_state(&self, policy: &ThoughtPolicy, now: DateTime<Utc>) -> u64 {
        let objective_age_ms = (now - self.objective_stable_since).num_milliseconds();
        if objective_age_ms >= policy.cadence_ms.cold as i64 {
            policy.cadence_ms.cold
        } else if objective_age_ms >= policy.cadence_ms.warm as i64 {
            policy.cadence_ms.warm
        } else {
            policy.cadence_ms.hot
        }
    }

    fn should_call_for_cadence(&self, policy: &ThoughtPolicy, now: DateTime<Utc>) -> bool {
        match self.last_call_at {
            Some(last_call) => {
                let elapsed = (now - last_call).num_milliseconds();
                elapsed >= self.cadence_for_state(policy, now) as i64
            }
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Public: session info needed by the loop
// ---------------------------------------------------------------------------

/// Snapshot of a single session's data, provided by the supervisor each tick.
pub struct SessionInfo {
    pub session_id: String,
    pub state: SessionState,
    pub exited: bool,
    /// The detected coding tool name (e.g. "Claude Code", "Codex"), if any.
    pub tool: Option<String>,
    /// Working directory of the session.
    pub cwd: String,
    /// Last ~500 chars of visible terminal text from the replay buffer.
    pub replay_text: String,
    /// Current persisted thought text from summary snapshot.
    pub thought: Option<String>,
    /// Current persisted thought lifecycle state.
    pub thought_state: ThoughtState,
    /// Current persisted thought source.
    pub thought_source: ThoughtSource,
    /// Last seen objective fingerprint used to avoid noisy rewrites.
    pub objective_fingerprint: Option<String>,
    /// Time of last persisted thought update.
    pub thought_updated_at: Option<DateTime<Utc>>,
    /// Token count from the session summary (placeholder -- wired for future use).
    pub token_count: u64,
    /// Context limit from the session summary (placeholder -- wired for future use).
    pub context_limit: u64,
    /// Last observed terminal activity timestamp.
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug)]
struct ThoughtCandidate {
    thought: String,
    token_count: u64,
    objective_fingerprint: String,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Trait abstracting the supervisor so the loop runner is testable in isolation.
pub trait SessionProvider: Send + Sync {
    /// Return info for every tracked session.
    fn session_snapshots(&self) -> Vec<SessionInfo>;

    /// Persist the latest thought snapshot for a session.
    fn persist_thought(
        &self,
        _session_id: &str,
        _thought: &str,
        _token_count: u64,
        _context_limit: u64,
        _thought_state: ThoughtState,
        _thought_source: ThoughtSource,
        _objective_fingerprint: Option<String>,
    ) {
    }
}

// ---------------------------------------------------------------------------
// ThoughtLoopRunner
// ---------------------------------------------------------------------------

/// Spawns a background task that periodically generates thoughts for all
/// sessions and broadcasts them as control events.
pub struct ThoughtLoopRunner {
    tick_ms: u64,
    event_tx: broadcast::Sender<ControlEvent>,
    thought_policy: ThoughtPolicy,
}

impl ThoughtLoopRunner {
    pub fn new(
        tick_ms: u64,
        event_tx: broadcast::Sender<ControlEvent>,
        thought_policy: ThoughtPolicy,
    ) -> Self {
        Self {
            tick_ms,
            event_tx,
            thought_policy,
        }
    }

    /// Start the thought loop as a detached tokio task.
    /// The loop runs until the returned `tokio::task::JoinHandle` is aborted.
    pub fn spawn<P: SessionProvider + 'static>(
        self,
        provider: std::sync::Arc<P>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!(
                "thought generation loop started (interval={}ms)",
                self.tick_ms
            );
            let mut interval = tokio::time::interval(Duration::from_millis(self.tick_ms));
            let mut per_session: HashMap<String, SessionThoughtState, RandomState> = HashMap::default();
            loop {
                interval.tick().await;

                let now = Utc::now();
                let snapshots = provider.session_snapshots();
                let count = snapshots.len();
                debug!("thought tick — {count} sessions");

                // Remove state for sessions that no longer exist.
                per_session.retain(|id, _| snapshots.iter().any(|s| s.session_id == *id));

                for info in &snapshots {
                    if info.exited {
                        debug!(session_id = %info.session_id, "skip (exited)");
                        continue;
                    }

                    // Collect file paths already claimed by OTHER sessions' readers so
                    // a new reader won't pick the same JSONL file.
                    let claimed: Vec<std::path::PathBuf> = per_session
                        .iter()
                        .filter(|(id, _)| *id != &info.session_id)
                        .filter_map(|(_, s)| s.context_reader.as_ref()?.claimed_path())
                        .collect();

                    let state = per_session
                        .entry(info.session_id.clone())
                        .or_insert_with(|| SessionThoughtState::initialize_from_session_info(info, now));

                    if is_sleeping_session(info, &self.thought_policy) {
                        let should_emit = state.thought_state != ThoughtState::Sleeping
                            || !state.sleeping_emitted
                            || !is_sleeping_text(state.last_emitted_thought.as_deref());
                        if should_emit {
                            emit_thought_update(
                                &self.event_tx,
                                provider.as_ref(),
                                &info.session_id,
                                STATIC_SLEEPING_THOUGHT,
                                info.token_count,
                                info.context_limit,
                                ThoughtState::Sleeping,
                                ThoughtSource::StaticSleeping,
                                Some("sleeping".to_string()),
                                true,
                                self.thought_policy.bubble_precedence,
                            );
                        }
                        state.sleeping_emitted = true;
                        state.thought_state = ThoughtState::Sleeping;
                        state.thought_source = ThoughtSource::StaticSleeping;
                        state.last_emitted_thought = Some(STATIC_SLEEPING_THOUGHT.to_string());
                        state.last_emitted_at = Some(now);
                        state.last_call_at = Some(now);
                        continue;
                    }

                    if state.thought_state == ThoughtState::Sleeping {
                        state.sleeping_emitted = false;
                        state.objective_stable_since = now;
                    }

                    // Recreate context reader if tool binding changes.
                    if state.context_reader.is_none() || state.last_tool != info.tool {
                        state.last_tool = info.tool.clone();
                        state.context_reader = match info.tool.as_deref() {
                            Some(tool) => context_reader_for(tool, &info.cwd, &claimed),
                            None => None,
                        };
                    }

                    let candidate = if state.context_reader.is_some() {
                        handle_context_aware(info, state).await
                    } else {
                        handle_terminal_fallback(info, state)
                            .await
                            .map(|t| t)
                    };

                    let Some(candidate) = candidate else {
                        state.thought_state = ThoughtState::Holding;
                        continue;
                    };

                    let objective_changed = candidate
                        .objective_fingerprint
                        .as_str()
                        .ne(state.objective_fingerprint.as_deref().unwrap_or(""));
                    let next_thought_state = if objective_changed {
                        ThoughtState::Active
                    } else {
                        ThoughtState::Holding
                    };

                    if !objective_changed && !state.should_call_for_cadence(&self.thought_policy, now) {
                        state.thought_state = next_thought_state;
                        continue;
                    }

                    state.last_call_at = Some(now);

                    // Persist objective transition even when we suppress content rewrites.
                    if objective_changed {
                        state.objective_stable_since = now;
                        state.objective_fingerprint = Some(candidate.objective_fingerprint.clone());
                    }

                    if is_duplicate_thought(state.last_emitted_thought.as_deref(), &candidate.thought) {
                        state.thought_state = next_thought_state;
                        state.thought_source = ThoughtSource::Llm;
                        continue;
                    }

                    state.last_emitted_thought = Some(candidate.thought.clone());
                    state.summary_history.push(candidate.thought.clone());
                    if state.summary_history.len() > SUMMARY_HISTORY_CAP {
                        let start = state.summary_history.len() - SUMMARY_HISTORY_CAP;
                        state.summary_history = state.summary_history.split_off(start);
                    }
                    state.last_emitted_at = Some(now);
                    state.thought_state = next_thought_state;
                    state.thought_source = ThoughtSource::Llm;

                    emit_thought_update(
                        &self.event_tx,
                        provider.as_ref(),
                        &info.session_id,
                        &candidate.thought,
                        candidate.token_count,
                        info.context_limit,
                        state.thought_state,
                        state.thought_source,
                        Some(candidate.objective_fingerprint.clone()),
                        objective_changed,
                        self.thought_policy.bubble_precedence,
                    );
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Emission helpers
// ---------------------------------------------------------------------------

fn emit_thought_update<P: SessionProvider>(
    event_tx: &broadcast::Sender<ControlEvent>,
    provider: &P,
    session_id: &str,
    thought: &str,
    token_count: u64,
    context_limit: u64,
    thought_state: ThoughtState,
    thought_source: ThoughtSource,
    objective_fingerprint: Option<String>,
    objective_changed: bool,
    bubble_precedence: BubblePrecedence,
) {
    provider.persist_thought(
        session_id,
        thought,
        token_count,
        context_limit,
        thought_state,
        thought_source,
        objective_fingerprint,
    );

    let payload = ThoughtUpdatePayload {
        thought: Some(thought.to_string()),
        token_count,
        context_limit,
        thought_state,
        thought_source,
        objective_changed,
        bubble_precedence,
        at: Utc::now(),
    };
    let event = ControlEvent {
        event: "thought_update".to_string(),
        session_id: session_id.to_string(),
        payload: serde_json::to_value(&payload).unwrap_or_default(),
    };
    // Broadcast — if no receivers are listening the send fails silently.
    let _ = event_tx.send(event);
}

fn is_sleeping_session(info: &SessionInfo, policy: &ThoughtPolicy) -> bool {
    if info.state != SessionState::Idle {
        return false;
    }

    let idle_ms = (Utc::now() - info.last_activity_at).num_milliseconds().max(0);
    idle_ms >= policy.sleeping_after_ms as i64
}

fn is_sleeping_text(thought: Option<&str>) -> bool {
    match thought {
        Some(t) => {
            let normalized = t.trim().to_lowercase();
            normalized == STATIC_SLEEPING_THOUGHT.to_lowercase()
                || normalized == STATIC_SLEEPING_THOUGHT.to_lowercase().trim_end_matches('.')
        }
        None => false,
    }
}

fn is_duplicate_thought(previous: Option<&str>, next: &str) -> bool {
    let Some(prev) = previous else {
        return false;
    };
    normalize_for_compare(prev) == normalize_for_compare(next)
}

fn normalize_for_compare(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// ---------------------------------------------------------------------------
// Context-aware path
// ---------------------------------------------------------------------------

/// Returns `(thought_text, token_count, objective_fingerprint)` from context reader.
async fn handle_context_aware(
    info: &SessionInfo,
    state: &mut SessionThoughtState,
) -> Option<ThoughtCandidate> {
    // ContextReader::read is blocking I/O — run on the blocking pool.
    let mut reader_box = state.context_reader.take()?;
    let result = tokio::task::spawn_blocking(move || {
        let snap = reader_box.read();
        (reader_box, snap)
    })
    .await;

    let (reader_box, snapshot) = match result {
        Ok(pair) => pair,
        Err(e) => {
            error!(session_id = %info.session_id, error = %e, "context reader task panicked");
            return None;
        }
    };
    state.context_reader = Some(reader_box);

    let snapshot = match snapshot {
        Some(s) => s,
        None => {
            debug!(session_id = %info.session_id, "skip (context unchanged)");
            return None;
        }
    };

    let objective_fingerprint = context_focus_fingerprint(&snapshot, info.state).to_string();
    if objective_fingerprint == state
        .objective_fingerprint
        .as_deref()
        .unwrap_or("")
    {
        debug!(session_id = %info.session_id, "skip (objective unchanged)");
        state.last_focus_hash = context_focus_fingerprint(&snapshot, info.state);
        return None;
    }
    state.last_focus_hash = objective_fingerprint.hash();
    let token_count = snapshot.token_count;
    let prompt = build_context_prompt(&snapshot, info.state, &state.summary_history);

    let prompt_hash = hash_string(&prompt);
    if prompt_hash == state.last_context_prompt_hash {
        debug!(
            session_id = %info.session_id,
            "skip (prompt unchanged)"
        );
        return None;
    }
    state.last_context_prompt_hash = prompt_hash;

    let task_preview = snapshot
        .user_task
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(50)
        .collect::<String>();
    info!(
        session_id = %info.session_id,
        state = ?info.state,
        task = %task_preview,
        "calling llm (context-aware)"
    );

    match call_llm(&prompt).await {
        Ok(thought) if thought.is_empty() => {
            debug!(session_id = %info.session_id, "llm returned empty");
            None
        }
        Ok(thought) => {
            info!(session_id = %info.session_id, thought = %thought, "llm returned");
            Some(ThoughtCandidate {
                thought,
                token_count,
                objective_fingerprint,
            })
        }
        Err(e) => {
            error!(session_id = %info.session_id, error = %e, "llm error");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal fallback path
// ---------------------------------------------------------------------------

/// Returns `(thought, token_count, objective_fingerprint)` from terminal fallback.
async fn handle_terminal_fallback(
    info: &SessionInfo,
    state: &mut SessionThoughtState,
) -> Option<ThoughtCandidate> {
    let hash = hash_string(&info.replay_text);
    if hash == state.last_replay_hash {
        debug!(session_id = %info.session_id, "skip (unchanged hash)");
        return None;
    }
    state.last_replay_hash = hash;

    let context: String = info
        .replay_text
        .chars()
        .rev()
        .take(TERMINAL_CONTEXT_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let context = context.trim().to_string();
    if context.is_empty() {
        debug!(session_id = %info.session_id, "skip (empty context)");
        return None;
    }

    let prev_context = state.last_thought_context.clone();
    if !has_meaningful_terminal_delta(&context, prev_context.as_deref()) {
        debug!(
            session_id = %info.session_id,
            min_chars = TERMINAL_MIN_MEANINGFUL_DELTA_CHARS,
            "skip (delta below threshold)"
        );
        return None;
    }
    state.last_thought_context = Some(context.clone());

    let objective_fingerprint = terminal_objective_fingerprint(&context, info.state);
    let prompt = build_terminal_prompt(&context, info.state, prev_context.as_deref());

    info!(
        session_id = %info.session_id,
        state = ?info.state,
        context_len = context.len(),
        "calling llm (terminal-fallback)"
    );

    match call_llm(&prompt).await {
        Ok(thought) if thought.is_empty() => {
            debug!(session_id = %info.session_id, "llm returned empty");
            None
        }
        Ok(thought) => {
            info!(session_id = %info.session_id, thought = %thought, "llm returned");
            Some(ThoughtCandidate {
                thought,
                token_count: info.token_count,
                objective_fingerprint: objective_fingerprint.to_string(),
            })
        }
        Err(e) => {
            error!(session_id = %info.session_id, error = %e, "llm error");
            None
        }
    }
}

/// Whether terminal output changed enough to justify an LLM thought call.
///
/// First-time snapshots always pass. For subsequent snapshots, require at
/// least TERMINAL_MIN_MEANINGFUL_DELTA_CHARS non-whitespace chars changed
/// after ANSI stripping.
fn has_meaningful_terminal_delta(context: &str, prev_context: Option<&str>) -> bool {
    let Some(prev) = prev_context else {
        return true;
    };

    let clean = strip_ansi(context);
    let clean_prev = strip_ansi(prev);

    if clean == clean_prev {
        return false;
    }

    changed_non_whitespace_chars(&clean, &clean_prev) >= TERMINAL_MIN_MEANINGFUL_DELTA_CHARS
}

/// Count non-whitespace chars in the changed span between two strings.
///
/// Uses longest common prefix/suffix to isolate the delta region in `current`.
fn changed_non_whitespace_chars(current: &str, previous: &str) -> usize {
    let cur: Vec<char> = current.chars().collect();
    let prev: Vec<char> = previous.chars().collect();

    let mut prefix = 0usize;
    while prefix < cur.len() && prefix < prev.len() && cur[prefix] == prev[prefix] {
        prefix += 1;
    }

    let mut cur_suffix = cur.len();
    let mut prev_suffix = prev.len();
    while cur_suffix > prefix && prev_suffix > prefix && cur[cur_suffix - 1] == prev[prev_suffix - 1] {
        cur_suffix -= 1;
        prev_suffix -= 1;
    }

    cur[prefix..cur_suffix]
        .iter()
        .filter(|c| !c.is_whitespace())
        .count()
}

fn context_focus_fingerprint(snapshot: &ContextSnapshot, state: SessionState) -> u64 {
    let mut parts = vec![format!("state={}", state_label(state))];

    if let Some(task) = snapshot.user_task.as_deref() {
        let normalized = normalize_for_focus(task);
        if !normalized.is_empty() {
            parts.push(format!("task={normalized}"));
        }
    }

    if let Some(current_tool) = snapshot.current_tool.as_ref() {
        let normalized = normalize_for_focus(&current_tool.tool);
        if !normalized.is_empty() {
            parts.push(format!("now={normalized}"));
        }
    }

    let recent_tools: Vec<String> = snapshot
        .recent_actions
        .iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|a| normalize_for_focus(&a.tool))
        .filter(|tool| !tool.is_empty())
        .collect();
    if !recent_tools.is_empty() {
        parts.push(format!("recent={}", recent_tools.join(",")));
    }

    hash_string(&parts.join("|"))
}

fn terminal_objective_fingerprint(context: &str, state: SessionState) -> String {
    let context = strip_ansi(context);
    let preview = context
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("|");

    let material = format!("state={}|{}", state_label(state), normalize_for_focus(&preview));
    hash_string(&material).to_string()
}

fn normalize_for_focus(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// ---------------------------------------------------------------------------
// Prompt builders (matching the JS implementation exactly)
// ---------------------------------------------------------------------------

fn build_context_prompt(
    snapshot: &ContextSnapshot,
    state: SessionState,
    summary_history: &[String],
) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push("You are a status reporter for a coding agent session.".to_string());
    parts.push(format!("State: {}", state_label(state)));

    if let Some(ref task) = snapshot.user_task {
        parts.push(format!("Task: {task}"));
    }

    if !summary_history.is_empty() {
        let recent: Vec<&String> = summary_history
            .iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        parts.push("Recent status:".to_string());
        for s in recent {
            parts.push(format!("  {s}"));
        }
    }

    if !snapshot.recent_actions.is_empty() {
        parts.push("Actions:".to_string());
        for a in &snapshot.recent_actions {
            if a.tool == "said" {
                parts.push(format!("  said: {}", a.detail.as_deref().unwrap_or("")));
            } else {
                let detail_part = a
                    .detail
                    .as_ref()
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default();
                parts.push(format!("  {}{detail_part}", a.tool));
            }
        }
    }

    if let Some(ref ct) = snapshot.current_tool {
        let detail_part = ct
            .detail
            .as_ref()
            .map(|d| format!(": {d}"))
            .unwrap_or_default();
        parts.push(format!("Now: {}{detail_part}", ct.tool));
    }

    parts.push(String::new());
    parts.push("Write a 1-line status (max 60 chars). Explain the PURPOSE and WHY, not the tool or command.".to_string());
    parts.push("Do not speculate about anticipated future steps.".to_string());
    parts.push(
        "Good: \"adding JWT refresh to prevent session timeouts\" or \"3 test failures — user_routes returns wrong status code\" or \"understanding DB schema before adding migrations\"".to_string(),
    );
    parts.push(
        "Bad: \"running tests\" or \"editing files\" or \"using Read tool\" or \"working on code\""
            .to_string(),
    );
    parts.push("Reply with ONLY the status line, nothing else.".to_string());

    parts.join("\n")
}

fn build_terminal_prompt(context: &str, state: SessionState, prev_context: Option<&str>) -> String {
    // Strip ANSI from the context we send to the LLM so it sees clean text.
    let clean = strip_ansi(context);
    let clean_prev = prev_context.map(strip_ansi);

    let context_block = if let Some(ref prev) = clean_prev {
        // Try to find new output since last check.
        let tail: String = prev
            .chars()
            .rev()
            .take(200)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        match clean.find(&tail) {
            Some(idx) => {
                let delta = clean[idx + tail.len()..].trim();
                if !delta.is_empty() {
                    format!("New output:\n{delta}")
                } else {
                    format!(
                        "Screen:\n{}",
                        clean
                            .chars()
                            .rev()
                            .take(300)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<String>()
                    )
                }
            }
            None => format!("Screen:\n{clean}"),
        }
    } else {
        format!("Screen:\n{clean}")
    };

    format!(
        "Terminal session status reporter.\n\
         State: {}\n\
         {context_block}\n\n\
         Write a 1-line status (max 60 chars). Infer the PURPOSE behind what's on screen — WHY is this happening, not WHAT command is running.\n\
         Do not speculate about anticipated future steps.\n\
         Good: \"verifying auth fix — 2 tests still failing in user_routes\" or \"rebasing feature branch, resolving 3 merge conflicts\" or \"sleeping\"\n\
         Bad: \"running cargo test\" or \"editing a file\" or \"using command line tools\" or \"git operations\"\n\
         Reply with ONLY the status line, nothing else.",
        state_label(state)
    )
}

// ---------------------------------------------------------------------------
// OpenRouter LLM call
// ---------------------------------------------------------------------------

/// Lazily-initialized shared HTTP client for thought generation.
fn http_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(CODEX_TIMEOUT)
            .build()
            .expect("failed to build reqwest client")
    })
}

/// Models to try, in order. Falls back through the list on failure.
fn thought_models() -> Vec<String> {
    let mut models = Vec::new();
    for key in [
        "THRONGTERM_THOUGHT_MODEL",
        "THRONGTERM_THOUGHT_MODEL_2",
        "THRONGTERM_THOUGHT_MODEL_3",
    ] {
        if let Ok(m) = std::env::var(key) {
            if !m.is_empty() {
                models.push(m);
            }
        }
    }
    if models.is_empty() {
        models.push("openrouter/aurora-alpha".to_string());
    }
    models
}

async fn call_llm(prompt: &str) -> Result<String, String> {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY not set".to_string())?;

    let models = thought_models();
    let mut last_err = String::new();

    for model in &models {
        match call_openrouter(prompt, model, &api_key).await {
            Ok(content) if !content.is_empty() => return Ok(content),
            Ok(_) => {
                last_err = format!("{model} returned empty");
                debug!(model = %model, "empty response, trying next model");
            }
            Err(e) => {
                last_err = format!("{model}: {e}");
                debug!(model = %model, error = %e, "model failed, trying next model");
            }
        }
    }

    Err(format!("all models failed, last: {last_err}"))
}

async fn call_openrouter(prompt: &str, model: &str, api_key: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 80,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    });

    let resp = http_client()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let preview: String = text.chars().take(500).collect();
        return Err(format!("{status}: {preview}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("json parse failed: {e}"))?;

    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    Ok(content)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn state_label(state: SessionState) -> &'static str {
    match state {
        SessionState::Idle => "idle",
        SessionState::Busy => "busy",
        SessionState::Error => "error",
        SessionState::Attention => "attention",
        SessionState::Exited => "exited",
    }
}

/// Strip ANSI escape sequences so hashing compares visible content only.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI sequence: ESC [ ... final byte
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            // OSC sequence: ESC ] ... ST(ESC \) or BEL
            } else if chars.peek() == Some(&']') {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            } else {
                // Other ESC sequence — skip next char
                chars.next();
            }
        } else if c.is_control() && c != '\n' && c != '\t' {
            // Skip other control chars (cursor blink, etc.)
        } else {
            out.push(c);
        }
    }
    out
}

fn hash_string(s: &str) -> u64 {
    let stripped = strip_ansi(s);
    let mut hasher = DefaultHasher::new();
    stripped.hash(&mut hasher);
    hasher.finish()
}

trait ObjectiveFingerprintExt {
    fn hash(self) -> u64
    where
        Self: ToString;
}
impl<T: ToString> ObjectiveFingerprintExt for T {
    fn hash(self) -> u64 {
        hash_string(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_label_matches() {
        assert_eq!(state_label(SessionState::Idle), "idle");
        assert_eq!(state_label(SessionState::Busy), "busy");
        assert_eq!(state_label(SessionState::Error), "error");
        assert_eq!(state_label(SessionState::Attention), "attention");
        assert_eq!(state_label(SessionState::Exited), "exited");
    }

    #[test]
    fn context_prompt_includes_task() {
        let snapshot = ContextSnapshot {
            user_task: Some("fix the login bug".to_string()),
            recent_actions: vec![],
            current_tool: None,
            token_count: 0,
        };
        let prompt = build_context_prompt(&snapshot, SessionState::Busy, &[]);
        assert!(prompt.contains("fix the login bug"));
        assert!(prompt.contains("State: busy"));
        assert!(prompt.contains("status"));
    }

    #[test]
    fn context_prompt_includes_actions() {
        let snapshot = ContextSnapshot {
            user_task: None,
            recent_actions: vec![
                AgentAction {
                    tool: "Read".to_string(),
                    detail: Some("main.rs".to_string()),
                },
                AgentAction {
                    tool: "said".to_string(),
                    detail: Some("I will fix this".to_string()),
                },
            ],
            current_tool: Some(AgentAction {
                tool: "Edit".to_string(),
                detail: Some("config.rs".to_string()),
            }),
            token_count: 0,
        };
        let prompt = build_context_prompt(&snapshot, SessionState::Busy, &[]);
        assert!(prompt.contains("Read: main.rs"));
        assert!(prompt.contains("said: I will fix this"));
        assert!(prompt.contains("Now: Edit: config.rs"));
    }

    #[test]
    fn context_prompt_includes_history() {
        let snapshot = ContextSnapshot {
            user_task: None,
            recent_actions: vec![],
            current_tool: None,
            token_count: 0,
        };
        let history = vec![
            "reading config files".to_string(),
            "writing new endpoint".to_string(),
        ];
        let prompt = build_context_prompt(&snapshot, SessionState::Idle, &history);
        assert!(prompt.contains("Recent status:"));
        assert!(prompt.contains("reading config files"));
        assert!(prompt.contains("writing new endpoint"));
    }

    #[test]
    fn terminal_prompt_first_time() {
        let prompt = build_terminal_prompt("$ ls\nfoo bar", SessionState::Idle, None);
        assert!(prompt.contains("Screen:"));
        assert!(prompt.contains("$ ls\nfoo bar"));
        assert!(prompt.contains("State: idle"));
    }

    #[test]
    fn terminal_prompt_with_delta() {
        let prev = "$ ls\nfoo bar";
        let current = "$ ls\nfoo bar\n$ echo hello\nhello";
        let prompt = build_terminal_prompt(current, SessionState::Busy, Some(prev));
        assert!(prompt.contains("State: busy"));
        assert!(prompt.contains("status"));
    }

    #[test]
    fn hash_string_deterministic() {
        let h1 = hash_string("hello world");
        let h2 = hash_string("hello world");
        let h3 = hash_string("different");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn duplicate_thought_normalizes_case_and_whitespace() {
        assert!(is_duplicate_thought(
            Some(" Working   on login fix "),
            "working on LOGIN fix"
        ));
        assert!(!is_duplicate_thought(
            Some("working on login fix"),
            "investigating failing auth tests"
        ));
    }

    #[test]
    fn context_focus_ignores_action_detail_noise() {
        let base = ContextSnapshot {
            user_task: Some("Fix auth bug".to_string()),
            recent_actions: vec![AgentAction {
                tool: "Read".to_string(),
                detail: Some("foo.rs".to_string()),
            }],
            current_tool: Some(AgentAction {
                tool: "Edit".to_string(),
                detail: Some("auth.rs".to_string()),
            }),
            token_count: 0,
        };
        let noisy = ContextSnapshot {
            user_task: Some("  Fix   auth bug ".to_string()),
            recent_actions: vec![AgentAction {
                tool: "Read".to_string(),
                detail: Some("bar.rs".to_string()),
            }],
            current_tool: Some(AgentAction {
                tool: "Edit".to_string(),
                detail: Some("routes.rs".to_string()),
            }),
            token_count: 0,
        };

        assert_eq!(
            context_focus_fingerprint(&base, SessionState::Busy),
            context_focus_fingerprint(&noisy, SessionState::Busy)
        );
    }
}
