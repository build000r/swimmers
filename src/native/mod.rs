use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, Duration};

use crate::types::{NativeDesktopOpenResponse, NativeDesktopStatusResponse};

const ITERM_APP_NAME: &str = "iTerm";
const ITERM_SCRIPT_RELATIVE_PATH: &str = "scripts/iterm-focus.scpt";
const ITERM_SCROLLBACK_PREFILL_LINES: usize = 2000;
const ITERM_OPEN_RETRY_ATTEMPTS: usize = 2;
const ITERM_OPEN_RETRY_DELAY_MS: u64 = 150;
const DEFAULT_ITERM_SESSION_NAME: &str = "Throngterm";
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
    cwd: &str,
) -> Result<NativeDesktopOpenResponse> {
    let _guard = NATIVE_OPEN_LOCK.lock().await;
    let script = script_path();
    if !script.exists() {
        return Err(anyhow!("native iTerm script missing: {}", script.display()));
    }

    let tmux_path = resolve_tmux_binary()?;
    let attach_command = build_iterm_attach_command(tmux_name, &tmux_path);
    let (tmux_pane_id, tmux_cwd) = query_tmux_pane_metadata(&tmux_path, tmux_name)
        .await
        .unwrap_or((None, None));
    let display_name = build_iterm_display_name(
        tmux_cwd.as_deref().unwrap_or(cwd),
        tmux_name,
        tmux_pane_id.as_deref(),
    );
    let known_pane_id = cached_pane_id(session_id);
    for attempt in 0..ITERM_OPEN_RETRY_ATTEMPTS {
        match run_open_or_focus_script(
            &script,
            session_id,
            tmux_name,
            &attach_command,
            &display_name,
            known_pane_id.as_deref(),
        )
        .await
        {
            Ok(result) => {
                remember_pane_id(session_id, result.pane_id.as_deref());
                return Ok(result);
            }
            Err(err)
                if attempt + 1 < ITERM_OPEN_RETRY_ATTEMPTS
                    && is_transient_iterm_missing_session_error(&err) =>
            {
                sleep(Duration::from_millis(ITERM_OPEN_RETRY_DELAY_MS)).await;
            }
            Err(err) => return Err(err),
        }
    }

    unreachable!("native iTerm open loop should always return or error")
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
    attach_command: &str,
    display_name: &str,
    known_pane_id: Option<&str>,
) -> Result<NativeDesktopOpenResponse> {
    let mut command = Command::new("osascript");
    command
        .arg(script)
        .arg(session_id)
        .arg(tmux_name)
        .arg(attach_command)
        .arg(display_name);
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

async fn query_tmux_pane_metadata(
    tmux_path: &Path,
    tmux_name: &str,
) -> Result<(Option<String>, Option<String>)> {
    let output = Command::new(tmux_path)
        .args([
            "display-message",
            "-p",
            "-t",
            tmux_name,
            "#{pane_id}\t#{pane_current_path}",
        ])
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .output()
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

fn build_iterm_display_name(cwd: &str, tmux_name: &str, tmux_pane_id: Option<&str>) -> String {
    let target_name = cwd_basename(cwd)
        .or_else(|| non_empty_trimmed(tmux_name).map(ToOwned::to_owned))
        .unwrap_or_else(|| DEFAULT_ITERM_SESSION_NAME.to_string());

    match normalize_tmux_pane_id(tmux_pane_id.unwrap_or_default()) {
        Some(pane_id) => format!("{pane_id} {target_name}"),
        None => target_name,
    }
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

fn build_iterm_attach_command(tmux_name: &str, tmux_path: &Path) -> String {
    let tmux_name = shell_single_quote(tmux_name);
    let tmux_path = shell_single_quote(tmux_path.to_string_lossy().as_ref());

    format!(
        "{tmux_path} capture-pane -p -J -S -{lines} -t {tmux_name} 2>/dev/null || true; \
printf '\\033[H\\033[2J'; exec {tmux_path} attach-session -t {tmux_name}",
        lines = ITERM_SCROLLBACK_PREFILL_LINES
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn is_transient_iterm_missing_session_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("session 1 of missing value") && message.contains("(-1728)")
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

    #[test]
    fn build_iterm_attach_command_prefills_scrollback_before_attach() {
        let command = build_iterm_attach_command("main", Path::new("/opt/homebrew/bin/tmux"));

        assert!(command.contains("capture-pane -p -J -S -2000 -t 'main'"));
        assert!(command.contains("2>/dev/null || true"));
        assert!(command.contains("printf '\\033[H\\033[2J'"));
        assert!(command.ends_with("exec '/opt/homebrew/bin/tmux' attach-session -t 'main'"));
    }

    #[test]
    fn build_iterm_attach_command_shell_quotes_tmux_values() {
        let command =
            build_iterm_attach_command("team's session", Path::new("/tmp/tmux builds/tmux"));

        assert!(command.contains("-t 'team'\"'\"'s session'"));
        assert!(command.contains("exec '/tmp/tmux builds/tmux' attach-session"));
    }

    #[test]
    fn build_iterm_display_name_prefers_normalized_pane_id_and_cwd_basename() {
        assert_eq!(
            build_iterm_display_name(
                "/Users/b/repos/throngterm/",
                "codex-20260302-162713",
                Some("%12")
            ),
            "12 throngterm"
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
            build_iterm_display_name("/Users/b/repos/throngterm", "codex-20260302-162713", None),
            "throngterm"
        );
    }

    #[test]
    fn detects_transient_missing_session_iterm_errors() {
        let retryable = anyhow!(
            "/tmp/iterm-focus.scpt: execution error: Can't get session 1 of missing value. (-1728)"
        );
        assert!(is_transient_iterm_missing_session_error(&retryable));

        let other = anyhow!("unexpected osascript status: created");
        assert!(!is_transient_iterm_missing_session_error(&other));
    }

    #[tokio::test]
    async fn open_or_focus_passes_cached_pane_id_on_repeat_calls() {
        let _env_guard = TEST_ENV_LOCK.lock().unwrap();
        remember_pane_id("sess-cache", None);

        let temp = tempdir().unwrap();
        let fake_bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();

        let fake_tmux = fake_bin_dir.join("tmux");
        std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%12\\t/Users/b/repos/throngterm\\n'\n  exit 0\nfi\nexit 0\n",
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
        assert!(first_call[3].contains("capture-pane -p -J -S -2000 -t 'tmux-cache'"));
        assert_eq!(first_call[4], "12 throngterm");
        assert_eq!(first_call.len(), 5);
        assert_eq!(second_call[4], "12 throngterm");
        assert_eq!(second_call[5], "pane-1");

        remember_pane_id("sess-cache", None);
    }

    #[tokio::test]
    async fn open_or_focus_retries_transient_missing_session_error() {
        let _env_guard = TEST_ENV_LOCK.lock().unwrap();
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

        let result =
            open_or_focus_iterm_session("sess-retry", "tmux-retry", "/Users/b/repos/retry")
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
}
