use super::*;

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
