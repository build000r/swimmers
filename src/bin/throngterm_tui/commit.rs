use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const COMMIT_TMUX_PREFIX: &str = "commit";
const COMMIT_TMUX_RUNTIME_DIR: &str = "throngterm-commit-tmux";
const COMMIT_CODEX_MODEL: &str = "gpt-5.4";
const COMMIT_CODEX_REASONING: &str = "low";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitCodexLaunch {
    pub(crate) session_name: String,
    pub(crate) watch_command: String,
}

pub(crate) trait CommitLauncher: Send + Sync {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitCodexLaunch>;
}

#[derive(Default)]
pub(crate) struct SystemCommitLauncher;

impl CommitLauncher for SystemCommitLauncher {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitCodexLaunch> {
        let git_state = collect_git_state(&session.cwd)?;
        launch_commit_codex_tmux(session, &git_state)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GitStateSnapshot {
    repo_root: PathBuf,
    status_short: String,
    unstaged_diff_stat: String,
    staged_diff_stat: String,
    unstaged_diff: String,
    staged_diff: String,
}

fn collect_git_state(cwd: &str) -> io::Result<GitStateSnapshot> {
    let repo_root = resolve_repo_root(cwd)?;
    Ok(GitStateSnapshot {
        status_short: run_git_capture(&repo_root, &["status", "--short"])?,
        unstaged_diff_stat: run_git_capture(&repo_root, &["diff", "--stat"])?,
        staged_diff_stat: run_git_capture(&repo_root, &["diff", "--cached", "--stat"])?,
        unstaged_diff: run_git_capture(&repo_root, &["diff"])?,
        staged_diff: run_git_capture(&repo_root, &["diff", "--cached"])?,
        repo_root,
    })
}

fn resolve_repo_root(cwd: &str) -> io::Result<PathBuf> {
    let output = ProcessCommand::new("git")
        .args(["-C", cwd, "rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            stderr
        };
        return Err(io::Error::other(format!(
            "git repo root lookup failed for {cwd}: {detail}"
        )));
    }

    let repo_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo_root.is_empty() {
        return Err(io::Error::other(format!(
            "git repo root lookup returned an empty path for {cwd}"
        )));
    }

    Ok(PathBuf::from(repo_root))
}

fn run_git_capture(repo_root: &Path, args: &[&str]) -> io::Result<String> {
    let output = ProcessCommand::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            stderr
        };
        return Err(io::Error::other(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo_root.display(),
            detail
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn launch_commit_codex_tmux(
    session: &SessionSummary,
    git_state: &GitStateSnapshot,
) -> io::Result<CommitCodexLaunch> {
    let session_name = commit_tmux_session_name(&session.tmux_name);
    let runtime_dir = std::env::temp_dir().join(COMMIT_TMUX_RUNTIME_DIR);
    fs::create_dir_all(&runtime_dir)?;

    let prompt_path = runtime_dir.join(format!("{session_name}.prompt.md"));
    let wrapper_path = runtime_dir.join(format!("{session_name}.sh"));
    fs::write(&prompt_path, build_commit_codex_prompt(session, git_state))?;
    fs::write(
        &wrapper_path,
        build_commit_tmux_wrapper(&session_name, &git_state.repo_root, &prompt_path),
    )?;

    let repo_root = git_state.repo_root.to_string_lossy().into_owned();
    let wrapper_command = format!(
        "bash {}",
        shell_single_quote(&wrapper_path.to_string_lossy())
    );
    let output = ProcessCommand::new("tmux")
        .args(["new-session", "-d", "-s", &session_name, "-c", &repo_root])
        .arg(wrapper_command)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("tmux exited with {}", output.status)
        } else {
            stderr
        };
        return Err(io::Error::other(format!(
            "tmux launch failed for {}: {}",
            repo_root, detail
        )));
    }

    Ok(CommitCodexLaunch {
        watch_command: format!("tmux a -t {session_name}"),
        session_name,
    })
}

fn build_commit_tmux_wrapper(session_name: &str, repo_root: &Path, prompt_path: &Path) -> String {
    let repo_root = shell_single_quote(&repo_root.to_string_lossy());
    let prompt_path = shell_single_quote(&prompt_path.to_string_lossy());

    format!(
        "#!/bin/bash\n\
SESSION={session_name:?}\n\
REPO_DIR={repo_root}\n\
PROMPT_FILE={prompt_path}\n\
\n\
echo \"=== throngterm commit codex: $SESSION ===\"\n\
echo \"Repo: $REPO_DIR\"\n\
echo \"Started: $(date)\"\n\
echo \"\"\n\
\n\
EXIT_CODE=0\n\
codex exec \\\n\
  -m {COMMIT_CODEX_MODEL} \\\n\
  -c 'model_reasoning_effort=\"{COMMIT_CODEX_REASONING}\"' \\\n\
  --dangerously-bypass-approvals-and-sandbox \\\n\
  --cd \"$REPO_DIR\" \\\n\
  - < \"$PROMPT_FILE\" || EXIT_CODE=$?\n\
\n\
echo \"\"\n\
echo \"Codex exited with code: $EXIT_CODE\"\n\
\n\
if [ \"$EXIT_CODE\" -ne 0 ]; then\n\
  echo \"\"\n\
  echo \"Failed. Session stays alive for inspection.\"\n\
  echo \"Attach: tmux a -t $SESSION\"\n\
  echo \"\"\n\
  echo \"Press enter to close, or Ctrl-C to keep session.\"\n\
  read -r\n\
fi\n"
    )
}

fn build_commit_codex_prompt(session: &SessionSummary, git_state: &GitStateSnapshot) -> String {
    let repo_root = git_state.repo_root.to_string_lossy();
    format!(
        "$commit\n\n\
You were launched from throngterm by clicking a [commit] opportunity in the clawgs rail.\n\
\n\
Source session:\n\
- tmux: {tmux_name}\n\
- session_id: {session_id}\n\
- cwd: {cwd}\n\
- repo_root: {repo_root}\n\
\n\
Run as a fresh detached Codex commit helper. Use model `{COMMIT_CODEX_MODEL}` with `{COMMIT_CODEX_REASONING}` reasoning.\n\
\n\
Task:\n\
- Use the commit skill workflow.\n\
- Work only in `{repo_root}`.\n\
- Treat the git state below as preloaded context so you do not need an extra rediscovery pass.\n\
- Commit only intentional changes in this repo.\n\
- Do not push.\n\
- If there is nothing intentional to commit, explain why and stop without creating an empty commit.\n\
\n\
## git status --short\n\
```text\n\
{status_short}\n\
```\n\
\n\
## git diff --stat\n\
```text\n\
{unstaged_diff_stat}\n\
```\n\
\n\
## git diff --cached --stat\n\
```text\n\
{staged_diff_stat}\n\
```\n\
\n\
## git diff --cached\n\
```diff\n\
{staged_diff}\n\
```\n\
\n\
## git diff\n\
```diff\n\
{unstaged_diff}\n\
```\n",
        tmux_name = session.tmux_name,
        session_id = session.session_id,
        cwd = session.cwd,
        repo_root = repo_root,
        status_short = display_git_output(&git_state.status_short),
        unstaged_diff_stat = display_git_output(&git_state.unstaged_diff_stat),
        staged_diff_stat = display_git_output(&git_state.staged_diff_stat),
        staged_diff = display_git_output(&git_state.staged_diff),
        unstaged_diff = display_git_output(&git_state.unstaged_diff),
    )
}

fn display_git_output(output: &str) -> &str {
    let trimmed = output.trim_end();
    if trimmed.is_empty() {
        "(no output)"
    } else {
        trimmed
    }
}

fn commit_tmux_session_name(tmux_name: &str) -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!(
        "{COMMIT_TMUX_PREFIX}-{}-{suffix}",
        sanitize_tmux_name(tmux_name)
    )
}

fn sanitize_tmux_name(tmux_name: &str) -> String {
    let sanitized = tmux_name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .collect::<String>();
    if sanitized.is_empty() {
        "session".to_string()
    } else {
        sanitized
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use throngterm::types::{ThoughtSource, ThoughtState, TransportHealth};

    fn sample_session() -> SessionSummary {
        SessionSummary {
            session_id: "sess-1".to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Busy,
            current_command: None,
            cwd: "/tmp/repos/throngterm/crate".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 0,
            thought: Some("commit this".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Active,
            commit_candidate: true,
            last_skill: None,
            is_stale: false,
            attached_clients: 0,
            transport_health: TransportHealth::Healthy,
            last_activity_at: Utc::now(),
            repo_theme_id: None,
        }
    }

    #[test]
    fn build_commit_codex_prompt_includes_preloaded_git_state() {
        let prompt = build_commit_codex_prompt(
            &sample_session(),
            &GitStateSnapshot {
                repo_root: PathBuf::from("/tmp/repos/throngterm"),
                status_short: " M src/main.rs\n?? src/new.rs\n".to_string(),
                unstaged_diff_stat: " src/main.rs | 2 +-\n".to_string(),
                staged_diff_stat: " src/lib.rs | 1 +\n".to_string(),
                unstaged_diff: "diff --git a/src/main.rs b/src/main.rs\n".to_string(),
                staged_diff: "diff --git a/src/lib.rs b/src/lib.rs\n".to_string(),
            },
        );

        assert!(prompt.starts_with("$commit"));
        assert!(prompt.contains("gpt-5.4"));
        assert!(prompt.contains("`low` reasoning"));
        assert!(prompt.contains("git status --short"));
        assert!(prompt.contains("M src/main.rs"));
        assert!(prompt.contains("git diff --cached"));
        assert!(prompt.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    }

    #[test]
    fn sanitize_tmux_name_falls_back_for_empty_tokens() {
        assert_eq!(sanitize_tmux_name(""), "session");
        assert_eq!(sanitize_tmux_name("$$$"), "session");
        assert_eq!(sanitize_tmux_name("dev-7"), "dev-7");
    }
}
