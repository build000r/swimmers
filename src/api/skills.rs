use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api::AppState;
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{ErrorResponse, SkillListResponse, SkillSummary};

const MAX_SCAN_DEPTH: usize = 6;

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
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::to_value(ErrorResponse {
                    code: "INVALID_SKILL_TOOL".to_string(),
                    message: Some("tool must be one of: claude, codex".to_string()),
                })
                .unwrap(),
            ),
        )
            .into_response();
    };

    let root = skill_root_for_tool(tool);
    let skills = root
        .as_deref()
        .map(collect_skill_summaries)
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(
            serde_json::to_value(SkillListResponse {
                tool: tool.wire_name().to_string(),
                skills,
            })
            .unwrap(),
        ),
    )
        .into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/skills", get(list_skills))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::PublishedSelectionState;
    use crate::auth::OPERATOR_SCOPES;
    use crate::config::Config;
    use crate::session::supervisor::SessionSupervisor;
    use crate::thought::protocol::SyncRequestSequence;
    use crate::thought::runtime_config::ThoughtConfig;
    use axum::body::to_bytes;
    use axum::extract::{Query, State};
    use axum::response::IntoResponse;
    use serde_json::Value;
    use std::fs;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn test_state() -> Arc<AppState> {
        let config = Arc::new(Config::default());
        let supervisor = SessionSupervisor::new(config.clone());
        Arc::new(AppState {
            supervisor,
            config,
            thought_config: Arc::new(RwLock::new(ThoughtConfig::default())),
            sync_request_sequence: Arc::new(SyncRequestSequence::new()),
            daemon_defaults: None,
            file_store: None,
            published_selection: Arc::new(RwLock::new(PublishedSelectionState::default())),
        })
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&body).expect("json body")
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
}
