use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;

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
static NATIVE_OPEN_LOCK: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));
static SESSION_PANE_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
    let _guard = NATIVE_OPEN_LOCK.lock().await;
    let script = script_path();
    if !script.exists() {
        return Err(anyhow!("native iTerm script missing: {}", script.display()));
    }

    let tmux_path = resolve_tmux_binary()?;
    let known_pane_id = cached_pane_id(session_id);
    let result = run_open_or_focus_script(
        &script,
        session_id,
        tmux_name,
        &tmux_path,
        known_pane_id.as_deref(),
    )
    .await?;
    remember_pane_id(session_id, result.pane_id.as_deref());
    Ok(result)
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

async fn run_open_or_focus_script(
    script: &Path,
    session_id: &str,
    tmux_name: &str,
    tmux_path: &Path,
    known_pane_id: Option<&str>,
) -> Result<NativeDesktopOpenResponse> {
    let mut command = Command::new("osascript");
    command
        .arg(script)
        .arg(session_id)
        .arg(tmux_name)
        .arg(tmux_path);
    if let Some(pane_id) = known_pane_id.filter(|value| !value.is_empty()) {
        command.arg(pane_id);
    }

    let output = command
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
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex as StdMutex;
    use tempfile::tempdir;

    static TEST_ENV_LOCK: LazyLock<StdMutex<()>> = LazyLock::new(|| StdMutex::new(()));

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

        let focused = parse_osascript_output("focused|pane-2", "sess-2").unwrap();
        assert_eq!(focused.status, "focused");
        assert_eq!(focused.pane_id.as_deref(), Some("pane-2"));
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

    #[tokio::test]
    async fn open_or_focus_passes_cached_pane_id_on_repeat_calls() {
        let _env_guard = TEST_ENV_LOCK.lock().unwrap();
        remember_pane_id("sess-cache", None);

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
                "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$*\" >> \"{}\"\nknown=\"${{5-}}\"\nif [ -z \"$known\" ]; then\n  printf 'created|pane-1\\n'\nelse\n  printf 'focused|%s\\n' \"$known\"\nfi\n",
                log_path.display()
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

        let first = open_or_focus_iterm_session("sess-cache", "tmux-cache")
            .await
            .unwrap();
        let second = open_or_focus_iterm_session("sess-cache", "tmux-cache")
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
        let first_call = lines.next().unwrap();
        let second_call = lines.next().unwrap();
        assert!(first_call.contains(" sess-cache tmux-cache "));
        assert!(!first_call.ends_with(" pane-1"));
        assert!(second_call.ends_with(" pane-1"));

        remember_pane_id("sess-cache", None);
    }
}
