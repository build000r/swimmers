//! Port of thought-loop.js — async thought generation worker.
//!
//! Runs on a configurable interval, generating short natural-language summaries
//! of what each session is doing. Uses context-aware prompts when a structured
//! agent reader is available, falling back to raw terminal output otherwise.
//!
//! Thought generation failures must **never** crash the server or affect
//! terminal streams.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use tokio::sync::broadcast;
use tracing::{debug, error, info};

#[cfg(test)]
use crate::thought::context::AgentAction;
use crate::thought::context::{context_reader_for, ContextReader, ContextSnapshot};
use crate::types::{ControlEvent, SessionState, ThoughtUpdatePayload};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const SUMMARY_HISTORY_CAP: usize = 10;
const CODEX_TIMEOUT: Duration = Duration::from_secs(15);
const TERMINAL_CONTEXT_CHARS: usize = 800;
const TERMINAL_MIN_MEANINGFUL_DELTA_CHARS: usize = 100;

// ---------------------------------------------------------------------------
// Per-session thought state
// ---------------------------------------------------------------------------

/// Mutable per-session state tracked across thought ticks.
struct SessionThoughtState {
    context_reader: Option<Box<dyn ContextReader>>,
    summary_history: Vec<String>,
    last_replay_hash: u64,
    last_thought_context: Option<String>,
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
    /// Token count from the session summary (placeholder -- wired for future use).
    pub token_count: u64,
    /// Context limit from the session summary (placeholder -- wired for future use).
    pub context_limit: u64,
}

/// Trait abstracting the supervisor so the loop runner is testable in isolation.
pub trait SessionProvider: Send + Sync {
    /// Return info for every tracked session.
    fn session_snapshots(&self) -> Vec<SessionInfo>;
}

// ---------------------------------------------------------------------------
// ThoughtLoopRunner
// ---------------------------------------------------------------------------

/// Spawns a background task that periodically generates thoughts for all
/// sessions and broadcasts them as control events.
pub struct ThoughtLoopRunner {
    tick_ms: u64,
    event_tx: broadcast::Sender<ControlEvent>,
}

impl ThoughtLoopRunner {
    pub fn new(tick_ms: u64, event_tx: broadcast::Sender<ControlEvent>) -> Self {
        Self { tick_ms, event_tx }
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
            let mut per_session: HashMap<String, SessionThoughtState> = HashMap::new();

            loop {
                interval.tick().await;

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

                    // Collect file paths already claimed by OTHER sessions'
                    // readers so a new reader won't pick the same JSONL file.
                    let claimed: Vec<std::path::PathBuf> = per_session
                        .iter()
                        .filter(|(id, _)| *id != &info.session_id)
                        .filter_map(|(_, s)| {
                            s.context_reader.as_ref()?.claimed_path()
                        })
                        .collect();

                    let state = per_session
                        .entry(info.session_id.clone())
                        .or_insert_with(|| SessionThoughtState {
                            context_reader: info
                                .tool
                                .as_deref()
                                .and_then(|t| context_reader_for(t, &info.cwd, &claimed)),
                            summary_history: Vec::new(),
                            last_replay_hash: 0,
                            last_thought_context: None,
                        });

                    // Re-create context reader if the tool changed and we
                    // don't have one yet.
                    if state.context_reader.is_none() {
                        if let Some(tool) = info.tool.as_deref() {
                            state.context_reader =
                                context_reader_for(tool, &info.cwd, &claimed);
                        }
                    }

                    // Returns (thought_text, token_count_from_reader)
                    let result = if state.context_reader.is_some() {
                        handle_context_aware(info, state).await
                    } else {
                        handle_terminal_fallback(info, state)
                            .await
                            .map(|t| (t, 0u64))
                    };

                    if let Some((thought, snapshot_tokens)) = result {
                        state.summary_history.push(thought.clone());
                        if state.summary_history.len() > SUMMARY_HISTORY_CAP {
                            let start = state.summary_history.len() - SUMMARY_HISTORY_CAP;
                            state.summary_history = state.summary_history.split_off(start);
                        }

                        // Use token data from context reader when available,
                        // fall back to session summary values.
                        let token_count = if snapshot_tokens > 0 {
                            snapshot_tokens
                        } else {
                            info.token_count
                        };
                        let context_limit =
                            crate::types::context_limit_for_tool(info.tool.as_deref());

                        let payload = ThoughtUpdatePayload {
                            thought: Some(thought),
                            token_count,
                            context_limit,
                            at: chrono::Utc::now(),
                        };
                        let event = ControlEvent {
                            event: "thought_update".to_string(),
                            session_id: info.session_id.clone(),
                            payload: serde_json::to_value(&payload).unwrap_or_default(),
                        };
                        // Broadcast — if no receivers are listening the send
                        // fails silently, which is fine.
                        let _ = self.event_tx.send(event);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Context-aware path
// ---------------------------------------------------------------------------

/// Returns `(thought_text, token_count)` from the context reader.
async fn handle_context_aware(
    info: &SessionInfo,
    state: &mut SessionThoughtState,
) -> Option<(String, u64)> {
    // ContextReader::read is blocking I/O — run on the blocking pool.
    // We need to get ownership briefly; take it out, run, put it back.
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

    let token_count = snapshot.token_count;
    let prompt = build_context_prompt(&snapshot, info.state, &state.summary_history);

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
        Ok(thought) => {
            if thought.is_empty() {
                debug!(session_id = %info.session_id, "llm returned empty");
                None
            } else {
                info!(session_id = %info.session_id, thought = %thought, "llm returned");
                Some((thought, token_count))
            }
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

async fn handle_terminal_fallback(
    info: &SessionInfo,
    state: &mut SessionThoughtState,
) -> Option<String> {
    // Hash check — skip if replay buffer hasn't changed.
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

    let prompt = build_terminal_prompt(&context, info.state, prev_context.as_deref());

    info!(
        session_id = %info.session_id,
        state = ?info.state,
        context_len = context.len(),
        "calling llm (terminal-fallback)"
    );

    match call_llm(&prompt).await {
        Ok(thought) => {
            if thought.is_empty() {
                debug!(session_id = %info.session_id, "llm returned empty");
                None
            } else {
                info!(session_id = %info.session_id, thought = %thought, "llm returned");
                Some(thought)
            }
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
    while cur_suffix > prefix
        && prev_suffix > prefix
        && cur[cur_suffix - 1] == prev[prev_suffix - 1]
    {
        cur_suffix -= 1;
        prev_suffix -= 1;
    }

    cur[prefix..cur_suffix]
        .iter()
        .filter(|c| !c.is_whitespace())
        .count()
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
    parts.push("Good: \"adding JWT refresh to prevent session timeouts\" or \"3 test failures — user_routes returns wrong status code\" or \"understanding DB schema before adding migrations\"".to_string());
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
         Good: \"verifying auth fix — 2 tests still failing in user_routes\" or \"rebasing feature branch, resolving 3 merge conflicts\" or \"idle, waiting for next task\"\n\
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
                debug!(model = %model, error = %e, "model failed, trying next");
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
    fn meaningful_delta_first_snapshot_always_true() {
        assert!(has_meaningful_terminal_delta("hello", None));
    }

    #[test]
    fn meaningful_delta_small_change_skips() {
        let prev = "prompt$ ";
        let current = "prompt$ ls";
        assert!(!has_meaningful_terminal_delta(current, Some(prev)));
    }

    #[test]
    fn meaningful_delta_large_change_triggers() {
        let prev = "prompt$ ";
        let current = format!("prompt$ {}\n", "x".repeat(120));
        assert!(has_meaningful_terminal_delta(&current, Some(prev)));
    }
}
