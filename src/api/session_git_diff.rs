use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;
use tokio::process::Command;

use crate::api::envelope::error_body;
use crate::api::{fetch_live_summary, remote_sessions, AppState};
use crate::auth::{AuthInfo, AuthScope};
use crate::types::{
    SessionGitDiffFileSummary, SessionGitDiffHunkSummary, SessionGitDiffResponse, SessionSummary,
};

const GIT_DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
const GIT_DIFF_MAX_BYTES: usize = 128 * 1024;

pub(crate) async fn get_git_diff(
    Extension(auth): Extension<AuthInfo>,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
) -> Response {
    if let Err(resp) = auth.require_scope(AuthScope::SessionsRead) {
        return resp;
    }

    fetch_git_diff_response(&state, &session_id).await
}

async fn fetch_git_diff_response(state: &Arc<AppState>, session_id: &str) -> Response {
    match remote_sessions::denamespace_for_target(session_id) {
        Ok(Some((target, remote_session_id))) => {
            return match remote_sessions::fetch_remote_git_diff(&target, remote_session_id).await {
                Ok(response) => (StatusCode::OK, Json(response)).into_response(),
                Err(err) => err.into_response(),
            };
        }
        Ok(None) => {}
        Err(err) => return err.into_response(),
    }

    let summary = match fetch_live_summary(state, session_id).await {
        Ok(Some(summary)) => summary,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "SESSION_NOT_FOUND", None);
        }
        Err(err) => {
            tracing::error!("git diff summary lookup failed: {err}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                Some(err.to_string()),
            );
        }
    };

    let response = read_git_diff_for_summary(summary).await;
    (StatusCode::OK, Json(response)).into_response()
}

pub(crate) async fn read_git_diff_for_summary(summary: SessionSummary) -> SessionGitDiffResponse {
    let repo_root = match run_git_capture(&summary.cwd, &["rev-parse", "--show-toplevel"]).await {
        Ok(root) => root.trim().to_string(),
        Err(message) => {
            return git_diff_unavailable(
                summary.session_id,
                summary.cwd,
                format!("git repo root unavailable: {message}"),
            );
        }
    };

    if repo_root.is_empty() {
        return git_diff_unavailable(
            summary.session_id,
            summary.cwd,
            "git repo root unavailable: empty git output",
        );
    }

    let status_short = match run_git_capture(&repo_root, &["status", "--short"]).await {
        Ok(output) => output,
        Err(message) => {
            return git_diff_unavailable(
                summary.session_id,
                summary.cwd,
                format!("git status unavailable: {message}"),
            );
        }
    };
    let unstaged_raw =
        match run_git_capture(&repo_root, &["diff", "--no-ext-diff", "--no-color"]).await {
            Ok(output) => output,
            Err(message) => {
                return git_diff_unavailable(
                    summary.session_id,
                    summary.cwd,
                    format!("git diff unavailable: {message}"),
                );
            }
        };
    let staged_raw = match run_git_capture(
        &repo_root,
        &["diff", "--cached", "--no-ext-diff", "--no-color"],
    )
    .await
    {
        Ok(output) => output,
        Err(message) => {
            return git_diff_unavailable(
                summary.session_id,
                summary.cwd,
                format!("git diff --cached unavailable: {message}"),
            );
        }
    };

    let (unstaged_diff, unstaged_truncated) = truncate_git_output(unstaged_raw);
    let (staged_diff, staged_truncated) = truncate_git_output(staged_raw);
    let files = summarize_git_diff_files(
        &staged_diff,
        staged_truncated,
        &unstaged_diff,
        unstaged_truncated,
    );
    SessionGitDiffResponse {
        session_id: summary.session_id,
        available: true,
        cwd: summary.cwd,
        repo_root: Some(repo_root),
        status_short,
        unstaged_diff,
        staged_diff,
        truncated: unstaged_truncated || staged_truncated,
        message: None,
        files,
    }
}

async fn run_git_capture(cwd: &str, args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(
        GIT_DIFF_TIMEOUT,
        Command::new("git").arg("-C").arg(cwd).args(args).output(),
    )
    .await
    .map_err(|_| format!("git {} timed out", args.join(" ")))?
    .map_err(|err| err.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(format!(
                "git {} exited with {}",
                args.join(" "),
                output.status
            ));
        }
        return Err(stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn truncate_git_output(output: String) -> (String, bool) {
    if output.len() <= GIT_DIFF_MAX_BYTES {
        return (output, false);
    }

    let mut end = GIT_DIFF_MAX_BYTES;
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    (output[..end].to_string(), true)
}

fn summarize_git_diff_files(
    staged_diff: &str,
    staged_truncated: bool,
    unstaged_diff: &str,
    unstaged_truncated: bool,
) -> Vec<SessionGitDiffFileSummary> {
    // Stamp each source with its own truncation flag: a staged file's summary
    // must not be marked truncated just because the unstaged diff overflowed.
    let mut files = Vec::new();
    files.extend(parse_git_diff_file_summaries(
        "staged",
        staged_diff,
        staged_truncated,
    ));
    files.extend(parse_git_diff_file_summaries(
        "unstaged",
        unstaged_diff,
        unstaged_truncated,
    ));
    files
}

fn parse_git_diff_file_summaries(
    source: &str,
    diff_text: &str,
    truncated: bool,
) -> Vec<SessionGitDiffFileSummary> {
    let mut files = Vec::new();
    let mut current: Option<SessionGitDiffFileSummary> = None;
    let mut current_hunk: Option<SessionGitDiffHunkSummary> = None;

    for line in diff_text.lines() {
        if line.starts_with("diff --git ") {
            push_diff_hunk(&mut current, &mut current_hunk);
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(SessionGitDiffFileSummary {
                path: parse_diff_git_path(line).unwrap_or_else(|| "unknown".to_string()),
                old_path: None,
                source: source.to_string(),
                change: "modified".to_string(),
                added_lines: 0,
                removed_lines: 0,
                truncated,
                hunks: Vec::new(),
            });
            continue;
        }

        let Some(file) = current.as_mut() else {
            continue;
        };

        if line.starts_with("new file mode ") {
            file.change = "added".to_string();
            continue;
        }
        if line.starts_with("deleted file mode ") {
            file.change = "deleted".to_string();
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            file.old_path = Some(path.to_string());
            file.change = "renamed".to_string();
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            file.path = path.to_string();
            file.change = "renamed".to_string();
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            if let Some(path) = normalize_diff_path(path) {
                file.path = path;
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            if let Some(path) = normalize_diff_path(path) {
                file.old_path = Some(path);
            }
            continue;
        }
        if line.starts_with("@@") {
            push_diff_hunk(&mut current, &mut current_hunk);
            current_hunk = Some(SessionGitDiffHunkSummary {
                header: line.to_string(),
                added_lines: 0,
                removed_lines: 0,
            });
            continue;
        }

        if line.starts_with('+') && !line.starts_with("+++") {
            file.added_lines += 1;
            if let Some(hunk) = current_hunk.as_mut() {
                hunk.added_lines += 1;
            }
        } else if line.starts_with('-') && !line.starts_with("---") {
            file.removed_lines += 1;
            if let Some(hunk) = current_hunk.as_mut() {
                hunk.removed_lines += 1;
            }
        }
    }

    push_diff_hunk(&mut current, &mut current_hunk);
    if let Some(file) = current {
        files.push(file);
    }
    files
}

fn push_diff_hunk(
    current: &mut Option<SessionGitDiffFileSummary>,
    current_hunk: &mut Option<SessionGitDiffHunkSummary>,
) {
    if let (Some(file), Some(hunk)) = (current.as_mut(), current_hunk.take()) {
        file.hunks.push(hunk);
    }
}

fn parse_diff_git_path(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let _diff = parts.next()?;
    let _git = parts.next()?;
    let _old = parts.next()?;
    let new = parts.next()?;
    normalize_diff_path(new)
}

fn normalize_diff_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed == "/dev/null" {
        return None;
    }
    Some(
        trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed)
            .to_string(),
    )
}

fn git_diff_unavailable(
    session_id: String,
    cwd: String,
    message: impl Into<String>,
) -> SessionGitDiffResponse {
    SessionGitDiffResponse {
        session_id,
        available: false,
        cwd,
        repo_root: None,
        status_short: String::new(),
        unstaged_diff: String::new(),
        staged_diff: String::new(),
        truncated: false,
        message: Some(message.into()),
        files: Vec::new(),
    }
}

fn error_response(
    status: StatusCode,
    code: impl Into<String>,
    message: Option<String>,
) -> Response {
    (status, Json(error_body(code, message))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_git_diff_files_marks_truncation_per_source() {
        let staged_diff = "diff --git a/staged.txt b/staged.txt\n\
            new file mode 100644\n\
            --- /dev/null\n\
            +++ b/staged.txt\n\
            @@ -0,0 +1 @@\n\
            +hello\n";
        let unstaged_diff = "diff --git a/unstaged.txt b/unstaged.txt\n\
            --- a/unstaged.txt\n\
            +++ b/unstaged.txt\n\
            @@ -1 +1 @@\n\
            -old\n\
            +new\n";

        let files = summarize_git_diff_files(staged_diff, false, unstaged_diff, true);

        let staged = files
            .iter()
            .find(|f| f.source == "staged")
            .expect("staged file summary");
        let unstaged = files
            .iter()
            .find(|f| f.source == "unstaged")
            .expect("unstaged file summary");

        assert!(
            !staged.truncated,
            "staged file must not be marked truncated when only the unstaged diff overflowed"
        );
        assert!(
            unstaged.truncated,
            "unstaged file must reflect its own truncation"
        );
    }
}
