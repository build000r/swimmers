use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

use crate::types::{GhosttyOpenMode, NativeDesktopApp, NativeDesktopOpenResponse};

struct GhosttyOpenContext {
    script: PathBuf,
    resolved_cwd: String,
    attach_command: String,
    display_name: String,
    active_tab_id: Option<String>,
    known_term_id: Option<String>,
}

pub(super) async fn open_or_focus_ghostty_session(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    mode: GhosttyOpenMode,
) -> Result<NativeDesktopOpenResponse> {
    let _guard = super::NATIVE_OPEN_LOCK.lock().await;
    let context = prepare_ghostty_open_context(session_id, tmux_name, cwd, mode).await?;
    let mut result = run_ghostty_open_script(
        &context.script,
        session_id,
        tmux_name,
        &context.resolved_cwd,
        &context.attach_command,
        &context.display_name,
        mode,
        context.known_term_id.as_deref(),
    )
    .await?;

    mark_stale_swap_fallback(mode, context.known_term_id.as_deref(), &mut result);
    remember_ghostty_open_result(mode, session_id, context.active_tab_id, &result).await;
    Ok(result)
}

async fn prepare_ghostty_open_context(
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    mode: GhosttyOpenMode,
) -> Result<GhosttyOpenContext> {
    let script = super::script_path_for_app(NativeDesktopApp::Ghostty)?;
    if !script.exists() {
        return Err(anyhow!(
            "native Ghostty script missing: {}",
            script.display()
        ));
    }
    if let Some(reason) = super::ghostty_unavailable_reason() {
        return Err(anyhow!(reason));
    }

    let tmux_path = super::resolve_tmux_binary()?;
    let attach_command = super::build_ghostty_attach_command(tmux_name, &tmux_path);
    let (_, tmux_cwd) = super::query_tmux_pane_metadata(&tmux_path, tmux_name)
        .await
        .unwrap_or((None, None));
    let resolved_cwd = tmux_cwd.unwrap_or_else(|| cwd.to_string());
    let display_name = super::build_ghostty_display_name(&resolved_cwd, tmux_name);
    let active_tab_id = active_ghostty_tab_for_mode(mode).await;
    let known_term_id = known_ghostty_term_for_mode(mode, session_id, active_tab_id.as_deref());

    Ok(GhosttyOpenContext {
        script,
        resolved_cwd,
        attach_command,
        display_name,
        active_tab_id,
        known_term_id,
    })
}

async fn active_ghostty_tab_for_mode(mode: GhosttyOpenMode) -> Option<String> {
    if mode == GhosttyOpenMode::Swap {
        query_front_ghostty_tab_id().await.unwrap_or(None)
    } else {
        None
    }
}

fn known_ghostty_term_for_mode(
    mode: GhosttyOpenMode,
    session_id: &str,
    active_tab_id: Option<&str>,
) -> Option<String> {
    match mode {
        GhosttyOpenMode::Swap => cached_ghostty_preview_term_id(active_tab_id),
        GhosttyOpenMode::Window => super::cached_pane_id(session_id),
        GhosttyOpenMode::Add => None,
    }
}

fn mark_stale_swap_fallback(
    mode: GhosttyOpenMode,
    known_term_id: Option<&str>,
    result: &mut NativeDesktopOpenResponse,
) {
    if mode == GhosttyOpenMode::Swap && known_term_id.is_some() && result.status == "created" {
        result.status = "fallback_created".to_string();
    }
}

async fn remember_ghostty_open_result(
    mode: GhosttyOpenMode,
    session_id: &str,
    active_tab_id: Option<String>,
    result: &NativeDesktopOpenResponse,
) {
    match mode {
        GhosttyOpenMode::Swap => {
            let resulting_tab_id = query_front_ghostty_tab_id().await.unwrap_or(active_tab_id);
            if result.status == "fallback_created" {
                remember_ghostty_preview_term_id(resulting_tab_id.as_deref(), None);
            } else {
                remember_ghostty_preview_term_id(
                    resulting_tab_id.as_deref(),
                    result.pane_id.as_deref(),
                );
            }
        }
        GhosttyOpenMode::Window => {
            super::remember_pane_id(session_id, result.pane_id.as_deref());
        }
        GhosttyOpenMode::Add => {}
    }
}

async fn query_front_ghostty_tab_id() -> Result<Option<String>> {
    let mut command = Command::new("osascript");
    command.args([
        "-e",
        "tell application \"Ghostty\"\nif (count of windows) = 0 then return \"\"\nreturn (id of selected tab of front window) as text\nend tell",
    ]);
    let output = super::run_osascript_output(&mut command, "querying Ghostty front tab")
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

    Ok(
        super::non_empty_trimmed(String::from_utf8_lossy(&output.stdout).trim())
            .map(ToOwned::to_owned),
    )
}

pub(super) fn cached_ghostty_preview_term_id(tab_id: Option<&str>) -> Option<String> {
    let tab_id = tab_id.and_then(super::non_empty_trimmed)?;
    super::GHOSTTY_PREVIEW_TERM_IDS
        .lock()
        .unwrap()
        .get(tab_id)
        .cloned()
}

pub(super) fn remember_ghostty_preview_term_id(tab_id: Option<&str>, term_id: Option<&str>) {
    let Some(tab_id) = tab_id.and_then(super::non_empty_trimmed) else {
        return;
    };

    let mut cache = super::GHOSTTY_PREVIEW_TERM_IDS.lock().unwrap();
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
pub(super) fn clear_ghostty_preview_term_cache() {
    super::GHOSTTY_PREVIEW_TERM_IDS.lock().unwrap().clear();
}

#[allow(clippy::too_many_arguments)]
async fn run_ghostty_open_script(
    script: &Path,
    session_id: &str,
    tmux_name: &str,
    cwd: &str,
    attach_command: &str,
    display_name: &str,
    mode: GhosttyOpenMode,
    known_term_id: Option<&str>,
) -> Result<NativeDesktopOpenResponse> {
    super::validate_osascript_script_arg("tmux_name", tmux_name)?;
    super::validate_osascript_script_arg("attach_command", attach_command)?;

    let safe_session_id = super::sanitize_osascript_text_arg(session_id);
    let safe_cwd = super::sanitize_osascript_text_arg(cwd);
    let safe_display_name = super::sanitize_osascript_text_arg(display_name);
    let managed_title_prefix = ghostty_managed_title_prefix(session_id);
    let mut command = Command::new("osascript");
    command
        .arg(script)
        .arg(&safe_session_id)
        .arg(tmux_name)
        .arg(&safe_cwd)
        .arg(attach_command)
        .arg(&safe_display_name)
        .arg(managed_title_prefix)
        .arg(mode.label());
    if let Some(term_id) = known_term_id.filter(|value| !value.is_empty()) {
        command.arg(term_id);
    }

    let output = super::run_osascript_output(&mut command, "opening/focusing Ghostty session")
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

fn ghostty_managed_title_prefix(session_id: &str) -> &'static str {
    if session_id == super::ATTENTION_GROUP_SESSION_ID {
        super::GHOSTTY_ATTENTION_MANAGED_TITLE_PREFIX
    } else {
        super::GHOSTTY_MANAGED_TITLE_PREFIX
    }
}

pub(super) fn parse_osascript_output(
    stdout: &str,
    session_id: &str,
) -> Result<NativeDesktopOpenResponse> {
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
        "created" | "focused" | "swapped" | "fallback_created" => Ok(NativeDesktopOpenResponse {
            session_id: session_id.to_string(),
            status,
            pane_id,
        }),
        other if !other.is_empty() => Err(anyhow!("unexpected osascript status: {other}")),
        _ => Err(anyhow!("osascript returned an empty response")),
    }
}
