//! Port of context-reader.js — reads structured agent JSONL files for
//! context-aware thought generation.
//!
//! All file I/O in this module is blocking (`std::fs`). Callers must run
//! reads from `spawn_blocking` to avoid stalling the async runtime.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tracing::warn;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of what the coding agent is doing.
#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    pub user_task: Option<String>,
    pub recent_actions: Vec<AgentAction>,
    pub current_tool: Option<AgentAction>,
    /// Most recent `input_tokens` from assistant message usage data.
    /// This approximates current context window utilization.
    pub token_count: u64,
}

/// A single action observed from the agent's JSONL log.
#[derive(Debug, Clone)]
pub struct AgentAction {
    pub tool: String,
    pub detail: Option<String>,
}

/// Trait implemented by each agent-specific reader.
pub trait ContextReader: Send + Sync {
    /// Read new data from the JSONL file and return a snapshot, or `None` if
    /// nothing has changed since the last call.
    fn read(&mut self) -> Option<ContextSnapshot>;
}

/// Factory: build the right reader for a detected tool.
pub fn context_reader_for(tool: &str, cwd: &str) -> Option<Box<dyn ContextReader>> {
    match tool {
        "Claude Code" => Some(Box::new(ClaudeCodeReader::new(cwd))),
        "Codex" => Some(Box::new(CodexReader::new(cwd))),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

const BOOTSTRAP_MAX: u64 = 1024 * 1024; // 1 MB

/// Read a byte range `[start, end)` from a file.
fn read_range(path: &Path, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let len = (end - start) as usize;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Parse JSONL lines from a byte buffer, skipping malformed lines.
fn parse_jsonl_lines(buf: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(buf);
    text.lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Extract the basename from a path string (last component after `/`).
fn basename(path_str: &str) -> &str {
    path_str.rsplit('/').next().unwrap_or(path_str)
}

// ---------------------------------------------------------------------------
// Claude Code Reader
// ---------------------------------------------------------------------------

/// Reads Claude Code JSONL session files from
/// `~/.claude/projects/{cwd-slash-to-dash}/{SESSION}.jsonl`.
pub struct ClaudeCodeReader {
    cwd: String,
    file_path: Option<PathBuf>,
    file_size: u64,
    user_task: Option<String>,
    recent_actions: Vec<AgentAction>,
    current_tool: Option<AgentAction>,
    bootstrapped: bool,
    /// Most recent input_tokens from assistant message usage.
    token_count: u64,
}

impl ClaudeCodeReader {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: cwd.to_string(),
            file_path: None,
            file_size: 0,
            user_task: None,
            recent_actions: Vec::new(),
            current_tool: None,
            bootstrapped: false,
            token_count: 0,
        }
    }

    /// Discover the most recently modified JSONL file in the project dir.
    fn discover_file(&self) -> Option<PathBuf> {
        let home = dirs_home()?;
        let cwd_slug = self.cwd.replace('/', "-");
        let project_dir = home.join(".claude").join("projects").join(&cwd_slug);

        let entries = fs::read_dir(&project_dir).ok()?;
        let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map_or(false, |ext| ext == "jsonl")
            })
            .filter_map(|e| {
                let md = e.metadata().ok()?;
                let mtime = md.modified().ok()?;
                Some((e.path(), mtime))
            })
            .collect();

        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.into_iter().next().map(|(p, _)| p)
    }

    /// Parse entries and update internal state.
    fn parse_entries(&mut self, entries: &[Value]) {
        for entry in entries {
            let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
            let msg = entry.get("message");

            // User message -> task
            if entry_type == "user" {
                if let Some(msg) = msg {
                    if msg.get("role").and_then(Value::as_str) == Some("user") {
                        let content = &msg["content"];
                        if let Some(text) = content.as_str() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                self.user_task = Some(truncate(trimmed, 300));
                            }
                        } else if let Some(blocks) = content.as_array() {
                            for block in blocks {
                                if block.get("type").and_then(Value::as_str) == Some("text") {
                                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                                        let trimmed = text.trim();
                                        if !trimmed.is_empty() {
                                            self.user_task = Some(truncate(trimmed, 300));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Assistant message -> tool uses, text, and token usage
            if entry_type == "assistant" {
                if let Some(msg) = msg {
                    if msg.get("role").and_then(Value::as_str) == Some("assistant") {
                        // Extract input_tokens from usage data
                        if let Some(usage) = msg.get("usage") {
                            if let Some(input_tokens) = usage.get("input_tokens").and_then(Value::as_u64) {
                                self.token_count = input_tokens;
                            }
                        }
                        if let Some(blocks) = msg.get("content").and_then(Value::as_array) {
                            for block in blocks {
                                let block_type =
                                    block.get("type").and_then(Value::as_str).unwrap_or("");

                                if block_type == "tool_use" {
                                    let tool_name = block
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .unwrap_or("unknown")
                                        .to_string();

                                    let detail = extract_tool_detail(block);

                                    let action = AgentAction {
                                        tool: tool_name,
                                        detail,
                                    };
                                    self.recent_actions.push(action.clone());
                                    cap_actions(&mut self.recent_actions, 10);
                                    self.current_tool = Some(action);
                                } else if block_type == "text" {
                                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                                        let trimmed = text.trim();
                                        if trimmed.len() > 5 {
                                            let action = AgentAction {
                                                tool: "said".to_string(),
                                                detail: Some(truncate(trimmed, 100)),
                                            };
                                            self.recent_actions.push(action);
                                            cap_actions(&mut self.recent_actions, 10);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl ContextReader for ClaudeCodeReader {
    fn read(&mut self) -> Option<ContextSnapshot> {
        let file_path = self.discover_file()?;

        let stat = fs::metadata(&file_path).ok()?;
        let current_size = stat.len();

        // No new data?
        if Some(&file_path) == self.file_path.as_ref() && current_size == self.file_size {
            return None;
        }

        // New or different file — reset state
        if self.file_path.as_ref() != Some(&file_path) {
            self.file_path = Some(file_path.clone());
            self.file_size = 0;
            self.user_task = None;
            self.recent_actions.clear();
            self.current_tool = None;
            self.bootstrapped = false;
        }

        if !self.bootstrapped {
            // Bootstrap: backward scan up to BOOTSTRAP_MAX
            let start = current_size.saturating_sub(BOOTSTRAP_MAX);
            match read_range(&file_path, start, current_size) {
                Ok(buf) => {
                    let entries = parse_jsonl_lines(&buf);
                    self.parse_entries(&entries);
                }
                Err(e) => {
                    warn!(path = %file_path.display(), error = %e, "bootstrap read failed");
                    return None;
                }
            }
            self.file_size = current_size;
            self.bootstrapped = true;
        } else {
            // Incremental: read only new bytes
            match read_range(&file_path, self.file_size, current_size) {
                Ok(buf) => {
                    let entries = parse_jsonl_lines(&buf);
                    self.parse_entries(&entries);
                }
                Err(e) => {
                    warn!(path = %file_path.display(), error = %e, "incremental read failed");
                    return None;
                }
            }
            self.file_size = current_size;
        }

        Some(ContextSnapshot {
            user_task: self.user_task.clone(),
            recent_actions: last_n(&self.recent_actions, 5),
            current_tool: self.current_tool.clone(),
            token_count: self.token_count,
        })
    }
}

// ---------------------------------------------------------------------------
// Codex Reader
// ---------------------------------------------------------------------------

/// Reads Codex JSONL session files from
/// `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
pub struct CodexReader {
    cwd: String,
    file_path: Option<PathBuf>,
    file_size: u64,
    user_task: Option<String>,
    recent_actions: Vec<AgentAction>,
    current_tool: Option<AgentAction>,
    bootstrapped: bool,
    token_count: u64,
}

impl CodexReader {
    pub fn new(cwd: &str) -> Self {
        Self {
            cwd: cwd.to_string(),
            file_path: None,
            file_size: 0,
            user_task: None,
            recent_actions: Vec::new(),
            current_tool: None,
            bootstrapped: false,
            token_count: 0,
        }
    }

    /// Walk `~/.codex/sessions/YYYY/MM/DD/` in reverse chronological order,
    /// returning the first `rollout-*.jsonl` whose `session_meta.cwd` matches.
    fn discover_file(&self) -> Option<PathBuf> {
        let home = dirs_home()?;
        let sessions_dir = home.join(".codex").join("sessions");

        let years = sorted_subdirs_reverse(&sessions_dir, r"^\d{4}$")?;
        for year in years {
            let months = sorted_subdirs_reverse(&year, r"^\d{2}$");
            for month in months.into_iter().flatten() {
                let days = sorted_subdirs_reverse(&month, r"^\d{2}$");
                for day in days.into_iter().flatten() {
                    let mut files: Vec<PathBuf> = fs::read_dir(&day)
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| {
                            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            name.starts_with("rollout-") && name.ends_with(".jsonl")
                        })
                        .collect();
                    files.sort();
                    files.reverse();

                    for f in files {
                        if self.matches_cwd(&f) {
                            return Some(f);
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if the first line of a JSONL file is a `session_meta` entry
    /// whose `cwd` matches ours.
    fn matches_cwd(&self, path: &Path) -> bool {
        let result: std::io::Result<bool> = (|| {
            use std::io::Read;
            let mut f = fs::File::open(path)?;
            let mut buf = vec![0u8; 2048];
            let n = f.read(&mut buf)?;
            buf.truncate(n);
            let text = String::from_utf8_lossy(&buf);
            let first_line = text.lines().next().unwrap_or("");
            if first_line.is_empty() {
                return Ok(false);
            }
            let entry: Value = serde_json::from_str(first_line).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            })?;
            if entry.get("type").and_then(Value::as_str) == Some("session_meta") {
                if let Some(payload) = entry.get("payload") {
                    return Ok(payload.get("cwd").and_then(Value::as_str) == Some(&self.cwd));
                }
            }
            Ok(false)
        })();
        result.unwrap_or(false)
    }

    /// Parse entries and update internal state.
    fn parse_entries(&mut self, entries: &[Value]) {
        for entry in entries {
            let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
            let payload = entry.get("payload").cloned().unwrap_or(Value::Object(Default::default()));

            // response_item with role=user -> user task
            if entry_type == "response_item"
                && payload.get("role").and_then(Value::as_str) == Some("user")
            {
                if let Some(blocks) = payload.get("content").and_then(Value::as_array) {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("input_text") {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                let trimmed = text.trim();
                                // Skip system/developer prompts (very long or XML-like)
                                if !trimmed.is_empty()
                                    && trimmed.len() < 1000
                                    && !trimmed.starts_with('<')
                                {
                                    self.user_task = Some(truncate(trimmed, 300));
                                }
                            }
                        }
                    }
                }
            }

            // event_msg with type=user_message -> cleaner task source
            if entry_type == "event_msg"
                && payload.get("type").and_then(Value::as_str) == Some("user_message")
            {
                if let Some(msg) = payload.get("message").and_then(Value::as_str) {
                    let trimmed = msg.trim();
                    if !trimmed.is_empty() {
                        self.user_task = Some(truncate(trimmed, 300));
                    }
                }
            }

            // response with usage data -> token tracking
            if entry_type == "response" {
                if let Some(usage) = payload.get("usage") {
                    if let Some(input_tokens) = usage.get("input_tokens").and_then(Value::as_u64) {
                        self.token_count = input_tokens;
                    }
                }
            }

            // response_item with type=function_call -> actions
            if entry_type == "response_item"
                && payload.get("type").and_then(Value::as_str) == Some("function_call")
            {
                let tool_name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();

                let detail = payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|args_str| serde_json::from_str::<Value>(args_str).ok())
                    .and_then(|args| {
                        if let Some(cmd) = args.get("command").and_then(Value::as_str) {
                            Some(truncate(cmd, 80))
                        } else if let Some(fp) = args.get("file_path").and_then(Value::as_str) {
                            Some(basename(fp).to_string())
                        } else {
                            None
                        }
                    });

                let action = AgentAction {
                    tool: tool_name,
                    detail,
                };
                self.recent_actions.push(action.clone());
                cap_actions(&mut self.recent_actions, 10);
                self.current_tool = Some(action);
            }

            // event_msg with type=agent_reasoning -> thinking
            if entry_type == "event_msg"
                && payload.get("type").and_then(Value::as_str) == Some("agent_reasoning")
            {
                if let Some(text) = payload.get("text").and_then(Value::as_str) {
                    self.current_tool = Some(AgentAction {
                        tool: "thinking".to_string(),
                        detail: Some(truncate(text, 100)),
                    });
                }
            }

            // response_item with type=reasoning summary -> thinking
            if entry_type == "response_item"
                && payload.get("type").and_then(Value::as_str) == Some("reasoning")
            {
                if let Some(summaries) = payload.get("summary").and_then(Value::as_array) {
                    for s in summaries {
                        if s.get("type").and_then(Value::as_str) == Some("summary_text") {
                            if let Some(text) = s.get("text").and_then(Value::as_str) {
                                self.current_tool = Some(AgentAction {
                                    tool: "thinking".to_string(),
                                    detail: Some(truncate(text, 100)),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

impl ContextReader for CodexReader {
    fn read(&mut self) -> Option<ContextSnapshot> {
        let file_path = self.file_path.clone().or_else(|| self.discover_file())?;

        let stat = fs::metadata(&file_path).ok()?;
        let current_size = stat.len();

        // No new data?
        if Some(&file_path) == self.file_path.as_ref() && current_size == self.file_size {
            return None;
        }

        // New or different file — reset state
        if self.file_path.as_ref() != Some(&file_path) {
            self.file_path = Some(file_path.clone());
            self.file_size = 0;
            self.user_task = None;
            self.recent_actions.clear();
            self.current_tool = None;
            self.bootstrapped = false;
        }

        if !self.bootstrapped {
            let start = current_size.saturating_sub(BOOTSTRAP_MAX);
            match read_range(&file_path, start, current_size) {
                Ok(buf) => {
                    let entries = parse_jsonl_lines(&buf);
                    self.parse_entries(&entries);
                }
                Err(e) => {
                    warn!(path = %file_path.display(), error = %e, "bootstrap read failed");
                    return None;
                }
            }
            self.file_size = current_size;
            self.bootstrapped = true;
        } else {
            match read_range(&file_path, self.file_size, current_size) {
                Ok(buf) => {
                    let entries = parse_jsonl_lines(&buf);
                    self.parse_entries(&entries);
                }
                Err(e) => {
                    warn!(path = %file_path.display(), error = %e, "incremental read failed");
                    return None;
                }
            }
            self.file_size = current_size;
        }

        Some(ContextSnapshot {
            user_task: self.user_task.clone(),
            recent_actions: last_n(&self.recent_actions, 5),
            current_tool: self.current_tool.clone(),
            token_count: self.token_count,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Get the user's home directory.
fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Read sub-directories matching a regex pattern, returned sorted in reverse.
fn sorted_subdirs_reverse(dir: &Path, pattern: &str) -> Option<Vec<PathBuf>> {
    let re = regex::Regex::new(pattern).ok()?;
    let entries = fs::read_dir(dir).ok()?;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().ok().map_or(false, |ft| ft.is_dir()))
        .filter(|e| {
            e.file_name()
                .to_str()
                .map_or(false, |name| re.is_match(name))
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.reverse();
    Some(dirs)
}

/// Truncate a string to at most `max` characters (not bytes).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Keep only the last `cap` actions in the vec.
fn cap_actions(actions: &mut Vec<AgentAction>, cap: usize) {
    if actions.len() > cap {
        let start = actions.len() - cap;
        *actions = actions.split_off(start);
    }
}

/// Return the last `n` elements cloned.
fn last_n(actions: &[AgentAction], n: usize) -> Vec<AgentAction> {
    let start = actions.len().saturating_sub(n);
    actions[start..].to_vec()
}

/// Extract a short detail string from a tool_use block's `input` field.
fn extract_tool_detail(block: &Value) -> Option<String> {
    let input = block.get("input")?;
    if let Some(fp) = input.get("file_path").and_then(Value::as_str) {
        Some(basename(fp).to_string())
    } else if let Some(cmd) = input.get("command").and_then(Value::as_str) {
        Some(truncate(cmd, 80))
    } else if let Some(pat) = input.get("pattern").and_then(Value::as_str) {
        Some(pat.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jsonl_lines_skips_bad() {
        let buf = b"{\"type\":\"user\"}\nnot json\n{\"type\":\"assistant\"}\n";
        let entries = parse_jsonl_lines(buf);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("hello", 3), "hel");
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn basename_extracts() {
        assert_eq!(basename("/foo/bar/baz.rs"), "baz.rs");
        assert_eq!(basename("baz.rs"), "baz.rs");
    }

    #[test]
    fn cap_actions_limits() {
        let mut actions: Vec<AgentAction> = (0..15)
            .map(|i| AgentAction {
                tool: format!("t{i}"),
                detail: None,
            })
            .collect();
        cap_actions(&mut actions, 10);
        assert_eq!(actions.len(), 10);
        assert_eq!(actions[0].tool, "t5");
    }

    #[test]
    fn context_reader_for_known_tools() {
        assert!(context_reader_for("Claude Code", "/tmp").is_some());
        assert!(context_reader_for("Codex", "/tmp").is_some());
        assert!(context_reader_for("Unknown", "/tmp").is_none());
    }
}
