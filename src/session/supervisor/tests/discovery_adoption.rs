use super::*;

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
        .adopt_tmux_session("alpha".to_string(), None, None)
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
        .adopt_tmux_session("alpha".to_string(), None, None)
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
            .adopt_tmux_session(String::new(), None, None)
            .await
            .expect_err("empty tmux target should still be rejected"),
        TmuxAdoptError::EmptyTmuxName
    );

    std::fs::write(&sessions_file, "  padded  \n").expect("sessions");
    let stale = supervisor.build_placeholder_summary("sess_7", "  padded  ");
    supervisor.stale_sessions.write().await.push(stale);

    let adopted = supervisor
        .adopt_tmux_session("  padded  ".to_string(), None, Some("sess_7".to_string()))
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
        .adopt_tmux_session("padded".to_string(), None, None)
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
            .adopt_tmux_session("beta".to_string(), None, None)
            .await
            .expect_err("missing tmux target should be rejected"),
        TmuxAdoptError::TargetNotFound {
            tmux_name: "beta".to_string()
        }
    );

    std::fs::write(&sessions_file, "alpha\nalpha\n").expect("duplicate sessions");
    assert_eq!(
        supervisor
            .adopt_tmux_session("alpha".to_string(), None, None)
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
            .adopt_tmux_session("alpha".to_string(), None, Some("sess_7".to_string()))
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
