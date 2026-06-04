use super::*;
use proptest::prelude::*;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

#[test]
fn sanitize_osascript_text_arg_strips_corrupting_bytes_and_preserves_rest() {
    let hostile = "name\0\n\r\t|with'\"\\chars; rm -rf /";
    let cleaned = sanitize_osascript_text_arg(hostile);

    // The five stripped bytes must be gone.
    assert!(!cleaned.contains('\0'));
    assert!(!cleaned.contains('\n'));
    assert!(!cleaned.contains('\r'));
    assert!(!cleaned.contains('\t'));
    assert!(!cleaned.contains('|'));

    // Quotes, backslashes, and shell metacharacters survive — they are
    // not in the strip set, and there is no shell sink anyway.
    assert!(cleaned.contains('\''));
    assert!(cleaned.contains('"'));
    assert!(cleaned.contains('\\'));
    assert!(cleaned.contains("; rm -rf /"));

    // Safe inputs round-trip byte-identical.
    let safe = "swimmers-session-1";
    assert_eq!(sanitize_osascript_text_arg(safe), safe);

    // Parser-desync regression: a hostile value cannot inject extra `|`
    // separators into a synthetic `parse_osascript_output`-style line.
    let line = format!(
        "created|{}|{}",
        sanitize_osascript_text_arg(hostile),
        "trailing"
    );
    assert_eq!(line.matches('|').count(), 2);
}

#[test]
fn validate_osascript_script_arg_rejects_subshell_payload() {
    let err = validate_osascript_script_arg("tmux_name", "$(rm -rf /)").unwrap_err();
    let typed = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<NativeScriptError>())
        .expect("typed native error");
    assert!(matches!(
        typed,
        NativeScriptError::InvalidOsaScriptArg { .. }
    ));
}

#[test]
fn validate_osascript_script_arg_rejects_quoted_payload() {
    let err = validate_osascript_script_arg("attach_command", "\"malicious\"").unwrap_err();
    let typed = err
        .downcast_ref::<NativeScriptError>()
        .expect("typed native error");
    assert!(matches!(
        typed,
        NativeScriptError::InvalidOsaScriptArg { .. }
    ));
}

#[test]
fn validate_osascript_script_arg_allows_shell_quoted_attach_command() {
    validate_osascript_script_arg(
        "attach_command",
        "exec '/tmp/tmux builds/tmux' attach-session -t '=team session'",
    )
    .expect("single-quoted shell words should be accepted for attach_command");
}

#[test]
fn validate_osascript_script_arg_rejects_newline_payload() {
    let err = validate_osascript_script_arg("tmux_name", "line1\nline2").unwrap_err();
    let typed = err
        .downcast_ref::<NativeScriptError>()
        .expect("typed native error");
    assert!(matches!(
        typed,
        NativeScriptError::InvalidOsaScriptArg { .. }
    ));
}

#[test]
fn validate_osascript_script_arg_rejects_empty_payloads() {
    for (field, value) in [
        ("tmux_name", ""),
        ("tmux_name", "   "),
        ("attach_command", ""),
        ("attach_command", "\t"),
    ] {
        let err = validate_osascript_script_arg(field, value).unwrap_err();
        let typed = err
            .downcast_ref::<NativeScriptError>()
            .expect("typed native error");
        assert!(
            matches!(typed, NativeScriptError::InvalidOsaScriptArg { .. }),
            "{field}={value:?} should be rejected as an invalid osascript argument"
        );
        assert!(err.to_string().contains("value cannot be empty"));
    }
}

#[test]
fn native_app_env_defaults_to_iterm() {
    assert_eq!(
        NativeDesktopApp::from_env_value(""),
        NativeDesktopApp::Iterm
    );
    assert_eq!(
        NativeDesktopApp::from_env_value("ghostty"),
        NativeDesktopApp::Ghostty
    );
    assert_eq!(
        NativeDesktopApp::from_env_value("something-else"),
        NativeDesktopApp::Iterm
    );
}

#[test]
fn ghostty_mode_env_defaults_to_swap() {
    assert_eq!(GhosttyOpenMode::from_env_value(""), GhosttyOpenMode::Swap);
    assert_eq!(
        GhosttyOpenMode::from_env_value("swap"),
        GhosttyOpenMode::Swap
    );
    assert_eq!(GhosttyOpenMode::from_env_value("add"), GhosttyOpenMode::Add);
    assert_eq!(
        GhosttyOpenMode::from_env_value("split"),
        GhosttyOpenMode::Add
    );
    assert_eq!(
        GhosttyOpenMode::from_env_value("window"),
        GhosttyOpenMode::Window
    );
}

#[test]
fn parse_version_triplet_handles_patchless_and_suffixed_versions() {
    assert_eq!(parse_version_triplet("1.2.3"), Some([1, 2, 3]));
    assert_eq!(parse_version_triplet("1.3"), Some([1, 3, 0]));
    assert_eq!(
        parse_version_triplet("Ghostty 1.3.0-dev.2"),
        Some([1, 3, 0])
    );
    assert_eq!(parse_version_triplet(""), None);
}

#[test]
fn ghostty_version_requirement_error_enforces_documented_minimum() {
    assert!(ghostty_version_requirement_error("1.3.0").is_none());
    assert!(ghostty_version_requirement_error("1.3.1").is_none());

    let message =
        ghostty_version_requirement_error("1.2.3").expect("older Ghostty should be rejected");
    assert!(message.contains("Ghostty 1.2.3 is installed"));
    assert!(message.contains("1.3.0+"));

    let invalid = ghostty_version_requirement_error("beta").expect("invalid versions fail closed");
    assert!(invalid.contains("Ghostty reported version"));
}

#[test]
fn host_loopback_accepts_local_variants() {
    assert!(host_is_loopback("localhost:3210"));
    assert!(host_is_loopback("127.0.0.1"));
    assert!(host_is_loopback("127.0.0.2:3210"));
    assert!(host_is_loopback("::1"));
    assert!(host_is_loopback("[::1]:3210"));
}

#[test]
fn host_loopback_rejects_remote_variants() {
    assert!(!host_is_loopback("100.101.1.2:3210"));
    assert!(!host_is_loopback("example.local:3210"));
}

#[test]
fn host_loopback_trims_surrounding_whitespace() {
    assert!(host_is_loopback(" localhost "));
    assert!(host_is_loopback("\t127.0.0.1:3210\n"));
    assert!(host_is_loopback(" [::1]:3210 "));
}

#[test]
fn host_loopback_accepts_bracketed_ipv6_with_and_without_port() {
    assert!(host_is_loopback("[::1]"));
    assert!(host_is_loopback("[::1]:3210"));
}

#[test]
fn host_loopback_accepts_unbracketed_ipv6() {
    assert!(host_is_loopback("::1"));
    assert!(host_is_loopback("0:0:0:0:0:0:0:1"));
}

#[test]
fn host_loopback_rejects_malformed_ports() {
    assert!(!host_is_loopback("localhost:http"));
    assert!(!host_is_loopback("127.0.0.1:http"));
    assert!(!host_is_loopback("::1:http"));
}

#[test]
fn host_loopback_rejects_colon_containing_remote_strings() {
    assert!(!host_is_loopback("example.local:3210:extra"));
    assert!(!host_is_loopback("100.101.1.2:3210:extra"));
}

#[test]
fn host_loopback_rejects_empty_and_non_loopback_dns_hosts() {
    assert!(!host_is_loopback(""));
    assert!(!host_is_loopback("   "));
    assert!(!host_is_loopback("example.com"));
    assert!(!host_is_loopback("example.com:3210"));
}

#[test]
fn parse_osascript_output_accepts_expected_statuses() {
    let created = parse_osascript_output("created\tpane-1", "sess-1").unwrap();
    assert_eq!(created.status, "created");
    assert_eq!(created.pane_id.as_deref(), Some("pane-1"));

    let focused = parse_osascript_output("focused|pane-2", "sess-2").unwrap();
    assert_eq!(focused.status, "focused");
    assert_eq!(focused.pane_id.as_deref(), Some("pane-2"));

    let swapped = parse_osascript_output("swapped|pane-3", "sess-3").unwrap();
    assert_eq!(swapped.status, "swapped");
    assert_eq!(swapped.pane_id.as_deref(), Some("pane-3"));

    let fallback = parse_osascript_output("fallback_created|pane-4", "sess-4").unwrap();
    assert_eq!(fallback.status, "fallback_created");
    assert_eq!(fallback.pane_id.as_deref(), Some("pane-4"));
}

#[test]
fn parse_osascript_output_rejects_empty_status() {
    let err = parse_osascript_output("", "sess-1").unwrap_err();
    assert!(err.to_string().contains("empty response"));
}

#[test]
fn find_binary_in_path_returns_absolute_match() {
    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();
    std::fs::write(fake_bin_dir.join("tmux"), "#!/bin/sh\n").unwrap();
    let path = OsString::from(fake_bin_dir.as_os_str());

    let resolved = find_binary_in_path_os("tmux", &path).unwrap();
    assert_eq!(resolved, fake_bin_dir.join("tmux"));
}

#[test]
fn validate_tmux_binary_rejects_relative_override() {
    let err = validate_tmux_binary(PathBuf::from("tmux")).unwrap_err();
    assert!(err.to_string().contains("not absolute"));
}

#[test]
fn build_iterm_attach_command_uses_simple_attach_exec() {
    let command = build_iterm_attach_command("main", Path::new("/opt/homebrew/bin/tmux"));

    assert_eq!(
        command,
        "exec /opt/homebrew/bin/tmux attach-session -t '=main'"
    );
}

#[test]
fn build_iterm_attach_command_preserves_whitelisted_tokens() {
    let command = build_iterm_attach_command("team-session", Path::new("/tmp/tmux/tmux"));
    assert_eq!(
        command,
        "exec /tmp/tmux/tmux attach-session -t '=team-session'"
    );
}

#[test]
fn build_ghostty_attach_command_preserves_whitelisted_tokens() {
    let command = build_ghostty_attach_command("team-session", Path::new("/tmp/tmux/tmux"));
    assert_eq!(
        command,
        "exec /tmp/tmux/tmux attach-session -t '=team-session'"
    );
}

#[test]
fn build_iterm_attach_command_quotes_words_with_spaces() {
    let command = build_iterm_attach_command("team session", Path::new("/tmp/tmux builds/tmux"));
    assert_eq!(
        command,
        "exec '/tmp/tmux builds/tmux' attach-session -t '=team session'"
    );
}

#[test]
fn build_ghostty_attach_command_quotes_words_with_spaces() {
    let command = build_ghostty_attach_command("team session", Path::new("/tmp/tmux builds/tmux"));
    assert_eq!(
        command,
        "exec '/tmp/tmux builds/tmux' attach-session -t '=team session'"
    );
}

#[test]
fn native_attach_commands_accept_tmux_names_with_colons() {
    let iterm = build_iterm_attach_command("team:api", Path::new("/tmp/tmux"));
    let ghostty = build_ghostty_attach_command("team:api", Path::new("/tmp/tmux"));

    assert_eq!(iterm, "exec /tmp/tmux attach-session -t '=team:api'");
    assert_eq!(ghostty, "exec /tmp/tmux attach-session -t '=team:api'");
    validate_osascript_script_arg("tmux_name", "team:api")
        .expect("tmux permits colons in session names");
    validate_osascript_script_arg("attach_command", &iterm)
        .expect("generated iTerm attach command should validate");
    validate_osascript_script_arg("attach_command", &ghostty)
        .expect("generated Ghostty attach command should validate");
}

#[tokio::test]
async fn query_tmux_pane_metadata_uses_exact_pane_target_for_numeric_names() {
    let temp = tempdir().unwrap();
    let fake_tmux = temp.path().join("tmux");
    let log_path = temp.path().join("tmux-args.log");
    std::fs::write(
            &fake_tmux,
            format!(
                "#!/bin/sh\nset -eu\nfor arg in \"$@\"; do\n  printf '%s\\n' \"$arg\" >> \"{log}\"\ndone\nif [ \"${{1-}}\" = \"display-message\" ]; then\n  printf '%%7\\t/tmp/project\\n'\n  exit 0\nfi\nexit 64\n",
                log = log_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let (pane_id, cwd) = query_tmux_pane_metadata(&fake_tmux, "7").await.unwrap();

    assert_eq!(pane_id.as_deref(), Some("7"));
    assert_eq!(cwd.as_deref(), Some("/tmp/project"));
    let args = std::fs::read_to_string(&log_path).unwrap();
    let lines = args.lines().collect::<Vec<_>>();
    assert_eq!(
        lines,
        vec![
            "display-message",
            "-p",
            "-t",
            "=7:",
            "#{pane_id}\t#{pane_current_path}"
        ]
    );
}

fn attention_group_summary(session_id: &str, tmux_name: &str) -> SessionSummary {
    SessionSummary {
        session_id: session_id.to_string(),
        tmux_name: tmux_name.to_string(),
        state: crate::types::SessionState::Idle,
        current_command: None,
        state_evidence: crate::types::StateEvidence::new("test"),
        cwd: "/tmp".to_string(),
        tool: Some("Codex".to_string()),
        token_count: 0,
        context_limit: 192_000,
        thought: None,
        thought_state: crate::types::ThoughtState::Holding,
        thought_source: crate::types::ThoughtSource::CarryForward,
        thought_updated_at: None,
        rest_state: crate::types::RestState::Drowsy,
        commit_candidate: false,
        action_cues: Vec::new(),
        objective_changed_at: None,
        last_skill: None,
        is_stale: false,
        attached_clients: 0,
        stale_attached_clients: 0,
        transport_health: crate::types::TransportHealth::Healthy,
        last_activity_at: chrono::Utc::now(),
        repo_theme_id: None,
        batch: None,
    }
}

#[tokio::test]
async fn no_focus_attention_group_refresh_reuses_existing_tmux_session() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let temp = tempdir().unwrap();
    let fake_tmux = temp.path().join("tmux");
    let log_path = temp.path().join("tmux.log");
    let first_command = build_attention_group_attach_command("new one", &fake_tmux);
    let second_command = build_attention_group_attach_command("next one", &fake_tmux);
    std::fs::write(
            &fake_tmux,
            format!(
                "#!/bin/sh\nset -eu\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncase \"${{1-}}\" in\n  has-session)\n    exit 0\n    ;;\n  list-panes)\n    case \"$*\" in\n      *pane_start_command*)\n        printf '%%1\\t%s\\n%%2\\t%s\\n' \"{first_command}\" \"{second_command}\"\n        ;;\n      *)\n        printf '%%1\\n%%2\\n'\n        ;;\n    esac\n    exit 0\n    ;;\n  *)\n    exit 0\n    ;;\nesac\n",
                log = log_path.display(),
                first_command = first_command,
                second_command = second_command
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);
    let sessions = vec![
        attention_group_summary("sess-new", "new one"),
        attention_group_summary("sess-next", "next one"),
    ];

    let response = open_native_attention_group(
        NativeDesktopApp::Iterm,
        GhosttyOpenMode::Swap,
        &sessions,
        false,
        AttentionGroupLayout::Tiled,
    )
    .await
    .unwrap();

    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(response.status, "refreshed");
    assert!(!response.focused);
    assert_eq!(response.session_ids, vec!["sess-new", "sess-next"]);
    assert_eq!(
        response.attach_command.as_deref(),
        Some("tmux attach -t swimmers-attention")
    );

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        !log.lines().any(|line| line.starts_with("kill-session")),
        "no-focus refresh must not kill the attached attention tmux session: {log}"
    );
    assert!(log.contains("has-session\t-t\t=swimmers-attention"));
    assert!(
        !log.contains("respawn-pane\t-k"),
        "unchanged no-focus refresh must not respawn already-correct panes: {log}"
    );
    assert!(
        !log.contains("kill-pane"),
        "unchanged no-focus refresh must not close already-correct panes: {log}"
    );
    assert!(
        !log.contains("split-window"),
        "unchanged no-focus refresh must not create duplicate panes: {log}"
    );
    assert!(
        !log.contains("select-layout"),
        "unchanged no-focus refresh must not retile unchanged panes: {log}"
    );
    assert!(
        log.contains("list-panes\t-t\t=swimmers-attention:\t-F\t#{pane_id}\t#{pane_start_command}")
    );
}

#[tokio::test]
async fn no_focus_attention_group_refresh_replaces_only_one_stale_pane() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let temp = tempdir().unwrap();
    let fake_tmux = temp.path().join("tmux");
    let log_path = temp.path().join("tmux.log");
    let old_command = build_attention_group_attach_command("old one", &fake_tmux);
    let keep_1 = build_attention_group_attach_command("keep 1", &fake_tmux);
    let keep_2 = build_attention_group_attach_command("keep 2", &fake_tmux);
    let keep_3 = build_attention_group_attach_command("keep 3", &fake_tmux);
    let keep_4 = build_attention_group_attach_command("keep 4", &fake_tmux);
    let keep_5 = build_attention_group_attach_command("keep 5", &fake_tmux);
    std::fs::write(
            &fake_tmux,
            format!(
                "#!/bin/sh\nset -eu\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncase \"${{1-}}\" in\n  has-session)\n    exit 0\n    ;;\n  list-panes)\n    printf '%%1\\t%s\\n%%2\\t%s\\n%%3\\t%s\\n%%4\\t%s\\n%%5\\t%s\\n%%6\\t%s\\n' \"{old}\" \"{keep_1}\" \"{keep_2}\" \"{keep_3}\" \"{keep_4}\" \"{keep_5}\"\n    exit 0\n    ;;\n  *)\n    exit 0\n    ;;\nesac\n",
                log = log_path.display(),
                old = old_command,
                keep_1 = keep_1,
                keep_2 = keep_2,
                keep_3 = keep_3,
                keep_4 = keep_4,
                keep_5 = keep_5
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);
    let sessions = vec![
        attention_group_summary("sess-keep-1", "keep 1"),
        attention_group_summary("sess-keep-2", "keep 2"),
        attention_group_summary("sess-keep-3", "keep 3"),
        attention_group_summary("sess-keep-4", "keep 4"),
        attention_group_summary("sess-keep-5", "keep 5"),
        attention_group_summary("sess-new", "new one"),
    ];

    let response = open_native_attention_group(
        NativeDesktopApp::Iterm,
        GhosttyOpenMode::Swap,
        &sessions,
        false,
        AttentionGroupLayout::EvenVertical,
    )
    .await
    .unwrap();

    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert_eq!(response.status, "refreshed");
    assert_eq!(response.session_ids.len(), 6);

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        !log.lines().any(|line| line.starts_with("kill-session")),
        "one-in-one-out refresh must preserve the attention tmux session: {log}"
    );
    assert_eq!(
        log.matches("respawn-pane\t-k").count(),
        1,
        "one-in-one-out refresh should respawn exactly one stale pane: {log}"
    );
    assert!(log.contains("respawn-pane\t-k\t-t\t%1\texec env TMUX="));
    assert!(log.contains("attach-session -t '=new one'"));
    assert!(
        !log.contains("kill-pane"),
        "one-in-one-out refresh must not close preserved panes: {log}"
    );
    assert!(
        !log.contains("split-window"),
        "full one-in-one-out refresh must not create extra panes: {log}"
    );
    assert_eq!(
        log.matches("select-layout\t-t\t=swimmers-attention:\teven-vertical")
            .count(),
        1,
        "changed refresh should retile once: {log}"
    );
}

#[tokio::test]
async fn no_focus_attention_group_refresh_does_not_rebuild_after_pane_error() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let temp = tempdir().unwrap();
    let fake_tmux = temp.path().join("tmux");
    let log_path = temp.path().join("tmux.log");
    std::fs::write(
            &fake_tmux,
            format!(
                "#!/bin/sh\nset -eu\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncase \"${{1-}}\" in\n  has-session)\n    exit 0\n    ;;\n  list-panes)\n    printf '%%1\\n%%2\\n'\n    exit 0\n    ;;\n  respawn-pane)\n    printf 'pane disappeared\\n' >&2\n    exit 1\n    ;;\n  *)\n    exit 0\n    ;;\nesac\n",
                log = log_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);
    let sessions = vec![attention_group_summary("sess-new", "new one")];

    let err = open_native_attention_group(
        NativeDesktopApp::Iterm,
        GhosttyOpenMode::Swap,
        &sessions,
        false,
        AttentionGroupLayout::Tiled,
    )
    .await
    .unwrap_err();

    match original_tmux {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    assert!(
        format!("{err:#}").contains("failed to refresh attention group pane for new one"),
        "unexpected error: {err:#}"
    );
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        !log.lines().any(|line| line.starts_with("kill-session")),
        "failed no-focus refresh must not detach the existing attention group: {log}"
    );
    assert!(
        !log.lines().any(|line| line.starts_with("new-session")),
        "failed no-focus refresh must not rebuild the attached group: {log}"
    );
}

#[tokio::test]
async fn focused_attention_group_reuses_tmux_session_and_opens_ghostty_window() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_pane_id(ATTENTION_GROUP_SESSION_ID, None);

    let temp = tempdir().unwrap();
    let fake_tmux = temp.path().join("tmux");
    let tmux_log_path = temp.path().join("tmux.log");
    let first_command = build_attention_group_attach_command("new one", &fake_tmux);
    let second_command = build_attention_group_attach_command("next one", &fake_tmux);
    std::fs::write(
            &fake_tmux,
            format!(
                "#!/bin/sh\nset -eu\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncase \"${{1-}}\" in\n  has-session)\n    exit 0\n    ;;\n  list-panes)\n    case \"$*\" in\n      *pane_start_command*)\n        printf '%%1\\t%s\\n%%2\\t%s\\n' \"{first_command}\" \"{second_command}\"\n        ;;\n      *)\n        printf '%%1\\n%%2\\n'\n        ;;\n    esac\n    exit 0\n    ;;\n  display-message)\n    printf '%%14\\t/tmp/swimmers\\n'\n    exit 0\n    ;;\n  *)\n    exit 0\n    ;;\nesac\n",
                log = tmux_log_path.display(),
                first_command = first_command,
                second_command = second_command
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let fake_osascript_dir = temp.path().join("osa-bin");
    std::fs::create_dir_all(&fake_osascript_dir).unwrap();
    let fake_osascript = fake_osascript_dir.join("osascript");
    let osa_log_path = temp.path().join("osascript.log");
    let osa_count_path = temp.path().join("osascript-count");
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*) printf '1.3.1\\n' ;;\n    *) printf 'ghostty-tab-main\\n' ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nif [ \"$count\" -eq 1 ]; then\n  printf 'created|attention-pane\\n'\nelse\n  known=\"${{9-}}\"\n  if [ -z \"$known\" ]; then\n    printf 'created|fresh-attention-pane\\n'\n  else\n    printf 'focused|%s\\n' \"$known\"\n  fi\nfi\n",
                log = osa_log_path.display(),
                counter = osa_count_path.display()
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    let original_path = std::env::var_os("PATH");
    let original_tmux = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(
        "PATH",
        std::env::join_paths([fake_osascript_dir.as_path()]).unwrap(),
    );
    std::env::set_var(TMUX_BIN_ENV, &fake_tmux);
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("generic-preview-pane"));

    let sessions = vec![
        attention_group_summary("sess-new", "new one"),
        attention_group_summary("sess-next", "next one"),
    ];
    let first_response = open_native_attention_group(
        NativeDesktopApp::Ghostty,
        GhosttyOpenMode::Swap,
        &sessions,
        true,
        AttentionGroupLayout::Tiled,
    )
    .await
    .unwrap();
    let second_response = open_native_attention_group(
        NativeDesktopApp::Ghostty,
        GhosttyOpenMode::Swap,
        &sessions,
        true,
        AttentionGroupLayout::Tiled,
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
    clear_ghostty_preview_term_cache();
    remember_pane_id(ATTENTION_GROUP_SESSION_ID, None);

    assert_eq!(first_response.status, "created");
    assert!(first_response.focused);
    assert_eq!(first_response.pane_id.as_deref(), Some("attention-pane"));
    assert_eq!(second_response.status, "focused");
    assert!(second_response.focused);
    assert_eq!(second_response.pane_id.as_deref(), Some("attention-pane"));

    let tmux_log = std::fs::read_to_string(&tmux_log_path).unwrap();
    assert!(
        !tmux_log
            .lines()
            .any(|line| line.starts_with("kill-session")),
        "focused refresh must preserve the managed attention tmux session: {tmux_log}"
    );
    assert!(tmux_log.contains("has-session\t-t\t=swimmers-attention"));
    assert!(
        !tmux_log.contains("respawn-pane\t-k"),
        "focused open must not respawn already-correct attention panes: {tmux_log}"
    );
    assert!(
        !tmux_log.contains("kill-pane"),
        "focused open must not close already-correct attention panes: {tmux_log}"
    );

    let osa_log = std::fs::read_to_string(&osa_log_path).unwrap();
    let mut calls = osa_log.lines();
    let first_call = calls
        .next()
        .expect("ghostty script call")
        .split('\t')
        .collect::<Vec<_>>();
    let second_call = calls
        .next()
        .expect("second ghostty script call")
        .split('\t')
        .collect::<Vec<_>>();
    assert_eq!(first_call[1], ATTENTION_GROUP_SESSION_ID);
    assert_eq!(first_call[2], ATTENTION_GROUP_TMUX_NAME);
    assert_eq!(
        first_call.len(),
        8,
        "first attention window open must not pass a generic preview cached terminal id"
    );
    assert!(
        first_call[6].starts_with("swimmers-attention :: "),
        "attention windows must use a dedicated Ghostty title namespace, got {:?}",
        first_call[6]
    );
    assert_eq!(first_call[7], GhosttyOpenMode::Window.label());
    assert_eq!(second_call[1], ATTENTION_GROUP_SESSION_ID);
    assert_eq!(second_call[2], ATTENTION_GROUP_TMUX_NAME);
    assert_eq!(second_call[7], GhosttyOpenMode::Window.label());
    assert_eq!(
        second_call.len(),
        9,
        "second attention click must pass the cached managed terminal id"
    );
    assert_eq!(
        second_call[8], "attention-pane",
        "repeat attention click should reuse the cached managed attention Ghostty terminal id"
    );
}

#[test]
fn build_iterm_display_name_prefers_normalized_pane_id_and_cwd_basename() {
    assert_eq!(
        build_iterm_display_name(
            "/Users/b/repos/swimmers/",
            "codex-20260302-162713",
            Some("%12")
        ),
        "12 swimmers"
    );
}

#[test]
fn build_iterm_display_name_falls_back_to_tmux_name_when_cwd_is_empty() {
    assert_eq!(
        build_iterm_display_name("", "codex-20260302-162713", Some("%7")),
        "7 codex-20260302-162713"
    );
}

#[test]
fn build_iterm_display_name_omits_separator_when_pane_id_is_missing() {
    assert_eq!(
        build_iterm_display_name("/Users/b/repos/swimmers", "codex-20260302-162713", None),
        "swimmers"
    );
}

#[test]
fn build_ghostty_display_name_prefers_cwd_basename() {
    assert_eq!(
        build_ghostty_display_name("/Users/b/repos/swimmers", "codex-20260302-162713"),
        "swimmers"
    );
}

#[test]
fn ghostty_swap_script_replaces_managed_preview_in_place() {
    let script = std::fs::read_to_string(
        script_path_for_app(NativeDesktopApp::Ghostty).expect("script path should resolve"),
    )
    .expect("ghostty script should be present");
    let focus_block = script
        .split("on focusManagedWindowTerminal")
        .nth(1)
        .and_then(|tail| tail.split("end focusManagedWindowTerminal").next())
        .expect("managed window focus handler should be present");

    assert!(script.contains("on managedTerminals(targetTab, managedTitlePrefix)"));
    assert!(script.contains("on closeManagedTerminals(targetTerms)"));
    assert!(
        script.contains("set managedTerms to my managedTerminals(targetTab, managedTitlePrefix)")
    );
    assert!(script.contains("my closeManagedTerminals(duplicateManagedTerms)"));
    assert!(
        script.contains("on replacePreviewSplit(managedTerm, cfg, managedTitle, attachCommand)")
    );
    assert!(
        script.contains("set newTerm to split managedTerm direction right with configuration cfg")
    );
    assert!(script.contains("on managedTerminalAcrossWindows(knownManagedId, managedTitlePrefix)"));
    assert!(script
        .contains("if (id of candidateTerm as text) is knownManagedId then return candidateTerm"));
    assert!(script.contains("if openMode is \"window\" then"));
    assert!(
        script.contains("set newTerm to my createManagedWindow(cfg, managedTitle, attachCommand)")
    );
    assert!(script.contains(
        "set newTerm to my replacePreviewSplit(managedTerm, cfg, managedTitle, attachCommand)"
    ));
    assert!(
            !focus_block.contains("sendAttachCommand"),
            "focusing an existing managed attention window must not send a duplicate tmux attach command"
        );
    assert_eq!(script.match_indices("my resizePreview(").count(), 1);
}

#[test]
fn resolve_script_path_prefers_override_root() {
    let temp = tempdir().unwrap();
    let override_root = temp.path().join("override");
    let override_script = override_root.join(ITERM_SCRIPT_RELATIVE_PATH);
    std::fs::create_dir_all(override_script.parent().unwrap()).unwrap();
    std::fs::write(&override_script, "-- override").unwrap();

    let resolved = resolve_script_path(
        ITERM_SCRIPT_RELATIVE_PATH,
        Some(override_root.as_path()),
        None,
        None,
        Path::new("/tmp/missing-manifest"),
        temp.path(),
        ITERM_SCRIPT_SOURCE,
    )
    .unwrap();

    assert_eq!(resolved, override_script);
}

#[test]
fn resolve_script_path_keeps_missing_override_visible() {
    let temp = tempdir().unwrap();
    let override_root = temp.path().join("override");
    let override_script = override_root.join(ITERM_SCRIPT_RELATIVE_PATH);

    let resolved = resolve_script_path(
        ITERM_SCRIPT_RELATIVE_PATH,
        Some(override_root.as_path()),
        None,
        None,
        Path::new("/tmp/missing-manifest"),
        temp.path(),
        ITERM_SCRIPT_SOURCE,
    )
    .unwrap();

    assert_eq!(resolved, override_script);
    assert!(!resolved.exists());
}

#[test]
fn resolve_script_path_finds_repo_relative_to_current_exe_after_move() {
    let temp = tempdir().unwrap();
    let repo_root = temp.path().join("opensource/swimmers");
    let script_path = repo_root.join(ITERM_SCRIPT_RELATIVE_PATH);
    let current_exe = repo_root.join("target/debug/swimmers");
    std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(current_exe.parent().unwrap()).unwrap();
    std::fs::write(&script_path, "-- repo script").unwrap();

    let resolved = resolve_script_path(
        ITERM_SCRIPT_RELATIVE_PATH,
        None,
        Some(current_exe.as_path()),
        None,
        Path::new("/tmp/old/swimmers"),
        temp.path(),
        ITERM_SCRIPT_SOURCE,
    )
    .unwrap();

    assert_eq!(resolved, script_path);
}

#[test]
fn resolve_script_path_materializes_bundled_script_without_checkout() {
    let temp = tempdir().unwrap();
    let bundled_root = temp.path().join("installed-assets");

    let resolved = resolve_script_path(
        ITERM_SCRIPT_RELATIVE_PATH,
        None,
        Some(Path::new("/opt/swimmers/bin/swimmers")),
        Some(Path::new("/Users/b")),
        Path::new("/var/empty/swimmers"),
        &bundled_root,
        ITERM_SCRIPT_SOURCE,
    )
    .unwrap();

    assert_eq!(resolved, bundled_root.join(ITERM_SCRIPT_RELATIVE_PATH));
    assert_eq!(
        std::fs::read_to_string(&resolved).unwrap(),
        ITERM_SCRIPT_SOURCE
    );
}

#[test]
fn unique_tmp_suffix_never_repeats_across_threads() {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    let seen = Arc::new(Mutex::new(HashSet::new()));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                for _ in 0..256 {
                    let suffix = unique_tmp_suffix();
                    let mut guard = seen.lock().unwrap();
                    assert!(
                        guard.insert(suffix.clone()),
                        "duplicate tmp suffix produced: {suffix}"
                    );
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(seen.lock().unwrap().len(), 8 * 256);
}

#[test]
fn concurrent_materialize_does_not_collide_on_tmp_path() {
    use std::sync::Arc;

    let temp = tempdir().unwrap();
    let bundled_root = Arc::new(temp.path().join("installed-assets"));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let bundled_root = Arc::clone(&bundled_root);
            std::thread::spawn(move || {
                materialize_bundled_script(
                    ITERM_SCRIPT_RELATIVE_PATH,
                    &bundled_root,
                    ITERM_SCRIPT_SOURCE,
                )
                .expect("concurrent materialize should not fail on tmp collision")
            })
        })
        .collect();

    for handle in handles {
        let resolved = handle.join().unwrap();
        assert_eq!(resolved, bundled_root.join(ITERM_SCRIPT_RELATIVE_PATH));
        assert_eq!(
            std::fs::read_to_string(&resolved).unwrap(),
            ITERM_SCRIPT_SOURCE
        );
    }
}

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

// ------------------------------------------------------------------
// Characterization tests for the Ghostty double-click handoff bug.
//
// Workgraph: local divide-and-conquer investigation
//   divide-and-conquer/2026-04-23T13-08-14Z/
// Describe packets:
//   - describe-swap-fallback.md       (TC-1..TC-5)
//   - describe-front-tab-empty.md     (TC-1..TC-5)
//   - describe-attach-race.md         (2 cases)
//   - describe-tmux-resolution.md     (4 cases)
//
// These tests characterize current behavior. Tests suffixed
// `_documents_bug` intentionally assert remaining *buggy* observable state
// so the behavior regresses loudly if someone changes it.
// ------------------------------------------------------------------

// Shared fake-binary builder to keep the new tests terse.
//
// osascript behavior:
//   - `-e "...get version..."`        -> prints `1.3.1`
//   - `-e "...selected tab..."`       -> prints tab_id_stdout (may be empty)
//                                         or exits non-zero if tab_id_err_exit
//   - any other argv (the script run) -> logs argv to `<log>` and prints
//                                         `created|<pane_prefix><N>` where
//                                         N is the sequential call count.
struct GhosttyFakes {
    _temp: tempfile::TempDir,
    fake_tmux: PathBuf,
    fake_osascript_dir: PathBuf,
    log_path: PathBuf,
}

fn write_ghostty_fakes(
    tab_id_stdout: &str,
    tab_id_err_exit: bool,
    pane_prefix: &str,
) -> GhosttyFakes {
    write_ghostty_fakes_with_status(tab_id_stdout, tab_id_err_exit, pane_prefix, "created")
}

fn write_ghostty_fakes_with_status(
    tab_id_stdout: &str,
    tab_id_err_exit: bool,
    pane_prefix: &str,
    result_status: &str,
) -> GhosttyFakes {
    let temp = tempdir().unwrap();
    let fake_bin_dir = temp.path().join("tmux-bin");
    std::fs::create_dir_all(&fake_bin_dir).unwrap();
    let fake_tmux = fake_bin_dir.join("tmux");
    std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/tmp/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_tmux, perms).unwrap();

    let log_path = temp.path().join("osascript.log");
    let fake_osascript_dir = temp.path().join("osa-bin");
    std::fs::create_dir_all(&fake_osascript_dir).unwrap();
    let fake_osascript = fake_osascript_dir.join("osascript");
    let counter_path = temp.path().join("osa-count");
    let tab_err = if tab_id_err_exit { 1 } else { 0 };
    std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      if [ \"{tab_err}\" = \"1\" ]; then\n        printf 'ghostty tab query failed\\n' >&2\n        exit 1\n      fi\n      printf '{tab}\\n'\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nprintf '{status}|{prefix}%s\\n' \"$count\"\n",
                tab = tab_id_stdout,
                tab_err = tab_err,
                log = log_path.display(),
                counter = counter_path.display(),
                prefix = pane_prefix,
                status = result_status,
            ),
        )
        .unwrap();
    let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_osascript, perms).unwrap();

    GhosttyFakes {
        _temp: temp,
        fake_tmux,
        fake_osascript_dir,
        log_path,
    }
}

// RAII wrapper that installs fake PATH + SWIMMERS_TMUX_BIN and restores them.
struct EnvSwap {
    original_path: Option<OsString>,
    original_tmux: Option<OsString>,
}

impl EnvSwap {
    fn install(fakes: &GhosttyFakes) -> Self {
        let original_path = std::env::var_os("PATH");
        let original_tmux = std::env::var_os(TMUX_BIN_ENV);
        let path_value = std::env::join_paths([fakes.fake_osascript_dir.as_path()]).unwrap();
        std::env::set_var("PATH", path_value);
        std::env::set_var(TMUX_BIN_ENV, &fakes.fake_tmux);
        Self {
            original_path,
            original_tmux,
        }
    }
}

impl Drop for EnvSwap {
    fn drop(&mut self) {
        match self.original_path.take() {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match self.original_tmux.take() {
            Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
            None => std::env::remove_var(TMUX_BIN_ENV),
        }
    }
}

// ------------------------------------------------------------------
// WG-S1: ghostty-swap-fallback (describe-swap-fallback.md)
// ------------------------------------------------------------------

// TC-1: Preview cache miss with title-prefix match — arg 8 is omitted.
#[tokio::test]
async fn swap_fallback_tc1_cache_miss_omits_known_preview_id() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "pane-tc1-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc1",
        "tmux-tc1",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    // argv: [script, sess, tmux, cwd, attach, display, prefix, mode]
    // Cache was empty, so no 8th index (index 8 would be the known preview id).
    assert_eq!(
        call.len(),
        8,
        "no known_preview_id arg expected on cache miss"
    );
    assert_eq!(call[7], GhosttyOpenMode::Swap.label());

    clear_ghostty_preview_term_cache();
}

// TC-2: Stale cache references a live but unlabeled terminal — arg 8 passed.
#[tokio::test]
async fn swap_fallback_tc2_stale_cache_passes_preview_id_arg() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("term-stale"));
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "pane-tc2-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc2",
        "tmux-tc2",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "fallback_created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(
        call.len(),
        9,
        "stale cache entry should be forwarded as arg 8"
    );
    assert_eq!(call[8], "term-stale");
    assert!(cached_ghostty_preview_term_id(Some("ghostty-tab-main")).is_none());

    clear_ghostty_preview_term_cache();
}

// TC-3 / TC-4: A stale known preview id that falls back to create-new is
// surfaced distinctly and does not overwrite the preview cache.
#[tokio::test]
async fn swap_fallback_tc3_tc4_stale_create_new_is_reported_and_not_cached() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("term-stale"));
    // Simulate the script choosing to createPreviewSplit instead of swap.
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "fresh-pane-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc3",
        "tmux-tc3",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(
        result.status, "fallback_created",
        "stale-id create-new fallback should be visible to callers"
    );
    assert_eq!(result.pane_id.as_deref(), Some("fresh-pane-1"));
    assert!(cached_ghostty_preview_term_id(Some("ghostty-tab-main")).is_none());

    clear_ghostty_preview_term_cache();
}

// TC-5: Successful swap baseline — cached id forwarded, cache updated.
#[tokio::test]
async fn swap_fallback_tc5_golden_path_forwards_cached_id_and_updates_cache() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("ghostty-tab-main"), Some("term-preview"));
    let fakes =
        write_ghostty_fakes_with_status("ghostty-tab-main", false, "new-preview-", "swapped");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-tc5",
        "tmux-tc5",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(call[8], "term-preview", "cached id must be forwarded");
    assert_eq!(result.pane_id.as_deref(), Some("new-preview-1"));
    assert_eq!(
        cached_ghostty_preview_term_id(Some("ghostty-tab-main")).as_deref(),
        Some("new-preview-1"),
    );

    clear_ghostty_preview_term_cache();
}

// ------------------------------------------------------------------
// WG-S4: front-tab-id empty/error path (describe-front-tab-empty.md)
// ------------------------------------------------------------------

// TC-1: query_front_ghostty_tab_id returns Ok(None) — cache lookup skipped,
// script invoked with known_preview_id=None even if an unrelated cache
// entry exists.
#[tokio::test]
async fn front_tab_empty_tc1_ok_empty_skips_cache_lookup() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("tab-known"), Some("term-prev"));
    // Tab query returns empty string -> Ok(None).
    let fakes = write_ghostty_fakes("", false, "pane-ft1-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-ft1",
        "tmux-ft1",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(call.len(), 8, "no cache arg when tab id is empty");
    // Unrelated cache entry must not be written over, and no new entry
    // must be written under an empty key.
    assert_eq!(
        cached_ghostty_preview_term_id(Some("tab-known")).as_deref(),
        Some("term-prev"),
    );

    clear_ghostty_preview_term_cache();
}

// TC-2: query_front_ghostty_tab_id returns Err(...) — error absorbed,
// swap attempt proceeds.
#[tokio::test]
async fn front_tab_empty_tc2_err_absorbed_swap_proceeds() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("tab-known"), Some("term-prev"));
    // Tab query exits non-zero -> Err absorbed by .unwrap_or(None).
    let fakes = write_ghostty_fakes("", true, "pane-ft2-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-ft2",
        "tmux-ft2",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
    assert_eq!(
        call.len(),
        8,
        "tab-id Err must not be forwarded as a cache arg"
    );

    clear_ghostty_preview_term_cache();
}

// TC-3 / TC-4: When pre-script AND post-script tab query both fail / return
// empty, the resulting pane id is NOT written into the cache, because
// remember_ghostty_preview_term_id short-circuits on an empty tab id.
// Unrelated entries remain intact.
#[tokio::test]
async fn front_tab_empty_tc3_tc4_no_stale_cache_write_when_tab_id_missing() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    remember_ghostty_preview_term_id(Some("tab-known"), Some("term-prev"));
    let fakes = write_ghostty_fakes("", false, "pane-ft3-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-ft3",
        "tmux-ft3",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.pane_id.as_deref(), Some("pane-ft3-1"));
    // Cache under the empty key: none created.
    assert!(cached_ghostty_preview_term_id(Some("")).is_none());
    // Cache for an unrelated tab: untouched.
    assert_eq!(
        cached_ghostty_preview_term_id(Some("tab-known")).as_deref(),
        Some("term-prev"),
    );

    clear_ghostty_preview_term_cache();
}

// TC-5 (characterization variant): remember_ghostty_preview_term_id writes
// through cleanly when the resulting tab id is Some(non_empty), even if
// the pre-script tab query was missing. Purely tests the cache shape.
#[test]
fn front_tab_empty_tc5_post_script_recovery_writes_cache() {
    clear_ghostty_preview_term_cache();
    // No entry pre-call.
    assert!(cached_ghostty_preview_term_id(Some("tab-recovered")).is_none());
    // Simulate post-script success path directly.
    remember_ghostty_preview_term_id(Some("tab-recovered"), Some("new-term-5"));
    assert_eq!(
        cached_ghostty_preview_term_id(Some("tab-recovered")).as_deref(),
        Some("new-term-5"),
    );
    clear_ghostty_preview_term_cache();
}

// ------------------------------------------------------------------
// WG-S2: ghostty-attach-race (describe-attach-race.md)
// ------------------------------------------------------------------

// Case 1 + Case 2: the Rust layer has no retry or readiness gate on the
// attach command. A single script invocation returning `created|<id>` is
// treated as full success, and the pane id is cached — regardless of
// whether the shell actually accepted the attach keystrokes. The race
// itself lives in Ghostty's input buffer and cannot be unit-tested here;
// this test characterizes the absent retry/validation at the Rust layer.
#[tokio::test]
async fn attach_race_rust_layer_has_no_retry_or_readiness_probe_documents_bug() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    clear_ghostty_preview_term_cache();
    let fakes = write_ghostty_fakes("ghostty-tab-main", false, "race-pane-");
    let _env = EnvSwap::install(&fakes);

    let result = open_or_focus_ghostty_session(
        "sess-race",
        "tmux-race",
        "/tmp/fallback",
        GhosttyOpenMode::Swap,
    )
    .await
    .unwrap();

    assert_eq!(result.status, "created");
    // Exactly ONE script run logged — no retry on potential attach drop.
    let log = std::fs::read_to_string(&fakes.log_path).unwrap();
    let script_invocations = log.lines().count();
    assert_eq!(
        script_invocations, 1,
        "bug: no retry/readiness probe after the script returns — the Rust \
             layer cannot detect a dropped attach command, so the pane id is \
             cached as success regardless of shell readiness"
    );
    // Cached as if successful.
    assert_eq!(
        cached_ghostty_preview_term_id(Some("ghostty-tab-main")).as_deref(),
        Some("race-pane-1"),
    );

    clear_ghostty_preview_term_cache();
}

// ------------------------------------------------------------------
// WG-S3: native-tmux-resolution (describe-tmux-resolution.md)
// ------------------------------------------------------------------

// Case 1: SWIMMERS_TMUX_BIN set to a relative path -> error names env var
// and "not absolute".
#[test]
fn tmux_resolution_case1_env_override_non_absolute_errors() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let original = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, "tmux");

    let err = resolve_tmux_binary().expect_err("relative env path must error");

    match original {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    let message = format!("{err:#}");
    assert!(
        message.contains(TMUX_BIN_ENV),
        "error names env var: {message}"
    );
    assert!(
        message.contains("not absolute"),
        "error cites non-absolute path: {message}"
    );
}

// Case 2: SWIMMERS_TMUX_BIN set to an absolute path that does not exist.
#[test]
fn tmux_resolution_case2_env_override_missing_file_errors() {
    let _env_guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let temp = tempdir().unwrap();
    let ghost_path = temp.path().join("nonexistent-tmux");
    let ghost_str = ghost_path.to_string_lossy().to_string();
    let original = std::env::var_os(TMUX_BIN_ENV);
    std::env::set_var(TMUX_BIN_ENV, &ghost_path);

    let err = resolve_tmux_binary().expect_err("missing-file env path must error");

    match original {
        Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
        None => std::env::remove_var(TMUX_BIN_ENV),
    }

    let message = format!("{err:#}");
    assert!(
        message.contains(TMUX_BIN_ENV),
        "error names env var: {message}"
    );
    assert!(
        message.contains(&ghost_str) || message.contains("binary not found"),
        "error cites missing path: {message}"
    );
}

// Case 3: find_binary_in_path_os returns None when PATH has no tmux.
// Exercises the PATH-lookup tier without requiring full fallback absence.
#[test]
fn tmux_resolution_case3_path_without_tmux_returns_none() {
    let temp = tempdir().unwrap();
    let empty_dir = temp.path().join("empty");
    std::fs::create_dir_all(&empty_dir).unwrap();
    let path_os = std::env::join_paths([empty_dir.as_path()]).unwrap();
    assert!(find_binary_in_path_os("tmux", &path_os).is_none());
}

// Case 4: find_binary_in_path_os finds tmux on PATH — documents that a
// PATH hit returns before any fallback iteration would run.
#[test]
fn tmux_resolution_case4_path_beats_fallbacks() {
    let temp = tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let tmux_path = bin_dir.join("tmux");
    std::fs::write(&tmux_path, b"#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = std::fs::metadata(&tmux_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmux_path, perms).unwrap();

    let path_os = std::env::join_paths([bin_dir.as_path()]).unwrap();
    let found = find_binary_in_path_os("tmux", &path_os).expect("PATH hit must resolve");
    assert_eq!(found, tmux_path);
    // None of the hardcoded fallback paths point inside the tempdir, so
    // the resolver would never have reached them.
    assert!(
        !TMUX_BIN_FALLBACKS
            .iter()
            .any(|candidate| found.starts_with(candidate)),
        "PATH-resolved tmux must not coincide with a hardcoded fallback"
    );
}
