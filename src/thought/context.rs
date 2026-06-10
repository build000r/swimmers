//! Port of context-reader.js — reads structured agent JSONL files for
//! context-aware thought generation.
//!
//! All file I/O in this module is blocking (`std::fs`). Callers must run
//! reads from `spawn_blocking` to avoid stalling the async runtime.

#![allow(dead_code)]

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
    pub user_turns: Vec<AgentUserTurn>,
    pub transcript_records: Vec<AgentTranscriptRecord>,
    pub source_size: u64,
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

/// A user-submitted turn found in an agent JSONL log.
#[derive(Debug, Clone)]
pub struct AgentUserTurn {
    pub id: String,
    pub source: String,
    pub text: String,
    pub byte_start: u64,
    pub byte_end: u64,
    pub order: u64,
    pub timestamp: Option<String>,
}

/// A byte-addressed JSONL record in the agent transcript.
#[derive(Debug, Clone)]
pub struct AgentTranscriptRecord {
    pub id: String,
    pub source: String,
    pub kind: String,
    pub role: Option<String>,
    pub summary: String,
    pub raw: String,
    pub byte_start: u64,
    pub byte_end: u64,
    pub timestamp: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
struct JsonlEntry {
    value: Value,
    raw: String,
    byte_start: u64,
    byte_end: u64,
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

const BOOTSTRAP_MAX: u64 = 16 * 1024 * 1024; // 16 MB
const USER_TURN_TEXT_MAX_CHARS: usize = 4000;
const TRANSCRIPT_SUMMARY_MAX_CHARS: usize = 240;
const TRANSCRIPT_RAW_MAX_CHARS: usize = 4000;

/// Read a byte range `[start, end)` from a file.
///
/// Returns an empty buffer when `end <= start` — this guards against the
/// caller passing a reversed range (e.g. when a file has been truncated since
/// the last read), which would otherwise underflow the `end - start`
/// subtraction and trigger a panic or a multi-exabyte allocation request.
fn read_range(path: &Path, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};

    if end <= start {
        return Ok(Vec::new());
    }

    let mut f = fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let len = (end - start) as usize;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Parse JSONL lines from a byte buffer, skipping malformed lines.
fn parse_jsonl_lines(buf: &[u8]) -> Vec<Value> {
    parse_jsonl_entries(buf, 0)
        .into_iter()
        .map(|entry| entry.value)
        .collect()
}

enum JsonlSegmentParse {
    Entry(JsonlEntry),
    Skip { consumed_offset: Option<u64> },
    Stop,
}

fn parse_jsonl_segment(
    segment: &[u8],
    base_offset: u64,
    segment_start: usize,
    segment_end: usize,
) -> JsonlSegmentParse {
    let complete_line = segment.ends_with(b"\n");
    let mut line = segment;
    if complete_line {
        line = &line[..line.len().saturating_sub(1)];
    }
    if line.ends_with(b"\r") {
        line = &line[..line.len().saturating_sub(1)];
    }

    let line_end_offset = base_offset + segment_end as u64;
    if line.is_empty() {
        return JsonlSegmentParse::Skip {
            consumed_offset: complete_line.then_some(line_end_offset),
        };
    }

    let raw = String::from_utf8_lossy(line).to_string();
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) => JsonlSegmentParse::Entry(JsonlEntry {
            value,
            raw,
            byte_start: base_offset + segment_start as u64,
            byte_end: line_end_offset,
        }),
        Err(_) if complete_line => JsonlSegmentParse::Skip {
            consumed_offset: Some(line_end_offset),
        },
        Err(_) => JsonlSegmentParse::Stop,
    }
}

fn parse_jsonl_entries_and_offset(buf: &[u8], base_offset: u64) -> (Vec<JsonlEntry>, u64) {
    let mut entries = Vec::new();
    let mut cursor = 0usize;
    let mut consumed_offset = base_offset;

    for segment in buf.split_inclusive(|byte| *byte == b'\n') {
        let segment_start = cursor;
        cursor += segment.len();

        match parse_jsonl_segment(segment, base_offset, segment_start, cursor) {
            JsonlSegmentParse::Entry(entry) => {
                consumed_offset = entry.byte_end;
                entries.push(entry);
            }
            JsonlSegmentParse::Skip {
                consumed_offset: Some(offset),
            } => consumed_offset = offset,
            JsonlSegmentParse::Skip {
                consumed_offset: None,
            } => {}
            JsonlSegmentParse::Stop => break,
        }
    }

    (entries, consumed_offset)
}

/// Parse JSONL lines and preserve byte offsets relative to the original file.
fn parse_jsonl_entries(buf: &[u8], base_offset: u64) -> Vec<JsonlEntry> {
    parse_jsonl_entries_and_offset(buf, base_offset).0
}

/// Extract the basename from a path string (last component after `/`).
fn basename(path_str: &str) -> &str {
    path_str.rsplit('/').next().unwrap_or(path_str)
}

fn truncate_with_flag(value: &str, max_chars: usize) -> (String, bool) {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        (format!("{truncated}..."), true)
    } else {
        (truncated, false)
    }
}

fn normalized_user_turn_text(text: &str, skip_xml_like: bool) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if skip_xml_like && trimmed.starts_with('<') {
        return None;
    }
    let (text, _) = truncate_with_flag(trimmed, USER_TURN_TEXT_MAX_CHARS);
    Some(text)
}

fn entry_timestamp(entry: &Value) -> Option<String> {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|timestamp| !timestamp.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn value_field_str<'a>(value: Option<&'a Value>, field: &str) -> Option<&'a str> {
    value
        .and_then(|value| value.get(field))
        .and_then(Value::as_str)
}

fn role_message_kind(value: &Value) -> Option<String> {
    value_field_str(Some(value), "role").map(|role| format!("{role}_message"))
}

fn payload_record_kind(entry_type: &str, payload: Option<&Value>) -> Option<String> {
    let payload = payload?;
    match entry_type {
        "response_item" => value_field_str(Some(payload), "type")
            .map(ToOwned::to_owned)
            .or_else(|| role_message_kind(payload)),
        "event_msg" => value_field_str(Some(payload), "type").map(ToOwned::to_owned),
        _ => None,
    }
}

fn transcript_record_kind(
    entry_type: &str,
    payload: Option<&Value>,
    message: Option<&Value>,
) -> String {
    payload_record_kind(entry_type, payload)
        .or_else(|| message.and_then(role_message_kind))
        .unwrap_or_else(|| entry_type.to_string())
}

fn transcript_record_role(payload: Option<&Value>, message: Option<&Value>) -> Option<String> {
    payload
        .and_then(|payload| payload.get("role"))
        .and_then(Value::as_str)
        .or_else(|| {
            message
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)
}

fn transcript_record_summary(
    entry_type: &str,
    payload: Option<&Value>,
    message: Option<&Value>,
    raw: &str,
) -> String {
    let text = if entry_type == "response_item" {
        response_item_summary(payload)
    } else if entry_type == "event_msg" {
        event_msg_summary(payload)
    } else {
        claude_message_summary(message)
    }
    .unwrap_or_else(|| raw.to_string());

    let normalized = text.replace('\r', "").replace('\n', " ");
    truncate(normalized.trim(), TRANSCRIPT_SUMMARY_MAX_CHARS)
}

fn response_item_summary(payload: Option<&Value>) -> Option<String> {
    let payload = payload?;
    if payload.get("type").and_then(Value::as_str) == Some("function_call") {
        let name = payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("function_call");
        let detail = payload
            .get("arguments")
            .and_then(Value::as_str)
            .and_then(parse_codex_function_call_detail);
        return Some(match detail {
            Some(detail) => format!("{name}: {detail}"),
            None => name.to_string(),
        });
    }

    let content = payload.get("content")?;
    content_text(content)
}

fn event_msg_summary(payload: Option<&Value>) -> Option<String> {
    let payload = payload?;
    payload
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| payload.get("text").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .get("type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn claude_message_summary(message: Option<&Value>) -> Option<String> {
    let message = message?;
    content_text(message.get("content")?)
}

fn claude_user_message_text<'a>(entry_type: &str, msg: Option<&'a Value>) -> Option<&'a str> {
    let msg = msg?;
    if entry_type != "user" {
        return None;
    }
    if msg.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    claude_user_content_text(msg.get("content")?)
}

fn claude_user_content_text(content: &Value) -> Option<&str> {
    if let Some(text) = content.as_str() {
        return Some(text);
    }

    content
        .as_array()?
        .iter()
        .find_map(claude_user_text_block_text)
}

fn claude_user_text_block_text(block: &Value) -> Option<&str> {
    if block.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }
    block.get("text").and_then(Value::as_str)
}

fn content_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    let parts = content
        .as_array()?
        .iter()
        .filter_map(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| block.get("content").and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();

    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn make_turn_id(source: &str, byte_start: u64) -> String {
    format!("{source}-turn-{byte_start}")
}

fn make_record_id(source: &str, byte_start: u64) -> String {
    format!("{source}-record-{byte_start}")
}

/// Claude JSONL can contain multiple record kinds; scan a small prefix for
/// top-level `cwd` fields and require an exact match when present.
fn claude_file_matches_cwd(path: &Path, cwd: &str) -> bool {
    use std::io::BufRead;

    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let reader = std::io::BufReader::new(file);

    scan_claude_cwd_prefix(reader.lines().take(64), cwd).matches()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaudeCwdScan {
    Match,
    Mismatch,
    Missing,
}

impl ClaudeCwdScan {
    fn matches(self) -> bool {
        matches!(self, Self::Match | Self::Missing)
    }
}

fn scan_claude_cwd_prefix<I>(lines: I, cwd: &str) -> ClaudeCwdScan
where
    I: IntoIterator<Item = std::io::Result<String>>,
{
    let mut saw_cwd_field = false;
    for line in lines {
        let Some(entry_cwd) = claude_line_cwd(line) else {
            continue;
        };
        saw_cwd_field = true;
        if entry_cwd == cwd {
            return ClaudeCwdScan::Match;
        }
    }

    if saw_cwd_field {
        ClaudeCwdScan::Mismatch
    } else {
        // Legacy files may omit top-level cwd metadata; preserve old behavior.
        ClaudeCwdScan::Missing
    }
}

fn claude_line_cwd(line: std::io::Result<String>) -> Option<String> {
    let line = line.ok()?;
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    serde_json::from_str::<Value>(line)
        .ok()?
        .get("cwd")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn claude_project_dir(cwd: &str) -> Option<PathBuf> {
    let cwd_slug = cwd.replace('/', "-");
    dirs_home().map(|home| home.join(".claude").join("projects").join(cwd_slug))
}

fn discover_latest_claude_jsonl(
    project_dir: &Path,
    cwd: &str,
    excluded_paths: &[PathBuf],
) -> Option<PathBuf> {
    let entries = fs::read_dir(project_dir).ok()?;
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = entries
        .filter_map(|entry| claude_discovery_candidate(entry.ok()?, cwd, excluded_paths))
        .collect();

    files.sort_by_key(|item| std::cmp::Reverse(item.1));
    files.into_iter().next().map(|(path, _)| path)
}

fn claude_discovery_candidate(
    entry: fs::DirEntry,
    cwd: &str,
    excluded_paths: &[PathBuf],
) -> Option<(PathBuf, std::time::SystemTime)> {
    let path = entry.path();
    if !is_jsonl_path(&path) {
        return None;
    }
    if !claude_file_matches_cwd(&path, cwd) {
        return None;
    }
    if excluded_paths.contains(&path) {
        return None;
    }

    let mtime = entry.metadata().ok()?.modified().ok()?;
    Some((path, mtime))
}

fn is_jsonl_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "jsonl")
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
    user_turns: Vec<AgentUserTurn>,
    transcript_records: Vec<AgentTranscriptRecord>,
    recent_actions: Vec<AgentAction>,
    current_tool: Option<AgentAction>,
    bootstrapped: bool,
    next_turn_order: u64,
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
            user_turns: Vec::new(),
            transcript_records: Vec::new(),
            recent_actions: Vec::new(),
            current_tool: None,
            bootstrapped: false,
            next_turn_order: 0,
            token_count: 0,
            context_limit: crate::types::context_limit_for_tool(Some("Claude Code")),
            excluded_paths: excluded.to_vec(),
        }
    }

    /// Discover the most recently modified JSONL file in the project dir,
    /// skipping files already claimed by other readers.
    fn discover_file(&self) -> Option<PathBuf> {
        let project_dir = claude_project_dir(&self.cwd)?;
        discover_latest_claude_jsonl(&project_dir, &self.cwd, &self.excluded_paths)
    }

    /// Parse entries and update internal state.
    fn parse_entries(&mut self, entries: &[JsonlEntry]) {
        for entry in entries {
            let entry_type = entry
                .value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("");
            let msg = entry.value.get("message");
            self.record_transcript_entry(entry_type, msg, entry);
            self.capture_claude_user_message(entry_type, msg, entry);
            self.capture_claude_assistant_message(entry_type, msg);
        }
    }

    fn capture_claude_user_message(
        &mut self,
        entry_type: &str,
        msg: Option<&Value>,
        entry: &JsonlEntry,
    ) {
        if let Some(text) = claude_user_message_text(entry_type, msg) {
            self.set_reader_user_task(text, entry);
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

    fn set_reader_user_task(&mut self, text: &str, entry: &JsonlEntry) {
        let Some(normalized) = normalized_user_turn_text(text, false) else {
            return;
        };
        self.user_task = Some(truncate(&normalized, 300));
        self.push_user_turn(normalized, entry);
    }

    fn push_user_turn(&mut self, text: String, entry: &JsonlEntry) {
        if self
            .user_turns
            .iter()
            .any(|turn| turn.byte_start == entry.byte_start)
        {
            return;
        }
        self.next_turn_order += 1;
        self.user_turns.push(AgentUserTurn {
            id: make_turn_id("claude", entry.byte_start),
            source: "Claude Code".to_string(),
            text,
            byte_start: entry.byte_start,
            byte_end: entry.byte_end,
            order: self.next_turn_order,
            timestamp: entry_timestamp(&entry.value),
        });
        cap_turns(&mut self.user_turns, 40);
    }

    fn record_transcript_entry(
        &mut self,
        entry_type: &str,
        msg: Option<&Value>,
        entry: &JsonlEntry,
    ) {
        let (raw, truncated) = truncate_with_flag(&entry.raw, TRANSCRIPT_RAW_MAX_CHARS);
        self.transcript_records.push(AgentTranscriptRecord {
            id: make_record_id("claude", entry.byte_start),
            source: "Claude Code".to_string(),
            kind: transcript_record_kind(entry_type, None, msg),
            role: transcript_record_role(None, msg),
            summary: transcript_record_summary(entry_type, None, msg, &entry.raw),
            raw,
            byte_start: entry.byte_start,
            byte_end: entry.byte_end,
            timestamp: entry_timestamp(&entry.value),
            truncated,
        });
        cap_transcript_records(&mut self.transcript_records, 400);
    }

    fn record_reader_action(&mut self, action: AgentAction, set_current_tool: bool) {
        self.recent_actions.push(action.clone());
        cap_actions(&mut self.recent_actions, 10);
        if set_current_tool {
            self.current_tool = Some(action);
        }
    }

    fn reset_reader_state(&mut self, file_path: PathBuf) {
        self.file_path = Some(file_path);
        self.file_size = 0;
        self.user_task = None;
        self.user_turns.clear();
        self.transcript_records.clear();
        self.recent_actions.clear();
        self.current_tool = None;
        self.bootstrapped = false;
        self.next_turn_order = 0;
        self.token_count = 0;
        self.context_limit = crate::types::context_limit_for_tool(Some("Claude Code"));
    }

    fn snapshot(&self, source_size: u64) -> ContextSnapshot {
        ContextSnapshot {
            user_task: self.user_task.clone(),
            user_turns: self.user_turns.clone(),
            transcript_records: self.transcript_records.clone(),
            source_size,
            recent_actions: last_n(&self.recent_actions, 5),
            current_tool: self.current_tool.clone(),
            token_count: self.token_count,
            context_limit: self.context_limit,
        }
    }
}

impl ContextReader for ClaudeCodeReader {
    fn read(&mut self) -> Option<ContextSnapshot> {
        let (file_path, cleared_claim) =
            resolve_current_log_path(self.file_path.clone(), || self.discover_file())?;
        if cleared_claim {
            self.file_path = None;
        }
        let plan = plan_log_read(
            self.file_path.as_ref(),
            self.file_size,
            self.bootstrapped,
            file_path,
        )?;

        if plan.reset_reader {
            self.reset_reader_state(plan.file_path.clone());
        }

        let (entries, consumed_offset) = read_planned_entries(&plan)?;
        self.file_size = consumed_offset;
        self.parse_entries(&entries);
        if plan.phase == LogReadPhase::Bootstrap {
            self.bootstrapped = true;
        }

        Some(self.snapshot(plan.current_size))
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
    user_turns: Vec<AgentUserTurn>,
    transcript_records: Vec<AgentTranscriptRecord>,
    recent_actions: Vec<AgentAction>,
    current_tool: Option<AgentAction>,
    bootstrapped: bool,
    token_count: u64,
    context_limit: u64,
    next_turn_order: u64,
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
            user_turns: Vec::new(),
            transcript_records: Vec::new(),
            recent_actions: Vec::new(),
            current_tool: None,
            bootstrapped: false,
            token_count: 0,
            context_limit: crate::types::context_limit_for_tool(Some("Codex")),
            next_turn_order: 0,
            excluded_paths: excluded.to_vec(),
        }
    }

    /// Walk `~/.codex/sessions/YYYY/MM/DD/` in reverse chronological order,
    /// returning the first `rollout-*.jsonl` whose `session_meta.cwd` matches.
    fn discover_file(&self) -> Option<PathBuf> {
        let sessions_dir = codex_sessions_dir()?;
        discover_codex_rollout_file(&sessions_dir, &self.cwd, &self.excluded_paths)
    }

    /// Check if the first line of a JSONL file is a `session_meta` entry
    /// whose `cwd` matches ours.
    fn matches_cwd(&self, path: &Path) -> bool {
        codex_file_matches_cwd(path, &self.cwd)
    }

    /// Parse entries and update internal state.
    fn parse_entries(&mut self, entries: &[JsonlEntry]) {
        for entry in entries {
            let entry_type = entry
                .value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("");
            let payload = entry
                .value
                .get("payload")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            self.record_transcript_entry(entry_type, &payload, entry);
            self.capture_codex_response_item_user(entry_type, &payload, entry);
            self.capture_codex_user_message(entry_type, &payload, entry);
            self.capture_codex_usage_response(entry_type, &payload);
            self.capture_codex_token_count(entry_type, &payload);
            self.capture_codex_function_call(entry_type, &payload);
            self.capture_codex_agent_reasoning(entry_type, &payload);
            self.capture_codex_reasoning_summary(entry_type, &payload);
        }
    }

    fn capture_codex_response_item_user(
        &mut self,
        entry_type: &str,
        payload: &Value,
        entry: &JsonlEntry,
    ) {
        if !is_codex_response_item_user(entry_type, payload) {
            return;
        }

        for text in codex_response_item_user_input_texts(payload) {
            self.set_codex_user_task(text, true, entry);
        }
    }

    fn capture_codex_user_message(
        &mut self,
        entry_type: &str,
        payload: &Value,
        entry: &JsonlEntry,
    ) {
        if entry_type == "event_msg"
            && payload.get("type").and_then(Value::as_str) == Some("user_message")
        {
            if let Some(message) = payload.get("message").and_then(Value::as_str) {
                self.set_codex_user_task(message, false, entry);
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
        if !is_codex_reasoning_response_item(entry_type, payload) {
            return;
        }

        for text in codex_reasoning_summary_texts(payload) {
            self.set_codex_thinking(text);
        }
    }

    fn set_codex_user_task(&mut self, text: &str, skip_xml_like: bool, entry: &JsonlEntry) {
        let Some(normalized) = normalized_user_turn_text(text, skip_xml_like) else {
            return;
        };
        self.user_task = Some(truncate(&normalized, 300));
        self.push_user_turn(normalized, entry);
    }

    fn push_user_turn(&mut self, text: String, entry: &JsonlEntry) {
        if self
            .user_turns
            .iter()
            .any(|turn| turn.byte_start == entry.byte_start)
        {
            return;
        }
        self.next_turn_order += 1;
        self.user_turns.push(AgentUserTurn {
            id: make_turn_id("codex", entry.byte_start),
            source: "Codex".to_string(),
            text,
            byte_start: entry.byte_start,
            byte_end: entry.byte_end,
            order: self.next_turn_order,
            timestamp: entry_timestamp(&entry.value),
        });
        cap_turns(&mut self.user_turns, 40);
    }

    fn record_transcript_entry(&mut self, entry_type: &str, payload: &Value, entry: &JsonlEntry) {
        let (raw, truncated) = truncate_with_flag(&entry.raw, TRANSCRIPT_RAW_MAX_CHARS);
        self.transcript_records.push(AgentTranscriptRecord {
            id: make_record_id("codex", entry.byte_start),
            source: "Codex".to_string(),
            kind: transcript_record_kind(entry_type, Some(payload), None),
            role: transcript_record_role(Some(payload), None),
            summary: transcript_record_summary(entry_type, Some(payload), None, &entry.raw),
            raw,
            byte_start: entry.byte_start,
            byte_end: entry.byte_end,
            timestamp: entry_timestamp(&entry.value),
            truncated,
        });
        cap_transcript_records(&mut self.transcript_records, 400);
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

    fn reset_reader_state(&mut self, file_path: PathBuf) {
        self.file_path = Some(file_path);
        self.file_size = 0;
        self.user_task = None;
        self.user_turns.clear();
        self.transcript_records.clear();
        self.recent_actions.clear();
        self.current_tool = None;
        self.bootstrapped = false;
        self.token_count = 0;
        self.context_limit = crate::types::context_limit_for_tool(Some("Codex"));
        self.next_turn_order = 0;
    }

    fn snapshot(&self, source_size: u64) -> ContextSnapshot {
        ContextSnapshot {
            user_task: self.user_task.clone(),
            user_turns: self.user_turns.clone(),
            transcript_records: self.transcript_records.clone(),
            source_size,
            recent_actions: last_n(&self.recent_actions, 5),
            current_tool: self.current_tool.clone(),
            token_count: self.token_count,
            context_limit: self.context_limit,
        }
    }
}

impl ContextReader for CodexReader {
    fn read(&mut self) -> Option<ContextSnapshot> {
        let (file_path, cleared_claim) =
            resolve_current_log_path(self.file_path.clone(), || self.discover_file())?;
        if cleared_claim {
            self.file_path = None;
        }
        let plan = plan_log_read(
            self.file_path.as_ref(),
            self.file_size,
            self.bootstrapped,
            file_path,
        )?;

        if plan.reset_reader {
            self.reset_reader_state(plan.file_path.clone());
        }

        let (entries, consumed_offset) = read_planned_entries(&plan)?;
        self.file_size = consumed_offset;
        self.parse_entries(&entries);
        if plan.phase == LogReadPhase::Bootstrap {
            self.bootstrapped = true;
        }

        Some(self.snapshot(plan.current_size))
    }

    fn claimed_path(&self) -> Option<PathBuf> {
        self.file_path.clone()
    }
}

fn codex_sessions_dir() -> Option<PathBuf> {
    dirs_home().map(|home| home.join(".codex").join("sessions"))
}

fn discover_codex_rollout_file(
    sessions_dir: &Path,
    cwd: &str,
    excluded_paths: &[PathBuf],
) -> Option<PathBuf> {
    for day in codex_session_days_reverse(sessions_dir) {
        for candidate in codex_rollout_files_reverse(&day) {
            if codex_rollout_candidate_matches(&candidate, cwd, excluded_paths) {
                return Some(candidate);
            }
        }
    }
    None
}

fn codex_session_days_reverse(sessions_dir: &Path) -> Vec<PathBuf> {
    sorted_subdirs_reverse(sessions_dir, r"^\d{4}$")
        .into_iter()
        .flatten()
        .flat_map(|year| sorted_subdirs_reverse(&year, r"^\d{2}$").unwrap_or_default())
        .flat_map(|month| sorted_subdirs_reverse(&month, r"^\d{2}$").unwrap_or_default())
        .collect()
}

fn codex_rollout_files_reverse(day: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(day)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| codex_is_rollout_jsonl(path))
        .collect();
    files.sort();
    files.reverse();
    files
}

fn codex_is_rollout_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
}

fn codex_rollout_candidate_matches(path: &Path, cwd: &str, excluded_paths: &[PathBuf]) -> bool {
    !excluded_paths.iter().any(|excluded| excluded == path) && codex_file_matches_cwd(path, cwd)
}

fn codex_file_matches_cwd(path: &Path, cwd: &str) -> bool {
    codex_file_session_meta_cwd(path).is_some_and(|candidate_cwd| candidate_cwd == cwd)
}

fn codex_file_session_meta_cwd(path: &Path) -> Option<String> {
    use std::io::BufRead;

    let file = fs::File::open(path).ok()?;
    let first_line = std::io::BufReader::new(file).lines().next()?.ok()?;
    let entry: Value = serde_json::from_str(&first_line).ok()?;
    (entry.get("type").and_then(Value::as_str) == Some("session_meta"))
        .then(|| {
            entry
                .get("payload")
                .and_then(|payload| payload.get("cwd"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .flatten()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogReadPhase {
    Bootstrap,
    Incremental,
}

impl LogReadPhase {
    fn warning_message(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap read failed",
            Self::Incremental => "incremental read failed",
        }
    }
}

struct LogReadPlan {
    file_path: PathBuf,
    current_size: u64,
    start: u64,
    phase: LogReadPhase,
    reset_reader: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LogReadDecision {
    start: u64,
    phase: LogReadPhase,
    reset_reader: bool,
}

fn resolve_current_log_path(
    claimed_path: Option<PathBuf>,
    discover_file: impl FnOnce() -> Option<PathBuf>,
) -> Option<(PathBuf, bool)> {
    match claimed_path {
        Some(path) if path.exists() => Some((path, false)),
        _ => {
            let discovered = discover_file()?;
            Some((discovered, true))
        }
    }
}

fn plan_log_read(
    claimed_path: Option<&PathBuf>,
    previous_size: u64,
    bootstrapped: bool,
    file_path: PathBuf,
) -> Option<LogReadPlan> {
    let current_size = fs::metadata(&file_path).ok()?.len();
    let same_file = claimed_path == Some(&file_path);
    let decision = log_read_decision(same_file, previous_size, current_size, bootstrapped)?;

    Some(LogReadPlan {
        file_path,
        current_size,
        start: decision.start,
        phase: decision.phase,
        reset_reader: decision.reset_reader,
    })
}

fn log_read_decision(
    same_file: bool,
    previous_size: u64,
    current_size: u64,
    bootstrapped: bool,
) -> Option<LogReadDecision> {
    if is_unchanged_log(same_file, previous_size, current_size) {
        return None;
    }

    let reset_reader = should_reset_log_reader(same_file, previous_size, current_size);
    let phase = log_read_phase(reset_reader, bootstrapped);
    Some(LogReadDecision {
        start: log_read_start(phase, previous_size, current_size),
        phase,
        reset_reader,
    })
}

fn is_unchanged_log(same_file: bool, previous_size: u64, current_size: u64) -> bool {
    same_file && current_size == previous_size
}

fn should_reset_log_reader(same_file: bool, previous_size: u64, current_size: u64) -> bool {
    !same_file || current_size < previous_size
}

fn log_read_phase(reset_reader: bool, bootstrapped: bool) -> LogReadPhase {
    if reset_reader || !bootstrapped {
        LogReadPhase::Bootstrap
    } else {
        LogReadPhase::Incremental
    }
}

fn log_read_start(phase: LogReadPhase, previous_size: u64, current_size: u64) -> u64 {
    match phase {
        LogReadPhase::Bootstrap => current_size.saturating_sub(BOOTSTRAP_MAX),
        LogReadPhase::Incremental => previous_size,
    }
}

fn read_planned_entries(plan: &LogReadPlan) -> Option<(Vec<JsonlEntry>, u64)> {
    match read_range(&plan.file_path, plan.start, plan.current_size) {
        Ok(buf) => Some(parse_jsonl_entries_and_offset(&buf, plan.start)),
        Err(e) => {
            warn!(
                path = %plan.file_path.display(),
                error = %e,
                "{}",
                plan.phase.warning_message()
            );
            None
        }
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
        .filter(|e| e.file_type().ok().is_some_and(|ft| ft.is_dir()))
        .filter(|e| e.file_name().to_str().is_some_and(|name| re.is_match(name)))
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

fn cap_turns(turns: &mut Vec<AgentUserTurn>, cap: usize) {
    if turns.len() > cap {
        let start = turns.len() - cap;
        *turns = turns.split_off(start);
    }
}

fn cap_transcript_records(records: &mut Vec<AgentTranscriptRecord>, cap: usize) {
    if records.len() > cap {
        let start = records.len() - cap;
        *records = records.split_off(start);
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
    } else {
        input
            .get("pattern")
            .and_then(Value::as_str)
            .map(|pat| pat.to_string())
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

fn is_codex_reasoning_response_item(entry_type: &str, payload: &Value) -> bool {
    entry_type == "response_item"
        && payload.get("type").and_then(Value::as_str) == Some("reasoning")
}

fn codex_reasoning_summary_texts(payload: &Value) -> impl Iterator<Item = &str> {
    payload
        .get("summary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(codex_reasoning_summary_text)
}

fn codex_reasoning_summary_text(summary: &Value) -> Option<&str> {
    (summary.get("type").and_then(Value::as_str) == Some("summary_text"))
        .then(|| summary.get("text").and_then(Value::as_str))
        .flatten()
}

fn is_codex_response_item_user(entry_type: &str, payload: &Value) -> bool {
    entry_type == "response_item" && payload.get("role").and_then(Value::as_str) == Some("user")
}

fn codex_response_item_user_input_texts(payload: &Value) -> impl Iterator<Item = &str> {
    payload
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
}

#[cfg(test)]
mod tests;
