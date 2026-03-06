use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

use crate::types::{NativeDesktopOpenResponse, NativeDesktopStatusResponse};

const ITERM_APP_NAME: &str = "iTerm";
const ITERM_SCRIPT_RELATIVE_PATH: &str = "scripts/iterm-focus.scpt";
const TMUX_BIN_ENV: &str = "THRONGTERM_TMUX_BIN";
const TMUX_BIN_FALLBACKS: &[&str] = &[
    "/opt/homebrew/bin/tmux",
    "/usr/local/bin/tmux",
    "/usr/bin/tmux",
    "/bin/tmux",
];

pub fn support_for_host(host: &str) -> NativeDesktopStatusResponse {
    let mut response = NativeDesktopStatusResponse {
        supported: false,
        platform: Some(std::env::consts::OS.to_string()),
        app: Some(ITERM_APP_NAME.to_string()),
        reason: None,
    };

    if !cfg!(target_os = "macos") {
        response.reason = Some("native iTerm control is only supported on macOS".to_string());
        return response;
    }

    if !host_is_loopback(host) {
        response.reason = Some("native iTerm control is only available from localhost".to_string());
        return response;
    }

    let script_path = script_path();
    if !script_path.exists() {
        response.reason = Some(format!(
            "native iTerm script missing: {}",
            script_path.display()
        ));
        return response;
    }

    response.supported = true;
    response.reason = None;
    response
}

pub async fn open_or_focus_iterm_session(
    session_id: &str,
    tmux_name: &str,
) -> Result<NativeDesktopOpenResponse> {
    let script = script_path();
    if !script.exists() {
        return Err(anyhow!("native iTerm script missing: {}", script.display()));
    }

    let tmux_path = resolve_tmux_binary()?;
    let output = Command::new("osascript")
        .arg(&script)
        .arg(session_id)
        .arg(tmux_name)
        .arg(&tmux_path)
        .output()
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
    parse_osascript_output(stdout.trim(), session_id)
}

fn script_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(ITERM_SCRIPT_RELATIVE_PATH)
}

fn resolve_tmux_binary() -> Result<PathBuf> {
    if let Some(configured) = std::env::var_os(TMUX_BIN_ENV) {
        return validate_tmux_binary(PathBuf::from(configured))
            .with_context(|| format!("{} must point to an absolute tmux binary", TMUX_BIN_ENV));
    }

    if let Some(path_from_env) = find_binary_in_path("tmux") {
        return Ok(path_from_env);
    }

    for fallback in TMUX_BIN_FALLBACKS {
        let candidate = PathBuf::from(fallback);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(anyhow!(
        "unable to locate tmux; set {} to an absolute tmux binary path",
        TMUX_BIN_ENV
    ))
}

fn validate_tmux_binary(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(anyhow!("path is not absolute"));
    }
    if !path.is_file() {
        return Err(anyhow!("binary not found at {}", path.display()));
    }
    Ok(path)
}

fn find_binary_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    find_binary_in_path_os(binary, &path)
}

fn find_binary_in_path_os(binary: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_absolute() && candidate.is_file())
}

fn parse_osascript_output(stdout: &str, session_id: &str) -> Result<NativeDesktopOpenResponse> {
    let mut parts = stdout
        .split_once('|')
        .map(|(status, pane_id)| vec![status, pane_id])
        .unwrap_or_else(|| stdout.split('\t').collect());
    if parts.is_empty() {
        return Err(anyhow!("osascript returned an empty response"));
    }
    let status = parts.remove(0).trim().to_string();
    let pane_id = parts
        .first()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    match status.as_str() {
        "created" | "focused" => Ok(NativeDesktopOpenResponse {
            session_id: session_id.to_string(),
            status,
            pane_id,
        }),
        other if !other.is_empty() => Err(anyhow!("unexpected osascript status: {other}")),
        _ => Err(anyhow!("osascript returned an empty response")),
    }
}

fn host_is_loopback(host: &str) -> bool {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return false;
    }

    let without_port = if let Some(stripped) = trimmed.strip_prefix('[') {
        stripped
            .split(']')
            .next()
            .unwrap_or(trimmed)
            .trim()
            .to_string()
    } else {
        trimmed
            .split(':')
            .next()
            .unwrap_or(trimmed)
            .trim()
            .to_string()
    };

    matches!(
        without_port.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0:0:0:0:0:0:0:1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn host_loopback_accepts_local_variants() {
        assert!(host_is_loopback("localhost:3210"));
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("[::1]:3210"));
    }

    #[test]
    fn host_loopback_rejects_remote_variants() {
        assert!(!host_is_loopback("100.101.1.2:3210"));
        assert!(!host_is_loopback("example.local:3210"));
    }

    #[test]
    fn parse_osascript_output_accepts_expected_statuses() {
        let created = parse_osascript_output("created\tpane-1", "sess-1").unwrap();
        assert_eq!(created.status, "created");
        assert_eq!(created.pane_id.as_deref(), Some("pane-1"));

        let focused = parse_osascript_output("focused", "sess-2").unwrap();
        assert_eq!(focused.status, "focused");
        assert_eq!(focused.pane_id, None);
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
}
