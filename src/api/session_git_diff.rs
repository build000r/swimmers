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
    let available_diff = match read_available_git_diff(&summary.cwd).await {
        Ok(diff) => diff,
        Err(message) => return git_diff_unavailable(summary.session_id, summary.cwd, message),
    };

    available_diff.into_response(summary.session_id, summary.cwd)
}

struct AvailableGitDiff {
    repo_root: String,
    status_short: String,
    unstaged_raw: String,
    staged_raw: String,
}

impl AvailableGitDiff {
    fn into_response(self, session_id: String, cwd: String) -> SessionGitDiffResponse {
        let (unstaged_diff, unstaged_truncated) = truncate_git_output(self.unstaged_raw);
        let (staged_diff, staged_truncated) = truncate_git_output(self.staged_raw);
        let files = summarize_git_diff_files(
            &staged_diff,
            staged_truncated,
            &unstaged_diff,
            unstaged_truncated,
        );
        SessionGitDiffResponse {
            session_id,
            available: true,
            cwd,
            repo_root: Some(self.repo_root),
            status_short: self.status_short,
            unstaged_diff,
            staged_diff,
            truncated: unstaged_truncated || staged_truncated,
            message: None,
            files,
        }
    }
}

async fn read_available_git_diff(cwd: &str) -> Result<AvailableGitDiff, String> {
    let repo_root = resolve_git_repo_root(cwd).await?;
    let status_short =
        run_labeled_git_capture(&repo_root, &["status", "--short"], "git status unavailable")
            .await?;
    let unstaged_raw = run_labeled_git_capture(
        &repo_root,
        &["diff", "--no-ext-diff", "--no-color"],
        "git diff unavailable",
    )
    .await?;
    let staged_raw = run_labeled_git_capture(
        &repo_root,
        &["diff", "--cached", "--no-ext-diff", "--no-color"],
        "git diff --cached unavailable",
    )
    .await?;

    Ok(AvailableGitDiff {
        repo_root,
        status_short,
        unstaged_raw,
        staged_raw,
    })
}

async fn resolve_git_repo_root(cwd: &str) -> Result<String, String> {
    let repo_root = run_labeled_git_capture(
        cwd,
        &["rev-parse", "--show-toplevel"],
        "git repo root unavailable",
    )
    .await?
    .trim()
    .to_string();

    if repo_root.is_empty() {
        return Err("git repo root unavailable: empty git output".to_string());
    }

    Ok(repo_root)
}

async fn run_labeled_git_capture(
    cwd: &str,
    args: &[&str],
    unavailable_label: &str,
) -> Result<String, String> {
    run_git_capture(cwd, args)
        .await
        .map_err(|message| format!("{unavailable_label}: {message}"))
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
    let Some(end) = git_output_truncation_end(&output) else {
        return (output, false);
    };

    (output[..end].to_string(), true)
}

fn git_output_truncation_end(output: &str) -> Option<usize> {
    if output.len() <= GIT_DIFF_MAX_BYTES {
        return None;
    }

    Some(previous_char_boundary_at_or_before(
        output,
        GIT_DIFF_MAX_BYTES,
    ))
}

fn previous_char_boundary_at_or_before(output: &str, limit: usize) -> usize {
    (0..=limit)
        .rev()
        .find(|&end| output.is_char_boundary(end))
        .unwrap_or(0)
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
    let mut parser = GitDiffSummaryParser::new(source, truncated);
    for line in diff_text.lines() {
        parser.apply_line(line);
    }
    parser.finish()
}

struct GitDiffSummaryParser<'a> {
    source: &'a str,
    truncated: bool,
    files: Vec<SessionGitDiffFileSummary>,
    current: Option<SessionGitDiffFileSummary>,
    current_hunk: Option<SessionGitDiffHunkSummary>,
}

impl<'a> GitDiffSummaryParser<'a> {
    fn new(source: &'a str, truncated: bool) -> Self {
        Self {
            source,
            truncated,
            files: Vec::new(),
            current: None,
            current_hunk: None,
        }
    }

    fn apply_line(&mut self, line: &str) {
        match classify_git_diff_line(line) {
            GitDiffLine::FileHeader => self.start_file(line),
            GitDiffLine::NewFileMode => self.set_change("added"),
            GitDiffLine::DeletedFileMode => self.set_change("deleted"),
            GitDiffLine::RenameFrom(path) => self.rename_from(path),
            GitDiffLine::RenameTo(path) => self.rename_to(path),
            GitDiffLine::NewPath(path) => self.set_new_path(path),
            GitDiffLine::OldPath(path) => self.set_old_path(path),
            GitDiffLine::HunkHeader => self.start_hunk(line),
            GitDiffLine::AddedContent => self.count_added_line(),
            GitDiffLine::RemovedContent => self.count_removed_line(),
            GitDiffLine::Other => {}
        }
    }

    fn finish(mut self) -> Vec<SessionGitDiffFileSummary> {
        self.push_current_file();
        self.files
    }

    fn start_file(&mut self, line: &str) {
        self.push_current_file();
        self.current = Some(SessionGitDiffFileSummary {
            path: parse_diff_git_path(line).unwrap_or_else(|| "unknown".to_string()),
            old_path: None,
            source: self.source.to_string(),
            change: "modified".to_string(),
            added_lines: 0,
            removed_lines: 0,
            truncated: self.truncated,
            hunks: Vec::new(),
        });
    }

    fn start_hunk(&mut self, line: &str) {
        if self.current.is_none() {
            return;
        }
        push_diff_hunk(&mut self.current, &mut self.current_hunk);
        self.current_hunk = Some(SessionGitDiffHunkSummary {
            header: line.to_string(),
            added_lines: 0,
            removed_lines: 0,
        });
    }

    fn push_current_file(&mut self) {
        push_diff_hunk(&mut self.current, &mut self.current_hunk);
        if let Some(file) = self.current.take() {
            self.files.push(file);
        }
    }

    fn set_change(&mut self, change: &str) {
        if let Some(file) = self.current.as_mut() {
            file.change = change.to_string();
        }
    }

    fn rename_from(&mut self, path: &str) {
        if let Some(file) = self.current.as_mut() {
            file.old_path = Some(path.to_string());
            file.change = "renamed".to_string();
        }
    }

    fn rename_to(&mut self, path: &str) {
        if let Some(file) = self.current.as_mut() {
            file.path = path.to_string();
            file.change = "renamed".to_string();
        }
    }

    fn set_new_path(&mut self, path: &str) {
        let Some(path) = normalize_diff_path(path) else {
            return;
        };
        if let Some(file) = self.current.as_mut() {
            file.path = path;
        }
    }

    fn set_old_path(&mut self, path: &str) {
        let Some(path) = normalize_diff_path(path) else {
            return;
        };
        if let Some(file) = self.current.as_mut() {
            file.old_path = Some(path);
        }
    }

    fn count_added_line(&mut self) {
        if let Some(file) = self.current.as_mut() {
            file.added_lines += 1;
        }
        if let Some(hunk) = self.current_hunk.as_mut() {
            hunk.added_lines += 1;
        }
    }

    fn count_removed_line(&mut self) {
        if let Some(file) = self.current.as_mut() {
            file.removed_lines += 1;
        }
        if let Some(hunk) = self.current_hunk.as_mut() {
            hunk.removed_lines += 1;
        }
    }
}

enum GitDiffLine<'a> {
    FileHeader,
    NewFileMode,
    DeletedFileMode,
    RenameFrom(&'a str),
    RenameTo(&'a str),
    NewPath(&'a str),
    OldPath(&'a str),
    HunkHeader,
    AddedContent,
    RemovedContent,
    Other,
}

fn classify_git_diff_line(line: &str) -> GitDiffLine<'_> {
    if let Some(classified) = classify_git_diff_boundary_line(line) {
        return classified;
    }
    if let Some(classified) = classify_git_diff_path_line(line) {
        return classified;
    }
    classify_git_diff_content_line(line)
}

fn classify_git_diff_boundary_line(line: &str) -> Option<GitDiffLine<'_>> {
    if line.starts_with("diff --git ") {
        return Some(GitDiffLine::FileHeader);
    }
    if line.starts_with("new file mode ") {
        return Some(GitDiffLine::NewFileMode);
    }
    if line.starts_with("deleted file mode ") {
        return Some(GitDiffLine::DeletedFileMode);
    }
    if line.starts_with("@@") {
        return Some(GitDiffLine::HunkHeader);
    }
    None
}

fn classify_git_diff_path_line(line: &str) -> Option<GitDiffLine<'_>> {
    if let Some(path) = line.strip_prefix("rename from ") {
        return Some(GitDiffLine::RenameFrom(path));
    }
    if let Some(path) = line.strip_prefix("rename to ") {
        return Some(GitDiffLine::RenameTo(path));
    }
    if let Some(path) = line.strip_prefix("+++ ") {
        return Some(GitDiffLine::NewPath(path));
    }
    if let Some(path) = line.strip_prefix("--- ") {
        return Some(GitDiffLine::OldPath(path));
    }
    None
}

fn classify_git_diff_content_line(line: &str) -> GitDiffLine<'_> {
    if line.starts_with('+') {
        return GitDiffLine::AddedContent;
    }
    if line.starts_with('-') {
        return GitDiffLine::RemovedContent;
    }
    GitDiffLine::Other
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

    fn parse_test_diff(diff_text: &str) -> Vec<SessionGitDiffFileSummary> {
        parse_git_diff_file_summaries("test", diff_text, false)
    }

    #[test]
    fn truncate_git_output_leaves_short_output_unchanged() {
        let output = "short diff".to_string();

        let (truncated_output, truncated) = truncate_git_output(output.clone());

        assert_eq!(truncated_output, output);
        assert!(!truncated);
    }

    #[test]
    fn truncate_git_output_cuts_ascii_at_byte_limit() {
        let output = "x".repeat(GIT_DIFF_MAX_BYTES + 1);

        let (truncated_output, truncated) = truncate_git_output(output);

        assert_eq!(truncated_output.len(), GIT_DIFF_MAX_BYTES);
        assert!(truncated);
    }

    #[test]
    fn truncate_git_output_cuts_before_partial_utf8_character() {
        let mut output = "x".repeat(GIT_DIFF_MAX_BYTES - 1);
        output.push('é');

        let (truncated_output, truncated) = truncate_git_output(output);

        assert_eq!(truncated_output.len(), GIT_DIFF_MAX_BYTES - 1);
        assert!(truncated_output.ends_with('x'));
        assert!(truncated);
    }

    #[test]
    fn parse_git_diff_file_summaries_handles_renamed_files() {
        let diff = "diff --git a/old.txt b/new.txt\n\
            similarity index 88%\n\
            rename from old.txt\n\
            rename to new.txt\n\
            --- a/old.txt\n\
            +++ b/new.txt\n\
            @@ -1 +1 @@\n\
            -old\n\
            +new\n";

        let files = parse_test_diff(diff);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[0].change, "renamed");
        assert_eq!(files[0].added_lines, 1);
        assert_eq!(files[0].removed_lines, 1);
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn parse_git_diff_file_summaries_handles_new_and_deleted_files() {
        let diff = "diff --git a/new.txt b/new.txt\n\
            new file mode 100644\n\
            --- /dev/null\n\
            +++ b/new.txt\n\
            @@ -0,0 +1,2 @@\n\
            +one\n\
            +two\n\
            diff --git a/deleted.txt b/deleted.txt\n\
            deleted file mode 100644\n\
            --- a/deleted.txt\n\
            +++ /dev/null\n\
            @@ -1,2 +0,0 @@\n\
            -one\n\
            -two\n";

        let files = parse_test_diff(diff);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].old_path, None);
        assert_eq!(files[0].change, "added");
        assert_eq!(files[0].added_lines, 2);
        assert_eq!(files[0].removed_lines, 0);

        assert_eq!(files[1].path, "deleted.txt");
        assert_eq!(files[1].old_path.as_deref(), Some("deleted.txt"));
        assert_eq!(files[1].change, "deleted");
        assert_eq!(files[1].added_lines, 0);
        assert_eq!(files[1].removed_lines, 2);
    }

    #[test]
    fn parse_git_diff_file_summaries_pushes_multiple_hunks_and_counts_lines() {
        let diff = "diff --git a/file.txt b/file.txt\n\
            --- a/file.txt\n\
            +++ b/file.txt\n\
            @@ -1,2 +1,2 @@\n\
             context\n\
            -old\n\
            +new\n\
            @@ -8,2 +8,3 @@\n\
            -gone\n\
            +added\n\
            +more\n";

        let files = parse_test_diff(diff);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].added_lines, 3);
        assert_eq!(files[0].removed_lines, 2);
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[0].header, "@@ -1,2 +1,2 @@");
        assert_eq!(files[0].hunks[0].added_lines, 1);
        assert_eq!(files[0].hunks[0].removed_lines, 1);
        assert_eq!(files[0].hunks[1].header, "@@ -8,2 +8,3 @@");
        assert_eq!(files[0].hunks[1].added_lines, 2);
        assert_eq!(files[0].hunks[1].removed_lines, 1);
    }

    #[test]
    fn parse_git_diff_file_summaries_excludes_plus_minus_headers_from_counts() {
        let diff = "diff --git a/file.txt b/file.txt\n\
            --- a/file.txt\n\
            +++ b/file.txt\n\
            @@ -1 +1 @@\n\
            --- literal removed line\n\
            +++ literal added line\n\
            -removed\n\
            +added\n";

        let files = parse_test_diff(diff);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].added_lines, 1);
        assert_eq!(files[0].removed_lines, 1);
        assert_eq!(files[0].hunks[0].added_lines, 1);
        assert_eq!(files[0].hunks[0].removed_lines, 1);
    }

    #[test]
    fn parse_git_diff_file_summaries_stamps_source_and_truncation_per_file() {
        let diff = "diff --git a/one.txt b/one.txt\n\
            --- a/one.txt\n\
            +++ b/one.txt\n\
            @@ -1 +1 @@\n\
            -a\n\
            +b\n\
            diff --git a/two.txt b/two.txt\n\
            --- a/two.txt\n\
            +++ b/two.txt\n\
            @@ -1 +1 @@\n\
            -c\n\
            +d\n";

        let files = parse_git_diff_file_summaries("custom", diff, true);

        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|file| file.source == "custom"));
        assert!(files.iter().all(|file| file.truncated));
    }

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
