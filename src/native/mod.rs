use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::{LazyLock, Mutex};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{sleep, Duration};

use crate::types::{
    GhosttyOpenMode, NativeDesktopApp, NativeDesktopOpenResponse, NativeDesktopStatusResponse,
};

const NATIVE_APP_ENV: &str = "SWIMMERS_NATIVE_APP";
const GHOSTTY_MODE_ENV: &str = "SWIMMERS_GHOSTTY_MODE";
const NATIVE_SCRIPT_ROOT_ENV: &str = "SWIMMERS_NATIVE_SCRIPT_ROOT";
const ITERM_SCRIPT_RELATIVE_PATH: &str = "scripts/iterm-focus.scpt";
const ITERM_SCROLLBACK_PREFILL_LINES: usize = 2000;
const ITERM_OPEN_RETRY_ATTEMPTS: usize = 2;
const ITERM_OPEN_RETRY_DELAY_MS: u64 = 150;
const DEFAULT_ITERM_SESSION_NAME: &str = "Swimmers";
const GHOSTTY_SCRIPT_RELATIVE_PATH: &str = "scripts/ghostty-open.scpt";
const GHOSTTY_MANAGED_TITLE_PREFIX: &str = "swimmers-preview :: ";
const GHOSTTY_MIN_APPLESCRIPT_VERSION: [u64; 3] = [1, 3, 0];
const GHOSTTY_MIN_APPLESCRIPT_VERSION_TEXT: &str = "1.3.0";
const TMUX_BIN_ENV: &str = "SWIMMERS_TMUX_BIN";
const TMUX_BIN_FALLBACKS: &[&str] = &[
    "/opt/homebrew/bin/tmux",
    "/usr/local/bin/tmux",
    "/usr/bin/tmux",
    "/bin/tmux",
];
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
}

pub fn default_native_app() -> NativeDesktopApp {
    std::env::var(NATIVE_APP_ENV)
        .ok()
        .as_deref()
        .map(NativeDesktopApp::from_env_value)
        .unwrap_or(NativeDesktopApp::Iterm)
}

pub fn default_ghostty_open_mode() -> GhosttyOpenMode {
    std::env::var(GHOSTTY_MODE_ENV)
        .ok()
        .as_deref()
        .map(GhosttyOpenMode::from_env_value)
        .unwrap_or(GhosttyOpenMode::Swap)
}

pub fn support_for_host(host: &str, app: NativeDesktopApp) -> NativeDesktopStatusResponse {
    let script_path = script_path_for_app(app);
    let app_unavailable_reason = match app {
        NativeDesktopApp::Iterm => None,
        NativeDesktopApp::Ghostty => ghostty_unavailable_reason(),
    };
    support_for_host_with(
        host,
        app,
        cfg!(target_os = "macos"),
        &script_path,
        app_unavailable_reason,
    )
}

fn support_for_host_with(
    host: &str,
    app: NativeDesktopApp,
    is_macos: bool,
    script_path: &Path,
    app_unavailable_reason: Option<String>,
) -> NativeDesktopStatusResponse {
    let mut response = NativeDesktopStatusResponse {
        supported: false,
        platform: Some(std::env::consts::OS.to_string()),
        app_id: Some(app),
        ghostty_mode: None,
        app: Some(app.display_name().to_string()),
        reason: None,
    };

    if !is_macos {
        response.reason = Some(format!(
            "native {} control is only supported on macOS",
            app.display_name()
        ));
        return response;
    }

    if !host_is_loopback(host) {
        response.reason = Some(format!(
            "native {} control is only available from localhost",
            app.display_name()
        ));
        return response;
    }

    if !script_path.exists() {
        response.reason = Some(format!(
            "native {} script missing: {}",
            app.display_name(),
            script_path.display()
        ));
        return response;
    }

    if let Some(reason) = app_unavailable_reason {
        response.reason = Some(reason);
        return response;
    }

    response.supported = true;
    response.reason = None;
    response
}

pub async fn open_native_session(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
) -> Result<NativeDesktopOpenResponse> {
    match app {
        NativeDesktopApp::Iterm => open_or_focus_iterm_session(session_id, tmux_name, cwd).await,
        NativeDesktopApp::Ghostty => {
            open_or_focus_ghostty_session(session_id, tmux_name, cwd, ghostty_mode).await
        }
    }
}

pub async fn open_or_focus_iterm_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
) -> Result<NativeDesktopOpenResponse> {
    let _guard = NATIVE_OPEN_LOCK.lock().await;
    let script = script_path_for_app(NativeDesktopApp::Iterm);
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
                    && is_transient_iterm_open_error(&err) =>
            {
                sleep(Duration::from_millis(ITERM_OPEN_RETRY_DELAY_MS)).await;
            }
            Err(err) => return Err(err),
        }
    }

    unreachable!("native iTerm open loop should always return or error")
}

async fn open_or_focus_ghostty_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    mode: GhosttyOpenMode,
) -> Result<NativeDesktopOpenResponse> {
    let _guard = NATIVE_OPEN_LOCK.lock().await;
    let script = script_path_for_app(NativeDesktopApp::Ghostty);
    if !script.exists() {
        return Err(anyhow!(
            "native Ghostty script missing: {}",
            script.display()
        ));
    }
    if let Some(reason) = ghostty_unavailable_reason() {
        return Err(anyhow!(reason));
    }

    let tmux_path = resolve_tmux_binary()?;
    let attach_command = build_ghostty_attach_command(tmux_name, &tmux_path);
    let (_, tmux_cwd) = query_tmux_pane_metadata(&tmux_path, tmux_name)
        .await
        .unwrap_or((None, None));
    let resolved_cwd = tmux_cwd.as_deref().unwrap_or(cwd);
    let display_name = build_ghostty_display_name(resolved_cwd, tmux_name);
    let active_tab_id = query_front_ghostty_tab_id().await.unwrap_or(None);
    let known_preview_id = cached_ghostty_preview_term_id(active_tab_id.as_deref());

    let result = run_ghostty_open_script(
        &script,
        session_id,
        tmux_name,
        resolved_cwd,
        &attach_command,
        &display_name,
        mode,
        known_preview_id.as_deref(),
    )
    .await?;

    if mode == GhosttyOpenMode::Swap {
        let resulting_tab_id = query_front_ghostty_tab_id().await.unwrap_or(active_tab_id);
        remember_ghostty_preview_term_id(resulting_tab_id.as_deref(), result.pane_id.as_deref());
    }

    Ok(result)
}

fn script_path_for_app(app: NativeDesktopApp) -> PathBuf {
    let override_root = std::env::var_os(NATIVE_SCRIPT_ROOT_ENV).map(PathBuf::from);
    let current_exe = std::env::current_exe().ok();
    let current_dir = std::env::current_dir().ok();
    resolve_script_path(
        app.script_relative_path(),
        override_root.as_deref(),
        current_exe.as_deref(),
        current_dir.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
}

fn resolve_script_path(
    script_relative_path: &str,
    override_root: Option<&Path>,
    current_exe: Option<&Path>,
    current_dir: Option<&Path>,
    manifest_dir: &Path,
) -> PathBuf {
    let mut roots = Vec::new();
    if let Some(root) = override_root {
        push_unique_root(&mut roots, root);
    }
    if let Some(dir) = current_dir {
        push_ancestor_roots(&mut roots, dir);
    }
    if let Some(exe_dir) = current_exe.and_then(Path::parent) {
        push_ancestor_roots(&mut roots, exe_dir);
    }
    push_unique_root(&mut roots, manifest_dir);

    for root in roots {
        let candidate = root.join(script_relative_path);
        if candidate.is_file() {
            return candidate;
        }
    }

    manifest_dir.join(script_relative_path)
}

fn push_ancestor_roots(roots: &mut Vec<PathBuf>, start: &Path) {
    for ancestor in start.ancestors() {
        push_unique_root(roots, ancestor);
    }
}

fn push_unique_root(roots: &mut Vec<PathBuf>, candidate: &Path) {
    if candidate.as_os_str().is_empty() {
        return;
    }
    if roots.iter().any(|existing| existing == candidate) {
        return;
    }
    roots.push(candidate.to_path_buf());
}

fn ghostty_unavailable_reason() -> Option<String> {
    let output = match ProcessCommand::new("osascript")
        .args(["-e", "tell application \"Ghostty\" to get version"])
        .output()
    {
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

async fn query_front_ghostty_tab_id() -> Result<Option<String>> {
    let output = Command::new("osascript")
        .args([
            "-e",
            "tell application \"Ghostty\"\nif (count of windows) = 0 then return \"\"\nreturn (id of selected tab of front window) as text\nend tell",
        ])
        .output()
        .await
        .map_err(|err| anyhow!("failed to query Ghostty front tab: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            "Ghostty tab query returned a non-zero exit status".to_string()
        } else {
            stderr
        };
        return Err(anyhow!(message));
    }

    Ok(non_empty_trimmed(String::from_utf8_lossy(&output.stdout).trim()).map(ToOwned::to_owned))
}

fn cached_ghostty_preview_term_id(tab_id: Option<&str>) -> Option<String> {
    let tab_id = tab_id.and_then(non_empty_trimmed)?;
    GHOSTTY_PREVIEW_TERM_IDS
        .lock()
        .unwrap()
        .get(tab_id)
        .cloned()
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

fn remember_ghostty_preview_term_id(tab_id: Option<&str>, term_id: Option<&str>) {
    let Some(tab_id) = tab_id.and_then(non_empty_trimmed) else {
        return;
    };

    let mut cache = GHOSTTY_PREVIEW_TERM_IDS.lock().unwrap();
    match term_id.filter(|value| !value.is_empty()) {
        Some(term_id) => {
            cache.insert(tab_id.to_string(), term_id.to_string());
        }
        None => {
            cache.remove(tab_id);
        }
    }
}

#[cfg(test)]
fn clear_ghostty_preview_term_cache() {
    GHOSTTY_PREVIEW_TERM_IDS.lock().unwrap().clear();
}

async fn run_open_or_focus_script(
    script: &Path,
    session_id: &str,
    tmux_name: &str,
    attach_command: &str,
    display_name: &str,
    known_pane_id: Option<&str>,
) -> Result<NativeDesktopOpenResponse> {
    let safe_session_id = sanitize_osascript_text_arg(session_id);
    let safe_display_name = sanitize_osascript_text_arg(display_name);
    let mut command = Command::new("osascript");
    command
        .arg(script)
        .arg(&safe_session_id)
        .arg(tmux_name)
        .arg(attach_command)
        .arg(&safe_display_name);
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

fn build_ghostty_display_name(cwd: &str, tmux_name: &str) -> String {
    cwd_basename(cwd)
        .or_else(|| non_empty_trimmed(tmux_name).map(ToOwned::to_owned))
        .unwrap_or_else(|| "session".to_string())
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

fn build_ghostty_attach_command(tmux_name: &str, tmux_path: &Path) -> String {
    let tmux_name = shell_single_quote(tmux_name);
    let tmux_path = shell_single_quote(tmux_path.to_string_lossy().as_ref());
    format!("exec {tmux_path} attach-session -t {tmux_name}")
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Defense-in-depth: strip bytes that could desync `parse_osascript_output`
/// (which splits on `|`) or corrupt downstream consumers like log lines and
/// terminal titles when request-borne strings round-trip back through them.
///
/// This is NOT shell or AppleScript escaping — both `osascript` invocations
/// pass these values via `Command::arg` (execve-style, no shell), and the
/// `.scpt` files read each argv slot as an opaque AppleScript text item.
fn sanitize_osascript_text_arg(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\0' | '\n' | '\r' | '\t' | '|'))
        .collect()
}

fn is_transient_iterm_open_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    (message.contains("session 1 of missing value") && message.contains("(-1728)"))
        || (message.contains("unable to resolve iTerm session after tab creation")
            && message.contains("(-2700)"))
}

async fn run_ghostty_open_script(
    script: &Path,
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    attach_command: &str,
    display_name: &str,
    mode: GhosttyOpenMode,
    known_preview_id: Option<&str>,
) -> Result<NativeDesktopOpenResponse> {
    let safe_session_id = sanitize_osascript_text_arg(session_id);
    let safe_cwd = sanitize_osascript_text_arg(cwd);
    let safe_display_name = sanitize_osascript_text_arg(display_name);
    let mut command = Command::new("osascript");
    command
        .arg(script)
        .arg(&safe_session_id)
        .arg(tmux_name)
        .arg(&safe_cwd)
        .arg(attach_command)
        .arg(&safe_display_name)
        .arg(GHOSTTY_MANAGED_TITLE_PREFIX)
        .arg(mode.label());
    if let Some(term_id) = known_preview_id.filter(|value| !value.is_empty()) {
        command.arg(term_id);
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
    use proptest::prelude::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

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

        let invalid =
            ghostty_version_requirement_error("beta").expect("invalid versions fail closed");
        assert!(invalid.contains("Ghostty reported version"));
    }

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
    fn build_ghostty_attach_command_shell_quotes_tmux_values() {
        let command =
            build_ghostty_attach_command("team's session", Path::new("/tmp/tmux builds/tmux"));

        assert_eq!(
            command,
            "exec '/tmp/tmux builds/tmux' attach-session -t 'team'\"'\"'s session'"
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
        let script = std::fs::read_to_string(script_path_for_app(NativeDesktopApp::Ghostty))
            .expect("ghostty script should be present");

        assert!(script.contains("on managedTerminals(targetTab, managedTitlePrefix)"));
        assert!(script.contains("on closeManagedTerminals(targetTerms)"));
        assert!(script
            .contains("set managedTerms to my managedTerminals(targetTab, managedTitlePrefix)"));
        assert!(script.contains("my closeManagedTerminals(duplicateManagedTerms)"));
        assert!(script
            .contains("on replacePreviewSplit(managedTerm, cfg, managedTitle, attachCommand)"));
        assert!(script
            .contains("set newTerm to split managedTerm direction right with configuration cfg"));
        assert!(script.contains(
            "set newTerm to my replacePreviewSplit(managedTerm, cfg, managedTitle, attachCommand)"
        ));
        assert_eq!(script.match_indices("my resizePreview(").count(), 1);
    }

    #[test]
    fn support_for_host_with_reports_ghostty_app_and_unavailable_reason() {
        let temp = tempdir().unwrap();
        let script_path = temp.path().join("ghostty-open.scpt");
        std::fs::write(&script_path, "").unwrap();
        let response = support_for_host_with(
            "localhost:3210",
            NativeDesktopApp::Ghostty,
            true,
            &script_path,
            Some(
                "Ghostty 1.2.3 is installed, but native AppleScript control requires Ghostty 1.3.0+."
                    .to_string(),
            ),
        );
        assert!(!response.supported);
        assert_eq!(response.app.as_deref(), Some("Ghostty"));
        assert!(response
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("Ghostty 1.2.3 is installed"));
    }

    #[test]
    fn support_for_host_with_uses_selected_app_in_loopback_errors() {
        let temp = tempdir().unwrap();
        let script_path = temp.path().join("ghostty-open.scpt");
        std::fs::write(&script_path, "").unwrap();
        let response = support_for_host_with(
            "example.com:3210",
            NativeDesktopApp::Ghostty,
            true,
            &script_path,
            None,
        );
        assert!(!response.supported);
        assert_eq!(response.app.as_deref(), Some("Ghostty"));
        assert_eq!(
            response.reason.as_deref(),
            Some("native Ghostty control is only available from localhost")
        );
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
        );

        assert_eq!(resolved, override_script);
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
        );

        assert_eq!(resolved, script_path);
    }

    #[test]
    fn detects_transient_iterm_open_errors() {
        let retryable = anyhow!(
            "/tmp/iterm-focus.scpt: execution error: Can't get session 1 of missing value. (-1728)"
        );
        assert!(is_transient_iterm_open_error(&retryable));

        let tab_creation_race = anyhow!(
            "/tmp/iterm-focus.scpt: execution error: unable to resolve iTerm session after tab creation (-2700)"
        );
        assert!(is_transient_iterm_open_error(&tab_creation_race));

        let other = anyhow!("unexpected osascript status: created");
        assert!(!is_transient_iterm_open_error(&other));
    }

    #[tokio::test]
    async fn open_or_focus_passes_cached_pane_id_on_repeat_calls() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        remember_pane_id("sess-cache", None);

        let temp = tempdir().unwrap();
        let fake_bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();

        let fake_tmux = fake_bin_dir.join("tmux");
        std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%12\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
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
        assert_eq!(first_call[4], "12 swimmers");
        assert_eq!(first_call.len(), 5);
        assert_eq!(second_call[4], "12 swimmers");
        assert_eq!(second_call[5], "pane-1");

        remember_pane_id("sess-cache", None);
    }

    #[tokio::test]
    async fn open_or_focus_retries_transient_missing_session_error() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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

    #[tokio::test]
    async fn open_or_focus_retries_transient_tab_creation_resolution_error() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        remember_pane_id("sess-race", None);

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
                "#!/bin/sh\nset -eu\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nif [ \"$count\" -eq 1 ]; then\n  printf \"%s\\n\" \"scripts/iterm-focus.scpt: execution error: unable to resolve iTerm session after tab creation (-2700)\" >&2\n  exit 1\nfi\nprintf 'created|pane-race\\n'\n",
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

        let result = open_or_focus_iterm_session("sess-race", "tmux-race", "/Users/b/repos/race")
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
        assert_eq!(result.pane_id.as_deref(), Some("pane-race"));
        assert_eq!(std::fs::read_to_string(&counter_path).unwrap().trim(), "2");

        remember_pane_id("sess-race", None);
    }

    #[tokio::test]
    async fn open_or_focus_ghostty_runs_script_with_expected_args() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_ghostty_preview_term_cache();

        let temp = tempdir().unwrap();
        let fake_bin_dir = temp.path().join("tmux builds");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();

        let fake_tmux = fake_bin_dir.join("tmux");
        std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_tmux, perms).unwrap();

        let log_path = temp.path().join("osascript.log");
        let fake_osascript_dir = temp.path().join("bin");
        std::fs::create_dir_all(&fake_osascript_dir).unwrap();
        let fake_osascript = fake_osascript_dir.join("osascript");
        std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      printf 'ghostty-tab-main\\n'\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\nprintf 'created|ghost-pane\\n'\n",
                log = log_path.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_osascript, perms).unwrap();

        let original_path = std::env::var_os("PATH");
        let original_tmux = std::env::var_os(TMUX_BIN_ENV);
        let path_value = std::env::join_paths([fake_osascript_dir.as_path()]).unwrap();
        std::env::set_var("PATH", path_value);
        std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

        let result = open_or_focus_ghostty_session(
            "sess-ghostty",
            "tmux-ghostty",
            "/Users/b/repos/fallback",
            GhosttyOpenMode::Swap,
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

        assert_eq!(result.status, "created");
        assert_eq!(result.pane_id.as_deref(), Some("ghost-pane"));

        let log = std::fs::read_to_string(&log_path).unwrap();
        let call: Vec<_> = log.lines().next().unwrap().split('\t').collect();
        assert_eq!(call[1], "sess-ghostty");
        assert_eq!(call[2], "tmux-ghostty");
        assert_eq!(call[3], "/Users/b/repos/swimmers");
        assert_eq!(
            call[4],
            format!(
                "exec '{}' attach-session -t 'tmux-ghostty'",
                fake_tmux.display()
            )
        );
        assert_eq!(call[5], "swimmers");
        assert_eq!(call[6], GHOSTTY_MANAGED_TITLE_PREFIX);
        assert_eq!(call[7], GhosttyOpenMode::Swap.label());

        clear_ghostty_preview_term_cache();
    }

    #[tokio::test]
    async fn open_or_focus_ghostty_swap_passes_cached_preview_id_on_repeat_calls() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_ghostty_preview_term_cache();

        let temp = tempdir().unwrap();
        let fake_bin_dir = temp.path().join("tmux builds");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();

        let fake_tmux = fake_bin_dir.join("tmux");
        std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_tmux, perms).unwrap();

        let log_path = temp.path().join("osascript.log");
        let counter_path = temp.path().join("osascript-count");
        let fake_osascript_dir = temp.path().join("bin");
        std::fs::create_dir_all(&fake_osascript_dir).unwrap();
        let fake_osascript = fake_osascript_dir.join("osascript");
        std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      printf 'ghostty-tab-main\\n'\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{counter}\" ]; then\n  IFS= read -r count < \"{counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{counter}\"\nprintf 'created|ghost-pane-%s\\n' \"$count\"\n",
                log = log_path.display(),
                counter = counter_path.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_osascript, perms).unwrap();

        let original_path = std::env::var_os("PATH");
        let original_tmux = std::env::var_os(TMUX_BIN_ENV);
        let path_value = std::env::join_paths([fake_osascript_dir.as_path()]).unwrap();
        std::env::set_var("PATH", path_value);
        std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

        let first = open_or_focus_ghostty_session(
            "sess-ghostty-a",
            "tmux-ghostty-a",
            "/Users/b/repos/fallback",
            GhosttyOpenMode::Swap,
        )
        .await
        .unwrap();
        let second = open_or_focus_ghostty_session(
            "sess-ghostty-b",
            "tmux-ghostty-b",
            "/Users/b/repos/fallback",
            GhosttyOpenMode::Swap,
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

        assert_eq!(first.pane_id.as_deref(), Some("ghost-pane-1"));
        assert_eq!(second.pane_id.as_deref(), Some("ghost-pane-2"));

        let log = std::fs::read_to_string(&log_path).unwrap();
        let mut lines = log.lines();
        let first_call: Vec<_> = lines.next().unwrap().split('\t').collect();
        let second_call: Vec<_> = lines.next().unwrap().split('\t').collect();
        assert_eq!(first_call[7], GhosttyOpenMode::Swap.label());
        assert_eq!(first_call.len(), 8);
        assert_eq!(second_call[7], GhosttyOpenMode::Swap.label());
        assert_eq!(second_call[8], "ghost-pane-1");

        clear_ghostty_preview_term_cache();
    }

    #[tokio::test]
    async fn open_or_focus_ghostty_swap_scopes_cached_preview_id_to_active_tab() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_ghostty_preview_term_cache();

        let temp = tempdir().unwrap();
        let fake_bin_dir = temp.path().join("tmux builds");
        std::fs::create_dir_all(&fake_bin_dir).unwrap();

        let fake_tmux = fake_bin_dir.join("tmux");
        std::fs::write(
            &fake_tmux,
            "#!/bin/sh\nset -eu\nif [ \"${1-}\" = \"display-message\" ]; then\n  printf '%%14\\t/Users/b/repos/swimmers\\n'\n  exit 0\nfi\nexit 0\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_tmux).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_tmux, perms).unwrap();

        let log_path = temp.path().join("osascript.log");
        let open_counter_path = temp.path().join("osascript-open-count");
        let tab_counter_path = temp.path().join("osascript-tab-count");
        let fake_osascript_dir = temp.path().join("bin");
        std::fs::create_dir_all(&fake_osascript_dir).unwrap();
        let fake_osascript = fake_osascript_dir.join("osascript");
        std::fs::write(
            &fake_osascript,
            format!(
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.3.1\\n'\n      ;;\n    *)\n      count=0\n      if [ -f \"{tab_counter}\" ]; then\n        IFS= read -r count < \"{tab_counter}\" || true\n      fi\n      count=$((count + 1))\n      printf '%s\\n' \"$count\" > \"{tab_counter}\"\n      case \"$count\" in\n        1|2) printf 'ghostty-tab-a\\n' ;;\n        3|4) printf 'ghostty-tab-b\\n' ;;\n        *) printf 'ghostty-tab-a\\n' ;;\n      esac\n      ;;\n  esac\n  exit 0\nfi\nfirst=1\nfor arg in \"$@\"; do\n  if [ \"$first\" -eq 1 ]; then\n    printf '%s' \"$arg\" >> \"{log}\"\n    first=0\n  else\n    printf '\\t%s' \"$arg\" >> \"{log}\"\n  fi\ndone\nprintf '\\n' >> \"{log}\"\ncount=0\nif [ -f \"{open_counter}\" ]; then\n  IFS= read -r count < \"{open_counter}\" || true\nfi\ncount=$((count + 1))\nprintf '%s\\n' \"$count\" > \"{open_counter}\"\nprintf 'created|ghost-pane-%s\\n' \"$count\"\n",
                log = log_path.display(),
                open_counter = open_counter_path.display(),
                tab_counter = tab_counter_path.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake_osascript).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_osascript, perms).unwrap();

        let original_path = std::env::var_os("PATH");
        let original_tmux = std::env::var_os(TMUX_BIN_ENV);
        let path_value = std::env::join_paths([fake_osascript_dir.as_path()]).unwrap();
        std::env::set_var("PATH", path_value);
        std::env::set_var(TMUX_BIN_ENV, &fake_tmux);

        open_or_focus_ghostty_session(
            "sess-ghostty-a1",
            "tmux-ghostty-a1",
            "/Users/b/repos/fallback",
            GhosttyOpenMode::Swap,
        )
        .await
        .unwrap();
        open_or_focus_ghostty_session(
            "sess-ghostty-b1",
            "tmux-ghostty-b1",
            "/Users/b/repos/fallback",
            GhosttyOpenMode::Swap,
        )
        .await
        .unwrap();
        open_or_focus_ghostty_session(
            "sess-ghostty-a2",
            "tmux-ghostty-a2",
            "/Users/b/repos/fallback",
            GhosttyOpenMode::Swap,
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

        let log = std::fs::read_to_string(&log_path).unwrap();
        let calls: Vec<Vec<_>> = log.lines().map(|line| line.split('\t').collect()).collect();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].len(), 8);
        assert_eq!(calls[1].len(), 8);
        assert_eq!(calls[2][8], "ghost-pane-1");

        clear_ghostty_preview_term_cache();
    }

    #[tokio::test]
    async fn open_or_focus_ghostty_rejects_older_versions_before_running_script() {
        let _env_guard = crate::test_support::ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        clear_ghostty_preview_term_cache();

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
                "#!/bin/sh\nset -eu\nif [ \"${{1-}}\" = \"-e\" ]; then\n  case \"${{2-}}\" in\n    *\"get version\"*)\n      printf '1.2.3\\n'\n      ;;\n    *)\n      printf 'ghostty-tab-main\\n'\n      ;;\n  esac\n  exit 0\nfi\nprintf '%s\\n' \"$*\" >> \"{log}\"\nprintf 'created|ghost-pane\\n'\n",
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

        let error = open_or_focus_ghostty_session(
            "sess-ghostty",
            "tmux-ghostty",
            "/tmp/fallback",
            GhosttyOpenMode::Swap,
        )
        .await
        .expect_err("older Ghostty should be rejected before running the script");

        match original_path {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
        match original_tmux {
            Some(value) => std::env::set_var(TMUX_BIN_ENV, value),
            None => std::env::remove_var(TMUX_BIN_ENV),
        }

        assert!(error.to_string().contains("Ghostty 1.2.3 is installed"));
        assert!(!log_path.exists());
        clear_ghostty_preview_term_cache();
    }

    #[test]
    fn ghostty_preview_cache_is_invariant_under_other_tabs() {
        let mut runner = proptest::test_runner::TestRunner::default();
        let strategy = proptest::collection::vec((0u8..3, 0u16..256), 0..40);

        runner
            .run(&strategy, |ops| {
                clear_ghostty_preview_term_cache();

                for (tab, pane) in &ops {
                    let tab_id = format!("tab-{tab}");
                    let pane_id = format!("pane-{pane}");
                    remember_ghostty_preview_term_id(Some(tab_id.as_str()), Some(pane_id.as_str()));
                }
                let interleaved = cached_ghostty_preview_term_id(Some("tab-0"));

                clear_ghostty_preview_term_cache();
                for (_, pane) in ops.iter().filter(|(tab, _)| *tab == 0) {
                    let pane_id = format!("pane-{pane}");
                    remember_ghostty_preview_term_id(Some("tab-0"), Some(pane_id.as_str()));
                }
                let projected = cached_ghostty_preview_term_id(Some("tab-0"));

                clear_ghostty_preview_term_cache();
                prop_assert_eq!(interleaved, projected);
                Ok(())
            })
            .unwrap();
    }
}
