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
    /// Effective context window limit used for UI gauge calculations.
    pub context_limit: u64,
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

    /// Return the file path currently claimed by this reader, if any.
    /// Used by the thought loop to prevent multiple readers from reading
    /// the same JSONL file when sessions share a working directory.
    fn claimed_path(&self) -> Option<PathBuf> {
        None
    }
}

/// Factory: build the right reader for a detected tool.
///
/// `excluded` contains file paths already claimed by other readers.
/// Readers must skip these during file discovery to avoid two sessions
/// reading the same JSONL file (which causes thoughts to cross-contaminate).
pub fn context_reader_for(
    tool: &str,
    cwd: &str,
    excluded: &[PathBuf],
) -> Option<Box<dyn ContextReader>> {
    match tool {
        "Claude Code" => Some(Box::new(ClaudeCodeReader::new(cwd, excluded))),
        "Codex" => Some(Box::new(CodexReader::new(cwd, excluded))),
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

/// Claude JSONL can contain multiple record kinds; scan a small prefix for
/// top-level `cwd` fields and require an exact match when present.
fn claude_file_matches_cwd(path: &Path, cwd: &str) -> bool {
    use std::io::BufRead;

    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let reader = std::io::BufReader::new(file);

    let mut saw_cwd_field = false;
    for line in reader.lines().take(64) {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if let Some(entry_cwd) = value.get("cwd").and_then(Value::as_str) {
            saw_cwd_field = true;
            if entry_cwd == cwd {
                return true;
            }
        }
    }

    // Legacy files may omit top-level cwd metadata; preserve old behavior.
    !saw_cwd_field
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
    context_limit: u64,
    /// File paths claimed by other readers — skip these during discovery
    /// to avoid two sessions reading the same JSONL file.
    excluded_paths: Vec<PathBuf>,
}

impl ClaudeCodeReader {
    pub fn new(cwd: &str, excluded: &[PathBuf]) -> Self {
        Self {
            cwd: cwd.to_string(),
            file_path: None,
            file_size: 0,
            user_task: None,
            recent_actions: Vec::new(),
            current_tool: None,
            bootstrapped: false,
            token_count: 0,
            context_limit: crate::types::context_limit_for_tool(Some("Claude Code")),
            excluded_paths: excluded.to_vec(),
        }
    }

    /// Discover the most recently modified JSONL file in the project dir,
    /// skipping files already claimed by other readers.
    fn discover_file(&self) -> Option<PathBuf> {
        let home = dirs_home()?;
        let cwd_slug = self.cwd.replace('/', "-");
        let project_dir = home.join(".claude").join("projects").join(&cwd_slug);

        let entries = fs::read_dir(&project_dir).ok()?;
        let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "jsonl"))
            .filter(|e| claude_file_matches_cwd(&e.path(), &self.cwd))
            .filter(|e| !self.excluded_paths.contains(&e.path()))
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
            self.capture_claude_user_message(entry_type, msg);
            self.capture_claude_assistant_message(entry_type, msg);
        }
    }

    fn capture_claude_user_message(&mut self, entry_type: &str, msg: Option<&Value>) {
        if entry_type != "user"
            || msg.and_then(|msg| msg.get("role").and_then(Value::as_str)) != Some("user")
        {
            return;
        }

        let Some(content) = msg.map(|msg| &msg["content"]) else {
            return;
        };

        if let Some(text) = content.as_str() {
            self.set_reader_user_task(text);
            return;
        }

        for block in content.as_array().into_iter().flatten() {
            if block.get("type").and_then(Value::as_str) != Some("text") {
                continue;
            }
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                self.set_reader_user_task(text);
                break;
            }
        }
    }

    fn capture_claude_assistant_message(&mut self, entry_type: &str, msg: Option<&Value>) {
        if entry_type != "assistant"
            || msg.and_then(|msg| msg.get("role").and_then(Value::as_str)) != Some("assistant")
        {
            return;
        }

        if let Some(input_tokens) = msg
            .and_then(|msg| msg.get("usage"))
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_u64)
        {
            self.token_count = input_tokens;
        }

        for block in msg
            .and_then(|msg| msg.get("content"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.capture_claude_assistant_block(block);
        }
    }

    fn capture_claude_assistant_block(&mut self, block: &Value) {
        match block.get("type").and_then(Value::as_str).unwrap_or("") {
            "tool_use" => {
                let action = AgentAction {
                    tool: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    detail: extract_tool_detail(block),
                };
                self.record_reader_action(action, true);
            }
            "text" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if trimmed.len() > 5 {
                        self.record_reader_action(
                            AgentAction {
                                tool: "said".to_string(),
                                detail: Some(truncate(trimmed, 100)),
                            },
                            false,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn set_reader_user_task(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        self.user_task = Some(truncate(trimmed, 300));
    }

    fn record_reader_action(&mut self, action: AgentAction, set_current_tool: bool) {
        self.recent_actions.push(action.clone());
        cap_actions(&mut self.recent_actions, 10);
        if set_current_tool {
            self.current_tool = Some(action);
        }
    }
}

impl ContextReader for ClaudeCodeReader {
    fn read(&mut self) -> Option<ContextSnapshot> {
        // Stick with our current file if it exists; only discover on first
        // call or after the file is deleted.  This prevents two readers with
        // the same cwd from flip-flopping onto the same JSONL file each tick.
        let file_path = match self.file_path.clone() {
            Some(p) if p.exists() => p,
            _ => {
                self.file_path = None;
                self.discover_file()?
            }
        };

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
            self.token_count = 0;
            self.context_limit = crate::types::context_limit_for_tool(Some("Claude Code"));
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
            context_limit: self.context_limit,
        })
    }

    fn claimed_path(&self) -> Option<PathBuf> {
        self.file_path.clone()
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
    context_limit: u64,
    /// File paths claimed by other readers — skip these during discovery
    /// to avoid two sessions reading the same JSONL file.
    excluded_paths: Vec<PathBuf>,
}

impl CodexReader {
    pub fn new(cwd: &str, excluded: &[PathBuf]) -> Self {
        Self {
            cwd: cwd.to_string(),
            file_path: None,
            file_size: 0,
            user_task: None,
            recent_actions: Vec::new(),
            current_tool: None,
            bootstrapped: false,
            token_count: 0,
            context_limit: crate::types::context_limit_for_tool(Some("Codex")),
            excluded_paths: excluded.to_vec(),
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
                        if self.excluded_paths.contains(&f) {
                            continue;
                        }
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
            let entry: Value = serde_json::from_str(first_line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
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
            let payload = entry
                .get("payload")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            self.capture_codex_response_item_user(entry_type, &payload);
            self.capture_codex_user_message(entry_type, &payload);
            self.capture_codex_usage_response(entry_type, &payload);
            self.capture_codex_token_count(entry_type, &payload);
            self.capture_codex_function_call(entry_type, &payload);
            self.capture_codex_agent_reasoning(entry_type, &payload);
            self.capture_codex_reasoning_summary(entry_type, &payload);
        }
    }

    fn capture_codex_response_item_user(&mut self, entry_type: &str, payload: &Value) {
        if entry_type != "response_item"
            || payload.get("role").and_then(Value::as_str) != Some("user")
        {
            return;
        }

        for block in payload
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if block.get("type").and_then(Value::as_str) != Some("input_text") {
                continue;
            }
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                self.set_codex_user_task(text, true);
            }
        }
    }

    fn capture_codex_user_message(&mut self, entry_type: &str, payload: &Value) {
        if entry_type == "event_msg"
            && payload.get("type").and_then(Value::as_str) == Some("user_message")
        {
            if let Some(message) = payload.get("message").and_then(Value::as_str) {
                self.set_codex_user_task(message, false);
            }
        }
    }

    fn capture_codex_usage_response(&mut self, entry_type: &str, payload: &Value) {
        if entry_type != "response" {
            return;
        }
        if let Some(input_tokens) = payload
            .get("usage")
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_u64)
        {
            self.token_count = input_tokens;
        }
    }

    fn capture_codex_token_count(&mut self, entry_type: &str, payload: &Value) {
        if entry_type != "event_msg"
            || payload.get("type").and_then(Value::as_str) != Some("token_count")
        {
            return;
        }

        if let Some(input_tokens) = payload
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
            .and_then(|usage| usage.get("input_tokens"))
            .and_then(Value::as_u64)
        {
            self.token_count = input_tokens;
        }

        let context_window = payload
            .get("model_context_window")
            .and_then(Value::as_u64)
            .or_else(|| {
                payload
                    .get("info")
                    .and_then(|info| info.get("model_context_window"))
                    .and_then(Value::as_u64)
            });

        if let Some(limit) = context_window.filter(|limit| *limit > 0) {
            self.context_limit = limit;
        }
    }

    fn capture_codex_function_call(&mut self, entry_type: &str, payload: &Value) {
        if entry_type != "response_item"
            || payload.get("type").and_then(Value::as_str) != Some("function_call")
        {
            return;
        }

        let action = AgentAction {
            tool: payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            detail: payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(parse_codex_function_call_detail),
        };
        self.record_codex_action(action, true);
    }

    fn capture_codex_agent_reasoning(&mut self, entry_type: &str, payload: &Value) {
        if entry_type == "event_msg"
            && payload.get("type").and_then(Value::as_str) == Some("agent_reasoning")
        {
            if let Some(text) = payload.get("text").and_then(Value::as_str) {
                self.set_codex_thinking(text);
            }
        }
    }

    fn capture_codex_reasoning_summary(&mut self, entry_type: &str, payload: &Value) {
        if entry_type != "response_item"
            || payload.get("type").and_then(Value::as_str) != Some("reasoning")
        {
            return;
        }

        for summary in payload
            .get("summary")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if summary.get("type").and_then(Value::as_str) != Some("summary_text") {
                continue;
            }
            if let Some(text) = summary.get("text").and_then(Value::as_str) {
                self.set_codex_thinking(text);
            }
        }
    }

    fn set_codex_user_task(&mut self, text: &str, skip_xml_like: bool) {
        let trimmed = text.trim();
        let looks_like_system_prompt = skip_xml_like && trimmed.starts_with('<');
        if trimmed.is_empty() || trimmed.len() >= 1000 || looks_like_system_prompt {
            return;
        }
        self.user_task = Some(truncate(trimmed, 300));
    }

    fn set_codex_thinking(&mut self, text: &str) {
        self.current_tool = Some(AgentAction {
            tool: "thinking".to_string(),
            detail: Some(truncate(text, 100)),
        });
    }

    fn record_codex_action(&mut self, action: AgentAction, set_current_tool: bool) {
        self.recent_actions.push(action.clone());
        cap_actions(&mut self.recent_actions, 10);
        if set_current_tool {
            self.current_tool = Some(action);
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
            self.token_count = 0;
            self.context_limit = crate::types::context_limit_for_tool(Some("Codex"));
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
            context_limit: self.context_limit,
        })
    }

    fn claimed_path(&self) -> Option<PathBuf> {
        self.file_path.clone()
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

fn parse_codex_function_call_detail(args_str: &str) -> Option<String> {
    serde_json::from_str::<Value>(args_str)
        .ok()
        .and_then(|args| {
            args.get("command")
                .and_then(Value::as_str)
                .map(|command| truncate(command, 80))
                .or_else(|| {
                    args.get("file_path")
                        .and_then(Value::as_str)
                        .map(|file_path| basename(file_path).to_string())
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

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
        assert!(context_reader_for("Claude Code", "/tmp", &[]).is_some());
        assert!(context_reader_for("Codex", "/tmp", &[]).is_some());
        assert!(context_reader_for("Unknown", "/tmp", &[]).is_none());
    }

    #[test]
    fn codex_reader_consumes_token_count_event_and_context_window() {
        let mut reader = CodexReader::new("/tmp", &[]);
        let entries = vec![serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": { "input_tokens": 99_735_u64 }
                },
                "model_context_window": 258_400_u64
            }
        })];

        reader.parse_entries(&entries);

        assert_eq!(reader.token_count, 99_735);
        assert_eq!(reader.context_limit, 258_400);
    }

    #[test]
    fn codex_reader_keeps_previous_context_limit_when_event_lacks_window() {
        let mut reader = CodexReader::new("/tmp", &[]);
        let default_limit = reader.context_limit;
        let entries = vec![serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": { "input_tokens": 12_345_u64 }
                }
            }
        })];

        reader.parse_entries(&entries);

        assert_eq!(reader.token_count, 12_345);
        assert_eq!(reader.context_limit, default_limit);
    }

    #[test]
    fn claude_reader_discovery_filters_slug_collision_by_exact_cwd() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");

        let cwd_a = "/tmp/a-b/c";
        let cwd_b = "/tmp/a/b-c";
        let slug_a = cwd_a.replace('/', "-");
        let slug_b = cwd_b.replace('/', "-");
        assert_eq!(slug_a, slug_b, "test requires slug collision");

        let project_dir = tmp.path().join(".claude").join("projects").join(slug_a);
        fs::create_dir_all(&project_dir).expect("mkdir");

        let file_a = project_dir.join("session-a.jsonl");
        fs::write(
            &file_a,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"TASK_A\"}}}}\n",
                cwd_a
            ),
        )
        .expect("write file a");
        thread::sleep(Duration::from_millis(50));

        let file_b = project_dir.join("session-b.jsonl");
        fs::write(
            &file_b,
            format!(
                "{{\"type\":\"user\",\"cwd\":\"{}\",\"message\":{{\"role\":\"user\",\"content\":\"TASK_B\"}}}}\n",
                cwd_b
            ),
        )
        .expect("write file b");

        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());

        let reader = ClaudeCodeReader::new(cwd_a, &[]);
        let discovered = reader.discover_file();
        assert_eq!(discovered, Some(file_a));

        if let Some(prev) = previous_home {
            std::env::set_var("HOME", prev);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn claude_reader_read_bootstraps_and_then_reads_incremental_updates() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = "/tmp/project-alpha";
        let slug = cwd.replace('/', "-");
        let project_dir = tmp.path().join(".claude").join("projects").join(slug);
        fs::create_dir_all(&project_dir).expect("project dir");
        let session_file = project_dir.join("session.jsonl");
        fs::write(
            &session_file,
            format!(
                concat!(
                    "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"investigate startup\"}}}}\n",
                    "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input_tokens\":321}},\"content\":[{{\"type\":\"tool_use\",\"name\":\"exec\",\"input\":{{\"cmd\":\"ls\"}}}}]}}}}\n"
                ),
                cwd = cwd
            ),
        )
        .expect("session file");

        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());

        let mut reader = ClaudeCodeReader::new(cwd, &[]);
        let first = reader.read().expect("bootstrap snapshot");
        assert_eq!(first.user_task.as_deref(), Some("investigate startup"));
        assert_eq!(first.token_count, 321);
        assert_eq!(first.current_tool.as_ref().map(|tool| tool.tool.as_str()), Some("exec"));
        assert!(reader.read().is_none(), "no new data should yield None");

        fs::write(
            &session_file,
            format!(
                concat!(
                    "{{\"type\":\"user\",\"cwd\":\"{cwd}\",\"message\":{{\"role\":\"user\",\"content\":\"investigate startup\"}}}}\n",
                    "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"usage\":{{\"input_tokens\":321}},\"content\":[{{\"type\":\"tool_use\",\"name\":\"exec\",\"input\":{{\"cmd\":\"ls\"}}}}]}}}}\n",
                    "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"done reading logs\"}}]}}}}\n"
                ),
                cwd = cwd
            ),
        )
        .expect("append assistant line");

        let second = reader.read().expect("incremental snapshot");
        assert_eq!(second.user_task.as_deref(), Some("investigate startup"));
        assert!(
            second
                .recent_actions
                .iter()
                .any(|action| action.tool == "said"),
            "incremental assistant text should be recorded"
        );

        if let Some(prev) = previous_home {
            std::env::set_var("HOME", prev);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn codex_reader_read_discovers_matching_rollout_and_tracks_incremental_usage() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let sessions_dir = tmp.path().join(".codex").join("sessions").join("2026").join("03").join("16");
        fs::create_dir_all(&sessions_dir).expect("sessions dir");

        let other = sessions_dir.join("rollout-other.jsonl");
        fs::write(
            &other,
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/other\"}}\n",
        )
        .expect("other rollout");

        let target = sessions_dir.join("rollout-target.jsonl");
        fs::write(
            &target,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fix websocket bug\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"cmd\\\":\\\"git status\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":555}},\"model_context_window\":258400}}\n"
            ),
        )
        .expect("target rollout");

        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());

        let mut reader = CodexReader::new("/tmp/project", &[]);
        let first = reader.read().expect("bootstrap snapshot");
        assert_eq!(first.user_task.as_deref(), Some("fix websocket bug"));
        assert_eq!(first.token_count, 555);
        assert_eq!(first.context_limit, 258_400);
        assert_eq!(first.current_tool.as_ref().map(|tool| tool.tool.as_str()), Some("exec"));

        fs::write(
            &target,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fix websocket bug\"}]}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec\",\"arguments\":\"{\\\"cmd\\\":\\\"git status\\\"}\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"input_tokens\":555}},\"model_context_window\":258400}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"tighten the retry path\"}}\n"
            ),
        )
        .expect("updated rollout");

        let second = reader.read().expect("incremental snapshot");
        assert_eq!(second.user_task.as_deref(), Some("tighten the retry path"));
        assert!(reader.read().is_none(), "steady state should not re-emit snapshot");

        if let Some(prev) = previous_home {
            std::env::set_var("HOME", prev);
        } else {
            std::env::remove_var("HOME");
        }
    }
}
