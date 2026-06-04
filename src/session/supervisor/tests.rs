use super::*;
use crate::types::{ActionCueConfidence, ActionCueKind, ActionCueSource, ActionCueStatus};
use chrono::{DateTime, Utc};
use std::iter::FromIterator;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;
use tokio::sync::mpsc;

mod discovery_adoption;
mod list_sessions;
mod persistence;
mod spawn_commands;
mod thought_snapshots;
mod tmux_commands;

fn test_summary(session_id: &str, state: SessionState) -> SessionSummary {
    let mut summary = SessionSummary::live(
        session_id,
        format!("tmux-{session_id}"),
        state,
        Some("cargo test".to_string()),
        Default::default(),
        "/tmp/project",
        Some("Codex".to_string()),
        0,
        0,
        Utc::now(),
    );
    summary.rest_state = fallback_rest_state(state, ThoughtState::Holding);
    summary
}

fn commit_ready_cue() -> ActionCue {
    ActionCue {
        kind: ActionCueKind::CommitReady,
        status: ActionCueStatus::Active,
        source: ActionCueSource::Transcript,
        confidence: ActionCueConfidence::Deterministic,
        evidence: ActionCue::expected_evidence(ActionCueKind::CommitReady)
            .iter()
            .map(|item| item.to_string())
            .collect(),
    }
}

fn test_thought_snapshot(thought: &str, thought_state: ThoughtState) -> ThoughtSnapshot {
    ThoughtSnapshot {
        thought: Some(thought.to_string()),
        thought_state,
        thought_source: ThoughtSource::Llm,
        rest_state: match thought_state {
            ThoughtState::Active => RestState::Active,
            ThoughtState::Holding => RestState::Drowsy,
            ThoughtState::Sleeping => RestState::Sleeping,
        },
        commit_candidate: thought_state == ThoughtState::Active,
        action_cues: Vec::new(),
        objective_changed_at: None,
        objective_fingerprint: None,
        token_count: 10,
        context_limit: 100,
        updated_at: Utc::now(),
        delivery: ThoughtDeliveryState::default(),
    }
}

fn write_test_repo_theme_colors(root: &std::path::Path, body: &str) {
    let theme_dir = root.join(".swimmers");
    std::fs::create_dir_all(&theme_dir).expect("create theme dir");
    std::fs::write(
        theme_dir.join("colors.json"),
        format!(
            r##"{{
  "palette": {{
"body": "{body}",
"outline": "#3D2F24",
"accent": "#1D1914",
"shirt": "#AA9370"
  }}
}}
"##
        ),
    )
    .expect("write colors.json");
}

async fn spawn_summary_handle(summary: SessionSummary) -> ActorHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle(
        summary.session_id.clone(),
        summary.tmux_name.clone(),
        cmd_tx,
    );
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = reply.send(summary.clone());
                }
                SessionCommand::GetSnapshot(reply) => {
                    let _ = reply.send(TerminalSnapshot {
                        session_id: summary.session_id.clone(),
                        latest_seq: 17,
                        truncated: false,
                        screen_text: "0123456789 replay tail".to_string(),
                    });
                }
                SessionCommand::Shutdown => break,
                _ => {}
            }
        }
    });
    handle
}

async fn spawn_dropped_summary_handle(
    session_id: &str,
    tmux_name: &str,
    state: SessionState,
) -> ActorHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle(session_id, tmux_name, cmd_tx);
    let summary = test_summary(session_id, state);
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(_reply) => {}
                SessionCommand::GetSnapshot(reply) => {
                    let _ = reply.send(TerminalSnapshot {
                        session_id: summary.session_id.clone(),
                        latest_seq: 0,
                        truncated: false,
                        screen_text: String::new(),
                    });
                }
                SessionCommand::Shutdown => break,
                _ => {}
            }
        }
    });
    handle
}

async fn spawn_hung_summary_handle(session_id: &str, tmux_name: &str) -> ActorHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle(session_id, tmux_name, cmd_tx);
    tokio::spawn(async move {
        let mut held_replies = Vec::new();
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    held_replies.push(reply);
                }
                SessionCommand::Shutdown => break,
                _ => {}
            }
        }
    });
    handle
}

async fn spawn_observed_hung_summary_handle(
    session_id: &str,
    tmux_name: &str,
    observed_tx: mpsc::UnboundedSender<String>,
) -> ActorHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
    let handle = ActorHandle::test_handle(session_id, tmux_name, cmd_tx);
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        let mut held_replies = Vec::new();
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                SessionCommand::GetSummary(reply) => {
                    let _ = observed_tx.send(session_id.clone());
                    held_replies.push(reply);
                }
                SessionCommand::Shutdown => break,
                _ => {}
            }
        }
    });
    handle
}

async fn spawn_closed_summary_handle(session_id: &str, tmux_name: &str) -> ActorHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    drop(cmd_rx);
    ActorHandle::test_handle(session_id, tmux_name, cmd_tx)
}

fn write_executable(path: &std::path::Path, contents: &str) {
    std::fs::write(path, contents).expect("write executable");
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

fn test_path_with_prepend(
    bin_dir: &std::path::Path,
    original_path: Option<&std::ffi::OsStr>,
) -> std::ffi::OsString {
    std::env::join_paths(test_path_entries(bin_dir, original_path)).expect("path")
}

fn test_path_entries(
    bin_dir: &std::path::Path,
    original_path: Option<&std::ffi::OsStr>,
) -> Vec<std::ffi::OsString> {
    let mut entries = test_path_entries_without_system_dirs(bin_dir, original_path);
    append_existing_system_path_entries(&mut entries);
    entries
}

fn test_path_entries_without_system_dirs(
    bin_dir: &std::path::Path,
    original_path: Option<&std::ffi::OsStr>,
) -> Vec<std::ffi::OsString> {
    std::iter::once(bin_dir.as_os_str().to_os_string())
        .chain(
            original_path
                .into_iter()
                .flat_map(std::env::split_paths)
                .map(|path| path.into_os_string()),
        )
        .collect()
}

fn append_existing_system_path_entries(entries: &mut Vec<std::ffi::OsString>) {
    for system_dir in ["/bin", "/usr/bin"].into_iter().map(std::path::Path::new) {
        append_existing_system_path_entry(entries, system_dir);
    }
}

fn append_existing_system_path_entry(
    entries: &mut Vec<std::ffi::OsString>,
    system_dir: &std::path::Path,
) {
    if let Some(entry) = system_path_entry_to_append(entries, system_dir) {
        entries.push(entry);
    }
}

fn system_path_entry_to_append(
    entries: &[std::ffi::OsString],
    system_dir: &std::path::Path,
) -> Option<std::ffi::OsString> {
    system_dir
        .is_dir()
        .then(|| system_dir.as_os_str().to_os_string())
        .filter(|_| !path_entries_contain(entries, system_dir))
}

fn path_entries_contain(entries: &[std::ffi::OsString], system_dir: &std::path::Path) -> bool {
    entries
        .iter()
        .any(|entry| std::path::Path::new(entry) == system_dir)
}

fn prepend_test_path(bin_dir: &std::path::Path, original_path: Option<&std::ffi::OsStr>) {
    std::env::set_var("PATH", test_path_with_prepend(bin_dir, original_path));
}

fn install_fake_tmux(script: &str) -> (tempfile::TempDir, Option<std::ffi::OsString>) {
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin");
    write_executable(&bin_dir.join("tmux"), script);
    let original_path = std::env::var_os("PATH");
    prepend_test_path(&bin_dir, original_path.as_deref());
    (dir, original_path)
}

fn restore_test_env_var(name: &str, original_value: Option<std::ffi::OsString>) {
    if let Some(value) = original_value {
        std::env::set_var(name, value);
    } else {
        std::env::remove_var(name);
    }
}

fn restore_test_path(original_path: Option<std::ffi::OsString>) {
    restore_test_env_var("PATH", original_path);
}

fn prompt_file_from_spawn_command(command: &str) -> PathBuf {
    let prefix = "prompt_file='";
    let suffix = "'; if prompt=\"$(cat \"$prompt_file\")\"; then rm -f \"$prompt_file\"; if command -v caam >/dev/null 2>&1; then caam run codex -- \"$prompt\" || { echo 'swimmers: caam codex launch failed; falling back to raw codex' >&2; if command -v codex-raw >/dev/null 2>&1; then codex-raw \"$prompt\"; else command codex \"$prompt\"; fi; }; else command codex \"$prompt\"; fi; else rm -f \"$prompt_file\"; echo 'swimmers: failed to read initial request' >&2; false; fi";
    assert!(command.starts_with(prefix), "unexpected command: {command}");
    assert!(command.ends_with(suffix), "unexpected command: {command}");
    PathBuf::from(&command[prefix.len()..command.len() - suffix.len()])
}

fn grok_prompt_file_from_spawn_command(command: &str) -> PathBuf {
    let prefix = "prompt_file='";
    let suffix = "'; if [ -r \"$prompt_file\" ]; then";
    assert!(command.starts_with(prefix), "unexpected command: {command}");
    let Some(end) = command.find(suffix) else {
        panic!("unexpected command: {command}");
    };
    PathBuf::from(&command[prefix.len()..end])
}

fn spawn_command_test_shell() -> &'static str {
    if cfg!(unix) {
        "/bin/sh"
    } else {
        "sh"
    }
}

fn prompt_dir_contains(marker: &str) -> bool {
    let dir = std::env::temp_dir().join("swimmers-initial-requests");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        std::fs::read_to_string(entry.path())
            .map(|contents| contents.contains(marker))
            .unwrap_or(false)
    })
}
