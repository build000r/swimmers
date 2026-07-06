use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::debug;

#[cfg(test)]
use crate::tmux_target::TmuxTarget;

use super::liveness;
#[cfg(test)]
use super::metadata::query_tmux_pane_metadata;
use super::metadata::TmuxPaneMetadata;

const PROCESS_ENTRIES_QUERY_TIMEOUT: Duration = Duration::from_millis(750);
pub(super) const PROCESS_ENTRIES_CACHE_TTL: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone)]
pub(super) struct ProcessEntry {
    pub(super) pid: u32,
    pub(super) ppid: u32,
    pub(super) pcpu: f32,
    pub(super) comm: String,
    pub(super) args: String,
}

#[derive(Debug, Default)]
pub(super) struct ProcessEntriesCache {
    pub(super) fetched_at: Option<Instant>,
    pub(super) entries: Vec<ProcessEntry>,
}

struct ProcessEntriesSnapshot {
    entries: Vec<ProcessEntry>,
    fresh: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum ProcessSnapshotToolDetection {
    Detected(String),
    Stale,
    NotFound,
}

pub(super) struct ProcessTreeIndex {
    pub(super) by_pid: HashMap<u32, ProcessEntry>,
    pub(super) children: HashMap<u32, Vec<u32>>,
}

impl ProcessTreeIndex {
    pub(super) fn from_entries(entries: Vec<ProcessEntry>) -> Self {
        let mut by_pid = HashMap::new();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();

        for entry in entries {
            children.entry(entry.ppid).or_default().push(entry.pid);
            by_pid.insert(entry.pid, entry);
        }

        Self { by_pid, children }
    }

    fn detect_tool_bfs(&self, root_pid: u32) -> Option<&'static str> {
        let mut queue = VecDeque::from([root_pid]);
        let mut visited = HashSet::new();

        while let Some(pid) = queue.pop_front() {
            if !visited.insert(pid) {
                continue;
            }

            if let Some(tool) = self
                .by_pid
                .get(&pid)
                .and_then(detect_tool_from_process_entry)
            {
                return Some(tool);
            }

            if let Some(child_pids) = self.children.get(&pid) {
                queue.extend(child_pids.iter().copied());
            }
        }

        None
    }
}

static PROCESS_ENTRIES_CACHE: OnceLock<Mutex<ProcessEntriesCache>> = OnceLock::new();

pub(super) fn process_entries_cache() -> &'static Mutex<ProcessEntriesCache> {
    PROCESS_ENTRIES_CACHE.get_or_init(|| Mutex::new(ProcessEntriesCache::default()))
}

#[cfg(test)]
pub(super) async fn query_tool_from_tmux_process_tree(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
) -> anyhow::Result<Option<String>> {
    let metadata = query_tmux_pane_metadata(tmux_name, tmux_target).await?;
    detect_tool_from_tmux_pane_metadata(&metadata).await
}

pub(super) async fn detect_tool_from_tmux_pane_metadata(
    metadata: &TmuxPaneMetadata,
) -> anyhow::Result<Option<String>> {
    if let Ok(comm) = metadata.current_command() {
        if let Some(tool) = crate::types::detect_tool_name(comm) {
            return Ok(Some(tool.to_string()));
        }
    }

    let pane_pid = metadata.pane_pid()?;
    let snapshot = query_process_entries().await?;

    match detect_tool_from_process_snapshot(pane_pid, snapshot) {
        ProcessSnapshotToolDetection::Detected(tool) => Ok(Some(tool)),
        ProcessSnapshotToolDetection::Stale => {
            debug!("skipping tool detection from stale process snapshot");
            Ok(None)
        }
        ProcessSnapshotToolDetection::NotFound => Ok(None),
    }
}

fn detect_tool_from_process_snapshot(
    pane_pid: u32,
    snapshot: ProcessEntriesSnapshot,
) -> ProcessSnapshotToolDetection {
    if !snapshot.fresh {
        return ProcessSnapshotToolDetection::Stale;
    }

    ProcessTreeIndex::from_entries(snapshot.entries)
        .detect_tool_bfs(pane_pid)
        .map(|tool| ProcessSnapshotToolDetection::Detected(tool.to_string()))
        .unwrap_or(ProcessSnapshotToolDetection::NotFound)
}

/// Result of a process-tree liveness check for a tmux pane.
#[derive(Debug, Clone, Copy)]
pub(super) struct PaneLiveness {
    /// True when the pane's shell has at least one child process.
    pub(super) has_children: bool,
    /// Sum of `%cpu` across all descendant processes (excludes the shell itself).
    #[allow(dead_code)]
    pub(super) descendant_cpu: f32,
    /// True only when the process tree came from a fresh `ps` snapshot.
    pub(super) process_snapshot_fresh: bool,
}

/// Query whether the pane's shell process has running children and their
/// aggregate CPU usage. This is the ground-truth signal for idle vs busy:
/// if the shell is the leaf process, no command is running regardless of what
/// the terminal output looks like.
pub(super) async fn query_pane_liveness_for_pid(pane_pid: u32) -> anyhow::Result<PaneLiveness> {
    let snapshot = query_process_entries().await?;
    let mut liveness = compute_pane_liveness(pane_pid, snapshot.entries);
    liveness.process_snapshot_fresh = snapshot.fresh;
    Ok(liveness)
}

/// Pure BFS over the process tree rooted at `pane_pid`. Exported for testing.
fn compute_pane_liveness(pane_pid: u32, entries: Vec<ProcessEntry>) -> PaneLiveness {
    liveness::compute_pane_liveness(pane_pid, entries)
}

async fn query_process_entries() -> anyhow::Result<ProcessEntriesSnapshot> {
    let mut cache = process_entries_cache().lock().await;
    if cache
        .fetched_at
        .map(|fetched_at| fetched_at.elapsed() <= PROCESS_ENTRIES_CACHE_TTL)
        .unwrap_or(false)
    {
        return Ok(ProcessEntriesSnapshot {
            entries: cache.entries.clone(),
            fresh: true,
        });
    }

    match query_process_entries_uncached().await {
        Ok(entries) => {
            cache.fetched_at = Some(Instant::now());
            cache.entries = entries.clone();
            Ok(ProcessEntriesSnapshot {
                entries,
                fresh: true,
            })
        }
        Err(err) if !cache.entries.is_empty() => {
            debug!(
                "using stale process snapshot after ps refresh failed: {}",
                err
            );
            Ok(ProcessEntriesSnapshot {
                entries: cache.entries.clone(),
                fresh: false,
            })
        }
        Err(err) => Err(err),
    }
}

async fn query_process_entries_uncached() -> anyhow::Result<Vec<ProcessEntry>> {
    let mut command = Command::new("ps");
    command
        .args(["-axo", "pid=,ppid=,pcpu=,comm=,args="])
        .kill_on_drop(true);

    let output = tokio::time::timeout(PROCESS_ENTRIES_QUERY_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "ps timed out after {}ms",
                PROCESS_ENTRIES_QUERY_TIMEOUT.as_millis()
            )
        })?
        .map_err(|e| anyhow::anyhow!("failed to run ps: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("ps failed: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in stdout.lines() {
        if let Some(entry) = parse_process_entry(line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn parse_process_entry(line: &str) -> Option<ProcessEntry> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let ppid = parts.next()?.parse::<u32>().ok()?;
    let pcpu = parts.next()?.parse::<f32>().ok()?;
    let comm = parts.next()?.to_string();
    let args = parts.collect::<Vec<&str>>().join(" ");

    Some(ProcessEntry {
        pid,
        ppid,
        pcpu,
        comm,
        args,
    })
}

fn detect_tool_from_process_entry(entry: &ProcessEntry) -> Option<&'static str> {
    crate::types::detect_tool_name(&entry.comm)
        .or_else(|| detect_tool_from_command_line(&entry.args))
}

fn detect_tool_from_command_line(command: &str) -> Option<&'static str> {
    command
        .split_whitespace()
        .find_map(crate::types::detect_tool_name)
}

pub(super) fn current_command_tool_update(
    current_command: Option<&str>,
    current_tool: Option<&str>,
) -> Option<&'static str> {
    let tool = current_command.and_then(detect_tool_from_command_line)?;
    (current_tool != Some(tool)).then_some(tool)
}

#[cfg(test)]
mod tests {
    use super::{
        compute_pane_liveness, detect_tool_from_command_line, detect_tool_from_process_entry,
        detect_tool_from_process_snapshot, parse_process_entry, process_entries_cache,
        query_tool_from_tmux_process_tree, ProcessEntriesCache, ProcessEntriesSnapshot,
        ProcessEntry, ProcessSnapshotToolDetection, PROCESS_ENTRIES_CACHE_TTL,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    async fn clear_process_entries_cache() {
        let mut cache = process_entries_cache().lock().await;
        *cache = ProcessEntriesCache::default();
    }

    async fn seed_process_entries_cache(entries: Vec<ProcessEntry>, fetched_at: Instant) {
        let mut cache = process_entries_cache().lock().await;
        cache.fetched_at = Some(fetched_at);
        cache.entries = entries;
    }

    fn restore_path(previous_path: Option<std::ffi::OsString>) {
        if let Some(value) = previous_path {
            std::env::set_var("PATH", value);
        } else {
            std::env::remove_var("PATH");
        }
    }

    fn make_executable(path: &std::path::Path) {
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    fn proc(pid: u32, ppid: u32, pcpu: f32) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid,
            pcpu,
            comm: "test".to_string(),
            args: String::new(),
        }
    }

    fn tool_proc(pid: u32, ppid: u32, comm: &str, args: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            ppid,
            pcpu: 0.0,
            comm: comm.to_string(),
            args: args.to_string(),
        }
    }

    #[test]
    fn detect_tool_from_command_line_handles_aliases() {
        assert_eq!(
            detect_tool_from_command_line("FOO=1 /usr/local/bin/claude-code --print"),
            Some("Claude Code")
        );
        assert_eq!(
            detect_tool_from_command_line("codex-cli --help"),
            Some("Codex")
        );
    }

    #[test]
    fn parse_process_entry_parses_ps_row() {
        let entry =
            parse_process_entry("10715 37039 2.3 claude /usr/local/bin/claude --print").unwrap();
        assert_eq!(entry.pid, 10_715);
        assert_eq!(entry.ppid, 37_039);
        assert!((entry.pcpu - 2.3).abs() < f32::EPSILON);
        assert_eq!(entry.comm, "claude");
        assert_eq!(entry.args, "/usr/local/bin/claude --print");
    }

    #[test]
    fn detect_tool_from_process_entry_checks_comm_then_args() {
        let from_comm = ProcessEntry {
            pid: 1,
            ppid: 0,
            pcpu: 0.0,
            comm: "codex".to_string(),
            args: "codex".to_string(),
        };
        assert_eq!(detect_tool_from_process_entry(&from_comm), Some("Codex"));

        let from_args = ProcessEntry {
            pid: 2,
            ppid: 1,
            pcpu: 0.0,
            comm: "node".to_string(),
            args: "/usr/local/bin/claude --json".to_string(),
        };
        assert_eq!(
            detect_tool_from_process_entry(&from_args),
            Some("Claude Code")
        );
    }

    #[test]
    fn query_tool_from_tmux_process_tree_helper_detects_comm_before_args() {
        let from_comm = ProcessEntriesSnapshot {
            fresh: true,
            entries: vec![
                tool_proc(101, 1, "bash", "bash"),
                tool_proc(102, 101, "codex", "/usr/local/bin/claude --print"),
            ],
        };
        assert_eq!(
            detect_tool_from_process_snapshot(101, from_comm),
            ProcessSnapshotToolDetection::Detected("Codex".to_string())
        );

        let from_args = ProcessEntriesSnapshot {
            fresh: true,
            entries: vec![
                tool_proc(101, 1, "bash", "bash"),
                tool_proc(102, 101, "node", "/usr/local/bin/claude --print"),
            ],
        };
        assert_eq!(
            detect_tool_from_process_snapshot(101, from_args),
            ProcessSnapshotToolDetection::Detected("Claude Code".to_string())
        );
    }

    #[test]
    fn query_tool_from_tmux_process_tree_helper_uses_bfs_order() {
        let snapshot = ProcessEntriesSnapshot {
            fresh: true,
            entries: vec![
                tool_proc(101, 1, "bash", "bash"),
                tool_proc(102, 101, "node", "node worker"),
                tool_proc(103, 101, "codex", "codex"),
                tool_proc(104, 102, "claude", "claude"),
            ],
        };

        assert_eq!(
            detect_tool_from_process_snapshot(101, snapshot),
            ProcessSnapshotToolDetection::Detected("Codex".to_string())
        );
    }

    #[test]
    fn query_tool_from_tmux_process_tree_helper_preserves_child_order() {
        let snapshot = ProcessEntriesSnapshot {
            fresh: true,
            entries: vec![
                tool_proc(101, 1, "bash", "bash"),
                tool_proc(102, 101, "claude", "claude"),
                tool_proc(103, 101, "codex", "codex"),
            ],
        };

        assert_eq!(
            detect_tool_from_process_snapshot(101, snapshot),
            ProcessSnapshotToolDetection::Detected("Claude Code".to_string())
        );
    }

    #[test]
    fn query_tool_from_tmux_process_tree_helper_handles_cycles() {
        let snapshot = ProcessEntriesSnapshot {
            fresh: true,
            entries: vec![
                tool_proc(101, 103, "bash", "bash"),
                tool_proc(102, 101, "node", "node worker"),
                tool_proc(103, 102, "python", "python worker"),
            ],
        };

        assert_eq!(
            detect_tool_from_process_snapshot(101, snapshot),
            ProcessSnapshotToolDetection::NotFound
        );
    }

    #[test]
    fn query_tool_from_tmux_process_tree_helper_marks_stale_snapshots() {
        let snapshot = ProcessEntriesSnapshot {
            fresh: false,
            entries: vec![
                tool_proc(101, 1, "bash", "bash"),
                tool_proc(102, 101, "codex", "codex"),
            ],
        };

        assert_eq!(
            detect_tool_from_process_snapshot(101, snapshot),
            ProcessSnapshotToolDetection::Stale
        );
    }

    #[tokio::test]
    async fn query_tool_from_tmux_process_tree_uses_current_command_fast_path() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let tmux = bin_dir.join("tmux");
        let tmux_log = dir.path().join("tmux.log");
        std::fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf '/tmp/project\\037codex\\037101\\0371774274168\\n'\n",
                tmux_log.display()
            ),
        )
        .expect("tmux");
        make_executable(&tmux);

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let tool =
            query_tool_from_tmux_process_tree("demo", &crate::tmux_target::TmuxTarget::Default)
                .await
                .expect("tool query");
        assert_eq!(tool.as_deref(), Some("Codex"));
        let calls = std::fs::read_to_string(&tmux_log).expect("tmux log");
        assert_eq!(calls.lines().count(), 1);
        assert!(calls.contains("#{pane_current_path}"));
        assert!(calls.contains("#{pane_current_command}"));
        assert!(calls.contains("#{pane_pid}"));
        assert!(calls.contains("#{session_created}"));

        restore_path(previous_path);
    }

    #[tokio::test]
    async fn query_tool_from_tmux_process_tree_walks_process_children_when_needed() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_process_entries_cache().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        let tmux = bin_dir.join("tmux");
        let tmux_log = dir.path().join("tmux.log");
        std::fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf '/tmp/project\\037bash\\037101\\0371774274168\\n'\n",
                tmux_log.display()
            ),
        )
        .expect("tmux");
        let ps = bin_dir.join("ps");
        std::fs::write(
            &ps,
            "#!/bin/sh\nprintf '101 1 0.0 bash bash\\n102 101 5.2 node /usr/local/bin/claude --print\\n'\n",
        )
        .expect("ps");
        for path in [&tmux, &ps] {
            make_executable(path);
        }

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );

        let tool =
            query_tool_from_tmux_process_tree("demo", &crate::tmux_target::TmuxTarget::Default)
                .await
                .expect("tool query");
        assert_eq!(tool.as_deref(), Some("Claude Code"));
        assert_eq!(
            std::fs::read_to_string(&tmux_log)
                .expect("tmux log")
                .lines()
                .count(),
            1
        );

        restore_path(previous_path);
        clear_process_entries_cache().await;
    }

    #[tokio::test]
    async fn query_tool_from_tmux_process_tree_skips_stale_process_cache() {
        let _guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_process_entries_cache().await;
        let dir = tempfile::tempdir().expect("tempdir");
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");

        let tmux = bin_dir.join("tmux");
        std::fs::write(
            &tmux,
            "#!/bin/sh\nprintf '/tmp/project\\037bash\\037101\\0371774274168\\n'\n",
        )
        .expect("tmux");
        let ps = bin_dir.join("ps");
        std::fs::write(&ps, "#!/bin/sh\nprintf 'ps unavailable\\n' >&2\nexit 1\n").expect("ps");
        make_executable(&tmux);
        make_executable(&ps);

        let previous_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([bin_dir.as_path()]).expect("path"),
        );
        seed_process_entries_cache(
            vec![
                ProcessEntry {
                    pid: 101,
                    ppid: 1,
                    pcpu: 0.0,
                    comm: "bash".to_string(),
                    args: "bash".to_string(),
                },
                ProcessEntry {
                    pid: 102,
                    ppid: 101,
                    pcpu: 0.0,
                    comm: "node".to_string(),
                    args: "/usr/local/bin/claude --print".to_string(),
                },
            ],
            Instant::now() - PROCESS_ENTRIES_CACHE_TTL - Duration::from_millis(1),
        )
        .await;

        let tool =
            query_tool_from_tmux_process_tree("demo", &crate::tmux_target::TmuxTarget::Default)
                .await
                .expect("tool query");
        assert_eq!(tool, None);

        restore_path(previous_path);
        clear_process_entries_cache().await;
    }

    #[test]
    fn compute_pane_liveness_idle_shell_has_no_children() {
        let liveness = compute_pane_liveness(100, vec![proc(99, 1, 0.0), proc(101, 99, 0.0)]);
        assert!(!liveness.has_children);
        assert_eq!(liveness.descendant_cpu, 0.0);
    }

    #[test]
    fn compute_pane_liveness_direct_child_marks_busy() {
        let liveness = compute_pane_liveness(100, vec![proc(100, 1, 0.0), proc(101, 100, 2.5)]);
        assert!(liveness.has_children);
        assert!((liveness.descendant_cpu - 2.5).abs() < 0.01);
    }

    #[test]
    fn compute_pane_liveness_sums_deep_descendant_cpu() {
        let entries = vec![proc(100, 1, 0.0), proc(101, 100, 1.0), proc(102, 101, 3.0)];
        let liveness = compute_pane_liveness(100, entries);
        assert!(liveness.has_children);
        assert!((liveness.descendant_cpu - 4.0).abs() < 0.01);
    }

    #[test]
    fn compute_pane_liveness_empty_process_list_is_idle() {
        let liveness = compute_pane_liveness(100, vec![]);
        assert!(!liveness.has_children);
    }
}
