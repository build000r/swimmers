use std::collections::HashMap;
#[cfg(test)]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command as ProcessCommand;
use std::sync::{LazyLock, Mutex};

use anyhow::{anyhow, Result};
#[cfg(test)]
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Duration;

use crate::session::actor::run_bounded_tmux_command;
use crate::tmux_target::{exact_pane_target, exact_session_target, TmuxTarget};
#[cfg(test)]
use crate::types::{AttentionGroupLayout, SessionSummary};
use crate::types::{GhosttyOpenMode, NativeDesktopApp, NativeDesktopOpenResponse};

mod attention_group;
mod ghostty_open;
mod host;
mod iterm;
mod osascript;
mod script_path;
mod support;
mod tmux_binary;

#[cfg(test)]
use attention_group::build_attention_group_attach_command;
use attention_group::ATTENTION_GROUP_SESSION_ID;
#[cfg(test)]
use attention_group::ATTENTION_GROUP_TMUX_NAME;
pub use attention_group::{
    attention_group_attach_command, clear_native_attention_group, open_native_attention_group,
};
use ghostty_open::open_or_focus_ghostty_session;
#[cfg(test)]
use ghostty_open::parse_osascript_output;
#[cfg(test)]
use ghostty_open::{
    cached_ghostty_preview_term_id, clear_ghostty_preview_term_cache,
    remember_ghostty_preview_term_id,
};
#[cfg(test)]
use host::host_is_loopback;
pub use iterm::open_or_focus_iterm_session;
#[cfg(test)]
use iterm::{build_iterm_attach_command, build_iterm_display_name, is_transient_iterm_open_error};
#[cfg(test)]
use osascript::NativeScriptError;
use osascript::{
    run_osascript_blocking_output, run_osascript_output, run_osascript_output_with_timeout,
    sanitize_osascript_text_arg, validate_osascript_script_arg,
};
use script_path::script_path_for_app;
#[cfg(test)]
use script_path::{materialize_bundled_script, resolve_script_path, unique_tmp_suffix};
pub use support::{
    default_ghostty_open_mode, default_native_app, script_dependency_health, support_for_host,
};
#[cfg(test)]
use tmux_binary::{find_binary_in_path_os, validate_tmux_binary, TMUX_BIN_ENV, TMUX_BIN_FALLBACKS};

const NATIVE_SCRIPT_ROOT_ENV: &str = "SWIMMERS_NATIVE_SCRIPT_ROOT";
const ITERM_SCRIPT_RELATIVE_PATH: &str = "scripts/iterm-focus.scpt";
const ITERM_SCRIPT_SOURCE: &str = include_str!("../../scripts/iterm-focus.scpt");
const GHOSTTY_SCRIPT_RELATIVE_PATH: &str = "scripts/ghostty-open.scpt";
const GHOSTTY_SCRIPT_SOURCE: &str = include_str!("../../scripts/ghostty-open.scpt");
const GHOSTTY_MANAGED_TITLE_PREFIX: &str = "swimmers-preview :: ";
const GHOSTTY_ATTENTION_MANAGED_TITLE_PREFIX: &str = "swimmers-attention :: ";
const GHOSTTY_MIN_APPLESCRIPT_VERSION: [u64; 3] = [1, 3, 0];
const GHOSTTY_MIN_APPLESCRIPT_VERSION_TEXT: &str = "1.3.0";
// 5s bounds AppleScript hangs while still allowing normal local automation latency.
const OSASCRIPT_TIMEOUT: Duration = Duration::from_secs(5);
const NATIVE_TMUX_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
static NATIVE_OPEN_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));
static SESSION_PANE_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static GHOSTTY_PREVIEW_TERM_IDS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl NativeDesktopApp {
    fn script_relative_path(self) -> &'static str {
        match self {
            Self::Iterm => ITERM_SCRIPT_RELATIVE_PATH,
            Self::Ghostty => GHOSTTY_SCRIPT_RELATIVE_PATH,
        }
    }

    fn bundled_script_source(self) -> &'static str {
        match self {
            Self::Iterm => ITERM_SCRIPT_SOURCE,
            Self::Ghostty => GHOSTTY_SCRIPT_SOURCE,
        }
    }
}

pub async fn open_native_session(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    session_id: &str,
    tmux_name: &str,
    tmux_target: &TmuxTarget,
    cwd: &str,
) -> Result<NativeDesktopOpenResponse> {
    match app {
        NativeDesktopApp::Iterm => {
            open_or_focus_iterm_session(session_id, tmux_name, tmux_target, cwd).await
        }
        NativeDesktopApp::Ghostty => {
            open_or_focus_ghostty_session(session_id, tmux_name, tmux_target, cwd, ghostty_mode)
                .await
        }
    }
}

fn ghostty_unavailable_reason() -> Option<String> {
    let output = match run_osascript_blocking_output(
        ["-e", "tell application \"Ghostty\" to get version"],
        "querying Ghostty version",
    ) {
        Ok(output) => output,
        Err(_) => return Some(ghostty_applescript_unavailable_message()),
    };

    if !output.status.success() {
        return Some(ghostty_applescript_unavailable_message());
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        return Some(ghostty_applescript_unavailable_message());
    }

    ghostty_version_requirement_error(&version)
}

fn ghostty_applescript_unavailable_message() -> String {
    format!(
        "Ghostty AppleScript unavailable. Install Ghostty {}+ and allow automation access.",
        GHOSTTY_MIN_APPLESCRIPT_VERSION_TEXT
    )
}

fn ghostty_version_requirement_error(version: &str) -> Option<String> {
    let parsed = match parse_version_triplet(version) {
        Some(parsed) => parsed,
        None => {
            return Some(format!(
                "Ghostty reported version {version:?}, but native AppleScript control requires Ghostty {}+.",
                GHOSTTY_MIN_APPLESCRIPT_VERSION_TEXT
            ));
        }
    };
    if parsed >= GHOSTTY_MIN_APPLESCRIPT_VERSION {
        return None;
    }

    Some(format!(
        "Ghostty {version} is installed, but native AppleScript control requires Ghostty {}+.",
        GHOSTTY_MIN_APPLESCRIPT_VERSION_TEXT
    ))
}

fn parse_version_triplet(version: &str) -> Option<[u64; 3]> {
    let mut parts = version
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some([major, minor, patch])
}

pub(super) fn resolve_tmux_binary() -> Result<PathBuf> {
    tmux_binary::resolve_tmux_binary()
}

fn cached_pane_id(session_id: &str) -> Option<String> {
    SESSION_PANE_CACHE.lock().unwrap().get(session_id).cloned()
}

fn remember_pane_id(session_id: &str, pane_id: Option<&str>) {
    let mut cache = SESSION_PANE_CACHE.lock().unwrap();
    match pane_id.filter(|value| !value.is_empty()) {
        Some(pane_id) => {
            cache.insert(session_id.to_string(), pane_id.to_string());
        }
        None => {
            cache.remove(session_id);
        }
    }
}

pub(super) async fn run_tmux_status(tmux_path: &Path, args: &[&str]) -> Result<()> {
    run_tmux_output(tmux_path, args).await.map(|_| ())
}

pub(super) async fn run_tmux_output(
    tmux_path: &Path,
    args: &[&str],
) -> Result<std::process::Output> {
    let output = run_bounded_tmux_command(
        tmux_path.as_os_str(),
        args,
        NATIVE_TMUX_COMMAND_TIMEOUT,
        "native",
    )
    .await
    .map_err(|err| anyhow!("failed to run tmux: {err}"))?;
    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(anyhow!(
        "tmux exited with {}{}",
        output.status,
        if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        }
    ))
}

async fn query_tmux_pane_metadata(
    tmux_path: &Path,
    tmux_name: &str,
    tmux_target: &TmuxTarget,
) -> Result<(Option<String>, Option<String>)> {
    let target = exact_pane_target(tmux_name);
    let args = tmux_target.command_args(&[
        "display-message",
        "-p",
        "-t",
        &target,
        "#{pane_id}\t#{pane_current_path}",
    ]);
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_bounded_tmux_command(
        tmux_path.as_os_str(),
        &args,
        NATIVE_TMUX_COMMAND_TIMEOUT,
        "display-message",
    )
    .await
    .map_err(|e| anyhow!("failed to run tmux display-message: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "tmux display-message returned a non-zero exit status".to_string()
        } else {
            stderr
        };
        return Err(anyhow!(message));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok((None, None));
    }

    let mut parts = stdout.splitn(2, '\t');
    let pane_id = parts.next().and_then(normalize_tmux_pane_id);
    let cwd = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok((pane_id, cwd))
}

fn cwd_basename(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    trimmed
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_tmux_pane_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.trim_start_matches('%').trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn build_ghostty_display_name(cwd: &str, tmux_name: &str) -> String {
    cwd_basename(cwd)
        .or_else(|| non_empty_trimmed(tmux_name).map(ToOwned::to_owned))
        .unwrap_or_else(|| "session".to_string())
}

pub(super) fn shell_quote_token(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/'))
    {
        return value.to_string();
    }

    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn build_ghostty_attach_command(
    tmux_name: &str,
    tmux_target: &TmuxTarget,
    tmux_path: &Path,
) -> String {
    let target_words = tmux_target
        .shell_words()
        .into_iter()
        .map(|word| shell_quote_token(&word))
        .collect::<Vec<_>>()
        .join(" ");
    let target_words = if target_words.is_empty() {
        String::new()
    } else {
        format!(" {target_words}")
    };
    format!(
        "exec {}{} attach-session -t {}",
        shell_quote_token(&tmux_path.to_string_lossy()),
        target_words,
        shell_quote_token(&exact_session_target(tmux_name))
    )
}

#[cfg(test)]
mod tests;
