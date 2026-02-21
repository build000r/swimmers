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
