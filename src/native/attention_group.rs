use std::collections::VecDeque;
use std::path::Path;

use anyhow::{anyhow, Result};

use crate::tmux_target::{exact_pane_target, exact_session_target};
use crate::types::{
    AttentionGroupLayout, GhosttyOpenMode, NativeAttentionGroupOpenResponse, NativeDesktopApp,
    SessionSummary,
};

pub(super) const ATTENTION_GROUP_SESSION_ID: &str = "attention-group";
pub(super) const ATTENTION_GROUP_TMUX_NAME: &str = "swimmers-attention";
const ATTENTION_GROUP_PANE_TITLE_PREFIX: &str = "swimmers-attention:";

pub async fn open_native_attention_group(
    app: NativeDesktopApp,
    ghostty_mode: GhosttyOpenMode,
    sessions: &[SessionSummary],
    focus: bool,
    layout: AttentionGroupLayout,
) -> Result<NativeAttentionGroupOpenResponse> {
    if sessions.is_empty() {
        return Err(anyhow!("no sessions are waiting for operator input"));
    }

    let tmux_path = super::resolve_tmux_binary()?;
    sync_attention_group_tmux_session(&tmux_path, sessions, layout).await?;
    let attach_command = attention_group_attach_command();
    let open_result = if focus {
        Some(
            super::open_native_session(
                app,
                attention_group_ghostty_mode(app, ghostty_mode),
                ATTENTION_GROUP_SESSION_ID,
                ATTENTION_GROUP_TMUX_NAME,
                "",
            )
            .await?,
        )
    } else {
        None
    };

    Ok(NativeAttentionGroupOpenResponse {
        session_id: ATTENTION_GROUP_SESSION_ID.to_string(),
        tmux_name: ATTENTION_GROUP_TMUX_NAME.to_string(),
        session_count: sessions.len(),
        session_ids: sessions
            .iter()
            .map(|session| session.session_id.clone())
            .collect(),
        backlog_session_ids: Vec::new(),
        status: open_result
            .as_ref()
            .map(|result| result.status.clone())
            .unwrap_or_else(|| "refreshed".to_string()),
        focused: focus,
        pane_id: open_result.and_then(|result| result.pane_id),
        attach_command: Some(attach_command),
    })
}

pub async fn clear_native_attention_group() -> Result<NativeAttentionGroupOpenResponse> {
    let tmux_path = super::resolve_tmux_binary()?;
    let session_target = attention_group_session_target();
    let _ = super::run_tmux_status(&tmux_path, &["kill-session", "-t", &session_target]).await;

    Ok(NativeAttentionGroupOpenResponse {
        session_id: ATTENTION_GROUP_SESSION_ID.to_string(),
        tmux_name: ATTENTION_GROUP_TMUX_NAME.to_string(),
        session_count: 0,
        session_ids: Vec::new(),
        backlog_session_ids: Vec::new(),
        status: "cleared".to_string(),
        focused: false,
        pane_id: None,
        attach_command: Some(attention_group_attach_command()),
    })
}

pub fn attention_group_attach_command() -> String {
    format!("tmux attach -t {ATTENTION_GROUP_TMUX_NAME}")
}

fn attention_group_ghostty_mode(
    app: NativeDesktopApp,
    configured_mode: GhosttyOpenMode,
) -> GhosttyOpenMode {
    match app {
        NativeDesktopApp::Ghostty => GhosttyOpenMode::Window,
        NativeDesktopApp::Iterm => configured_mode,
    }
}

async fn sync_attention_group_tmux_session(
    tmux_path: &Path,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    let session_target = attention_group_session_target();
    if super::run_tmux_status(tmux_path, &["has-session", "-t", &session_target])
        .await
        .is_err()
    {
        return rebuild_attention_group_tmux_session(tmux_path, sessions, layout).await;
    }

    replace_attention_group_tmux_panes(tmux_path, sessions, layout).await
}

async fn rebuild_attention_group_tmux_session(
    tmux_path: &Path,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    let session_target = attention_group_session_target();
    let pane_target = attention_group_pane_target();
    let _ = super::run_tmux_status(tmux_path, &["kill-session", "-t", &session_target]).await;

    let first = sessions
        .first()
        .ok_or_else(|| anyhow!("no sessions are waiting for operator input"))?;
    let output = super::run_tmux_output(
        tmux_path,
        &[
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            ATTENTION_GROUP_TMUX_NAME,
            "-n",
            "attention",
            &build_attention_group_attach_command(&first.tmux_name, tmux_path),
        ],
    )
    .await
    .map_err(|err| anyhow!("failed to create tmux session {ATTENTION_GROUP_TMUX_NAME}: {err}"))?;
    if let Some(pane_id) = first_output_line(output.stdout.as_slice())? {
        set_attention_group_pane_title(tmux_path, &pane_id, &first.session_id).await?;
    }

    for session in sessions.iter().skip(1) {
        split_attention_group_pane(tmux_path, &pane_target, session).await?;
        tile_attention_group_tmux_session(tmux_path, &pane_target, layout).await?;
    }

    tile_attention_group_tmux_session(tmux_path, &pane_target, layout).await?;
    Ok(())
}

async fn replace_attention_group_tmux_panes(
    tmux_path: &Path,
    sessions: &[SessionSummary],
    layout: AttentionGroupLayout,
) -> Result<()> {
    let pane_target = attention_group_pane_target();
    let panes = list_attention_group_panes(tmux_path, &pane_target).await?;
    let desired = sessions
        .iter()
        .map(|session| {
            (
                session,
                build_attention_group_attach_command(&session.tmux_name, tmux_path),
            )
        })
        .collect::<Vec<_>>();

    if panes.len() == desired.len()
        && panes
            .iter()
            .zip(desired.iter())
            .all(|(pane, (_, command))| pane.start_command == *command)
    {
        return Ok(());
    }

    let mut available = panes;
    let mut missing = Vec::new();
    for (session, command) in desired {
        if let Some(index) = available
            .iter()
            .position(|pane| pane.start_command == command)
        {
            available.remove(index);
        } else {
            missing.push(session);
        }
    }

    let mut stale_panes = VecDeque::from(available);
    let mut changed = false;
    for session in missing {
        if let Some(pane) = stale_panes.pop_front() {
            respawn_attention_group_pane(tmux_path, &pane.pane_id, session).await?;
        } else {
            split_attention_group_pane(tmux_path, &pane_target, session).await?;
        }
        changed = true;
    }

    for pane in stale_panes {
        super::run_tmux_status(tmux_path, &["kill-pane", "-t", &pane.pane_id])
            .await
            .map_err(|err| anyhow!("failed to clear stale attention group pane: {err}"))?;
        changed = true;
    }

    if changed {
        tile_attention_group_tmux_session(tmux_path, &pane_target, layout).await?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct AttentionGroupPane {
    pane_id: String,
    start_command: String,
}

async fn list_attention_group_panes(
    tmux_path: &Path,
    pane_target: &str,
) -> Result<Vec<AttentionGroupPane>> {
    let output = super::run_tmux_output(
        tmux_path,
        &[
            "list-panes",
            "-t",
            pane_target,
            "-F",
            "#{pane_id}\t#{pane_start_command}",
        ],
    )
    .await?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| anyhow!("tmux list-panes returned non-UTF-8 output: {err}"))?;
    let panes = stdout
        .lines()
        .filter_map(parse_attention_group_pane_line)
        .collect::<Vec<_>>();
    if panes.is_empty() {
        return Err(anyhow!("attention group has no panes"));
    }
    Ok(panes)
}

fn parse_attention_group_pane_line(line: &str) -> Option<AttentionGroupPane> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (pane_id, start_command) = trimmed
        .split_once('\t')
        .map(|(pane_id, command)| (pane_id.trim(), command.trim()))
        .unwrap_or((trimmed, ""));
    if pane_id.is_empty() {
        return None;
    }
    Some(AttentionGroupPane {
        pane_id: pane_id.to_string(),
        start_command: start_command.to_string(),
    })
}

async fn respawn_attention_group_pane(
    tmux_path: &Path,
    pane_id: &str,
    session: &SessionSummary,
) -> Result<()> {
    super::run_tmux_status(
        tmux_path,
        &[
            "respawn-pane",
            "-k",
            "-t",
            pane_id,
            &build_attention_group_attach_command(&session.tmux_name, tmux_path),
        ],
    )
    .await
    .map_err(|err| {
        anyhow!(
            "failed to refresh attention group pane for {}: {err}",
            session.tmux_name
        )
    })?;
    set_attention_group_pane_title(tmux_path, pane_id, &session.session_id).await
}

async fn split_attention_group_pane(
    tmux_path: &Path,
    pane_target: &str,
    session: &SessionSummary,
) -> Result<()> {
    let output = super::run_tmux_output(
        tmux_path,
        &[
            "split-window",
            "-t",
            pane_target,
            "-P",
            "-F",
            "#{pane_id}",
            &build_attention_group_attach_command(&session.tmux_name, tmux_path),
        ],
    )
    .await
    .map_err(|err| {
        anyhow!(
            "failed to add {} to attention group: {err}",
            session.tmux_name
        )
    })?;
    if let Some(pane_id) = first_output_line(output.stdout.as_slice())? {
        set_attention_group_pane_title(tmux_path, &pane_id, &session.session_id).await?;
    }
    Ok(())
}

async fn set_attention_group_pane_title(
    tmux_path: &Path,
    pane_id: &str,
    session_id: &str,
) -> Result<()> {
    super::run_tmux_status(
        tmux_path,
        &[
            "select-pane",
            "-t",
            pane_id,
            "-T",
            &attention_group_pane_title(session_id),
        ],
    )
    .await
    .map_err(|err| anyhow!("failed to label attention group pane {pane_id}: {err}"))
}

fn first_output_line(stdout: &[u8]) -> Result<Option<String>> {
    let stdout = String::from_utf8(stdout.to_vec())
        .map_err(|err| anyhow!("tmux returned non-UTF-8 output: {err}"))?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned))
}

fn attention_group_pane_title(session_id: &str) -> String {
    let sanitized = sanitize_tmux_pane_title(session_id);
    format!("{ATTENTION_GROUP_PANE_TITLE_PREFIX}{sanitized}")
}

fn sanitize_tmux_pane_title(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '\0' | '\n' | '\r' | '\t'))
        .collect()
}

async fn tile_attention_group_tmux_session(
    tmux_path: &Path,
    pane_target: &str,
    layout: AttentionGroupLayout,
) -> Result<()> {
    super::run_tmux_status(
        tmux_path,
        &[
            "select-layout",
            "-t",
            pane_target,
            attention_group_tmux_layout(layout),
        ],
    )
    .await
    .map_err(|err| anyhow!("failed to tile attention group panes: {err}"))
}

fn attention_group_tmux_layout(layout: AttentionGroupLayout) -> &'static str {
    match layout {
        AttentionGroupLayout::Tiled => "tiled",
        AttentionGroupLayout::EvenHorizontal => "even-horizontal",
        AttentionGroupLayout::EvenVertical => "even-vertical",
        AttentionGroupLayout::MainHorizontal => "main-horizontal",
        AttentionGroupLayout::MainVertical => "main-vertical",
    }
}

pub(super) fn attention_group_session_target() -> String {
    exact_session_target(ATTENTION_GROUP_TMUX_NAME)
}

pub(super) fn attention_group_pane_target() -> String {
    exact_pane_target(ATTENTION_GROUP_TMUX_NAME)
}

pub(super) fn build_attention_group_attach_command(tmux_name: &str, tmux_path: &Path) -> String {
    format!(
        "exec env TMUX= {} attach-session -t {}",
        super::shell_quote_token(&tmux_path.to_string_lossy()),
        super::shell_quote_token(&exact_session_target(tmux_name))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_attention_group_attach_command_unsets_nested_tmux_and_exact_targets() {
        let command = build_attention_group_attach_command("team session", Path::new("/tmp/tmux"));
        assert_eq!(
            command,
            "exec env TMUX= /tmp/tmux attach-session -t '=team session'"
        );
    }

    #[test]
    fn attention_group_uses_session_target_for_kill_and_pane_target_for_layout_commands() {
        assert_eq!(attention_group_session_target(), "=swimmers-attention");
        assert_eq!(attention_group_pane_target(), "=swimmers-attention:");
    }
}
