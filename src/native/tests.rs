use super::*;
use proptest::prelude::*;
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

mod ghostty_scenarios;
mod open_or_focus;
mod tmux_resolution;

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
    let command = build_iterm_attach_command(
        "main",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/opt/homebrew/bin/tmux"),
    );

    assert_eq!(
        command,
        "exec /opt/homebrew/bin/tmux attach-session -t '=main'"
    );
}

#[test]
fn build_iterm_attach_command_preserves_whitelisted_tokens() {
    let command = build_iterm_attach_command(
        "team-session",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/tmp/tmux/tmux"),
    );
    assert_eq!(
        command,
        "exec /tmp/tmux/tmux attach-session -t '=team-session'"
    );
}

#[test]
fn build_ghostty_attach_command_preserves_whitelisted_tokens() {
    let command = build_ghostty_attach_command(
        "team-session",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/tmp/tmux/tmux"),
    );
    assert_eq!(
        command,
        "exec /tmp/tmux/tmux attach-session -t '=team-session'"
    );
}

#[test]
fn build_iterm_attach_command_quotes_words_with_spaces() {
    let command = build_iterm_attach_command(
        "team session",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/tmp/tmux builds/tmux"),
    );
    assert_eq!(
        command,
        "exec '/tmp/tmux builds/tmux' attach-session -t '=team session'"
    );
}

#[test]
fn build_ghostty_attach_command_quotes_words_with_spaces() {
    let command = build_ghostty_attach_command(
        "team session",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/tmp/tmux builds/tmux"),
    );
    assert_eq!(
        command,
        "exec '/tmp/tmux builds/tmux' attach-session -t '=team session'"
    );
}

#[test]
fn native_attach_commands_accept_tmux_names_with_colons() {
    let iterm = build_iterm_attach_command(
        "team:api",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/tmp/tmux"),
    );
    let ghostty = build_ghostty_attach_command(
        "team:api",
        &crate::tmux_target::TmuxTarget::Default,
        Path::new("/tmp/tmux"),
    );

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

    let (pane_id, cwd) =
        query_tmux_pane_metadata(&fake_tmux, "7", &crate::tmux_target::TmuxTarget::Default)
            .await
            .unwrap();

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
        tmux_target: crate::tmux_target::TmuxTarget::Default,
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
        environment: Default::default(),
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
    let first_command = build_attention_group_attach_command(
        &attention_group_summary("sess", "new one"),
        &fake_tmux,
    );
    let second_command = build_attention_group_attach_command(
        &attention_group_summary("sess", "next one"),
        &fake_tmux,
    );
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
    let old_command = build_attention_group_attach_command(
        &attention_group_summary("sess", "old one"),
        &fake_tmux,
    );
    let keep_1 = build_attention_group_attach_command(
        &attention_group_summary("sess", "keep 1"),
        &fake_tmux,
    );
    let keep_2 = build_attention_group_attach_command(
        &attention_group_summary("sess", "keep 2"),
        &fake_tmux,
    );
    let keep_3 = build_attention_group_attach_command(
        &attention_group_summary("sess", "keep 3"),
        &fake_tmux,
    );
    let keep_4 = build_attention_group_attach_command(
        &attention_group_summary("sess", "keep 4"),
        &fake_tmux,
    );
    let keep_5 = build_attention_group_attach_command(
        &attention_group_summary("sess", "keep 5"),
        &fake_tmux,
    );
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
    let first_command = build_attention_group_attach_command(
        &attention_group_summary("sess", "new one"),
        &fake_tmux,
    );
    let second_command = build_attention_group_attach_command(
        &attention_group_summary("sess", "next one"),
        &fake_tmux,
    );
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
