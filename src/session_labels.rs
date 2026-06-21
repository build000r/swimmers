use std::path::{Component, Path};

use crate::types::{SessionEnvironmentScope, SessionSummary};

const REPO_NAMESPACE_PARTS: &[&str] = &["opensource", "clients", "personal", "work", "projects"];

pub fn session_raw_cwd(session: &SessionSummary) -> &str {
    trim_non_empty(&session.cwd).unwrap_or("")
}

pub fn session_canonical_cwd(session: &SessionSummary) -> &str {
    session
        .environment
        .canonical_cwd
        .as_deref()
        .and_then(trim_non_empty)
        .unwrap_or_else(|| session_raw_cwd(session))
}

pub fn session_canonical_cwd_key(session: &SessionSummary) -> String {
    normalize_cwd(session_canonical_cwd(session))
}

pub fn session_cwd_label(session: &SessionSummary) -> Option<String> {
    let label = cwd_tail_label(session_canonical_cwd(session))?;
    Some(host_qualified_label(label, session_host_label(session)))
}

pub fn session_repo_key(session: &SessionSummary) -> String {
    let key = repo_key_for_cwd(session_canonical_cwd(session));
    if key.is_empty() {
        session.tmux_name.trim().to_string()
    } else {
        key
    }
}

pub fn session_repo_family_key(session: &SessionSummary) -> String {
    repo_family_key_for_cwd(session_canonical_cwd(session))
}

pub fn repo_key_for_cwd(cwd: &str) -> String {
    let normalized = normalize_cwd(cwd);
    let parts = cwd_path_parts(&normalized);
    let Some(repo_index) = repo_component_index(&parts) else {
        return normalized;
    };
    rooted_path(&parts[..=repo_index], normalized.starts_with('/'))
}

pub fn repo_label_for_key(repo_key: &str) -> String {
    let parts = cwd_path_parts(repo_key);
    repo_label_parts(&parts)
        .unwrap_or_else(|| cwd_tail_label(repo_key).unwrap_or_else(|| repo_key.trim().to_string()))
}

pub fn repo_family_key_for_cwd(cwd: &str) -> String {
    let parts = cwd_path_parts(cwd)
        .into_iter()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>();
    repo_component_index(&parts)
        .and_then(|index| parts.get(index).cloned())
        .or_else(|| parts.last().cloned())
        .unwrap_or_default()
}

fn trim_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn normalize_cwd(cwd: &str) -> String {
    let trimmed = cwd.trim();
    if trimmed == "/" {
        return "/".to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

fn cwd_tail_label(cwd: &str) -> Option<String> {
    normalize_cwd(cwd)
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

fn session_host_label(session: &SessionSummary) -> Option<&str> {
    if session.environment.scope != SessionEnvironmentScope::Remote {
        return None;
    }

    trim_non_empty(&session.environment.display_host)
        .or_else(|| trim_non_empty(&session.environment.target_label))
        .or_else(|| trim_non_empty(&session.environment.target_id))
}

fn host_qualified_label(label: String, host: Option<&str>) -> String {
    match host {
        Some(host) => format!("{label} @ {host}"),
        None => label,
    }
}

fn repo_label_parts(parts: &[String]) -> Option<String> {
    let repo_index = repo_component_index(parts)?;
    let repo = parts.get(repo_index)?;
    let label = repo_namespace_for_index(parts, repo_index)
        .map(|namespace| format!("{namespace}/{repo}"))
        .unwrap_or_else(|| repo.clone());
    Some(label)
}

fn repo_namespace_for_index(parts: &[String], repo_index: usize) -> Option<&str> {
    let namespace_index = repo_index.checked_sub(1)?;
    let namespace = parts.get(namespace_index)?;
    repo_namespace_part(namespace).then_some(namespace.as_str())
}

fn repo_component_index(parts: &[String]) -> Option<usize> {
    let repos_index = parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case("repos"))?;
    let first = repos_index + 1;
    let first_part = parts.get(first)?;
    if repo_namespace_part(first_part) && parts.get(first + 1).is_some() {
        Some(first + 1)
    } else {
        Some(first)
    }
}

fn repo_namespace_part(part: &str) -> bool {
    REPO_NAMESPACE_PARTS
        .iter()
        .any(|namespace| part.eq_ignore_ascii_case(namespace))
}

fn cwd_path_parts(cwd: &str) -> Vec<String> {
    Path::new(cwd)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str().map(|value| value.trim().to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .collect()
}

fn rooted_path(parts: &[String], absolute: bool) -> String {
    let joined = parts.join("/");
    if absolute && !joined.is_empty() {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::types::{
        LaunchTargetSummary, SessionEnvironmentSummary, SessionState, StateEvidence,
    };

    fn session_with_cwd(cwd: &str) -> SessionSummary {
        SessionSummary::live(
            "sess-1",
            "1",
            SessionState::Idle,
            None,
            StateEvidence::new("test"),
            cwd,
            Some("Codex".to_string()),
            0,
            0,
            Utc::now(),
        )
    }

    #[test]
    fn mapped_remote_session_uses_local_canonical_repo_key() {
        let mut session = session_with_cwd("/srv/skillbox/repos/swimmers");
        session.environment = SessionEnvironmentSummary::remote(
            &LaunchTargetSummary {
                id: "skillbox".to_string(),
                label: "Skillbox devbox".to_string(),
                kind: "swimmers_api".to_string(),
                base_url: None,
                auth_token_env: None,
                ssh_alias: None,
                remote_attach_command_template: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            "remote-1",
            "/srv/skillbox/repos/swimmers",
            Some("/Users/b/repos/opensource/swimmers".to_string()),
            "remote_swimmers_api",
        );

        assert_eq!(
            session_repo_key(&session),
            "/Users/b/repos/opensource/swimmers"
        );
        assert_eq!(
            repo_label_for_key(&session_repo_key(&session)),
            "opensource/swimmers"
        );
        assert_eq!(session_repo_family_key(&session), "swimmers");
        assert_eq!(
            session_cwd_label(&session).as_deref(),
            Some("swimmers @ Skillbox devbox")
        );
        assert_eq!(session_raw_cwd(&session), "/srv/skillbox/repos/swimmers");
    }

    #[test]
    fn unmapped_remote_session_keeps_honest_remote_repo_key() {
        let mut session = session_with_cwd("/srv/skillbox/repos/swimmers");
        session.environment = SessionEnvironmentSummary::remote(
            &LaunchTargetSummary {
                id: "skillbox".to_string(),
                label: "Skillbox devbox".to_string(),
                kind: "swimmers_api".to_string(),
                base_url: None,
                auth_token_env: None,
                ssh_alias: None,
                remote_attach_command_template: None,
                bootstrap_hint: None,
                path_mappings: Vec::new(),
            },
            "remote-1",
            "/srv/skillbox/repos/swimmers",
            None,
            "remote_swimmers_api",
        );

        assert_eq!(session_repo_key(&session), "/srv/skillbox/repos/swimmers");
        assert_eq!(repo_label_for_key(&session_repo_key(&session)), "swimmers");
    }

    #[test]
    fn repo_key_trims_subdirectories_under_repos_root() {
        assert_eq!(
            repo_key_for_cwd("/Users/b/repos/personal/swimmers/packages/api/"),
            "/Users/b/repos/personal/swimmers"
        );
        assert_eq!(
            repo_family_key_for_cwd("/Users/b/repos/htma_server"),
            "htma_server"
        );
    }
}
