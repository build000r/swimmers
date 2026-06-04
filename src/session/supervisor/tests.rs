use super::*;
use crate::types::{ActionCueConfidence, ActionCueKind, ActionCueSource, ActionCueStatus};
use chrono::{DateTime, Utc};
use std::iter::FromIterator;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;
use tokio::sync::mpsc;

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

#[test]
fn summary_lifecycle_helpers_use_shared_fallback_causes() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));

    let placeholder = supervisor.build_placeholder_summary("sess_1", "work");
    assert_eq!(
        placeholder.state_evidence.cause,
        crate::types::SUMMARY_CAUSE_SUPERVISOR_PLACEHOLDER
    );
    assert_eq!(placeholder.transport_health, TransportHealth::Healthy);
    assert!(!placeholder.is_stale);

    let missing = SessionSupervisor::mark_missing_tmux_summary(placeholder);
    assert_eq!(
        missing.state_evidence.cause,
        SUMMARY_CAUSE_TMUX_RECONCILE_MISSING
    );
    assert_eq!(missing.state, SessionState::Exited);
    assert_eq!(missing.transport_health, TransportHealth::Disconnected);
    assert!(missing.is_stale);
}

#[test]
fn kill_tmux_session_result_accepts_success_status() {
    let result = classify_kill_tmux_session_result(true, b"unexpected stderr");

    assert!(result.is_ok());
}

#[test]
fn kill_tmux_session_result_accepts_missing_session_stderr() {
    let result = classify_kill_tmux_session_result(false, b"can't find session: demo");

    assert!(result.is_ok());
}

#[test]
fn kill_tmux_session_result_accepts_no_server_stderr() {
    let result = classify_kill_tmux_session_result(false, b"no server running on /tmp/tmux");

    assert!(result.is_ok());
}

#[test]
fn kill_tmux_session_result_reports_trimmed_unexpected_stderr() {
    let err = classify_kill_tmux_session_result(false, b"  permission denied\n")
        .expect_err("unexpected kill failure should be reported");

    assert_eq!(
        err.to_string(),
        "tmux kill-session failed: permission denied"
    );
}

#[test]
fn tmux_list_output_classifies_successful_stdout_as_reliable_names() {
    let outcome = classify_tmux_list_sessions_output(true, b"alpha\n\nbeta\n", b"ignored stderr");

    assert_eq!(
        outcome,
        TmuxListSessionsOutcome::Listed(vec!["alpha".to_string(), "beta".to_string()])
    );
}

#[test]
fn tmux_list_output_classifies_no_session_stderr_as_reliable_empty() {
    let outcome = classify_tmux_list_sessions_output(
        false,
        b"",
        b"no server running on /tmp/tmux-1000/default\n",
    );

    assert_eq!(outcome, TmuxListSessionsOutcome::NoSessions);
}

#[test]
fn tmux_list_output_classifies_unexpected_stderr_as_unreliable_failure() {
    let outcome = classify_tmux_list_sessions_output(false, b"", b"permission denied\n");

    assert_eq!(
        outcome,
        TmuxListSessionsOutcome::TmuxError("permission denied\n".to_string())
    );
}

#[test]
fn tmux_list_command_error_classifies_as_unreliable_failure() {
    let outcome =
        classify_tmux_list_sessions_command_error("failed to run tmux list-sessions: denied");

    assert_eq!(
        outcome,
        TmuxListSessionsOutcome::CommandError(
            "failed to run tmux list-sessions: denied".to_string()
        )
    );
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

#[test]
fn test_path_with_prepend_appends_existing_path_entries() {
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    let existing_one = dir.path().join("existing-one");
    let existing_two = dir.path().join("existing-two");
    let original_path = std::env::join_paths([existing_one.as_path(), existing_two.as_path()])
        .expect("original path");

    let test_path = test_path_with_prepend(&bin_dir, Some(&original_path));
    let entries = std::env::split_paths(&test_path).collect::<Vec<_>>();

    assert_eq!(entries[0], bin_dir);
    assert_eq!(entries[1], existing_one);
    assert_eq!(entries[2], existing_two);
}

#[test]
fn test_path_with_prepend_dedupes_existing_system_dirs() {
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    let bin = std::path::Path::new("/bin");
    let usr_bin = std::path::Path::new("/usr/bin");
    let original_path = std::env::join_paths([bin, usr_bin]).expect("original path");

    let test_path = test_path_with_prepend(&bin_dir, Some(&original_path));
    let entries = std::env::split_paths(&test_path).collect::<Vec<_>>();

    assert_eq!(entries[0], bin_dir);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.as_path() == bin)
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.as_path() == usr_bin)
            .count(),
        1
    );
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

#[test]
fn next_session_counter_parses_expected_format() {
    assert_eq!(next_session_counter("sess_0"), Some(1));
    assert_eq!(next_session_counter("sess_41"), Some(42));
}

#[test]
fn next_session_counter_rejects_unexpected_format() {
    assert_eq!(next_session_counter("session_1"), None);
    assert_eq!(next_session_counter("sess_not_a_number"), None);
    assert_eq!(next_session_counter(""), None);
}

#[test]
fn tmux_query_command_scrubs_tmux_env_vars() {
    let command = tmux_query_command(&["list-sessions", "-F", "#{session_name}"]);

    let tmux_value = command
        .as_std()
        .get_envs()
        .find_map(|(key, value)| (key == std::ffi::OsStr::new("TMUX")).then_some(value));
    assert_eq!(tmux_value, Some(None));

    let tmux_pane_value = command
        .as_std()
        .get_envs()
        .find_map(|(key, value)| (key == std::ffi::OsStr::new("TMUX_PANE")).then_some(value));
    assert_eq!(tmux_pane_value, Some(None));
}

#[tokio::test]
async fn bounded_tmux_command_times_out_non_returning_fake_tmux() {
    let dir = tempdir().expect("tempdir");
    let fake_tmux = dir.path().join("tmux");
    write_executable(
        &fake_tmux,
        "#!/bin/sh\nif [ -x /bin/sleep ]; then exec /bin/sleep 10; fi\nexec sleep 10\n",
    );

    let started = Instant::now();
    let err = run_bounded_tmux_command(
        fake_tmux.as_os_str(),
        &["list-sessions", "-F", "#{session_name}"],
        Duration::from_millis(25),
        "test-hanging-list-sessions",
    )
    .await
    .expect_err("hanging tmux should time out");

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "bounded tmux helper should not wait for the fake tmux sleep"
    );
    assert!(
        err.to_string().contains("timed out after 25ms"),
        "timeout error should mention the bounded wait: {err:#}"
    );
}

#[test]
fn spawn_tool_roundtrip_sets_correct_display_name() {
    use crate::types::{context_limit_for_tool, detect_tool_name, SpawnTool};

    for (tool, expected_name, expected_limit) in [
        (SpawnTool::Claude, "Claude Code", 200_000),
        (SpawnTool::Codex, "Codex", 192_000),
        (SpawnTool::Grok, "Grok", 128_000),
    ] {
        let display = detect_tool_name(tool.command()).unwrap_or(tool.command());
        assert_eq!(display, expected_name);
        assert_eq!(context_limit_for_tool(Some(display)), expected_limit);
    }
}

#[test]
fn normalize_initial_request_trims_blank_values() {
    assert_eq!(normalize_initial_request(None), None);
    assert_eq!(normalize_initial_request(Some("   ".to_string())), None);
    assert_eq!(
        normalize_initial_request(Some("  investigate tmux  ".to_string())),
        Some("investigate tmux".to_string())
    );
}

#[test]
fn build_initial_request_input_appends_carriage_return() {
    assert_eq!(
        build_initial_request_input("hello codex"),
        b"hello codex\r".to_vec()
    );
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

#[test]
fn build_spawn_tool_command_uses_prompt_file_for_grok_initial_request() {
    let prompt = "investigate Grok launch\nwithout argv prompt leaks";
    let command = build_spawn_tool_command_with_launcher(
        crate::types::SpawnTool::Grok,
        Some("/tmp/repos/swim mer's"),
        Some(prompt),
        SpawnToolLauncher::with_program_override(
            crate::types::SpawnTool::Grok,
            Some(std::ffi::OsString::from("/tmp/bin/grok wrapper")),
        ),
    );
    let prompt_path = grok_prompt_file_from_spawn_command(&command);

    assert!(!command.contains("investigate Grok launch"));
    assert!(!command.contains('\n'));
    assert!(command.contains("'/tmp/bin/grok wrapper' --prompt-file \"$prompt_file\""));
    assert!(command.contains("--cwd '/tmp/repos/swim mer'\\''s'"));
    assert!(command.contains("--always-approve --no-alt-screen"));
    assert!(!command.contains("--session-id"));
    assert!(!command.contains("--output-format"));
    assert!(!command.contains("--max-turns"));
    assert_eq!(
        std::fs::read_to_string(&prompt_path).expect("prompt file"),
        prompt
    );
    let _ = std::fs::remove_file(prompt_path);
}

#[tokio::test]
async fn create_session_cleans_grok_prompt_file_when_spawn_rejects_cwd() {
    let dir = tempdir().expect("tempdir");
    let missing_cwd = dir.path().join("missing");
    let marker = format!("grok prompt cleanup marker {}", Uuid::new_v4());
    assert!(
        !prompt_dir_contains(&marker),
        "test marker should not exist before session creation"
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let err = supervisor
        .create_session(
            None,
            Some(missing_cwd.to_string_lossy().into_owned()),
            Some(crate::types::SpawnTool::Grok),
            Some(marker.clone()),
        )
        .await
        .expect_err("invalid cwd should reject session creation");

    assert!(err.to_string().contains("session cwd does not exist"));
    assert!(
        !prompt_dir_contains(&marker),
        "failed session spawn must remove Grok prompt file"
    );
}

#[test]
fn build_spawn_tool_command_uses_grok_override_for_no_prompt_launch() {
    let command = build_spawn_tool_command_with_launcher(
        crate::types::SpawnTool::Grok,
        Some("/tmp/repos/swimmers"),
        None,
        SpawnToolLauncher::with_program_override(
            crate::types::SpawnTool::Grok,
            Some(std::ffi::OsString::from("/tmp/bin/grok wrapper")),
        ),
    );

    assert_eq!(command, "'/tmp/bin/grok wrapper'");
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

#[test]
fn grok_prompt_command_removes_prompt_file_after_success() {
    let temp = tempdir().expect("tempdir");
    let grok = temp.path().join("grok");
    let captured_prompt = temp.path().join("captured-prompt.txt");
    let restricted_path = temp.path().join("restricted-path");
    std::fs::create_dir_all(&restricted_path).expect("restricted path");
    let capture_script = format!(
        "#!/bin/sh\nprompt_file=\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--prompt-file\" ]; then shift; prompt_file=$1; fi\n  shift || true\ndone\nif [ -n \"$prompt_file\" ]; then\n  IFS= read -r prompt < \"$prompt_file\" || true\n  printf '%s' \"$prompt\" > {}\nfi\nexit 0\n",
        shell_single_quote(&captured_prompt.to_string_lossy())
    );
    write_executable(&grok, &capture_script);

    let command = build_spawn_tool_command_with_launcher(
        crate::types::SpawnTool::Grok,
        Some(temp.path().to_str().expect("utf8 tempdir")),
        Some("private Grok prompt"),
        SpawnToolLauncher::with_program_override(
            crate::types::SpawnTool::Grok,
            Some(grok.into_os_string()),
        ),
    );
    let prompt_path = grok_prompt_file_from_spawn_command(&command);
    let status = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .env("PATH", &restricted_path)
        .status()
        .expect("run Grok spawn command");

    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(captured_prompt).expect("captured prompt"),
        "private Grok prompt"
    );
    assert!(!prompt_path.exists(), "prompt file should be removed");
}

#[test]
fn grok_prompt_command_removes_prompt_file_after_failure() {
    let temp = tempdir().expect("tempdir");
    let grok = temp.path().join("grok");
    let restricted_path = temp.path().join("restricted-path");
    std::fs::create_dir_all(&restricted_path).expect("restricted path");
    write_executable(&grok, "#!/bin/sh\nexit 42\n");

    let command = build_spawn_tool_command_with_launcher(
        crate::types::SpawnTool::Grok,
        None,
        Some("private Grok prompt"),
        SpawnToolLauncher::with_program_override(
            crate::types::SpawnTool::Grok,
            Some(grok.into_os_string()),
        ),
    );
    let prompt_path = grok_prompt_file_from_spawn_command(&command);
    let status = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .env("PATH", &restricted_path)
        .status()
        .expect("run Grok spawn command");

    assert!(!status.success());
    assert!(!prompt_path.exists(), "prompt file should be removed");
}

#[test]
fn build_spawn_tool_command_uses_prompt_file_for_codex_initial_request() {
    let prompt = "investigate tmux startup\nthen inspect imports";
    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
    let prompt_path = prompt_file_from_spawn_command(&command);

    assert!(!command.contains("investigate tmux startup"));
    assert!(!command.contains('\n'));
    assert!(command.contains("rm -f \"$prompt_file\""));
    assert_eq!(
        std::fs::read_to_string(&prompt_path).expect("prompt file"),
        prompt
    );
    let _ = std::fs::remove_file(prompt_path);
}

#[test]
fn codex_prompt_command_reports_missing_prelaunch_prompt_file() {
    let command =
        build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some("lost prompt"));
    let prompt_path = prompt_file_from_spawn_command(&command);
    std::fs::remove_file(&prompt_path).expect("remove prompt file before command runs");
    let output = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .output()
        .expect("run spawn command");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("swimmers: failed to read initial request"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!prompt_path.exists());
}

#[test]
fn delayed_prelaunch_cleanup_allows_codex_prompt_command_to_read_file() {
    let temp = tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let captured_prompt = temp.path().join("captured-prompt.txt");
    let capture_script = format!(
        "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
        shell_single_quote(&captured_prompt.to_string_lossy())
    );
    write_executable(&bin_dir.join("codex"), &capture_script);
    let test_path = test_path_with_prepend(&bin_dir, None);

    let prompt = "prompt survives delayed cleanup";
    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
    let prompt_path = prompt_file_from_spawn_command(&command);
    schedule_prelaunch_file_cleanup_after(vec![prompt_path.clone()], Duration::from_millis(100));

    assert!(
        prompt_path.exists(),
        "cleanup must not remove the handoff immediately"
    );
    let output = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .env("PATH", test_path)
        .output()
        .expect("run spawn command");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(captured_prompt).expect("captured prompt"),
        prompt
    );
    assert!(
        !prompt_path.exists(),
        "prompt file should be removed by the command"
    );
}

#[test]
fn delayed_prelaunch_cleanup_removes_unread_prompt_file() {
    let temp = tempdir().expect("tempdir");
    let prompt_path = temp.path().join("orphaned-prompt.txt");
    std::fs::write(&prompt_path, "orphaned prompt").expect("prompt file");

    schedule_prelaunch_file_cleanup_after(vec![prompt_path.clone()], Duration::from_millis(10));

    for _ in 0..50 {
        if !prompt_path.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !prompt_path.exists(),
        "orphaned prompt file should be removed"
    );
}

#[test]
fn build_spawn_tool_command_prompt_file_preserves_quote_sensitive_prompt() {
    let prompt = "fix Bob's tmux startup with \"fresh eyes\"";
    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
    let prompt_path = prompt_file_from_spawn_command(&command);

    assert!(!command.contains("Bob"));
    assert!(!command.contains("fresh eyes"));
    assert_eq!(
        std::fs::read_to_string(&prompt_path).expect("prompt file"),
        prompt
    );
    let _ = std::fs::remove_file(prompt_path);
}

#[test]
fn wrap_spawn_tool_command_for_tmux_keeps_shell_after_tool_exits() {
    assert_eq!(
        wrap_spawn_tool_command_for_tmux("codex 'investigate tmux startup'"),
        "{ codex 'investigate tmux startup'; }; exec \"${SHELL:-/bin/sh}\""
    );
}

#[cfg(unix)]
#[test]
fn codex_prompt_file_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let prompt = "private prompt";
    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
    let prompt_path = prompt_file_from_spawn_command(&command);
    let dir_mode = std::fs::metadata(prompt_path.parent().expect("prompt dir"))
        .expect("prompt dir metadata")
        .permissions()
        .mode()
        & 0o777;
    let file_mode = std::fs::metadata(&prompt_path)
        .expect("prompt file metadata")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(dir_mode, 0o700);
    assert_eq!(file_mode, 0o600);
    let _ = std::fs::remove_file(prompt_path);
}

#[test]
fn codex_prompt_command_reads_and_removes_prompt_file() {
    let temp = tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let captured_prompt = temp.path().join("captured-prompt.txt");
    let capture_script = format!(
        "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
        shell_single_quote(&captured_prompt.to_string_lossy())
    );
    write_executable(&bin_dir.join("codex"), &capture_script);

    let test_path = test_path_with_prepend(&bin_dir, None);

    let prompt = "fix shell quoting\nwithout leaking prompt text";
    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
    let prompt_path = prompt_file_from_spawn_command(&command);
    let status = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .env("PATH", test_path)
        .status()
        .expect("run spawn command");

    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(captured_prompt).expect("captured prompt"),
        prompt
    );
    assert!(!prompt_path.exists(), "prompt file should be removed");
}

#[test]
fn codex_prompt_command_prefers_caam_when_available() {
    let temp = tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let captured_args = temp.path().join("caam-args.txt");
    let caam_script = format!(
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\" > {}\n",
        shell_single_quote(&captured_args.to_string_lossy())
    );
    write_executable(&bin_dir.join("caam"), &caam_script);
    write_executable(
        &bin_dir.join("codex"),
        "#!/usr/bin/env bash\necho 'codex fallback should not run' >&2\nexit 99\n",
    );

    let test_path = test_path_with_prepend(&bin_dir, None);

    let prompt = "route through caam";
    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some(prompt));
    let prompt_path = prompt_file_from_spawn_command(&command);
    let status = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .env("PATH", test_path)
        .status()
        .expect("run spawn command");

    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(captured_args).expect("captured caam args"),
        "run\ncodex\n--\nroute through caam\n"
    );
    assert!(!prompt_path.exists(), "prompt file should be removed");
}

#[test]
fn codex_prompt_command_falls_back_after_caam_failure() {
    let temp = tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    write_executable(&bin_dir.join("caam"), "#!/usr/bin/env bash\nexit 42\n");
    let captured_prompt = temp.path().join("fallback-prompt.txt");
    let fallback_script = format!(
        "#!/usr/bin/env bash\nprintf '%s' \"$1\" > {}\n",
        shell_single_quote(&captured_prompt.to_string_lossy())
    );
    write_executable(&bin_dir.join("codex-raw"), &fallback_script);

    let test_path = test_path_with_prepend(&bin_dir, None);

    let command = build_spawn_tool_command(crate::types::SpawnTool::Codex, None, Some("blocked"));
    let prompt_path = prompt_file_from_spawn_command(&command);
    let output = std::process::Command::new(spawn_command_test_shell())
        .arg("-c")
        .arg(&command)
        .env("PATH", test_path)
        .output()
        .expect("run spawn command");

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("swimmers: caam codex launch failed; falling back to raw codex"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(captured_prompt).expect("captured fallback prompt"),
        "blocked"
    );
    assert!(!prompt_path.exists(), "prompt file should be removed");
}

#[test]
fn build_spawn_tool_command_inlines_claude_initial_request() {
    assert_eq!(
        build_spawn_tool_command(
            crate::types::SpawnTool::Claude,
            None,
            Some("investigate tmux startup")
        ),
        "claude 'investigate tmux startup'"
    );
    assert!(spawn_tool_consumes_initial_request(
        crate::types::SpawnTool::Claude
    ));
}

#[tokio::test]
async fn init_persistence_bumps_id_counter_from_thought_snapshot_ids() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    store
        .save_thought(
            "sess_42",
            Some("stale thought"),
            7,
            128_000,
            ThoughtState::Holding,
            ThoughtSource::CarryForward,
            RestState::Drowsy,
            false,
            Vec::new(),
            Utc::now(),
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let allocated = supervisor.allocate_unique_session_id().await;
    assert_eq!(allocated, "sess_43");
}

#[tokio::test]
async fn init_persistence_keeps_persisted_session_id_progression() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    store
        .save_sessions(&[PersistedSession {
            session_id: "sess_7".to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Idle,
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            objective_fingerprint: None,
            batch: None,
            cwd: "/tmp".to_string(),
            last_activity_at: Utc::now(),
        }])
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let allocated = supervisor.allocate_unique_session_id().await;
    assert_eq!(allocated, "sess_8");
}

#[tokio::test]
async fn init_persistence_preserves_batch_membership_on_stale_sessions() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    store
        .save_sessions(&[PersistedSession {
            session_id: "sess_7".to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Idle,
            tool: Some("Codex".to_string()),
            token_count: 0,
            context_limit: 192_000,
            thought: None,
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: None,
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: None,
            objective_fingerprint: None,
            batch: Some(SessionBatchMembership {
                id: "batch-auth".to_string(),
                label: "auth-rebuild".to_string(),
                index: 0,
                total: 2,
                created_at: Utc::now(),
                prompt_excerpt: Some("auth-rebuild".to_string()),
            }),
            cwd: "/tmp".to_string(),
            last_activity_at: Utc::now(),
        }])
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let stale = supervisor.stale_sessions.read().await;
    let batch = stale[0].batch.as_ref().expect("batch membership");
    assert_eq!(batch.id, "batch-auth");
    assert_eq!(batch.label, "auth-rebuild");
    assert_eq!(batch.index, 0);
    assert_eq!(batch.total, 2);
    assert_eq!(
        stale[0].state_evidence.cause,
        SUMMARY_CAUSE_PERSISTENCE_STALE
    );
    assert!(stale[0].state_evidence.observed_at.is_none());
    assert_eq!(
        stale[0].state_evidence.confidence,
        crate::types::StateConfidence::Low
    );
}

#[tokio::test]
async fn init_persistence_hydrates_stale_session_from_thought_snapshot() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    let persisted_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:00Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let thought_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let objective_changed_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:02Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let action_cues = vec![commit_ready_cue()];

    store
        .save_sessions(&[PersistedSession {
            session_id: "sess_7".to_string(),
            tmux_name: "7".to_string(),
            state: SessionState::Idle,
            tool: Some("Codex".to_string()),
            token_count: 12,
            context_limit: 192_000,
            thought: Some("persisted thought".to_string()),
            thought_state: ThoughtState::Holding,
            thought_source: ThoughtSource::CarryForward,
            thought_updated_at: Some(persisted_at),
            rest_state: RestState::Drowsy,
            commit_candidate: false,
            action_cues: Vec::new(),
            objective_changed_at: None,
            last_skill: Some("rust".to_string()),
            objective_fingerprint: Some("old-objective".to_string()),
            batch: None,
            cwd: "/tmp".to_string(),
            last_activity_at: persisted_at,
        }])
        .await;
    store
        .save_thought(
            "sess_7",
            Some("snapshot thought"),
            88,
            256_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            true,
            action_cues.clone(),
            thought_at,
            ThoughtDeliveryState::default(),
            Some(objective_changed_at),
            Some("new-objective".to_string()),
        )
        .await;

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store).await;

    let stale = supervisor.stale_sessions.read().await;
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].session_id, "sess_7");
    assert_eq!(stale[0].state, SessionState::Exited);
    assert_eq!(stale[0].thought.as_deref(), Some("snapshot thought"));
    assert_eq!(stale[0].thought_state, ThoughtState::Active);
    assert_eq!(stale[0].thought_source, ThoughtSource::Llm);
    assert_eq!(stale[0].thought_updated_at, Some(thought_at));
    assert_eq!(stale[0].rest_state, RestState::Active);
    assert_eq!(stale[0].token_count, 88);
    assert_eq!(stale[0].context_limit, 256_000);
    assert!(stale[0].commit_candidate);
    assert_eq!(stale[0].action_cues, action_cues);
    assert_eq!(stale[0].objective_changed_at, Some(objective_changed_at));
    assert_eq!(stale[0].last_skill.as_deref(), Some("rust"));
    assert_eq!(stale[0].last_activity_at, persisted_at);
}

#[tokio::test]
async fn persist_thought_preserves_supplied_updated_at() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);

    supervisor
        .persist_thought(
            "sess_1",
            Some("reading logs"),
            12,
            192_000,
            ThoughtState::Holding,
            ThoughtSource::Llm,
            RestState::Drowsy,
            false,
            Vec::new(),
            updated_at,
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        )
        .await;

    let thoughts = supervisor.thought_snapshots.read().await;
    let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
    assert_eq!(snapshot.updated_at, updated_at);
    assert_eq!(snapshot.thought.as_deref(), Some("reading logs"));
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_provider_coalesces_latest_thought_when_persist_queue_is_full() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let provider = SupervisorProvider::new_with_persist_queue_capacity(supervisor.clone(), 1);
    let first_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:01Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);
    let second_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:02Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);
    let third_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:03Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);

    assert!(provider.persist_thought(
        "sess_1",
        Some("first queued"),
        1,
        192_000,
        ThoughtState::Active,
        ThoughtSource::Llm,
        RestState::Active,
        false,
        Vec::new(),
        first_at,
        ThoughtDeliveryState {
            stream_instance_id: Some("stream-a".to_string()),
            emission_seq: 1,
        },
        None,
        Some("obj-1".to_string()),
    ));
    assert!(
        !provider.persist_thought(
            "sess_1",
            Some("second overflow"),
            2,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            second_at,
            ThoughtDeliveryState {
                stream_instance_id: Some("stream-a".to_string()),
                emission_seq: 2,
            },
            None,
            Some("obj-2".to_string()),
        ),
        "queue-full writes should be accepted for coalesced persistence but reported as degraded"
    );
    assert!(
        !provider.persist_thought(
            "sess_1",
            Some("third latest"),
            3,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            third_at,
            ThoughtDeliveryState {
                stream_instance_id: Some("stream-a".to_string()),
                emission_seq: 3,
            },
            None,
            Some("obj-3".to_string()),
        ),
        "overwriting an overflow slot remains a degraded durability path"
    );

    let pressure = supervisor.thought_persistence_backpressure_snapshot();
    assert_eq!(
        pressure.queue_capacity, 1,
        "snapshot must report the configured queue capacity, not the default"
    );
    assert_eq!(pressure.queue_depth, 1);
    assert_eq!(pressure.pending_count, 2);
    assert_eq!(pressure.overflow_slots, 1);
    assert_eq!(pressure.queue_full_count, 2);
    assert_eq!(pressure.coalesced_count, 1);
    assert_eq!(pressure.dropped_count, 0);

    assert!(
        supervisor
            .wait_for_pending_thought_persists(Duration::from_secs(1))
            .await,
        "queued and coalesced thought writes should drain"
    );

    let thoughts = supervisor.thought_snapshots.read().await;
    let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
    assert_eq!(snapshot.thought.as_deref(), Some("third latest"));
    assert_eq!(snapshot.token_count, 3);
    assert_eq!(snapshot.updated_at, third_at);
    assert_eq!(snapshot.delivery.emission_seq, 3);
    assert_eq!(
        snapshot.delivery.stream_instance_id.as_deref(),
        Some("stream-a")
    );
    drop(thoughts);

    let drained = supervisor.thought_persistence_backpressure_snapshot();
    assert_eq!(drained.pending_count, 0);
    assert_eq!(drained.overflow_slots, 0);
}

#[tokio::test]
async fn persist_thought_retains_objective_shift_timestamp_until_next_shift() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let shifted_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);
    let later_update = DateTime::parse_from_rfc3339("2026-03-08T14:00:09Z")
        .expect("timestamp should parse")
        .with_timezone(&Utc);

    supervisor
        .persist_thought(
            "sess_1",
            Some("reframed objective"),
            12,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            shifted_at,
            ThoughtDeliveryState::default(),
            Some(shifted_at),
            Some("obj-1".to_string()),
        )
        .await;
    supervisor
        .persist_thought(
            "sess_1",
            Some("continuing work"),
            14,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            false,
            Vec::new(),
            later_update,
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        )
        .await;

    let thoughts = supervisor.thought_snapshots.read().await;
    let snapshot = thoughts.get("sess_1").expect("snapshot should exist");
    assert_eq!(snapshot.updated_at, later_update);
    assert_eq!(snapshot.objective_changed_at, Some(shifted_at));
    assert_eq!(snapshot.thought.as_deref(), Some("continuing work"));
}

#[tokio::test]
async fn persist_registry_uses_actor_state_without_querying_tmux() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin");
    let command_file = dir.path().join("tmux-command.txt");
    write_executable(
        &bin_dir.join("tmux"),
        &format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$1\" > \"{}\"\nexit 1\n",
            command_file.display()
        ),
    );
    let original_path = std::env::var_os("PATH");
    prepend_test_path(&bin_dir, original_path.as_deref());

    let store = FileStore::new(dir.path()).await.expect("file store");
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store.clone()).await;
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
        )
        .await;

    supervisor.persist_registry().await;
    restore_test_path(original_path);

    let persisted = store.load_sessions().await;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].session_id, "sess-live");
    assert!(
        !command_file.exists(),
        "persist_registry should not shell out to tmux"
    );
}

#[tokio::test]
async fn persist_registry_merges_direct_thought_snapshot_into_registry() {
    let dir = tempdir().expect("tempdir");
    let store = FileStore::new(dir.path()).await.expect("file store");
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor.init_persistence(store.clone()).await;
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
        )
        .await;

    let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    supervisor
        .persist_thought(
            "sess-live",
            Some("reading logs"),
            12,
            192_000,
            ThoughtState::Active,
            ThoughtSource::Llm,
            RestState::Active,
            true,
            Vec::new(),
            updated_at,
            ThoughtDeliveryState::default(),
            None,
            Some("obj-1".to_string()),
        )
        .await;

    supervisor.persist_registry().await;

    let persisted = store.load_sessions().await;
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].thought.as_deref(), Some("reading logs"));
    assert_eq!(persisted[0].thought_updated_at, Some(updated_at));
    assert_eq!(persisted[0].rest_state, RestState::Active);
    assert!(persisted[0].commit_candidate);
    assert_eq!(persisted[0].objective_fingerprint.as_deref(), Some("obj-1"));
}

#[test]
fn thought_snapshot_for_summary_matches_active_tmux_pane() {
    let summary = SessionSummary {
        session_id: "sess_1".to_string(),
        tmux_name: "work".to_string(),
        state: SessionState::Idle,
        current_command: None,
        state_evidence: Default::default(),
        cwd: "/tmp".to_string(),
        tool: None,
        token_count: 0,
        context_limit: 0,
        thought: None,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        thought_updated_at: None,
        rest_state: RestState::Drowsy,
        commit_candidate: false,
        action_cues: Vec::new(),
        objective_changed_at: None,
        last_skill: None,
        is_stale: false,
        attached_clients: 0,
        stale_attached_clients: 0,
        transport_health: crate::types::TransportHealth::Healthy,
        last_activity_at: Utc::now(),
        repo_theme_id: None,
        batch: None,
    };

    let older = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let newer = DateTime::parse_from_rfc3339("2026-03-08T14:00:06Z")
        .expect("timestamp")
        .with_timezone(&Utc);

    let snapshots = HashMap::from([
        (
            "tmux:work:1.0:%1".to_string(),
            ThoughtSnapshot {
                thought: Some("pane one".to_string()),
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: older,
                delivery: ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 1,
                },
            },
        ),
        (
            "tmux:work:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: newer,
                delivery: ThoughtDeliveryState {
                    stream_instance_id: Some("stream-a".to_string()),
                    emission_seq: 2,
                },
            },
        ),
    ]);

    let matched = thought_snapshot_for_summary(&summary, Some("tmux:work:1.1:%2"), &snapshots)
        .expect("tmux pane snapshot");
    assert_eq!(matched.thought.as_deref(), Some("pane two"));
    assert_eq!(matched.delivery.emission_seq, 2);
}

#[test]
fn thought_snapshot_for_summary_does_not_fall_back_to_latest_tmux_pane_without_active_binding() {
    let summary = SessionSummary {
        session_id: "sess_1".to_string(),
        tmux_name: "work".to_string(),
        state: SessionState::Idle,
        current_command: None,
        state_evidence: Default::default(),
        cwd: "/tmp".to_string(),
        tool: None,
        token_count: 0,
        context_limit: 0,
        thought: None,
        thought_state: ThoughtState::Holding,
        thought_source: ThoughtSource::CarryForward,
        thought_updated_at: None,
        rest_state: RestState::Drowsy,
        commit_candidate: false,
        action_cues: Vec::new(),
        objective_changed_at: None,
        last_skill: None,
        is_stale: false,
        attached_clients: 0,
        stale_attached_clients: 0,
        transport_health: crate::types::TransportHealth::Healthy,
        last_activity_at: Utc::now(),
        repo_theme_id: None,
        batch: None,
    };

    let snapshots = HashMap::from([
        (
            "tmux:work:1.0:%1".to_string(),
            ThoughtSnapshot {
                thought: Some("pane one".to_string()),
                thought_state: ThoughtState::Holding,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Drowsy,
                commit_candidate: false,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        ),
        (
            "tmux:work:1.1:%2".to_string(),
            ThoughtSnapshot {
                thought: Some("pane two".to_string()),
                thought_state: ThoughtState::Active,
                thought_source: ThoughtSource::Llm,
                rest_state: RestState::Active,
                commit_candidate: true,
                action_cues: Vec::new(),
                objective_changed_at: None,
                objective_fingerprint: None,
                token_count: 10,
                context_limit: 100,
                updated_at: Utc::now(),
                delivery: ThoughtDeliveryState::default(),
            },
        ),
    ]);

    assert!(thought_snapshot_for_summary(&summary, None, &snapshots).is_none());
}

#[test]
fn thought_snapshot_for_summary_prefers_direct_snapshot_over_active_tmux_pane() {
    let summary = test_summary("sess_1", SessionState::Idle);
    let snapshots = HashMap::from([
        (
            "sess_1".to_string(),
            test_thought_snapshot("direct session", ThoughtState::Holding),
        ),
        (
            "tmux:tmux-sess_1:1.1:%2".to_string(),
            test_thought_snapshot("active pane", ThoughtState::Active),
        ),
    ]);

    let matched =
        thought_snapshot_for_summary(&summary, Some("tmux:tmux-sess_1:1.1:%2"), &snapshots)
            .expect("direct session snapshot");

    assert_eq!(matched.thought.as_deref(), Some("direct session"));
    assert_eq!(matched.thought_state, ThoughtState::Holding);
}

#[test]
fn active_pane_lookup_not_required_without_thought_snapshots() {
    let summaries = [
        test_summary("sess-live", SessionState::Idle),
        test_summary("sess-busy", SessionState::Busy),
    ];
    let snapshots = HashMap::new();

    let tmux_names = tmux_names_requiring_active_pane_lookup(summaries.iter(), &snapshots);

    assert!(tmux_names.is_empty());
}

#[tokio::test]
async fn list_sessions_merges_thought_snapshots_and_skips_exited_summaries() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-live", SessionState::Idle)).await,
        )
        .await;
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-exited", SessionState::Exited)).await,
        )
        .await;

    supervisor.thought_snapshots.write().await.insert(
        "sess-live".to_string(),
        ThoughtSnapshot {
            thought: Some("checking logs".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: vec![commit_ready_cue()],
            objective_changed_at: Some(Utc::now()),
            objective_fingerprint: None,
            token_count: 44,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-live");
    assert_eq!(sessions[0].thought.as_deref(), Some("checking logs"));
    assert_eq!(sessions[0].thought_state, ThoughtState::Active);
    assert_eq!(sessions[0].token_count, 44);
    assert_eq!(sessions[0].action_cues, vec![commit_ready_cue()]);
    assert!(sessions[0].objective_changed_at.is_some());
}

#[tokio::test]
async fn list_sessions_resolves_repo_theme_after_thought_merge_when_theme_id_missing() {
    let repo = tempdir().expect("tempdir");
    write_test_repo_theme_colors(repo.path(), "#B89875");
    let expected_theme_id = repo.path().to_string_lossy().into_owned();
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-themed", SessionState::Idle);
    summary.cwd = expected_theme_id.clone();
    summary.repo_theme_id = None;
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "sess-themed".to_string(),
        test_thought_snapshot("checking themed repo", ThoughtState::Active),
    );

    let sessions = supervisor.list_sessions().await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].thought.as_deref(), Some("checking themed repo"));
    assert_eq!(
        sessions[0].repo_theme_id.as_deref(),
        Some(expected_theme_id.as_str())
    );
}

#[tokio::test]
async fn list_sessions_clears_repo_theme_id_after_thought_merge_when_theme_missing() {
    let repo = tempdir().expect("tempdir");
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-unthemed", SessionState::Idle);
    summary.cwd = repo.path().to_string_lossy().into_owned();
    summary.repo_theme_id = Some("/stale/theme".to_string());
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "sess-unthemed".to_string(),
        test_thought_snapshot("checking missing theme", ThoughtState::Active),
    );

    let sessions = supervisor.list_sessions().await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].thought.as_deref(),
        Some("checking missing theme")
    );
    assert_eq!(sessions[0].repo_theme_id, None);
}

#[tokio::test]
async fn startup_idle_session_only_sleeps_after_waiting_thought_snapshot() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let aged = DateTime::parse_from_rfc3339("2026-03-08T13:55:00Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    let mut summary = test_summary("sess-startup", SessionState::Idle);
    summary.rest_state = RestState::Drowsy;
    summary.last_activity_at = aged;
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;

    let sessions = supervisor.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-startup");
    assert!(sessions[0].thought.is_none());
    assert_eq!(sessions[0].thought_state, ThoughtState::Holding);
    assert_eq!(sessions[0].rest_state, RestState::Drowsy);
    assert_eq!(sessions[0].last_activity_at, aged);

    let updated_at = DateTime::parse_from_rfc3339("2026-03-08T14:00:05Z")
        .expect("timestamp")
        .with_timezone(&Utc);
    supervisor
        .persist_thought(
            "sess-startup",
            Some("Need your approval to continue."),
            12,
            192_000,
            ThoughtState::Sleeping,
            ThoughtSource::CarryForward,
            RestState::Sleeping,
            false,
            Vec::new(),
            updated_at,
            ThoughtDeliveryState::default(),
            None,
            None,
        )
        .await;

    let sessions = supervisor.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].thought.as_deref(),
        Some("Need your approval to continue.")
    );
    assert_eq!(sessions[0].thought_state, ThoughtState::Sleeping);
    assert_eq!(sessions[0].thought_source, ThoughtSource::CarryForward);
    assert_eq!(sessions[0].rest_state, RestState::Sleeping);
    assert_eq!(sessions[0].thought_updated_at, Some(updated_at));
    assert_eq!(sessions[0].last_activity_at, aged);
}

#[tokio::test]
async fn list_sessions_merges_thought_snapshot_from_active_tmux_pane_batch_lookup() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
sep=$(printf '\037')
name=$(printf 'work\tspace')
printf '%s%s0%s1%s1.0:%%1\n' "$name" "$sep" "$sep" "$sep"
printf '%s%s1%s1%s1.1:%%2\n' "$name" "$sep" "$sep" "$sep"
;;
  *)
printf 'unexpected tmux command: %s\n' "${1-}" >&2
exit 1
;;
esac
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Idle);
    summary.tmux_name = "work\tspace".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work\tspace:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("pane two".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: None,
            token_count: 77,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;

    restore_test_path(original_path);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].thought.as_deref(), Some("pane two"));
    assert_eq!(sessions[0].thought_state, ThoughtState::Active);
    assert_eq!(sessions[0].rest_state, RestState::Active);
    assert_eq!(sessions[0].token_count, 77);
}

#[tokio::test]
async fn list_sessions_keeps_summary_when_active_tmux_pane_batch_lookup_fails() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
printf 'boom\n' >&2
exit 1
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Idle);
    summary.tmux_name = "work".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("pane two".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: None,
            token_count: 77,
            context_limit: 200_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let sessions = supervisor.list_sessions().await;

    restore_test_path(original_path);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-live");
    assert_eq!(sessions[0].thought.as_deref(), None);
    assert_eq!(sessions[0].thought_state, ThoughtState::Holding);
}

#[tokio::test]
async fn list_sessions_skips_dropped_summary_replies() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(
            spawn_dropped_summary_handle("sess-drop", "tmux-drop", SessionState::Idle).await,
        )
        .await;

    let sessions = supervisor.list_sessions().await;

    assert!(sessions.is_empty());
}

#[tokio::test]
async fn list_sessions_keeps_cached_summary_when_live_reply_drops() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Idle);
    summary.tmux_name = "tmux-live".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;

    let initial = supervisor.list_sessions().await;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].transport_health, TransportHealth::Healthy);

    supervisor
        .insert_test_handle(
            spawn_dropped_summary_handle("sess-live", "tmux-live", SessionState::Idle).await,
        )
        .await;

    let sessions = supervisor.list_sessions().await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-live");
    assert_eq!(sessions[0].tmux_name, "tmux-live");
    assert_eq!(sessions[0].transport_health, TransportHealth::Degraded);
    assert_eq!(
        sessions[0].state_evidence.cause,
        SummaryFallbackReason::Dropped
            .cached_fallback()
            .expect("dropped fallback cause")
            .0
    );
    assert!(sessions[0].state_evidence.observed_at.is_none());
    assert!(!sessions[0].is_stale);
}

#[tokio::test]
async fn collect_live_summaries_keeps_cached_summary_when_live_reply_times_out() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(
            spawn_summary_handle(test_summary("sess-timeout", SessionState::Busy)).await,
        )
        .await;

    let initial = supervisor
        .collect_live_summaries(Duration::from_millis(10))
        .await;
    assert_eq!(initial.len(), 1);

    supervisor
        .insert_test_handle(spawn_hung_summary_handle("sess-timeout", "tmux-sess-timeout").await)
        .await;

    let sessions = supervisor
        .collect_live_summaries(Duration::from_millis(10))
        .await;

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "sess-timeout");
    assert_eq!(sessions[0].transport_health, TransportHealth::Overloaded);
    assert_eq!(
        sessions[0].state_evidence.cause,
        SummaryFallbackReason::Timeout
            .cached_fallback()
            .expect("timeout fallback cause")
            .0
    );
    assert!(sessions[0].state_evidence.observed_at.is_none());
    assert!(!sessions[0].is_stale);
}

#[tokio::test]
async fn list_sessions_skips_closed_command_channels() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(spawn_closed_summary_handle("sess-closed", "").await)
        .await;

    let sessions = supervisor.list_sessions().await;

    assert!(sessions.is_empty());
}

#[tokio::test]
async fn collect_session_snapshots_uses_summary_snapshot_and_thought_cache() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .insert_test_handle(spawn_summary_handle(test_summary("sess-1", SessionState::Busy)).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "sess-1".to_string(),
        ThoughtSnapshot {
            thought: Some("building release".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: Some("obj-1".to_string()),
            token_count: 55,
            context_limit: 210_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let infos = supervisor.collect_session_snapshots().await;
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].session_id, "sess-1");
    assert!(infos[0].replay_text.ends_with("replay tail"));
    assert_eq!(infos[0].thought.as_deref(), Some("building release"));
    assert_eq!(infos[0].token_count, 55);
    assert_eq!(infos[0].objective_fingerprint.as_deref(), Some("obj-1"));
}

#[tokio::test]
async fn collect_session_snapshots_merges_thought_snapshot_from_active_tmux_pane_batch_lookup() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, original_path) = install_fake_tmux(
        r#"#!/bin/sh
set -eu
case "${1-}" in
  list-panes)
sep=$(printf '\037')
name=$(printf 'work\tspace')
printf '%s%s0%s1%s1.0:%%1\n' "$name" "$sep" "$sep" "$sep"
printf '%s%s1%s1%s1.1:%%2\n' "$name" "$sep" "$sep" "$sep"
;;
  *)
printf 'unexpected tmux command: %s\n' "${1-}" >&2
exit 1
;;
esac
"#,
    );

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut summary = test_summary("sess-live", SessionState::Busy);
    summary.tmux_name = "work\tspace".to_string();
    supervisor
        .insert_test_handle(spawn_summary_handle(summary).await)
        .await;
    supervisor.thought_snapshots.write().await.insert(
        "tmux:work\tspace:1.1:%2".to_string(),
        ThoughtSnapshot {
            thought: Some("pane two".to_string()),
            thought_state: ThoughtState::Active,
            thought_source: ThoughtSource::Llm,
            rest_state: RestState::Active,
            commit_candidate: true,
            action_cues: Vec::new(),
            objective_changed_at: None,
            objective_fingerprint: Some("obj-pane".to_string()),
            token_count: 88,
            context_limit: 199_000,
            updated_at: Utc::now(),
            delivery: ThoughtDeliveryState::default(),
        },
    );

    let infos = supervisor.collect_session_snapshots().await;

    restore_test_path(original_path);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].session_id, "sess-live");
    assert_eq!(infos[0].thought.as_deref(), Some("pane two"));
    assert_eq!(infos[0].thought_state, ThoughtState::Active);
    assert_eq!(infos[0].rest_state, RestState::Active);
    assert_eq!(infos[0].objective_fingerprint.as_deref(), Some("obj-pane"));
    assert_eq!(infos[0].token_count, 88);
}

#[tokio::test]
async fn collect_session_snapshots_fans_out_actor_requests_before_timeouts() {
    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();

    for session_id in ["sess-a", "sess-b", "sess-c"] {
        supervisor
            .insert_test_handle(
                spawn_observed_hung_summary_handle(session_id, "", observed_tx.clone()).await,
            )
            .await;
    }
    drop(observed_tx);

    let collect = supervisor.collect_session_snapshots_with_timeout(Duration::from_secs(10));
    tokio::pin!(collect);
    let observations = async {
        let mut observed = Vec::new();
        for _ in 0..3 {
            observed.push(observed_rx.recv().await.expect("observed summary request"));
        }
        observed
    };
    tokio::pin!(observations);

    let observed = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            _ = &mut collect => panic!("hung actors should keep collection pending"),
            observed = &mut observations => observed,
        }
    })
    .await
    .expect("snapshot collection should request every actor before the first timeout");

    let observed: HashSet<_> = observed.into_iter().collect();
    let expected = HashSet::from_iter([
        "sess-a".to_string(),
        "sess-b".to_string(),
        "sess-c".to_string(),
    ]);
    assert_eq!(observed, expected);
}

#[tokio::test]
async fn create_session_uses_fake_tmux_and_bootstraps_codex_spawn() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (dir, original_path) = install_fake_tmux(
        r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  new-session|attach-session)
if [ "$cmd" = "new-session" ] && [ -n "${SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG:-}" ]; then
  printf '%s\n' "$@" > "${SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG}"
fi
while IFS= read -r line; do
  printf '%s\r\n' "$line"
done
;;
  display-message)
case "${5-}" in
  "#{pane_current_path}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_CWD:-/tmp/project}" ;;
  "#{pane_current_command}") printf '%s\n' "${SWIMMERS_FAKE_TMUX_COMMAND:-codex}" ;;
  "#{pane_pid}") printf '101\n' ;;
  "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
esac
;;
  send-keys)
printf 'unexpected send-keys during spawn\n' >&2
exit 9
;;
  kill-session)
exit 0
;;
  capture-pane)
printf 'captured pane\n'
;;
  list-sessions)
if [ -f "${SWIMMERS_FAKE_TMUX_SESSIONS:-}" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    printf '%s\n' "$line"
  done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
fi
;;
esac
"##,
    );

    let original_cwd = std::env::var_os("SWIMMERS_FAKE_TMUX_CWD");
    let original_cmd = std::env::var_os("SWIMMERS_FAKE_TMUX_COMMAND");
    let original_new_session_log = std::env::var_os("SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG");
    let new_session_log = dir.path().join("new-session.log");
    std::env::set_var("SWIMMERS_FAKE_TMUX_CWD", dir.path());
    std::env::set_var("SWIMMERS_FAKE_TMUX_COMMAND", "codex");
    std::env::set_var("SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG", &new_session_log);

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let created = supervisor
        .create_session(
            None,
            Some(dir.path().to_string_lossy().into_owned()),
            Some(crate::types::SpawnTool::Codex),
            Some("investigate startup".to_string()),
        )
        .await
        .expect("create session");

    assert_eq!(created.0.session_id, "sess_0");
    assert_eq!(created.0.tmux_name, "0");
    assert_eq!(created.0.tool.as_deref(), Some("Codex"));
    assert_eq!(created.0.cwd, dir.path().to_string_lossy());
    for _ in 0..20 {
        if new_session_log.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let new_session_log = std::fs::read_to_string(new_session_log).expect("new-session log");
    assert!(new_session_log.contains("new-session\n-s\n0\n-c\n"));
    assert!(new_session_log.contains("{ prompt_file="));
    assert!(new_session_log.contains("caam run codex -- \"$prompt\""));
    assert!(new_session_log.contains("falling back to raw codex"));
    assert!(new_session_log.contains("exec \"${SHELL:-/bin/sh}\""));
    assert!(!new_session_log.contains("investigate startup"));
    assert!(
        new_session_log
            .find("caam run codex -- \"$prompt\"")
            .expect("caam command")
            < new_session_log.find("codex-raw").expect("raw fallback"),
        "caam must be attempted before raw fallback"
    );
    supervisor
        .delete_session(
            &created.0.session_id,
            crate::config::SessionDeleteMode::DetachBridge,
        )
        .await
        .expect("cleanup session");

    restore_test_path(original_path);
    restore_test_env_var("SWIMMERS_FAKE_TMUX_CWD", original_cwd);
    restore_test_env_var("SWIMMERS_FAKE_TMUX_COMMAND", original_cmd);
    restore_test_env_var(
        "SWIMMERS_FAKE_TMUX_NEW_SESSION_LOG",
        original_new_session_log,
    );
}

#[tokio::test]
async fn discover_tmux_sessions_with_reason_uses_fake_tmux_listings() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin");
    let sessions_file = dir.path().join("sessions.txt");
    std::fs::write(&sessions_file, "11\nworkspace\n").expect("sessions");
    write_executable(
        &bin_dir.join("tmux"),
        r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
while IFS= read -r line || [ -n "$line" ]; do
  printf '%s\n' "$line"
done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
;;
  attach-session|new-session)
while IFS= read -r line; do
  printf '%s\r\n' "$line"
done
;;
  display-message)
case "${5-}" in
  "#{pane_current_command}") printf 'codex\n' ;;
  "#{pane_current_path}") printf '/tmp/project\n' ;;
  "#{pane_pid}") printf '101\n' ;;
  "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
esac
;;
  send-keys|kill-session|capture-pane)
exit 0
;;
esac
"##,
    );

    let original_path = std::env::var_os("PATH");
    let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
    prepend_test_path(&bin_dir, original_path.as_deref());
    std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    supervisor
        .discover_tmux_sessions_with_reason("test_discovery")
        .await
        .expect("discover sessions");

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_sessions {
        Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
        None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
    }

    let sessions = supervisor.sessions.read().await;
    assert_eq!(sessions.len(), 2);
    assert!(sessions.values().any(|handle| handle.tmux_name == "11"));
    assert!(sessions
        .values()
        .any(|handle| handle.tmux_name == "workspace"));
}

#[tokio::test]
async fn discover_tmux_sessions_reconciles_external_create_remove_and_restart() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let sessions_file;
    let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
    let (_dir, original_path) = install_fake_tmux(
        r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
while IFS= read -r line || [ -n "$line" ]; do
  printf '%s\n' "$line"
done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
;;
  list-panes)
exit 0
;;
  attach-session|new-session)
while IFS= read -r line; do
  printf '%s\r\n' "$line"
done
;;
  display-message)
case "${5-}" in
  "#{pane_current_command}") printf 'codex\n' ;;
  "#{pane_current_path}") printf '/tmp/project\n' ;;
  "#{pane_pid}") printf '101\n' ;;
  "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
esac
;;
  send-keys|kill-session|capture-pane)
exit 0
;;
esac
"##,
    );
    sessions_file = _dir.path().join("sessions.txt");
    std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    std::fs::write(&sessions_file, "alpha\nbeta\n").expect("initial sessions");
    supervisor
        .discover_tmux_sessions_with_reason("test_discovery")
        .await
        .expect("initial discover");
    let first_ids = {
        let sessions = supervisor.sessions.read().await;
        sessions
            .values()
            .map(|handle| (handle.tmux_name.clone(), handle.session_id.clone()))
            .collect::<HashMap<_, _>>()
    };
    assert_eq!(first_ids.len(), 2);
    let alpha_id = first_ids.get("alpha").expect("alpha id").clone();
    let beta_id = first_ids.get("beta").expect("beta id").clone();

    std::fs::write(&sessions_file, "beta\ngamma\n").expect("updated sessions");
    supervisor
        .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
        .await
        .expect("rediscover after remove/create");
    let after_remove = supervisor.list_sessions().await;
    assert_eq!(after_remove.len(), 2);
    assert!(after_remove
        .iter()
        .any(|summary| { summary.tmux_name == "beta" && summary.session_id == beta_id }));
    assert!(after_remove
        .iter()
        .any(|summary| summary.tmux_name == "gamma"));
    assert!(!after_remove
        .iter()
        .any(|summary| summary.tmux_name == "alpha"));
    {
        let stale = supervisor.stale_sessions.read().await;
        let alpha = stale
            .iter()
            .find(|summary| summary.tmux_name == "alpha")
            .expect("removed alpha should become stale");
        assert_eq!(alpha.session_id, alpha_id);
        assert_eq!(alpha.state, SessionState::Exited);
        assert!(alpha.is_stale);
        assert_eq!(alpha.transport_health, TransportHealth::Disconnected);
    }

    std::fs::write(&sessions_file, "alpha\nbeta\ngamma\n").expect("restarted sessions");
    supervisor
        .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
        .await
        .expect("rediscover after restart");
    let after_restart = supervisor.list_sessions().await;
    assert_eq!(after_restart.len(), 3);
    assert!(after_restart
        .iter()
        .any(|summary| { summary.tmux_name == "alpha" && summary.session_id == alpha_id }));

    supervisor
        .discover_tmux_sessions_with_reason("periodic_tmux_reconcile")
        .await
        .expect("dedup rediscover");
    let final_ids = {
        let sessions = supervisor.sessions.read().await;
        sessions
            .values()
            .map(|handle| (handle.tmux_name.clone(), handle.session_id.clone()))
            .collect::<HashMap<_, _>>()
    };
    assert_eq!(final_ids.len(), 3);
    assert_eq!(final_ids.get("alpha"), Some(&alpha_id));
    assert_eq!(final_ids.get("beta"), Some(&beta_id));

    restore_test_path(original_path);
    match original_sessions {
        Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
        None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
    }
}

#[tokio::test]
async fn adopt_tmux_session_reuses_stale_identity_and_rejects_duplicates() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
    let (_dir, original_path) = install_fake_tmux(
        r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
while IFS= read -r line || [ -n "$line" ]; do
  printf '%s\n' "$line"
done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
;;
  list-panes)
exit 0
;;
  attach-session|new-session)
while IFS= read -r line; do
  printf '%s\r\n' "$line"
done
;;
  display-message)
case "${5-}" in
  "#{pane_current_command}") printf 'codex\n' ;;
  "#{pane_current_path}") printf '/tmp/project\n' ;;
  "#{pane_pid}") printf '101\n' ;;
  "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
esac
;;
  send-keys|kill-session|capture-pane)
exit 0
;;
esac
"##,
    );
    let sessions_file = _dir.path().join("sessions.txt");
    std::fs::write(&sessions_file, "alpha\n").expect("sessions");
    std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    let mut stale = supervisor.build_placeholder_summary("sess_42", "alpha");
    stale.state = SessionState::Exited;
    stale.is_stale = true;
    stale.transport_health = TransportHealth::Disconnected;
    stale.cwd = "/tmp/project".to_string();
    supervisor.stale_sessions.write().await.push(stale);

    let adopted = supervisor
        .adopt_tmux_session("alpha".to_string(), None)
        .await
        .expect("adopt stale tmux session");
    assert!(adopted.reused_session_id);
    assert_eq!(adopted.session.session_id, "sess_42");
    assert_eq!(adopted.session.tmux_name, "alpha");
    assert!(!adopted.session.is_stale);

    let active = supervisor.list_sessions().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, "sess_42");
    assert!(supervisor.stale_sessions.read().await.is_empty());

    let duplicate = supervisor
        .adopt_tmux_session("alpha".to_string(), None)
        .await
        .expect_err("already tracked tmux should be rejected");
    assert_eq!(
        duplicate,
        TmuxAdoptError::AlreadyTracked {
            tmux_name: "alpha".to_string(),
            session_id: "sess_42".to_string()
        }
    );

    restore_test_path(original_path);
    match original_sessions {
        Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
        None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
    }
}

#[tokio::test]
async fn adopt_tmux_session_preserves_exact_whitespace_padded_name() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
    let original_attach_log = std::env::var_os("SWIMMERS_FAKE_TMUX_ATTACH_LOG");
    let (_dir, original_path) = install_fake_tmux(
        r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
while IFS= read -r line || [ -n "$line" ]; do
  printf '%s\n' "$line"
done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
;;
  attach-session)
if [ -n "${SWIMMERS_FAKE_TMUX_ATTACH_LOG:-}" ]; then
  printf '%s\n' "$@" > "${SWIMMERS_FAKE_TMUX_ATTACH_LOG}"
fi
exit 0
;;
  list-panes|send-keys|kill-session|capture-pane)
exit 0
;;
  display-message)
case "${5-}" in
  "#{pane_current_command}") printf 'codex\n' ;;
  "#{pane_current_path}") printf '/tmp/project\n' ;;
  "#{pane_pid}") printf '101\n' ;;
  "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
esac
;;
esac
"##,
    );
    let sessions_file = _dir.path().join("sessions.txt");
    let attach_log = _dir.path().join("attach.log");
    std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);
    std::env::set_var("SWIMMERS_FAKE_TMUX_ATTACH_LOG", &attach_log);

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    assert_eq!(
        supervisor
            .adopt_tmux_session(String::new(), None)
            .await
            .expect_err("empty tmux target should still be rejected"),
        TmuxAdoptError::EmptyTmuxName
    );

    std::fs::write(&sessions_file, "  padded  \n").expect("sessions");
    let stale = supervisor.build_placeholder_summary("sess_7", "  padded  ");
    supervisor.stale_sessions.write().await.push(stale);

    let adopted = supervisor
        .adopt_tmux_session("  padded  ".to_string(), Some("sess_7".to_string()))
        .await
        .expect("exact whitespace-padded tmux target should be adopted");
    assert!(adopted.reused_session_id);
    assert_eq!(adopted.session.session_id, "sess_7");
    assert_eq!(adopted.session.tmux_name, "  padded  ");
    let attach_args = (0..20)
        .find_map(|_| {
            std::fs::read_to_string(&attach_log).ok().or_else(|| {
                std::thread::sleep(Duration::from_millis(10));
                None
            })
        })
        .expect("attach log");
    assert_eq!(attach_args, "attach-session\n-t\n=  padded  \n");

    let missing_trimmed = supervisor
        .adopt_tmux_session("padded".to_string(), None)
        .await
        .expect_err("trimmed spelling should not match exact tmux target");
    assert_eq!(
        missing_trimmed,
        TmuxAdoptError::TargetNotFound {
            tmux_name: "padded".to_string()
        }
    );

    restore_test_path(original_path);
    match original_sessions {
        Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
        None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
    }
    match original_attach_log {
        Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_ATTACH_LOG", value),
        None => std::env::remove_var("SWIMMERS_FAKE_TMUX_ATTACH_LOG"),
    }
}

#[tokio::test]
async fn adopt_tmux_session_rejects_missing_ambiguous_and_conflicting_targets() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let original_sessions = std::env::var_os("SWIMMERS_FAKE_TMUX_SESSIONS");
    let (_dir, original_path) = install_fake_tmux(
        r##"#!/bin/sh
set -eu
cmd="${1-}"
case "$cmd" in
  list-sessions)
while IFS= read -r line || [ -n "$line" ]; do
  printf '%s\n' "$line"
done < "${SWIMMERS_FAKE_TMUX_SESSIONS}"
;;
  list-panes|send-keys|kill-session|capture-pane)
exit 0
;;
  attach-session|new-session)
while IFS= read -r line; do
  printf '%s\r\n' "$line"
done
;;
  display-message)
case "${5-}" in
  "#{pane_current_command}") printf 'codex\n' ;;
  "#{pane_current_path}") printf '/tmp/project\n' ;;
  "#{pane_pid}") printf '101\n' ;;
  "#{window_index}.#{pane_index}:#{pane_id}") printf '0.0:%%1\n' ;;
esac
;;
esac
"##,
    );
    let sessions_file = _dir.path().join("sessions.txt");
    std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", &sessions_file);

    let supervisor = SessionSupervisor::new(Arc::new(Config::default()));
    std::fs::write(&sessions_file, "alpha\n").expect("sessions");
    assert_eq!(
        supervisor
            .adopt_tmux_session("beta".to_string(), None)
            .await
            .expect_err("missing tmux target should be rejected"),
        TmuxAdoptError::TargetNotFound {
            tmux_name: "beta".to_string()
        }
    );

    std::fs::write(&sessions_file, "alpha\nalpha\n").expect("duplicate sessions");
    assert_eq!(
        supervisor
            .adopt_tmux_session("alpha".to_string(), None)
            .await
            .expect_err("ambiguous tmux target should be rejected"),
        TmuxAdoptError::AmbiguousTarget {
            tmux_name: "alpha".to_string(),
            matches: 2
        }
    );

    std::fs::write(&sessions_file, "alpha\n").expect("sessions");
    let stale = supervisor.build_placeholder_summary("sess_7", "beta");
    supervisor.stale_sessions.write().await.push(stale);
    assert_eq!(
        supervisor
            .adopt_tmux_session("alpha".to_string(), Some("sess_7".to_string()))
            .await
            .expect_err("conflicting stale identity should be rejected"),
        TmuxAdoptError::StaleSessionConflict {
            session_id: "sess_7".to_string(),
            stale_tmux_name: "beta".to_string(),
            requested_tmux_name: "alpha".to_string()
        }
    );

    restore_test_path(original_path);
    match original_sessions {
        Some(value) => std::env::set_var("SWIMMERS_FAKE_TMUX_SESSIONS", value),
        None => std::env::remove_var("SWIMMERS_FAKE_TMUX_SESSIONS"),
    }
}
