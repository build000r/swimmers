use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;

use crate::api::envelope::{
    api_error, error_body, error_body_msg, success_json, INVALID_SKILL_TOOL,
};
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    SessionSkillIssue, SessionSkillListResponse, SessionSkillSummary, SkillListResponse,
    SkillSummary,
};

const MAX_SCAN_DEPTH: usize = 6;
const SBP_SKILLS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone, Copy)]
enum SkillRegistryTool {
    Claude,
    Codex,
}

impl SkillRegistryTool {
    fn from_query(value: Option<&str>) -> Option<Self> {
        let normalized = value?.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    fn wire_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn env_var(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_SKILLS_DIR",
            Self::Codex => "CODEX_SKILLS_DIR",
        }
    }

    fn default_subdir(self) -> &'static str {
        match self {
            Self::Claude => ".claude/skills",
            Self::Codex => ".codex/skills",
        }
    }
}

#[derive(serde::Deserialize)]
struct SkillQuery {
    tool: Option<String>,
}

#[derive(serde::Deserialize)]
struct SessionSkillQuery {
    source: Option<String>,
    q: Option<String>,
}

fn skill_root_for_tool(tool: SkillRegistryTool) -> Option<PathBuf> {
    if let Ok(value) = std::env::var(tool.env_var()) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(tool.default_subdir()))
}

fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    unquoted_body(trimmed).unwrap_or(trimmed).to_string()
}

fn unquoted_body(value: &str) -> Option<&str> {
    value
        .strip_prefix('"')
        .and_then(|body| body.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|body| body.strip_suffix('\''))
        })
        .map(str::trim)
}

#[derive(Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
}

fn parse_skill_md(path: &Path, fallback_name: &str) -> SkillSummary {
    let mut summary = SkillSummary {
        name: fallback_name.trim().to_string(),
        description: None,
    };

    if let Some(frontmatter) = read_skill_frontmatter(path) {
        apply_skill_frontmatter(&mut summary, frontmatter);
    }

    summary
}

fn read_skill_frontmatter(path: &Path) -> Option<SkillFrontmatter> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_skill_frontmatter(&content)
}

fn parse_skill_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    let mut lines = content.lines();
    (lines.next().map(str::trim) == Some("---")).then(|| {
        lines.take_while(|line| line.trim() != "---").fold(
            SkillFrontmatter::default(),
            |mut frontmatter, line| {
                apply_skill_frontmatter_line(&mut frontmatter, line);
                frontmatter
            },
        )
    })
}

fn apply_skill_frontmatter(summary: &mut SkillSummary, frontmatter: SkillFrontmatter) {
    if let Some(name) = frontmatter.name {
        summary.name = name;
    }
    summary.description = frontmatter.description;
}

fn apply_skill_frontmatter_line(frontmatter: &mut SkillFrontmatter, line: &str) {
    let trimmed = line.trim();
    if let Some(name) = parse_skill_frontmatter_value(trimmed, "name:") {
        frontmatter.name = Some(name);
    } else if let Some(description) = parse_skill_frontmatter_value(trimmed, "description:") {
        frontmatter.description = Some(description);
    }
}

fn parse_skill_frontmatter_value(line: &str, prefix: &str) -> Option<String> {
    let parsed = unquote(line.strip_prefix(prefix)?);
    (!parsed.is_empty()).then_some(parsed)
}

enum SkillSource {
    Directory { skill_md: PathBuf, fallback: String },
    Package { path: PathBuf, fallback: String },
}

fn collect_skill_summaries(root: &Path) -> Vec<SkillSummary> {
    let mut summaries = unique_skill_summaries(discover_skill_sources(root));
    sort_skill_summaries(&mut summaries);
    summaries
}

fn discover_skill_sources(root: &Path) -> Vec<SkillSource> {
    let Some(mut stack) = scan_stack_for_root(root) else {
        return Vec::new();
    };

    let mut visited_dirs = HashSet::new();
    let mut sources = Vec::new();

    while let Some((dir, depth)) = stack.pop() {
        let Some(canonical) = canonical_unvisited_dir(&dir, &mut visited_dirs) else {
            continue;
        };

        let Ok(entries) = std::fs::read_dir(&canonical) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            if is_ignored_skill_entry(&entry_name) {
                continue;
            }

            collect_entry_source(path, entry_name, depth, &mut stack, &mut sources);
        }
    }

    sources
}

fn scan_stack_for_root(root: &Path) -> Option<Vec<(PathBuf, usize)>> {
    root.exists().then(|| vec![(root.to_path_buf(), 0)])
}

fn canonical_unvisited_dir(dir: &Path, visited_dirs: &mut HashSet<PathBuf>) -> Option<PathBuf> {
    let canonical = dir.canonicalize().ok()?;
    visited_dirs.insert(canonical.clone()).then_some(canonical)
}

fn is_ignored_skill_entry(entry_name: &str) -> bool {
    entry_name.starts_with('.') && entry_name != ".system"
}

fn collect_entry_source(
    path: PathBuf,
    entry_name: String,
    depth: usize,
    stack: &mut Vec<(PathBuf, usize)>,
    sources: &mut Vec<SkillSource>,
) {
    if path.is_dir() {
        collect_directory_source(path, entry_name, depth, stack, sources);
    } else if is_packaged_skill_path(&path) {
        sources.push(SkillSource::Package {
            path,
            fallback: entry_name,
        });
    }
}

fn collect_directory_source(
    path: PathBuf,
    entry_name: String,
    depth: usize,
    stack: &mut Vec<(PathBuf, usize)>,
    sources: &mut Vec<SkillSource>,
) {
    let skill_md = path.join("SKILL.md");
    if skill_md.is_file() {
        sources.push(SkillSource::Directory {
            skill_md,
            fallback: entry_name,
        });
    } else if depth < MAX_SCAN_DEPTH {
        stack.push((path, depth + 1));
    }
}

fn is_packaged_skill_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("skill"))
        .unwrap_or(false)
}

fn unique_skill_summaries(sources: Vec<SkillSource>) -> Vec<SkillSummary> {
    let mut summaries = Vec::new();
    let mut seen_names = HashSet::new();

    for source in sources {
        push_unique_summary(
            skill_summary_from_source(source),
            &mut seen_names,
            &mut summaries,
        );
    }

    summaries
}

fn skill_summary_from_source(source: SkillSource) -> SkillSummary {
    match source {
        SkillSource::Directory { skill_md, fallback } => parse_skill_md(&skill_md, &fallback),
        SkillSource::Package { path, fallback } => SkillSummary {
            name: packaged_skill_name(&path, fallback),
            description: None,
        },
    }
}

fn packaged_skill_name(path: &Path, fallback: String) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(fallback)
}

fn push_unique_summary(
    summary: SkillSummary,
    seen_names: &mut HashSet<String>,
    summaries: &mut Vec<SkillSummary>,
) {
    if seen_names.insert(summary.name.to_ascii_lowercase()) {
        summaries.push(summary);
    }
}

fn sort_skill_summaries(summaries: &mut [SkillSummary]) {
    summaries.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
}

async fn list_skills(
    Extension(auth): Extension<AuthInfo>,
    State(_state): State<Arc<AppState>>,
    Query(query): Query<SkillQuery>,
) -> impl IntoResponse {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    let Some(tool) = SkillRegistryTool::from_query(query.tool.as_deref()) else {
        return api_error(&INVALID_SKILL_TOOL);
    };

    let root = skill_root_for_tool(tool);
    let skills = root
        .as_deref()
        .map(collect_skill_summaries)
        .unwrap_or_default();

    success_json(
        StatusCode::OK,
        &SkillListResponse {
            tool: tool.wire_name().to_string(),
            skills,
        },
    )
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/skills", get(list_skills))
        .route("/v1/sessions/{session_id}/skills", get(list_session_skills))
}

async fn list_session_skills(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    Query(query): Query<SessionSkillQuery>,
) -> Response {
    if let Some(response) = session_skills_preflight_response(&auth, &session_id, &query) {
        return response;
    }

    local_session_skills_response(state, session_id, query.q).await
}

fn session_skills_preflight_response(
    auth: &AuthInfo,
    session_id: &str,
    query: &SessionSkillQuery,
) -> Option<Response> {
    auth.require_scope(AuthScope::SessionsRead)
        .err()
        .or_else(|| unsupported_session_skill_source_response(query.source.as_deref()))
        .or_else(|| remote_session_skills_response(session_id, query.q.clone()))
}

fn unsupported_session_skill_source_response(source: Option<&str>) -> Option<Response> {
    (!is_supported_session_skill_source(source)).then(invalid_session_skill_source_response)
}

fn is_supported_session_skill_source(source: Option<&str>) -> bool {
    normalized_session_skill_source(source) == "sbp"
}

fn normalized_session_skill_source(source: Option<&str>) -> &str {
    source
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .unwrap_or("sbp")
}

fn invalid_session_skill_source_response() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(error_body_msg(
            "INVALID_SKILL_SOURCE",
            "session skills source must be sbp",
        )),
    )
        .into_response()
}

fn remote_session_skills_response(session_id: &str, query: Option<String>) -> Option<Response> {
    remote_sessions::split_remote_session_id(session_id).map(|_| {
        (
            StatusCode::OK,
            Json(remote_session_skills_payload(session_id.to_string(), query)),
        )
            .into_response()
    })
}

fn remote_session_skills_payload(
    session_id: String,
    query: Option<String>,
) -> SessionSkillListResponse {
    SessionSkillListResponse {
        session_id,
        source: "sbp".to_string(),
        cwd: String::new(),
        available: false,
        query,
        skills: Vec::new(),
        issues: Vec::new(),
        message: Some("remote session skills must be queried on the target host".to_string()),
    }
}

async fn local_session_skills_response(
    state: Arc<AppState>,
    session_id: String,
    query: Option<String>,
) -> Response {
    let summary = match fetch_session_skills_summary(&state, &session_id).await {
        Ok(summary) => summary,
        Err(response) => return response,
    };

    let response = read_sbp_session_skills(&session_id, &summary.cwd, query.as_deref()).await;
    (StatusCode::OK, Json(response)).into_response()
}

async fn fetch_session_skills_summary(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<crate::types::SessionSummary, Response> {
    match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => Ok(summary),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(error_body("SESSION_NOT_FOUND", None)),
        )
            .into_response()),
        Err(err) => {
            tracing::error!("session skills summary lookup failed: {err}");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_body_msg("INTERNAL_ERROR", err.to_string())),
            )
                .into_response())
        }
    }
}

pub async fn read_sbp_session_skills(
    session_id: &str,
    cwd: &str,
    query: Option<&str>,
) -> SessionSkillListResponse {
    let Some(sbp) = resolve_sbp_command(cwd) else {
        return unavailable_session_skills(
            session_id,
            cwd,
            query,
            sbp_command_unavailable_message(),
        );
    };

    let output = match run_sbp_skills_command(sbp, cwd).await {
        Ok(output) => output,
        Err(message) => return unavailable_session_skills(session_id, cwd, query, message),
    };

    sbp_session_skills_from_output(session_id, cwd, query, output)
}

async fn run_sbp_skills_command(sbp: PathBuf, cwd: &str) -> Result<std::process::Output, String> {
    let mut command = Command::new(sbp);
    command.args(["skills", "--format", "json", "--cwd", cwd]);
    match tokio::time::timeout(SBP_SKILLS_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(format!("sbp skills failed to start: {err}")),
        Err(_) => Err("sbp skills timed out".to_string()),
    }
}

fn sbp_session_skills_from_output(
    session_id: &str,
    cwd: &str,
    query: Option<&str>,
    output: std::process::Output,
) -> SessionSkillListResponse {
    if output.status.success() {
        parse_sbp_skills_stdout(session_id, cwd, query, &output.stdout)
    } else {
        unavailable_session_skills(session_id, cwd, query, format_sbp_failure_message(&output))
    }
}

fn format_sbp_failure_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("sbp skills exited with {}", output.status)
    } else {
        format!("sbp skills failed: {stderr}")
    }
}

fn parse_sbp_skills_stdout(
    session_id: &str,
    cwd: &str,
    query: Option<&str>,
    stdout: &[u8],
) -> SessionSkillListResponse {
    match parse_sbp_session_skills(session_id, cwd, query, &String::from_utf8_lossy(stdout)) {
        Ok(response) => response,
        Err(err) => unavailable_session_skills(
            session_id,
            cwd,
            query,
            format!("sbp skills output could not be parsed: {err}"),
        ),
    }
}

fn resolve_sbp_command(cwd: &str) -> Option<PathBuf> {
    sbp_command_from_env()
        .or_else(|| find_sbp_command_near(cwd))
        .or_else(|| Some(fallback_sbp_command()))
}

fn sbp_command_from_env() -> Option<PathBuf> {
    let value = std::env::var("SWIMMERS_SBP").ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn find_sbp_command_near(cwd: &str) -> Option<PathBuf> {
    Path::new(cwd)
        .ancestors()
        .flat_map(sbp_command_candidates_for_ancestor)
        .find(|candidate| is_valid_sbp_command_candidate(candidate))
}

fn sbp_command_candidates_for_ancestor(ancestor: &Path) -> [PathBuf; 2] {
    [
        ancestor.join("skillbox").join("scripts").join("sbp"),
        ancestor
            .join("opensource")
            .join("skillbox")
            .join("scripts")
            .join("sbp"),
    ]
}

fn is_valid_sbp_command_candidate(candidate: &Path) -> bool {
    candidate.is_file()
}

fn fallback_sbp_command() -> PathBuf {
    PathBuf::from("sbp")
}

fn sbp_command_unavailable_message() -> &'static str {
    "sbp command unavailable; set SWIMMERS_SBP or add sbp to PATH"
}

fn parse_sbp_session_skills(
    session_id: &str,
    cwd: &str,
    query: Option<&str>,
    output: &str,
) -> Result<SessionSkillListResponse, serde_json::Error> {
    let value: Value = serde_json::from_str(output)?;
    let query = query
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_string);
    let query_lc = query.as_deref().map(str::to_ascii_lowercase);
    let skills = value
        .get("effective")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(session_skill_from_value)
        .filter(|skill| match query_lc.as_deref() {
            Some(query) => session_skill_matches(skill, query),
            None => true,
        })
        .collect();
    let issues = value
        .get("recommendations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(session_skill_issue_from_value)
        .collect();

    Ok(SessionSkillListResponse {
        session_id: session_id.to_string(),
        source: "sbp".to_string(),
        cwd: value
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or(cwd)
            .to_string(),
        available: true,
        query,
        skills,
        issues,
        message: None,
    })
}

fn session_skill_from_value(value: &Value) -> Option<SessionSkillSummary> {
    Some(SessionSkillSummary {
        name: value.get("name")?.as_str()?.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        state: value
            .get("state")
            .and_then(Value::as_str)
            .map(str::to_string),
        availability: value
            .get("availability")
            .and_then(Value::as_str)
            .map(str::to_string),
        layer: value
            .get("layer")
            .and_then(Value::as_str)
            .map(str::to_string),
        source_bucket: value
            .get("source_bucket")
            .and_then(Value::as_str)
            .map(str::to_string),
        source: value
            .get("source")
            .and_then(Value::as_str)
            .or_else(|| value.get("link_target").and_then(Value::as_str))
            .map(str::to_string),
        path: value
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn session_skill_matches(skill: &SessionSkillSummary, query: &str) -> bool {
    skill.name.to_ascii_lowercase().contains(query)
        || skill
            .description
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(query))
        || skill
            .source_bucket
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(query))
        || skill
            .path
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(query))
}

fn session_skill_issue_from_value(value: &Value) -> SessionSkillIssue {
    let skill = value
        .get("skill")
        .and_then(Value::as_str)
        .map(str::to_string);
    let action = value
        .get("action")
        .and_then(Value::as_str)
        .map(str::to_string);
    let hint = value
        .get("hint")
        .and_then(Value::as_str)
        .map(str::to_string);
    let source_path = value
        .get("source_path")
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = match (&skill, &action, &hint) {
        (Some(skill), Some(action), Some(hint)) => format!("{skill}: {action}: {hint}"),
        (Some(skill), Some(action), None) => format!("{skill}: {action}"),
        (Some(skill), None, Some(hint)) => format!("{skill}: {hint}"),
        (None, Some(action), Some(hint)) => format!("{action}: {hint}"),
        (None, Some(action), None) => action.clone(),
        (None, None, Some(hint)) => hint.clone(),
        (Some(skill), None, None) => skill.clone(),
        (None, None, None) => "sbp recommendation".to_string(),
    };
    SessionSkillIssue {
        skill,
        action,
        hint,
        source_path,
        message,
    }
}

fn unavailable_session_skills(
    session_id: &str,
    cwd: &str,
    query: Option<&str>,
    message: impl Into<String>,
) -> SessionSkillListResponse {
    SessionSkillListResponse {
        session_id: session_id.to_string(),
        source: "sbp".to_string(),
        cwd: cwd.to_string(),
        available: false,
        query: query
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_string),
        skills: Vec::new(),
        issues: Vec::new(),
        message: Some(redact_known_secrets(message.into())),
    }
}

fn redact_known_secrets(mut message: String) -> String {
    for key in ["AUTH_TOKEN", "OBSERVER_TOKEN"] {
        if let Ok(secret) = std::env::var(key) {
            let trimmed = secret.trim();
            if !trimmed.is_empty() {
                message = message.replace(trimmed, "[redacted]");
            }
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::actor::{ActorHandle, SessionCommand};
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use crate::types::{SessionState, StateEvidence, ThoughtSource, ThoughtState, TransportHealth};
    use axum::body::to_bytes;
    use axum::extract::{Query, State};
    use axum::response::IntoResponse;
    use chrono::Utc;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path as FsPath;
    use std::sync::Arc;
    use tokio::sync::{mpsc, RwLock};

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            native_desktop_app: Arc::new(RwLock::new(crate::types::NativeDesktopApp::Iterm)),
            ghostty_open_mode: Arc::new(RwLock::new(crate::types::GhosttyOpenMode::Swap)),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: crate::api::once_lock_with(None),
            file_store: crate::api::once_lock_with(None),
            bridge_health: Arc::new(crate::thought::health::BridgeHealthState::new_with_tick(
                std::time::Duration::from_secs(15),
            )),
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
            repo_actions: crate::host_actions::RepoActionTracker::default(),
        })
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
    }

    fn summary(session_id: &str, cwd: &str) -> crate::types::SessionSummary {
        crate::types::SessionSummary {
            session_id: session_id.to_string(),
            tmux_name: format!("tmux-{session_id}"),
            state: SessionState::Idle,
            current_command: None,
            state_evidence: StateEvidence::new("osc133_prompt"),
            cwd: cwd.to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: crate::types::fallback_rest_state(
                SessionState::Idle,
                ThoughtState::Holding,
            ),
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            stale_attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
            batch: None,
        }
    }

    async fn insert_summary_test_handle(
        state: &Arc<AppState>,
        summary: crate::types::SessionSummary,
    ) {
        let session_id = summary.session_id.clone();
        let tmux_name = summary.tmux_name.clone();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(4);
        state
            .supervisor
            .insert_test_handle(ActorHandle::test_handle(&session_id, &tmux_name, cmd_tx))
            .await;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let SessionCommand::GetSummary(reply) = cmd {
                    let _ = reply.send(summary.clone());
                }
            }
        });
    }

    struct TestEnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl TestEnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_executable(path: &FsPath, contents: &str) {
        fs::write(path, contents).expect("write executable");
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[test]
    fn resolve_sbp_command_prefers_env_override_as_trimmed_path() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("project");
        let local_sbp = cwd.join("skillbox").join("scripts").join("sbp");
        fs::create_dir_all(local_sbp.parent().expect("local sbp parent")).expect("local sbp dir");
        fs::write(&local_sbp, "#!/bin/sh\n").expect("local sbp");
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", "  /custom/bin/sbp  ");

        assert_eq!(
            resolve_sbp_command(&cwd.to_string_lossy()),
            Some(PathBuf::from("/custom/bin/sbp"))
        );
    }

    #[test]
    fn resolve_sbp_command_uses_nearest_ancestor_and_candidate_order() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("project");
        let cwd = project.join("nested");
        fs::create_dir_all(&cwd).expect("cwd");
        let root_sbp = dir.path().join("skillbox").join("scripts").join("sbp");
        let project_nested_sbp = project
            .join("opensource")
            .join("skillbox")
            .join("scripts")
            .join("sbp");
        fs::create_dir_all(root_sbp.parent().expect("root sbp parent")).expect("root sbp dir");
        fs::create_dir_all(project_nested_sbp.parent().expect("project sbp parent"))
            .expect("project sbp dir");
        fs::write(&root_sbp, "#!/bin/sh\n").expect("root sbp");
        fs::write(&project_nested_sbp, "#!/bin/sh\n").expect("project sbp");
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", "  ");

        assert_eq!(
            resolve_sbp_command(&cwd.to_string_lossy()),
            Some(project_nested_sbp)
        );
    }

    #[test]
    fn resolve_sbp_command_prefers_sibling_candidate_before_nested_candidate() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let sibling_sbp = dir.path().join("skillbox").join("scripts").join("sbp");
        let nested_sbp = dir
            .path()
            .join("opensource")
            .join("skillbox")
            .join("scripts")
            .join("sbp");
        fs::create_dir_all(sibling_sbp.parent().expect("sibling sbp parent"))
            .expect("sibling sbp dir");
        fs::create_dir_all(nested_sbp.parent().expect("nested sbp parent"))
            .expect("nested sbp dir");
        fs::write(&sibling_sbp, "#!/bin/sh\n").expect("sibling sbp");
        fs::write(&nested_sbp, "#!/bin/sh\n").expect("nested sbp");
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", "  ");

        assert_eq!(
            resolve_sbp_command(&dir.path().to_string_lossy()),
            Some(sibling_sbp)
        );
    }

    #[test]
    fn resolve_sbp_command_falls_back_to_path_lookup_name() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", "  ");

        assert_eq!(
            resolve_sbp_command(&dir.path().to_string_lossy()),
            Some(PathBuf::from("sbp"))
        );
    }

    #[test]
    fn parse_skill_md_reads_frontmatter_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_md = dir.path().join("SKILL.md");
        fs::write(
            &skill_md,
            r#"---
name: "Code Review"
description: 'Review risky code paths'
---
# ignored body
"#,
        )
        .expect("write skill");

        let parsed = parse_skill_md(&skill_md, "fallback");
        assert_eq!(parsed.name, "Code Review");
        assert_eq!(
            parsed.description.as_deref(),
            Some("Review risky code paths")
        );
    }

    #[test]
    fn parse_skill_md_uses_trimmed_fallback_when_file_cannot_be_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let parsed = parse_skill_md(&dir.path().join("missing.md"), "  fallback  ");

        assert_eq!(parsed.name, "fallback");
        assert_eq!(parsed.description, None);
    }

    #[test]
    fn parse_skill_md_ignores_frontmatter_not_on_first_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_md = dir.path().join("SKILL.md");
        fs::write(
            &skill_md,
            "\n---\nname: Body Name\ndescription: Body\n---\n",
        )
        .expect("write skill");

        let parsed = parse_skill_md(&skill_md, "fallback");
        assert_eq!(parsed.name, "fallback");
        assert_eq!(parsed.description, None);
    }

    #[test]
    fn parse_skill_md_stops_at_closing_frontmatter_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_md = dir.path().join("SKILL.md");
        fs::write(
            &skill_md,
            "---\nname: Frontmatter\ndescription: Kept\n---\nname: Body\n",
        )
        .expect("write skill");

        let parsed = parse_skill_md(&skill_md, "fallback");
        assert_eq!(parsed.name, "Frontmatter");
        assert_eq!(parsed.description.as_deref(), Some("Kept"));
    }

    #[test]
    fn parse_skill_md_ignores_empty_parsed_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_md = dir.path().join("SKILL.md");
        fs::write(&skill_md, "---\nname: \"\"\ndescription: ''\n---\n").expect("write skill");

        let parsed = parse_skill_md(&skill_md, "fallback");
        assert_eq!(parsed.name, "fallback");
        assert_eq!(parsed.description, None);
    }

    #[test]
    fn collect_skill_summaries_discovers_nested_dirs_and_packaged_skills() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("alpha")).expect("alpha dir");
        fs::write(
            dir.path().join("alpha").join("SKILL.md"),
            "---\nname: Alpha\ndescription: first\n---\n",
        )
        .expect("alpha skill");

        fs::create_dir_all(dir.path().join(".system").join("beta")).expect("beta dir");
        fs::write(
            dir.path().join(".system").join("beta").join("SKILL.md"),
            "---\nname: Beta\n---\n",
        )
        .expect("beta skill");

        fs::create_dir_all(dir.path().join("nested").join("gamma")).expect("gamma dir");
        fs::write(
            dir.path().join("nested").join("gamma").join("SKILL.md"),
            "---\nname: alpha\n---\n",
        )
        .expect("duplicate alpha");

        fs::write(dir.path().join("delta.skill"), "packed").expect("packaged skill");
        fs::create_dir_all(dir.path().join(".hidden")).expect("hidden dir");
        fs::write(
            dir.path().join(".hidden").join("SKILL.md"),
            "---\nname: Hidden\n---\n",
        )
        .expect("hidden skill");

        let summaries = collect_skill_summaries(dir.path());
        let names: Vec<&str> = summaries
            .iter()
            .map(|summary| summary.name.as_str())
            .collect();
        assert_eq!(names, vec!["Alpha", "Beta", "delta"]);
        assert_eq!(summaries[0].description.as_deref(), Some("first"));
    }

    #[tokio::test]
    async fn list_skills_reads_home_default_registry() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let skill_root = dir.path().join(".codex").join("skills").join("focus");
        fs::create_dir_all(&skill_root).expect("skill root");
        fs::write(
            skill_root.join("SKILL.md"),
            "---\nname: Focus\ndescription: stay on the current slice\n---\n",
        )
        .expect("skill file");

        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());

        let response = list_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Query(SkillQuery {
                tool: Some("codex".to_string()),
            }),
        )
        .await
        .into_response();

        if let Some(value) = previous_home {
            std::env::set_var("HOME", value);
        } else {
            std::env::remove_var("HOME");
        }

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["tool"], "codex");
        assert_eq!(json["skills"][0]["name"], "Focus");
    }

    #[tokio::test]
    async fn list_skills_rejects_unknown_tool() {
        let response = list_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            Query(SkillQuery {
                tool: Some("unknown".to_string()),
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INVALID_SKILL_TOOL");
    }

    #[tokio::test]
    async fn session_skills_reads_sbp_json_for_session_cwd() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("project");
        fs::create_dir_all(&cwd).expect("cwd");
        let sbp = dir.path().join("sbp");
        let args_log = dir.path().join("args.log");
        let output_file = dir.path().join("skills.json");
        fs::write(
            &output_file,
            format!(
                r#"{{
  "cwd": "{}",
  "effective": [
    {{"name":"ui","availability":"installed","state":"ok","layer":"global:claude","source_bucket":"opensource/skills","source":"/src/opensource/skills/ui","path":"/skills/ui"}},
    {{"name":"smart","availability":"installed","state":"ok","layer":"global:claude","source_bucket":"skills-private","source":"/src/skills-private/smart","path":"/skills/smart"}}
  ],
  "recommendations": [
    {{"skill":"ui","action":"move_global_to_project","hint":"project-specific UI skill","source_path":"/skills/ui"}}
  ]
}}"#,
                cwd.to_string_lossy()
            ),
        )
        .expect("sbp json");
        write_executable(
            &sbp,
            r#"#!/bin/sh
printf '%s\n' "$*" > "$SBP_ARGS_LOG"
cat "$SBP_OUTPUT_FILE"
"#,
        );
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", sbp.as_os_str());
        let _args_guard = TestEnvGuard::set("SBP_ARGS_LOG", args_log.as_os_str());
        let _output_guard = TestEnvGuard::set("SBP_OUTPUT_FILE", output_file.as_os_str());

        let state = test_state();
        insert_summary_test_handle(&state, summary("sess-skills", &cwd.to_string_lossy())).await;

        let response = list_session_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            axum::extract::Path("sess-skills".to_string()),
            Query(SessionSkillQuery {
                source: Some("sbp".to_string()),
                q: Some("ui".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], "sess-skills");
        assert_eq!(json["source"], "sbp");
        assert_eq!(json["cwd"], cwd.to_string_lossy().as_ref());
        assert_eq!(json["available"], true);
        assert_eq!(json["query"], "ui");
        assert_eq!(json["skills"].as_array().unwrap().len(), 1);
        assert_eq!(json["skills"][0]["name"], "ui");
        assert_eq!(json["skills"][0]["source"], "/src/opensource/skills/ui");
        assert_eq!(json["issues"][0]["skill"], "ui");

        let args = fs::read_to_string(args_log).expect("args log");
        assert!(args.contains("skills --format json --cwd"));
        assert!(args.contains(cwd.to_string_lossy().as_ref()));
        assert!(!args.contains("skill add"));
        assert!(!args.contains("skill sync"));
        assert!(!args.contains("skill prune"));
    }

    #[tokio::test]
    async fn session_skills_rejects_non_sbp_source() {
        let response = list_session_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            axum::extract::Path("sess-skills".to_string()),
            Query(SessionSkillQuery {
                source: Some("codex".to_string()),
                q: Some("ui".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = response_json(response).await;
        assert_eq!(json["code"], "INVALID_SKILL_SOURCE");
        assert_eq!(json["message"], "session skills source must be sbp");
    }

    #[tokio::test]
    async fn session_skills_reports_missing_local_session() {
        let response = list_session_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            axum::extract::Path("missing-session".to_string()),
            Query(SessionSkillQuery {
                source: None,
                q: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = response_json(response).await;
        assert_eq!(json["code"], "SESSION_NOT_FOUND");
    }

    #[tokio::test]
    async fn session_skills_degrades_remote_namespaced_sessions() {
        let remote_id =
            crate::api::remote_sessions::namespace_session_id("remote-target", "sess/weird?x#frag");

        let response = list_session_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(test_state()),
            axum::extract::Path(remote_id.clone()),
            Query(SessionSkillQuery {
                source: Some("sbp".to_string()),
                q: Some("remote".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["session_id"], remote_id);
        assert_eq!(json["source"], "sbp");
        assert_eq!(json["cwd"], "");
        assert_eq!(json["available"], false);
        assert_eq!(json["query"], "remote");
        assert_eq!(json["skills"].as_array().unwrap().len(), 0);
        assert_eq!(json["issues"].as_array().unwrap().len(), 0);
        assert_eq!(
            json["message"],
            "remote session skills must be queried on the target host"
        );
    }

    #[tokio::test]
    async fn session_skills_route_degrades_encoded_remote_namespaced_sessions() {
        let remote_id =
            crate::api::remote_sessions::namespace_session_id("remote-target", "sess/weird?x#frag");
        let encoded_id = crate::api::remote_sessions::encode_path_segment(&remote_id);
        let app = routes()
            .layer(Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())))
            .with_state(test_state());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind skills route test server");
        let addr = listener.local_addr().expect("skills route test addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve skills route test server");
        });

        let response = reqwest::get(format!(
            "http://{addr}/v1/sessions/{encoded_id}/skills?source=sbp&q=remote"
        ))
        .await
        .expect("session skills route response");

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let json = response.json::<Value>().await.expect("session skills json");
        assert_eq!(json["session_id"], remote_id);
        assert_eq!(json["source"], "sbp");
        assert_eq!(json["cwd"], "");
        assert_eq!(json["available"], false);
        assert_eq!(json["query"], "remote");
        assert_eq!(json["skills"].as_array().unwrap().len(), 0);
        assert_eq!(json["issues"].as_array().unwrap().len(), 0);
        assert_eq!(
            json["message"],
            "remote session skills must be queried on the target host"
        );

        handle.abort();
    }

    #[tokio::test]
    async fn session_skills_redacts_sbp_failure_and_degrades() {
        let _lock = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("project");
        fs::create_dir_all(&cwd).expect("cwd");
        let sbp = dir.path().join("sbp");
        write_executable(
            &sbp,
            r#"#!/bin/sh
printf 'failed with %s\n' "$AUTH_TOKEN" >&2
exit 2
"#,
        );
        let _sbp_guard = TestEnvGuard::set("SWIMMERS_SBP", sbp.as_os_str());
        let _auth_guard = TestEnvGuard::set("AUTH_TOKEN", "secret-token");

        let state = test_state();
        insert_summary_test_handle(&state, summary("sess-skills-fail", &cwd.to_string_lossy()))
            .await;

        let response = list_session_skills(
            Extension(AuthInfo::new(OPERATOR_SCOPES.to_vec())),
            State(state),
            axum::extract::Path("sess-skills-fail".to_string()),
            Query(SessionSkillQuery {
                source: Some("sbp".to_string()),
                q: None,
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        assert_eq!(json["available"], false);
        assert_eq!(json["skills"].as_array().unwrap().len(), 0);
        let message = json["message"].as_str().expect("message");
        assert!(message.contains("[redacted]"));
        assert!(!message.contains("secret-token"));
    }
}
