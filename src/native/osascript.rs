use std::process::{Command as ProcessCommand, Output, Stdio};
use std::time::Instant;

use anyhow::{anyhow, Result};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const OSASCRIPT_ARG_MAX_LEN: usize = 256;

#[derive(Debug, Error)]
pub(super) enum NativeScriptError {
    #[error("osascript timed out while {operation} after {timeout_ms}ms")]
    OsaScriptTimeout {
        operation: &'static str,
        timeout_ms: u128,
    },
    #[error("invalid osascript argument `{field}`: {reason}")]
    InvalidOsaScriptArg { field: &'static str, reason: String },
}

fn osascript_timeout_error(operation: &'static str, timeout_duration: Duration) -> anyhow::Error {
    anyhow::Error::new(NativeScriptError::OsaScriptTimeout {
        operation,
        timeout_ms: timeout_duration.as_millis(),
    })
}

pub(super) fn run_osascript_blocking_output<const N: usize>(
    args: [&str; N],
    operation: &'static str,
) -> Result<Output> {
    let mut child = ProcessCommand::new("osascript")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| anyhow!("failed to start osascript: {err}"))?;
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child
                    .wait_with_output()
                    .map_err(|err| anyhow!("failed to collect osascript output: {err}"));
            }
            Ok(None) => {
                if started.elapsed() >= super::OSASCRIPT_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(osascript_timeout_error(operation, super::OSASCRIPT_TIMEOUT));
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(err) => return Err(anyhow!("failed while waiting for osascript: {err}")),
        }
    }
}

pub(super) async fn run_osascript_output(
    command: &mut Command,
    operation: &'static str,
) -> Result<std::process::Output> {
    run_osascript_output_with_timeout(command, operation, super::OSASCRIPT_TIMEOUT).await
}

pub(super) async fn run_osascript_output_with_timeout(
    command: &mut Command,
    operation: &'static str,
    timeout_duration: Duration,
) -> Result<std::process::Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| anyhow!("failed to run osascript: {err}"))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let status = match timeout(timeout_duration, async {
        let wait_status = async {
            child
                .wait()
                .await
                .map_err(|err| anyhow!("failed to run osascript: {err}"))
        };
        let read_stdout = async {
            if let Some(pipe) = stdout_pipe.as_mut() {
                pipe.read_to_end(&mut stdout)
                    .await
                    .map_err(|err| anyhow!("failed to collect osascript output: {err}"))?;
            }
            Ok::<_, anyhow::Error>(())
        };
        let read_stderr = async {
            if let Some(pipe) = stderr_pipe.as_mut() {
                pipe.read_to_end(&mut stderr)
                    .await
                    .map_err(|err| anyhow!("failed to collect osascript output: {err}"))?;
            }
            Ok::<_, anyhow::Error>(())
        };
        let (status, _, _) = tokio::try_join!(wait_status, read_stdout, read_stderr)?;
        Ok::<_, anyhow::Error>(status)
    })
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(osascript_timeout_error(operation, timeout_duration));
        }
    };

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Defense-in-depth: strip bytes that could desync `parse_osascript_output`
/// (which splits on `|`) or corrupt downstream consumers like log lines and
/// terminal titles when request-borne strings round-trip back through them.
///
/// This is NOT shell or AppleScript escaping - both `osascript` invocations
/// pass these values via `Command::arg` (execve-style, no shell), and the
/// `.scpt` files read each argv slot as an opaque AppleScript text item.
pub(super) fn sanitize_osascript_text_arg(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\0' | '\n' | '\r' | '\t' | '|'))
        .collect()
}

pub(super) fn validate_osascript_script_arg(field: &'static str, value: &str) -> Result<()> {
    if let Some(reason) = invalid_osascript_script_arg_reason(field, value) {
        return Err(anyhow::Error::new(NativeScriptError::InvalidOsaScriptArg {
            field,
            reason,
        }));
    }
    Ok(())
}

fn invalid_osascript_script_arg_reason(field: &'static str, value: &str) -> Option<String> {
    osascript_arg_empty_error(value)
        .or_else(|| osascript_arg_length_error(value))
        .or_else(|| osascript_arg_newline_error(value))
        .or_else(|| osascript_arg_quote_error(field, value))
        .or_else(|| osascript_arg_character_error(field, value))
}

fn osascript_arg_empty_error(value: &str) -> Option<String> {
    value
        .trim()
        .is_empty()
        .then(|| "value cannot be empty".to_string())
}

fn osascript_arg_length_error(value: &str) -> Option<String> {
    (value.len() > OSASCRIPT_ARG_MAX_LEN).then(|| {
        format!(
            "value length {} exceeds maximum {}",
            value.len(),
            OSASCRIPT_ARG_MAX_LEN
        )
    })
}

fn osascript_arg_newline_error(value: &str) -> Option<String> {
    (value.contains('\n') || value.contains('\r')).then(|| "newlines are not allowed".to_string())
}

fn osascript_arg_quote_error(field: &'static str, value: &str) -> Option<String> {
    let contains_disallowed_quote =
        value.contains('"') || (!osascript_arg_allows_shell_quotes(field) && value.contains('\''));
    contains_disallowed_quote.then(|| "quotes are not allowed".to_string())
}

fn osascript_arg_character_error(field: &'static str, value: &str) -> Option<String> {
    value
        .chars()
        .find(|ch| !is_allowed_osascript_script_arg_char(field, *ch))
        .map(|invalid| format!("contains disallowed character `{invalid}`"))
}

fn osascript_arg_allows_shell_quotes(field: &'static str) -> bool {
    field == "attach_command"
}

fn is_allowed_osascript_script_arg_char(field: &'static str, ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(ch, '_' | '-' | '.' | '/' | ':' | ' ')
        || (osascript_arg_allows_shell_quotes(field) && matches!(ch, '\'' | '='))
        || (osascript_arg_allows_shell_quotes(field) && ch == '@')
}
