use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;
use tokio::time::{sleep, Duration};

use crate::tmux_target::exact_session_target;
use crate::types::{NativeDesktopApp, NativeDesktopOpenResponse};

const ITERM_OPEN_RETRY_ATTEMPTS: usize = 2;
const ITERM_OPEN_RETRY_DELAY_MS: u64 = 150;
const DEFAULT_ITERM_SESSION_NAME: &str = "Swimmers";

struct ItermOpenContext {
    script: PathBuf,
    attach_command: String,
    display_name: String,
    known_pane_id: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ItermTmuxMetadata {
    pane_id: Option<String>,
    cwd: Option<String>,
}

pub async fn open_or_focus_iterm_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
) -> Result<NativeDesktopOpenResponse> {
    let _guard = super::NATIVE_OPEN_LOCK.lock().await;
    let context = prepare_iterm_open_context(session_id, tmux_name, cwd).await?;
    open_iterm_with_retries(session_id, tmux_name, &context).await
}

async fn prepare_iterm_open_context(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
) -> Result<ItermOpenContext> {
    let script = super::script_path_for_app(NativeDesktopApp::Iterm)?;
    ensure_iterm_script_exists(&script)?;

    let tmux_path = super::resolve_tmux_binary()?;
    let attach_command = build_iterm_attach_command(tmux_name, &tmux_path);
    let metadata = load_iterm_tmux_metadata(&tmux_path, tmux_name).await;
    let display_name = display_name_from_iterm_tmux_metadata(cwd, tmux_name, &metadata);
    let known_pane_id = super::cached_pane_id(session_id);

    Ok(ItermOpenContext {
        script,
        attach_command,
        display_name,
        known_pane_id,
    })
}

fn ensure_iterm_script_exists(script: &Path) -> Result<()> {
    if script.exists() {
        Ok(())
    } else {
        Err(anyhow!("native iTerm script missing: {}", script.display()))
    }
}

async fn load_iterm_tmux_metadata(tmux_path: &Path, tmux_name: &str) -> ItermTmuxMetadata {
    let (pane_id, cwd) = super::query_tmux_pane_metadata(tmux_path, tmux_name)
        .await
        .unwrap_or((None, None));
    ItermTmuxMetadata { pane_id, cwd }
}

fn display_name_from_iterm_tmux_metadata(
    cwd: &str,
    tmux_name: &str,
    metadata: &ItermTmuxMetadata,
) -> String {
    build_iterm_display_name(
        metadata.cwd.as_deref().unwrap_or(cwd),
        tmux_name,
        metadata.pane_id.as_deref(),
    )
}

async fn open_iterm_with_retries(
    session_id: &str,
    tmux_name: &str,
    context: &ItermOpenContext,
) -> Result<NativeDesktopOpenResponse> {
    for attempt in 0..ITERM_OPEN_RETRY_ATTEMPTS {
        match run_iterm_open_script(
            &context.script,
            session_id,
            tmux_name,
            &context.attach_command,
            &context.display_name,
            context.known_pane_id.as_deref(),
        )
        .await
        {
            Ok(result) => return Ok(remember_iterm_open_result(session_id, result)),
            Err(err) if should_retry_iterm_open(attempt, &err) => {
                sleep(Duration::from_millis(ITERM_OPEN_RETRY_DELAY_MS)).await;
            }
            Err(err) => return Err(err),
        }
    }

    unreachable!("native iTerm open loop should always return or error")
}

fn remember_iterm_open_result(
    session_id: &str,
    result: NativeDesktopOpenResponse,
) -> NativeDesktopOpenResponse {
    super::remember_pane_id(session_id, result.pane_id.as_deref());
    result
}

fn should_retry_iterm_open(attempt: usize, err: &anyhow::Error) -> bool {
    attempt + 1 < ITERM_OPEN_RETRY_ATTEMPTS && is_transient_iterm_open_error(err)
}

async fn run_iterm_open_script(
    script: &Path,
    session_id: &str,
    tmux_name: &str,
    attach_command: &str,
    display_name: &str,
    known_pane_id: Option<&str>,
) -> Result<NativeDesktopOpenResponse> {
    run_iterm_open_script_with_timeout(
        script,
        session_id,
        tmux_name,
        attach_command,
        display_name,
        known_pane_id,
        super::OSASCRIPT_TIMEOUT,
    )
    .await
}

async fn run_iterm_open_script_with_timeout(
    script: &Path,
    session_id: &str,
    tmux_name: &str,
    attach_command: &str,
    display_name: &str,
    known_pane_id: Option<&str>,
    timeout_duration: Duration,
) -> Result<NativeDesktopOpenResponse> {
    let args = iterm_open_script_args(
        session_id,
        tmux_name,
        attach_command,
        display_name,
        known_pane_id,
    )?;
    let mut command = Command::new("osascript");
    command.arg(script).args(&args);

    let output = super::run_osascript_output_with_timeout(
        &mut command,
        "opening/focusing iTerm session",
        timeout_duration,
    )
    .await
    .with_context(|| format!("failed to run {}", script.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "osascript returned a non-zero exit status".to_string()
        } else {
            stderr
        };
        return Err(anyhow!(message));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    super::ghostty_open::parse_osascript_output(stdout.trim(), session_id)
}

fn iterm_open_script_args(
    session_id: &str,
    tmux_name: &str,
    attach_command: &str,
    display_name: &str,
    known_pane_id: Option<&str>,
) -> Result<Vec<String>> {
    super::validate_osascript_script_arg("tmux_name", tmux_name)?;
    super::validate_osascript_script_arg("attach_command", attach_command)?;

    let mut args = vec![
        super::sanitize_osascript_text_arg(session_id),
        tmux_name.to_string(),
        attach_command.to_string(),
        super::sanitize_osascript_text_arg(display_name),
    ];
    if let Some(pane_id) = known_pane_id.filter(|value| !value.is_empty()) {
        args.push(pane_id.to_string());
    }
    Ok(args)
}

pub(super) fn build_iterm_display_name(
    cwd: &str,
    tmux_name: &str,
    tmux_pane_id: Option<&str>,
) -> String {
    let target_name = super::cwd_basename(cwd)
        .or_else(|| super::non_empty_trimmed(tmux_name).map(ToOwned::to_owned))
        .unwrap_or_else(|| DEFAULT_ITERM_SESSION_NAME.to_string());

    match super::normalize_tmux_pane_id(tmux_pane_id.unwrap_or_default()) {
        Some(pane_id) => format!("{pane_id} {target_name}"),
        None => target_name,
    }
}

pub(super) fn build_iterm_attach_command(tmux_name: &str, tmux_path: &Path) -> String {
    format!(
        "exec {} attach-session -t {}",
        super::shell_quote_token(&tmux_path.to_string_lossy()),
        super::shell_quote_token(&exact_session_target(tmux_name))
    )
}

pub(super) fn is_transient_iterm_open_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    (message.contains("session 1 of missing value") && message.contains("(-1728)"))
        || (message.contains("unable to resolve iTerm session after tab creation")
            && message.contains("(-2700)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_iterm_open_script_args_sanitize_text_fields_and_keep_script_args() {
        let args = iterm_open_script_args(
            "sess|\n1",
            "team:api",
            "exec /tmp/tmux attach-session -t '=team:api'",
            "7 repo\tname",
            Some("pane-1"),
        )
        .unwrap();

        assert_eq!(
            args,
            vec![
                "sess1",
                "team:api",
                "exec /tmp/tmux attach-session -t '=team:api'",
                "7 reponame",
                "pane-1",
            ]
        );
    }

    #[test]
    fn native_iterm_open_script_args_omit_blank_known_pane_id() {
        let args = iterm_open_script_args(
            "sess-1",
            "team-api",
            "exec /tmp/tmux attach-session -t '=team-api'",
            "repo",
            Some(""),
        )
        .unwrap();

        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "sess-1");
        assert_eq!(args[3], "repo");
    }

    #[test]
    fn native_iterm_display_name_prefers_tmux_metadata_over_fallback_cwd() {
        let metadata = ItermTmuxMetadata {
            pane_id: Some("%9".to_string()),
            cwd: Some("/Users/b/repos/from-tmux".to_string()),
        };

        assert_eq!(
            display_name_from_iterm_tmux_metadata(
                "/Users/b/repos/fallback",
                "codex-20260302-162713",
                &metadata,
            ),
            "9 from-tmux"
        );
    }

    #[test]
    fn native_iterm_display_name_falls_back_to_request_cwd_without_metadata() {
        assert_eq!(
            display_name_from_iterm_tmux_metadata(
                "/Users/b/repos/fallback",
                "codex-20260302-162713",
                &ItermTmuxMetadata::default(),
            ),
            "fallback"
        );
    }

    #[test]
    fn native_iterm_retry_decision_stops_after_last_attempt() {
        let err = anyhow!(
            "/tmp/iterm-focus.scpt: execution error: Can't get session 1 of missing value. (-1728)"
        );

        assert!(should_retry_iterm_open(0, &err));
        assert!(!should_retry_iterm_open(1, &err));
    }

    #[test]
    fn native_iterm_retry_decision_rejects_non_transient_errors() {
        let err = anyhow!("unexpected osascript status: broken");

        assert!(!should_retry_iterm_open(0, &err));
    }
}
