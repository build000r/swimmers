use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::warn;
use uuid::Uuid;

use crate::launcher::{self, SpawnToolLauncher};
use crate::session::actor::{ActorHandle, SessionCommand};
use crate::types::SpawnTool;

const INITIAL_REQUEST_INPUT_DELAY: Duration = Duration::from_millis(200);
const PRELAUNCH_PROMPT_CLEANUP_DELAY: Duration = Duration::from_secs(30);

pub(super) fn current_working_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

pub(super) fn normalize_initial_request(initial_request: Option<String>) -> Option<String> {
    initial_request.and_then(|request| {
        let trimmed = request.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(super) fn normalize_requested_tmux_name(requested_name: Option<String>) -> Option<String> {
    requested_name.and_then(|name| {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(super) fn initial_tool_name(spawn_tool: Option<&SpawnTool>) -> Option<String> {
    spawn_tool.map(|tool| {
        crate::types::detect_tool_name(tool.command())
            .unwrap_or(tool.command())
            .to_string()
    })
}

pub(super) fn initial_request_delay(
    spawn_tool: Option<SpawnTool>,
    initial_request: Option<&String>,
) -> Duration {
    if spawn_tool.is_some() && initial_request.is_some() {
        INITIAL_REQUEST_INPUT_DELAY
    } else {
        Duration::ZERO
    }
}

pub(super) fn build_initial_request_input(initial_request: &str) -> Vec<u8> {
    let mut input = initial_request.as_bytes().to_vec();
    input.push(b'\r');
    input
}

pub(super) fn spawn_tool_consumes_initial_request(_tool: SpawnTool) -> bool {
    true
}

#[derive(Debug, Default)]
pub(super) struct PreparedSpawnToolCommand {
    pub(super) command: String,
    pub(super) cleanup_paths: Vec<PathBuf>,
}

impl PreparedSpawnToolCommand {
    fn new(command: String) -> Self {
        Self {
            command,
            cleanup_paths: Vec::new(),
        }
    }

    fn with_prompt_file(
        initial_request: &str,
        command: impl FnOnce(&str) -> String,
    ) -> io::Result<Self> {
        let prompt_path = write_spawn_prompt_file(initial_request)?;
        let prompt_path_arg = shell_single_quote(&prompt_path.to_string_lossy());
        Ok(Self {
            command: command(&prompt_path_arg),
            cleanup_paths: vec![prompt_path],
        })
    }
}

pub(super) fn schedule_prelaunch_file_cleanup(paths: Vec<PathBuf>) {
    schedule_prelaunch_file_cleanup_after(paths, PRELAUNCH_PROMPT_CLEANUP_DELAY);
}

pub(super) fn schedule_prelaunch_file_cleanup_after(paths: Vec<PathBuf>, delay: Duration) {
    if paths.is_empty() {
        return;
    }

    if delay.is_zero() {
        cleanup_prelaunch_files_now(&paths);
        return;
    }

    if let Err(err) = std::thread::Builder::new()
        .name("swimmers-prelaunch-cleanup".to_string())
        .spawn(move || {
            std::thread::sleep(delay);
            cleanup_prelaunch_files_now(&paths);
        })
    {
        warn!(
            "failed to schedule pre-launch prompt cleanup; leaving prompt file for OS temp cleanup: {}",
            err
        );
    }
}

fn cleanup_prelaunch_files_now(paths: &[PathBuf]) {
    for path in paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => warn!(
                path = %path.display(),
                "failed to remove pre-launch prompt file after session spawn failure: {}",
                err
            ),
        }
    }
}

pub(super) fn prepare_spawn_tool_command(
    tool: SpawnTool,
    cwd: Option<&str>,
    initial_request: Option<&str>,
) -> PreparedSpawnToolCommand {
    let launcher = SpawnToolLauncher::from_env(tool);
    prepare_spawn_tool_command_with_launcher(tool, cwd, initial_request, launcher)
}

#[cfg(test)]
pub(super) fn build_spawn_tool_command(
    tool: SpawnTool,
    cwd: Option<&str>,
    initial_request: Option<&str>,
) -> String {
    prepare_spawn_tool_command(tool, cwd, initial_request).command
}

#[cfg(test)]
pub(super) fn build_spawn_tool_command_with_launcher(
    tool: SpawnTool,
    cwd: Option<&str>,
    initial_request: Option<&str>,
    launcher: SpawnToolLauncher,
) -> String {
    prepare_spawn_tool_command_with_launcher(tool, cwd, initial_request, launcher).command
}

fn prepare_spawn_tool_command_with_launcher(
    tool: SpawnTool,
    cwd: Option<&str>,
    initial_request: Option<&str>,
    launcher: SpawnToolLauncher,
) -> PreparedSpawnToolCommand {
    let Some(initial_request) = initial_request else {
        return PreparedSpawnToolCommand::new(launcher.shell_program());
    };
    if !spawn_tool_consumes_initial_request(tool) {
        return PreparedSpawnToolCommand::new(launcher.shell_program());
    }

    prepare_spawn_tool_command_with_initial_request(tool, cwd, initial_request, launcher)
}

fn prepare_spawn_tool_command_with_initial_request(
    tool: SpawnTool,
    cwd: Option<&str>,
    initial_request: &str,
    launcher: SpawnToolLauncher,
) -> PreparedSpawnToolCommand {
    if tool == SpawnTool::Codex {
        return build_codex_spawn_command_or_fallback(tool, initial_request);
    }
    if tool == SpawnTool::Grok {
        return build_grok_spawn_command_or_fallback(cwd, initial_request, launcher);
    }
    PreparedSpawnToolCommand::new(format!(
        "{} {}",
        tool.command(),
        shell_single_quote(initial_request)
    ))
}

pub(super) fn wrap_spawn_tool_command_for_tmux(command: &str) -> String {
    format!("{{ {command}; }}; exec \"${{SHELL:-/bin/sh}}\"")
}

fn build_codex_spawn_command_or_fallback(
    tool: SpawnTool,
    initial_request: &str,
) -> PreparedSpawnToolCommand {
    match build_codex_prompt_file_command(initial_request) {
        Ok(command) => command,
        Err(err) => {
            warn!(
                tool = ?tool,
                "failed to create prompt file for spawn command; starting without initial request: {}",
                err
            );
            PreparedSpawnToolCommand::new(tool.command().to_string())
        }
    }
}

fn build_grok_spawn_command_or_fallback(
    cwd: Option<&str>,
    initial_request: &str,
    launcher: SpawnToolLauncher,
) -> PreparedSpawnToolCommand {
    let fallback_program = launcher.shell_program();
    match build_grok_prompt_file_command(cwd, initial_request, launcher) {
        Ok(command) => command,
        Err(err) => {
            warn!(
                "failed to create Grok prompt file for spawn command; starting without initial request: {}",
                err
            );
            PreparedSpawnToolCommand::new(fallback_program)
        }
    }
}

fn build_grok_prompt_file_command(
    cwd: Option<&str>,
    initial_request: &str,
    launcher: SpawnToolLauncher,
) -> io::Result<PreparedSpawnToolCommand> {
    PreparedSpawnToolCommand::with_prompt_file(initial_request, |prompt_path_arg| {
        let cwd_arg = cwd
            .map(|cwd| format!(" --cwd {}", shell_single_quote(cwd)))
            .unwrap_or_default();
        let cleanup_command = prompt_file_cleanup_shell_command();
        format!(
        "prompt_file={prompt_path_arg}; if [ -r \"$prompt_file\" ]; then {} --prompt-file \"$prompt_file\"{cwd_arg} --always-approve --no-alt-screen; launch_status=$?; {cleanup_command}; test \"$launch_status\" -eq 0; else {cleanup_command}; echo 'swimmers: failed to read Grok initial request' >&2; false; fi",
        launcher.shell_program()
        )
    })
}

fn prompt_file_cleanup_shell_command() -> &'static str {
    "if [ -x /bin/rm ]; then /bin/rm -f \"$prompt_file\"; elif [ -x /usr/bin/rm ]; then /usr/bin/rm -f \"$prompt_file\"; else rm -f \"$prompt_file\"; fi"
}

fn build_codex_prompt_file_command(initial_request: &str) -> io::Result<PreparedSpawnToolCommand> {
    PreparedSpawnToolCommand::with_prompt_file(initial_request, |prompt_path| {
        format!(
        "prompt_file={prompt_path}; if prompt=\"$(cat \"$prompt_file\")\"; then rm -f \"$prompt_file\"; if command -v caam >/dev/null 2>&1; then caam run codex -- \"$prompt\" || {{ echo 'swimmers: caam codex launch failed; falling back to raw codex' >&2; if command -v codex-raw >/dev/null 2>&1; then codex-raw \"$prompt\"; else command codex \"$prompt\"; fi; }}; else command codex \"$prompt\"; fi; else rm -f \"$prompt_file\"; echo 'swimmers: failed to read initial request' >&2; false; fi"
        )
    })
}

fn write_spawn_prompt_file(initial_request: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("swimmers-initial-requests");
    prepare_private_prompt_dir(&dir)?;
    let prompt_path = dir.join(format!("{}.prompt.txt", Uuid::new_v4()));
    let mut file = create_private_prompt_file(&prompt_path)?;
    file.write_all(initial_request.as_bytes())?;
    Ok(prompt_path)
}

fn prepare_private_prompt_dir(path: &Path) -> io::Result<()> {
    launcher::prepare_private_dir(path)
}

pub(super) fn shell_single_quote(value: &str) -> String {
    launcher::shell_single_quote(value)
}

fn create_private_prompt_file(path: &Path) -> io::Result<fs::File> {
    launcher::create_private_file(path)
}

pub(super) fn enqueue_initial_request_input(
    handle: ActorHandle,
    session_id: String,
    tmux_name: String,
    initial_request: String,
    delay: Duration,
) {
    tokio::spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }

        if let Err(e) = handle
            .send(SessionCommand::WriteInput(build_initial_request_input(
                &initial_request,
            )))
            .await
        {
            warn!(
                session_id = %session_id,
                tmux_name = %tmux_name,
                "failed to enqueue initial request input: {}",
                e
            );
        }
    });
}
