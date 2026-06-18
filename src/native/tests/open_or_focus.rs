use super::*;

#[test]
fn detects_transient_iterm_open_errors() {
    let retryable = anyhow!(
        "/tmp/iterm-focus.scpt: execution error: Can't get session 1 of missing value. (-1728)"
    );
    assert!(is_transient_iterm_open_error(&retryable));

    let tab_creation_race = anyhow!(
            "/tmp/iterm-focus.scpt: execution error: unable to resolve iTerm session after tab creation (-2700)"
        );
    assert!(is_transient_iterm_open_error(&tab_creation_race));

    let other = anyhow!("unexpected osascript status: created");
    assert!(!is_transient_iterm_open_error(&other));
}

#[tokio::test]
async fn run_osascript_output_kills_child_on_timeout() {
    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_osascript = fake_bin_dir.join("osascript");
    let pid_path = temp.path().join("osascript.pid");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$$\" > \"{pid}\"\n/bin/sleep 2\nprintf 'created|pane-timeout\\n'\n",
                pid = pid_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let mut command = Command::new(&fake_osascript);
    let err = run_osascript_output_with_timeout(
        &mut command,
        "opening/focusing iTerm session",
        Duration::from_secs(1),
    )
    .await
    .expect_err("sleeping osascript should time out");

    assert!(
        err.chain().any(|cause| cause.is::<NativeScriptError>()),
        "typed native error missing in chain: {err:#}"
    );
    assert!(
        format!("{err:#}").contains("osascript timed out"),
        "timeout message missing: {err:#}"
    );

    let mut pid = None;
    for _ in 0..20 {
        if let Ok(raw_pid) = std::fs::read_to_string(&pid_path) {
            let trimmed = raw_pid.trim();
            if !trimmed.is_empty() {
                pid = Some(trimmed.to_string());
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let pid = pid.expect("fake osascript should persist its pid");
    let probe = ProcessCommand::new("/bin/kill")
        .args(["-0", pid.as_str()])
        .status()
        .expect("kill -0 should execute");
    assert!(
        !probe.success(),
        "timed-out osascript child should be terminated"
    );
}

#[tokio::test]
async fn open_or_focus_passes_cached_pane_id_on_repeat_calls() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    remember_pane_id("sess-cache", None);

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%12\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let fake_osascript = fake_bin_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\nknown=\"${{6-}}\"\nif [ -z \"$known\" ]; then\n  printf 'created|pane-1\\n'\nelse\n  printf 'focused|%s\\n' \"$known\"\nfi\n",
                log = log_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_bin_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    let first = open_or_focus_iterm_session("sess-cache", "tmux-cache", "/tmp/fallback")
        .await
        .unwrap();
    let second = open_or_focus_iterm_session("sess-cache", "tmux-cache", "/tmp/fallback")
        .await
        .unwrap();

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(first.status, "created");
    assert_eq!(first.pane_id.as_deref(), Some("pane-1"));
    assert_eq!(second.status, "focused");
    assert_eq!(second.pane_id.as_deref(), Some("pane-1"));

    let log = std::fs::read_to_string(&log_path).unwrap();
    let mut lines = log.lines();
    let first_call: Vec<_> = lines.next().unwrap().split('\t').collect();
    let second_call: Vec<_> = lines.next().unwrap().split('\t').collect();
    assert_eq!(first_call[1], "sess-cache");
    assert_eq!(first_call[2], "tmux-cache");
    assert_eq!(
        first_call[3],
        format!(
            "exec {} attach-session -t '=tmux-cache'",
            fake_tmux.display()
        )
    );
    assert_eq!(first_call[4], "12 swimmers");
    assert_eq!(first_call.len(), 5);
    assert_eq!(second_call[4], "12 swimmers");
    assert_eq!(second_call[5], "pane-1");

    remember_pane_id("sess-cache", None);
}

#[tokio::test]
async fn open_or_focus_retries_transient_missing_session_error() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    remember_pane_id("sess-retry", None);

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(&fake_tmux, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let counter_path = temp.path().join("osascript-count");
    let fake_osascript = fake_bin_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nif [ \"$count\" -eq 1 ]; then\n  printf \"%s\\n\" \"scripts/iterm-focus.scpt: execution error: Can't get session 1 of missing value. (-1728)\" >&2\n  exit 1\nfi\nprintf 'created|pane-retry\\n'\n",
                counter = counter_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_bin_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    let result = open_or_focus_iterm_session("sess-retry", "tmux-retry", "/Users/b/repos/retry")
        .await
        .unwrap();

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(result.status, "created");
    assert_eq!(result.pane_id.as_deref(), Some("pane-retry"));
    assert_eq!(std::fs::read_to_string(&counter_path).unwrap().trim(), "2");

    remember_pane_id("sess-retry", None);
}

#[tokio::test]
async fn open_or_focus_retries_transient_tab_creation_resolution_error() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    remember_pane_id("sess-race", None);

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(&fake_tmux, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let counter_path = temp.path().join("osascript-count");
    let fake_osascript = fake_bin_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nif [ \"$count\" -eq 1 ]; then\n  printf \"%s\\n\" \"scripts/iterm-focus.scpt: execution error: unable to resolve iTerm session after tab creation (-2700)\" >&2\n  exit 1\nfi\nprintf 'created|pane-race\\n'\n",
                counter = counter_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_bin_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    let result = open_or_focus_iterm_session("sess-race", "tmux-race", "/Users/b/repos/race")
        .await
        .unwrap();

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(result.status, "created");
    assert_eq!(result.pane_id.as_deref(), Some("pane-race"));
    assert_eq!(std::fs::read_to_string(&counter_path).unwrap().trim(), "2");

    remember_pane_id("sess-race", None);
}

#[tokio::test]
async fn open_or_focus_ghostty_runs_script_with_expected_args() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("tmux builds");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let fake_osascript_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_osascript_dir).unwrap();
    let fake_osascript = fake_osascript_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      printf 'ghostty-tab-main\\n'\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\nprintf 'created|ghost-pane\\n'\n",
                log = log_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_osascript_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    let result = open_or_focus_ghostty_session(
        "sess-ghostty",
        "tmux-ghostty",
        "/Users/b/repos/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(result.status, "created");
    assert_eq!(result.pane_id.as_deref(), Some("ghost-pane"));

    let log = std::fs::read_to_string(&log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(call[1], "sess-ghostty");
    assert_eq!(call[2], "tmux-ghostty");
    assert_eq!(call[3], "/Users/b/repos/swimmers");
    assert_eq!(
        call[4],
        format!(
            "exec '{}' attach-session -t '=tmux-ghostty'",
            fake_tmux.display()
        )
    );
    assert_eq!(call[5], "swimmers");
    assert_eq!(call[6], GHOSTTY_MANAGED_TITLE_PREFIX);
    assert_eq!(call[7], GhosttyOpenMode::Swap.label());

    clear_ghostty_preview_term_cache();
}

#[tokio::test]
async fn open_or_focus_ghostty_swap_passes_cached_preview_id_on_repeat_calls() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("tmux builds");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let counter_path = temp.path().join("osascript-count");
    let fake_osascript_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_osascript_dir).unwrap();
    let fake_osascript = fake_osascript_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      printf 'ghostty-tab-main\\n'\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nprintf 'created|ghost-pane-%s\\n' \"$count\"\n",
                log = log_path.display(),
                counter = counter_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_osascript_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    let first = open_or_focus_ghostty_session(
        "sess-ghostty-a",
        "tmux-ghostty-a",
        "/Users/b/repos/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();
    let second = open_or_focus_ghostty_session(
        "sess-ghostty-b",
        "tmux-ghostty-b",
        "/Users/b/repos/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(first.pane_id.as_deref(), Some("ghost-pane-1"));
    assert_eq!(second.pane_id.as_deref(), Some("ghost-pane-2"));

    let log = std::fs::read_to_string(&log_path).unwrap();
    let mut lines = log.lines();
    let first_call: Vec<_> = lines.next().unwrap().split('\t').collect();
    let second_call: Vec<_> = lines.next().unwrap().split('\t').collect();
    assert_eq!(first_call[7], GhosttyOpenMode::Swap.label());
    assert_eq!(first_call.len(), 8);
    assert_eq!(second_call[7], GhosttyOpenMode::Swap.label());
    assert_eq!(second_call[8], "ghost-pane-1");

    clear_ghostty_preview_term_cache();
}

#[tokio::test]
async fn open_or_focus_ghostty_swap_scopes_cached_preview_id_to_active_tab() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("tmux builds");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let open_counter_path = temp.path().join("osascript-open-count");
    let tab_counter_path = temp.path().join("osascript-tab-count");
    let fake_osascript_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_osascript_dir).unwrap();
    let fake_osascript = fake_osascript_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      count=0\n      if [ -f \"{tab_counter}\" ]; then\n        IFS= read -r count < \"{tab_counter}\" || true\n      fi\n      count=$((count + 1))\n      printf '%s\\n' \"$count\" > \"{tab_counter}\"\n      case \"$count\" in\n        1|2) printf 'ghostty-tab-a\\n' ;;\n        3|4) printf 'ghostty-tab-b\\n' ;;\n        *) printf 'ghostty-tab-a\\n' ;;\n      esac\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{open_counter}\" ]; then\n  IFS= read -r count < \"{open_counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{open_counter}\"\nprintf 'created|ghost-pane-%s\\n' \"$count\"\n",
                log = log_path.display(),
                open_counter = open_counter_path.display(),
                tab_counter = tab_counter_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_osascript_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    open_or_focus_ghostty_session(
        "sess-ghostty-a1",
        "tmux-ghostty-a1",
        "/Users/b/repos/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();
    open_or_focus_ghostty_session(
        "sess-ghostty-b1",
        "tmux-ghostty-b1",
        "/Users/b/repos/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();
    open_or_focus_ghostty_session(
        "sess-ghostty-a2",
        "tmux-ghostty-a2",
        "/Users/b/repos/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    let log = std::fs::read_to_string(&log_path).unwrap();
    let calls: Vec<Vec<_>> = log.lines().map(|line| line.split('\t').collect()).collect();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].len(), 8);
    assert_eq!(calls[1].len(), 8);
    assert_eq!(calls[2][8], "ghost-pane-1");

    clear_ghostty_preview_term_cache();
}

#[tokio::test]
async fn open_or_focus_ghostty_rejects_older_versions_before_running_script() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();

    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();

    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(&fake_tmux, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let fake_osascript = fake_bin_dir.join("osascript");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.2.3\\n'\n      ;;\n    *)\n      printf 'ghostty-tab-main\\n'\n      ;;\n  esac\n  exit 0\nfi\nprintf '%s\\n' \"$*\" >> \"{log}\"\nprintf 'created|ghost-pane\\n'\n",
                log = log_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    let path_value = std::env::join_paths([fake_bin_dir.as_path()]).unwrap();
    std::env::set_var("PATH", path_value);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

    let error = open_or_focus_ghostty_session(
        "sess-ghostty",
        "tmux-ghostty",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .expect_err("older Ghostty should be rejected before running the script");

    match original_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert!(error.to_string().contains("Ghostty 1.2.3 is installed"));
    assert!(!log_path.exists());
    clear_ghostty_preview_term_cache();
}

#[test]
fn ghostty_preview_cache_is_invariant_under_other_tabs() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let mut runner = proptest::test_runner::TestRunner::default();
    let strategy = proptest::collection::vec((0u8..3, 0u16..256), 0..40);

    runner
        .run(&strategy, |ops| {
            clear_ghostty_preview_term_cache();

            for (tab, pane) in &ops {
                let tab_id = format!("tab-{tab}");
                let pane_id = format!("pane-{pane}");
                remember_ghostty_preview_term_id(Some(tab_id.as_str()), Some(pane_id.as_str()));
            }
            let interleaved = cached_ghostty_preview_term_id(Some("tab-0"));

            clear_ghostty_preview_term_cache();
            for (_, pane) in ops.iter().filter(|(tab, _)| *tab == 0) {
                let pane_id = format!("pane-{pane}");
                remember_ghostty_preview_term_id(Some("tab-0"), Some(pane_id.as_str()));
            }
            let projected = cached_ghostty_preview_term_id(Some("tab-0"));

            clear_ghostty_preview_term_cache();
            prop_assert_eq!(interleaved, projected);
            Ok(())
        })
        .unwrap();
}
