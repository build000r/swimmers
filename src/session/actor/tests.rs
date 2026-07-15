use super::process_tree::{
    process_entries_cache, ProcessEntriesCache, ProcessEntry, PROCESS_ENTRIES_CACHE_TTL,
};
use super::{build_tmux_spawn_command, build_tmux_spawn_command_args};
use super::{
    capture_pane_tail_with_command, compare_session_state_change, cwd_from_osc7_payload,
    cwd_from_osc7_payload_with_local_hosts, cwd_update, deadline_sleep, deadline_sleep_after,
    extract_cwd_from_title, find_osc_payload_end, initial_spawn_pty_size,
    normalize_submit_line_text, osc7_cwd_update_plan, osc_payloads, pty_read_error_log,
    pty_read_step, query_tmux_session_created, resolve_tmux_colorterm, resolve_tmux_term,
    resolve_tmux_terminal_env, run_bounded_tmux_command, should_clear_startup_replay,
    should_refresh_cwd_from_tmux, should_refresh_tool_from_tmux, state_detector_for_initial_tool,
    submit_line_fallback_input, subscriber_cap_rejection, title_cwd_update, title_tool_update,
    tmux_input_chunks, tool_refresh_changes_tool, validate_spawn_start_cwd, write_and_flush_input,
    write_input_counts_as_activity, ControlEvent, DeadlineSleep, LivenessReconciliation,
    LivenessRefresh, OutputFrame, PaneLiveness, PtyReadErrorLog, PtyReadLoopStep, SessionActor,
    SessionCommand, SubscribeOutcome, TmuxInputChunk, TmuxSpawnMode, CWD_REFRESH_MIN_INTERVAL,
    MAX_OUTPUT_SUBSCRIBERS_PER_SESSION, TOOL_REFRESH_MIN_INTERVAL,
};
use crate::config::Config;
use crate::scroll::guard::ScrollGuard;
use crate::session::replay_ring::ReplayRing;
use crate::types::{SessionState, SessionStatePayload, StateEvidence, TransportHealth};
use chrono::{TimeZone, Utc};
use portable_pty::{native_pty_system, PtySize};
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};

fn argv_strings(command: &portable_pty::CommandBuilder) -> Vec<String> {
    command
        .get_argv()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn test_actor() -> SessionActor {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let writer = pair.master.take_writer().expect("writer");
    let (_cmd_tx, cmd_rx) = mpsc::channel(8);
    let (event_tx, _) = broadcast::channel::<ControlEvent>(8);

    SessionActor {
        session_id: "sess-test".to_string(),
        tmux_name: "demo".to_string(),
        tmux_target: crate::tmux_target::TmuxTarget::Default,
        config: Arc::new(Config::default()),
        master: pair.master,
        writer,
        state_detector: state_detector_for_initial_tool(Some("Codex")),
        scroll_guard: ScrollGuard::new(),
        replay_ring: ReplayRing::new(512 * 1024),
        subscribers: HashMap::new(),
        cmd_rx,
        event_tx,
        cols: 80,
        rows: 24,
        cwd: "/tmp/project".to_string(),
        last_cwd_refresh_at: Instant::now(),
        last_tool_refresh_at: Instant::now(),
        last_liveness_check_at: Instant::now(),
        tmux_pane_metadata_cache: None,
        tool: Some("Codex".to_string()),
        last_skill: None,
        batch: None,
        input_line_buffer: String::new(),
        last_activity_at: Utc::now(),
        session_started_at: Utc::now(),
        clear_replay_on_first_idle: false,
    }
}

#[tokio::test]
async fn startup_metadata_refreshes_reuse_one_tmux_metadata_query() {
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
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf '/tmp/batched\\037codex\\037101\\0371774274168\\n'\n",
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

    let mut actor = test_actor();
    actor.cwd.clear();
    actor.tool = None;
    actor.maybe_refresh_session_started_at().await;
    actor.maybe_refresh_cwd_from_tmux(true).await;
    actor.maybe_refresh_tool_from_tmux(true).await;

    assert_eq!(actor.cwd, "/tmp/batched");
    assert_eq!(actor.tool.as_deref(), Some("Codex"));
    assert_eq!(
        actor.session_started_at,
        Utc.timestamp_opt(1_774_274_168, 0).single().unwrap()
    );
    let calls = std::fs::read_to_string(&tmux_log).expect("tmux log");
    assert_eq!(calls.lines().count(), 1, "{calls}");
    assert!(calls.contains("#{pane_current_path}"));
    assert!(calls.contains("#{pane_current_command}"));
    assert!(calls.contains("#{pane_pid}"));
    assert!(calls.contains("#{session_created}"));

    restore_path(previous_path);
}

fn output_frame(seq: u64, data: &[u8]) -> OutputFrame {
    OutputFrame {
        seq,
        data: data.to_vec(),
    }
}

#[test]
fn deadline_sleep_without_deadline_pends() {
    assert_eq!(deadline_sleep(None), DeadlineSleep::Pending);
}

#[test]
fn deadline_sleep_after_ready_for_past_and_current_deadlines() {
    let now = Instant::now();

    assert_eq!(deadline_sleep_after(now, now), DeadlineSleep::Ready);
    assert_eq!(
        deadline_sleep_after(now - Duration::from_millis(1), now),
        DeadlineSleep::Ready
    );
}

#[test]
fn deadline_sleep_after_preserves_positive_duration() {
    let now = Instant::now();
    let duration = Duration::from_millis(123);

    assert_eq!(
        deadline_sleep_after(now + duration, now),
        DeadlineSleep::Sleep(duration)
    );
}

#[tokio::test]
async fn sleep_until_deadline_returns_immediately_for_past_deadline() {
    let past_deadline = Instant::now() - Duration::from_millis(1);

    tokio::time::timeout(
        Duration::from_millis(50),
        SessionActor::sleep_until_deadline(Some(past_deadline)),
    )
    .await
    .expect("past deadlines should return immediately");
}

#[tokio::test]
async fn sleep_until_deadline_without_deadline_can_be_cancelled() {
    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        SessionActor::sleep_until_deadline(None)
    )
    .await
    .is_err());
}

#[tokio::test]
async fn sleep_until_deadline_future_wait_can_be_cancelled() {
    let future_deadline = Instant::now() + Duration::from_secs(60);

    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        SessionActor::sleep_until_deadline(Some(future_deadline)),
    )
    .await
    .is_err());
}

#[test]
fn pty_read_step_forwards_exact_read_slice_and_continues() {
    let (tx, mut rx) = mpsc::channel(1);

    let step = pty_read_step("sess-test", Ok(3), b"abcdef", &tx);

    assert_eq!(step, PtyReadLoopStep::Continue);
    assert_eq!(rx.try_recv().expect("pty bytes"), b"abc".to_vec());
    assert!(rx.try_recv().is_err());
}

#[test]
fn pty_read_step_eof_stops_without_sending() {
    let (tx, mut rx) = mpsc::channel(1);

    let step = pty_read_step("sess-test", Ok(0), b"abcdef", &tx);

    assert_eq!(step, PtyReadLoopStep::Stop);
    assert!(rx.try_recv().is_err());
}

#[test]
fn pty_read_step_retries_on_interrupted_error() {
    let (tx, mut rx) = mpsc::channel(1);

    let interrupted = std::io::Error::from(std::io::ErrorKind::Interrupted);
    let step = pty_read_step("sess-test", Err(interrupted), b"abcdef", &tx);

    // EINTR is retryable, so the read loop must continue rather than stop.
    assert_eq!(step, PtyReadLoopStep::Continue);
    assert!(rx.try_recv().is_err());
}

#[test]
fn pty_read_step_stops_when_receiver_dropped() {
    let (tx, rx) = mpsc::channel(1);
    drop(rx);

    let step = pty_read_step("sess-test", Ok(3), b"abcdef", &tx);

    assert_eq!(step, PtyReadLoopStep::Stop);
}

#[test]
fn pty_read_step_stops_for_likely_child_exit_error() {
    let (tx, mut rx) = mpsc::channel(1);
    let err = io::Error::new(io::ErrorKind::Other, "child exited");

    let step = pty_read_step("sess-test", Err(err), b"abcdef", &tx);

    assert_eq!(step, PtyReadLoopStep::Stop);
    assert!(rx.try_recv().is_err());
}

#[test]
fn pty_read_step_stops_for_non_other_read_error() {
    let (tx, mut rx) = mpsc::channel(1);
    // A genuinely fatal non-Other error (not the retryable Interrupted) stops.
    let err = io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe");

    let step = pty_read_step("sess-test", Err(err), b"abcdef", &tx);

    assert_eq!(step, PtyReadLoopStep::Stop);
    assert!(rx.try_recv().is_err());
}

#[test]
fn pty_read_error_log_classifies_other_as_likely_child_exit() {
    let err = io::Error::new(io::ErrorKind::Other, "child exited");

    assert_eq!(pty_read_error_log(&err), PtyReadErrorLog::LikelyChildExit);
}

#[test]
fn pty_read_error_log_classifies_non_other_as_error() {
    let err = io::Error::new(io::ErrorKind::Interrupted, "interrupted");

    assert_eq!(pty_read_error_log(&err), PtyReadErrorLog::Error);
}

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

fn restore_env_var(key: &str, value: Option<std::ffi::OsString>) {
    if let Some(value) = value {
        std::env::set_var(key, value);
    } else {
        std::env::remove_var(key);
    }
}

fn make_executable(path: &std::path::Path) {
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

fn install_fake_tmux(script: &str) -> (tempfile::TempDir, Option<std::ffi::OsString>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let tmux = bin_dir.join("tmux");
    std::fs::write(&tmux, script).expect("tmux script");
    make_executable(&tmux);
    let previous_path = std::env::var_os("PATH");
    let mut entries = vec![bin_dir.as_os_str().to_os_string()];
    if let Some(existing) = previous_path.as_ref() {
        entries.extend(std::env::split_paths(existing).map(|path| path.into_os_string()));
    }
    for system_dir in ["/bin", "/usr/bin"] {
        let system_dir = std::path::Path::new(system_dir);
        if system_dir.is_dir()
            && !entries
                .iter()
                .any(|entry| std::path::Path::new(entry) == system_dir)
        {
            entries.push(system_dir.as_os_str().to_os_string());
        }
    }
    std::env::set_var("PATH", std::env::join_paths(entries).expect("path"));
    (dir, previous_path)
}

#[test]
fn spawn_initial_pty_size_matches_tmux_bootstrap_contract() {
    let size = initial_spawn_pty_size();

    assert_eq!(size.rows, 24);
    assert_eq!(size.cols, 80);
    assert_eq!(size.pixel_width, 0);
    assert_eq!(size.pixel_height, 0);
}

#[test]
fn spawn_start_cwd_validation_only_applies_to_new_sessions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, "contents").expect("file");
    let file = file.to_string_lossy().into_owned();

    let error =
        validate_spawn_start_cwd(TmuxSpawnMode::New, Some(&file)).expect_err("file rejected");
    assert_eq!(
        error.to_string(),
        format!("session cwd does not exist or is not a directory: {file}")
    );
    validate_spawn_start_cwd(TmuxSpawnMode::Attach, Some(&file))
        .expect("attach skips cwd validation");
    validate_spawn_start_cwd(
        TmuxSpawnMode::New,
        Some(dir.path().to_str().expect("utf8 path")),
    )
    .expect("directory accepted");
    validate_spawn_start_cwd(TmuxSpawnMode::New, None).expect("missing cwd accepted");
}

#[test]
fn spawn_attach_command_targets_exact_tmux_session() {
    let command = build_tmux_spawn_command_args(
        TmuxSpawnMode::Attach,
        &crate::tmux_target::TmuxTarget::Default,
        "demo.session",
        None,
        None,
    );

    assert_eq!(
        argv_strings(&command),
        vec![
            "tmux".to_string(),
            "attach-session".to_string(),
            "-t".to_string(),
            crate::tmux_target::exact_session_target("demo.session"),
        ]
    );
}

#[test]
fn spawn_new_session_command_preserves_optional_cwd_and_initial_command_order() {
    let command = build_tmux_spawn_command_args(
        TmuxSpawnMode::New,
        &crate::tmux_target::TmuxTarget::Default,
        "demo.session",
        Some("/tmp/project"),
        Some("cargo test"),
    );

    assert_eq!(
        argv_strings(&command),
        vec![
            "tmux".to_string(),
            "new-session".to_string(),
            "-s".to_string(),
            "demo.session".to_string(),
            "-c".to_string(),
            "/tmp/project".to_string(),
            "cargo test".to_string(),
        ]
    );
}

#[test]
fn spawn_new_session_command_omits_absent_optional_args() {
    let command = build_tmux_spawn_command_args(
        TmuxSpawnMode::New,
        &crate::tmux_target::TmuxTarget::Default,
        "demo.session",
        None,
        None,
    );

    assert_eq!(
        argv_strings(&command),
        vec![
            "tmux".to_string(),
            "new-session".to_string(),
            "-s".to_string(),
            "demo.session".to_string(),
        ]
    );
}

#[test]
fn spawn_command_env_removes_nested_tmux_and_sets_terminal_defaults() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let previous_tmux = std::env::var_os("TMUX");
    let previous_tmux_pane = std::env::var_os("TMUX_PANE");
    let previous_term = std::env::var_os("TERM");
    let previous_colorterm = std::env::var_os("COLORTERM");
    std::env::set_var("TMUX", "nested");
    std::env::set_var("TMUX_PANE", "%1");
    std::env::set_var("TERM", "dumb");
    std::env::remove_var("COLORTERM");

    let command = build_tmux_spawn_command(
        TmuxSpawnMode::Attach,
        &crate::tmux_target::TmuxTarget::Default,
        "sess-test",
        "demo.session",
        None,
        None,
    );

    assert_eq!(command.get_env("TMUX"), None);
    assert_eq!(command.get_env("TMUX_PANE"), None);
    assert_eq!(
        command.get_env("TERM"),
        Some(std::ffi::OsStr::new("xterm-256color"))
    );
    assert_eq!(
        command.get_env("COLORTERM"),
        Some(std::ffi::OsStr::new("truecolor"))
    );
    assert_eq!(
        command.get_env("TERM_PROGRAM"),
        Some(std::ffi::OsStr::new("swimmers"))
    );

    restore_env_var("TMUX", previous_tmux);
    restore_env_var("TMUX_PANE", previous_tmux_pane);
    restore_env_var("TERM", previous_term);
    restore_env_var("COLORTERM", previous_colorterm);
}

#[test]
fn evidence_refresh_emits_session_state_event_without_state_transition() {
    let mut actor = test_actor();
    let mut rx = actor.event_tx.subscribe();
    let previous_state = actor.state_detector.state();
    let previous_evidence = actor.state_detector.state_evidence();

    actor.state_detector.process_output(b"\x1b]133;A\x07");
    let result = actor.maybe_emit_state_change(previous_state, previous_evidence);

    assert_eq!(result, None);
    let event = rx.try_recv().expect("session_state event");
    assert_eq!(event.event, "session_state");
    let payload: SessionStatePayload =
        serde_json::from_value(event.payload).expect("session_state payload");
    assert_eq!(payload.state, SessionState::Idle);
    assert_eq!(payload.previous_state, SessionState::Idle);
    assert_eq!(payload.state_evidence.cause, "osc133_prompt");
    assert_eq!(
        payload.state_evidence.confidence,
        crate::types::StateConfidence::High
    );
}

#[test]
fn state_change_detection_distinguishes_noop_evidence_and_state_paths() {
    let previous_evidence = StateEvidence::unobserved("initial");

    let noop = compare_session_state_change(
        SessionState::Idle,
        previous_evidence.clone(),
        SessionState::Idle,
        None,
        previous_evidence.clone(),
    );
    assert!(!noop.should_emit_event());
    assert_eq!(noop.changed_state(), None);

    let evidence_only = compare_session_state_change(
        SessionState::Idle,
        previous_evidence.clone(),
        SessionState::Idle,
        None,
        StateEvidence::unobserved("osc133_prompt"),
    );
    assert!(evidence_only.should_emit_event());
    assert_eq!(evidence_only.changed_state(), None);

    let state_transition = compare_session_state_change(
        SessionState::Idle,
        previous_evidence,
        SessionState::Busy,
        Some("cargo test".to_string()),
        StateEvidence::unobserved("local_input"),
    );
    assert!(state_transition.should_emit_event());
    assert_eq!(state_transition.changed_state(), Some(SessionState::Busy));
}

#[test]
fn state_change_payload_preserves_exit_reason_and_transport_health() {
    let detection = compare_session_state_change(
        SessionState::Busy,
        StateEvidence::unobserved("local_input"),
        SessionState::Exited,
        None,
        StateEvidence::unobserved("process_exit"),
    );

    let payload = detection.into_payload(Some("process_exit".to_string()));

    assert_eq!(payload.state, SessionState::Exited);
    assert_eq!(payload.previous_state, SessionState::Busy);
    assert_eq!(payload.state_evidence.cause, "process_exit");
    // A terminal exit reports the transport as gone, matching discovery.rs.
    assert_eq!(payload.transport_health, TransportHealth::Disconnected);
    assert_eq!(payload.exit_reason.as_deref(), Some("process_exit"));
}

#[test]
fn state_change_event_with_exit_reason_preserves_payload_fields() {
    let mut actor = test_actor();
    let mut rx = actor.event_tx.subscribe();
    let previous_state = actor.state_detector.state();
    let previous_evidence = actor.state_detector.state_evidence();

    actor.state_detector.mark_exited();
    let result = actor.maybe_emit_state_change_with_exit_reason(
        previous_state,
        previous_evidence,
        Some("process_exit".to_string()),
    );

    assert_eq!(result, Some(SessionState::Exited));
    let event = rx.try_recv().expect("session_state event");
    assert_eq!(event.event, "session_state");
    assert_eq!(event.session_id, "sess-test");
    let payload: SessionStatePayload =
        serde_json::from_value(event.payload).expect("session_state payload");
    assert_eq!(payload.state, SessionState::Exited);
    assert_eq!(payload.previous_state, SessionState::Idle);
    // A terminal exit reports the transport as gone, matching discovery.rs.
    assert_eq!(payload.transport_health, TransportHealth::Disconnected);
    assert_eq!(payload.exit_reason.as_deref(), Some("process_exit"));
}

#[test]
fn state_change_event_returns_transition_with_no_receivers() {
    let mut actor = test_actor();
    let previous_state = actor.state_detector.state();
    let previous_evidence = actor.state_detector.state_evidence();

    actor.state_detector.mark_exited();
    let result = actor.maybe_emit_state_change_with_exit_reason(
        previous_state,
        previous_evidence,
        Some("process_exit".to_string()),
    );

    assert_eq!(result, Some(SessionState::Exited));
}

#[test]
fn liveness_refresh_actions_preserve_cwd_then_tool_order() {
    let actions: Vec<_> = LivenessReconciliation {
        refresh_cwd: true,
        refresh_tool: true,
    }
    .refresh_actions()
    .collect();

    assert_eq!(actions, vec![LivenessRefresh::Cwd, LivenessRefresh::Tool]);
}

#[test]
fn liveness_refresh_actions_skip_disabled_refreshes() {
    let no_actions: Vec<_> = LivenessReconciliation::default()
        .refresh_actions()
        .collect();
    assert!(no_actions.is_empty());

    let tool_only: Vec<_> = LivenessReconciliation {
        refresh_cwd: false,
        refresh_tool: true,
    }
    .refresh_actions()
    .collect();
    assert_eq!(tool_only, vec![LivenessRefresh::Tool]);
}

#[test]
fn initial_tool_enables_tui_mode_before_liveness_reconciliation() {
    let mut actor = test_actor();

    actor.state_detector.note_input();
    actor.reconcile_liveness(PaneLiveness {
        has_children: true,
        descendant_cpu: 0.0,
        process_snapshot_fresh: true,
    });

    assert_eq!(actor.state_detector.state(), SessionState::Busy);
    assert_eq!(actor.state_detector.state_evidence().cause, "local_input");
}

#[tokio::test]
async fn maybe_check_liveness_skips_exited_sessions() {
    let mut actor = test_actor();
    actor.state_detector.mark_exited();
    // Should return immediately without trying tmux (tmux_name "demo" does not exist)
    actor.maybe_check_liveness().await;
    // If we reach here without hanging/panicking, the early-return worked
}

#[tokio::test]
async fn build_summary_reports_drowsy_when_idle_past_threshold() {
    // End-to-end wiring check: prove that build_summary feeds
    // self.last_activity_at into rest_state_from_idle and that the result
    // lands on SessionSummary.rest_state unclobbered. Pure math for the
    // ladder is covered by types::rest_state_tests; this guards the
    // actor-side plumbing.
    let mut actor = test_actor();
    // StateDetector::new() defaults to SessionState::Idle.
    let aged = Utc::now() - chrono::Duration::minutes(10);
    actor.last_activity_at = aged;

    let summary = actor.build_summary();

    assert_eq!(summary.state, crate::types::SessionState::Idle);
    assert_eq!(summary.rest_state, crate::types::RestState::Drowsy);
    assert_eq!(summary.last_activity_at, aged);
}

#[tokio::test]
async fn build_summary_reports_active_for_fresh_idle_session() {
    // Regression guard: a brand-new idle session (last_activity_at = now)
    // must not immediately report Drowsy/Sleeping.
    let actor = test_actor();
    let summary = actor.build_summary();
    assert_eq!(summary.state, crate::types::SessionState::Idle);
    assert_eq!(summary.rest_state, crate::types::RestState::Active);
}

#[test]
fn handle_resize_clamps_zero_and_one_cell_dimensions() {
    let mut actor = test_actor();

    actor.handle_resize(0, 1);

    assert_eq!(actor.cols, crate::types::TERMINAL_RESIZE_MIN_COLS);
    assert_eq!(actor.rows, crate::types::TERMINAL_RESIZE_MIN_ROWS);
}

#[test]
fn handle_resize_clamps_huge_dimensions() {
    let mut actor = test_actor();

    actor.handle_resize(u16::MAX, u16::MAX);

    assert_eq!(actor.cols, crate::types::TERMINAL_RESIZE_MAX_COLS);
    assert_eq!(actor.rows, crate::types::TERMINAL_RESIZE_MAX_ROWS);
}

#[tokio::test]
async fn broadcast_delivers_frame_to_active_subscribers() {
    let mut actor = test_actor();
    let (client_one_tx, mut client_one_rx) = mpsc::channel(1);
    let (client_two_tx, mut client_two_rx) = mpsc::channel(1);
    actor.subscribers.insert(11, client_one_tx);
    actor.subscribers.insert(22, client_two_tx);

    actor.broadcast(output_frame(7, b"hello")).await;

    let client_one_frame = client_one_rx.try_recv().expect("client one frame");
    assert_eq!(client_one_frame.seq, 7);
    assert_eq!(client_one_frame.data, b"hello".to_vec());
    let client_two_frame = client_two_rx.try_recv().expect("client two frame");
    assert_eq!(client_two_frame.seq, 7);
    assert_eq!(client_two_frame.data, b"hello".to_vec());
    assert_eq!(actor.subscribers.len(), 2);
}

#[tokio::test]
async fn broadcast_removes_full_subscriber_without_replacing_queued_frame() {
    let mut actor = test_actor();
    let (client_tx, mut client_rx) = mpsc::channel(1);
    client_tx
        .try_send(output_frame(1, b"queued"))
        .expect("prefill subscriber channel");
    actor.subscribers.insert(33, client_tx);

    actor.broadcast(output_frame(2, b"new")).await;

    assert!(!actor.subscribers.contains_key(&33));
    let queued_frame = client_rx.try_recv().expect("queued frame");
    assert_eq!(queued_frame.seq, 1);
    assert_eq!(queued_frame.data, b"queued".to_vec());
    assert!(client_rx.try_recv().is_err());
}

#[tokio::test]
async fn broadcast_removes_closed_subscriber() {
    let mut actor = test_actor();
    let (client_tx, client_rx) = mpsc::channel(1);
    drop(client_rx);
    actor.subscribers.insert(44, client_tx);

    actor.broadcast(output_frame(3, b"closed")).await;

    assert!(!actor.subscribers.contains_key(&44));
}

#[tokio::test]
async fn handle_subscribe_replays_requested_frames_before_attaching_client() {
    let mut actor = test_actor();
    let first_seq = actor.replay_ring.push(b"first");
    let second_seq = actor.replay_ring.push(b"second");
    let (client_tx, mut client_rx) = mpsc::channel(4);

    let outcome = actor
        .handle_subscribe(55, client_tx, Some(first_seq.saturating_sub(1)))
        .await;

    assert!(matches!(outcome, SubscribeOutcome::Ok));
    assert!(actor.subscribers.contains_key(&55));
    let first = client_rx.try_recv().expect("first replay frame");
    assert_eq!(first.seq, first_seq);
    assert_eq!(first.data, b"first".to_vec());
    let second = client_rx.try_recv().expect("second replay frame");
    assert_eq!(second.seq, second_seq);
    assert_eq!(second.data, b"second".to_vec());
}

#[tokio::test]
async fn handle_subscribe_prunes_closed_subscribers_before_cap_check() {
    let mut actor = test_actor();
    for client_id in 0..MAX_OUTPUT_SUBSCRIBERS_PER_SESSION as u64 {
        let (client_tx, client_rx) = mpsc::channel(1);
        drop(client_rx);
        actor.subscribers.insert(client_id, client_tx);
    }
    let (client_tx, _client_rx) = mpsc::channel(1);

    let outcome = actor.handle_subscribe(99, client_tx, None).await;

    assert!(matches!(outcome, SubscribeOutcome::Ok));
    assert_eq!(actor.subscribers.len(), 1);
    assert!(actor.subscribers.contains_key(&99));
}

#[tokio::test]
async fn handle_subscribe_rejects_when_open_subscriber_cap_is_reached() {
    let mut actor = test_actor();
    let mut receivers = Vec::new();
    for client_id in 0..MAX_OUTPUT_SUBSCRIBERS_PER_SESSION as u64 {
        let (client_tx, client_rx) = mpsc::channel(1);
        receivers.push(client_rx);
        actor.subscribers.insert(client_id, client_tx);
    }
    let (client_tx, _client_rx) = mpsc::channel(1);

    let outcome = actor.handle_subscribe(100, client_tx, None).await;

    match outcome {
        SubscribeOutcome::Rejected { reason } => {
            assert_eq!(
                reason,
                subscriber_cap_rejection(MAX_OUTPUT_SUBSCRIBERS_PER_SESSION).reason
            );
        }
        _ => panic!("expected subscriber cap rejection"),
    }
    assert_eq!(actor.subscribers.len(), MAX_OUTPUT_SUBSCRIBERS_PER_SESSION);
}

#[tokio::test]
async fn handle_subscribe_does_not_attach_client_that_drops_during_replay() {
    let mut actor = test_actor();
    actor.replay_ring.push(b"first");
    let (client_tx, client_rx) = mpsc::channel(1);
    drop(client_rx);

    let outcome = actor.handle_subscribe(66, client_tx, Some(0)).await;

    assert!(matches!(outcome, SubscribeOutcome::Ok));
    assert!(!actor.subscribers.contains_key(&66));
}

#[test]
fn cwd_update_trims_rejects_empty_and_skips_unchanged_paths() {
    assert_eq!(
        cwd_update("/tmp/project", " /tmp/other "),
        Some("/tmp/other".to_string())
    );
    assert_eq!(cwd_update("/tmp/project", "   "), None);
    assert_eq!(cwd_update("/tmp/project", "/tmp/project"), None);
}

#[test]
fn osc7_cwd_update_plan_preserves_payload_order_and_update_semantics() {
    let text = concat!(
        "\x1b]7;file://localhost/tmp/project\x07",
        "\x1b]7;file://localhost/tmp/one\x07",
        "\x1b]7;http://localhost/tmp/ignored\x07",
        "\x1b]7;\x07",
        "\x1b]7;file://localhost/tmp/one\x07",
        "\x1b]7;file://localhost/tmp/two\x1b\\",
        "\x1b]7;file://localhost/tmp/one\x07",
    );

    assert_eq!(
        osc7_cwd_update_plan("/tmp/project", text),
        vec![
            "/tmp/one".to_string(),
            "/tmp/two".to_string(),
            "/tmp/one".to_string()
        ]
    );
}

#[test]
fn apply_osc7_payloads_updates_cwd_and_emits_events_in_order() {
    let mut actor = test_actor();
    let mut rx = actor.event_tx.subscribe();

    actor.apply_osc7_payloads(concat!(
        "\x1b]7;file://localhost/tmp/project\x07",
        "\x1b]7;file://localhost/tmp/one\x07",
        "\x1b]7;not-file-uri\x07",
        "\x1b]7;file://localhost/tmp/one\x07",
        "\x1b]7;file://localhost/tmp/two\x1b\\",
    ));

    assert_eq!(actor.cwd, "/tmp/two");
    for expected_title in ["/tmp/one", "/tmp/two"] {
        let event = rx.try_recv().expect("cwd title event");
        assert_eq!(event.event, "session_title");
        assert_eq!(event.session_id, "sess-test");
        let payload: crate::types::SessionTitlePayload =
            serde_json::from_value(event.payload).expect("session title payload");
        assert_eq!(payload.title, expected_title);
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn update_cwd_and_emit_only_emits_when_cwd_changes() {
    let mut actor = test_actor();
    let mut rx = actor.event_tx.subscribe();

    actor.update_cwd_and_emit(" /tmp/project ".to_string());
    actor.update_cwd_and_emit("   ".to_string());
    assert!(rx.try_recv().is_err());

    actor.update_cwd_and_emit(" /tmp/other ".to_string());

    assert_eq!(actor.cwd, "/tmp/other");
    let event = rx.try_recv().expect("cwd title event");
    assert_eq!(event.event, "session_title");
    assert_eq!(event.session_id, "sess-test");
    let payload: crate::types::SessionTitlePayload =
        serde_json::from_value(event.payload).expect("session title payload");
    assert_eq!(payload.title, "/tmp/other");
}

#[test]
fn title_cwd_update_only_extracts_when_current_cwd_is_empty() {
    assert_eq!(
        title_cwd_update("", "user@host:/tmp/project"),
        Some("/tmp/project".to_string())
    );
    assert_eq!(
        title_cwd_update("/already/set", "user@host:/tmp/project"),
        None
    );
    assert_eq!(title_cwd_update("", "plain-title"), None);
}

#[test]
fn update_cwd_from_title_preserves_existing_cwd_and_fills_empty_cwd() {
    let mut actor = test_actor();

    actor.update_cwd_from_title("user@host:/tmp/ignored");
    assert_eq!(actor.cwd, "/tmp/project");

    actor.cwd.clear();
    actor.update_cwd_from_title("user@host:/tmp/from-title");
    assert_eq!(actor.cwd, "/tmp/from-title");
}

#[test]
fn title_tool_update_only_detects_when_tool_is_missing() {
    assert_eq!(
        title_tool_update(None, "codex - swimmers"),
        Some("Codex".to_string())
    );
    assert_eq!(title_tool_update(Some("Codex"), "claude"), None);
    assert_eq!(title_tool_update(None, "plain shell"), None);
}

#[test]
fn update_tool_from_title_sets_tool_mode_once_for_missing_tool() {
    let mut actor = test_actor();
    actor.tool = None;

    actor.update_tool_from_title("claude code");

    assert_eq!(actor.tool.as_deref(), Some("Claude Code"));
    actor.state_detector.note_input();
    assert_eq!(actor.state_detector.state(), SessionState::Busy);

    actor.update_tool_from_title("codex");
    assert_eq!(actor.tool.as_deref(), Some("Claude Code"));
}

#[tokio::test]
async fn maybe_check_liveness_throttled_by_interval() {
    let mut actor = test_actor();
    // last_liveness_check_at is set to Instant::now() by test_actor,
    // so the interval guard fires immediately and we never touch tmux.
    actor.maybe_check_liveness().await;
}

#[tokio::test]
async fn maybe_check_liveness_runs_query_when_interval_elapsed() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, previous_path) = install_fake_tmux("#!/bin/sh\nexit 1\n");
    let mut actor = test_actor();
    // Push last_liveness_check_at far enough back to pass the interval guard.
    actor.last_liveness_check_at = Instant::now() - Duration::from_millis(5_100); // past LIVENESS_CHECK_INTERVAL.
                                                                                  // tmux metadata lookup will fail for tmux_name "demo" (no real tmux),
                                                                                  // but the Err branch just logs — it must not panic.
    actor.maybe_check_liveness().await;
    // last_liveness_check_at is updated even on query failure
    assert!(actor.last_liveness_check_at.elapsed() < Duration::from_secs(1));
    restore_path(previous_path);
}

#[tokio::test]
async fn maybe_check_liveness_skips_stale_process_cache_that_would_mark_busy() {
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
        vec![proc(101, 1, 0.0), proc(102, 101, 0.0)],
        Instant::now() - PROCESS_ENTRIES_CACHE_TTL - Duration::from_millis(1),
    )
    .await;

    let mut actor = test_actor();
    actor.last_liveness_check_at = Instant::now() - Duration::from_secs(3);
    actor.maybe_check_liveness().await;

    assert_eq!(actor.state_detector.state(), SessionState::Idle);
    assert_eq!(actor.state_detector.state_evidence().cause, "initial_state");

    restore_path(previous_path);
    clear_process_entries_cache().await;
}

#[tokio::test]
async fn maybe_check_liveness_skips_stale_process_cache_that_would_mark_idle() {
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
        vec![proc(101, 1, 0.0)],
        Instant::now() - PROCESS_ENTRIES_CACHE_TTL - Duration::from_millis(1),
    )
    .await;

    let mut actor = test_actor();
    actor.state_detector.note_input();
    actor.last_liveness_check_at = Instant::now() - Duration::from_secs(3);
    actor.maybe_check_liveness().await;

    assert_eq!(actor.state_detector.state(), SessionState::Busy);
    assert_eq!(actor.state_detector.state_evidence().cause, "local_input");

    restore_path(previous_path);
    clear_process_entries_cache().await;
}

#[test]
fn extract_cwd_from_title_supports_absolute_home_and_host_prefixed_paths() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let previous_home = std::env::var_os("HOME");
    std::env::set_var("HOME", "/Users/tester");

    assert_eq!(
        extract_cwd_from_title("user@host:/tmp/project"),
        Some("/tmp/project".to_string())
    );
    assert_eq!(
        extract_cwd_from_title("user@host: /tmp/other"),
        Some("/tmp/other".to_string())
    );
    assert_eq!(
        extract_cwd_from_title("/var/tmp"),
        Some("/var/tmp".to_string())
    );
    assert_eq!(
        extract_cwd_from_title("~/repo"),
        Some("/Users/tester/repo".to_string())
    );
    assert_eq!(extract_cwd_from_title("plain-title"), None);

    if let Some(value) = previous_home {
        std::env::set_var("HOME", value);
    } else {
        std::env::remove_var("HOME");
    }
}

#[test]
fn extract_cwd_from_title_ignores_blank_host_prefixed_paths_and_plain_titles() {
    assert_eq!(extract_cwd_from_title("user@host:"), None);
    assert_eq!(extract_cwd_from_title("user@host: "), None);
    assert_eq!(extract_cwd_from_title("plain-title"), None);
    assert_eq!(extract_cwd_from_title("build finished: ok"), None);
}

#[test]
fn extract_cwd_from_title_preserves_home_when_home_is_absent() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let previous_home = std::env::var_os("HOME");
    std::env::remove_var("HOME");

    assert_eq!(extract_cwd_from_title("~/repo"), Some("~/repo".to_string()));
    assert_eq!(extract_cwd_from_title("~"), Some("~".to_string()));

    if let Some(value) = previous_home {
        std::env::set_var("HOME", value);
    }
}

#[test]
fn extract_cwd_from_title_ignores_invalid_prefix_shapes() {
    assert_eq!(extract_cwd_from_title("user@host:relative/path"), None);
    assert_eq!(extract_cwd_from_title("user@host: ./project"), None);
    assert_eq!(extract_cwd_from_title("user@host /tmp/project"), None);
}

#[test]
fn osc_payload_helpers_extract_bel_and_st_terminated_sequences() {
    let text = "\x1b]7;file://localhost/tmp/project\x1b\\ middle \x1b]2;codex\x07";
    assert_eq!(find_osc_payload_end("title\x07tail"), Some((5, 1)));
    assert_eq!(find_osc_payload_end("title\x1b\\tail"), Some((5, 2)));
    assert_eq!(
        find_osc_payload_end("title\x07before-st\x1b\\tail"),
        Some((5, 1))
    );
    assert_eq!(
        find_osc_payload_end("title\x1b\\before-bel\x07tail"),
        Some((5, 2))
    );
    assert_eq!(find_osc_payload_end("unterminated title"), None);
    assert_eq!(
        osc_payloads(text, "\x1b]7;"),
        vec!["file://localhost/tmp/project"]
    );
    assert_eq!(osc_payloads(text, "\x1b]2;"), vec!["codex"]);
    assert_eq!(
        cwd_from_osc7_payload("file://localhost/tmp/My%20Repo"),
        Some("/tmp/My Repo".to_string())
    );
    assert_eq!(
        cwd_from_osc7_payload("file://localhost/tmp/caf%C3%A9"),
        Some("/tmp/caf\u{e9}".to_string())
    );
}

#[test]
fn osc7_cwd_payload_preserves_non_local_host_identity() {
    let local_hosts = BTreeSet::new();

    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts(
            "file://devbox.example.com/srv/repos/swimmers",
            &local_hosts,
        ),
        Some("devbox.example.com:/srv/repos/swimmers".to_string())
    );
    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts("file://DevBox/srv/repos/swimmers", &local_hosts),
        Some("devbox:/srv/repos/swimmers".to_string())
    );
    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts("file://devbox", &local_hosts),
        None
    );
}

#[test]
fn osc7_cwd_payload_accepts_configured_local_host_aliases() {
    let local_hosts = BTreeSet::from(["workstation".to_string(), "macbook.local".to_string()]);

    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts("file://workstation/tmp/project", &local_hosts),
        Some("/tmp/project".to_string())
    );
    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts("file://macbook.local/tmp/project", &local_hosts),
        Some("/tmp/project".to_string())
    );
    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts("file:///tmp/project", &local_hosts),
        Some("/tmp/project".to_string())
    );
    assert_eq!(
        cwd_from_osc7_payload_with_local_hosts("file://remote/tmp/project", &local_hosts),
        Some("remote:/tmp/project".to_string())
    );
}

#[test]
fn startup_replay_clears_once_after_first_idle() {
    let mut actor = test_actor();
    actor.clear_replay_on_first_idle = true;
    actor.state_detector.note_input();
    actor.replay_ring.push(b"startup noise");

    assert!(!should_clear_startup_replay(
        true,
        actor.state_detector.state()
    ));
    assert_eq!(actor.state_detector.state(), SessionState::Busy);

    actor.clear_startup_replay_if_idle();
    assert!(actor.clear_replay_on_first_idle);
    assert_eq!(actor.replay_ring.snapshot(), "startup noise");

    actor.state_detector.process_output(b"\x1b]133;A\x07");
    actor.clear_startup_replay_if_idle();

    assert!(!actor.clear_replay_on_first_idle);
    assert_eq!(actor.replay_ring.snapshot(), "");

    actor.replay_ring.push(b"real output");
    actor.clear_startup_replay_if_idle();
    assert_eq!(actor.replay_ring.snapshot(), "real output");
}

#[test]
fn startup_replay_clear_predicate_requires_flag_and_idle_state() {
    assert!(should_clear_startup_replay(true, SessionState::Idle));
    assert!(!should_clear_startup_replay(false, SessionState::Idle));
    assert!(!should_clear_startup_replay(true, SessionState::Busy));
    assert!(!should_clear_startup_replay(true, SessionState::Exited));
}

#[test]
fn refresh_predicates_only_poll_when_needed() {
    let now = Instant::now();
    assert!(should_refresh_cwd_from_tmux(
        true,
        SessionState::Busy,
        now,
        now
    ));
    assert!(!should_refresh_cwd_from_tmux(
        false,
        SessionState::Busy,
        now - CWD_REFRESH_MIN_INTERVAL,
        now
    ));
    assert!(should_refresh_cwd_from_tmux(
        false,
        SessionState::Idle,
        now - CWD_REFRESH_MIN_INTERVAL,
        now
    ));

    assert!(should_refresh_tool_from_tmux(
        true,
        SessionState::Idle,
        Some("Codex"),
        now,
        now
    ));
    assert!(!should_refresh_tool_from_tmux(
        false,
        SessionState::Busy,
        None,
        now,
        now
    ));
    assert!(!should_refresh_tool_from_tmux(
        false,
        SessionState::Idle,
        Some("Codex"),
        now - TOOL_REFRESH_MIN_INTERVAL,
        now
    ));
    assert!(should_refresh_tool_from_tmux(
        false,
        SessionState::Busy,
        Some("Codex"),
        now - TOOL_REFRESH_MIN_INTERVAL,
        now
    ));
}

#[test]
fn tmux_tool_refresh_result_applies_only_detected_changes() {
    let mut actor = test_actor();

    actor.apply_tmux_tool_refresh_result("demo", Ok(None));
    assert_eq!(actor.tool.as_deref(), Some("Codex"));

    assert!(!tool_refresh_changes_tool(Some("Codex"), "Codex"));
    actor.apply_tmux_tool_refresh_result("demo", Ok(Some("Codex".to_string())));
    assert_eq!(actor.tool.as_deref(), Some("Codex"));

    assert!(tool_refresh_changes_tool(Some("Codex"), "Claude Code"));
    actor.apply_tmux_tool_refresh_result("demo", Ok(Some("Claude Code".to_string())));
    assert_eq!(actor.tool.as_deref(), Some("Claude Code"));

    actor.apply_tmux_tool_refresh_result("demo", Err(anyhow::anyhow!("tmux failed")));
    assert_eq!(actor.tool.as_deref(), Some("Claude Code"));
}

#[test]
fn actor_tool_refresh_predicate_uses_current_actor_state() {
    let mut actor = test_actor();
    let now = Instant::now();
    actor.last_tool_refresh_at = now - TOOL_REFRESH_MIN_INTERVAL;

    assert!(!actor.should_refresh_tool_from_tmux_at(false, now));

    actor.state_detector.note_input();
    assert!(actor.should_refresh_tool_from_tmux_at(false, now));
    assert!(actor.should_refresh_tool_from_tmux_at(true, now));
}

#[tokio::test]
async fn get_summary_uses_cached_metadata_without_tmux_refresh() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");

    let tmux = bin_dir.join("tmux");
    std::fs::write(&tmux, "#!/bin/sh\nsleep 2\nprintf 'codex\\n'\n").expect("tmux");
    let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmux, perms).expect("chmod");

    let previous_path = std::env::var_os("PATH");
    std::env::set_var(
        "PATH",
        std::env::join_paths([bin_dir.as_path()]).expect("path"),
    );

    let mut actor = test_actor();
    actor
        .state_detector
        .process_output(b"running build output\n");
    actor.last_tool_refresh_at = Instant::now() - TOOL_REFRESH_MIN_INTERVAL;

    let (tx, rx) = oneshot::channel();
    tokio::time::timeout(
        Duration::from_millis(200),
        actor.handle_command(SessionCommand::GetSummary(tx), false),
    )
    .await
    .expect("GetSummary should not block on tmux refresh");

    let summary = tokio::time::timeout(Duration::from_millis(200), rx)
        .await
        .expect("summary reply")
        .expect("summary payload");
    assert_eq!(summary.tool.as_deref(), Some("Codex"));
    assert_eq!(summary.cwd, "/tmp/project");

    if let Some(value) = previous_path {
        std::env::set_var("PATH", value);
    } else {
        std::env::remove_var("PATH");
    }
}

#[derive(Default)]
struct TrackingWriterState {
    writes: Vec<u8>,
    flushes: usize,
}

struct TrackingWriter {
    state: Arc<Mutex<TrackingWriterState>>,
}

impl Write for TrackingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.writes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.flushes += 1;
        Ok(())
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "writer failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn set_tracking_writer(actor: &mut SessionActor) -> Arc<Mutex<TrackingWriterState>> {
    let state = Arc::new(Mutex::new(TrackingWriterState::default()));
    actor.writer = Box::new(TrackingWriter {
        state: Arc::clone(&state),
    });
    state
}

#[test]
fn write_and_flush_input_flushes_pty_writer() {
    let state = Arc::new(Mutex::new(TrackingWriterState::default()));
    let mut writer: Box<dyn Write + Send> = Box::new(TrackingWriter {
        state: Arc::clone(&state),
    });

    write_and_flush_input(&mut writer, b"echo hi\r").expect("write and flush");

    let state = state.lock().unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(state.writes, b"echo hi\r");
    assert_eq!(state.flushes, 1);
}

#[tokio::test]
async fn handle_write_input_ignores_closed_pty_without_activity() {
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);
    let mut rx = actor.event_tx.subscribe();

    let result = actor.handle_write_input(b"hello\r".to_vec(), true).await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(!result.delivered);
    assert_eq!(result.method, "none");
    assert_eq!(
        result.message.as_deref(),
        Some("session process has exited")
    );
    assert!(writer_state.writes.is_empty());
    assert_eq!(writer_state.flushes, 0);
    assert_eq!(actor.state_detector.state(), SessionState::Idle);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn handle_write_input_uses_tmux_send_keys_without_raw_writer_when_available() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
    std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
    let (_dir, previous_path) = install_fake_tmux(
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
exit 0
"#,
    );
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);
    let mut rx = actor.event_tx.subscribe();

    let result = actor.handle_write_input(b"hello\r".to_vec(), false).await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(result.delivered);
    assert_eq!(result.method, "tmux_send_keys");
    assert_eq!(result.message, None);
    assert!(writer_state.writes.is_empty());
    assert_eq!(writer_state.flushes, 0);
    assert_eq!(actor.state_detector.state(), SessionState::Busy);
    assert_eq!(actor.state_detector.state_evidence().cause, "local_input");
    let event = rx.try_recv().expect("session_state event");
    let payload: SessionStatePayload =
        serde_json::from_value(event.payload).expect("session_state payload");
    assert_eq!(payload.state, SessionState::Busy);
    assert_eq!(payload.state_evidence.cause, "local_input");

    let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
    assert!(log.contains("send-keys -t =demo: -X cancel"));
    assert!(log.contains("send-keys -t =demo: -l -- hello"));
    assert!(log.contains("send-keys -t =demo: Enter"));

    std::env::remove_var("TMUX_SEND_LOG");
    restore_path(previous_path);
}

#[tokio::test]
async fn handle_write_input_sends_leading_dash_literal_after_end_of_options() {
    // Regression: a literal beginning with `-` (e.g. a user typing `-N5`,
    // `-rf`, `--flag`) must be typed verbatim, not parsed as a send-keys flag.
    // tmux getopt does not stop after `-l`, so the `--` separator is required;
    // without it `-N5` exits 0 having typed nothing.
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
    std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
    let (_dir, previous_path) = install_fake_tmux(
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
exit 0
"#,
    );
    let mut actor = test_actor();
    let _writer_state = set_tracking_writer(&mut actor);

    let result = actor.handle_write_input(b"-N5\r".to_vec(), false).await;

    assert!(result.delivered);
    assert_eq!(result.method, "tmux_send_keys");
    let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
    assert!(
        log.contains("send-keys -t =demo: -l -- -N5"),
        "leading-dash literal must be sent after `--`; log was:\n{log}"
    );

    std::env::remove_var("TMUX_SEND_LOG");
    restore_path(previous_path);
}

#[tokio::test]
async fn handle_write_input_falls_back_to_raw_writer_when_tmux_send_keys_fails() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, previous_path) = install_fake_tmux(
        r#"#!/bin/sh
printf 'no such target\n' >&2
exit 1
"#,
    );
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);

    let result = actor.handle_write_input(b"hello\r".to_vec(), false).await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(result.delivered);
    assert_eq!(result.method, "pty_write");
    assert_eq!(result.message, None);
    assert_eq!(writer_state.writes, b"hello\r");
    assert_eq!(writer_state.flushes, 1);
    assert_eq!(actor.state_detector.state(), SessionState::Busy);
    assert_eq!(actor.state_detector.state_evidence().cause, "local_input");

    restore_path(previous_path);
}

#[tokio::test]
async fn handle_write_input_does_not_replay_raw_buffer_after_partial_tmux_delivery() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
    std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
    let (_dir, previous_path) = install_fake_tmux(
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
case "$*" in
  *" Enter") exit 1 ;;
  *) exit 0 ;;
esac
"#,
    );
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);

    let result = actor.handle_write_input(b"hello\r".to_vec(), false).await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(result.delivered);
    assert_eq!(result.method, "tmux_send_keys_partial");
    // Partial delivery surfaces a warning so a 200/ok isn't mistaken for a
    // complete submit even though `delivered` stays true.
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("partially delivered")),
        "partial delivery must surface a warning message"
    );
    assert!(writer_state.writes.is_empty());
    assert_eq!(writer_state.flushes, 0);

    let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
    assert!(log.contains("send-keys -t =demo: -l -- hello"));
    assert!(log.contains("send-keys -t =demo: Enter"));

    std::env::remove_var("TMUX_SEND_LOG");
    restore_path(previous_path);
}

#[tokio::test]
async fn handle_write_input_preserves_control_byte_fallback_payloads() {
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);

    let result = actor.handle_write_input(b"abc\t".to_vec(), false).await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(result.delivered);
    assert_eq!(result.method, "pty_write");
    assert_eq!(result.message, None);
    assert_eq!(writer_state.writes, b"abc\t");
    assert_eq!(writer_state.flushes, 1);
    assert_eq!(actor.state_detector.state(), SessionState::Busy);
}

#[tokio::test]
async fn handle_write_input_reports_raw_writer_errors_as_pty_write() {
    let mut actor = test_actor();
    actor.writer = Box::new(FailingWriter);

    let result = actor.handle_write_input(b"abc\t".to_vec(), false).await;

    assert!(!result.delivered);
    assert_eq!(result.method, "pty_write");
    assert_eq!(result.message.as_deref(), Some("writer failed"));
    assert_eq!(actor.state_detector.state(), SessionState::Busy);
}

#[tokio::test]
async fn handle_submit_line_uses_tmux_paste_buffer_and_double_enter() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let tmux_log = tempfile::NamedTempFile::new().expect("tmux log");
    std::env::set_var("TMUX_SEND_LOG", tmux_log.path());
    let (_dir, previous_path) = install_fake_tmux(
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$TMUX_SEND_LOG"
exit 0
"#,
    );
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);
    let mut rx = actor.event_tx.subscribe();

    actor
        .handle_submit_line("hello codex\n".to_string(), false)
        .await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(writer_state.writes.is_empty());
    assert_eq!(writer_state.flushes, 0);
    assert_eq!(actor.state_detector.state(), SessionState::Busy);
    let event = rx.try_recv().expect("session_state event");
    let payload: SessionStatePayload =
        serde_json::from_value(event.payload).expect("session_state payload");
    assert_eq!(payload.state, SessionState::Busy);

    let log = std::fs::read_to_string(tmux_log.path()).expect("tmux log");
    assert!(log.contains("send-keys -t =demo: -X cancel"));
    assert!(log.contains("set-buffer -b swimmers-submit-"));
    assert!(log.contains("-- hello codex"));
    assert!(log.contains("paste-buffer -dpr -b swimmers-submit-"));
    assert_eq!(
        log.lines()
            .filter(|line| *line == "send-keys -t =demo: Enter")
            .count(),
        2
    );

    std::env::remove_var("TMUX_SEND_LOG");
    restore_path(previous_path);
}

#[tokio::test]
async fn handle_submit_line_falls_back_to_raw_writer_when_tmux_submit_fails() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let (_dir, previous_path) = install_fake_tmux(
        r#"#!/bin/sh
printf 'no such target\n' >&2
exit 1
"#,
    );
    let mut actor = test_actor();
    let writer_state = set_tracking_writer(&mut actor);

    actor
        .handle_submit_line("hello codex".to_string(), false)
        .await;

    let writer_state = writer_state
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert_eq!(writer_state.writes, b"hello codex\r\r");
    assert_eq!(writer_state.flushes, 1);
    assert_eq!(actor.state_detector.state(), SessionState::Busy);

    restore_path(previous_path);
}

#[test]
fn tmux_input_chunks_splits_literal_text_and_enter() {
    assert_eq!(
        tmux_input_chunks(b"printf \"hello\\n\"\r"),
        Some(vec![
            TmuxInputChunk::Literal("printf \"hello\\n\"".to_string()),
            TmuxInputChunk::Enter,
        ])
    );
}

#[test]
fn tmux_input_chunks_rejects_control_sequences() {
    assert_eq!(tmux_input_chunks(b"\x1b[A"), None);
    assert_eq!(tmux_input_chunks(b"abc\t"), None);
}

#[test]
fn normalize_submit_line_text_trims_trailing_newlines_only() {
    assert_eq!(
        normalize_submit_line_text("  hello codex  \n\n"),
        Some("  hello codex  ".to_string())
    );
    assert_eq!(normalize_submit_line_text("\r\n"), None);
}

#[test]
fn submit_line_fallback_input_adds_double_enter() {
    assert_eq!(submit_line_fallback_input("hello"), b"hello\r\r");
}

#[test]
fn resolve_tmux_terminal_env_uses_fallback_for_missing_or_dumb_term() {
    let (term, colorterm, fallback) = resolve_tmux_terminal_env(None, None);
    assert_eq!(term, "xterm-256color");
    assert_eq!(colorterm, "truecolor");
    assert!(fallback);

    let (term, colorterm, fallback) = resolve_tmux_terminal_env(Some("  dumb  "), Some(" 24bit "));
    assert_eq!(term, "xterm-256color");
    assert_eq!(colorterm, "24bit");
    assert!(fallback);
}

#[test]
fn resolve_tmux_terminal_env_preserves_valid_term() {
    let (term, colorterm, fallback) =
        resolve_tmux_terminal_env(Some("  screen-256color  "), Some("truecolor"));
    assert_eq!(term, "screen-256color");
    assert_eq!(colorterm, "truecolor");
    assert!(!fallback);
}

#[test]
fn resolve_tmux_term_falls_back_for_unknown_and_blank_values() {
    for inherited_term in [Some("unknown"), Some("  UNKNOWN  "), Some("   ")] {
        let (term, fallback) = resolve_tmux_term(inherited_term);
        assert_eq!(term, "xterm-256color");
        assert!(fallback);
    }
}

#[test]
fn resolve_tmux_colorterm_trims_or_uses_default() {
    assert_eq!(resolve_tmux_colorterm(Some("  truecolor  ")), "truecolor");

    for inherited_colorterm in [None, Some(""), Some("   ")] {
        assert_eq!(resolve_tmux_colorterm(inherited_colorterm), "truecolor");
    }
}

#[test]
fn replay_ring_snapshot_preserves_recent_output() {
    let mut ring = ReplayRing::new(512 * 1024);
    ring.push(b"$ hello world\n");
    ring.push(b"output line 2\n");
    let snapshot_text = ring.snapshot();
    assert_eq!(snapshot_text, "$ hello world\noutput line 2\n");
    assert!(ring.latest_seq() > 0);
}

#[tokio::test]
async fn query_tmux_session_created_reads_epoch_from_tmux() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let tmux = bin_dir.join("tmux");
    std::fs::write(
        &tmux,
        "#!/bin/sh\nprintf '/tmp/project\\037bash\\037101\\0371774274168\\n'\n",
    )
    .expect("tmux");
    let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmux, perms).expect("chmod");

    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let previous_path = std::env::var_os("PATH");
    std::env::set_var(
        "PATH",
        std::env::join_paths([bin_dir.as_path()]).expect("path"),
    );

    let created_at = query_tmux_session_created("demo", &crate::tmux_target::TmuxTarget::Default)
        .await
        .expect("session_created query");
    assert_eq!(
        created_at,
        Utc.timestamp_opt(1_774_274_168, 0).single().unwrap()
    );

    if let Some(value) = previous_path {
        std::env::set_var("PATH", value);
    } else {
        std::env::remove_var("PATH");
    }
}

#[tokio::test]
async fn capture_pane_tail_uses_exact_session_target_for_numeric_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let target_file = dir.path().join("target.txt");
    let tmux = bin_dir.join("tmux");
    std::fs::write(
        &tmux,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"${{5-}}\" > \"{}\"\nprintf 'captured\\n'\n",
            target_file.display()
        ),
    )
    .expect("tmux");
    let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmux, perms).expect("chmod");

    let captured =
        capture_pane_tail_with_command(&tmux, "0", &crate::tmux_target::TmuxTarget::Default, 20)
            .await
            .expect("capture pane");
    assert_eq!(captured.trim(), "captured");
    assert_eq!(
        std::fs::read_to_string(&target_file).expect("target file"),
        "=0:\n"
    );
}

#[tokio::test]
async fn bounded_tmux_command_scrubs_nested_tmux_env_vars() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let tmux = dir.path().join("tmux");
    std::fs::write(
        &tmux,
        "#!/bin/sh\nprintf 'TMUX=%s\\nTMUX_PANE=%s\\n' \"${TMUX-unset}\" \"${TMUX_PANE-unset}\"\n",
    )
    .expect("tmux");
    let mut perms = std::fs::metadata(&tmux).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmux, perms).expect("chmod");

    let previous_tmux = std::env::var_os("TMUX");
    let previous_tmux_pane = std::env::var_os("TMUX_PANE");
    std::env::set_var("TMUX", "/tmp/tmux,123,0");
    std::env::set_var("TMUX_PANE", "%1");

    let output = run_bounded_tmux_command(
        tmux.as_os_str(),
        &["display-message"],
        Duration::from_secs(2),
        "test-env-scrub",
    )
    .await;

    match previous_tmux {
        Some(value) => std::env::set_var("TMUX", value),
        None => std::env::remove_var("TMUX"),
    }
    match previous_tmux_pane {
        Some(value) => std::env::set_var("TMUX_PANE", value),
        None => std::env::remove_var("TMUX_PANE"),
    }

    let output = output.expect("tmux env probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "TMUX=unset\nTMUX_PANE=unset\n"
    );
}

#[test]
fn standalone_focus_reports_do_not_count_as_activity_input() {
    assert!(!write_input_counts_as_activity(b"\x1b[I"));
    assert!(!write_input_counts_as_activity(b"\x1b[O"));
    assert!(!write_input_counts_as_activity(b"\x1b[I\x1b[O\x1b[I"));
}

#[test]
fn mixed_focus_reports_and_real_input_still_count_as_activity() {
    assert!(write_input_counts_as_activity(b"\x1b[Ia"));
    assert!(write_input_counts_as_activity(b"\x1b[O\r"));
    assert!(write_input_counts_as_activity(b"\t"));
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
