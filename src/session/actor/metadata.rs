use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::Utc;
use tracing::debug;

use crate::state::detector::StateDetector;
use crate::tmux_target::exact_pane_target;
use crate::types::{ControlEvent, SessionState, SessionTitlePayload};

use super::percent_decode::percent_decode;
use super::process_tree::{current_command_tool_update, query_tool_from_tmux_process_tree};
use super::{run_bounded_tmux_command, SessionActor};

pub(super) const CWD_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(750);
pub(super) const TOOL_REFRESH_MIN_INTERVAL: Duration = Duration::from_millis(1_000);
const TMUX_DISPLAY_MESSAGE_TIMEOUT: Duration = Duration::from_millis(500);

impl SessionActor {
    /// Detect OSC title and CWD sequences in raw PTY output.
    ///
    /// OSC 0: `\x1b]0;title\x07` -- set window title + icon name
    /// OSC 2: `\x1b]2;title\x07` -- set window title
    /// OSC 7: `\x1b]7;file://host/path\x07` -- set working directory
    ///
    /// Emits `session_title` ControlEvents and updates internal cwd state.
    pub(super) fn detect_and_emit_title(&mut self, raw: &[u8]) {
        let text = String::from_utf8_lossy(raw);
        self.apply_osc7_payloads(&text);
        self.apply_title_payloads(&text);
    }

    pub(super) fn apply_osc7_payloads(&mut self, text: &str) {
        for cwd in osc7_cwd_update_plan(&self.cwd, text) {
            self.apply_cwd_update(cwd);
        }
    }

    fn apply_title_payloads(&mut self, text: &str) {
        for title in osc_payloads(text, "\x1b]0;")
            .into_iter()
            .chain(osc_payloads(text, "\x1b]2;"))
        {
            self.apply_title_payload(title);
        }
    }

    fn apply_title_payload(&mut self, title: &str) {
        if title.is_empty() {
            return;
        }
        self.update_cwd_from_title(title);
        self.update_tool_from_title(title);
        self.emit_title_event(title);
    }

    pub(super) async fn maybe_refresh_cwd_from_tmux(&mut self, force: bool) {
        if !should_refresh_cwd_from_tmux(
            force,
            self.state_detector.state(),
            self.last_cwd_refresh_at,
            Instant::now(),
        ) {
            return;
        }
        self.last_cwd_refresh_at = Instant::now();

        let tmux_name = self.tmux_name.clone();
        match query_tmux_cwd(&tmux_name).await {
            Ok(cwd) => self.update_cwd_and_emit(cwd),
            Err(e) => {
                debug!(
                    session_id = %self.session_id,
                    tmux_name = %tmux_name,
                    "tmux cwd refresh failed: {}",
                    e
                );
            }
        }
    }

    pub(super) fn maybe_update_tool_from_current_command(&mut self) {
        let current_command = self.state_detector.current_command();
        let Some(tool) =
            current_command_tool_update(current_command.as_deref(), self.tool.as_deref())
        else {
            return;
        };

        self.tool = Some(tool.to_string());
        self.state_detector.set_tui_tool_mode(true);
    }

    pub(super) async fn maybe_refresh_tool_from_tmux(&mut self, force: bool) {
        let now = Instant::now();
        if !self.should_refresh_tool_from_tmux_at(force, now) {
            return;
        }

        self.last_tool_refresh_at = now;

        let tmux_name = self.tmux_name.clone();
        let result = query_tool_from_tmux_process_tree(&tmux_name).await;
        self.apply_tmux_tool_refresh_result(&tmux_name, result);
    }

    pub(super) fn should_refresh_tool_from_tmux_at(&self, force: bool, now: Instant) -> bool {
        should_refresh_tool_from_tmux(
            force,
            self.state_detector.state(),
            self.tool.as_deref(),
            self.last_tool_refresh_at,
            now,
        )
    }

    pub(super) fn apply_tmux_tool_refresh_result(
        &mut self,
        tmux_name: &str,
        result: anyhow::Result<Option<String>>,
    ) {
        match result {
            Ok(Some(tool)) => self.apply_detected_tmux_tool(tool),
            Ok(None) => {}
            Err(e) => self.log_tool_refresh_failure(tmux_name, e),
        }
    }

    fn apply_detected_tmux_tool(&mut self, tool: String) {
        if !tool_refresh_changes_tool(self.tool.as_deref(), &tool) {
            return;
        }

        self.tool = Some(tool);
        self.state_detector.set_tui_tool_mode(true);
    }

    fn log_tool_refresh_failure(&self, tmux_name: &str, error: anyhow::Error) {
        debug!(
            session_id = %self.session_id,
            tmux_name,
            "tmux tool refresh failed: {}",
            error
        );
    }

    pub(super) fn update_cwd_and_emit(&mut self, cwd: String) {
        let _ = cwd_update(&self.cwd, &cwd).map(|cwd| self.apply_cwd_update(cwd));
    }

    pub(super) fn update_cwd_from_title(&mut self, title: &str) {
        let _ = title_cwd_update(&self.cwd, title).map(|cwd| self.cwd = cwd);
    }

    pub(super) fn update_tool_from_title(&mut self, title: &str) {
        let _ = title_tool_update(self.tool.as_deref(), title)
            .map(|tool| self.apply_detected_tool_from_title(tool));
    }

    fn apply_cwd_update(&mut self, cwd: String) {
        self.cwd = cwd;
        let _ = self
            .event_tx
            .send(build_title_event(&self.session_id, self.cwd.clone()));
    }

    fn apply_detected_tool_from_title(&mut self, tool: String) {
        self.tool = Some(tool);
        self.state_detector.set_tui_tool_mode(true);
    }

    fn emit_title_event(&self, title: &str) {
        let _ = self
            .event_tx
            .send(build_title_event(&self.session_id, title.to_string()));
    }
}

pub(super) fn state_detector_for_initial_tool(initial_tool: Option<&str>) -> StateDetector {
    let mut detector = StateDetector::new();
    if initial_tool
        .and_then(crate::types::detect_tool_name)
        .is_some()
    {
        detector.set_tui_tool_mode(true);
    }
    detector
}

pub(super) fn cwd_update(current_cwd: &str, candidate: &str) -> Option<String> {
    let normalized = non_empty_trimmed_cwd(candidate)?;
    changed_cwd(current_cwd, normalized).map(str::to_string)
}

fn non_empty_trimmed_cwd(candidate: &str) -> Option<&str> {
    let normalized = candidate.trim();
    (!normalized.is_empty()).then_some(normalized)
}

fn changed_cwd<'a>(current_cwd: &str, candidate: &'a str) -> Option<&'a str> {
    (candidate != current_cwd).then_some(candidate)
}

fn build_title_event(session_id: &str, title: String) -> ControlEvent {
    let payload = SessionTitlePayload {
        title,
        at: Utc::now(),
    };
    ControlEvent {
        event: "session_title".to_string(),
        session_id: session_id.to_string(),
        payload: serde_json::to_value(&payload).unwrap_or_default(),
    }
}

pub(super) fn title_cwd_update(current_cwd: &str, title: &str) -> Option<String> {
    current_cwd
        .is_empty()
        .then(|| extract_cwd_from_title(title))
        .flatten()
}

pub(super) fn osc7_cwd_update_plan(current_cwd: &str, text: &str) -> Vec<String> {
    let mut planned_cwd = current_cwd.to_string();
    let mut updates = Vec::new();

    for payload in osc_payloads(text, "\x1b]7;") {
        let Some(candidate) = cwd_from_osc7_payload(payload) else {
            continue;
        };
        let Some(cwd) = cwd_update(&planned_cwd, &candidate) else {
            continue;
        };
        planned_cwd = cwd.clone();
        updates.push(cwd);
    }

    updates
}

pub(super) fn title_tool_update(current_tool: Option<&str>, title: &str) -> Option<String> {
    current_tool
        .is_none()
        .then(|| detect_tool_from_title(title))
        .flatten()
}

pub(super) fn should_refresh_cwd_from_tmux(
    force: bool,
    state: SessionState,
    last_refresh_at: Instant,
    now: Instant,
) -> bool {
    force
        || (state == SessionState::Idle
            && now.duration_since(last_refresh_at) >= CWD_REFRESH_MIN_INTERVAL)
}

pub(super) fn should_refresh_tool_from_tmux(
    force: bool,
    state: SessionState,
    tool: Option<&str>,
    last_refresh_at: Instant,
    now: Instant,
) -> bool {
    if force {
        return true;
    }

    if now.duration_since(last_refresh_at) < TOOL_REFRESH_MIN_INTERVAL {
        return false;
    }

    !(tool.is_some() && state == SessionState::Idle)
}

pub(super) fn tool_refresh_changes_tool(current_tool: Option<&str>, detected_tool: &str) -> bool {
    current_tool != Some(detected_tool)
}

pub(super) async fn query_tmux_display_message(
    tmux_name: &str,
    format: &str,
) -> anyhow::Result<String> {
    let target = exact_pane_target(tmux_name);
    let output = run_bounded_tmux_command(
        "tmux",
        &["display-message", "-p", "-t", &target, format],
        TMUX_DISPLAY_MESSAGE_TIMEOUT,
        "display-message",
    )
    .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "tmux display-message failed: {}",
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn query_tmux_cwd(tmux_name: &str) -> anyhow::Result<String> {
    let cwd = query_tmux_display_message(tmux_name, "#{pane_current_path}").await?;
    if cwd.is_empty() {
        return Err(anyhow::anyhow!("tmux returned empty pane_current_path"));
    }
    Ok(cwd)
}

pub(super) fn osc_payloads<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
    let mut payloads = Vec::new();
    let mut search_from = 0;

    while let Some(start) = text[search_from..].find(prefix) {
        let payload_start = search_from + start + prefix.len();
        let Some((end_offset, terminator_len)) = find_osc_payload_end(&text[payload_start..])
        else {
            break;
        };
        payloads.push(&text[payload_start..payload_start + end_offset]);
        search_from = payload_start + end_offset + terminator_len;
    }

    payloads
}

pub(super) fn find_osc_payload_end(text: &str) -> Option<(usize, usize)> {
    [
        text.find('\x07').map(|offset| (offset, 1)),
        text.find("\x1b\\").map(|offset| (offset, 2)),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|(offset, _)| *offset)
}

pub(super) fn cwd_from_osc7_payload(payload: &str) -> Option<String> {
    cwd_from_osc7_payload_with_local_hosts(payload, local_osc7_host_aliases())
}

pub(super) fn cwd_from_osc7_payload_with_local_hosts(
    payload: &str,
    local_hosts: &BTreeSet<String>,
) -> Option<String> {
    let file_uri = parse_osc7_file_uri(payload)?;
    let path = percent_decode(file_uri.path);
    let Some(host) = file_uri.host else {
        return Some(path);
    };

    let normalized_host = normalize_osc7_host(host)?;
    if osc7_host_is_local(&normalized_host, local_hosts) {
        Some(path)
    } else {
        Some(format!("{normalized_host}:{path}"))
    }
}

struct Osc7FileUri<'a> {
    host: Option<&'a str>,
    path: &'a str,
}

fn parse_osc7_file_uri(payload: &str) -> Option<Osc7FileUri<'_>> {
    let body = payload.strip_prefix("file://")?;
    if body.starts_with('/') {
        return Some(Osc7FileUri {
            host: None,
            path: body,
        });
    }

    let slash_pos = body.find('/')?;
    let (host, path) = body.split_at(slash_pos);
    (!path.is_empty()).then_some(Osc7FileUri {
        host: Some(host),
        path,
    })
}

fn osc7_host_is_local(normalized_host: &str, local_hosts: &BTreeSet<String>) -> bool {
    matches!(normalized_host, "localhost" | "127.0.0.1" | "::1")
        || local_hosts.contains(normalized_host)
}

fn local_osc7_host_aliases() -> &'static BTreeSet<String> {
    static LOCAL_HOST_ALIASES: OnceLock<BTreeSet<String>> = OnceLock::new();
    LOCAL_HOST_ALIASES.get_or_init(compute_local_osc7_host_aliases)
}

fn compute_local_osc7_host_aliases() -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    for key in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(value) = std::env::var(key) {
            insert_osc7_host_aliases(&mut aliases, &value);
        }
    }
    if let Some(hostname) = system_hostname() {
        insert_osc7_host_aliases(&mut aliases, &hostname);
    }
    aliases
}

fn insert_osc7_host_aliases(aliases: &mut BTreeSet<String>, host: &str) {
    let Some(normalized) = normalize_osc7_host(host) else {
        return;
    };
    aliases.insert(normalized.clone());
    if let Some(short) = normalized.split('.').next() {
        if !short.is_empty() {
            aliases.insert(short.to_string());
        }
    }
}

fn normalize_osc7_host(host: &str) -> Option<String> {
    let host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

#[cfg(unix)]
fn system_hostname() -> Option<String> {
    let mut buffer = [0_u8; 256];
    let rc = unsafe { libc::gethostname(buffer.as_mut_ptr().cast::<libc::c_char>(), buffer.len()) };
    if rc != 0 {
        return None;
    }
    let len = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    String::from_utf8(buffer[..len].to_vec()).ok()
}

#[cfg(not(unix))]
fn system_hostname() -> Option<String> {
    None
}

/// Try to extract a cwd path from an OSC 0/2 window title.
/// Common formats: "user@host: /path", "user@host:/path", "/path/to/dir"
pub(super) fn extract_cwd_from_title(title: &str) -> Option<String> {
    title_prefixed_cwd(title)
        .or_else(|| title_absolute_cwd(title))
        .or_else(|| title_home_cwd(title))
}

fn title_prefixed_cwd(title: &str) -> Option<String> {
    title
        .find(": /")
        .map(|pos| pos + 2)
        .or_else(|| title.find(":/").map(|pos| pos + 1))
        .and_then(|path_start| non_blank_trimmed(title.get(path_start..)?))
        .map(str::to_string)
}

fn title_absolute_cwd(title: &str) -> Option<String> {
    title.starts_with('/').then(|| title.trim().to_string())
}

fn title_home_cwd(title: &str) -> Option<String> {
    title.starts_with('~').then(|| expand_home_title(title))
}

fn expand_home_title(title: &str) -> String {
    std::env::var("HOME")
        .map(|home| title.replacen('~', &home, 1))
        .unwrap_or_else(|_| title.trim().to_string())
}

fn non_blank_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

/// Detect a coding tool name from the window title.
fn detect_tool_from_title(title: &str) -> Option<String> {
    let lower = title.to_lowercase();
    // Check for known tool process names in the title
    for (pattern, name) in &[
        ("claude", "Claude Code"),
        ("codex", "Codex"),
        ("grok", "Grok"),
        ("aider", "Aider"),
        ("goose", "Goose"),
        ("cline", "Cline"),
    ] {
        if lower.contains(pattern) {
            return Some(name.to_string());
        }
    }
    None
}
