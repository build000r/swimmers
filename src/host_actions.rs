#![cfg_attr(not(feature = "personal-workflows"), allow(dead_code))]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;

use crate::types::{RepoActionKind, RepoActionState, RepoActionStatus, SessionSummary};

const COMMIT_TMUX_PREFIX: &str = "commit";
const COMMIT_TMUX_RUNTIME_DIR: &str = "swimmers-commit-tmux";
const COMMIT_CODEX_RUNTIME_DIR: &str = "swimmers-commit-codex";
const COMMIT_CODEX_MODEL: &str = "gpt-5.4";
const COMMIT_CODEX_REASONING: &str = "low";
const REPO_ACTION_STATUS_TTL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitCodexLaunch {
    pub session_name: String,
    pub watch_command: String,
}

pub trait CommitLauncher: Send + Sync {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitCodexLaunch>;
}

#[derive(Default)]
pub struct SystemCommitLauncher;

impl CommitLauncher for SystemCommitLauncher {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitCodexLaunch> {
        let git_state = collect_git_state(&session.cwd)?;
        launch_commit_codex_tmux(session, &git_state)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRepoSummary {
    pub repo_root: PathBuf,
    pub dirty: bool,
}

pub trait RepoActionExecutor: Send + Sync {
    fn execute(&self, repo_root: PathBuf, kind: RepoActionKind) -> io::Result<Option<String>>;
}

#[derive(Default)]
pub struct SystemRepoActionExecutor;

impl RepoActionExecutor for SystemRepoActionExecutor {
    fn execute(&self, repo_root: PathBuf, kind: RepoActionKind) -> io::Result<Option<String>> {
        match kind {
            RepoActionKind::Commit => run_commit_codex_for_repo(&repo_root),
        }
    }
}

#[derive(Clone, Default)]
pub struct RepoActionTracker {
    inner: Arc<RwLock<HashMap<PathBuf, RepoActionRecord>>>,
}

struct RepoActionRecord {
    status: RepoActionStatus,
    finished_at: Option<Instant>,
}

impl RepoActionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(
        &self,
        repo_root: PathBuf,
        kind: RepoActionKind,
        executor: Arc<dyn RepoActionExecutor>,
    ) -> io::Result<()> {
        let repo_root = repo_root.canonicalize().unwrap_or(repo_root);
        let mut inner = self.inner.write().await;
        Self::prune_locked(&mut inner);
        if inner
            .get(&repo_root)
            .map(|record| record.status.state == RepoActionState::Running)
            .unwrap_or(false)
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("repo action already running for {}", repo_root.display()),
            ));
        }

        inner.insert(
            repo_root.clone(),
            RepoActionRecord {
                status: RepoActionStatus {
                    kind,
                    state: RepoActionState::Running,
                    detail: None,
                },
                finished_at: None,
            },
        );
        drop(inner);

        let tracker = self.clone();
        tokio::spawn(async move {
            let repo_root_for_exec = repo_root.clone();
            let result =
                tokio::task::spawn_blocking(move || executor.execute(repo_root_for_exec, kind))
                    .await;

            let (state, detail) = match result {
                Ok(Ok(detail)) => (RepoActionState::Succeeded, detail),
                Ok(Err(err)) => (RepoActionState::Failed, Some(err.to_string())),
                Err(err) => (
                    RepoActionState::Failed,
                    Some(format!("repo action task failed: {err}")),
                ),
            };
            tracker.finish(repo_root, kind, state, detail).await;
        });

        Ok(())
    }

    pub async fn status_for(&self, repo_root: &Path) -> Option<RepoActionStatus> {
        let repo_root = repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf());
        // Read lock only — `list_dirs` calls this once per entry via
        // `buffered(GIT_PROBE_CONCURRENCY)`, and a write lock here would
        // serialize every probe across every concurrent picker request.
        // Prune still runs in `start` and `finish`, which write, so expired
        // records are reclaimed on the next action lifecycle tick.
        let inner = self.inner.read().await;
        inner.get(&repo_root).and_then(|record| {
            if record.status.state == RepoActionState::Running {
                Some(record.status.clone())
            } else {
                record
                    .finished_at
                    .filter(|ts| ts.elapsed() < REPO_ACTION_STATUS_TTL)
                    .map(|_| record.status.clone())
            }
        })
    }

    async fn finish(
        &self,
        repo_root: PathBuf,
        kind: RepoActionKind,
        state: RepoActionState,
        detail: Option<String>,
    ) {
        let mut inner = self.inner.write().await;
        inner.insert(
            repo_root,
            RepoActionRecord {
                status: RepoActionStatus {
                    kind,
                    state,
                    detail,
                },
                finished_at: Some(Instant::now()),
            },
        );
        Self::prune_locked(&mut inner);
    }

    fn prune_locked(inner: &mut HashMap<PathBuf, RepoActionRecord>) {
        inner.retain(|_, record| {
            record.status.state == RepoActionState::Running
                || record
                    .finished_at
                    .map(|finished_at| finished_at.elapsed() < REPO_ACTION_STATUS_TTL)
                    .unwrap_or(false)
        });
    }
}

pub trait ArtifactOpener: Send + Sync {
    fn open(&self, path: &str) -> io::Result<()>;
}

#[derive(Default)]
pub struct SystemArtifactOpener;

impl ArtifactOpener for SystemArtifactOpener {
    fn open(&self, path: &str) -> io::Result<()> {
        if cfg!(target_os = "macos") {
            ProcessCommand::new("open").arg(path).spawn().map(|_| ())
        } else if cfg!(target_os = "windows") {
            ProcessCommand::new("cmd")
                .args(["/C", "start", "", path])
                .spawn()
                .map(|_| ())
        } else {
            ProcessCommand::new("xdg-open")
                .arg(path)
                .spawn()
                .map(|_| ())
        }
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

/// Wall-clock budget for the entire `inspect_git_repo` probe (both git calls).
/// Hung git returns as `Ok(None)` (treated as "not a repo") instead of
/// blocking the caller indefinitely.
const GIT_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Async probe: offloads the blocking git subprocess calls to Tokio's blocking
/// pool via `spawn_blocking`. Using `tokio::process::Command` with many
/// concurrent children wedged the runtime on macOS (concurrent process-wait
/// contention), so we go through the blocking pool instead — it's sized for
/// exactly this pattern and keeps the main worker threads free.
pub async fn inspect_git_repo(path: &Path) -> io::Result<Option<GitRepoSummary>> {
    let path = path.to_path_buf();
    let join_handle = tokio::task::spawn_blocking(move || inspect_git_repo_sync(&path));
    match tokio::time::timeout(GIT_PROBE_TIMEOUT, join_handle).await {
        Ok(Ok(result)) => result,
        Ok(Err(join_err)) => Err(io::Error::other(format!(
            "inspect_git_repo task failed: {join_err}"
        ))),
        // Timed out: let the blocking task finish detached and report "no repo"
        // so the picker simply omits dirty/action indicators for this entry.
        Err(_) => Ok(None),
    }
}

fn inspect_git_repo_sync(path: &Path) -> io::Result<Option<GitRepoSummary>> {
    let Some(repo_root) = try_resolve_repo_root(path)? else {
        return Ok(None);
    };

    let status_short = run_git_capture(&repo_root, &["status", "--short"])?;
    Ok(Some(GitRepoSummary {
        repo_root,
        dirty: !status_short.trim().is_empty(),
    }))
}

fn collect_git_state(cwd: &str) -> io::Result<GitStateSnapshot> {
    let repo_root = resolve_repo_root(cwd)?;
    collect_git_state_from_root(&repo_root)
}

fn collect_git_state_from_root(repo_root: &Path) -> io::Result<GitStateSnapshot> {
    Ok(GitStateSnapshot {
        status_short: run_git_capture(&repo_root, &["status", "--short"])?,
        unstaged_diff_stat: run_git_capture(&repo_root, &["diff", "--stat"])?,
        staged_diff_stat: run_git_capture(&repo_root, &["diff", "--cached", "--stat"])?,
        unstaged_diff: run_git_capture(&repo_root, &["diff"])?,
        staged_diff: run_git_capture(&repo_root, &["diff", "--cached"])?,
        repo_root: repo_root.to_path_buf(),
    })
}

fn resolve_repo_root(cwd: &str) -> io::Result<PathBuf> {
    let cwd = Path::new(cwd);
    let Some(repo_root) = try_resolve_repo_root(cwd)? else {
        return Err(io::Error::other(format!(
            "git repo root lookup failed for {}: not a git repository",
            cwd.display()
        )));
    };

    Ok(repo_root)
}

fn try_resolve_repo_root(path: &Path) -> io::Result<Option<PathBuf>> {
    let output = ProcessCommand::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }

    let repo_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo_root.is_empty() {
        return Ok(None);
    }

    Ok(Some(PathBuf::from(repo_root)))
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

fn run_commit_codex_for_repo(repo_root: &Path) -> io::Result<Option<String>> {
    let git_state = collect_git_state_from_root(repo_root)?;
    let prompt = build_picker_commit_codex_prompt(&git_state);
    run_commit_codex_command(repo_root, prompt, "picker")
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
echo \"=== swimmers commit codex: $SESSION ===\"\n\
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
echo \"\"\n\
if [ \"$EXIT_CODE\" -eq 0 ]; then\n\
  echo \"Finished. Session stays alive for inspection.\"\n\
else\n\
  echo \"Failed. Session stays alive for inspection.\"\n\
fi\n\
echo \"Attach: tmux a -t $SESSION\"\n\
echo \"\"\n\
echo \"Press enter to close, or Ctrl-C to keep session.\"\n\
read -r\n"
    )
}

fn build_commit_codex_prompt(session: &SessionSummary, git_state: &GitStateSnapshot) -> String {
    let repo_root = git_state.repo_root.to_string_lossy();
    format!(
        "$commit\n\n\
You were launched from swimmers by clicking a [commit] opportunity in the clawgs rail.\n\
\n\
Source session:\n\
- tmux: {tmux_name}\n\
- session_id: {session_id}\n\
- cwd: {cwd}\n\
- repo_root: {repo_root}\n\
\n\
Run as a fresh detached Codex commit helper. Use model `{COMMIT_CODEX_MODEL}` with `{COMMIT_CODEX_REASONING}` reasoning.\n\
\n\
{task_body}",
        tmux_name = session.tmux_name,
        session_id = session.session_id,
        cwd = session.cwd,
        repo_root = repo_root,
        task_body = build_commit_task_body(git_state),
    )
}

fn build_picker_commit_codex_prompt(git_state: &GitStateSnapshot) -> String {
    let repo_root = git_state.repo_root.to_string_lossy();
    format!(
        "$commit\n\n\
You were launched from swimmers by clicking [commit] next to a repo in the picker.\n\
\n\
Repo:\n\
- repo_root: {repo_root}\n\
\n\
Run as a fresh detached Codex commit helper. Use model `{COMMIT_CODEX_MODEL}` with `{COMMIT_CODEX_REASONING}` reasoning.\n\
\n\
{task_body}",
        repo_root = repo_root,
        task_body = build_commit_task_body(git_state),
    )
}

fn build_commit_task_body(git_state: &GitStateSnapshot) -> String {
    let repo_root = git_state.repo_root.to_string_lossy();
    format!(
        "Task:\n\
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
        repo_root = repo_root,
        status_short = display_git_output(&git_state.status_short),
        unstaged_diff_stat = display_git_output(&git_state.unstaged_diff_stat),
        staged_diff_stat = display_git_output(&git_state.staged_diff_stat),
        staged_diff = display_git_output(&git_state.staged_diff),
        unstaged_diff = display_git_output(&git_state.unstaged_diff),
    )
}

fn run_commit_codex_command(
    repo_root: &Path,
    prompt: String,
    label: &str,
) -> io::Result<Option<String>> {
    let runtime_dir = std::env::temp_dir().join(COMMIT_CODEX_RUNTIME_DIR);
    fs::create_dir_all(&runtime_dir)?;
    let prompt_path = runtime_dir.join(format!(
        "{label}-{}.prompt.md",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    fs::write(&prompt_path, prompt)?;

    let prompt_file = fs::File::open(&prompt_path)?;
    let output = ProcessCommand::new("codex")
        .arg("exec")
        .arg("-m")
        .arg(COMMIT_CODEX_MODEL)
        .arg("-c")
        .arg(format!(
            "model_reasoning_effort=\"{COMMIT_CODEX_REASONING}\""
        ))
        .arg("--dangerously-bypass-approvals-and-sandbox")
        .arg("--cd")
        .arg(repo_root)
        .arg("-")
        .stdin(Stdio::from(prompt_file))
        .output()?;

    if output.status.success() {
        Ok(command_success_detail(&output))
    } else {
        Err(io::Error::other(command_failure_detail(&output)))
    }
}

fn display_git_output(output: &str) -> &str {
    let trimmed = output.trim_end();
    if trimmed.is_empty() {
        "(no output)"
    } else {
        trimmed
    }
}

fn command_success_detail(output: &Output) -> Option<String> {
    let detail = String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().chars().take(300).collect::<String>());

    detail.filter(|value| !value.is_empty())
}

fn command_failure_detail(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr.chars().take(600).collect();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("repo action failed")
        .trim()
        .chars()
        .take(600)
        .collect()
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
    use chrono::Utc;

    use super::*;
    use crate::types::{RestState, SessionState, ThoughtSource, ThoughtState, TransportHealth};

    fn sample_session() -> SessionSummary {
        SessionSummary {
            session_id: "sess-1".to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Busy,
            current_command: None,
            cwd: "/tmp/repos/swimmers/crate".to_string(),
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 0,
            thought: Some("commit this".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Active,
            commit_candidate: true,
            objective_changed_at: None,
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
                repo_root: PathBuf::from("/tmp/repos/swimmers"),
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

    #[test]
    fn build_commit_tmux_wrapper_keeps_successful_sessions_open_for_inspection() {
        let wrapper = build_commit_tmux_wrapper(
            "commit-7-123",
            Path::new("/tmp/repos/swimmers"),
            Path::new("/tmp/prompt.md"),
        );

        assert!(wrapper.contains("Codex exited with code: $EXIT_CODE"));
        assert!(wrapper.contains("Finished. Session stays alive for inspection."));
        assert!(wrapper.contains("Press enter to close, or Ctrl-C to keep session."));
    }
}
