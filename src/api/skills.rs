#![cfg_attr(not(feature = "personal-workflows"), allow(dead_code))]

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
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        return trimmed[1..trimmed.len() - 1].trim().to_string();
    }
    trimmed.to_string()
}

fn parse_skill_md(path: &Path, fallback_name: &str) -> SkillSummary {
    let mut name = fallback_name.trim().to_string();
    let mut description: Option<String> = None;

    if let Ok(content) = std::fs::read_to_string(path) {
        let mut lines = content.lines();
        if lines.next().map(|line| line.trim()) == Some("---") {
            for line in lines {
                let trimmed = line.trim();
                if trimmed == "---" {
                    break;
                }
                if let Some(rest) = trimmed.strip_prefix("name:") {
                    let parsed = unquote(rest);
                    if !parsed.is_empty() {
                        name = parsed;
                    }
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix("description:") {
                    let parsed = unquote(rest);
                    if !parsed.is_empty() {
                        description = Some(parsed);
                    }
                }
            }
        }
    }

    SkillSummary { name, description }
}

fn collect_skill_summaries(root: &Path) -> Vec<SkillSummary> {
    if !root.exists() {
        return Vec::new();
    }

    let mut summaries = Vec::new();
    let mut seen_names = HashSet::new();
    let mut visited_dirs = HashSet::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];

    while let Some((dir, depth)) = stack.pop() {
        let canonical = match dir.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !visited_dirs.insert(canonical.clone()) {
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&canonical) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let entry_name = entry.file_name().to_string_lossy().into_owned();
            if entry_name.starts_with('.') && entry_name != ".system" {
                continue;
            }

            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.is_file() {
                    let parsed = parse_skill_md(&skill_md, &entry_name);
                    let dedupe_key = parsed.name.to_ascii_lowercase();
                    if seen_names.insert(dedupe_key) {
                        summaries.push(parsed);
                    }
                    continue;
                }

                if depth < MAX_SCAN_DEPTH {
                    stack.push((path, depth + 1));
                }
                continue;
            }

            let is_packaged_skill = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("skill"))
                .unwrap_or(false);
            if !is_packaged_skill {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.trim().to_string())
                .filter(|stem| !stem.is_empty())
                .unwrap_or(entry_name);

            let dedupe_key = name.to_ascii_lowercase();
            if seen_names.insert(dedupe_key) {
                summaries.push(SkillSummary {
                    name,
                    description: None,
                });
            }
        }
    }

    summaries.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    summaries
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
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    let source = query
        .source
        .as_deref()
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .unwrap_or("sbp");
    if source != "sbp" {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_body_msg(
                "INVALID_SKILL_SOURCE",
                "session skills source must be sbp",
            )),
        )
            .into_response();
    }

    if remote_sessions::split_remote_session_id(&session_id).is_some() {
        return (
            StatusCode::OK,
            Json(SessionSkillListResponse {
                session_id,
                source: "sbp".to_string(),
                cwd: String::new(),
                available: false,
                query: query.q,
                skills: Vec::new(),
                issues: Vec::new(),
                message: Some(
                    "remote session skills must be queried on the target host".to_string(),
                ),
            }),
        )
            .into_response();
    }

    let summary = match fetch_live_summary(&state, &session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(error_body("SESSION_NOT_FOUND", None)),
            )
                .into_response();
        }
        Err(err) => {
            tracing::error!("session skills summary lookup failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_body_msg("INTERNAL_ERROR", err.to_string())),
            )
                .into_response();
        }
    };

    let response = read_sbp_session_skills(&session_id, &summary.cwd, query.q.as_deref()).await;
    (StatusCode::OK, Json(response)).into_response()
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
            "sbp command unavailable; set SWIMMERS_SBP or add sbp to PATH",
        );
    };

    let mut command = Command::new(sbp);
    command.args(["skills", "--format", "json", "--cwd", cwd]);
    let output = match tokio::time::timeout(SBP_SKILLS_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            return unavailable_session_skills(
                session_id,
                cwd,
                query,
                redact_known_secrets(format!("sbp skills failed to start: {err}")),
            );
        }
        Err(_) => {
            return unavailable_session_skills(session_id, cwd, query, "sbp skills timed out");
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("sbp skills exited with {}", output.status)
        } else {
            format!("sbp skills failed: {stderr}")
        };
        return unavailable_session_skills(session_id, cwd, query, redact_known_secrets(message));
    }

    match parse_sbp_session_skills(
        session_id,
        cwd,
        query,
        &String::from_utf8_lossy(&output.stdout),
    ) {
        Ok(response) => response,
        Err(err) => unavailable_session_skills(
            session_id,
            cwd,
            query,
            redact_known_secrets(format!("sbp skills output could not be parsed: {err}")),
        ),
    }
}

fn resolve_sbp_command(cwd: &str) -> Option<PathBuf> {
    if let Ok(value) = std::env::var("SWIMMERS_SBP") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    for ancestor in Path::new(cwd).ancestors() {
        let sibling = ancestor.join("skillbox").join("scripts").join("sbp");
        if sibling.is_file() {
            return Some(sibling);
        }
        let nested = ancestor
            .join("opensource")
            .join("skillbox")
            .join("scripts")
            .join("sbp");
        if nested.is_file() {
            return Some(nested);
        }
    }

    Some(PathBuf::from("sbp"))
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
