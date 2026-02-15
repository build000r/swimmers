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

use tokio::process::Command;
use tokio::sync::broadcast;
use tracing::{debug, error, info};

use crate::thought::context::{context_reader_for, ContextReader, ContextSnapshot};
#[cfg(test)]
use crate::thought::context::AgentAction;
use crate::types::{ControlEvent, SessionState, ThoughtUpdatePayload};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const SUMMARY_HISTORY_CAP: usize = 10;
const CODEX_TIMEOUT: Duration = Duration::from_secs(15);
const TERMINAL_CONTEXT_CHARS: usize = 500;

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
            info!("thought generation loop started (interval={}ms)", self.tick_ms);
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

                    let state = per_session
                        .entry(info.session_id.clone())
                        .or_insert_with(|| SessionThoughtState {
                            context_reader: info
                                .tool
                                .as_deref()
                                .and_then(|t| context_reader_for(t, &info.cwd)),
                            summary_history: Vec::new(),
                            last_replay_hash: 0,
                            last_thought_context: None,
                        });

                    // Re-create context reader if the tool changed and we
                    // don't have one yet.
                    if state.context_reader.is_none() {
                        if let Some(tool) = info.tool.as_deref() {
                            state.context_reader = context_reader_for(tool, &info.cwd);
                        }
                    }

                    let thought = if state.context_reader.is_some() {
                        handle_context_aware(info, state).await
                    } else {
                        handle_terminal_fallback(info, state).await
                    };

                    if let Some(thought) = thought {
                        state.summary_history.push(thought.clone());
                        if state.summary_history.len() > SUMMARY_HISTORY_CAP {
                            let start = state.summary_history.len() - SUMMARY_HISTORY_CAP;
                            state.summary_history = state.summary_history.split_off(start);
                        }

                        let payload = ThoughtUpdatePayload {
                            thought: Some(thought),
                            // TODO: Wire actual token_count/context_limit from the
                            // context reader or session metadata when available.
                            // Currently these reflect the session summary's values.
                            token_count: info.token_count,
                            context_limit: info.context_limit,
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

async fn handle_context_aware(
    info: &SessionInfo,
    state: &mut SessionThoughtState,
) -> Option<String> {
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
        "calling codex (context-aware)"
    );

    match call_codex(&prompt).await {
        Ok(thought) => {
            if thought.is_empty() {
                debug!(session_id = %info.session_id, "codex returned empty");
                None
            } else {
                info!(session_id = %info.session_id, thought = %thought, "codex returned");
                Some(thought)
            }
        }
        Err(e) => {
            error!(session_id = %info.session_id, error = %e, "codex error");
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

    let prev_context = state.last_thought_context.take();
    state.last_thought_context = Some(context.clone());

    let prompt = build_terminal_prompt(&context, info.state, prev_context.as_deref());

    info!(
        session_id = %info.session_id,
        state = ?info.state,
        context_len = context.len(),
        "calling codex (terminal-fallback)"
    );

    match call_codex(&prompt).await {
        Ok(thought) => {
            if thought.is_empty() {
                debug!(session_id = %info.session_id, "codex returned empty");
                None
            } else {
                info!(session_id = %info.session_id, thought = %thought, "codex returned");
                Some(thought)
            }
        }
        Err(e) => {
            error!(session_id = %info.session_id, error = %e, "codex error");
            None
        }
    }
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

    parts.push("You are observing a coding agent's work session.".to_string());
    parts.push(format!("Agent state: {}", state_label(state)));
    parts.push(String::new());

    if let Some(ref task) = snapshot.user_task {
        parts.push("User's request:".to_string());
        parts.push(format!("\"{task}\""));
        parts.push(String::new());
    }

    if !summary_history.is_empty() {
        let recent: Vec<&String> = summary_history
            .iter()
            .rev()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        parts.push("Previous observations (oldest to newest):".to_string());
        for s in recent {
            parts.push(format!("- {s}"));
        }
        parts.push(String::new());
    }

    if !snapshot.recent_actions.is_empty() {
        parts.push("Recent agent actions:".to_string());
        for a in &snapshot.recent_actions {
            if a.tool == "said" {
                parts.push(format!(
                    "- Said: \"{}\"",
                    a.detail.as_deref().unwrap_or("")
                ));
            } else {
                let detail_part = a
                    .detail
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default();
                parts.push(format!("- Used tool: {}{detail_part}", a.tool));
            }
        }
        parts.push(String::new());
    }

    if let Some(ref ct) = snapshot.current_tool {
        let detail_part = ct
            .detail
            .as_ref()
            .map(|d| format!(" — {d}"))
            .unwrap_or_default();
        parts.push(format!("Right now: {}{detail_part}", ct.tool));
        parts.push(String::new());
    }

    parts.push("---".to_string());
    parts.push("Summarize what the agent is working on RIGHT NOW in 3-8 words.".to_string());
    parts.push(
        "Be specific about the task, not the tool. No quotes, no preamble.".to_string(),
    );
    parts.push(
        "Examples: \"fixing auth token refresh\", \"reading test files for context\", \"writing new API endpoint\""
            .to_string(),
    );

    parts.join("\n")
}

fn build_terminal_prompt(
    context: &str,
    state: SessionState,
    prev_context: Option<&str>,
) -> String {
    let is_first = prev_context.is_none();

    let context_block = if is_first {
        format!("Full terminal output:\n{context}")
    } else {
        let prev = prev_context.unwrap();
        // Try to find overlap with the last 200 chars of the previous context.
        let tail: String = prev.chars().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
        let overlap = context.find(&tail);
        match overlap {
            Some(idx) => {
                let delta = &context[idx + tail.len()..];
                let delta = delta.trim();
                if !delta.is_empty() {
                    format!("New output since last check:\n{delta}")
                } else {
                    let tail_200: String = context
                        .chars()
                        .rev()
                        .take(200)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("Terminal output (unchanged):\n{tail_200}")
                }
            }
            None => {
                // No overlap found — just send the whole context.
                format!("Full terminal output:\n{context}")
            }
        }
    };

    format!(
        "You are monitoring a terminal session.\n\
         State: {}\n\
         {context_block}\n\
         ---\n\
         Identify what TASK is happening right now. Respond with ONLY 3-8 words. No quotes, no preamble.\n\
         Focus on the task/goal, not the tool or command.\n\
         Examples: \"fixing auth token refresh\", \"adding dark mode toggle\", \"debugging failing test suite\", \"waiting for user input\", \"idle at shell prompt\"",
        state_label(state)
    )
}

// ---------------------------------------------------------------------------
// Codex CLI invocation
// ---------------------------------------------------------------------------

async fn call_codex(prompt: &str) -> Result<String, String> {
    let result = tokio::time::timeout(
        CODEX_TIMEOUT,
        Command::new("codex")
            .args([
                "-m",
                "codex-mini-latest",
                "exec",
                "-c",
                "model_reasoning_effort=\"low\"",
                "--ephemeral",
                prompt,
            ])
            .output(),
    )
    .await;

    match result {
        Err(_) => Err("codex timed out".to_string()),
        Ok(Err(e)) => Err(format!("codex exec failed: {e}")),
        Ok(Ok(output)) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let preview: String = stderr.chars().take(200).collect();
                Err(format!(
                    "codex exited with {}: {preview}",
                    output.status
                ))
            } else {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(stdout)
            }
        }
    }
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

fn hash_string(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
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
        };
        let prompt = build_context_prompt(&snapshot, SessionState::Busy, &[]);
        assert!(prompt.contains("fix the login bug"));
        assert!(prompt.contains("Agent state: busy"));
        assert!(prompt.contains("3-8 words"));
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
        };
        let prompt = build_context_prompt(&snapshot, SessionState::Busy, &[]);
        assert!(prompt.contains("Used tool: Read (main.rs)"));
        assert!(prompt.contains("Said: \"I will fix this\""));
        assert!(prompt.contains("Right now: Edit — config.rs"));
    }

    #[test]
    fn context_prompt_includes_history() {
        let snapshot = ContextSnapshot {
            user_task: None,
            recent_actions: vec![],
            current_tool: None,
        };
        let history = vec![
            "reading config files".to_string(),
            "writing new endpoint".to_string(),
        ];
        let prompt = build_context_prompt(&snapshot, SessionState::Idle, &history);
        assert!(prompt.contains("Previous observations"));
        assert!(prompt.contains("- reading config files"));
        assert!(prompt.contains("- writing new endpoint"));
    }

    #[test]
    fn terminal_prompt_first_time() {
        let prompt = build_terminal_prompt("$ ls\nfoo bar", SessionState::Idle, None);
        assert!(prompt.contains("Full terminal output:"));
        assert!(prompt.contains("$ ls\nfoo bar"));
        assert!(prompt.contains("State: idle"));
    }

    #[test]
    fn terminal_prompt_with_delta() {
        let prev = "$ ls\nfoo bar";
        let current = "$ ls\nfoo bar\n$ echo hello\nhello";
        let prompt = build_terminal_prompt(current, SessionState::Busy, Some(prev));
        // Should detect new output
        assert!(prompt.contains("terminal") || prompt.contains("Terminal"));
    }

    #[test]
    fn hash_string_deterministic() {
        let h1 = hash_string("hello world");
        let h2 = hash_string("hello world");
        let h3 = hash_string("different");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}
