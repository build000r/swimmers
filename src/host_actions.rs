use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::launcher::{
    prepare_private_dir, shell_single_quote, write_private_file, SpawnToolLauncher,
};
use crate::types::{RepoActionKind, RepoActionState, RepoActionStatus, SessionSummary, SpawnTool};

const COMMIT_TMUX_PREFIX: &str = "commit";
const COMMIT_TMUX_RUNTIME_DIR: &str = "swimmers-commit-tmux";
const REPO_ACTION_STATUS_TTL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitGrokLaunch {
    pub session_name: String,
    pub watch_command: String,
}

pub trait CommitLauncher: Send + Sync {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitGrokLaunch>;
}

#[derive(Default)]
pub struct SystemCommitLauncher;

impl CommitLauncher for SystemCommitLauncher {
    fn launch(&self, session: &SessionSummary) -> io::Result<CommitGrokLaunch> {
        let git_state = collect_git_state(&session.cwd)?;
        launch_commit_grok_for_session_tmux(session, &git_state)
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
            RepoActionKind::Commit => run_commit_grok_for_repo(&repo_root),
            RepoActionKind::Restart | RepoActionKind::Open => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{kind:?} is not handled through the default executor"),
            )),
        }
    }
}

/// Executor for service restart actions that runs overlay-defined shell commands.
pub struct RestartExecutor {
    /// Shell commands to execute in order.
    pub commands: Vec<(String, String)>, // (service_name, shell_command)
}

impl RepoActionExecutor for RestartExecutor {
    fn execute(&self, _repo_root: PathBuf, _kind: RepoActionKind) -> io::Result<Option<String>> {
        for (service_name, cmd) in &self.commands {
            let output = ProcessCommand::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let detail: String = if stderr.is_empty() {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .rev()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("restart failed")
                        .trim()
                        .chars()
                        .take(600)
                        .collect()
                } else {
                    stderr.chars().take(600).collect()
                };
                return Err(io::Error::other(format!("{service_name}: {detail}")));
            }
        }
        let names: Vec<&str> = self.commands.iter().map(|(n, _)| n.as_str()).collect();
        Ok(Some(format!("restarted {}", names.join(", "))))
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

/// TTL for the `inspect_git_repo` per-path memo. `list_dirs` is the hot caller
/// and the TUI's directory picker may refire it 2–5× per session as the user
/// changes filters; reusing probes within the window cuts each retry to ~µs.
/// Write paths (`start_dir_repo_action`) call [`invalidate_inspect_git_repo`]
/// after kicking off a mutation so the next read sees fresh `dirty` state.
const INSPECT_GIT_REPO_CACHE_TTL: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct InspectGitRepoCacheEntry {
    fetched_at: Instant,
    summary: Option<GitRepoSummary>,
}

static INSPECT_GIT_REPO_CACHE: std::sync::OnceLock<
    std::sync::Mutex<HashMap<PathBuf, InspectGitRepoCacheEntry>>,
> = std::sync::OnceLock::new();

fn inspect_git_repo_cache() -> &'static std::sync::Mutex<HashMap<PathBuf, InspectGitRepoCacheEntry>>
{
    INSPECT_GIT_REPO_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn lookup_cached_inspect_git_repo(path: &Path) -> Option<Option<GitRepoSummary>> {
    let guard = inspect_git_repo_cache().lock().ok()?;
    let entry = guard.get(path)?;
    if entry.fetched_at.elapsed() < INSPECT_GIT_REPO_CACHE_TTL {
        Some(entry.summary.clone())
    } else {
        None
    }
}

fn store_cached_inspect_git_repo(path: &Path, summary: &Option<GitRepoSummary>) {
    if let Ok(mut guard) = inspect_git_repo_cache().lock() {
        guard.insert(
            path.to_path_buf(),
            InspectGitRepoCacheEntry {
                fetched_at: Instant::now(),
                summary: summary.clone(),
            },
        );
    }
}

/// Drops cached `inspect_git_repo` results for the given path. Call this after
/// any mutation that may change the repo's dirty state (commit, reset, etc.)
/// so the next read picks up the new status instead of the cached one.
pub fn invalidate_inspect_git_repo(path: &Path) {
    let Some(cache) = INSPECT_GIT_REPO_CACHE.get() else {
        return;
    };
    if let Ok(mut guard) = cache.lock() {
        guard.remove(path);
    }
}

/// Async probe: offloads the blocking git subprocess calls to Tokio's blocking
/// pool via `spawn_blocking`. Using `tokio::process::Command` with many
/// concurrent children wedged the runtime on macOS (concurrent process-wait
/// contention), so we go through the blocking pool instead — it's sized for
/// exactly this pattern and keeps the main worker threads free.
///
/// Memoized for [`INSPECT_GIT_REPO_CACHE_TTL`]; mutating code paths must call
/// [`invalidate_inspect_git_repo`] to clear stale `dirty` entries.
pub async fn inspect_git_repo(path: &Path) -> io::Result<Option<GitRepoSummary>> {
    if let Some(cached) = lookup_cached_inspect_git_repo(path) {
        return Ok(cached);
    }

    let owned_path = path.to_path_buf();
    let join_handle = tokio::task::spawn_blocking({
        let owned_path = owned_path.clone();
        move || inspect_git_repo_sync(&owned_path)
    });
    let result = match tokio::time::timeout(GIT_PROBE_TIMEOUT, join_handle).await {
        Ok(Ok(result)) => result?,
        Ok(Err(join_err)) => {
            return Err(io::Error::other(format!(
                "inspect_git_repo task failed: {join_err}"
            )))
        }
        // Timed out: let the blocking task finish detached and report "no repo"
        // so the picker simply omits dirty/action indicators for this entry.
        // Don't cache timeouts — a hung git is likely transient and we want the
        // next call to retry.
        Err(_) => return Ok(None),
    };

    store_cached_inspect_git_repo(&owned_path, &result);
    Ok(result)
}

fn inspect_git_repo_sync(path: &Path) -> io::Result<Option<GitRepoSummary>> {
    let repo_root = match direct_repo_root(path)? {
        Some(repo_root) => Some(repo_root),
        None => try_resolve_repo_root(path)?,
    };
    let Some(repo_root) = repo_root else {
        return Ok(None);
    };

    let status_short = run_git_capture(&repo_root, &["status", "--short"])?;
    Ok(Some(GitRepoSummary {
        repo_root,
        dirty: !status_short.trim().is_empty(),
    }))
}

fn direct_repo_root(path: &Path) -> io::Result<Option<PathBuf>> {
    if !path.join(".git").exists() {
        return Ok(None);
    }
    path.canonicalize().map(Some)
}

fn collect_git_state(cwd: &str) -> io::Result<GitStateSnapshot> {
    let repo_root = resolve_repo_root(cwd)?;
    collect_git_state_from_root(&repo_root)
}

fn collect_git_state_from_root(repo_root: &Path) -> io::Result<GitStateSnapshot> {
    Ok(GitStateSnapshot {
        status_short: run_git_capture(repo_root, &["status", "--short"])?,
        unstaged_diff_stat: run_git_capture(repo_root, &["diff", "--stat"])?,
        staged_diff_stat: run_git_capture(repo_root, &["diff", "--cached", "--stat"])?,
        unstaged_diff: run_git_capture(repo_root, &["diff"])?,
        staged_diff: run_git_capture(repo_root, &["diff", "--cached"])?,
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

fn launch_commit_grok_for_session_tmux(
    session: &SessionSummary,
    git_state: &GitStateSnapshot,
) -> io::Result<CommitGrokLaunch> {
    launch_commit_grok_tmux(
        &session.tmux_name,
        &git_state.repo_root,
        build_commit_grok_prompt(session, git_state),
    )
}

fn launch_commit_grok_tmux(
    session_label: &str,
    repo_root: &Path,
    prompt: String,
) -> io::Result<CommitGrokLaunch> {
    let session_name = commit_tmux_session_name(session_label);
    let runtime_dir = std::env::temp_dir().join(COMMIT_TMUX_RUNTIME_DIR);
    prepare_private_dir(&runtime_dir)?;

    // The session name includes a millisecond timestamp, but two launches that
    // share `(tmux_name, ms)` would collide on the prompt/wrapper file paths
    // and potentially deliver the wrong prompt to the surviving tmux session
    // if the loser overwrites the wrapper before tmux starts reading it.
    // A per-launch nonce is cheaper than reasoning about that race.
    let nonce = Uuid::new_v4();
    let prompt_path = runtime_dir.join(format!("{session_name}-{nonce}.prompt.md"));
    let wrapper_path = runtime_dir.join(format!("{session_name}-{nonce}.sh"));
    write_private_file(&prompt_path, &prompt)?;
    fs::write(
        &wrapper_path,
        build_commit_tmux_wrapper(&session_name, repo_root, &prompt_path),
    )?;

    let repo_root = repo_root.to_string_lossy().into_owned();
    let wrapper_command = format!(
        "bash {}",
        shell_single_quote(&wrapper_path.to_string_lossy())
    );
    let output = ProcessCommand::new("tmux")
        .args(["new-session", "-d", "-s", &session_name, "-c", &repo_root])
        .arg(wrapper_command)
        .output()?;
    if !output.status.success() {
        let _ = fs::remove_file(&prompt_path);
        let _ = fs::remove_file(&wrapper_path);
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

    Ok(CommitGrokLaunch {
        watch_command: format!("tmux a -t {session_name}"),
        session_name,
    })
}

fn run_commit_grok_for_repo(repo_root: &Path) -> io::Result<Option<String>> {
    let git_state = collect_git_state_from_root(repo_root)?;
    let session_label = repo_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("repo");
    let launch = launch_commit_grok_tmux(
        session_label,
        &git_state.repo_root,
        build_picker_commit_grok_prompt(&git_state),
    )?;
    Ok(Some(format!("commit grok: {}", launch.watch_command)))
}

fn build_commit_tmux_wrapper(session_name: &str, repo_root: &Path, prompt_path: &Path) -> String {
    build_commit_tmux_wrapper_with_launcher(
        session_name,
        repo_root,
        prompt_path,
        SpawnToolLauncher::from_env(SpawnTool::Grok),
    )
}

fn build_commit_tmux_wrapper_with_launcher(
    session_name: &str,
    repo_root: &Path,
    prompt_path: &Path,
    launcher: SpawnToolLauncher,
) -> String {
    let repo_root = shell_single_quote(&repo_root.to_string_lossy());
    let prompt_path = shell_single_quote(&prompt_path.to_string_lossy());
    let grok_program = launcher.shell_program();

    format!(
        "#!/bin/bash\n\
SESSION={session_name:?}\n\
REPO_DIR={repo_root}\n\
PROMPT_FILE={prompt_path}\n\
WRAPPER_FILE=$0\n\
\n\
cleanup_prompt() {{\n\
  rm -f \"$PROMPT_FILE\" \"$WRAPPER_FILE\"\n\
}}\n\
trap cleanup_prompt EXIT\n\
\n\
echo \"=== swimmers commit grok: $SESSION ===\"\n\
echo \"Repo: $REPO_DIR\"\n\
echo \"Started: $(date)\"\n\
echo \"\"\n\
\n\
EXIT_CODE=0\n\
{grok_program} \\\n\
  --prompt-file \"$PROMPT_FILE\" \\\n\
  --cwd \"$REPO_DIR\" \\\n\
  --always-approve \\\n\
  --no-alt-screen || EXIT_CODE=$?\n\
cleanup_prompt\n\
\n\
echo \"\"\n\
echo \"Grok exited with code: $EXIT_CODE\"\n\
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

fn build_commit_grok_prompt(session: &SessionSummary, git_state: &GitStateSnapshot) -> String {
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
Run as a fresh detached Grok headless commit helper. Use the named Grok session for this one-shot commit task.\n\
\n\
{task_body}",
        tmux_name = session.tmux_name,
        session_id = session.session_id,
        cwd = session.cwd,
        repo_root = repo_root,
        task_body = build_commit_task_body(git_state),
    )
}

fn build_picker_commit_grok_prompt(git_state: &GitStateSnapshot) -> String {
    let repo_root = git_state.repo_root.to_string_lossy();
    format!(
        "$commit\n\n\
You were launched from swimmers by clicking [commit] next to a repo in the picker.\n\
\n\
Repo:\n\
- repo_root: {repo_root}\n\
\n\
Run as a fresh detached Grok headless commit helper. Use the named Grok session for this one-shot commit task.\n\
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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    use super::*;
    use crate::types::{RestState, SessionState, ThoughtSource, ThoughtState, TransportHealth};

    fn sample_session() -> SessionSummary {
        SessionSummary {
            session_id: "sess-1".to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Busy,
            current_command: None,
            state_evidence: Default::default(),
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

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write executable");
        let mut perms = fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: OsString) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn test_path_with_prepend(bin_dir: &Path) -> OsString {
        let mut entries = vec![bin_dir.as_os_str().to_os_string()];
        if let Some(existing) = std::env::var_os("PATH") {
            entries.extend(std::env::split_paths(&existing).map(|path| path.into_os_string()));
        }
        std::env::join_paths(entries).expect("join PATH")
    }

    fn init_dirty_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("repo dir");
        let status = ProcessCommand::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");
        fs::write(path.join("README.md"), "dirty\n").expect("write readme");
    }

    #[test]
    fn inspect_git_repo_skips_rev_parse_for_direct_repo_root() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join(".git")).expect("direct git marker");

        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        let git_log = temp.path().join("git.log");
        write_executable(
            &bin_dir.join("git"),
            &format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> {}
if [ "${{1-}}" = "-C" ]; then
  shift 2
fi
case "${{1-}}" in
  status)
    printf ' M README.md\n'
    ;;
  rev-parse)
    printf 'unexpected rev-parse\n' >&2
    exit 44
    ;;
esac
"#,
                shell_single_quote(&git_log.to_string_lossy())
            ),
        );
        let _path_guard = EnvVarGuard::set("PATH", test_path_with_prepend(&bin_dir));

        let summary = inspect_git_repo_sync(&repo)
            .expect("inspect direct repo")
            .expect("repo summary");

        assert_eq!(
            summary.repo_root,
            repo.canonicalize().expect("canonical repo")
        );
        assert!(summary.dirty);
        let git_log = fs::read_to_string(git_log).expect("git log");
        assert!(git_log.contains("status --short"));
        assert!(!git_log.contains("rev-parse"));
    }

    #[test]
    fn build_commit_grok_prompt_includes_preloaded_git_state() {
        let prompt = build_commit_grok_prompt(
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
        assert!(prompt.contains("Grok headless commit helper"));
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
    fn commit_runtime_path_format_uses_uuid_to_avoid_concurrent_overwrite() {
        // Regression: two launches that shared `(tmux_name, millisecond)` used
        // to share prompt/wrapper paths. A per-launch UUID keeps both files
        // isolated even when the tmux session-name prefix collides.
        let runtime_dir = std::env::temp_dir().join(COMMIT_TMUX_RUNTIME_DIR);
        let session_name = "commit-picker-123";
        let nonce_a = Uuid::new_v4();
        let nonce_b = Uuid::new_v4();
        let path_a = runtime_dir.join(format!("{session_name}-{nonce_a}.prompt.md"));
        let path_b = runtime_dir.join(format!("{session_name}-{nonce_b}.prompt.md"));
        assert_ne!(
            nonce_a, nonce_b,
            "uuid::Uuid::new_v4 must produce distinct values"
        );
        assert_ne!(
            path_a, path_b,
            "concurrent picker commits must never share a prompt path"
        );
    }

    #[test]
    fn build_commit_tmux_wrapper_keeps_successful_sessions_open_for_inspection() {
        let wrapper = build_commit_tmux_wrapper(
            "commit-7-123",
            Path::new("/tmp/repos/swimmers"),
            Path::new("/tmp/prompt.md"),
        );

        assert!(wrapper.contains("grok \\"));
        assert!(wrapper.contains("--prompt-file \"$PROMPT_FILE\""));
        assert!(wrapper.contains("--cwd \"$REPO_DIR\""));
        assert!(!wrapper.contains(r#"-p "$(cat "$PROMPT_FILE")""#));
        assert!(!wrapper.contains("--session-id"));
        assert!(!wrapper.contains("--output-format"));
        assert!(!wrapper.contains("--max-turns"));
        assert!(wrapper.contains("Grok exited with code: $EXIT_CODE"));
        assert!(wrapper.contains("trap cleanup_prompt EXIT"));
        assert!(wrapper.contains("WRAPPER_FILE=$0"));
        assert!(wrapper.contains("rm -f \"$PROMPT_FILE\" \"$WRAPPER_FILE\""));
        assert!(
            wrapper
                .find("--no-alt-screen || EXIT_CODE=$?\ncleanup_prompt")
                .expect("prompt cleanup after Grok exits")
                < wrapper
                    .find("Grok exited with code")
                    .expect("exit status line"),
            "prompt file should be removed before the inspectable-pane wait"
        );
        assert!(wrapper.contains("Finished. Session stays alive for inspection."));
        assert!(wrapper.contains("Press enter to close, or Ctrl-C to keep session."));
    }

    #[test]
    fn build_commit_tmux_wrapper_shell_quotes_grok_override() {
        let wrapper = build_commit_tmux_wrapper_with_launcher(
            "commit-7-123",
            Path::new("/tmp/repos/swimmers"),
            Path::new("/tmp/prompt.md"),
            SpawnToolLauncher::with_program_override(
                SpawnTool::Grok,
                Some(std::ffi::OsString::from("/tmp/agent bins/grok wrapper")),
            ),
        );

        assert!(wrapper.contains("'/tmp/agent bins/grok wrapper' \\"));
        assert!(wrapper.contains("--prompt-file \"$PROMPT_FILE\""));
    }

    #[test]
    fn picker_commit_action_launches_tmux_grok_helper() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("swimmers");
        init_dirty_git_repo(&repo);

        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux_log = temp.path().join("tmux-args.txt");
        write_executable(
            &bin_dir.join("tmux"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n",
                shell_single_quote(&tmux_log.to_string_lossy())
            ),
        );
        let fake_grok = temp.path().join("agent bins").join("grok wrapper");
        fs::create_dir_all(fake_grok.parent().expect("fake grok parent")).expect("fake grok dir");
        write_executable(&fake_grok, "#!/bin/sh\nexit 0\n");
        let _path_guard = EnvVarGuard::set("PATH", test_path_with_prepend(&bin_dir));
        let _grok_guard = EnvVarGuard::set(
            crate::launcher::SWIMMERS_GROK_BIN_ENV,
            fake_grok.as_os_str().to_os_string(),
        );

        let detail = run_commit_grok_for_repo(&repo).expect("run picker commit helper");

        let detail = detail.expect("watch command detail");
        assert!(detail.starts_with("commit grok: tmux a -t commit-swimmers-"));
        let tmux_args = fs::read_to_string(&tmux_log).expect("tmux args");
        assert!(tmux_args.contains("new-session\n-d\n-s\ncommit-swimmers-"));
        assert!(tmux_args.contains(&format!("-c\n{}\n", repo.to_string_lossy())));
        let wrapper_command = tmux_args.lines().last().expect("wrapper command");
        let wrapper_path = wrapper_command
            .strip_prefix("bash '")
            .and_then(|path| path.strip_suffix('\''))
            .expect("quoted wrapper path");
        let wrapper = fs::read_to_string(wrapper_path).expect("wrapper script");
        assert!(wrapper.contains(&format!(
            "{} \\",
            shell_single_quote(&fake_grok.to_string_lossy())
        )));
        assert!(wrapper.contains("--prompt-file \"$PROMPT_FILE\""));
        assert!(wrapper.contains("--cwd \"$REPO_DIR\""));
        assert!(!wrapper.contains("--session-id"));
        assert!(!wrapper.contains("--output-format"));
        assert!(!wrapper.contains("--max-turns"));
        let prompt_path = wrapper
            .lines()
            .find_map(|line| line.strip_prefix("PROMPT_FILE='"))
            .and_then(|path| path.strip_suffix('\''))
            .expect("prompt path");
        let prompt = fs::read_to_string(prompt_path).expect("prompt file");
        assert!(prompt.contains("You were launched from swimmers by clicking [commit]"));
        assert!(prompt.contains("## git status --short"));
        let _ = fs::remove_file(prompt_path);
        let _ = fs::remove_file(wrapper_path);
    }
}
