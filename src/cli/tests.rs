use super::*;

fn cfg(bind: &str, mode: AuthMode, token: Option<&str>) -> Config {
    Config {
        bind: bind.to_string(),
        auth_mode: mode,
        auth_token: token.map(|s| s.to_string()),
        ..Config::default()
    }
}

#[test]
fn loopback_strings_are_loopback() {
    assert!(is_loopback_bind("127.0.0.1"));
    assert!(is_loopback_bind("127.0.0.2"));
    assert!(is_loopback_bind("::1"));
    assert!(is_loopback_bind("localhost"));
    assert!(!is_loopback_bind("0.0.0.0"));
    assert!(!is_loopback_bind("192.168.1.1"));
    assert!(!is_loopback_bind("10.0.0.1"));
}

#[test]
fn loopback_bind_parses_host_port_forms() {
    assert!(is_loopback_bind("127.0.0.1:3210"));
    assert!(is_loopback_bind("[::1]:3210"));
    assert!(!is_loopback_bind("0.0.0.0:3210"));
}

#[test]
fn tailnet_bind_detects_tailscale_ranges() {
    assert!(is_tailnet_bind("100.64.0.1"));
    assert!(is_tailnet_bind("100.86.253.9:3210"));
    assert!(is_tailnet_bind("[fd7a:115c:a1e0::1]:3210"));
    assert!(!is_tailnet_bind("100.63.255.255"));
    assert!(!is_tailnet_bind("100.128.0.1"));
    assert!(!is_tailnet_bind("10.0.0.1"));
    assert!(!is_tailnet_bind("localhost"));
}

#[test]
fn bind_host_passes_through_plain_hosts() {
    assert_eq!(bind_host("127.0.0.1"), "127.0.0.1");
    assert_eq!(bind_host("0.0.0.0"), "0.0.0.0");
    assert_eq!(bind_host("localhost"), "localhost");
    assert_eq!(bind_host("example.internal"), "example.internal");
}

#[test]
fn bind_host_strips_numeric_port_from_host_port_forms() {
    assert_eq!(bind_host("127.0.0.1:3210"), "127.0.0.1");
    assert_eq!(bind_host("0.0.0.0:8080"), "0.0.0.0");
    assert_eq!(bind_host("localhost:80"), "localhost");
}

#[test]
fn bind_host_strips_brackets_from_ipv6_host_port_forms() {
    assert_eq!(bind_host("[::1]:3210"), "::1");
    assert_eq!(bind_host("[fe80::1]:8080"), "fe80::1");
}

#[test]
fn bind_host_keeps_plain_ipv6_literal_intact() {
    // Bare `::1` has multiple `:` so the host:port path bails out and the
    // whole string is returned, letting `is_loopback_bind` parse it as IP.
    assert_eq!(bind_host("::1"), "::1");
    assert_eq!(bind_host("fe80::1"), "fe80::1");
}

#[test]
fn bind_host_handles_bracketed_host_without_port() {
    assert_eq!(bind_host("[::1]"), "::1");
}

#[test]
fn bind_host_rejects_malformed_bracketed_input() {
    // No closing `]` falls through to the rsplit path; trailing junk after
    // the bracket must not be silently dropped.
    assert_eq!(bind_host("[::1"), "[::1");
    assert_eq!(bind_host("[::1]extra"), "[::1]extra");
}

#[test]
fn bind_host_rejects_non_numeric_or_empty_ports() {
    // Empty port, alphabetic port, or empty host all fall through to the
    // whole-string return so callers see the original (malformed) input.
    assert_eq!(bind_host("127.0.0.1:"), "127.0.0.1:");
    assert_eq!(bind_host("127.0.0.1:abc"), "127.0.0.1:abc");
    assert_eq!(bind_host(":3210"), ":3210");
}

#[test]
fn bind_host_trims_surrounding_whitespace() {
    assert_eq!(bind_host("  127.0.0.1  "), "127.0.0.1");
    assert_eq!(bind_host("\t[::1]:3210\n"), "::1");
}

#[test]
fn bind_host_handles_empty_string() {
    assert_eq!(bind_host(""), "");
    assert_eq!(bind_host("   "), "");
}

#[test]
fn parse_serve_subcommand() {
    let cli = ServerCli::parse_from(["swimmers", "serve"]);
    assert_eq!(cli.command, Some(ServerCommand::Serve));
}

#[test]
fn parse_bare_invocation_without_subcommand() {
    let cli = ServerCli::parse_from(["swimmers"]);
    assert_eq!(cli.command, None);
}

#[test]
fn parse_config_subcommand_without_action() {
    let cli = ServerCli::parse_from(["swimmers", "config"]);
    assert_eq!(cli.command, Some(ServerCommand::Config { action: None }));
}

#[test]
fn parse_config_ssh_import_requires_explicit_dry_run_flag_shape() {
    let cli = ServerCli::parse_from([
        "swimmers",
        "config",
        "ssh-import",
        "--dry-run",
        "--ssh-config",
        "/tmp/ssh-config",
    ]);
    assert_eq!(
        cli.command,
        Some(ServerCommand::Config {
            action: Some(ConfigAction::SshImport {
                dry_run: true,
                ssh_config: Some(PathBuf::from("/tmp/ssh-config")),
            })
        })
    );
}

#[test]
fn parse_tmux_next_name_subcommand() {
    let cli = ServerCli::parse_from(["swimmers", "tmux", "next-name"]);
    assert_eq!(
        cli.command,
        Some(ServerCommand::Tmux {
            action: TmuxAction::NextName
        })
    );
}

#[test]
fn parse_tmux_new_subcommand_with_cwd() {
    let cli = ServerCli::parse_from(["swimmers", "tmux", "new", "--cwd", "/tmp/project"]);
    assert_eq!(
        cli.command,
        Some(ServerCommand::Tmux {
            action: TmuxAction::New {
                cwd: Some(PathBuf::from("/tmp/project"))
            }
        })
    );
}

#[test]
fn numbered_tmux_name_rejects_empty_and_non_exact_digits() {
    for name in ["", " 8", "8 ", "\t8", "+8", "-8", "8a", "８", "１２"] {
        assert_eq!(numbered_tmux_name(name), None, "{name:?}");
    }
}

#[test]
fn numbered_tmux_name_accepts_ascii_digits_with_leading_zeroes() {
    assert_eq!(numbered_tmux_name("0"), Some(0));
    assert_eq!(numbered_tmux_name("0008"), Some(8));
}

#[test]
fn numbered_tmux_name_handles_u64_bounds() {
    assert_eq!(numbered_tmux_name("18446744073709551615"), Some(u64::MAX));
    assert_eq!(numbered_tmux_name("18446744073709551616"), None);
}

#[test]
fn next_numeric_tmux_name_uses_highest_existing_number_plus_one() {
    let names = [
        "6",
        "8",
        "dac-cyclechef-wave-01",
        "swimmers-attention",
        "08",
    ];

    assert_eq!(
        next_numeric_tmux_name_from_names(names).expect("next numeric name"),
        "9"
    );
}

#[test]
fn next_numeric_tmux_name_starts_at_zero_without_numbered_sessions() {
    let names = ["swimmers-attention", "dac-cyclechef-wave-01", "alpha"];

    assert_eq!(
        next_numeric_tmux_name_from_names(names).expect("next numeric name"),
        "0"
    );
}

#[test]
fn next_numeric_tmux_name_requires_exact_numeric_names() {
    let names = [" 8 ", "\t9", "10"];

    assert_eq!(
        next_numeric_tmux_name_from_names(names).expect("next numeric name"),
        "11"
    );
}

#[test]
fn next_numeric_tmux_name_errors_when_highest_number_overflows() {
    let names = ["18446744073709551615", "18446744073709551614", "alpha"];

    assert_eq!(
        next_numeric_tmux_name_from_names(names),
        Err("numeric tmux session counter exhausted".to_string())
    );
}

#[test]
fn create_numbered_tmux_session_retries_name_collisions() {
    let cwd = PathBuf::from("/tmp/project");
    let mut names = ["2".to_string(), "3".to_string()].into_iter();
    let mut calls = Vec::new();

    let created = create_numbered_tmux_session_with(
        Some(&cwd),
        64,
        || names.next().ok_or_else(|| "out of names".to_string()),
        |name, cwd_arg| {
            calls.push((name.to_string(), cwd_arg.map(|path| path.to_path_buf())));
            if name == "2" {
                Err(TmuxNewSessionError::AlreadyExists)
            } else {
                Ok(())
            }
        },
    )
    .expect("second name should be created");

    assert_eq!(created, "3");
    assert_eq!(
        calls,
        vec![
            ("2".to_string(), Some(cwd.clone())),
            ("3".to_string(), Some(cwd))
        ]
    );
}

#[test]
fn create_numbered_tmux_session_stops_on_non_collision_error() {
    let err = create_numbered_tmux_session_with(
        None,
        64,
        || Ok("4".to_string()),
        |_, _| Err(TmuxNewSessionError::Failed("permission denied".to_string())),
    )
    .expect_err("non-collision tmux failure should stop retries");

    assert_eq!(err, "permission denied");
}

#[test]
fn create_numbered_tmux_session_reports_exhaustion() {
    let mut attempts = 0;
    let err = create_numbered_tmux_session_with(
        None,
        2,
        || {
            attempts += 1;
            Ok(attempts.to_string())
        },
        |_, _| Err(TmuxNewSessionError::AlreadyExists),
    )
    .expect_err("persistent name collisions should exhaust attempts");

    assert_eq!(attempts, 2);
    assert_eq!(
        err,
        "failed to allocate a numeric tmux session after 2 attempts"
    );
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: std::ffi::OsString) -> Self {
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

fn test_path_with_prepend(bin_dir: &std::path::Path) -> std::ffi::OsString {
    let mut entries = vec![bin_dir.as_os_str().to_os_string()];
    if let Some(existing) = std::env::var_os("PATH") {
        entries.extend(std::env::split_paths(&existing).map(|path| path.into_os_string()));
    }
    std::env::join_paths(entries).expect("join PATH")
}

#[test]
fn create_numbered_tmux_session_uses_real_tmux_wrapper_with_fake_tmux() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .expect("env lock poisoned");
    let tmp = tempfile::tempdir().expect("tempdir");
    let created_file = tmp.path().join("created-name");
    let cwd_file = tmp.path().join("created-cwd");
    let script = format!(
        r#"#!/bin/sh
set -eu
case "${{1:-}}" in
  list-sessions)
printf '0\n7\nnot-numbered\n'
;;
  new-session)
if [ "${{2:-}}" != "-d" ] || [ "${{3:-}}" != "-s" ]; then
  printf 'unexpected tmux new-session flags\n' >&2
  exit 64
fi
printf '%s\n' "$4" > "{created_file}"
if [ "${{5:-}}" = "-c" ]; then
  printf '%s\n' "$6" > "{cwd_file}"
fi
;;
  *)
printf 'unexpected tmux command: %s\n' "${{1:-}}" >&2
exit 64
;;
esac
"#,
        created_file = created_file.display(),
        cwd_file = cwd_file.display()
    );
    write_executable_script(tmp.path(), "tmux", &script);
    let _path_guard = EnvVarGuard::set("PATH", test_path_with_prepend(tmp.path()));
    let cwd = tmp.path().join("launch");
    std::fs::create_dir(&cwd).expect("create cwd");

    let created = create_numbered_tmux_session(Some(&cwd)).expect("create session");

    assert_eq!(created, "8");
    assert_eq!(
        std::fs::read_to_string(created_file).expect("created file"),
        "8\n"
    );
    assert_eq!(
        std::fs::read_to_string(cwd_file).expect("cwd file"),
        format!("{}\n", cwd.display())
    );
}

#[test]
fn list_tmux_session_names_preserves_exact_tmux_names() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .expect("env lock poisoned");
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = r#"#!/bin/sh
set -eu
case "${1:-}" in
  list-sessions)
printf 'alpha\n  padded  \n\tindented\n\nbeta\n'
;;
  *)
printf 'unexpected tmux command: %s\n' "${1:-}" >&2
exit 64
;;
esac
"#;
    write_executable_script(tmp.path(), "tmux", script);
    let _path_guard = EnvVarGuard::set("PATH", test_path_with_prepend(tmp.path()));

    let names = list_tmux_session_names().expect("tmux session names");

    assert_eq!(
        names,
        vec![
            "alpha".to_string(),
            "  padded  ".to_string(),
            "\tindented".to_string(),
            "beta".to_string()
        ]
    );
}

#[test]
fn localtrust_loopback_passes_gate() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    assert!(enforce_localtrust_loopback(&c).is_ok());
}

#[test]
fn localtrust_non_loopback_refused() {
    let c = cfg("0.0.0.0", AuthMode::LocalTrust, None);
    let err = enforce_trust_bind_safety(&c).unwrap_err();
    assert!(err.contains("SWIMMERS_BIND=0.0.0.0"));
    assert!(err.contains("AUTH_MODE=tailnet_trust"));
}

#[test]
fn token_mode_non_loopback_allowed() {
    let c = cfg("0.0.0.0", AuthMode::Token, Some("secret"));
    assert!(enforce_trust_bind_safety(&c).is_ok());
}

#[test]
fn tailnet_trust_tailscale_bind_allowed() {
    let c = cfg("100.86.253.9", AuthMode::TailnetTrust, None);
    assert!(enforce_trust_bind_safety(&c).is_ok());
}

#[test]
fn tailnet_trust_public_bind_refused() {
    let c = cfg("0.0.0.0", AuthMode::TailnetTrust, None);
    let err = enforce_trust_bind_safety(&c).unwrap_err();
    assert!(err.contains("AUTH_MODE=tailnet_trust"));
    assert!(err.contains("Tailscale"));
}

#[test]
fn doctor_flags_localtrust_non_loopback() {
    let c = cfg("0.0.0.0", AuthMode::LocalTrust, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Ok("`clawgs defaults` ok".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    let auth_bind = findings.iter().find(|f| f.name == "auth/bind").unwrap();
    assert!(!auth_bind.ok);
    assert!(auth_bind.detail.contains("non-loopback"));
}

#[test]
fn doctor_orders_findings_and_preserves_mixed_check_details() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let findings = run_doctor_checks(
        &c,
        false,
        Err("clawgs not found at `clawgs`".to_string()),
        Err("permission denied".to_string()),
    );

    assert_eq!(
        findings,
        vec![
            DoctorFinding {
                ok: true,
                level: DoctorLevel::Ok,
                name: "auth/bind",
                detail: "bind=127.0.0.1 auth_mode=local_trust (safe)".to_string(),
            },
            DoctorFinding {
                ok: true,
                level: DoctorLevel::Ok,
                name: "auth/token",
                detail: "token not required in local_trust mode".to_string(),
            },
            DoctorFinding {
                ok: false,
                level: DoctorLevel::Fail,
                name: "tmux",
                detail: "tmux not found on PATH. Install with: brew install tmux (macOS) or apt install tmux (Debian/Ubuntu).".to_string(),
            },
            DoctorFinding {
                ok: false,
                level: DoctorLevel::Fail,
                name: "clawgs",
                detail: "clawgs not found at `clawgs`".to_string(),
            },
            DoctorFinding {
                ok: false,
                level: DoctorLevel::Fail,
                name: "data_dir",
                detail: "data dir not writable: permission denied".to_string(),
            },
        ]
    );
}

#[test]
fn doctor_preserves_exact_auth_failure_details() {
    let localtrust = cfg("0.0.0.0", AuthMode::LocalTrust, None);
    let tailnet = cfg("0.0.0.0", AuthMode::TailnetTrust, None);
    let token = cfg("127.0.0.1", AuthMode::Token, None);

    assert_eq!(
        run_doctor_checks(
            &localtrust,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        )[0],
        DoctorFinding {
            ok: false,
            level: DoctorLevel::Fail,
            name: "auth/bind",
            detail: "SWIMMERS_BIND=0.0.0.0 is non-loopback while AUTH_MODE=local_trust. This exposes the API to the network with no authentication. Bind to 127.0.0.1, use AUTH_MODE=tailnet_trust with a Tailscale bind address, or set AUTH_MODE=token AUTH_TOKEN=<secret>.".to_string(),
        }
    );
    assert_eq!(
        run_doctor_checks(
            &tailnet,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        )[0],
        DoctorFinding {
            ok: false,
            level: DoctorLevel::Fail,
            name: "auth/bind",
            detail: "SWIMMERS_BIND=0.0.0.0 is not a Tailscale address while AUTH_MODE=tailnet_trust. Bind to a Tailscale IP in 100.64.0.0/10 or fd7a:115c:a1e0::/48, or use AUTH_MODE=token AUTH_TOKEN=<secret> for non-tailnet exposure.".to_string(),
        }
    );
    assert_eq!(
        run_doctor_checks(
            &token,
            true,
            Ok("`clawgs defaults` ok".to_string()),
            Ok(PathBuf::from("/tmp")),
        )[1],
        DoctorFinding {
            ok: false,
            level: DoctorLevel::Fail,
            name: "auth/token",
            detail: "AUTH_MODE=token but AUTH_TOKEN is not set. Set AUTH_TOKEN=<secret>."
                .to_string(),
        }
    );
}

#[test]
fn doctor_flags_token_mode_without_token() {
    let c = cfg("127.0.0.1", AuthMode::Token, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Ok("`clawgs defaults` ok".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    let auth_token = findings.iter().find(|f| f.name == "auth/token").unwrap();
    assert!(!auth_token.ok);
    assert!(auth_token.detail.contains("AUTH_TOKEN"));
}

#[test]
fn doctor_remote_targets_warning_reports_counts_without_env_names() {
    let snapshot = DependencyHealthSnapshot::degraded(chrono::Utc::now(), "auth_env_missing")
        .with_detail("configured_targets", "2")
        .with_detail("swimmers_api_targets", "1")
        .with_detail("ssh_only_targets", "1")
        .with_detail("handoff_targets", "1")
        .with_detail("attach_hint_missing", "1")
        .with_detail("auth_env_missing", "1")
        .with_detail("targets_without_path_mappings", "1");

    let finding = doctor_remote_targets_finding(&snapshot);
    assert!(
        finding.ok,
        "remote targets are advisory for local operation"
    );
    assert_eq!(finding.level, DoctorLevel::Warn);
    assert_eq!(finding.name, "remote_targets");
    assert!(finding.detail.contains("status=degraded"));
    assert!(finding.detail.contains("configured=2"));
    assert!(finding.detail.contains("swimmers_api=1"));
    assert!(finding.detail.contains("ssh_only=1"));
    assert!(finding.detail.contains("handoff=1"));
    assert!(finding.detail.contains("attach_hint_missing=1"));
    assert!(finding.detail.contains("auth_env_missing=1"));
    assert!(!finding.detail.contains("SWIMMERS_REMOTE_TEST_TOKEN"));
}

#[test]
fn ssh_import_proposes_untrusted_ssh_only_targets_without_connecting() {
    let config = r#"
Host skillbox-devbox skillbox-short
  HostName skillbox-portfolio-devbox
  User skillbox

Host *.internal unsafe;alias !negated
  HostName ignored.internal

Match host *
  Host ignored-after-match
"#;

    let report = ssh_import_report_from_config("/tmp/ssh-config", config);

    assert_eq!(report.mode, "dry_run");
    assert!(!report.writes_files);
    assert!(!report.connects_to_hosts);
    assert_eq!(report.proposals.len(), 2);
    assert_eq!(report.proposals[0].id, "skillbox-devbox");
    assert_eq!(report.proposals[0].kind, "ssh_only");
    assert_eq!(report.proposals[0].trust, "untrusted");
    assert_eq!(report.proposals[0].attach_hint, "ssh skillbox-devbox");
    assert_eq!(
        report.proposals[0].bootstrap_hint,
        "ssh skillbox-devbox 'swimmers serve'"
    );
    assert_eq!(
        report.proposals[0].label,
        "skillbox@skillbox-portfolio-devbox"
    );
    assert_eq!(
        report.proposals[0].host_name.as_deref(),
        Some("skillbox-portfolio-devbox")
    );
    assert_eq!(report.proposals[0].user.as_deref(), Some("skillbox"));
    assert!(report.proposals[0].overlay_snippet.contains("dev_sanity"));
    assert!(report.proposals[0]
        .overlay_snippet
        .contains("kind: ssh_only"));
    assert_eq!(report.proposals[1].id, "skillbox-short");
}

#[test]
fn ssh_import_ignores_wildcards_negations_and_shell_unsafe_aliases() {
    let config = r#"
Host * !blocked devbox? unsafe;alias safe.alias user@host:22
  HostName example.test
"#;

    let proposals = ssh_import_proposals_from_config(config);

    assert_eq!(proposals.len(), 2);
    assert_eq!(proposals[0].id, "safe.alias");
    assert_eq!(proposals[1].id, "user@host:22");
    assert!(proposals
        .iter()
        .all(|proposal| proposal.trust == "untrusted"));
}

#[test]
fn doctor_reports_config_warning_without_failing() {
    let diagnostics = [ConfigDiagnostic {
        level: ConfigDiagnosticLevel::Warning,
        key: "PORT",
        message: "value \"bad\" is not a valid port; using default 3210".to_string(),
    }];
    let findings = config_diagnostic_findings(&diagnostics);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].ok);
    assert!(findings[0].detail.starts_with("warning: PORT"));
}

#[test]
fn doctor_reports_config_error_as_failure() {
    let diagnostics = [ConfigDiagnostic {
        level: ConfigDiagnosticLevel::Error,
        key: "AUTH_MODE",
        message: "unsupported value \"open\"".to_string(),
    }];
    let findings = config_diagnostic_findings(&diagnostics);
    assert_eq!(findings.len(), 1);
    assert!(!findings[0].ok);
    assert!(findings[0].detail.starts_with("error: AUTH_MODE"));
}

#[test]
fn doctor_flags_tailnet_trust_without_tailnet_bind() {
    let c = cfg("0.0.0.0", AuthMode::TailnetTrust, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Ok("`clawgs defaults` ok".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    let auth_bind = findings.iter().find(|f| f.name == "auth/bind").unwrap();
    assert!(!auth_bind.ok);
    assert!(auth_bind.detail.contains("tailnet_trust"));
}

#[test]
fn doctor_all_pass_with_tailnet_trust_tailscale_bind() {
    let c = cfg("100.86.253.9", AuthMode::TailnetTrust, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Ok("`clawgs defaults` ok".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    assert!(findings.iter().all(|f| f.ok));
}

#[test]
fn doctor_flags_missing_tmux() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let findings = run_doctor_checks(
        &c,
        false,
        Ok("`clawgs defaults` ok".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    let tmux = findings.iter().find(|f| f.name == "tmux").unwrap();
    assert!(!tmux.ok);
    assert!(tmux.detail.contains("brew install tmux"));
}

#[test]
fn doctor_flags_missing_clawgs() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Err("clawgs not found at `clawgs`".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    let clawgs = findings.iter().find(|f| f.name == "clawgs").unwrap();
    assert!(!clawgs.ok);
    assert!(clawgs.detail.contains("clawgs not found"));
}

#[test]
fn doctor_flags_unwritable_data_dir() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Ok("`clawgs defaults` ok".to_string()),
        Err("permission denied".to_string()),
    );
    let dd = findings.iter().find(|f| f.name == "data_dir").unwrap();
    assert!(!dd.ok);
    assert!(dd.detail.contains("permission denied"));
}

#[test]
fn check_data_dir_writable_creates_dir_writes_probe_and_cleans_it_up() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("nested").join("swimmers");

    let checked = check_data_dir_writable(&data_dir).expect("writable data dir");

    assert_eq!(checked, data_dir);
    assert!(checked.is_dir());
    assert!(!checked.join(".swimmers-doctor-probe").exists());
}

#[test]
fn create_data_dir_reports_create_dir_all_failures_with_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file_path = tmp.path().join("not-a-dir");
    std::fs::write(&file_path, b"file").expect("write regular file");

    let err = create_data_dir(&file_path).expect_err("regular file is not a directory");

    assert!(err.starts_with(&format!("create_dir_all({}) failed: ", file_path.display())));
}

#[test]
fn write_data_dir_probe_reports_write_failures_with_probe_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file_path = tmp.path().join("not-a-dir");
    std::fs::write(&file_path, b"file").expect("write regular file");
    let probe = file_path.join(".swimmers-doctor-probe");

    let err = write_data_dir_probe(&file_path).expect_err("probe under file fails");

    assert!(err.starts_with(&format!("write {} failed: ", probe.display())));
}

#[test]
fn doctor_finding_format_preserves_status_markers() {
    let fail = DoctorFinding {
        ok: false,
        level: DoctorLevel::Fail,
        name: "auth/bind",
        detail: "unsafe bind".to_string(),
    };
    let warn = DoctorFinding {
        ok: true,
        level: DoctorLevel::Warn,
        name: "config/env",
        detail: "warning: PORT".to_string(),
    };
    let ok = DoctorFinding {
        ok: true,
        level: DoctorLevel::Ok,
        name: "tmux",
        detail: "tmux found on PATH".to_string(),
    };

    assert_eq!(
        format_doctor_finding(&fail),
        "[FAIL] auth/bind: unsafe bind"
    );
    assert_eq!(
        format_doctor_finding(&warn),
        "[WARN] config/env: warning: PORT"
    );
    assert_eq!(format_doctor_finding(&ok), "[ok ] tmux: tmux found on PATH");
}

#[test]
fn doctor_finding_stream_routes_failures_and_warnings_to_stderr() {
    assert_eq!(
        doctor_output_stream(DoctorLevel::Fail),
        DoctorOutputStream::Stderr
    );
    assert_eq!(
        doctor_output_stream(DoctorLevel::Warn),
        DoctorOutputStream::Stderr
    );
    assert_eq!(
        doctor_output_stream(DoctorLevel::Ok),
        DoctorOutputStream::Stdout
    );
}

#[test]
fn doctor_all_pass_with_safe_config() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let findings = run_doctor_checks(
        &c,
        true,
        Ok("`clawgs defaults` ok".to_string()),
        Ok(PathBuf::from("/tmp")),
    );
    assert!(findings.iter().all(|f| f.ok));
}

#[test]
fn env_table_includes_all_documented_vars() {
    let rows = env_var_rows();
    for spec in ENV_VAR_SPECS {
        assert!(
            rows.iter().any(|r| r.name == spec.name),
            "missing {}",
            spec.name
        );
    }
}

#[test]
fn env_var_help_mirrors_every_spec() {
    // ENV_VAR_HELP is a hand-maintained literal; guard it against drifting
    // from the ENV_VAR_SPECS source of truth.
    for spec in ENV_VAR_SPECS {
        assert!(
            ENV_VAR_HELP.contains(spec.name),
            "ENV_VAR_HELP is missing a line for {}",
            spec.name
        );
    }
}

#[test]
fn env_table_redacts_secrets() {
    let load = ConfigLoad {
        config: Config {
            auth_token: Some("supersecret".to_string()),
            ..Config::default()
        },
        diagnostics: Vec::new(),
    };
    let rows = env_var_rows_from_load(&load);
    let auth = rows.iter().find(|r| r.name == "AUTH_TOKEN").unwrap();
    assert_eq!(auth.current, "***");
    assert_ne!(auth.current, "supersecret");
}

#[test]
fn env_table_exposes_native_script_root_override() {
    let _guard = crate::test_support::ENV_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let _native_root = EnvVarGuard::set(
        "SWIMMERS_NATIVE_SCRIPT_ROOT",
        std::ffi::OsString::from("/tmp/swimmers-native-scripts"),
    );
    let load = ConfigLoad {
        config: Config::default(),
        diagnostics: Vec::new(),
    };

    let rows = env_var_rows_from_load(&load);
    let native_root = rows
        .iter()
        .find(|r| r.name == "SWIMMERS_NATIVE_SCRIPT_ROOT")
        .expect("native script root row");

    assert_eq!(native_root.default, "(bundled)");
    assert_eq!(native_root.current, "/tmp/swimmers-native-scripts");
    assert_eq!(native_root.source, "env");
}

#[test]
fn env_table_exposes_personal_workflow_runtime_switch() {
    let load = ConfigLoad {
        config: Config {
            personal_workflows_enabled: true,
            ..Config::default()
        },
        diagnostics: Vec::new(),
    };

    let rows = env_var_rows_from_load(&load);
    let personal = rows
        .iter()
        .find(|r| r.name == "SWIMMERS_PERSONAL_WORKFLOWS")
        .expect("personal workflow row");

    assert_eq!(
        personal.default,
        bool_env_value(Config::default().personal_workflows_enabled)
    );
    assert_eq!(personal.current, "1");
}

#[test]
fn trust_startup_config_rejects_config_errors() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let diagnostics = [ConfigDiagnostic {
        level: ConfigDiagnosticLevel::Error,
        key: "AUTH_MODE",
        message: "unsupported value \"open\"".to_string(),
    }];
    let err = enforce_startup_config(&c, &diagnostics).unwrap_err();
    assert!(err.contains("invalid configuration"));
    assert!(err.contains("AUTH_MODE"));
}

#[test]
fn trust_startup_config_allows_warnings_when_bind_is_safe() {
    let c = cfg("127.0.0.1", AuthMode::LocalTrust, None);
    let diagnostics = [ConfigDiagnostic {
        level: ConfigDiagnosticLevel::Warning,
        key: "PORT",
        message: "using default 3210".to_string(),
    }];
    assert!(enforce_startup_config(&c, &diagnostics).is_ok());
}

fn fast_timeout() -> Duration {
    Duration::from_secs(2)
}

fn write_executable_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp).expect("create script");
        file.write_all(body.as_bytes()).expect("write script");
        file.flush().expect("flush script");
        file.sync_all().expect("sync script");
    }
    let mut perms = std::fs::metadata(&tmp).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmp, perms).expect("chmod");
    std::fs::rename(&tmp, &path).expect("rename script into place");
    path
}

fn write_plain_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write script");
    path
}

#[test]
fn check_clawgs_defaults_reports_not_found_for_missing_bin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("does-not-exist-clawgs");
    let err = check_clawgs_defaults_for_bin(missing.to_str().unwrap(), fast_timeout())
        .expect_err("missing bin must error");
    assert!(
        err.contains("clawgs not found"),
        "expected NotFound branch, got: {err}"
    );
}

#[test]
fn check_clawgs_defaults_reports_spawn_error_when_bin_is_not_executable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = write_plain_script(
        tmp.path(),
        "not-executable-clawgs",
        "#!/bin/sh\nprintf '{}\\n'\n",
    );
    let err = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
        .expect_err("non-executable bin must error");
    assert!(
        err.contains("failed to run"),
        "expected generic spawn branch, got: {err}"
    );
    assert!(
        err.contains("Set CLAWGS_BIN=/path/to/clawgs"),
        "spawn error should include CLAWGS_BIN guidance: {err}"
    );
}

#[test]
fn check_clawgs_defaults_reports_failure_when_bin_exits_non_zero() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = write_executable_script(
        tmp.path(),
        "fail-clawgs",
        "#!/bin/sh\nprintf 'bad defaults\\nsecond line\\n' >&2\nexit 7\n",
    );
    let err = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
        .expect_err("non-zero exit must error");
    assert!(
        err.contains("failed"),
        "expected non-zero failure branch, got: {err}"
    );
    assert!(
        err.contains("bad defaults"),
        "expected stderr detail branch, got: {err}"
    );
    assert!(
        !err.contains("second line"),
        "stderr detail should stay compact, got: {err}"
    );
}

#[test]
fn check_clawgs_defaults_reports_invalid_json_for_garbage_stdout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = write_executable_script(
        tmp.path(),
        "garbage-clawgs",
        "#!/bin/sh\nprintf 'not json\\n'\n",
    );
    let err = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
        .expect_err("invalid JSON must error");
    assert!(
        err.contains("invalid JSON"),
        "expected JSON parse branch, got: {err}"
    );
}

#[test]
fn check_clawgs_defaults_reports_missing_model_as_invalid_json() {
    let err = summarize_successful_clawgs_defaults(
        "clawgs",
        br#"{"backend":"claude","agent_prompt":"a","terminal_prompt":"t"}"#,
    )
    .expect_err("missing model must stay in invalid JSON bucket");
    assert!(
        err.contains("invalid JSON"),
        "expected JSON validation branch, got: {err}"
    );
    assert!(
        err.contains("missing field"),
        "expected missing-field detail, got: {err}"
    );
}

#[test]
fn check_clawgs_defaults_reports_timeout_for_slow_bin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = write_executable_script(tmp.path(), "slow-clawgs", "#!/bin/sh\nsleep 5\n");
    let err = check_clawgs_defaults_for_bin(script.to_str().unwrap(), Duration::from_millis(100))
        .expect_err("slow bin must time out");
    assert!(
        err.contains("timed out"),
        "expected timeout branch, got: {err}"
    );
}

#[test]
fn check_clawgs_defaults_formats_wait_and_collect_errors() {
    let inspect = format_clawgs_inspect_error(
        "clawgs",
        std::io::Error::new(ErrorKind::Other, "wait failed"),
    );
    assert_eq!(inspect, "failed to inspect `clawgs defaults`: wait failed");

    let collect = format_clawgs_collect_error(
        "clawgs",
        std::io::Error::new(ErrorKind::Other, "pipe failed"),
    );
    assert_eq!(
        collect,
        "failed to collect `clawgs defaults` output: pipe failed"
    );
}

#[test]
fn check_clawgs_defaults_returns_summary_for_valid_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = write_executable_script(
        tmp.path(),
        "ok-clawgs",
        "#!/bin/sh\nprintf '{\"model\":\"sonnet-4\",\"backend\":\"claude\",\
            \"agent_prompt\":\"a\",\"terminal_prompt\":\"t\"}\\n'\n",
    );
    let ok = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
        .expect("valid bin must succeed");
    assert!(
        ok.contains("backend=claude"),
        "summary missing backend: {ok}"
    );
    assert!(ok.contains("model=sonnet-4"), "summary missing model: {ok}");
}

#[test]
fn check_clawgs_defaults_uses_unknown_for_missing_backend() {
    let ok = summarize_successful_clawgs_defaults(
        "clawgs",
        br#"{"model":"m","agent_prompt":"a","terminal_prompt":"t"}"#,
    )
    .expect("missing backend should use serde default");
    assert!(
        ok.contains("backend=unknown"),
        "missing backend should fall back to 'unknown': {ok}"
    );
}

#[test]
fn check_clawgs_defaults_uses_unknown_for_blank_backend() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = write_executable_script(
        tmp.path(),
        "blank-backend-clawgs",
        "#!/bin/sh\nprintf '{\"model\":\"m\",\"backend\":\"   \",\
            \"agent_prompt\":\"a\",\"terminal_prompt\":\"t\"}\\n'\n",
    );
    let ok = check_clawgs_defaults_for_bin(script.to_str().unwrap(), fast_timeout())
        .expect("valid bin must succeed");
    assert!(
        ok.contains("backend=unknown"),
        "blank backend should fall back to 'unknown': {ok}"
    );
}
