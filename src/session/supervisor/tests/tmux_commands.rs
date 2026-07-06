use super::*;
use crate::session::actor::{
    reset_tmux_health_for_tests, run_bounded_tmux_command_for_target,
    run_bounded_tmux_probe_for_target,
};
use crate::tmux_target::TmuxTarget;

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
    let err = run_bounded_tmux_command_for_target(
        fake_tmux.as_os_str(),
        &TmuxTarget::socket_name("bounded-timeout"),
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

#[tokio::test]
async fn tmux_probe_circuit_breaker_skips_probes_until_essential_success() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_tmux_health_for_tests();
    let dir = tempdir().expect("tempdir");
    let sleepy_tmux = dir.path().join("sleepy-tmux");
    let fast_tmux = dir.path().join("fast-tmux");
    let log = dir.path().join("tmux.log");
    let target = TmuxTarget::socket_name("breaker");
    write_executable(
        &sleepy_tmux,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ -x /bin/sleep ]; then exec /bin/sleep 10; fi\nexec sleep 10\n",
            log.display()
        ),
    );
    write_executable(
        &fast_tmux,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf 'ok\\n'\n",
            log.display()
        ),
    );

    let timeout = run_bounded_tmux_probe_for_target(
        sleepy_tmux.as_os_str(),
        &target,
        &["display-message"],
        Duration::from_millis(100),
        "display-message",
    )
    .await
    .expect_err("first probe should time out");
    assert!(timeout.to_string().contains("timed out"));

    let skipped = run_bounded_tmux_probe_for_target(
        fast_tmux.as_os_str(),
        &target,
        &["capture-pane"],
        Duration::from_secs(1),
        "capture-pane",
    )
    .await
    .expect_err("second nonessential probe should be skipped");
    assert!(skipped.to_string().contains("skipped"));
    assert_eq!(
        std::fs::read_to_string(&log).expect("tmux log"),
        "-L breaker display-message\n"
    );

    run_bounded_tmux_command_for_target(
        fast_tmux.as_os_str(),
        &target,
        &["send-keys"],
        Duration::from_secs(1),
        "send-keys",
    )
    .await
    .expect("essential success should recover breaker");

    run_bounded_tmux_probe_for_target(
        fast_tmux.as_os_str(),
        &target,
        &["capture-pane"],
        Duration::from_secs(1),
        "capture-pane",
    )
    .await
    .expect("probe should run after recovery");

    assert_eq!(
        std::fs::read_to_string(&log).expect("tmux log"),
        "-L breaker display-message\n-L breaker send-keys\n-L breaker capture-pane\n"
    );
    reset_tmux_health_for_tests();
}

#[tokio::test]
async fn tmux_probe_circuit_breaker_is_scoped_by_target() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_tmux_health_for_tests();
    let dir = tempdir().expect("tempdir");
    let sleepy_tmux = dir.path().join("sleepy-tmux");
    let fast_tmux = dir.path().join("fast-tmux");
    let log = dir.path().join("tmux.log");
    let isolated = TmuxTarget::socket_name("tiktok");
    let baseline = TmuxTarget::socket_name("baseline");
    write_executable(
        &sleepy_tmux,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ -x /bin/sleep ]; then exec /bin/sleep 10; fi\nexec sleep 10\n",
            log.display()
        ),
    );
    write_executable(
        &fast_tmux,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf 'ok\\n'\n",
            log.display()
        ),
    );

    run_bounded_tmux_probe_for_target(
        sleepy_tmux.as_os_str(),
        &isolated,
        &["display-message"],
        Duration::from_millis(100),
        "display-message",
    )
    .await
    .expect_err("isolated target probe should time out");

    run_bounded_tmux_probe_for_target(
        fast_tmux.as_os_str(),
        &baseline,
        &["capture-pane"],
        Duration::from_secs(1),
        "capture-pane",
    )
    .await
    .expect("default target should not inherit isolated target cooldown");

    assert_eq!(
        std::fs::read_to_string(&log).expect("tmux log"),
        "-L tiktok display-message\n-L baseline capture-pane\n"
    );
    reset_tmux_health_for_tests();
}
